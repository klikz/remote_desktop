//! Windows: Host'ni dastur ichidan yoqish/o'chirish, ulanganlar ro'yxati va avtostart.
//! "Ulanishga ruxsat berish" holati `host.enabled` fayli bilan eslab qolinadi:
//! dastur qayta ochilganda Host avtomatik yoqiladi.

use super::theme;
use egui::Ui;
use host_windows::RunningHost;
use rdcore::paths;
use std::{os::windows::process::CommandExt, process::Command, time::Duration};

const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "RemoteDesktop";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct WinHost {
    host: Option<RunningHost>,
    error: Option<String>,
    autostart: bool,
}

fn enabled_file() -> Option<std::path::PathBuf> {
    paths::config_dir().ok().map(|d| d.join("host.enabled"))
}

fn reg(args: &[&str]) -> bool {
    Command::new("reg")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn autostart_enabled() -> bool {
    reg(&["query", RUN_KEY, "/v", RUN_NAME])
}

fn set_autostart(on: bool) -> bool {
    if on {
        let Ok(exe) = std::env::current_exe() else { return false };
        let value = format!("\"{}\"", exe.display());
        reg(&["add", RUN_KEY, "/v", RUN_NAME, "/t", "REG_SZ", "/d", &value, "/f"])
    } else {
        reg(&["delete", RUN_KEY, "/v", RUN_NAME, "/f"])
    }
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

impl WinHost {
    pub fn new() -> Self {
        let mut w = WinHost { host: None, error: None, autostart: autostart_enabled() };
        if enabled_file().is_some_and(|p| p.exists()) {
            w.start();
        }
        w
    }

    pub fn running(&self) -> bool {
        self.host.is_some()
    }

    pub fn port(&self) -> Option<u16> {
        self.host.as_ref().map(|h| h.port)
    }

    fn set_enabled(on: bool) {
        if let Some(p) = enabled_file() {
            if on {
                if let Some(dir) = p.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(p, b"1\n");
            } else {
                let _ = std::fs::remove_file(p);
            }
        }
    }

    pub fn start(&mut self) {
        if self.host.is_some() {
            return;
        }
        let port = paths::read_host_port().unwrap_or(protocol::DEFAULT_PORT);
        match RunningHost::start(port) {
            Ok(h) => {
                self.host = Some(h);
                self.error = None;
                Self::set_enabled(true);
            }
            Err(e) => self.error = Some(format!("{e:#}")),
        }
    }

    pub fn stop(&mut self) {
        if let Some(h) = self.host.take() {
            h.stop();
        }
        Self::set_enabled(false);
    }

    /// Port yoki parol o'zgarganda: ishlayotgan Host'ni yangi sozlamalar bilan qayta ishga tushiradi.
    pub fn restart_if_running(&mut self) -> bool {
        if self.host.is_none() {
            return false;
        }
        if let Some(h) = self.host.take() {
            h.stop();
        }
        self.start();
        true
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        let mut on = self.host.is_some();
        let label = if on { "Ulanishga ruxsat berilgan" } else { "Ulanishga ruxsat berish" };
        if theme::toggle(ui, &mut on, label).changed() {
            if on {
                self.start();
            } else {
                self.stop();
            }
        }
        if let Some(e) = &self.error {
            theme::note(ui, e, theme::BAD);
        }

        let mut kick: Option<u64> = None;
        if let Some(h) = &self.host {
            theme::note(ui, &format!("Tinglamoqda: UDP va TCP {}", h.port), theme::GOOD);
            let peers = h.peers();
            ui.add_space(4.0);
            if peers.is_empty() {
                theme::hint(ui, "Hozir hech kim ulanmagan.");
            } else {
                ui.label("Ulanganlar:");
                for p in &peers {
                    ui.horizontal(|ui| {
                        ui.monospace(if p.via_tcp { format!("{} (TCP)", p.ip) } else { p.ip.to_string() });
                        ui.weak(fmt_dur(p.since.elapsed()));
                        if theme::button(ui, "Uzish").clicked() {
                            kick = Some(p.id);
                        }
                    });
                }
            }
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
        if let (Some(id), Some(h)) = (kick, &self.host) {
            h.kick(id);
        }

        ui.add_space(6.0);
        let mut auto = self.autostart;
        if theme::toggle(ui, &mut auto, "Windows ishga tushganda dasturni avtomatik ochish").changed() {
            if set_autostart(auto) {
                self.autostart = auto;
            }
        }
        theme::hint(ui, "Dastur ochilganda, ruxsat yoqilgan bo'lsa, Host o'zi ishga tushadi.");
    }
}
