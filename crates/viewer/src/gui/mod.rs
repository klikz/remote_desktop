//! Viewer'ning oynali interfeysi (egui): ulanish oynasi, saqlangan Host'lar, sifat sozlamalari,
//! va ulanish ichidagi panel.

mod firewall;
mod hostpanel;
#[cfg(windows)]
mod winhost;
mod yuv;
mod hosts;
mod input;
mod keys;
mod ondemand;
#[cfg(target_os = "linux")]
mod linuxhost;
mod relaypanel;
mod secrets;
mod session;
mod theme;
mod welcome;
#[cfg(windows)]
mod tray;

use egui::{Align, Align2, Color32, Key, Layout, RichText, Sense, TextureHandle, TextureOptions, ViewportCommand};
use hosts::SavedHost;
use rdcore::client::Transport;
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

/// `minimized`: oyna yig'ilgan holda ochiladi (kirishda avtostart).
pub fn run(minimized: bool) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Ulan")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(std::sync::Arc::new(egui::IconData { rgba: crate::icon_draw::icon_rgba(256), width: 256, height: 256 })),
        ..Default::default()
    };
    eframe::run_native("Ulan", options, Box::new(move |cc| Ok(Box::new(App::new(cc, minimized)))))
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
/// P2P bo'limida (matn maydoni fokusda bo'lmaganda) shu so'z yozilsa, yashirin HOST menyusi bir martaga ko'rinadi.
const HOST_SECRET: &str = "host123uzb";

const HEIGHTS: [(u16, &str); 5] = [(720, "720p"), (1080, "1080p"), (1440, "1440p"), (2160, "2160p"), (0, "Asl o'lcham")];

/// "SIFAT" kartasi: shablonlar va qo'shimcha sozlamalar (P2P va "Server orqali" uchun umumiy).
fn quality_card(ui: &mut egui::Ui, salt: &str, fps: &mut u16, kbps: &mut u32, height: &mut u16) {
    theme::card(ui, Some("SIFAT"), |ui| quality_picker(ui, salt, fps, kbps, height));
}

fn quality_picker(ui: &mut egui::Ui, salt: &str, fps: &mut u16, kbps: &mut u32, height: &mut u16) {
    let current = PRESETS.iter().find(|p| p.1 == *fps && p.2 == *kbps && p.3 == *height).map(|p| p.0);
    ui.horizontal_wrapped(|ui| {
        for p in PRESETS {
            if theme::pill(ui, current == Some(p.0), p.0).clicked() {
                *fps = p.1;
                *kbps = p.2;
                *height = p.3;
            }
        }
    });
    egui::CollapsingHeader::new("Qo'shimcha sozlamalar").id_salt((salt, "adv")).default_open(false).show(ui, |ui| {
        ui.add_space(4.0);
        egui::Grid::new((salt, "quality")).num_columns(2).spacing([14.0, 10.0]).min_col_width(theme::LABEL_W - 22.0).show(ui, |ui| {
            ui.label(theme::dim("FPS"));
            ui.add(egui::Slider::new(fps, 5..=60));
            ui.end_row();
            ui.label(theme::dim("Bitrate"));
            ui.add(egui::Slider::new(kbps, 500..=50_000).logarithmic(true).suffix(" kbit/s"));
            ui.end_row();
            ui.label(theme::dim("O'lcham"));
            let hname = HEIGHTS.iter().find(|h| h.0 == *height).map(|h| h.1).unwrap_or("Boshqa");
            egui::ComboBox::from_id_salt((salt, "height")).width(160.0).selected_text(hname).show_ui(ui, |ui| {
                for (v, n) in HEIGHTS {
                    ui.selectable_value(height, v, n);
                }
            });
            ui.end_row();
        });
    });
}

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
    /// Kalitni shu ulanish uchun xavfsiz saqlash (`secrets`).
    save_key: bool,
}

/// `RD_DIAG=1` bo'lsa viewer diagnostika chipi va konsol loglarini ko'rsatadi.
fn diag_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RD_DIAG").is_some())
}

struct Stats {
    last: Instant,
    frames: u64,
    bytes: u64,
    fps: f64,
    kbps: f64,
    /// Diagnostika (`RD_DIAG`): dekodlangan kadrlar va ularning tezligi, oxirgi kadr kelgan vaqt.
    decoded: u64,
    dec_fps: f64,
    last_frame: Instant,
}

/// Fayl uzatish qatori (ulanish ichidagi "Fayllar" panelida).
struct Transfer {
    id: u64,
    name: String,
    upload: bool,
    done: u64,
    total: u64,
    /// `None` = davom etmoqda.
    result: Option<Result<String, String>>,
}

#[derive(Default)]
struct Files {
    open: bool,
    /// Host'dagi almashinuv papkasi (`None` = hali olinmagan).
    remote: Option<Result<Vec<session::RemoteFile>, String>>,
    transfers: Vec<Transfer>,
}

fn fmt_size(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

struct App {
    /// Ochilishdagi dialog (yopilgach `None`).
    welcome: Option<welcome::Welcome>,
    tab: Tab,
    /// Yashirin HOST menyusi hozir ko'rinadimi (boshqa bo'limga o'tilganda yana yashirinadi).
    host_tab_shown: bool,
    /// P2P bo'limida yozilgan oxirgi belgilar (`HOST_SECRET` ni aniqlash uchun).
    typed: String,
    files: Files,
    /// Avtostartda birinchi kadrda oyna yig'iladi.
    start_minimized: bool,
    host: hostpanel::HostPanel,
    relay: relaypanel::RelayPanel,
    /// Server orqali ulanayotgan ID (muvaffaqiyatda kontaktlarga saqlanadi).
    relay_target: Option<String>,
    screen: Screen,
    hosts: Vec<SavedHost>,
    selected: Option<usize>,
    form: Form,
    error: Option<String>,
    /// Host ekranni ola olmadi: ulanish ochiq, ruxsatni qaytadan berish tugmasi ko'rsatiladi.
    refused: Option<String>,
    /// Aloqa uzildi va avtomatik qayta ulanilmoqda: (urinish, sabab).
    reconnecting: Option<(u32, String)>,
    handle: Option<session::Handle>,
    /// Host bilan autentifikatsiya o'tdi (video hali kelmagan bo'lishi mumkin).
    authed: bool,
    /// Boshlangan P2P ulanish kaliti: (yozuv, kalit, saqlansinmi). Kalit to'g'ri chiqqach saqlanadi yoki o'chiriladi.
    pending_key: Option<(String, String, bool)>,
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
    fn new(cc: &eframe::CreationContext<'_>, minimized: bool) -> Self {
        theme::apply(&cc.egui_ctx);
        let hosts = hosts::load();
        let mut app = App {
            welcome: Some(welcome::Welcome::new(&cc.egui_ctx)),
            tab: Tab::Relay,
            host_tab_shown: false,
            typed: String::new(),
            files: Files::default(),
            start_minimized: minimized,
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
                save_key: false,
            },
            pending_key: None,
            error: None,
            refused: None,
            reconnecting: None,
            handle: None,
            authed: false,
            title: String::new(),
            input_on: true,
            tex: None,
            fwd: input::Forwarder::default(),
            stats: Stats {
                last: Instant::now(),
                frames: 0,
                bytes: 0,
                fps: 0.0,
                kbps: 0.0,
                decoded: 0,
                dec_fps: 0.0,
                last_frame: Instant::now(),
            },
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
            // Shu ulanish uchun saqlangan kalit bo'lsa maydonga qo'yiladi
            let saved = secrets::get(&secrets::p2p_key(&self.full_addr()));
            self.form.save_key = saved.is_some();
            self.form.password = saved.unwrap_or_default();
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
        self.authed = false;
        if self.form.port.trim().parse::<u16>().is_err() && !f.addr.contains(':') {
            self.error = Some("Port 1 dan 65535 gacha son bo'lishi kerak".into());
            return;
        }
        self.title = if f.name.trim().is_empty() { full.clone() } else { format!("{} ({})", f.name.trim(), full) };
        self.input_on = f.input;
        self.tex = None;
        self.fwd = input::Forwarder::default();
        self.relay_target = None;
        self.pending_key = Some((secrets::p2p_key(&full), f.password.clone(), f.save_key));
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
            self.relay.set_error(Some("Server bilan aloqa yo'q".into()));
            return;
        };
        self.relay.set_error(None);
        let f = &self.form;
        self.error = None;
        self.authed = false;
        self.title = format!("ID {}", rdcore::relay::pretty_id(&req.id));
        self.input_on = f.input;
        self.tex = None;
        self.fwd = input::Forwarder::default();
        self.relay_target = Some(req.id.clone());
        self.pending_key = None;
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
        self.refused = None;
        self.reconnecting = None;
        self.tex = None;
        self.files = Files::default();
        self.screen = Screen::Form;
        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
    }

    fn poll_session(&mut self, ctx: &egui::Context) {
        let mut ended: Option<Option<String>> = None;
        let mut connected = false;
        let mut refresh_files = false;
        if let Some(h) = &self.handle {
            while let Ok(ev) = h.events.try_recv() {
                match ev {
                    session::Event::Connected { via_tcp } => {
                        let reconnect = self.reconnecting.is_some();
                        if via_tcp && !self.title.ends_with(" [TCP]") {
                            self.title.push_str(" [TCP]");
                        }
                        self.authed = true;
                        // qayta ulanishda Host ro'yxati qaytadan saqlanmaydi
                        connected = !reconnect;
                    }
                    session::Event::VideoStarted => {
                        self.refused = None;
                        self.reconnecting = None;
                        self.screen = Screen::Viewing;
                    }
                    session::Event::Reconnecting { attempt, reason } => self.reconnecting = Some((attempt, reason)),
                    session::Event::VideoRefused(msg) => self.refused = Some(msg),
                    session::Event::Files(r) => self.files.remote = Some(r),
                    session::Event::Transfer { id, done, total, result } => {
                        let succeeded = matches!(result, Some(Ok(_)));
                        if let Some(t) = self.files.transfers.iter_mut().find(|t| t.id == id) {
                            if total > 0 {
                                t.total = total;
                            }
                            match &result {
                                None => t.done = done,
                                Some(Ok(_)) => t.done = t.total,
                                Some(Err(_)) => {}
                            }
                            // yuborilgan fayl Host ro'yxatida ko'rinishi uchun
                            refresh_files |= succeeded && t.upload;
                            t.result = result;
                        }
                    }
                    session::Event::Ended(msg) => ended = Some(msg),
                }
            }
        }
        if refresh_files {
            if let Some(h) = &self.handle {
                h.list_files();
            }
        }
        if connected {
            // parol to'g'ri, Host ishonchli: ro'yxatga saqlanadi (yoki sozlamalari yangilanadi)
            match self.relay_target.clone() {
                Some(id) => self.relay.remember_contact(&id),
                None => {
                    self.remember_host();
                    // Kalit to'g'ri: belgilangan bo'lsa faqat shu ulanish uchun saqlanadi, aks holda eski yozuv o'chiriladi
                    if let Some((conn, pw, save)) = self.pending_key.take() {
                        if save && self.form.save {
                            if let Err(e) = secrets::set(&conn, &pw) {
                                eprintln!("{e}");
                            }
                        } else {
                            secrets::delete(&conn);
                        }
                    }
                }
            }
        }
        if let Some(msg) = ended {
            let was_viewing = self.screen == Screen::Viewing;
            self.disconnect(ctx);
            let err = match msg {
                Some(m) => Some(m),
                None if was_viewing => Some("Ulanish yopildi".into()),
                None => None,
            };
            // ID orqali ulanishda xato "Server orqali" bo'limida ko'rinadi (P2P formasida emas)
            if self.relay_target.is_some() {
                self.relay.set_error(err);
            } else {
                self.error = err;
            }
        }
    }

    /// P2P bo'limida `HOST_SECRET` yozilganini kuzatadi. Matn maydoniga yozilgani hisobga olinmaydi.
    fn watch_secret(&mut self, ctx: &egui::Context) {
        if self.tab != Tab::Connect || self.host_tab_shown || ctx.wants_keyboard_input() {
            return;
        }
        let typed: String = ctx.input(|i| {
            i.events.iter().filter_map(|e| if let egui::Event::Text(t) = e { Some(t.to_lowercase()) } else { None }).collect()
        });
        if typed.is_empty() {
            return;
        }
        self.typed.push_str(&typed);
        let n = self.typed.chars().count();
        if n > HOST_SECRET.len() {
            self.typed = self.typed.chars().skip(n - HOST_SECRET.len()).collect();
        }
        if self.typed == HOST_SECRET {
            self.host_tab_shown = true;
            self.typed.clear();
        }
    }

    fn ui_form(&mut self, ctx: &egui::Context) {
        self.watch_secret(ctx);
        let (host_on, host_label) = self.host.header_status();
        let show_host = self.host_tab_shown || self.tab == Tab::Host;
        egui::TopBottomPanel::top("header").frame(theme::header_frame()).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.add(egui::Image::new(&theme::logo(ctx)).fit_to_exact_size(egui::vec2(26.0, 26.0)));
                ui.label(RichText::new("Ulan").size(17.0).strong());
                ui.add_space(14.0);
                for (t, name) in [(Tab::Relay, "Server orqali"), (Tab::Connect, "P2P"), (Tab::ThisPc, "P2P Sozlash"), (Tab::Host, "HOST")] {
                    if t == Tab::Host && !show_host {
                        continue;
                    }
                    if theme::tab(ui, self.tab == t, name).clicked() && self.tab != t {
                        self.tab = t;
                        self.typed.clear();
                        // HOST bir martaga ko'rinadi: boshqa bo'limga o'tilganda yana yashirinadi
                        if t != Tab::Host {
                            self.host_tab_shown = false;
                        }
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    theme::chip(ui, host_label, if host_on { theme::GOOD } else { theme::DIM });
                    if show_host && self.relay.server_running() {
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
            // Chapda "Oldin ulanganlar" (P2P'dagi saqlangan Host'lar kabi), o'rtada ulanish formasi
            let mut req = egui::SidePanel::left("relay_contacts")
                .exact_width(250.0)
                .resizable(false)
                .frame(theme::side_frame())
                .show(ctx, |ui| self.relay.ui_side(ui))
                .inner;
            let form = &mut self.form;
            egui::CentralPanel::default().frame(theme::page_frame()).show(ctx, |ui| {
                if let Some(r) = self.relay.ui(ui, &mut |ui| quality_card(ui, "relay", &mut form.fps, &mut form.kbps, &mut form.height)) {
                    req = Some(r);
                }
            });
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
                    let stroke = if selected { egui::Stroke::new(1.0_f32, theme::ACCENT) } else { egui::Stroke::NONE };
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
                let title = match self.selected.and_then(|i| self.hosts.get(i)) {
                    Some(h) => h.name.clone(),
                    None => "Yangi ulanish".to_string(),
                };
                theme::page_title(ui, &title, &["Host manzili va kalitini kiriting. Kalit faqat \"Saqlash\" belgilansa saqlanadi (shifrlangan holda)."]);

                let mut pw_enter = false;
                theme::card(ui, Some("HOST"), |ui| {
                    theme::form(ui, "form", |ui| {
                        ui.label(theme::dim("Nom"));
                        ui.add(theme::input(&mut self.form.name).hint_text("masalan: Ofis Ubuntu"));
                        ui.end_row();
                        ui.label(theme::dim("Manzil"));
                        ui.horizontal(|ui| {
                            ui.add(theme::input(&mut self.form.addr).hint_text("IP yoki nom").desired_width(theme::FIELD_W - 134.0));
                            ui.label(theme::dim("Port"));
                            ui.add(theme::input(&mut self.form.port).desired_width(70.0));
                        });
                        ui.end_row();
                        ui.label(theme::dim("Kalit"));
                        ui.horizontal(|ui| {
                            let r = ui.add(theme::input(&mut self.form.password).password(!self.form.show_password));
                            pw_enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                            let eye = if self.form.show_password { "Yashirish" } else { "Ko'rsatish" };
                            if theme::button(ui, eye).clicked() {
                                self.form.show_password = !self.form.show_password;
                            }
                            ui.add_enabled_ui(secrets::available() && self.form.save, |ui| {
                                ui.checkbox(&mut self.form.save_key, "Saqlash")
                                    .on_hover_text("Kalit faqat shu ulanish uchun, shifrlangan holda saqlanadi")
                                    .on_disabled_hover_text("\"Ro'yxatga saqlash\" yoqilgan va tizim kalit ombori mavjud bo'lishi kerak");
                            });
                        });
                        ui.end_row();
                    });
                });

                theme::card(ui, Some("ULANISH"), |ui| {
                    let tname = |t: Transport| match t {
                        Transport::Auto => "Avto",
                        Transport::Udp => "UDP",
                        Transport::Tcp => "TCP",
                    };
                    theme::form(ui, "transport", |ui| {
                        ui.label(theme::dim("Transport"));
                        ui.horizontal(|ui| {
                            for t in [Transport::Auto, Transport::Udp, Transport::Tcp] {
                                if theme::pill(ui, self.form.transport == t, tname(t)).clicked() {
                                    self.form.transport = t;
                                }
                            }
                        });
                        ui.end_row();
                    });
                    theme::hint(
                        ui,
                        match self.form.transport {
                            Transport::Auto => "Avval UDP (tez), bo'lmasa TCP orqali ulanadi.",
                            Transport::Udp => "Eng tez; UDP bloklangan tarmoqlarda ishlamaydi.",
                            Transport::Tcp => "UDP bloklangan tarmoqlar uchun; biroz sekinroq.",
                        },
                    );
                    ui.add_space(6.0);
                    theme::toggle(ui, &mut self.form.input, "Sichqoncha va klaviatura bilan boshqarish");
                    theme::toggle(ui, &mut self.form.save, "Ro'yxatga saqlash");
                });
                quality_card(ui, "p2p", &mut self.form.fps, &mut self.form.kbps, &mut self.form.height);

                let can = !self.form.addr.trim().is_empty() && !self.form.password.is_empty();
                if let Some(e) = &self.error {
                    theme::error_banner(ui, e);
                    ui.add_space(6.0);
                    if can && theme::button(ui, "Qayta ulanish").clicked() {
                        do_connect = true;
                    }
                    ui.add_space(10.0);
                }
                if theme::primary_button(ui, "Ulanish", can).clicked() || (pw_enter && can) {
                    do_connect = true;
                }
            });
        });

        if let Some(i) = delete {
            if i < self.hosts.len() {
                let removed = self.hosts.remove(i);
                secrets::delete(&secrets::p2p_key(&removed.addr));
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
            self.form.save_key = false;
            self.form.port = protocol::DEFAULT_PORT.to_string();
            self.error = None;
        }
        if do_connect {
            self.connect(ctx);
        }
    }

    fn ui_connecting(&mut self, ctx: &egui::Context) {
        let mut cancel = false;
        let mut reset = false;
        egui::CentralPanel::default().frame(theme::page_frame()).show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() / 3.0).max(20.0));
                if self.refused.is_none() {
                    ui.add(egui::Spinner::new().size(34.0).color(theme::ACCENT));
                    ui.add_space(12.0);
                }
                let what = if self.refused.is_some() {
                    "Ekran olinmadi"
                } else if !self.authed {
                    "Ulanilmoqda"
                } else {
                    "Video kutilmoqda"
                };
                ui.label(RichText::new(what).size(17.0).strong());
                ui.label(theme::dim(&self.title));
                if self.relay_target.is_some() && !self.authed {
                    theme::hint(ui, "Qabul qiluvchi ruxsat berishi kutilmoqda (30 soniyagacha)");
                }
                if let Some(msg) = &self.refused {
                    ui.add_space(10.0);
                    theme::error_banner(ui, msg);
                    ui.add_space(6.0);
                    theme::hint(ui, "Host kompyuterida ruxsat dialogi chiqadi: ekranni tanlab, ulashishga ruxsat bering");
                    ui.add_space(10.0);
                    reset = ui.add(egui::Button::new("Ruxsatni qaytadan berish").min_size(egui::vec2(220.0, 32.0))).clicked();
                }
                ui.add_space(14.0);
                cancel = ui.add(egui::Button::new("Bekor qilish").min_size(egui::vec2(140.0, 32.0))).clicked();
            });
        });
        if reset {
            self.refused = None;
            if let Some(h) = &self.handle {
                h.reset_permissions();
            }
        }
        if cancel {
            self.disconnect(ctx);
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
            let decoded = shared.decoded.load(Ordering::Relaxed);
            self.stats.dec_fps = (decoded - self.stats.decoded.min(decoded)) as f64 / secs;
            self.stats.decoded = decoded;
            if frames != self.stats.frames {
                self.stats.last_frame = Instant::now();
            }
            self.stats.frames = frames;
            self.stats.bytes = bytes;
            self.stats.last = Instant::now();
            if diag_enabled() {
                eprintln!(
                    "[diag] recv {:.0} fps, dec {:.0} fps, {:.2} Mbit/s, queue {}, need_key {}, rtt {:.0} ms, oxirgi kadr {:.1}s oldin",
                    self.stats.fps,
                    self.stats.dec_fps,
                    self.stats.kbps / 1000.0,
                    shared.queue.load(Ordering::Relaxed),
                    shared.need_key.load(Ordering::Relaxed),
                    shared.rtt_us.load(Ordering::Relaxed) as f64 / 1000.0,
                    self.stats.last_frame.elapsed().as_secs_f64(),
                );
            }
        }
        let rtt = shared.rtt_us.load(Ordering::Relaxed) as f64 / 1000.0;

        let fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
        let hover = ctx.input(|i| i.pointer.hover_pos());
        let show_bar = !fullscreen || hover.map_or(false, |p| p.y < 40.0);
        let mut want_disconnect = false;
        let mut want_reset = false;
        let mut want_files = false;
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
                    if diag_enabled() {
                        theme::chip(
                            ui,
                            format!(
                                "dec {:.0} · q {} · key {} · {:.0}s",
                                self.stats.dec_fps,
                                shared.queue.load(Ordering::Relaxed),
                                if shared.need_key.load(Ordering::Relaxed) { "kutilmoqda" } else { "ok" },
                                self.stats.last_frame.elapsed().as_secs_f64(),
                            ),
                            theme::DIM,
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::danger_button(ui, "Uzish").clicked() {
                            want_disconnect = true;
                        }
                        let fs_text = if fullscreen { "Oynaga qaytish" } else { "To'liq ekran" };
                        if theme::button(ui, fs_text).clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen));
                        }
                        if theme::pill(ui, self.input_on, "Boshqaruv").clicked() {
                            self.input_on = !self.input_on;
                        }
                        if theme::pill(ui, self.files.open, "Fayllar")
                            .on_hover_text("Host bilan fayl almashish (fayllarni shu oynaga sudrab tashlash ham mumkin)")
                            .clicked()
                        {
                            want_files = true;
                        }
                        if theme::button(ui, "Ruxsatni qaytadan berish")
                            .on_hover_text("Host'dagi ekran va boshqarish ruxsatini bekor qilib, qaytadan so'raydi (Linux Host). Ulanish uziladi.")
                            .clicked()
                        {
                            want_reset = true;
                        }
                    });
                });
            });
        }

        self.ui_files(ctx, want_files);

        // Yangi kadr bo'lsa tekstura yangilanadi
        if let Some(pic) = shared.picture.lock().unwrap().take() {
            let img = pic.image;
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

        // Aloqa uzildi: oxirgi kadr ustida qayta ulanish holati
        if let Some((attempt, reason)) = &self.reconnecting {
            let screen = ctx.screen_rect();
            ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("reconnect_dim")))
                .rect_filled(screen, 0.0, Color32::from_black_alpha(160));
            egui::Area::new(egui::Id::new("reconnect_box")).order(egui::Order::Foreground).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                theme::card(ui, None, |ui| {
                    ui.set_max_width(380.0);
                    ui.vertical_centered(|ui| {
                        ui.add(egui::Spinner::new().size(28.0).color(theme::ACCENT));
                        ui.add_space(8.0);
                        ui.label(RichText::new(format!("Aloqa uzildi. Qayta ulanilmoqda… ({attempt})")).size(16.0).strong());
                        ui.add_space(4.0);
                        ui.label(theme::dim(reason));
                        ui.add_space(10.0);
                        if ui.add(egui::Button::new("Bekor qilish").min_size(egui::vec2(140.0, 32.0))).clicked() {
                            want_disconnect = true;
                        }
                    });
                });
            });
        }

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

        if want_reset {
            if let Some(h) = &self.handle {
                h.reset_permissions();
            }
        }
        if want_disconnect || want_reset {
            self.disconnect(ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}

impl App {
    /// Fayl almashinuvi: oynaga tashlangan fayllarni Host'ga yuboradi va o'ng panelda Host papkasi bilan uzatishlarni ko'rsatadi.
    fn ui_files(&mut self, ctx: &egui::Context, toggle: bool) {
        let Some(h) = &self.handle else { return };
        if toggle {
            self.files.open = !self.files.open;
            if self.files.open && self.files.remote.is_none() {
                h.list_files();
            }
        }
        let (dropped, hovering) = ctx.input(|i| {
            (i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect::<Vec<_>>(), !i.raw.hovered_files.is_empty())
        });
        for path in dropped {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let total = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let id = h.send_file(path);
            self.files.transfers.push(Transfer { id, name, upload: true, done: 0, total, result: None });
            if !self.files.open {
                self.files.open = true;
                h.list_files();
            }
        }
        if hovering {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("file_drop")));
            let screen = ctx.screen_rect();
            painter.rect_filled(screen, 0.0, Color32::from_black_alpha(150));
            painter.text(screen.center(), Align2::CENTER_CENTER, "Faylni Host'ga yuborish uchun qo'yib yuboring", egui::FontId::proportional(20.0), Color32::WHITE);
        }
        if !self.files.open {
            return;
        }

        let (mut refresh, mut clear, mut get) = (false, false, None::<String>);
        egui::SidePanel::right("files").exact_width(300.0).resizable(false).frame(theme::side_frame()).show(ctx, |ui| {
            ui.label(RichText::new("FAYLLAR").size(11.0).color(theme::DIM).strong());
            ui.add_space(2.0);
            theme::hint(ui, "Fayl yuborish: uni shu oynaga sudrab tashlang. U Host'ning \"downloads/Ulan\" papkasiga tushadi.");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                refresh = theme::button(ui, "Yangilash").clicked();
                clear = theme::button(ui, "Tarixni tozalash").clicked();
            });
            ui.add_space(6.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(theme::dim("Host papkasidagi fayllar"));
                ui.add_space(2.0);
                match &self.files.remote {
                    None => {
                        ui.label(theme::dim("Yuklanmoqda..."));
                    }
                    Some(Err(e)) => theme::note(ui, e, theme::BAD),
                    Some(Ok(list)) if list.is_empty() => theme::hint(ui, "Papka bo'sh."),
                    Some(Ok(list)) => {
                        for f in list {
                            ui.label(RichText::new(&f.name).size(13.0));
                            ui.horizontal(|ui| {
                                ui.label(theme::dim(&fmt_size(f.size)));
                                if theme::button(ui, "Yuklab olish").clicked() {
                                    get = Some(f.name.clone());
                                }
                            });
                            ui.add_space(4.0);
                        }
                    }
                }
                if !self.files.transfers.is_empty() {
                    ui.add_space(8.0);
                    ui.label(theme::dim("Uzatishlar"));
                    ui.add_space(2.0);
                }
                for t in self.files.transfers.iter().rev() {
                    ui.label(RichText::new(format!("{} {}", if t.upload { "↑" } else { "↓" }, t.name)).size(13.0));
                    match &t.result {
                        None => {
                            let frac = if t.total > 0 { (t.done as f32 / t.total as f32).clamp(0.0, 1.0) } else { 0.0 };
                            ui.add(egui::ProgressBar::new(frac).desired_height(8.0));
                            ui.label(theme::dim(&format!("{} / {}", fmt_size(t.done), fmt_size(t.total))));
                        }
                        Some(Ok(msg)) => theme::note(ui, msg, theme::GOOD),
                        Some(Err(e)) => theme::note(ui, e, theme::BAD),
                    }
                    ui.add_space(6.0);
                }
            });
        });

        if clear {
            self.files.transfers.retain(|t| t.result.is_none());
        }
        if refresh {
            self.files.remote = None;
            h.list_files();
        }
        if let Some(name) = get {
            let total = match &self.files.remote {
                Some(Ok(list)) => list.iter().find(|f| f.name == name).map_or(0, |f| f.size),
                _ => 0,
            };
            let id = h.get_file(name.clone());
            self.files.transfers.push(Transfer { id, name, upload: false, done: 0, total, result: None });
        }
    }

    /// Tray: oyna yopilsa/pastga tushirilsa (agent yoqilgan bo'lsa) yashirinadi; ikonka orqali qaytadi.
    #[cfg(windows)]
    fn tray_update(&mut self, ctx: &egui::Context) {
        let Some(tray) = self.tray.as_mut() else { return };
        tray.set_status(&self.relay.status_line());
        let mut show = false;
        for c in tray.poll() {
            match c {
                tray::Cmd::Show => show = true,
                tray::Cmd::Shown => self.hidden = false,
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

/// Diagnostika: `ULAN_FPS_DEBUG=1` bo'lsa soniyasiga `update()` chaqiruvlari sonini stderr ga chiqaradi.
fn fps_debug(hidden: bool, welcome: bool) {
    use std::sync::{atomic::{AtomicU32, Ordering::Relaxed}, Mutex, OnceLock};
    static ON: OnceLock<bool> = OnceLock::new();
    static COUNT: AtomicU32 = AtomicU32::new(0);
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    if !*ON.get_or_init(|| std::env::var_os("ULAN_FPS_DEBUG").is_some()) {
        return;
    }
    COUNT.fetch_add(1, Relaxed);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let t = last.get_or_insert(now);
    if now.duration_since(*t) >= Duration::from_secs(1) {
        eprintln!("[fps] update/s={} hidden={hidden} welcome={welcome}", COUNT.swap(0, Relaxed));
        *last = Some(now);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        fps_debug(self.hidden, self.welcome.is_some());
        if std::mem::take(&mut self.start_minimized) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        self.tray_update(ctx);
        self.poll_session(ctx);
        self.relay.poll(ctx);
        match self.screen {
            Screen::Form => self.ui_form(ctx),
            Screen::Connecting => self.ui_connecting(ctx),
            Screen::Viewing => self.ui_viewing(ctx),
        }
        if self.welcome.as_mut().is_some_and(|w| !w.show(ctx)) {
            self.welcome = None;
        }
        self.relay.incoming_dialog(ctx);
    }
}
