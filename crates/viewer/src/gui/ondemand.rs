//! ID orqali (relay) ulanishlar uchun talab bo'yicha Host: faqat loopback'da va bo'sh portda ishlaydi,
//! shuning uchun "Ulanishga ruxsat berish" yoqilmagan bo'lsa ham, Host paroli o'rnatilmagan bo'lsa ham
//! ishlaydi. Parol: qarovsiz parol (`unattended.key`) bo'lsa o'sha, bo'lmasa avtomatik yaratilgan tasodifiy
//! kalit (`relay_host.key`, hech kimga ko'rsatilmaydi) — u holda faqat ruxsat oynasidan keyingi bir martalik
//! parol bilan kirish mumkin. Birinchi so'rovda ishga tushadi, agent to'xtatilganda yoki dastur yopilganda to'xtaydi.

use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use anyhow::bail;
use rdcore::{auth::Credentials, paths, relay::agent::EnsureHost};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

/// Saqlangan qarovsiz parol bormi.
pub fn has_password() -> bool {
    paths::unattended_file().is_ok_and(|p| p.exists())
}

/// Qarovsiz parol yo'q bo'lsa ishlatiladigan tasodifiy kalit (yo'q bo'lsa yaratiladi).
fn fallback_file() -> Result<PathBuf> {
    let p = paths::config_dir()?.join("relay_host.key");
    if Credentials::load(&p).is_err() {
        let pw: String = rdcore::hexfmt::encode(&rdcore::auth::random_bytes::<24>());
        Credentials::from_password(&pw)?.save(&p)?;
    }
    Ok(p)
}

/// Parolni xesh qilib saqlaydi (ochiq parol saqlanmaydi).
pub fn save_password(pw: &str) -> Result<()> {
    Credentials::from_password(pw)?.save(&paths::unattended_file()?)
}

/// Ishlayotgan Host va uning parol tuzi.
type Slot = Mutex<Option<(Running, [u8; protocol::SALT_LEN])>>;

/// Joriy ishlayotgan Host'ni to'xtatish uchun tutqich. `drop` to'xtatadi.
#[derive(Default)]
pub struct Guard(Arc<Slot>);

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

fn ensure(slot: &Slot) -> Result<(u16, [u8; protocol::SALT_LEN])> {
    let mut g = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((r, salt)) = g.as_mut() {
        if r.alive() {
            return Ok((r.port(), *salt));
        }
        *g = None;
    }
    let file = if has_password() { paths::unattended_file()? } else { fallback_file()? };
    let creds = Credentials::load(&file).context("Host kaliti o'qilmadi")?;
    let salt = creds.salt;
    let r = Running::start(creds, &file)?;
    let port = r.port();
    *g = Some((r, salt));
    Ok((port, salt))
}

#[cfg(windows)]
struct Running(host_windows::RunningHost);

#[cfg(windows)]
impl Running {
    fn start(creds: Credentials, _file: &std::path::Path) -> Result<Self> {
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
    fn start(_creds: Credentials, pw_file: &std::path::Path) -> Result<Self> {
        let bin = super::linuxhost::bin_path().filter(|p| p.exists());
        let Some(bin) = bin else {
            bail!("Host o'rnatilmagan: avval \"Bu kompyuter\" bo'limidan \"Ulanishga ruxsat berish\" orqali o'rnating")
        };
        // Bo'sh UDP portni topamiz
        let port = std::net::UdpSocket::bind("127.0.0.1:0")?.local_addr()?.port();
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
