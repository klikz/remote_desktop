//! Viewer (Windows). Bosqich 1: ulanish, autentifikatsiya, ping.
//! Video ko'rsatish va kiritish keyingi bosqichlarda qo'shiladi.

use anyhow::{bail, Context, Result};
use protocol::DEFAULT_PORT;
use rdcore::{
    client::{connect, ConnectOptions},
    known_hosts::KnownHosts,
    paths,
};
use std::{
    io::{self, Write},
    net::{IpAddr, SocketAddr},
    time::Duration,
};

const USAGE: &str = "\
viewer-windows: Remote Desktop Viewer

Foydalanish:
  viewer-windows connect <manzil[:port]>

Misol:
  viewer-windows connect 203.0.113.10
  viewer-windows connect host.example.com:7777
";

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("connect"), Some(target)) => run_connect(target).await,
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

async fn run_connect(target: &str) -> Result<()> {
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
    let mut session = connect(ConnectOptions {
        addr,
        password: &password,
        known_hosts: &known_hosts,
        confirm_new_host: &confirm,
    })
    .await?;

    println!("Ulandi. Host imkoniyatlari: {:?}", session.host_caps);
    for i in 1..=5 {
        match session.ping().await {
            Ok(rtt) => println!("ping {i}: {:.1} ms", rtt.as_secs_f64() * 1000.0),
            Err(e) => bail!("ping xatosi: {e:#}"),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    session.close().await;
    Ok(())
}
