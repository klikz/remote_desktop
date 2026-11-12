//! Ekran olish va H.264 kodlash oqimi (alohida std thread). Kodlangan kadrlar tokio kanaliga yuboriladi.
//!
//! Trafik va CPU uchun:
//! - Ekran o'zgarmasa kadr kodlanmaydi va yuborilmaydi (oldingi kadr bilan solishtiriladi); uzoq vaqt o'zgarmasa
//!   ekran yarim tezlikda tekshiriladi.
//! - Tarmoq yoki Viewer ulgurmasa (`VideoControl::should_skip`) yoki yuborish navbati to'la bo'lsa, kadr **kodlanmaydi**:
//!   H.264 zanjiri uzilmaydi, shuning uchun katta kalit kadr (IDR) kerak emas.
//! - IDR faqat boshida, o'lcham o'zgarganda va Viewer so'raganda.

use crate::capture::{scale_bgra, Grabber};
use anyhow::{anyhow, Result};
use openh264::{
    encoder::{Encoder, EncoderConfig, FrameType, RateControlMode, UsageType},
    formats::{BgraSliceU8, YUVBuffer},
    OpenH264API,
};
use rdcore::pacing::VideoControl;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc::{Sender, UnboundedSender};

pub struct EncodedFrame {
    pub keyframe: bool,
    pub pts_us: u64,
    pub data: Vec<u8>,
}

/// Soniyalik host diagnostikasi (`RD_DIAG=1`): qotganda sabab shu qatorlardan ko'rinadi.
/// `skip_ctl` = Viewer/tarmoq ulgurmayapti, `skip_chan` = yuborish navbati to'la, `unchanged` = ekran o'zgarmagan,
/// `enc_skip` = kodlagich kadrni o'tkazdi.
struct Diag {
    on: bool,
    since: Instant,
    loops: u32,
    grabbed: u32,
    unchanged: u32,
    skip_ctl: u32,
    skip_chan: u32,
    enc_skip: u32,
    sent: u32,
    keyframes: u32,
    encode_ms_max: u64,
}

impl Diag {
    fn new() -> Self {
        Diag {
            on: std::env::var_os("RD_DIAG").is_some(),
            since: Instant::now(),
            loops: 0,
            grabbed: 0,
            unchanged: 0,
            skip_ctl: 0,
            skip_chan: 0,
            enc_skip: 0,
            sent: 0,
            keyframes: 0,
            encode_ms_max: 0,
        }
    }

    fn report(&mut self, ctl: &VideoControl, force_key: bool) {
        if !self.on || self.since.elapsed() < Duration::from_secs(1) {
            return;
        }
        eprintln!(
            "[diag] host: loop {} grab {} oʻzgarmagan {} skip(ctl {}, kanal {}, kodlagich {}) yuborildi {} (kalit {}) enc max {} ms | in_flight {}/{} viewer_queue {} force_key {}",
            self.loops,
            self.grabbed,
            self.unchanged,
            self.skip_ctl,
            self.skip_chan,
            self.enc_skip,
            self.sent,
            self.keyframes,
            self.encode_ms_max,
            ctl.in_flight(),
            ctl.allowed_in_flight(),
            ctl.viewer_queue(),
            force_key,
        );
        *self = Diag { on: true, since: Instant::now(), ..Diag::new() };
    }
}

fn target_size(sw: usize, sh: usize, max_height: u32) -> (usize, usize) {
    let mh = max_height as usize;
    if mh > 0 && sh > mh {
        let th = (mh & !1).max(2);
        let tw = ((sw * th / sh) & !1).max(2);
        (tw, th)
    } else {
        ((sw & !1).max(2), (sh & !1).max(2))
    }
}

fn make_encoder(fps: u32, kbps: u32) -> Result<Encoder> {
    let cfg = EncoderConfig::new()
        .set_bitrate_bps(kbps.saturating_mul(1000))
        .max_frame_rate(fps as f32)
        .rate_control_mode(RateControlMode::Bitrate)
        .usage_type(UsageType::ScreenContentRealTime)
        // Bitrate chegarasidan oshsa kodlagich kadrni o'zi o'tkazib yuboradi (zanjir uzilmaydi)
        .enable_skip_frame(true);
    Encoder::with_api_config(OpenH264API::from_source(), cfg).map_err(|e| anyhow!("H.264 kodlagich yaratilmadi: {e}"))
}

/// Ekran DC sini qayta yaratish shuncha vaqt muvaffaqiyatsiz bo'lsa, video xato bilan to'xtaydi.
const REGRAB_WINDOW: Duration = Duration::from_secs(10);
/// Ketma-ket shuncha kodlash xatosidan keyin video xato bilan to'xtaydi.
const MAX_ENCODE_ERRORS: u32 = 5;

/// Ekran olish va kodlashni ishga tushiradi. `stop` o'rnatilsa yoki kanal yopilsa thread tugaydi.
pub fn spawn(
    fps: u32,
    kbps: u32,
    max_height: u32,
    tx: Sender<EncodedFrame>,
    stop: Arc<AtomicBool>,
    errors: UnboundedSender<String>,
    ctl: Arc<VideoControl>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        if let Err(e) = run(fps, kbps, max_height, tx, &stop, &ctl) {
            let _ = errors.send(format!("{e:#}"));
        }
    })
}

fn run(fps: u32, kbps: u32, max_height: u32, tx: Sender<EncodedFrame>, stop: &AtomicBool, ctl: &VideoControl) -> Result<()> {
    let fps = fps.max(1);
    let interval = Duration::from_micros(1_000_000 / u64::from(fps));
    let started = Instant::now();

    crate::desktop::attach_input_desktop();
    let mut grab = Grabber::new()?;
    let (mut tw, mut th) = target_size(grab.w, grab.h, max_height);
    let mut enc = make_encoder(fps, kbps)?;
    let mut yuv = YUVBuffer::new(tw, th);
    let mut scaled: Vec<u8> = Vec::new();
    // Oldingi olingan kadr: o'zgarmagan ekranni kodlamaslik uchun
    let mut prev: Vec<u8> = Vec::new();
    // Ketma-ket o'zgarmagan kadrlar soni (1 soniyadan oshsa ekran yarim tezlikda tekshiriladi)
    let mut idle: u32 = 0;
    let mut force_key = true;
    let mut diag = Diag::new();
    // Ekran DC sini qayta yaratish kerak (desktop yoki o'lcham o'zgardi); muvaffaqiyatsiz urinish boshlangan vaqt
    let mut regrab = false;
    let mut regrab_failing: Option<Instant> = None;
    // Ketma-ket kodlash xatolari
    let mut enc_errors: u32 = 0;

    while !stop.load(Ordering::Relaxed) {
        let t0 = Instant::now();

        // Desktop almashdimi (qulf ekrani <-> ish stoli)? Ekran DC si yangi desktopga qayta yaratiladi.
        let desktop_changed = crate::desktop::attach_throttled();
        if desktop_changed || grab.size_changed() {
            regrab = true;
        }
        if regrab {
            // Desktop almashayotgan yoki ekran ruxsati o'zgarayotgan payt DC vaqtincha yaratilmasligi mumkin:
            // video (va u bilan butun sessiya) to'xtamaydi, bir necha soniya qayta uriniladi.
            match Grabber::new() {
                Ok(g) => {
                    grab = g;
                    regrab = false;
                    regrab_failing = None;
                }
                Err(e) => {
                    let since = *regrab_failing.get_or_insert_with(Instant::now);
                    if since.elapsed() > REGRAB_WINDOW {
                        return Err(e);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            }
            // Ekran o'lchami o'zgardi: yangi kadr o'lchamiga mos kodlagich va kalit kadr
            (tw, th) = target_size(grab.w, grab.h, max_height);
            enc = make_encoder(fps, kbps)?;
            yuv = YUVBuffer::new(tw, th);
            prev.clear();
            force_key = true;
        }
        if ctl.take_keyframe_request() {
            force_key = true;
        }

        diag.loops += 1;
        'frame: {
            // Tarmoq/Viewer ulgurmayapti yoki yuborish navbati to'la: shu kadr kodlanmaydi
            if ctl.should_skip() {
                diag.skip_ctl += 1;
                break 'frame;
            }
            if tx.capacity() == 0 {
                diag.skip_chan += 1;
                break 'frame;
            }
            let (sw, sh) = (grab.w, grab.h);
            let frame = grab.grab();
            diag.grabbed += 1;
            if !force_key && frame == prev.as_slice() {
                idle = idle.saturating_add(1);
                diag.unchanged += 1;
                break 'frame;
            }
            idle = 0;
            prev.clear();
            prev.extend_from_slice(frame);

            if tw == sw && th == sh {
                // Masshtab kerak emas: nusxa olmasdan to'g'ridan-to'g'ri
                yuv.read_rgb(BgraSliceU8::new(&prev, (tw, th)));
            } else {
                scale_bgra(&prev, sw, sh, tw, th, &mut scaled);
                yuv.read_rgb(BgraSliceU8::new(&scaled, (tw, th)));
            }

            if force_key {
                enc.force_intra_frame();
            }
            let te = Instant::now();
            let bs = match enc.encode(&yuv) {
                Ok(bs) => bs,
                Err(e) => {
                    // Bitta xato sessiyani tugatmaydi: kodlagich qayta yaratiladi va oqim kalit kadrdan davom etadi
                    enc_errors += 1;
                    if enc_errors >= MAX_ENCODE_ERRORS {
                        return Err(anyhow!("kodlash xatosi: {e}"));
                    }
                    enc = make_encoder(fps, kbps)?;
                    force_key = true;
                    break 'frame;
                }
            };
            enc_errors = 0;
            diag.encode_ms_max = diag.encode_ms_max.max(te.elapsed().as_millis() as u64);
            let ft = bs.frame_type();
            if matches!(ft, FrameType::Skip | FrameType::Invalid) {
                // IDR so'ralgan bo'lsa u yo'qolmasin: keyingi kadrda qayta uriniladi (force_key saqlanadi)
                diag.enc_skip += 1;
                break 'frame;
            }
            let data = bs.to_vec();
            if data.is_empty() {
                break 'frame;
            }
            let keyframe = matches!(ft, FrameType::IDR | FrameType::I);
            if keyframe {
                force_key = false;
                diag.keyframes += 1;
            }
            diag.sent += 1;
            let f = EncodedFrame { keyframe, pts_us: started.elapsed().as_micros() as u64, data };
            // Joy borligi yuqorida tekshirilgan (yagona yuboruvchi): kutish bo'lmaydi
            if tx.blocking_send(f).is_err() {
                return Ok(());
            }
        }

        diag.report(ctl, force_key);

        // Ekran uzoq vaqt o'zgarmasa (1 s dan ko'p) yarim tezlikda tekshiramiz: CPU kamroq, birinchi o'zgarish
        // eng ko'pi bilan ikki kadr oralig'ida seziladi.
        let period = if idle > fps { interval * 2 } else { interval };
        if let Some(rest) = period.checked_sub(t0.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    Ok(())
}
