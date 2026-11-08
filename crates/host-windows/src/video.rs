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

    while !stop.load(Ordering::Relaxed) {
        let t0 = Instant::now();

        // Desktop almashdimi (qulf ekrani <-> ish stoli)? Ekran DC si yangi desktopga qayta yaratiladi.
        let desktop_changed = crate::desktop::attach_throttled();
        if desktop_changed || grab.size_changed() {
            // Ekran o'lchami o'zgardi: yangi kadr o'lchamiga mos kodlagich va kalit kadr
            grab = Grabber::new()?;
            (tw, th) = target_size(grab.w, grab.h, max_height);
            enc = make_encoder(fps, kbps)?;
            yuv = YUVBuffer::new(tw, th);
            prev.clear();
            force_key = true;
        }
        if ctl.take_keyframe_request() {
            force_key = true;
        }

        'frame: {
            // Tarmoq/Viewer ulgurmayapti yoki yuborish navbati to'la: shu kadr kodlanmaydi
            if ctl.should_skip() || tx.capacity() == 0 {
                break 'frame;
            }
            let (sw, sh) = (grab.w, grab.h);
            let frame = grab.grab();
            if !force_key && frame == prev.as_slice() {
                idle = idle.saturating_add(1);
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
                force_key = false;
            }
            let bs = enc.encode(&yuv).map_err(|e| anyhow!("kodlash xatosi: {e}"))?;
            let ft = bs.frame_type();
            if matches!(ft, FrameType::Skip | FrameType::Invalid) {
                break 'frame;
            }
            let data = bs.to_vec();
            if data.is_empty() {
                break 'frame;
            }
            let keyframe = matches!(ft, FrameType::IDR | FrameType::I);
            let f = EncodedFrame { keyframe, pts_us: started.elapsed().as_micros() as u64, data };
            // Joy borligi yuqorida tekshirilgan (yagona yuboruvchi): kutish bo'lmaydi
            if tx.blocking_send(f).is_err() {
                return Ok(());
            }
        }

        // Ekran uzoq vaqt o'zgarmasa (1 s dan ko'p) yarim tezlikda tekshiramiz: CPU kamroq, birinchi o'zgarish
        // eng ko'pi bilan ikki kadr oralig'ida seziladi.
        let period = if idle > fps { interval * 2 } else { interval };
        if let Some(rest) = period.checked_sub(t0.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    Ok(())
}
