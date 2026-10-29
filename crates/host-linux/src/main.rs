//! Host (Ubuntu, Wayland). Bosqich 1: ulanish, autentifikatsiya, ping.
//! Ekran olish va kiritish keyingi bosqichlarda qo'shiladi.

use anyhow::{bail, Context, Result};
use protocol::{Capabilities, CODEC_H264, DEFAULT_PORT};
use rdcore::{auth::Credentials, cert::HostIdentity, hexfmt, paths, server::Host};
use std::net::{IpAddr, SocketAddr};

const USAGE: &str = "\
host-linux: Remote Desktop Host

Buyruqlar:
  host-linux set-password            Ulanish parolini o'rnatish (kamida 8 belgi)
  host-linux run [--bind IP] [--port PORT]
                                     Host'ni ishga tushirish
                                     (standart: --bind 0.0.0.0 --port 7777)
";

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("set-password") => set_password(),
        Some("run") => run(&args[1..]).await,
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

async fn run(args: &[String]) -> Result<()> {
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
        monitors: vec![], // Bosqich 2 da to'ldiriladi
    };

    let host = Host::bind(SocketAddr::new(ip, port), &identity, creds, caps)?;
    println!("Host ishga tushdi: {} (UDP)", host.local_addr()?);
    println!("Fingerprint: {}", hexfmt::colon(&host.fingerprint()));
    println!("Viewer birinchi ulanishda shu fingerprint'ni tasdiqlaydi.");
    host.run().await
}
