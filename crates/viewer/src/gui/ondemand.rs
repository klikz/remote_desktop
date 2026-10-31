//! Qarovsiz (ID orqali) ulanish uchun talab bo'yicha Host. Alohida parol (`unattended.key`) bilan,
//! faqat loopback'da va bo'sh portda ishlaydi, shuning uchun "Ulanishga ruxsat berish" yoqilmagan
//! bo'lsa ham, Host paroli o'rnatilmagan bo'lsa ham ishlaydi. Birinchi so'rovda ishga tushadi,
//! agent to'xtatilganda yoki dastur yopilganda to'xtaydi.

use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use anyhow::bail;
use rdcore::{auth::Credentials, paths, relay::agent::EnsureHost};
use std::sync::{Arc, Mutex};

/// Saqlangan qarovsiz parol bormi.
pub fn has_password() -> bool {
    paths::unattended_file().is_ok_and(|p| p.exists())
}

/// Parolni xesh qilib saqlaydi (ochiq parol saqlanmaydi).
pub fn save_password(pw: &str) -> Result<()> {
    Credentials::from_password(pw)?.save(&paths::unattended_file()?)
}

/// Joriy ishlayotgan Host'ni to'xtatish uchun tutqich. `drop` to'xtatadi.
#[derive(Default)]
pub struct Guard(Arc<Mutex<Option<Running>>>);

impl Guard {
    pub fn ensure_fn(&self) -> EnsureHost {
        let slot = self.0.clone();
        Arc::new(move || ensure(&slot))
    }

    /// Parol o'zgarganda: keyingi so'rovda yangi parol bilan qayta ishga tushadi.
    pub fn stop(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

fn ensure(slot: &Mutex<Option<Running>>) -> Result<u16> {
    let mut g = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = g.as_mut() {
        if r.alive() {
            return Ok(r.port());
        }
        *g = None;
    }
    let creds = Credentials::load(&paths::unattended_file()?).context("qarovsiz parol o'rnatilmagan")?;
    let r = Running::start(creds)?;
    let port = r.port();
    *g = Some(r);
    Ok(port)
}

#[cfg(windows)]
struct Running(host_windows::RunningHost);

#[cfg(windows)]
impl Running {
    fn start(creds: Credentials) -> Result<Self> {
        Ok(Running(host_windows::RunningHost::start_dedicated(creds)?))
    }
    fn port(&self) -> u16 {
        self.0.port
    }
    fn alive(&mut self) -> bool {
        true
    }
}

#[cfg(target_os = "linux")]
struct Running {
    child: std::process::Child,
    port: u16,
}

#[cfg(target_os = "linux")]
impl Running {
    fn start(_creds: Credentials) -> Result<Self> {
        let bin = super::linuxhost::bin_path().filter(|p| p.exists());
        let Some(bin) = bin else {
            bail!("Host o'rnatilmagan: avval \"Bu kompyuter\" bo'limidan \"Ulanishga ruxsat berish\" orqali o'rnating")
        };
        // Bo'sh UDP portni topamiz
        let port = std::net::UdpSocket::bind("127.0.0.1:0")?.local_addr()?.port();
        let pw_file = paths::unattended_file()?;
        let mut child = std::process::Command::new(bin)
            .args(["run", "--bind", "127.0.0.1", "--port", &port.to_string(), "--password-file"])
            .arg(pw_file)
            .stdin(std::process::Stdio::null())
            .spawn()
            .context("Host ishga tushmadi")?;
        std::thread::sleep(std::time::Duration::from_millis(1500));
        if let Ok(Some(st)) = child.try_wait() {
            bail!("Host darhol to'xtadi ({st})");
        }
        Ok(Running { child, port })
    }
    fn port(&self) -> u16 {
        self.port
    }
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

#[cfg(target_os = "linux")]
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
