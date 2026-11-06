//! PipeWire oqimidan kadr olish va H.264 ga kodlash (GStreamer).
//!
//! Trafik va CPU uchun muhim qoidalar:
//! - Ekran o'zgarmasa kadr kodlanmaydi: `videorate drop-only=true` faqat ortiqcha kadrlarni tashlaydi, takrorlamaydi
//!   (PipeWire faqat o'zgarishda kadr beradi).
//! - Kodlangan kadr hech qachon tashlanmaydi (H.264 zanjiri uzilmasin). Ortiqcha yuk kodlagichdan **oldin** tashlanadi:
//!   `queue leaky` (kodlagich ulgurmasa) va kodlagich kirishidagi probe (`VideoControl::should_skip`, tarmoq/Viewer ulgurmasa).
//! - Kalit kadr (IDR) faqat boshida va Viewer so'raganda (`force_keyframe`): davriy IDR trafikni ko'paytiradi.
//! - Masshtab rang o'girishdan oldin: kichraytirilgan kadr o'giriladi (4K manbada CPU ancha kam).

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use protocol::video_reason;
use rdcore::pacing::VideoControl;
use std::{
    os::fd::{AsRawFd, OwnedFd},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::mpsc;

/// Kodlagichlar ustuvorlik tartibida: apparat tezlatish birinchi.
const ENCODERS: [&str; 4] = ["vah264enc", "nvh264enc", "x264enc", "openh264enc"];

pub struct Frame {
    pub keyframe: bool,
    pub pts_us: u64,
    pub data: Vec<u8>,
}

pub struct Capture {
    pipeline: gst::Pipeline,
    enc: gst::Element,
    pub frames: mpsc::Receiver<Frame>,
    pub errors: mpsc::UnboundedReceiver<String>,
    pub encoder: &'static str,
    /// PipeWire bergan haqiqiy kadr o'lchami (piksel). Caps kelishilgach to'ladi.
    pub source_size: Arc<Mutex<Option<(i32, i32)>>>,
    stop: Arc<AtomicBool>,
    _fd: Option<OwnedFd>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

impl Capture {
    /// Kodlagichdan keyingi kadrni kalit kadr (IDR, SPS/PPS bilan) qilishni so'raydi.
    pub fn force_keyframe(&self) {
        let s = gst::Structure::builder("GstForceKeyUnit")
            .field("all-headers", true)
            .field("count", 0u32)
            .build();
        if let Some(pad) = self.enc.static_pad("src") {
            if !pad.send_event(gst::event::CustomUpstream::new(s)) {
                eprintln!("[host] kodlagich kalit kadr so'rovini qabul qilmadi");
            }
        }
    }
}

pub struct CaptureError {
    pub reason: u8,
    pub error: anyhow::Error,
}

fn fail(reason: u8, error: anyhow::Error) -> CaptureError {
    CaptureError { reason, error }
}

fn encoder_desc(factory: &str, kbps: u32, fps: u32) -> String {
    // Sozlash uchun: RD_ENC_OPTS="preset=p5 tune=low-latency" kabi qo'shimcha xususiyatlar
    let extra = std::env::var("RD_ENC_OPTS").unwrap_or_default();
    let base = encoder_base(factory, kbps, fps);
    if extra.trim().is_empty() { base } else { format!("{base} {extra}") }
}

/// Sifat va trafik uchun qo'shimcha sozlamalar, afzalligi bo'yicha. Sozlama shu GStreamer versiyasida bo'lmasa,
/// keyingisi sinaladi; oxirgisi bo'sh (asosiy sozlamalar). `RD_ENC_OPTS` berilgan bo'lsa, bu ro'yxat qo'llanmaydi.
/// Birinchi variantlar: doimiy bitrate (CBR) o'rniga sifat/VBR, foydalanuvchi bitrate'i esa yuqori chegara: oddiy
/// ish stolida trafik kamayadi, murakkab sahnada sifat avvalgidek.
fn quality_candidates(factory: &str, kbps: u32) -> Vec<String> {
    let avg = kbps * 4 / 5;
    match factory {
        "nvh264enc" => vec![
            format!("preset=p5 tune=low-latency rc-mode=vbr bitrate={avg} max-bitrate={kbps}"),
            "preset=p5 tune=low-latency".into(),
            "preset=p4".into(),
            String::new(),
        ],
        "vah264enc" => vec!["target-usage=2 rate-control=vbr target-percentage=80".into(), "target-usage=2".into(), String::new()],
        // pass=qual: doimiy sifat (CRF), bitrate esa VBV yuqori chegarasi
        "x264enc" => vec!["speed-preset=superfast pass=qual quantizer=20".into(), "speed-preset=superfast".into(), String::new()],
        _ => vec![String::new()],
    }
}

/// `name=enc`: kalit kadr so'rovi va kodlashni o'tkazib yuborish shu element orqali.
/// GOP juda uzun: IDR faqat so'ralganda (oqim ishonchli QUIC'da, kadr yo'qolmaydi).
fn encoder_base(factory: &str, kbps: u32, fps: u32) -> String {
    let long_gop = fps.max(1) * 600;
    match factory {
        "vah264enc" => format!("vah264enc name=enc bitrate={kbps} key-int-max=1024"),
        "nvh264enc" => format!("nvh264enc name=enc bitrate={kbps} gop-size=-1"),
        "x264enc" => format!(
            "x264enc name=enc tune=zerolatency speed-preset=ultrafast bitrate={kbps} key-int-max={long_gop} byte-stream=true"
        ),
        _ => format!("openh264enc name=enc bitrate={} gop-size={long_gop}", kbps * 1000),
    }
}

/// Kodlagich oldidagi rang o'girish zanjiri. `RD_GPU_CONVERT=1`: apparat kodlagichda o'girish GPU'da (tajriba).
fn convert_chains(encoder: &str, size_caps: &str) -> Vec<String> {
    let cpu = format!("videoscale ! video/x-raw{size_caps} ! videoconvert n-threads=4");
    let gpu = std::env::var("RD_GPU_CONVERT").is_ok_and(|v| v == "1");
    match (gpu, encoder) {
        (true, "nvh264enc") => {
            vec![format!("cudaupload ! cudaconvertscale ! video/x-raw(memory:CUDAMemory),format=NV12{size_caps}"), cpu]
        }
        (true, "vah264enc") => vec![format!("vapostproc ! video/x-raw(memory:VAMemory),format=NV12{size_caps}"), cpu],
        _ => vec![cpu],
    }
}

/// `fd`: portal bergan PipeWire ulanishi. Mutter backend'ida `None` (odatiy PipeWire ulanishi ishlatiladi).
pub fn start(
    fd: Option<OwnedFd>,
    node_id: u32,
    fps: u32,
    kbps: u32,
    max_height: u32,
    src_size: (i32, i32),
    ctl: Arc<VideoControl>,
) -> Result<Capture, CaptureError> {
    gst::init()
        .context("GStreamer ishga tushmadi")
        .map_err(|e| fail(video_reason::NO_CAPTURE, e))?;

    for need in ["pipewiresrc", "videoconvert", "videoscale", "videorate"] {
        if gst::ElementFactory::find(need).is_none() {
            return Err(fail(
                video_reason::NO_CAPTURE,
                anyhow!("`{need}` GStreamer elementi topilmadi (sudo apt install gstreamer1.0-pipewire gstreamer1.0-plugins-base)"),
            ));
        }
    }
    let forced = std::env::var("RD_ENCODER").ok();
    let encoder = ENCODERS
        .iter()
        .copied()
        .find(|e| forced.as_deref().map_or(true, |f| f == *e) && gst::ElementFactory::find(e).is_some())
        .ok_or_else(|| {
            fail(
                video_reason::NO_ENCODER,
                anyhow!("H.264 kodlagich topilmadi (sudo apt install gstreamer1.0-plugins-ugly yoki gstreamer1.0-plugins-bad)"),
            )
        })?;

    let size_caps = if max_height > 0 {
        // Nisbatni saqlash uchun kenglikni o'zimiz hisoblaymiz (juft son)
        let (sw, sh) = src_size;
        if sw > 0 && sh > 0 {
            let w = ((max_height as u64 * sw as u64 / sh as u64) as u32 + 1) & !1;
            format!(",width={w},height={}", max_height & !1)
        } else {
            format!(",height={},pixel-aspect-ratio=1/1", max_height & !1)
        }
    } else {
        String::new()
    };
    let fd_prop = fd.as_ref().map_or(String::new(), |f| format!("fd={} ", f.as_raw_fd()));
    let build = |convert: &str, enc: &str| {
        format!(
            "pipewiresrc name=src {fd_prop}path={node_id} do-timestamp=true ! videorate drop-only=true ! \
             video/x-raw,framerate={fps}/1 ! {convert} ! \
             queue leaky=downstream max-size-buffers=1 max-size-bytes=0 max-size-time=0 ! {enc} ! \
             video/x-h264,stream-format=byte-stream,alignment=au ! \
             appsink name=sink sync=false max-buffers=8 drop=false"
        )
    };
    let user_opts = std::env::var("RD_ENC_OPTS").map_or(false, |s| !s.trim().is_empty());
    let candidates = if user_opts { vec![String::new()] } else { quality_candidates(encoder, kbps) };
    let mut launched: Option<gst::Element> = None;
    let mut last_err: Option<String> = None;
    'outer: for convert in convert_chains(encoder, &size_caps) {
        for extra in &candidates {
            let enc = format!("{} {extra}", encoder_desc(encoder, kbps, fps));
            // Sifat sozlamasi yaroqsiz bo'lsa (xato bo'lishi shart), keyingi variantga o'tiladi.
            let flags = if extra.is_empty() { gst::ParseFlags::empty() } else { gst::ParseFlags::FATAL_ERRORS };
            match gst::parse::launch_full(&build(&convert, &enc), None, flags) {
                Ok(p) => {
                    if !extra.is_empty() {
                        eprintln!("[host] kodlagich sozlamasi: {encoder} {extra}");
                    }
                    if convert.contains("memory:") {
                        eprintln!("[host] rang o'girish GPU'da: {convert}");
                    }
                    launched = Some(p);
                    break 'outer;
                }
                Err(e) => last_err = Some(e.to_string()),
            }
        }
    }
    let pipeline = launched
        .ok_or_else(|| {
            fail(
                video_reason::NO_CAPTURE,
                anyhow!("pipeline yaratilmadi: {}", last_err.unwrap_or_default()),
            )
        })?
        .downcast::<gst::Pipeline>()
        .map_err(|_| fail(video_reason::NO_CAPTURE, anyhow!("pipeline turi noto'g'ri")))?;
    let sink = pipeline
        .by_name("sink")
        .ok_or_else(|| fail(video_reason::NO_CAPTURE, anyhow!("appsink topilmadi")))?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| fail(video_reason::NO_CAPTURE, anyhow!("appsink turi noto'g'ri")))?;
    let enc = pipeline
        .by_name("enc")
        .ok_or_else(|| fail(video_reason::NO_CAPTURE, anyhow!("kodlagich elementi topilmadi")))?;

    // Tarmoq yoki Viewer ulgurmayotgan bo'lsa, xom kadr kodlagichga kirmaydi (zanjir uzilmaydi, IDR kerak emas).
    if let Some(pad) = enc.static_pad("sink") {
        let ctl = ctl.clone();
        pad.add_probe(gst::PadProbeType::BUFFER, move |_, _| {
            if ctl.should_skip() {
                gst::PadProbeReturn::Drop
            } else {
                gst::PadProbeReturn::Ok
            }
        });
    }

    // Manba kadr o'lchami: masshtabli ekranlarda (masalan 133%) logik o'lchamdan farq qiladi.
    let source_size: Arc<Mutex<Option<(i32, i32)>>> = Arc::new(Mutex::new(None));
    if let Some(pad) = pipeline.by_name("src").and_then(|e| e.static_pad("src")) {
        let size = source_size.clone();
        pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
            if let Some(gst::PadProbeData::Event(ref ev)) = info.data {
                if let gst::EventView::Caps(c) = ev.view() {
                    if let Some(s) = c.caps().structure(0) {
                        if let (Ok(w), Ok(h)) = (s.get::<i32>("width"), s.get::<i32>("height")) {
                            *size.lock().unwrap() = Some((w, h));
                        }
                    }
                }
            }
            gst::PadProbeReturn::Ok
        });
    }

    // Kanal to'lsa (yuboruvchi tarmoqni kutyapti) appsink oqimi kutadi: bosim kodlagichgacha boradi va
    // xom kadrlar `queue leaky` da tashlanadi. Kodlangan kadr tashlanmaydi. To'xtatishda (`stop`) kutish uziladi,
    // aks holda `set_state(Null)` shu oqimni kutib qolardi.
    let (tx, rx) = mpsc::channel::<Frame>(8);
    let stop = Arc::new(AtomicBool::new(false));
    let sink_stop = stop.clone();
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let frame = Frame {
                    keyframe: !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT),
                    pts_us: buffer.pts().map(|t| t.useconds()).unwrap_or(0),
                    data: map.as_slice().to_vec(),
                };
                let mut frame = frame;
                loop {
                    match tx.try_send(frame) {
                        Ok(()) => return Ok(gst::FlowSuccess::Ok),
                        Err(mpsc::error::TrySendError::Closed(_)) => return Err(gst::FlowError::Flushing),
                        Err(mpsc::error::TrySendError::Full(f)) => {
                            if sink_stop.load(Ordering::SeqCst) {
                                return Err(gst::FlowError::Flushing);
                            }
                            frame = f;
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                    }
                }
            })
            .build(),
    );

    let bus = pipeline
        .bus()
        .ok_or_else(|| fail(video_reason::NO_CAPTURE, anyhow!("bus yo'q")))?;
    let (err_tx, err_rx) = mpsc::unbounded_channel::<String>();
    {
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let msg = bus.timed_pop_filtered(
                    gst::ClockTime::from_mseconds(200),
                    &[gst::MessageType::Error, gst::MessageType::Eos],
                );
                if let Some(msg) = msg {
                    match msg.view() {
                        gst::MessageView::Error(e) => {
                            let _ = err_tx.send(format!("{} ({:?})", e.error(), e.debug()));
                            break;
                        }
                        gst::MessageView::Eos(_) => {
                            let _ = err_tx.send("oqim tugadi (EOS)".to_string());
                            break;
                        }
                        _ => {}
                    }
                }
            }
        });
    }

    pipeline
        .set_state(gst::State::Playing)
        .context("pipeline ishga tushmadi")
        .map_err(|e| fail(video_reason::NO_CAPTURE, e))?;

    Ok(Capture {
        pipeline,
        enc,
        frames: rx,
        errors: err_rx,
        encoder,
        source_size,
        stop,
        _fd: fd,
    })
}
