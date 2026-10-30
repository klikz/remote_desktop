//! "Bu kompyuter" bo'limi: ushbu kompyuter Host sifatida qanday ulanishni qabul qilishini ko'rsatadi
//! (manzil, fingerprint, holat) va ulanish parolini o'rnatish imkonini beradi.
//! Parol faqat xesh (argon2 kalit) ko'rinishida saqlanadi, shuning uchun keyin ko'rsatib bo'lmaydi:
//! o'rnatilgan parol faqat shu dastur ochiq turganda, ko'rsatish uchun xotirada saqlanadi.

use egui::{Color32, RichText, Ui};
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

    pub fn ui(&mut self, ui: &mut Ui) {
        ui.add_space(8.0);
        ui.heading("Bu kompyuter");
        ui.label("Boshqa kompyuterlar shu manzil, port va parol bilan ulana oladi.");
        ui.add_space(10.0);

        egui::Grid::new("hostinfo").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Holat");
            #[cfg(target_os = "linux")]
            {
                match service_state().as_deref() {
                    Some("active") => ui.colored_label(Color32::from_rgb(110, 200, 120), "Host ishlayapti (tinglamoqda)"),
                    Some(other) => ui.colored_label(Color32::from_rgb(235, 170, 80), format!("Host xizmati: {other}")),
                    None => ui.weak("Aniqlab bo'lmadi"),
                };
            }
            #[cfg(windows)]
            {
                match self.win.port() {
                    Some(_) => ui.colored_label(Color32::from_rgb(110, 200, 120), "Host ishlayapti (tinglamoqda)"),
                    None => ui.weak("Host o'chiq: ulanishga ruxsat berilmagan"),
                };
            }
            #[cfg(not(any(target_os = "linux", windows)))]
            {
                ui.weak("Bu platformada Host mavjud emas");
            }
            ui.end_row();

            ui.label("Manzil");
            ui.horizontal(|ui| {
                let addr = format!("{}:{}", self.ip, paths::read_host_port().unwrap_or(protocol::DEFAULT_PORT));
                ui.monospace(&addr);
                if ui.small_button("Nusxa").clicked() {
                    ui.ctx().output_mut(|o| o.copied_text = addr.clone());
                }
            });
            ui.end_row();

            ui.label("Fingerprint");
            match &self.fingerprint {
                Some(fp) => {
                    let half = fp.len() / 2;
                    let split = fp[..half].rfind(':').map(|i| i + 1).unwrap_or(half);
                    ui.vertical(|ui| {
                        ui.monospace(&fp[..split]);
                        ui.monospace(&fp[split..]);
                        if ui.small_button("Nusxa").clicked() {
                            ui.ctx().output_mut(|o| o.copied_text = fp.clone());
                        }
                    });
                }
                None => {
                    ui.weak("Hali yaratilmagan");
                }
            }
            ui.end_row();

            ui.label("Parol");
            if Self::password_is_set() {
                ui.colored_label(Color32::from_rgb(110, 200, 120), "o'rnatilgan");
            } else {
                ui.colored_label(Color32::from_rgb(235, 110, 100), "o'rnatilmagan: ulanishni hech kim qabul qila olmaydi");
            }
            ui.end_row();
        });

        ui.add_space(6.0);
        ui.weak("Yangi Host'ga birinchi ulanishda Viewer shu fingerprint'ni ko'rsatadi: solishtirib tasdiqlang.");
        ui.separator();

        #[cfg(windows)]
        {
            self.win.ui(ui);
            ui.separator();
        }

        ui.label(RichText::new("Port (UDP)").strong());
        ui.add_space(4.0);
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
            let color = if *ok { Color32::from_rgb(110, 200, 120) } else { Color32::from_rgb(235, 110, 100) };
            ui.colored_label(color, text);
        }
        ui.separator();

        ui.label(RichText::new("Parolni o'rnatish yoki o'zgartirish").strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_password)
                    .password(!self.show_new)
                    .hint_text(format!("kamida {MIN_PASSWORD_LEN} belgi")),
            );
            ui.checkbox(&mut self.show_new, "Ko'rsatish");
            if ui.button("Tasodifiy yaratish").clicked() {
                self.new_password = random_password();
                self.show_new = true;
            }
        });
        ui.add_space(4.0);
        let ok_len = self.new_password.chars().count() >= MIN_PASSWORD_LEN;
        if ui.add_enabled(ok_len, egui::Button::new("Saqlash")).clicked() {
            self.save_password();
        }
        if let Some((ok, text)) = &self.message {
            ui.add_space(6.0);
            let color = if *ok { Color32::from_rgb(110, 200, 120) } else { Color32::from_rgb(235, 110, 100) };
            ui.colored_label(color, text);
        }
        if let Some(last) = self.last_set.clone() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Oxirgi o'rnatilgan parol:");
                if self.show_last {
                    ui.monospace(&last);
                } else {
                    ui.monospace("••••••••");
                }
                ui.checkbox(&mut self.show_last, "Ko'rsatish");
                if ui.small_button("Nusxa").clicked() {
                    ui.ctx().output_mut(|o| o.copied_text = last.clone());
                }
            });
            ui.weak("Parol xavfsizlik uchun faqat xesh ko'rinishida saqlanadi. Bu yerda u dastur yopilguncha ko'rinadi.");
        }
    }
}
