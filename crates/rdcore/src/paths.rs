//! Konfiguratsiya fayllari joylashuvi.
//! Linux: `~/.config/remote-desktop/`, Windows: `%APPDATA%\remote-desktop\`.

use anyhow::{anyhow, Result};
use std::path::PathBuf;

pub fn config_dir() -> Result<PathBuf> {
    // `RD_CONFIG_DIR`: boshqa papka (bir kompyuterda bir nechta nusxa yoki testlar uchun).
    if let Some(d) = std::env::var_os("RD_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(d));
    }
    dirs::config_dir()
        .map(|d| d.join("remote-desktop"))
        .ok_or_else(|| anyhow!("konfiguratsiya papkasi aniqlanmadi"))
}

/// Host parol kaliti (xesh) fayli.
pub fn host_password_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("password.key"))
}

/// Host tinglaydigan port saqlanadigan fayl (bitta son).
pub fn host_port_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("port"))
}

/// Saqlangan Host porti. Fayl yo'q yoki noto'g'ri bo'lsa `None` (standart port ishlatiladi).
pub fn read_host_port() -> Option<u16> {
    let text = std::fs::read_to_string(host_port_file().ok()?).ok()?;
    let p: u16 = text.trim().parse().ok()?;
    (p >= 1024).then_some(p)
}

/// Bir martalik ulanish paroli (relay'da ruxsat berilganda yaratiladi): kalit va amal qilish muddati.
pub fn onetime_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("onetime.key"))
}

/// Viewer ishonchli Host'lar ro'yxati.
pub fn known_hosts_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("known_hosts"))
}
