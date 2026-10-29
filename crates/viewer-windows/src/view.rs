//! Video'ni dekodlash (openh264) va oynada ko'rsatish (minifb).
//!
//! Oqim: tokio vazifasi kadrlarni qabul qiladi -> dekoder oqimi (std thread) dekodlaydi ->
//! "oxirgi kadr" uyasi -> asosiy oqimdagi oyna uni chizadi. Hamma narsada eng yangi kadr ustun.

use anyhow::{Context, Result};
use minifb::{Key, ScaleMode, Window, WindowOptions};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
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

pub async fn run(session: &mut Session, fps: u16, kbps: u32, max_height: u16, monitor: u8) -> Result<()> {
    println!("Video so'ralmoqda: {fps} FPS, {kbps} kbit/s, balandlik {max_height}...");
    let mut stream = session.request_video(fps, kbps, max_height, monitor).await?;
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

/// Kadrni oyna o'lchamiga nisbatni saqlab sig'diradi. Kichraytirishda "quti" filtri (o'rtacha),
/// kattalashtirishda eng yaqin piksel. Qolgan joy qora.
fn fit_scale(src: &Picture, ww: usize, wh: usize, out: &mut Vec<u32>) {
    out.clear();
    out.resize(ww * wh, 0);
    if ww == 0 || wh == 0 || src.w == 0 || src.h == 0 {
        return;
    }
    let scale = (ww as f64 / src.w as f64).min(wh as f64 / src.h as f64);
    let dw = ((src.w as f64 * scale) as usize).clamp(1, ww);
    let dh = ((src.h as f64 * scale) as usize).clamp(1, wh);
    let (ox, oy) = ((ww - dw) / 2, (wh - dh) / 2);
    for y in 0..dh {
        let sy0 = y * src.h / dh;
        let sy1 = ((y + 1) * src.h / dh).clamp(sy0 + 1, src.h);
        for x in 0..dw {
            let sx0 = x * src.w / dw;
            let sx1 = ((x + 1) * src.w / dw).clamp(sx0 + 1, src.w);
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for sy in sy0..sy1 {
                for p in &src.pixels[sy * src.w + sx0..sy * src.w + sx1] {
                    r += (p >> 16) & 0xff;
                    g += (p >> 8) & 0xff;
                    b += p & 0xff;
                }
            }
            let n = ((sy1 - sy0) * (sx1 - sx0)) as u32;
            out[(oy + y) * ww + ox + x] = ((r / n) << 16) | ((g / n) << 8) | (b / n);
        }
    }
}

fn window_loop(slot: Slot, stop: Arc<AtomicBool>) -> Result<()> {
    let mut window: Option<Window> = None;
    let mut shown: Option<Picture> = None;
    let mut dirty = false;
    let mut last_size = (0usize, 0usize);
    let mut buf: Vec<u32> = Vec::new();

    while !stop.load(Ordering::Relaxed) {
        if let Some(p) = slot.lock().unwrap().take() {
            shown = Some(p);
            dirty = true;
        }
        let Some(p) = &shown else {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        if window.is_none() {
            let scale = (1920.0 / p.w as f64).min(1000.0 / p.h as f64).min(1.0);
            let (ww, wh) = (((p.w as f64) * scale) as usize, ((p.h as f64) * scale) as usize);
            let mut w = Window::new(
                "Remote Desktop",
                ww.max(160),
                wh.max(90),
                WindowOptions { resize: true, scale_mode: ScaleMode::Stretch, ..WindowOptions::default() },
            )
            .context("oyna yaratilmadi")?;
            w.set_target_fps(120);
            window = Some(w);
        }
        let w = window.as_mut().unwrap();
        if !w.is_open() || w.is_key_down(Key::Escape) {
            break;
        }
        let size = w.get_size();
        if size.0 == 0 || size.1 == 0 {
            w.update();
            continue;
        }
        if dirty || size != last_size {
            fit_scale(p, size.0, size.1, &mut buf);
            last_size = size;
            dirty = false;
            w.update_with_buffer(&buf, size.0, size.1).context("oyna yangilanmadi")?;
        } else {
            w.update();
        }
    }
    Ok(())
}
