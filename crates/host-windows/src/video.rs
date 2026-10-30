//! Ekran olish va H.264 kodlash oqimi (alohida std thread). Kodlangan kadrlar tokio kanaliga yuboriladi.

use crate::capture::{scale_bgra, Grabber};
use anyhow::{anyhow, Result};
use openh264::{
    encoder::{Encoder, EncoderConfig, FrameType, RateControlMode, UsageType},
    formats::{BgraSliceU8, YUVBuffer},
    OpenH264API,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc::{error::TrySendError, Sender, UnboundedSender};

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
        .enable_skip_frame(false);
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
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        if let Err(e) = run(fps, kbps, max_height, tx, &stop) {
            let _ = errors.send(format!("{e:#}"));
        }
    })
}

fn run(fps: u32, kbps: u32, max_height: u32, tx: Sender<EncodedFrame>, stop: &AtomicBool) -> Result<()> {
    let interval = Duration::from_micros(1_000_000 / u64::from(fps.max(1)));
    let started = Instant::now();

    let mut grab = Grabber::new()?;
    let (mut tw, mut th) = target_size(grab.w, grab.h, max_height);
    let mut enc = make_encoder(fps, kbps)?;
    let mut yuv = YUVBuffer::new(tw, th);
    let mut scaled: Vec<u8> = Vec::new();

    let mut n: u64 = 0;
    let mut force_key = true;
    let mut dropping = false;

    while !stop.load(Ordering::Relaxed) {
        let t0 = Instant::now();

        if grab.size_changed() {
            // Ekran o'lchami o'zgardi: yangi kadr o'lchamiga mos kodlagich va kalit kadr
            grab = Grabber::new()?;
            (tw, th) = target_size(grab.w, grab.h, max_height);
            enc = make_encoder(fps, kbps)?;
            yuv = YUVBuffer::new(tw, th);
            force_key = true;
        }

        let (sw, sh) = (grab.w, grab.h);
        let frame = grab.grab();
        scale_bgra(frame, sw, sh, tw, th, &mut scaled);
        yuv.read_rgb(BgraSliceU8::new(&scaled, (tw, th)));

        if force_key || n % u64::from(fps.max(1)) == 0 {
            enc.force_intra_frame();
            force_key = false;
        }
        let bs = enc.encode(&yuv).map_err(|e| anyhow!("kodlash xatosi: {e}"))?;
        let ft = bs.frame_type();
        if !matches!(ft, FrameType::Skip | FrameType::Invalid) {
            let data = bs.to_vec();
            if !data.is_empty() {
                let keyframe = matches!(ft, FrameType::IDR | FrameType::I);
                // Viewer sekin bo'lsa keyingi kalit kadrgacha kadrlarni tashlaymiz (oqim buzilmasin)
                if !(dropping && !keyframe) {
                    let f = EncodedFrame { keyframe, pts_us: started.elapsed().as_micros() as u64, data };
                    match tx.try_send(f) {
                        Ok(()) => dropping = false,
                        Err(TrySendError::Full(_)) => {
                            dropping = true;
                            force_key = true;
                        }
                        Err(TrySendError::Closed(_)) => return Ok(()),
                    }
                }
            }
        }
        n += 1;

        if let Some(rest) = interval.checked_sub(t0.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    Ok(())
}
