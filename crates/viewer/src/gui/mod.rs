//! Viewer'ning oynali interfeysi (egui): ulanish oynasi, saqlangan Host'lar, sifat sozlamalari,
//! fingerprint tasdig'i va ulanish ichidagi panel.

mod firewall;
mod hostpanel;
#[cfg(windows)]
mod winhost;
mod hosts;
mod input;
mod keys;
mod ondemand;
#[cfg(target_os = "linux")]
mod linuxhost;
mod relaypanel;
mod session;
mod theme;
#[cfg(windows)]
mod tray;

use egui::{Align, Align2, Color32, Key, Layout, RichText, Sense, TextureHandle, TextureOptions, ViewportCommand};
use hosts::SavedHost;
use rdcore::client::Transport;
use std::{
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

pub fn run() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Remote Desktop")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native("Remote Desktop", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
        .map_err(|e| anyhow::anyhow!("oyna ochilmadi: {e}"))
}

/// Sifat shablonlari: (nom, fps, kbps, balandlik; 0 = asl o'lcham)
const PRESETS: [(&str, u16, u32, u16); 5] = [
    ("Past · 720p/15", 15, 2500, 720),
    ("O'rta · 1080p/30", 30, 8000, 1080),
    ("Yuqori · 1080p/30", 30, 15000, 1080),
    ("Matn · 1440p/20", 20, 20_000, 1440),
    ("Maksimal · asl", 30, 40_000, 0),
];
const HEIGHTS: [(u16, &str); 5] = [(720, "720p"), (1080, "1080p"), (1440, "1440p"), (2160, "2160p"), (0, "Asl o'lcham")];

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Connect,
    Host,
    Relay,
    ThisPc,
}

#[derive(PartialEq)]
enum Screen {
    Form,
    Connecting,
    Viewing,
}

struct Form {
    name: String,
    addr: String,
    port: String,
    password: String,
    transport: Transport,
    show_password: bool,
    fps: u16,
    kbps: u32,
    height: u16,
    input: bool,
    save: bool,
}

struct Stats {
    last: Instant,
    frames: u64,
    bytes: u64,
    fps: f64,
    kbps: f64,
}

struct App {
    tab: Tab,
    host: hostpanel::HostPanel,
    relay: relaypanel::RelayPanel,
    /// Server orqali ulanayotgan ID (muvaffaqiyatda kontaktlarga saqlanadi).
    relay_target: Option<String>,
    screen: Screen,
    hosts: Vec<SavedHost>,
    selected: Option<usize>,
    form: Form,
    error: Option<String>,
    handle: Option<session::Handle>,
    confirm: Option<(String, mpsc::Sender<bool>)>,
    fingerprint: String,
    title: String,
    input_on: bool,
    tex: Option<TextureHandle>,
    fwd: input::Forwarder,
    stats: Stats,
    #[cfg(windows)]
    tray: Option<tray::Tray>,
    /// Oyna trayga yashirilgan.
    hidden: bool,
    quitting: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let hosts = hosts::load();
        let mut app = App {
            tab: Tab::Connect,
            host: hostpanel::HostPanel::new(),
            relay: relaypanel::RelayPanel::new(),
            relay_target: None,
            screen: Screen::Form,
            hosts,
            selected: None,
            form: Form {
                name: String::new(),
                addr: String::new(),
                port: protocol::DEFAULT_PORT.to_string(),
                password: String::new(),
                transport: Transport::Auto,
                show_password: false,
                fps: 30,
                kbps: 8000,
                height: 1080,
                input: true,
                save: true,
            },
            error: None,
            handle: None,
            confirm: None,
            fingerprint: String::new(),
            title: String::new(),
            input_on: true,
            tex: None,
            fwd: input::Forwarder::default(),
            stats: Stats { last: Instant::now(), frames: 0, bytes: 0, fps: 0.0, kbps: 0.0 },
            #[cfg(windows)]
            tray: tray::Tray::new(&cc.egui_ctx),
            hidden: false,
            quitting: false,
        };
        if !app.hosts.is_empty() {
            app.select(0);
        }
        app
    }

    fn select(&mut self, i: usize) {
        if let Some(h) = self.hosts.get(i).cloned() {
            self.selected = Some(i);
            self.form.name = h.name;
            match h.addr.rsplit_once(':') {
                Some((host, port)) if !host.contains(':') && port.parse::<u16>().is_ok() => {
                    self.form.addr = host.to_string();
                    self.form.port = port.to_string();
                }
                _ => {
                    self.form.addr = h.addr;
                    self.form.port = protocol::DEFAULT_PORT.to_string();
                }
            }
            self.form.fps = h.fps;
            self.form.kbps = h.kbps;
            self.form.height = h.height;
            self.form.input = h.input;
            self.form.password.clear();
        }
    }

    /// Manzil + port (`IP:port`). Manzil maydoniga port allaqachon yozilgan bo'lsa, o'sha ishlatiladi.
    fn full_addr(&self) -> String {
        let host = self.form.addr.trim();
        if host.contains(':') {
            return host.to_string();
        }
        let port = self.form.port.trim();
        format!("{host}:{}", if port.is_empty() { protocol::DEFAULT_PORT.to_string() } else { port.to_string() })
    }

    fn connect(&mut self, ctx: &egui::Context) {
        let full = self.full_addr();
        let f = &self.form;
        if f.addr.trim().is_empty() {
            self.error = Some("Manzilni kiriting".into());
            return;
        }
        self.error = None;
        self.fingerprint.clear();
        if self.form.port.trim().parse::<u16>().is_err() && !f.addr.contains(':') {
            self.error = Some("Port 1 dan 65535 gacha son bo'lishi kerak".into());
            return;
        }
        self.title = if f.name.trim().is_empty() { full.clone() } else { format!("{} ({})", f.name.trim(), full) };
        self.input_on = f.input;
        self.tex = None;
        self.fwd = input::Forwarder::default();
        self.relay_target = None;
        self.handle = Some(session::start(
            ctx.clone(),
            session::Params {
                addr: full,
                password: f.password.clone(),
                fps: f.fps,
                kbps: f.kbps,
                height: f.height,
                transport: f.transport,
                relay: None,
            },
        ));
        self.screen = Screen::Connecting;
    }

    /// Server (relay) orqali ID ga ulanadi. Sifat sozlamalari "Ulanish" bo'limidagi formadan olinadi.
    fn connect_relay(&mut self, ctx: &egui::Context, req: relaypanel::ConnectRequest) {
        let Some(connector) = self.relay.connector() else {
            self.error = Some("Server bilan aloqa yo'q".into());
            return;
        };
        let f = &self.form;
        self.error = None;
        self.fingerprint.clear();
        self.title = format!("ID {}", rdcore::relay::pretty_id(&req.id));
        self.input_on = f.input;
        self.tex = None;
        self.fwd = input::Forwarder::default();
        self.relay_target = Some(req.id.clone());
        self.handle = Some(session::start(
            ctx.clone(),
            session::Params {
                addr: String::new(),
                password: req.password,
                fps: f.fps,
                kbps: f.kbps,
                height: f.height,
                transport: Transport::Auto,
                relay: Some((req.id, connector)),
            },
        ));
        self.screen = Screen::Connecting;
    }

    fn remember_host(&mut self) {
        if !self.form.save {
            return;
        }
        let full = self.full_addr();
        let f = &self.form;
        let entry = SavedHost {
            name: if f.name.trim().is_empty() { full.clone() } else { f.name.trim().to_string() },
            addr: full,
            fps: f.fps,
            kbps: f.kbps,
            height: f.height,
            input: f.input,
        };
        match self.hosts.iter().position(|h| h.addr == entry.addr) {
            Some(i) => {
                self.hosts[i] = entry;
                self.selected = Some(i);
            }
            None => {
                self.hosts.push(entry);
                self.selected = Some(self.hosts.len() - 1);
            }
        }
        if let Err(e) = hosts::save(&self.hosts) {
            eprintln!("Host ro'yxatini saqlab bo'lmadi: {e}");
        }
    }

    fn disconnect(&mut self, ctx: &egui::Context) {
        if let Some(h) = &self.handle {
            self.fwd.release_all(&mut |ev| h.input(ev));
            h.close();
        }
        self.handle = None;
        self.confirm = None;
        self.tex = None;
        self.screen = Screen::Form;
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
    }

    fn poll_session(&mut self, ctx: &egui::Context) {
        let mut ended: Option<Option<String>> = None;
        let mut connected = false;
        if let Some(h) = &self.handle {
            while let Ok(ev) = h.events.try_recv() {
                match ev {
                    session::Event::Confirm(fp, tx) => self.confirm = Some((fp, tx)),
                    session::Event::Connected { fingerprint, via_tcp } => {
                        if via_tcp {
                            self.title.push_str(" [TCP]");
                        }
                        self.fingerprint = fingerprint;
                        connected = true;
                    }
                    session::Event::VideoStarted => self.screen = Screen::Viewing,
                    session::Event::Ended(msg) => ended = Some(msg),
                }
            }
        }
        if connected {
            // parol to'g'ri, Host ishonchli: ro'yxatga saqlanadi (yoki sozlamalari yangilanadi)
            match self.relay_target.clone() {
                Some(id) => self.relay.remember_contact(&id),
                None => self.remember_host(),
            }
        }
        if let Some(msg) = ended {
            let was_viewing = self.screen == Screen::Viewing;
            self.disconnect(ctx);
            self.error = match msg {
                Some(m) => Some(m),
                None if was_viewing => Some("Ulanish yopildi".into()),
                None => None,
            };
        }
    }

    fn ui_form(&mut self, ctx: &egui::Context) {
        let (host_on, host_label) = self.host.header_status();
        egui::TopBottomPanel::top("header").frame(theme::header_frame()).show(ctx, |ui| {
            ui.horizontal(|ui| {
                theme::dot(ui, theme::ACCENT);
                ui.label(RichText::new("Remote Desktop").size(16.0).strong());
                ui.add_space(14.0);
                if theme::tab(ui, self.tab == Tab::Connect, "P2P").clicked() {
                    self.tab = Tab::Connect;
                }
                if theme::tab(ui, self.tab == Tab::Relay, "Server orqali").clicked() {
                    self.tab = Tab::Relay;
                }
                if theme::tab(ui, self.tab == Tab::Host, "HOST").clicked() {
                    self.tab = Tab::Host;
                }
                if theme::tab(ui, self.tab == Tab::ThisPc, "Bu kompyuter").clicked() {
                    self.tab = Tab::ThisPc;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    theme::chip(ui, host_label, if host_on { theme::GOOD } else { theme::DIM });
                    if self.relay.server_running() {
                        theme::chip(ui, "Server: ishlayapti", theme::GOOD);
                    }
                    if self.relay.is_online() {
                        theme::chip(ui, "Server: onlayn", theme::GOOD);
                    }
                });
            });
        });
        if self.tab == Tab::ThisPc {
            egui::CentralPanel::default().frame(theme::page_frame()).show(ctx, |ui| self.host.ui(ui));
            return;
        }
        if self.tab == Tab::Host {
            egui::CentralPanel::default().frame(theme::page_frame()).show(ctx, |ui| self.relay.ui_server(ui));
            return;
        }
        if self.tab == Tab::Relay {
            let mut req = None;
            egui::CentralPanel::default().frame(theme::page_frame()).show(ctx, |ui| req = self.relay.ui(ui));
            if let Some(r) = req {
                self.connect_relay(ctx, r);
            }
            return;
        }

        let mut do_connect = false;
        let mut select: Option<usize> = None;
        let mut connect_idx: Option<usize> = None;
        let mut delete: Option<usize> = None;
        let mut new_conn = false;

        // --- Saqlangan Host'lar (chap panel)
        egui::SidePanel::left("hosts").exact_width(250.0).resizable(false).frame(theme::side_frame()).show(ctx, |ui| {
            ui.label(RichText::new("SAQLANGAN HOST'LAR").size(11.0).color(theme::DIM).strong());
            ui.add_space(6.0);
            let list_h = (ui.available_height() - 50.0).max(40.0);
            egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, false]).show(ui, |ui| {
                if self.hosts.is_empty() {
                    ui.label(
                        RichText::new("Hozircha yo'q. Muvaffaqiyatli ulanishdan keyin shu yerda paydo bo'ladi.")
                            .size(12.5)
                            .color(theme::DIM),
                    );
                }
                for (i, h) in self.hosts.iter().enumerate() {
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 50.0), Sense::click());
                    let selected = self.selected == Some(i);
                    let fill = if selected {
                        theme::ACCENT.gamma_multiply(0.22)
                    } else if resp.hovered() {
                        theme::CARD
                    } else {
                        Color32::TRANSPARENT
                    };
                    let stroke = if selected { egui::Stroke::new(1.0, theme::ACCENT) } else { egui::Stroke::NONE };
                    let p = ui.painter_at(rect);
                    p.rect(rect, egui::Rounding::same(8.0), fill, stroke);
                    p.text(rect.min + egui::vec2(12.0, 9.0), Align2::LEFT_TOP, &h.name, egui::FontId::proportional(14.0), theme::TEXT);
                    p.text(rect.min + egui::vec2(12.0, 29.0), Align2::LEFT_TOP, &h.addr, egui::FontId::monospace(11.5), theme::DIM);
                    let x_rect = egui::Rect::from_center_size(egui::pos2(rect.right() - 16.0, rect.center().y), egui::vec2(24.0, 24.0));
                    let xr = ui.interact(x_rect, ui.id().with(("del", i)), Sense::click());
                    if resp.hovered() || xr.hovered() {
                        let c = if xr.hovered() { theme::BAD } else { theme::DIM };
                        p.text(x_rect.center(), Align2::CENTER_CENTER, "×", egui::FontId::proportional(17.0), c);
                    }
                    if xr.clicked() {
                        delete = Some(i);
                    } else if resp.double_clicked() {
                        connect_idx = Some(i);
                    } else if resp.clicked() {
                        select = Some(i);
                    }
                    ui.add_space(3.0);
                }
            });
            ui.add_space(4.0);
            if ui.add_sized([ui.available_width(), 34.0], egui::Button::new("+  Yangi ulanish")).clicked() {
                new_conn = true;
            }
        });

        // --- Ulanish formasi
        egui::CentralPanel::default().frame(theme::page_frame()).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(620.0);
                let title = match self.selected.and_then(|i| self.hosts.get(i)) {
                    Some(h) => h.name.clone(),
                    None => "Yangi ulanish".to_string(),
                };
                ui.label(RichText::new(title).size(20.0).strong());
                theme::hint(ui, "Host manzili va parolini kiriting. Parol hech qayerda saqlanmaydi.");
                ui.add_space(12.0);

                let mut pw_enter = false;
                theme::card(ui, Some("HOST"), |ui| {
                    egui::Grid::new("form").num_columns(2).spacing([14.0, 10.0]).min_col_width(70.0).show(ui, |ui| {
                        ui.label(theme::dim("Nom"));
                        ui.add(egui::TextEdit::singleline(&mut self.form.name).desired_width(330.0).hint_text("masalan: Ofis Ubuntu"));
                        ui.end_row();
                        ui.label(theme::dim("Manzil"));
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.form.addr).desired_width(220.0).hint_text("IP yoki nom"));
                            ui.label(theme::dim("Port"));
                            ui.add(egui::TextEdit::singleline(&mut self.form.port).desired_width(64.0));
                        });
                        ui.end_row();
                        ui.label(theme::dim("Parol"));
                        ui.horizontal(|ui| {
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut self.form.password)
                                    .password(!self.form.show_password)
                                    .desired_width(240.0),
                            );
                            pw_enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                            let eye = if self.form.show_password { "Yashirish" } else { "Ko'rsatish" };
                            if ui.small_button(eye).clicked() {
                                self.form.show_password = !self.form.show_password;
                            }
                        });
                        ui.end_row();
                    });
                });

                theme::card(ui, Some("ULANISH VA SIFAT"), |ui| {
                    let tname = |t: Transport| match t {
                        Transport::Auto => "Avto",
                        Transport::Udp => "UDP",
                        Transport::Tcp => "TCP",
                    };
                    ui.horizontal(|ui| {
                        ui.label(theme::dim("Transport"));
                        for t in [Transport::Auto, Transport::Udp, Transport::Tcp] {
                            ui.selectable_value(&mut self.form.transport, t, tname(t));
                        }
                    });
                    theme::hint(
                        ui,
                        match self.form.transport {
                            Transport::Auto => "Avval UDP (tez), bo'lmasa TCP orqali ulanadi.",
                            Transport::Udp => "Eng tez; UDP bloklangan tarmoqlarda ishlamaydi.",
                            Transport::Tcp => "UDP bloklangan tarmoqlar uchun; biroz sekinroq.",
                        },
                    );
                    ui.add_space(8.0);
                    ui.label(theme::dim("Sifat"));
                    let current = PRESETS
                        .iter()
                        .find(|p| p.1 == self.form.fps && p.2 == self.form.kbps && p.3 == self.form.height)
                        .map(|p| p.0);
                    ui.horizontal_wrapped(|ui| {
                        for p in PRESETS {
                            if ui.selectable_label(current == Some(p.0), p.0).clicked() {
                                self.form.fps = p.1;
                                self.form.kbps = p.2;
                                self.form.height = p.3;
                            }
                        }
                    });
                    egui::CollapsingHeader::new("Qo'shimcha sozlamalar").default_open(false).show(ui, |ui| {
                        egui::Grid::new("quality").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                            ui.label(theme::dim("FPS"));
                            ui.add(egui::Slider::new(&mut self.form.fps, 5..=60));
                            ui.end_row();
                            ui.label(theme::dim("Bitrate"));
                            ui.add(egui::Slider::new(&mut self.form.kbps, 500..=50_000).logarithmic(true).suffix(" kbit/s"));
                            ui.end_row();
                            ui.label(theme::dim("O'lcham"));
                            let hname = HEIGHTS.iter().find(|h| h.0 == self.form.height).map(|h| h.1).unwrap_or("Boshqa");
                            egui::ComboBox::from_id_salt("height").selected_text(hname).show_ui(ui, |ui| {
                                for (v, n) in HEIGHTS {
                                    ui.selectable_value(&mut self.form.height, v, n);
                                }
                            });
                            ui.end_row();
                        });
                    });
                    ui.add_space(8.0);
                    ui.checkbox(&mut self.form.input, "Sichqoncha va klaviatura bilan boshqarish");
                    ui.checkbox(&mut self.form.save, "Ro'yxatga saqlash");
                });

                if let Some(e) = &self.error {
                    theme::error_banner(ui, e);
                    ui.add_space(10.0);
                }
                let can = !self.form.addr.trim().is_empty() && !self.form.password.is_empty();
                if theme::primary_button(ui, "Ulanish", can).clicked() || (pw_enter && can) {
                    do_connect = true;
                }
            });
        });

        if let Some(i) = delete {
            if i < self.hosts.len() {
                self.hosts.remove(i);
                self.selected = None;
                let _ = hosts::save(&self.hosts);
            }
        }
        if let Some(i) = select.or(connect_idx) {
            self.select(i);
        }
        if connect_idx.is_some() && !self.form.password.is_empty() {
            do_connect = true;
        }
        if new_conn {
            self.selected = None;
            self.form.name.clear();
            self.form.addr.clear();
            self.form.password.clear();
            self.form.port = protocol::DEFAULT_PORT.to_string();
            self.error = None;
        }
        if do_connect {
            self.connect(ctx);
        }
    }

    fn ui_connecting(&mut self, ctx: &egui::Context) {
        let mut cancel = false;
        egui::CentralPanel::default().frame(theme::page_frame()).show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() / 3.0).max(20.0));
                ui.add(egui::Spinner::new().size(34.0).color(theme::ACCENT));
                ui.add_space(12.0);
                let what = if self.fingerprint.is_empty() { "Ulanilmoqda" } else { "Video kutilmoqda" };
                ui.label(RichText::new(what).size(17.0).strong());
                ui.label(theme::dim(&self.title));
                if self.relay_target.is_some() && self.fingerprint.is_empty() {
                    theme::hint(ui, "Qabul qiluvchi ruxsat berishi kutilmoqda (30 soniyagacha)");
                }
                ui.add_space(14.0);
                cancel = ui.add(egui::Button::new("Bekor qilish").min_size(egui::vec2(140.0, 32.0))).clicked();
            });
        });
        if cancel {
            self.disconnect(ctx);
        }
    }

    fn ui_confirm(&mut self, ctx: &egui::Context) {
        let Some((fp, _)) = &self.confirm else { return };
        let fp = fp.clone();
        let mut answer = None;
        egui::Window::new("Yangi Host")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Bu Host'ga birinchi marta ulanyapsiz.");
                theme::hint(ui, "Fingerprint'ni Host ekranidagi qiymat bilan solishtiring:");
                ui.add_space(6.0);
                let half = fp.len() / 2;
                let split = fp[..half].rfind(':').map(|i| i + 1).unwrap_or(half);
                egui::Frame::none()
                    .fill(theme::BG)
                    .stroke(egui::Stroke::new(1.0, theme::BORDER))
                    .rounding(egui::Rounding::same(8.0))
                    .inner_margin(egui::Margin::same(10.0))
                    .show(ui, |ui| {
                        ui.monospace(&fp[..split]);
                        ui.monospace(&fp[split..]);
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if theme::accent_button(ui, "Ha, ishonaman", egui::vec2(140.0, 32.0), true).clicked() {
                        answer = Some(true);
                    }
                    if ui.add(egui::Button::new("Yo'q").min_size(egui::vec2(80.0, 32.0))).clicked() {
                        answer = Some(false);
                    }
                });
            });
        if let Some(a) = answer {
            if let Some((_, tx)) = self.confirm.take() {
                let _ = tx.send(a);
            }
        }
    }

    fn ui_viewing(&mut self, ctx: &egui::Context) {
        let Some(handle) = &self.handle else { return };
        let shared = handle.shared.clone();

        // Statistika (har soniyada)
        if self.stats.last.elapsed() >= Duration::from_secs(1) {
            let secs = self.stats.last.elapsed().as_secs_f64();
            let frames = shared.frames.load(Ordering::Relaxed);
            let bytes = shared.bytes.load(Ordering::Relaxed);
            self.stats.fps = (frames - self.stats.frames.min(frames)) as f64 / secs;
            self.stats.kbps = (bytes - self.stats.bytes.min(bytes)) as f64 * 8.0 / 1000.0 / secs;
            self.stats.frames = frames;
            self.stats.bytes = bytes;
            self.stats.last = Instant::now();
        }
        let rtt = shared.rtt_us.load(Ordering::Relaxed) as f64 / 1000.0;

        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let show_bar = !fullscreen || hover.map_or(false, |p| p.y < 40.0);
        let mut want_disconnect = false;
        if show_bar {
            let rtt_color = if rtt < 60.0 {
                theme::GOOD
            } else if rtt < 150.0 {
                theme::WARN
            } else {
                theme::BAD
            };
            egui::TopBottomPanel::top("bar").frame(theme::bar_frame()).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    theme::dot(ui, theme::GOOD);
                    ui.label(RichText::new(&self.title).size(13.5).strong());
                    ui.add_space(6.0);
                    theme::chip(ui, format!("{:.0} FPS", self.stats.fps), theme::DIM);
                    theme::chip(ui, format!("{:.1} Mbit/s", self.stats.kbps / 1000.0), theme::DIM);
                    theme::chip(ui, format!("{:.0} ms", rtt), rtt_color);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::danger_button(ui, "Uzish").clicked() {
                            want_disconnect = true;
                        }
                        let fs_text = if fullscreen { "Oynaga qaytish" } else { "To'liq ekran" };
                        if ui.button(fs_text).clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen));
                        }
                        if ui.selectable_label(self.input_on, "Boshqaruv").clicked() {
                            self.input_on = !self.input_on;
                        }
                    });
                });
            });
        }

        // Yangi kadr bo'lsa tekstura yangilanadi
        if let Some(pic) = shared.picture.lock().unwrap().take() {
            let img = egui::ColorImage::from_rgb([pic.w, pic.h], &pic.rgb);
            match &mut self.tex {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => self.tex = Some(ctx.load_texture("video", img, TextureOptions::LINEAR)),
            }
        }

        let mut video_rect = egui::Rect::NOTHING;
        egui::CentralPanel::default().frame(egui::Frame::none().fill(Color32::BLACK)).show(ctx, |ui| {
            let avail = ui.max_rect();
            match &self.tex {
                Some(tex) => {
                    let [w, h] = tex.size();
                    let scale = (avail.width() / w as f32).min(avail.height() / h as f32);
                    let size = egui::vec2(w as f32 * scale, h as f32 * scale);
                    video_rect = egui::Rect::from_center_size(avail.center(), size);
                    ui.painter().image(
                        tex.id(),
                        video_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().text(avail.center(), Align2::CENTER_CENTER, "Video kutilmoqda...", egui::FontId::proportional(18.0), Color32::GRAY);
                }
            }
        });

        // Boshqaruv: panel tugmalari klaviatura fokusini olib qolmasin (aks holda Space/Enter ularni bosadi)
        if self.input_on {
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
            let (events, mods, focused) = ctx.input(|i| (i.events.clone(), i.modifiers, i.focused));
            let h = self.handle.as_ref().unwrap();
            self.fwd.process(&events, mods, focused, video_rect, hover, &mut |ev| h.input(ev));
        } else if let Some(h) = &self.handle {
            self.fwd.release_all(&mut |ev| h.input(ev));
        }

        if want_disconnect {
            self.disconnect(ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

impl App {
    /// Tray: oyna yopilsa/pastga tushirilsa (agent yoqilgan bo'lsa) yashirinadi; ikonka orqali qaytadi.
    #[cfg(windows)]
    fn tray_update(&mut self, ctx: &egui::Context) {
        let Some(tray) = self.tray.as_mut() else { return };
        tray.set_status(&self.relay.status_line());
        let mut show = false;
        for c in tray.poll() {
            match c {
                tray::Cmd::Show => show = true,
                tray::Cmd::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
        // Ruxsat so'rovi kelsa yashirin oynani ochamiz (30 soniyada javob berish uchun)
        if self.hidden && self.relay.has_incoming() {
            show = true;
        }
        if show {
            self.hidden = false;
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        let (close, minimized) = ctx.input(|i| (i.viewport().close_requested(), i.viewport().minimized == Some(true)));
        if !self.quitting && !self.hidden && self.relay.agent_active() && (close || minimized) {
            if close {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
            self.hidden = true;
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }
    }

    #[cfg(not(windows))]
    fn tray_update(&mut self, _ctx: &egui::Context) {}
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.tray_update(ctx);
        self.poll_session(ctx);
        self.relay.poll(ctx);
        match self.screen {
            Screen::Form => self.ui_form(ctx),
            Screen::Connecting => self.ui_connecting(ctx),
            Screen::Viewing => self.ui_viewing(ctx),
        }
        self.ui_confirm(ctx);
        self.relay.incoming_dialog(ctx);
    }
}
