//! Video'ni dekodlash (openh264) va oynada ko'rsatish (minifb).
//!
//! Oqim: tokio vazifasi kadrlarni qabul qiladi -> dekoder oqimi (std thread) dekodlaydi ->
//! "oxirgi kadr" uyasi -> asosiy oqimdagi oyna uni chizadi. Hamma narsada eng yangi kadr ustun.

use anyhow::{Context, Result};
use minifb::{Key, ScaleMode, Window, WindowOptions};
use openh264::decoder::Decoder;
use rdcore::client::Session;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{sync_channel, TrySendError},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

struct Picture {
    w: usize,
    h: usize,
    pixels: Vec<u32>,
}

type Slot = Arc<Mutex<Option<Picture>>>;

pub async fn run(session: &mut Session, fps: u16, kbps: u32, max_height: u16) -> Result<()> {
    println!("Video so'ralmoqda: {fps} FPS, {kbps} kbit/s, balandlik {max_height}...");
    let mut stream = session.request_video(fps, kbps, max_height).await?;
    println!("Host videoni boshladi (kodek {}). Chiqish: Esc yoki oynani yoping.", stream.codec);

    let stop = Arc::new(AtomicBool::new(false));
    let slot: Slot = Arc::new(Mutex::new(None));
    let (tx, rx) = sync_channel::<(bool, Vec<u8>)>(8);

    // Dekoder oqimi
    let dec_slot = slot.clone();
    let dec_stop = stop.clone();
    let decoder_thread = std::thread::spawn(move || {
        let mut decoder = match Decoder::new() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Dekoder yaratilmadi: {e}");
                dec_stop.store(true, Ordering::Relaxed);
                return;
            }
        };
        let mut rgb: Vec<u8> = Vec::new();
        while let Ok((_key, data)) = rx.recv() {
            match decoder.decode(&data) {
                Ok(Some(yuv)) => {
                    let (w, h) = yuv.dimensions();
                    rgb.resize(w * h * 3, 0);
                    yuv.write_rgb8(&mut rgb);
                    let pixels: Vec<u32> = rgb
                        .chunks_exact(3)
                        .map(|p| (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2]))
                        .collect();
                    *dec_slot.lock().unwrap() = Some(Picture { w, h, pixels });
                }
                Ok(None) => {}
                Err(e) => eprintln!("dekodlash xatosi: {e}"),
            }
        }
    });

    // Qabul qilish vazifasi
    let recv_stop = stop.clone();
    let recv_task = tokio::spawn(async move {
        let mut need_key = false;
        let (mut frames, mut bytes) = (0u64, 0u64);
        let mut last = Instant::now();
        while !recv_stop.load(Ordering::Relaxed) {
            let frame = match tokio::time::timeout(Duration::from_millis(250), stream.next_frame()).await {
                Err(_) => continue,
                Ok(Ok(Some(f))) => f,
                Ok(Ok(None)) => {
                    println!("Host video oqimini tugatdi.");
                    break;
                }
                Ok(Err(e)) => {
                    eprintln!("video xatosi: {e:#}");
                    break;
                }
            };
            frames += 1;
            bytes += frame.data.len() as u64;
            if need_key && !frame.keyframe {
                continue; // dekoder orqada qoldi: keyingi kalit kadrgacha tashlab ketamiz
            }
            match tx.try_send((frame.keyframe, frame.data)) {
                Ok(()) => need_key = false,
                Err(TrySendError::Full(_)) => need_key = true,
                Err(TrySendError::Disconnected(_)) => break,
            }
            if last.elapsed() >= Duration::from_secs(5) {
                let s = last.elapsed().as_secs_f64();
                println!("  qabul: {:.1} FPS, {:.0} kbit/s", frames as f64 / s, bytes as f64 * 8.0 / 1000.0 / s);
                frames = 0;
                bytes = 0;
                last = Instant::now();
            }
        }
        recv_stop.store(true, Ordering::Relaxed);
    });

    // Oyna asosiy oqimda
    let win_stop = stop.clone();
    tokio::task::block_in_place(move || window_loop(slot, win_stop))?;

    stop.store(true, Ordering::Relaxed);
    let _ = recv_task.await;
    let _ = decoder_thread.join();
    Ok(())
}

fn window_loop(slot: Slot, stop: Arc<AtomicBool>) -> Result<()> {
    let mut window: Option<Window> = None;
    let mut size = (0usize, 0usize);
    let mut shown: Option<Picture> = None;

    while !stop.load(Ordering::Relaxed) {
        if let Some(p) = slot.lock().unwrap().take() {
            shown = Some(p);
        }
        if let Some(p) = &shown {
            if window.is_none() || size != (p.w, p.h) {
                let scale = (1600.0 / p.w as f64).min(900.0 / p.h as f64).min(1.0);
                let (ww, wh) = (((p.w as f64) * scale) as usize, ((p.h as f64) * scale) as usize);
                let mut w = Window::new(
                    "Remote Desktop",
                    ww.max(160),
                    wh.max(90),
                    WindowOptions {
                        resize: true,
                        scale_mode: ScaleMode::AspectRatioStretch,
                        ..WindowOptions::default()
                    },
                )
                .context("oyna yaratilmadi")?;
                w.set_target_fps(120);
                window = Some(w);
                size = (p.w, p.h);
            }
        }
        match (&mut window, &shown) {
            (Some(w), Some(p)) => {
                if !w.is_open() || w.is_key_down(Key::Escape) {
                    break;
                }
                w.update_with_buffer(&p.pixels, p.w, p.h).context("oyna yangilanmadi")?;
            }
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    Ok(())
}
