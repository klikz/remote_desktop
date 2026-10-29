//! PipeWire oqimidan kadr olish va H.264 ga kodlash (GStreamer orqali). Faqat sinov uchun.

use anyhow::{anyhow, bail, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::{
    fs::File,
    io::Write,
    os::fd::{AsRawFd, OwnedFd},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

/// Kodlagichlar ustuvorlik tartibida: apparat tezlatish birinchi.
const ENCODERS: [&str; 4] = ["vah264enc", "nvh264enc", "x264enc", "openh264enc"];

fn encoder_desc(factory: &str, kbps: u32, gop: u32) -> String {
    match factory {
        "vah264enc" => format!("vah264enc bitrate={kbps} key-int-max={gop}"),
        "nvh264enc" => format!("nvh264enc bitrate={kbps} gop-size={gop}"),
        "x264enc" => format!(
            "x264enc tune=zerolatency speed-preset=ultrafast bitrate={kbps} key-int-max={gop} byte-stream=true"
        ),
        _ => format!("openh264enc bitrate={} gop-size={gop}", kbps * 1000),
    }
}

#[derive(Default)]
struct Stats {
    frames: AtomicU64,
    keyframes: AtomicU64,
    bytes: AtomicU64,
    first_frame_ms: AtomicU64,
    printed_caps: AtomicBool,
}

pub fn run(
    fd: OwnedFd,
    node_id: u32,
    seconds: u64,
    fps: u32,
    kbps: u32,
    out: Option<PathBuf>,
) -> Result<()> {
    gst::init().context("GStreamer ishga tushmadi")?;

    for need in ["pipewiresrc", "videoconvert", "videorate"] {
        if gst::ElementFactory::find(need).is_none() {
            bail!("`{need}` GStreamer elementi topilmadi. O'rnating: sudo apt install gstreamer1.0-pipewire gstreamer1.0-plugins-base");
        }
    }

    println!("Kodlagichlar:");
    for e in ENCODERS {
        let found = gst::ElementFactory::find(e).is_some();
        println!("  {:<12} {}", e, if found { "bor" } else { "yo'q" });
    }
    let forced = std::env::var("RD_ENCODER").ok();
    let encoder = ENCODERS
        .iter()
        .copied()
        .find(|e| forced.as_deref().map_or(true, |f| f == *e) && gst::ElementFactory::find(e).is_some())
        .ok_or_else(|| {
            anyhow!("H.264 kodlagich topilmadi. O'rnating: sudo apt install gstreamer1.0-plugins-ugly (x264enc) yoki gstreamer1.0-plugins-bad (vah264enc)")
        })?;
    println!("Tanlangan kodlagich: {encoder}");

    let gop = fps * 2;
    let desc = format!(
        "pipewiresrc fd={} path={} do-timestamp=true ! videoconvert ! videorate ! \
         video/x-raw,framerate={}/1 ! {} ! video/x-h264,stream-format=byte-stream,alignment=au ! \
         appsink name=sink sync=false max-buffers=4 drop=true",
        fd.as_raw_fd(),
        node_id,
        fps,
        encoder_desc(encoder, kbps, gop)
    );
    println!("Pipeline: {desc}");

    let pipeline = gst::parse::launch(&desc)
        .context("pipeline yaratilmadi")?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("pipeline turi noto'g'ri"))?;
    let sink = pipeline
        .by_name("sink")
        .ok_or_else(|| anyhow!("appsink topilmadi"))?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| anyhow!("appsink turi noto'g'ri"))?;

    let stats = Arc::new(Stats::default());
    stats.first_frame_ms.store(u64::MAX, Ordering::SeqCst);
    let file = match &out {
        Some(p) => Some(Mutex::new(File::create(p).with_context(|| format!("{} yaratilmadi", p.display()))?)),
        None => None,
    };
    let started = Instant::now();

    let cb_stats = stats.clone();
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;

                if !cb_stats.printed_caps.swap(true, Ordering::SeqCst) {
                    if let Some(s) = sample.caps().and_then(|c| c.structure(0)) {
                        let w = s.get::<i32>("width").unwrap_or(0);
                        let h = s.get::<i32>("height").unwrap_or(0);
                        println!("Birinchi kadr: kodlangan video {w}x{h}");
                    }
                }
                if cb_stats.frames.fetch_add(1, Ordering::SeqCst) == 0 {
                    cb_stats
                        .first_frame_ms
                        .store(started.elapsed().as_millis() as u64, Ordering::SeqCst);
                }
                if !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT) {
                    cb_stats.keyframes.fetch_add(1, Ordering::SeqCst);
                }
                cb_stats.bytes.fetch_add(map.len() as u64, Ordering::SeqCst);
                if let Some(f) = &file {
                    let _ = f.lock().unwrap_or_else(|e| e.into_inner()).write_all(map.as_slice());
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    pipeline
        .set_state(gst::State::Playing)
        .context("pipeline ishga tushmadi")?;
    println!("Yozib olish boshlandi: {seconds} soniya (ekranda harakat qiling, aks holda kadr kam bo'ladi)...");

    let bus = pipeline.bus().ok_or_else(|| anyhow!("bus yo'q"))?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if let Some(msg) = bus.timed_pop_filtered(
            gst::ClockTime::from_mseconds(200),
            &[gst::MessageType::Error, gst::MessageType::Eos],
        ) {
            match msg.view() {
                gst::MessageView::Error(err) => {
                    let _ = pipeline.set_state(gst::State::Null);
                    bail!("GStreamer xatosi: {} ({:?})", err.error(), err.debug());
                }
                gst::MessageView::Eos(_) => break,
                _ => {}
            }
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let _ = pipeline.set_state(gst::State::Null);

    let frames = stats.frames.load(Ordering::SeqCst);
    let keys = stats.keyframes.load(Ordering::SeqCst);
    let bytes = stats.bytes.load(Ordering::SeqCst);
    let first = stats.first_frame_ms.load(Ordering::SeqCst);
    println!("--- Natija ---");
    println!("Kadrlar: {frames} (kalit kadrlar: {keys})");
    if frames == 0 {
        bail!("birorta ham kadr kelmadi (oqim bo'sh yoki format kelishilmadi)");
    }
    println!("O'rtacha FPS: {:.1}", frames as f64 / elapsed);
    println!("O'rtacha bitrate: {:.0} kbit/s", bytes as f64 * 8.0 / 1000.0 / elapsed);
    println!("Birinchi kadrgacha: {first} ms");
    if let Some(p) = out {
        println!("H.264 fayl saqlandi: {} (ochish: ffplay -f h264 fayl yoki VLC)", p.display());
    }
    Ok(())
}
