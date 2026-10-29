//! Go/no-go tekshiruv: Ubuntu (GNOME Wayland) da ekran olish va kiritish portallari
//! restore token bilan DIALOGSIZ (qarovsiz) ishlaydimi?
//!
//! Foydalanish:
//!   portal-probe [--delay SEKUND] [--watch SEKUND] [--no-input] [--reset]
//!                [--capture SEKUND [--fps N] [--kbps N] [--out FAYL.h264]]
//!
//!   --capture: PipeWire oqimidan kadr olib H.264 ga kodlaydi (faqat `--features capture` bilan
//!   build qilinganda). Statistika chiqaradi, --out berilsa faylga yozadi.
//!
//!   --watch N: sessiyani N soniya ochiq ushlab, har 2 soniyada kursorni 1 px surib
//!   (ok/xato chiqaradi). Shu vaqt ichida ekranni qulflab/ochib sinang.
//!
//! Dastur ScreenCast + RemoteDesktop sessiyasini ochadi, olingan restore token'ni saqlaydi
//! va (ixtiyoriy) kursorni 30 px o'ngga surib, orqaga qaytaradi.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("portal-probe faqat Linux uchun.");
}

#[cfg(all(target_os = "linux", feature = "capture"))]
mod capture;

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    imp::run(std::env::args().skip(1).collect()).await
}

#[cfg(target_os = "linux")]
mod imp {
    use anyhow::{Context, Result};
    use ashpd::desktop::{
        remote_desktop::{DeviceType, RemoteDesktop},
        screencast::{CursorMode, Screencast, SourceType},
        PersistMode,
    };
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };

    fn token_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("remote-desktop").join("portal.token")
    }

    pub async fn run(args: Vec<String>) -> Result<()> {
        let mut delay: u64 = 0;
        let mut watch: u64 = 0;
        let mut capture_secs: u64 = 0;
        let mut fps: u32 = 30;
        let mut kbps: u32 = 4000;
        let mut out: Option<PathBuf> = None;
        let mut no_input = false;
        let mut reset = false;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--delay" => delay = it.next().context("--delay uchun qiymat kerak")?.parse().context("noto'g'ri son")?,
                "--watch" => watch = it.next().context("--watch uchun qiymat kerak")?.parse().context("noto'g'ri son")?,
                "--capture" => capture_secs = it.next().context("--capture uchun qiymat kerak")?.parse().context("noto'g'ri son")?,
                "--fps" => fps = it.next().context("--fps uchun qiymat kerak")?.parse().context("noto'g'ri son")?,
                "--kbps" => kbps = it.next().context("--kbps uchun qiymat kerak")?.parse().context("noto'g'ri son")?,
                "--out" => out = Some(PathBuf::from(it.next().context("--out uchun yo'l kerak")?)),
                "--no-input" => no_input = true,
                "--reset" => reset = true,
                other => anyhow::bail!("noma'lum argument: {other}"),
            }
        }

        let path = token_path();
        if reset {
            let _ = std::fs::remove_file(&path);
            println!("Saqlangan token o'chirildi.");
        }
        let saved = std::fs::read_to_string(&path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        println!(
            "Saqlangan token: {}",
            if saved.is_some() { "bor" } else { "yo'q (birinchi marta ruxsat so'raladi)" }
        );

        if delay > 0 {
            println!("{delay} soniyadan keyin boshlanadi (ekranni qulflash uchun vaqt)...");
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }

        let remote_desktop = RemoteDesktop::new()
            .await
            .context("RemoteDesktop portaliga ulanib bo'lmadi")?;
        let screencast = Screencast::new()
            .await
            .context("ScreenCast portaliga ulanib bo'lmadi")?;

        let session = remote_desktop.create_session().await.context("sessiya yaratilmadi")?;
        remote_desktop
            .select_devices(
                &session,
                DeviceType::Keyboard | DeviceType::Pointer,
                saved.as_deref(),
                PersistMode::ExplicitlyRevoked,
            )
            .await
            .context("select_devices xatosi")?;
        screencast
            .select_sources(
                &session,
                CursorMode::Embedded,
                SourceType::Monitor.into(),
                false,
                None,
                // RemoteDesktop sessiyasida persist faqat select_devices'da beriladi.
                PersistMode::DoNot,
            )
            .await
            .context("select_sources xatosi")?;

        println!("Sessiya boshlanmoqda...");
        let t0 = Instant::now();
        let started = remote_desktop
            .start(&session, None)
            .await
            .context("start xatosi")?
            .response()
            .context("portal sessiyani rad etdi (dialogda 'Bekor qilish' bosilgan bo'lishi mumkin)")?;
        let elapsed = t0.elapsed();

        println!("Qurilmalar: {:?}", started.devices());
        for s in started.streams().unwrap_or_default() {
            println!(
                "Oqim: PipeWire node {}, o'lcham {:?}, joylashuv {:?}",
                s.pipe_wire_node_id(),
                s.size(),
                s.position()
            );
        }
        println!("Start javobi {:.2} soniyada keldi", elapsed.as_secs_f64());

        match started.restore_token() {
            Some(t) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, t).context("token faylini yozib bo'lmadi")?;
                println!("Yangi restore token saqlandi: {}", path.display());
            }
            None => println!("DIQQAT: portal restore token QAYTARMADI (qarovsiz ishlash bunday ishlamaydi)."),
        }

        if saved.is_some() {
            if elapsed < Duration::from_millis(1500) {
                println!("NATIJA: token ishladi, ruxsat dialogi ko'rsatilmagan ko'rinadi (tez javob).");
            } else {
                println!("NATIJA: javob sekin keldi, dialog ko'rsatilgan bo'lishi mumkin. Dialog chiqdimi?");
            }
        }

        if !no_input {
            println!("Sichqoncha sinovi: kursor 30 px o'ngga, so'ng orqaga...");
            remote_desktop.notify_pointer_motion(&session, 30.0, 0.0).await.context("notify_pointer_motion xatosi")?;
            tokio::time::sleep(Duration::from_millis(600)).await;
            remote_desktop.notify_pointer_motion(&session, -30.0, 0.0).await.context("notify_pointer_motion xatosi")?;
        }

        #[cfg(feature = "capture")]
        {
            if capture_secs > 0 {
                let node = started
                    .streams()
                    .unwrap_or_default()
                    .first()
                    .map(|s| s.pipe_wire_node_id())
                    .context("portal oqim bermadi")?;
                let fd = screencast
                    .open_pipe_wire_remote(&session)
                    .await
                    .context("PipeWire ulanishini (fd) olib bo'lmadi")?;
                println!("PipeWire node {node}, kodlash: {fps} FPS, {kbps} kbit/s");
                let out = out.clone();
                tokio::task::spawn_blocking(move || crate::capture::run(fd, node, capture_secs, fps, kbps, out))
                    .await
                    .context("capture oqimi to'xtadi")??;
            }
        }
        #[cfg(not(feature = "capture"))]
        {
            let _ = (fps, kbps, &out);
            if capture_secs > 0 {
                println!("--capture uchun `--features capture` bilan build qiling (README ga qarang).");
            }
        }

        if watch > 0 {
            println!("Kuzatuv: {watch} soniya. Hozir ekranni qulflab (Super+L), keyin oching...");
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(watch) {
                let t = start.elapsed().as_secs();
                let a = remote_desktop.notify_pointer_motion(&session, 1.0, 0.0).await;
                let b = remote_desktop.notify_pointer_motion(&session, -1.0, 0.0).await;
                match (a, b) {
                    (Ok(_), Ok(_)) => println!("t={t:>3}s: kiritish ok"),
                    (Err(e), _) | (_, Err(e)) => println!("t={t:>3}s: kiritish XATO: {e}"),
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }

        tokio::time::sleep(Duration::from_secs(2)).await;
        session.close().await.context("sessiyani yopib bo'lmadi")?;
        println!("Tugadi.");
        Ok(())
    }
}
