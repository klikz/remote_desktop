//! Video'ni dekodlash (openh264) va oynada ko'rsatish (minifb).
//!
//! Oqim: tokio vazifasi kadrlarni qabul qiladi -> dekoder oqimi (std thread) dekodlaydi ->
//! "oxirgi kadr" uyasi -> asosiy oqimdagi oyna uni chizadi. Hamma narsada eng yangi kadr ustun.

use anyhow::{Context, Result};
use crate::keymap;
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, ScaleMode, Window, WindowOptions};
use protocol::InputEvent;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
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

pub async fn run(session: &mut Session, fps: u16, kbps: u32, max_height: u16, monitor: u8, input: bool) -> Result<()> {
    println!("Video so'ralmoqda: {fps} FPS, {kbps} kbit/s, balandlik {max_height}...");
    let mut stream = session.request_video(fps, kbps, max_height, monitor).await?;
    println!("Host videoni boshladi (kodek {}). Chiqish: oynani yoping yoki Ctrl+Alt+Q.", stream.codec);
    if input {
        println!("Boshqaruv yoqilgan: sichqoncha va klaviatura Host'ga uzatiladi (faqat ko'rish: --no-input).");
    }

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

    // Oyna alohida oqimda; kiritish hodisalari shu yerda Host'ga yuboriladi
    let (itx, mut irx) = unbounded_channel::<InputEvent>();
    let win_stop = stop.clone();
    let win_slot = slot.clone();
    let window_thread = std::thread::spawn(move || {
        let r = window_loop(win_slot, win_stop.clone(), if input { Some(itx) } else { None });
        win_stop.store(true, Ordering::Relaxed);
        r
    });
    loop {
        tokio::select! {
            ev = irx.recv() => match ev {
                Some(ev) => {
                    if session.send_input(ev).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    match window_thread.join() {
        Ok(Err(e)) => eprintln!("oyna xatosi: {e:#}"),
        Err(_) => eprintln!("oyna oqimi to'xtadi"),
        _ => {}
    }
    let _ = recv_task.await;
    let _ = decoder_thread.join();
    Ok(())
}

/// Kadrni oyna o'lchamiga nisbatni saqlab sig'diradi. Kichraytirishda "quti" filtri (o'rtacha),
/// kattalashtirishda eng yaqin piksel. Qolgan joy qora.
fn fit_scale(src: &Picture, ww: usize, wh: usize, out: &mut Vec<u32>) -> (usize, usize, usize, usize) {
    out.clear();
    out.resize(ww * wh, 0);
    if ww == 0 || wh == 0 || src.w == 0 || src.h == 0 {
        return (0, 0, 1, 1);
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
    (ox, oy, dw, dh)
}

fn window_loop(slot: Slot, stop: Arc<AtomicBool>, input: Option<UnboundedSender<InputEvent>>) -> Result<()> {
    let mut window: Option<Window> = None;
    let mut shown: Option<Picture> = None;
    let mut dirty = false;
    let mut last_size = (0usize, 0usize);
    let mut buf: Vec<u32> = Vec::new();
    let mut rect = (0usize, 0usize, 1usize, 1usize);
    let mut inp = InputState::default();

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
        if !w.is_open() {
            break;
        }
        if let Some(tx) = &input {
            let quit = w.is_key_down(Key::Q)
                && (w.is_key_down(Key::LeftCtrl) || w.is_key_down(Key::RightCtrl))
                && (w.is_key_down(Key::LeftAlt) || w.is_key_down(Key::RightAlt));
            if quit {
                inp.release_all(tx);
                break;
            }
            inp.poll(w, tx, rect);
        }
        let size = w.get_size();
        if size.0 == 0 || size.1 == 0 {
            w.update();
            continue;
        }
        if dirty || size != last_size {
            rect = fit_scale(p, size.0, size.1, &mut buf);
            last_size = size;
            dirty = false;
            w.update_with_buffer(&buf, size.0, size.1).context("oyna yangilanmadi")?;
        } else {
            w.update();
        }
    }
    if let Some(tx) = &input {
        inp.release_all(tx);
    }
    Ok(())
}

/// Oynadagi sichqoncha/klaviatura holatini kuzatib, o'zgarishlarni hodisaga aylantiradi.
#[derive(Default)]
struct InputState {
    last_pos: Option<(u16, u16)>,
    buttons: [bool; 3],
    keys: Vec<(Key, u32)>,
}

impl InputState {
    fn poll(&mut self, w: &Window, tx: &UnboundedSender<InputEvent>, rect: (usize, usize, usize, usize)) {
        if !w.is_active() {
            self.release_all(tx);
            return;
        }
        let (ox, oy, dw, dh) = rect;
        let mut inside = false;
        if let Some((mx, my)) = w.get_mouse_pos(MouseMode::Discard) {
            let (fx, fy) = ((mx - ox as f32) / dw as f32, (my - oy as f32) / dh as f32);
            if (0.0..=1.0).contains(&fx) && (0.0..=1.0).contains(&fy) {
                inside = true;
                let pos = ((fx * 65535.0) as u16, (fy * 65535.0) as u16);
                if self.last_pos != Some(pos) {
                    self.last_pos = Some(pos);
                    let _ = tx.send(InputEvent::PointerMove { x: pos.0, y: pos.1 });
                }
            }
        }
        for (i, b) in [MouseButton::Left, MouseButton::Right, MouseButton::Middle].into_iter().enumerate() {
            let down = w.get_mouse_down(b);
            if down != self.buttons[i] && (inside || !down) {
                self.buttons[i] = down;
                let _ = tx.send(InputEvent::Button { button: i as u8, pressed: down });
            }
        }
        if inside {
            if let Some((sx, sy)) = w.get_scroll_wheel() {
                let step = |v: f32| -> i16 {
                    if v == 0.0 {
                        0
                    } else if v.abs() < 1.0 {
                        if v > 0.0 { 1 } else { -1 }
                    } else {
                        v.round() as i16
                    }
                };
                // minifb: musbat = yuqoriga; portal: musbat = pastga
                let (dx, dy) = (step(sx), -step(sy));
                if dx != 0 || dy != 0 {
                    let _ = tx.send(InputEvent::Scroll { dx, dy });
                }
            }
        }
        for k in w.get_keys_released() {
            if let Some(pos) = self.keys.iter().position(|(kk, _)| *kk == k) {
                let (_, code) = self.keys.remove(pos);
                let _ = tx.send(InputEvent::Key { code, pressed: false });
            }
        }
        for k in w.get_keys_pressed(KeyRepeat::No) {
            if let Some(code) = keymap::evdev(k) {
                if !self.keys.iter().any(|(kk, _)| *kk == k) {
                    self.keys.push((k, code));
                    let _ = tx.send(InputEvent::Key { code, pressed: true });
                }
            }
        }
    }

    /// Bosilgan hamma tugma va klavishlarni qo'yib yuboradi (oyna fokusni yo'qotganda yoki yopilganda).
    fn release_all(&mut self, tx: &UnboundedSender<InputEvent>) {
        for (_, code) in self.keys.drain(..) {
            let _ = tx.send(InputEvent::Key { code, pressed: false });
        }
        for (i, down) in self.buttons.iter_mut().enumerate() {
            if *down {
                *down = false;
                let _ = tx.send(InputEvent::Button { button: i as u8, pressed: false });
            }
        }
    }
}
