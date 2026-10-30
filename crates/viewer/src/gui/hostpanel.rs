//! "Bu kompyuter" bo'limi: ushbu kompyuter Host sifatida qanday ulanishni qabul qilishini ko'rsatadi
//! (manzil, fingerprint, holat) va ulanish parolini o'rnatish imkonini beradi.
//! Parol faqat xesh (argon2 kalit) ko'rinishida saqlanadi, shuning uchun keyin ko'rsatib bo'lmaydi:
//! o'rnatilgan parol faqat shu dastur ochiq turganda, ko'rsatish uchun xotirada saqlanadi.

use super::theme;
use egui::{RichText, Ui};
use rdcore::{
    auth::{self, Credentials, MIN_PASSWORD_LEN},
    cert::HostIdentity,
    hexfmt, paths,
};

const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

pub struct HostPanel {
    new_password: String,
    port_text: String,
    port_msg: Option<(bool, String)>,
    show_new: bool,
    /// Shu ish seansida o'rnatilgan parol (faqat xotirada, ko'rsatish uchun).
    last_set: Option<String>,
    show_last: bool,
    message: Option<(bool, String)>,
    fingerprint: Option<String>,
    ip: String,
    #[cfg(windows)]
    win: super::winhost::WinHost,
    #[cfg(target_os = "linux")]
    svc: Option<(std::time::Instant, Option<String>, bool)>,
    #[cfg(target_os = "linux")]
    svc_msg: Option<(bool, String)>,
}

fn local_ip() -> String {
    // UDP "ulanish" paket yubormaydi, faqat qaysi lokal manzil ishlatilishini aniqlaydi.
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "aniqlanmadi".into())
}

fn random_password() -> String {
    let b = auth::random_bytes::<16>();
    let chars: Vec<char> = b.iter().map(|x| ALPHABET[(*x & 31) as usize] as char).collect();
    chars.chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

#[cfg(target_os = "linux")]
fn service_state() -> Option<String> {
    let out = std::process::Command::new("systemctl")
        .args(["--user", "is-active", "remote-desktop-host"])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(target_os = "linux")]
fn service_installed() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "cat", "remote-desktop-host"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn restart_service() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "restart", "remote-desktop-host"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

impl HostPanel {
    pub fn new() -> Self {
        let fingerprint = paths::config_dir()
            .ok()
            .and_then(|d| HostIdentity::load_or_create(&d).ok())
            .map(|id| hexfmt::colon(&id.fingerprint()));
        HostPanel {
            new_password: String::new(),
            port_text: paths::read_host_port().unwrap_or(protocol::DEFAULT_PORT).to_string(),
            port_msg: None,
            show_new: false,
            last_set: None,
            show_last: false,
            message: None,
            fingerprint,
            ip: local_ip(),
            #[cfg(windows)]
            win: super::winhost::WinHost::new(),
            #[cfg(target_os = "linux")]
            svc: None,
            #[cfg(target_os = "linux")]
            svc_msg: None,
        }
    }

    fn password_is_set() -> bool {
        paths::host_password_file().map(|p| p.exists()).unwrap_or(false)
    }

    fn save_port(&mut self) {
        let port: u16 = match self.port_text.trim().parse() {
            Ok(p) if p >= 1024 => p,
            _ => {
                self.port_msg = Some((false, "Port 1024 dan 65535 gacha son bo'lishi kerak".into()));
                return;
            }
        };
        let result = (|| -> anyhow::Result<()> {
            let path = paths::host_port_file()?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, format!("{port}\n"))?;
            Ok(())
        })();
        self.port_msg = Some(match result {
            Ok(()) => {
                #[cfg(target_os = "linux")]
                let note = if service_state().as_deref().is_some_and(|s| s == "active" || s == "activating") {
                    if restart_service() {
                        " Host qayta ishga tushirildi."
                    } else {
                        " Host xizmatini qayta ishga tushirib bo'lmadi."
                    }
                } else {
                    " Host ishlamayapti: port keyingi ishga tushishda ishlatiladi."
                };
                #[cfg(windows)]
                let note = if self.win.restart_if_running() { " Host qayta ishga tushirildi." } else { "" };
                #[cfg(not(any(target_os = "linux", windows)))]
                let note = "";
                (
                    true,
                    format!("Port {port} saqlandi.{note} Router'da port yo'naltirish (forwarding) va xavfsizlik devorini (UDP {port}) yangilashni unutmang."),
                )
            }
            Err(e) => (false, format!("{e:#}")),
        });
    }

    fn save_password(&mut self) {
        let pw = self.new_password.clone();
        let result = (|| -> anyhow::Result<()> {
            let creds = Credentials::from_password(&pw)?;
            creds.save(&paths::host_password_file()?)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.last_set = Some(pw);
                self.new_password.clear();
                #[cfg(target_os = "linux")]
                let note = if service_state().as_deref().is_some_and(|s| s == "active" || s == "activating") {
                    if restart_service() {
                        " Host xizmati qayta ishga tushirildi."
                    } else {
                        " Host xizmatini qayta ishga tushirib bo'lmadi: `systemctl --user restart remote-desktop-host`."
                    }
                } else {
                    " Host xizmati ishlamayapti: parol keyingi ishga tushishda ishlatiladi."
                };
                #[cfg(windows)]
                let note = if self.win.restart_if_running() { " Host qayta ishga tushirildi." } else { " Host ishlamayapti: parol keyingi yoqilganda ishlatiladi." };
                #[cfg(not(any(target_os = "linux", windows)))]
                let note = "";
                self.message = Some((true, format!("Parol saqlandi.{note}")));
            }
            Err(e) => self.message = Some((false, format!("{e:#}"))),
        }
    }

    /// Yuqori paneldagi Host holati belgisi: (yoqilganmi, matn).
    pub fn header_status(&mut self) -> (bool, String) {
        #[cfg(windows)]
        {
            return match self.win.port() {
                Some(p) => (true, format!("Host yoqilgan · {p}")),
                None => (false, "Host o'chiq".to_string()),
            };
        }
        #[cfg(target_os = "linux")]
        {
            let on = self.service_state_cached().as_deref() == Some("active");
            return (on, if on { "Host yoqilgan".to_string() } else { "Host o'chiq".to_string() });
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        {
            (false, "Host mavjud emas".to_string())
        }
    }

    #[cfg(target_os = "linux")]
    fn service_state_cached(&mut self) -> Option<String> {
        let stale = self.svc.as_ref().map_or(true, |(t, _, _)| t.elapsed() > std::time::Duration::from_secs(3));
        if stale {
            self.svc = Some((std::time::Instant::now(), service_state(), service_installed()));
        }
        self.svc.as_ref().and_then(|(_, s, _)| s.clone())
    }

    /// Host xizmatini yoqadi (ishga tushiradi va avtostartga qo'shadi) yoki o'chiradi.
    #[cfg(target_os = "linux")]
    fn set_service(&mut self, on: bool) {
        let verb = if on { "enable" } else { "disable" };
        let ok = std::process::Command::new("systemctl")
            .args(["--user", verb, "--now", "remote-desktop-host"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        self.svc = None;
        self.svc_msg = Some(if ok {
            (true, if on { "Host yoqildi: ulanishlar qabul qilinadi.".into() } else { "Host o'chirildi.".into() })
        } else {
            (false, "Xizmatni boshqarib bo'lmadi. Terminalda: systemctl --user status remote-desktop-host".into())
        });
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        #[cfg(target_os = "linux")]
        let svc = self.service_state_cached();
        #[cfg(target_os = "linux")]
        let svc_installed = self.svc.as_ref().map_or(false, |s| s.2);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(640.0);
            ui.label(RichText::new("Bu kompyuter").size(20.0).strong());
            theme::hint(ui, "Boshqa kompyuterlar shu manzil, port va parol bilan ulana oladi.");
            ui.add_space(12.0);

            theme::card(ui, Some("HOLAT"), |ui| {
                ui.horizontal(|ui| {
                    #[cfg(target_os = "linux")]
                    {
                        match svc.as_deref() {
                            Some("active") => theme::chip(ui, "Host ishlayapti", theme::GOOD),
                            Some(other) => theme::chip(ui, format!("Host xizmati: {other}"), theme::WARN),
                            None => theme::chip(ui, "Aniqlab bo'lmadi", theme::DIM),
                        }
                    }
                    #[cfg(windows)]
                    {
                        match self.win.port() {
                            Some(_) => theme::chip(ui, "Host ishlayapti", theme::GOOD),
                            None => theme::chip(ui, "Host o'chiq", theme::DIM),
                        }
                    }
                    #[cfg(not(any(target_os = "linux", windows)))]
                    {
                        theme::chip(ui, "Host mavjud emas", theme::DIM);
                    }
                    if Self::password_is_set() {
                        theme::chip(ui, "Parol o'rnatilgan", theme::GOOD);
                    } else {
                        theme::chip(ui, "Parol o'rnatilmagan", theme::BAD);
                    }
                });
                ui.add_space(8.0);
                let addr = format!("{}:{}", self.ip, paths::read_host_port().unwrap_or(protocol::DEFAULT_PORT));
                ui.horizontal(|ui| {
                    ui.label(theme::dim("Manzil"));
                    ui.label(RichText::new(&addr).monospace().size(16.0).strong());
                    if ui.small_button("Nusxa").clicked() {
                        ui.ctx().output_mut(|o| o.copied_text = addr.clone());
                    }
                });
                match &self.fingerprint {
                    Some(fp) => {
                        let half = fp.len() / 2;
                        let split = fp[..half].rfind(':').map(|i| i + 1).unwrap_or(half);
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(theme::dim("Fingerprint"));
                            if ui.small_button("Nusxa").clicked() {
                                ui.ctx().output_mut(|o| o.copied_text = fp.clone());
                            }
                        });
                        ui.label(RichText::new(&fp[..split]).monospace().size(11.5).color(theme::DIM));
                        ui.label(RichText::new(&fp[split..]).monospace().size(11.5).color(theme::DIM));
                        theme::hint(ui, "Yangi Host'ga birinchi ulanishda Viewer shu qiymatni ko'rsatadi: solishtirib tasdiqlang.");
                    }
                    None => {
                        ui.label(theme::dim("Fingerprint hali yaratilmagan"));
                    }
                }
            });

            #[cfg(windows)]
            theme::card(ui, Some("ULANISHGA RUXSAT"), |ui| {
                self.win.ui(ui);
            });

            #[cfg(target_os = "linux")]
            theme::card(ui, Some("ULANISHGA RUXSAT"), |ui| {
                if !svc_installed {
                    ui.label(RichText::new("Host xizmati o'rnatilmagan.").color(theme::WARN));
                    theme::hint(ui, "Terminalda loyiha papkasida ishga tushiring: bash ./scripts/install-host.sh");
                } else {
                    let mut on = matches!(svc.as_deref(), Some("active") | Some("activating"));
                    let label = if on { "Ulanishga ruxsat berilgan" } else { "Ulanishga ruxsat berish" };
                    if ui.checkbox(&mut on, label).changed() {
                        self.set_service(on);
                    }
                    theme::hint(ui, "Yoqilsa, Host kompyuter ishga tushganda ham o'zi yonadi.");
                }
                if let Some((ok, text)) = &self.svc_msg {
                    ui.add_space(4.0);
                    ui.label(RichText::new(text).color(if *ok { theme::GOOD } else { theme::BAD }));
                }
            });

            theme::card(ui, Some("PORT (UDP VA TCP)"), |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.port_text).desired_width(80.0));
                    if ui.button("Saqlash").clicked() {
                        self.save_port();
                    }
                    if ui.button(format!("Standart ({})", protocol::DEFAULT_PORT)).clicked() {
                        self.port_text = protocol::DEFAULT_PORT.to_string();
                    }
                });
                if let Some((ok, text)) = &self.port_msg {
                    ui.add_space(4.0);
                    ui.label(RichText::new(text).color(if *ok { theme::GOOD } else { theme::BAD }));
                }
            });

            theme::card(ui, Some("PAROL"), |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_password)
                            .password(!self.show_new)
                            .desired_width(230.0)
                            .hint_text(format!("kamida {MIN_PASSWORD_LEN} belgi")),
                    );
                    let eye = if self.show_new { "Yashirish" } else { "Ko'rsatish" };
                    if ui.small_button(eye).clicked() {
                        self.show_new = !self.show_new;
                    }
                    if ui.button("Tasodifiy").clicked() {
                        self.new_password = random_password();
                        self.show_new = true;
                    }
                });
                ui.add_space(4.0);
                let ok_len = self.new_password.chars().count() >= MIN_PASSWORD_LEN;
                if theme::accent_button(ui, "Parolni saqlash", egui::vec2(150.0, 32.0), ok_len).clicked() {
                    self.save_password();
                }
                if let Some((ok, text)) = &self.message {
                    ui.add_space(4.0);
                    ui.label(RichText::new(text).color(if *ok { theme::GOOD } else { theme::BAD }));
                }
                if let Some(last) = self.last_set.clone() {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.label(theme::dim("Oxirgi o'rnatilgan"));
                        if self.show_last {
                            ui.monospace(&last);
                        } else {
                            ui.monospace("••••••••");
                        }
                        if ui.small_button(if self.show_last { "Yashirish" } else { "Ko'rsatish" }).clicked() {
                            self.show_last = !self.show_last;
                        }
                        if ui.small_button("Nusxa").clicked() {
                            ui.ctx().output_mut(|o| o.copied_text = last.clone());
                        }
                    });
                    theme::hint(ui, "Parol faqat xesh ko'rinishida saqlanadi; bu yerda dastur yopilguncha ko'rinadi.");
                }
            });
        });
    }
}
