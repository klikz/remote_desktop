//! Viewer (Windows va Linux). Argumentsiz: oynali interfeys. `connect ...`: buyruq qatori (sinov uchun).

mod gui;

use anyhow::{bail, Context, Result};
use protocol::DEFAULT_PORT;
use rdcore::{
    client::{connect_with, ConnectOptions, Transport},
    known_hosts::KnownHosts,
    paths,
};
use std::{
    fs::File,
    io::{self, Write},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    time::{Duration, Instant},
};

const USAGE: &str = "\
viewer: Remote Desktop Viewer

Foydalanish:
  viewer                               oynali interfeys (tavsiya etiladi)
  viewer connect <manzil[:port]> [--video SEKUND] [--fps N] [--kbps N] [--height N] [--monitor N] [--tcp | --udp] [--out FAYL.h264]

Buyruq qatori sinov uchun:
  viewer connect 192.168.2.226                 ulanish va ping sinovi
  viewer connect 192.168.2.226 --video 10      10 soniya video qabul qilib statistika chiqarish
  viewer connect 192.168.2.226 --video 10 --out test.h264
";

struct Options {
    video_secs: u64,
    fps: u16,
    kbps: u32,
    max_height: u16,
    monitor: u8,
    out: Option<PathBuf>,
    transport: Transport,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Argumentsiz: oynali interfeys. `connect ...`: buyruq qatori (sinov va diagnostika uchun).
    if args.is_empty() {
        return gui::run();
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(cli_main(args))
}

async fn cli_main(args: Vec<String>) -> Result<()> {
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("connect"), Some(target)) => {
            let mut opts = Options { video_secs: 0, fps: 30, kbps: 10000, max_height: 1080, monitor: 0, out: None, transport: Transport::Auto };
            let mut it = args[2..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--video" => opts.video_secs = it.next().context("--video uchun qiymat kerak")?.parse().context("noto'g'ri son")?,
                    "--fps" => opts.fps = it.next().context("--fps uchun qiymat kerak")?.parse().context("noto'g'ri son")?,
                    "--kbps" => opts.kbps = it.next().context("--kbps uchun qiymat kerak")?.parse().context("noto'g'ri son")?,
                    "--height" => opts.max_height = it.next().context("--height uchun qiymat kerak")?.parse().context("noto'g'ri son")?,
                    "--monitor" => opts.monitor = it.next().context("--monitor uchun qiymat kerak")?.parse().context("noto'g'ri son")?,
                    "--tcp" => opts.transport = Transport::Tcp,
                    "--udp" => opts.transport = Transport::Udp,
                    "--out" => opts.out = Some(PathBuf::from(it.next().context("--out uchun yo'l kerak")?)),
                    other => bail!("noma'lum argument: {other}\n\n{USAGE}"),
                }
            }
            run_connect(target, opts).await
        }
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    }
}

async fn resolve(target: &str) -> Result<SocketAddr> {
    if let Ok(a) = target.parse::<SocketAddr>() {
        return Ok(a);
    }
    if let Ok(ip) = target.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    let with_port = if target.contains(':') {
        target.to_string()
    } else {
        format!("{target}:{DEFAULT_PORT}")
    };
    let mut addrs = tokio::net::lookup_host(with_port)
        .await
        .with_context(|| format!("{target} manzilini aniqlab bo'lmadi"))?;
    let first = addrs.next();
    first.with_context(|| format!("{target} uchun manzil topilmadi"))
}

fn ask_yes_no(question: &str) -> bool {
    print!("{question} [yes/no]: ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "yes" | "y")
}

async fn run_connect(target: &str, opts: Options) -> Result<()> {
    let addr = resolve(target).await?;
    let password = rpassword::prompt_password("Parol: ")?;
    let known_hosts = KnownHosts::new(paths::known_hosts_file()?);

    let confirm = |fp: &str| {
        println!("Bu Host'ga birinchi marta ulanyapsiz.");
        println!("Fingerprint: {fp}");
        println!("Uni Host ekranidagi fingerprint bilan solishtiring.");
        ask_yes_no("Ishonasizmi?")
    };

    println!("{addr} ga ulanilmoqda...");
    let mut session = connect_with(
        ConnectOptions { addr, password: &password, known_hosts: &known_hosts, confirm_new_host: &confirm },
        opts.transport,
    )
    .await?;
    println!("Ulandi{}. Host imkoniyatlari: {:?}", session.host_caps);

    if opts.video_secs > 0 {
        receive_video(&mut session, &opts).await?;
    } else {
        for i in 1..=5 {
            match session.ping().await {
                Ok(rtt) => println!("ping {i}: {:.1} ms", rtt.as_secs_f64() * 1000.0),
                Err(e) => bail!("ping xatosi: {e:#}"),
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    session.close().await;
    Ok(())
}

async fn receive_video(session: &mut rdcore::client::Session, opts: &Options) -> Result<()> {
    println!("Video so'ralmoqda: {} FPS, {} kbit/s...", opts.fps, opts.kbps);
    let requested = Instant::now();
    let mut stream = session.request_video(opts.fps, opts.kbps, opts.max_height, opts.monitor).await?;
    println!(
        "Host videoni boshladi (kodek {}, {} FPS, {} kbit/s). {} soniya qabul qilinadi...",
        stream.codec, stream.fps, stream.kbps, opts.video_secs
    );

    let mut file = match &opts.out {
        Some(p) => Some(File::create(p).with_context(|| format!("{} yaratilmadi", p.display()))?),
        None => None,
    };

    let started = Instant::now();
    let deadline = started + Duration::from_secs(opts.video_secs);
    let (mut frames, mut keyframes, mut bytes) = (0u64, 0u64, 0u64);
    let mut first_frame: Option<Duration> = None;
    let mut last_pts: Option<u64> = None;
    let mut max_gap_ms = 0u64;
    let mut last_print = Instant::now();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let next = match tokio::time::timeout(remaining, stream.next_frame()).await {
            Err(_) => break, // vaqt tugadi
            Ok(r) => r?,
        };
        let Some(frame) = next else {
            println!("Host video oqimini tugatdi.");
            break;
        };
        if first_frame.is_none() {
            first_frame = Some(requested.elapsed());
        }
        frames += 1;
        keyframes += u64::from(frame.keyframe);
        bytes += frame.data.len() as u64;
        if let Some(prev) = last_pts {
            max_gap_ms = max_gap_ms.max(frame.pts_us.saturating_sub(prev) / 1000);
        }
        last_pts = Some(frame.pts_us);
        if let Some(f) = file.as_mut() {
            f.write_all(&frame.data)?;
        }
        if last_print.elapsed() >= Duration::from_secs(2) {
            println!("  ... {frames} kadr, {:.1} MB", bytes as f64 / 1e6);
            last_print = Instant::now();
        }
    }

    let elapsed = started.elapsed().as_secs_f64();
    println!("--- Natija ---");
    println!("Kadrlar: {frames} (kalit kadrlar: {keyframes})");
    if frames == 0 {
        bail!("birorta ham kadr kelmadi");
    }
    println!("O'rtacha FPS: {:.1}", frames as f64 / elapsed);
    println!("O'rtacha bitrate: {:.0} kbit/s", bytes as f64 * 8.0 / 1000.0 / elapsed);
    println!("So'rovdan birinchi kadrgacha: {:?}", first_frame.unwrap_or_default());
    println!("Eng katta uzilish kadrlar orasida: {max_gap_ms} ms");
    if let Some(p) = &opts.out {
        println!("Saqlandi: {} (VLC yoki `ffplay -f h264 fayl` bilan oching)", p.display());
    }
    Ok(())
}
