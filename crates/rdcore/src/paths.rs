//! Konfiguratsiya fayllari joylashuvi.
//! Linux: `~/.config/remote-desktop/`, Windows: `%APPDATA%\remote-desktop\`.

use anyhow::{anyhow, Result};
use std::path::PathBuf;

pub fn config_dir() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|d| d.join("remote-desktop"))
        .ok_or_else(|| anyhow!("konfiguratsiya papkasi aniqlanmadi"))
}

/// Host parol kaliti (xesh) fayli.
pub fn host_password_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("password.key"))
}

/// Viewer ishonchli Host'lar ro'yxati.
pub fn known_hosts_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("known_hosts"))
}
