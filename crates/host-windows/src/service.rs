//! Windows Host CLI va sessiya mantiqi (Linux Host bilan bir xil protokol).

use crate::{input, video};
use anyhow::{bail, Context, Result};
use protocol::{video_reason, Capabilities, Message, VideoHeader, CODEC_H264, DEFAULT_PORT};
use rdcore::{
    auth::Credentials,
    cert::HostIdentity,
    framing::{read_message, write_message},
    hexfmt, paths,
    server::{Handler, Host, SessionIo},
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{sync::mpsc, task::JoinHandle};
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

const USAGE: &str = "\
host-windows: Remote Desktop Host (Windows)

Buyruqlar:
  host-windows set-password            Ulanish parolini o'rnatish (kamida 8 belgi)
  host-windows run [--bind IP] [--port PORT]
                                       Host'ni ishga tushirish
                                       (standart: --bind 0.0.0.0, port: UI'da saqlangan yoki 7777)

Birinchi ishga tushirishda Windows xavfsizlik devori (Firewall) so'raydi: \"Ruxsat berish\" ni bosing.
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
    // Koordinatalar va o'lchamlar fizik pikselda bo'lishi uchun (ekran masshtabi 125% va h.k.)
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let mut ip: IpAddr = "0.0.0.0".parse()?;
    let mut port: u16 = paths::read_host_port().unwrap_or(DEFAULT_PORT);
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
        .context("parol o'rnatilmagan. Avval `host-windows set-password` yoki dasturdagi \"Bu kompyuter\" bo'limi")?;
    let caps = Capabilities { codecs: CODEC_H264, max_fps: 60, monitors: vec![] };

    let host = Host::bind(SocketAddr::new(ip, port), &identity, creds, caps)?;
    println!("Host ishga tushdi: {} (UDP)", host.local_addr()?);
    println!("Fingerprint: {}", hexfmt::colon(&host.fingerprint()));
    println!("Viewer birinchi ulanishda shu fingerprint'ni tasdiqlaydi. To'xtatish: Ctrl+C");

    let handler: Handler = Arc::new(|io| Box::pin(session(io)));
    host.run_with(handler).await
}

struct Video {
    task: JoinHandle<()>,
    stop: Arc<AtomicBool>,
}

impl Video {
    fn running(&self) -> bool {
        !self.task.is_finished()
    }
    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.task.abort();
    }
}

/// Bitta Viewer sessiyasi: control xabarlari, video va kiritish.
async fn session(mut io: SessionIo) {
    let mut video: Option<Video> = None;
    loop {
        match read_message(&mut io.recv).await {
            Ok(Message::Ping { id, sent_ms }) => {
                if write_message(&mut io.send, &Message::Pong { id, sent_ms }).await.is_err() {
                    break;
                }
            }
            Ok(Message::VideoRequest { fps, kbps, max_height, .. }) => {
                let reply = if video.as_ref().map_or(false, Video::running) {
                    Message::VideoFailed { reason: video_reason::BUSY }
                } else {
                    let (fps, kbps) = (fps.clamp(5, 60), kbps.clamp(500, 50_000));
                    video = Some(start_video(&io, fps, kbps, max_height));
                    println!("[host] video boshlandi: {fps} FPS, {kbps} kbit/s, balandlik {max_height}");
                    Message::VideoStarted { codec: CODEC_H264, fps, kbps }
                };
                if write_message(&mut io.send, &reply).await.is_err() {
                    break;
                }
            }
            Ok(Message::Input(ev)) => input::apply(ev),
            Ok(Message::Bye { .. }) | Err(_) => break,
            Ok(_) => {}
        }
    }
    if let Some(v) = video {
        v.stop();
    }
    io.conn.close(0u32.into(), b"bye");
}

fn start_video(io: &SessionIo, fps: u16, kbps: u32, max_height: u16) -> Video {
    let stop = Arc::new(AtomicBool::new(false));
    let conn = io.conn.clone();
    let stop2 = stop.clone();
    let task = tokio::spawn(async move {
        let (tx, mut rx) = mpsc::channel::<video::EncodedFrame>(8);
        let (etx, mut erx) = mpsc::unbounded_channel::<String>();
        let thread = video::spawn(u32::from(fps), kbps, u32::from(max_height), tx, stop2.clone(), etx);

        let mut stream = match conn.open_uni().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[host] video oqimini ochib bo'lmadi: {e}");
                stop2.store(true, Ordering::Relaxed);
                return;
            }
        };
        let mut sent: u64 = 0;
        loop {
            tokio::select! {
                frame = rx.recv() => {
                    let Some(f) = frame else { break };
                    let header = VideoHeader { keyframe: f.keyframe, pts_us: f.pts_us, len: f.data.len() as u32 }.encode();
                    if stream.write_all(&header).await.is_err() || stream.write_all(&f.data).await.is_err() {
                        break; // Viewer uzildi
                    }
                    sent += 1;
                }
                err = erx.recv() => {
                    if let Some(m) = err {
                        eprintln!("[host] ekran/kodlash xatosi: {m}");
                    }
                    break;
                }
            }
        }
        stop2.store(true, Ordering::Relaxed);
        let _ = stream.finish();
        let _ = thread.join();
        println!("[host] video to'xtadi ({sent} kadr yuborildi)");
    });
    Video { task, stop }
}
