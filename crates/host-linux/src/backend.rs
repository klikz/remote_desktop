//! Ekran oqimi va kiritish backend'i: xdg-desktop-portal yoki Mutter D-Bus (`mutter-ctl`).
//!
//! Tanlash: `RD_BACKEND` muhit o'zgaruvchisi yoki sozlamalar papkasidagi `backend` fayli:
//!   portal (standart) | mutter | auto (Mutter ishlasa u, bo'lmasa portal).
//! Qulflangan ekranda ishlash uchun `mutter` kerak (+ "Allow Locked Remote Desktop" kengaytmasi).

use anyhow::Result;
use protocol::InputEvent;
use std::path::Path;

use crate::portal::{PortalHandle, StreamInfo};

#[derive(Clone)]
pub enum Backend {
    Portal(PortalHandle),
    Mutter(mutter_ctl::MutterHandle),
}

fn configured(dir: &Path) -> String {
    std::env::var("RD_BACKEND")
        .ok()
        .or_else(|| std::fs::read_to_string(dir.join("backend")).ok())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "portal".to_string())
}

impl Backend {
    pub async fn select(dir: &Path) -> Backend {
        let want = configured(dir);
        let mutter = match want.as_str() {
            "mutter" => true,
            "auto" => match mutter_ctl::available().await {
                Ok(()) => true,
                Err(e) => {
                    eprintln!("[host] Mutter API mavjud emas, portal ishlatiladi: {e:#}");
                    false
                }
            },
            "portal" => false,
            other => {
                eprintln!("[host] noma'lum backend `{other}`, portal ishlatiladi");
                false
            }
        };
        if mutter {
            println!("[host] backend: mutter (portal ruxsat dialogisiz, qulfda kengaytma bilan ishlaydi)");
            Backend::Mutter(mutter_ctl::MutterHandle::spawn())
        } else {
            println!("[host] backend: portal");
            Backend::Portal(PortalHandle::spawn(dir.join("portal.token")))
        }
    }

    pub async fn input(&self, ev: InputEvent) {
        match self {
            Backend::Portal(p) => p.input(ev).await,
            Backend::Mutter(m) => m.input(ev).await,
        }
    }

    pub async fn set_physical_size(&self, w: i32, h: i32) {
        match self {
            Backend::Portal(p) => p.set_physical_size(w, h).await,
            Backend::Mutter(m) => m.set_physical_size(w, h).await,
        }
    }

    /// Ruxsatni bekor qilish: faqat portal'da ma'noli (Mutter'da ruxsat dialogi yo'q).
    pub async fn reset(&self) {
        if let Backend::Portal(p) = self {
            p.reset().await;
        }
    }

    pub async fn open_stream(&self, monitor: usize) -> Result<StreamInfo> {
        match self {
            Backend::Portal(p) => p.open_stream(monitor).await,
            Backend::Mutter(m) => {
                // Hozircha faqat asosiy monitor.
                let s = m.open_stream().await?;
                Ok(StreamInfo { fd: None, node_id: s.node_id, index: 0, count: 1, width: s.width, height: s.height })
            }
        }
    }
}
