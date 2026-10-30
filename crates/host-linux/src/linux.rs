//! Host CLI va sessiya mantiqi.

use anyhow::{bail, Context, Result};
use protocol::{
    video_reason, Capabilities, Message, VideoHeader, CODEC_H264, DEFAULT_PORT,
};
use rdcore::{
    auth::Credentials,
    cert::HostIdentity,
    framing::{read_message, write_message},
    hexfmt, paths,
    server::{Handler, Host, SessionIo},
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tokio::task::JoinHandle;

use crate::{capture, portal::PortalHandle};

const USAGE: &str = "\
host-linux: Remote Desktop Host

Buyruqlar:
  host-linux set-password            Ulanish parolini o'rnatish (kamida 8 belgi)
  host-linux run [--bind IP] [--port PORT]
                                     Host'ni ishga tushirish
                                     (standart: --bind 0.0.0.0 --port 7777)

Muhit o'zgaruvchisi: RD_ENCODER=x264enc|nvh264enc|vah264enc|openh264enc (kodlagichni majburlash)
";

pub async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("set-password") => set_password(),
        Some("run") => run_host(&args[1..]).await,
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    }
}

fn set_password() -> Result<()> {
    let p1 = rpassword::prompt_password("Yangi parol (kamida 8 belgi): ")?;
    let p2 = rpassword::prompt_password("Parolni takrorlang: ")?;
    if p1 != p2 {
        bail!("parollar mos kelmadi");
    }
    let creds = Credentials::from_password(&p1)?;
    let path = paths::host_password_file()?;
    creds.save(&path)?;
    println!("Parol saqlandi: {}", path.display());
    Ok(())
}

async fn run_host(args: &[String]) -> Result<()> {
    let mut ip: IpAddr = "0.0.0.0".parse()?;
    let mut port: u16 = DEFAULT_PORT;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--bind" => ip = it.next().context("--bind uchun qiymat kerak")?.parse().context("noto'g'ri IP")?,
            "--port" => port = it.next().context("--port uchun qiymat kerak")?.parse().context("noto'g'ri port")?,
            other => bail!("noma'lum argument: {other}\n\n{USAGE}"),
        }
    }

    let dir = paths::config_dir()?;
    let identity = HostIdentity::load_or_create(&dir)?;
    let creds = Credentials::load(&paths::host_password_file()?)
        .context("parol o'rnatilmagan. Avval `host-linux set-password` ni ishga tushiring")?;
    let caps = Capabilities {
        codecs: CODEC_H264,
        max_fps: 60,
        monitors: vec![], // keyingi bosqichda to'ldiriladi
    };

    let portal = PortalHandle::spawn(dir.join("portal.token"));
    let host = Host::bind(SocketAddr::new(ip, port), &identity, creds, caps)?;
    println!("Host ishga tushdi: {} (UDP)", host.local_addr()?);
    println!("Fingerprint: {}", hexfmt::colon(&host.fingerprint()));
    println!("Viewer birinchi ulanishda shu fingerprint'ni tasdiqlaydi.");

    let handler: Handler = Arc::new(move |io| {
        let portal = portal.clone();
        Box::pin(session(io, portal))
    });
    host.run_with(handler).await
}

/// Bitta Viewer sessiyasi: control xabarlari va video.
async fn session(mut io: SessionIo, portal: PortalHandle) {
    let mut video: Option<JoinHandle<()>> = None;
    loop {
        match read_message(&mut io.recv).await {
            Ok(Message::Ping { id, sent_ms }) => {
                if write_message(&mut io.send, &Message::Pong { id, sent_ms }).await.is_err() {
                    break;
                }
            }
            Ok(Message::VideoRequest { fps, kbps, max_height, monitor }) => {
                let reply = if video.as_ref().map_or(false, |h| !h.is_finished()) {
                    Message::VideoFailed { reason: video_reason::BUSY }
                } else {
                    match start_video(&io, &portal, fps, kbps, max_height, monitor).await {
                        Ok((handle, fps, kbps)) => {
                            video = Some(handle);
                            Message::VideoStarted { codec: CODEC_H264, fps, kbps }
                        }
                        Err(reason) => Message::VideoFailed { reason },
                    }
                };
                if write_message(&mut io.send, &reply).await.is_err() {
                    break;
                }
            }
            Ok(Message::Input(ev)) => portal.input(ev).await,
            Ok(Message::Bye { .. }) | Err(_) => break,
            Ok(_) => {}
        }
    }
    if let Some(h) = video {
        h.abort(); // kodlagich ham to'xtaydi
    }
    io.conn.close(0u32.into(), b"bye");
}

async fn start_video(
    io: &SessionIo,
    portal: &PortalHandle,
    fps: u16,
    kbps: u32,
    max_height: u16,
    monitor: u8,
) -> Result<(JoinHandle<()>, u16, u32), u8> {
    let fps = fps.clamp(5, 60);
    let kbps = kbps.clamp(500, 50_000);
    let info = portal.open_stream(monitor as usize).await.map_err(|e| {
        eprintln!("[host] ekranni olib bo'lmadi: {e:#}");
        video_reason::NO_CAPTURE
    })?;
    let (index, count, width, height) = (info.index, info.count, info.width, info.height);
    let cap = capture::start(info.fd, info.node_id, fps as u32, kbps, max_height as u32, (width, height)).map_err(|e| {
        eprintln!("[host] kodlashni boshlab bo'lmadi: {:#}", e.error);
        e.reason
    })?;
    println!(
        "[host] video boshlandi: monitor {}/{} ({}x{}), {fps} FPS, {kbps} kbit/s, balandlik {}, kodlagich {}",
        index,
        count,
        width,
        height,
        if max_height == 0 { "asl".to_string() } else { max_height.to_string() },
        cap.encoder
    );
    // GNOME (Mutter) sichqoncha koordinatasini portal bergan LOGIK o'lchamda kutadi (sinovda tasdiqlandi:
    // 133% masshtabda 2880x1620, kadr esa 3840x2160). Haqiqiy kadr o'lchami faqat logga yoziladi.
    {
        let size = cap.source_size.clone();
        let portal = portal.clone();
        tokio::spawn(async move {
            for _ in 0..100 {
                let got = *size.lock().unwrap();
                if let Some((w, h)) = got {
                    println!("[host] manba kadr: {w}x{h} (sichqoncha fazosi logik {width}x{height})");
                    let _ = &portal;
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            eprintln!("[host] manba kadr o'lchami aniqlanmadi, logik o'lcham ishlatiladi");
        });
    }
    let conn = io.conn.clone();
    Ok((tokio::spawn(video_sender(conn, cap)), fps, kbps))
}

async fn video_sender(conn: rdcore::quinn::Connection, mut cap: capture::Capture) {
    let mut stream = match conn.open_uni().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[host] video oqimini ochib bo'lmadi: {e}");
            return;
        }
    };
    let mut sent: u64 = 0;
    loop {
        tokio::select! {
            frame = cap.frames.recv() => {
                let Some(f) = frame else { break };
                let header = VideoHeader {
                    keyframe: f.keyframe,
                    pts_us: f.pts_us,
                    len: f.data.len() as u32,
                }
                .encode();
                if stream.write_all(&header).await.is_err() || stream.write_all(&f.data).await.is_err() {
                    break; // Viewer uzildi
                }
                sent += 1;
            }
            err = cap.errors.recv() => {
                if let Some(m) = err {
                    eprintln!("[host] kodlash xatosi: {m}");
                }
                break;
            }
        }
    }
    let _ = stream.finish();
    println!("[host] video to'xtadi ({sent} kadr yuborildi)");
}
