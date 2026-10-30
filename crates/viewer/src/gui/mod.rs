//! Viewer'ning oynali interfeysi (egui): ulanish oynasi, saqlangan Host'lar, sifat sozlamalari,
//! fingerprint tasdig'i va ulanish ichidagi panel.

mod hostpanel;
mod hosts;
mod input;
mod keys;
mod session;

use egui::{Align, Align2, Color32, Key, Layout, RichText, TextureHandle, TextureOptions, ViewportCommand};
use hosts::SavedHost;
use std::{
    sync::{atomic::Ordering, mpsc},
    time::{Duration, Instant},
};

pub fn run() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Remote Desktop")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native("Remote Desktop", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
        .map_err(|e| anyhow::anyhow!("oyna ochilmadi: {e}"))
}

/// Sifat shablonlari: (nom, fps, kbps, balandlik; 0 = asl o'lcham)
const PRESETS: [(&str, u16, u32, u16); 4] = [
    ("Past (720p, 15 FPS)", 15, 2500, 720),
    ("O'rta (1080p, 30 FPS)", 30, 8000, 1080),
    ("Yuqori (1080p, 30 FPS)", 30, 15000, 1080),
    ("Maksimal (asl o'lcham)", 30, 25000, 0),
];
const HEIGHTS: [(u16, &str); 5] = [(720, "720p"), (1080, "1080p"), (1440, "1440p"), (2160, "2160p"), (0, "Asl o'lcham")];

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Connect,
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
    password: String,
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
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let hosts = hosts::load();
        let mut app = App {
            tab: Tab::Connect,
            host: hostpanel::HostPanel::new(),
            screen: Screen::Form,
            hosts,
            selected: None,
            form: Form {
                name: String::new(),
                addr: String::new(),
                password: String::new(),
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
            self.form.addr = h.addr;
            self.form.fps = h.fps;
            self.form.kbps = h.kbps;
            self.form.height = h.height;
            self.form.input = h.input;
            self.form.password.clear();
        }
    }

    fn connect(&mut self, ctx: &egui::Context) {
        let f = &self.form;
        if f.addr.trim().is_empty() {
            self.error = Some("Manzilni kiriting (IP yoki IP:port)".into());
            return;
        }
        self.error = None;
        self.fingerprint.clear();
        self.title = if f.name.trim().is_empty() { f.addr.trim().to_string() } else { format!("{} ({})", f.name.trim(), f.addr.trim()) };
        self.input_on = f.input;
        self.tex = None;
        self.fwd = input::Forwarder::default();
        self.handle = Some(session::start(
            ctx.clone(),
            session::Params {
                addr: f.addr.trim().to_string(),
                password: f.password.clone(),
                fps: f.fps,
                kbps: f.kbps,
                height: f.height,
            },
        ));
        self.screen = Screen::Connecting;
    }

    fn remember_host(&mut self) {
        if !self.form.save {
            return;
        }
        let f = &self.form;
        let entry = SavedHost {
            name: if f.name.trim().is_empty() { f.addr.trim().to_string() } else { f.name.trim().to_string() },
            addr: f.addr.trim().to_string(),
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
                    session::Event::Connected { fingerprint } => {
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
            self.remember_host();
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
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Connect, "Ulanish");
                ui.selectable_value(&mut self.tab, Tab::ThisPc, "Bu kompyuter");
            });
        });
        if self.tab == Tab::ThisPc {
            egui::CentralPanel::default().show(ctx, |ui| self.host.ui(ui));
            return;
        }
        let mut do_connect = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(8.0);
            ui.heading("Remote Desktop");
            ui.add_space(8.0);
            ui.columns(2, |cols| {
                // --- Saqlangan Host'lar
                let ui = &mut cols[0];
                ui.label(RichText::new("Saqlangan Host'lar").strong());
                ui.add_space(4.0);
                let mut select = None;
                let mut connect_idx = None;
                egui::ScrollArea::vertical().max_height(ui.available_height() - 40.0).show(ui, |ui| {
                    if self.hosts.is_empty() {
                        ui.weak("Hozircha yo'q. Muvaffaqiyatli ulanishdan keyin shu yerda paydo bo'ladi.");
                    }
                    for (i, h) in self.hosts.iter().enumerate() {
                        let text = format!("{}\n{}", h.name, h.addr);
                        let r = ui.selectable_label(self.selected == Some(i), text);
                        if r.clicked() {
                            select = Some(i);
                        }
                        if r.double_clicked() {
                            connect_idx = Some(i);
                        }
                    }
                });
                if let Some(i) = select.or(connect_idx) {
                    self.select(i);
                }
                if connect_idx.is_some() && !self.form.password.is_empty() {
                    do_connect = true;
                }
                if let Some(i) = self.selected {
                    if ui.button("Ro'yxatdan o'chirish").clicked() {
                        self.hosts.remove(i);
                        self.selected = None;
                        let _ = hosts::save(&self.hosts);
                    }
                }

                // --- Ulanish formasi
                let ui = &mut cols[1];
                ui.label(RichText::new("Ulanish").strong());
                ui.add_space(4.0);
                let mut pw_enter = false;
                egui::Grid::new("form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Nom");
                    ui.add(egui::TextEdit::singleline(&mut self.form.name).hint_text("masalan: Ofis Ubuntu"));
                    ui.end_row();
                    ui.label("Manzil");
                    ui.add(egui::TextEdit::singleline(&mut self.form.addr).hint_text("IP yoki IP:port"));
                    ui.end_row();
                    ui.label("Parol");
                    ui.horizontal(|ui| {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.form.password).password(!self.form.show_password),
                        );
                        pw_enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        ui.checkbox(&mut self.form.show_password, "Ko'rsatish");
                    });
                    ui.end_row();

                    ui.label("Sifat");
                    let current = PRESETS
                        .iter()
                        .find(|p| p.1 == self.form.fps && p.2 == self.form.kbps && p.3 == self.form.height)
                        .map(|p| p.0)
                        .unwrap_or("Boshqa");
                    egui::ComboBox::from_id_salt("preset").selected_text(current).show_ui(ui, |ui| {
                        for p in PRESETS {
                            if ui.selectable_label(current == p.0, p.0).clicked() {
                                self.form.fps = p.1;
                                self.form.kbps = p.2;
                                self.form.height = p.3;
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("FPS");
                    ui.add(egui::Slider::new(&mut self.form.fps, 5..=60));
                    ui.end_row();
                    ui.label("Bitrate");
                    ui.add(egui::Slider::new(&mut self.form.kbps, 500..=50_000).logarithmic(true).suffix(" kbit/s"));
                    ui.end_row();
                    ui.label("O'lcham");
                    let hname = HEIGHTS.iter().find(|h| h.0 == self.form.height).map(|h| h.1).unwrap_or("Boshqa");
                    egui::ComboBox::from_id_salt("height").selected_text(hname).show_ui(ui, |ui| {
                        for (v, n) in HEIGHTS {
                            ui.selectable_value(&mut self.form.height, v, n);
                        }
                    });
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.checkbox(&mut self.form.input, "Sichqoncha va klaviatura bilan boshqarish");
                ui.checkbox(&mut self.form.save, "Ro'yxatga saqlash (parol saqlanmaydi)");
                ui.add_space(10.0);
                let can = !self.form.addr.trim().is_empty() && !self.form.password.is_empty();
                if ui.add_enabled(can, egui::Button::new(RichText::new("  Ulanish  ").strong())).clicked() || (pw_enter && can) {
                    do_connect = true;
                }
                if let Some(e) = &self.error {
                    ui.add_space(10.0);
                    ui.colored_label(Color32::from_rgb(235, 110, 100), e);
                }
            });
        });
        if do_connect {
            self.connect(ctx);
        }
    }

    fn ui_connecting(&mut self, ctx: &egui::Context) {
        let mut cancel = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.add_space(ui.available_height() / 3.0);
                ui.spinner();
                ui.add_space(8.0);
                let what = if self.fingerprint.is_empty() { "Ulanilmoqda" } else { "Video kutilmoqda" };
                ui.label(format!("{what}: {}", self.title));
                ui.add_space(8.0);
                cancel = ui.button("Bekor qilish").clicked();
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
                ui.label("Bu Host'ga birinchi marta ulanyapsiz. Fingerprint'ni Host ekranidagi yoki");
                ui.label("o'rnatishda ko'rsatilgan qiymat bilan solishtiring:");
                ui.add_space(6.0);
                let half = fp.len() / 2;
                let split = fp[..half].rfind(':').map(|i| i + 1).unwrap_or(half);
                ui.monospace(&fp[..split]);
                ui.monospace(&fp[split..]);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Ha, ishonaman").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Yo'q").clicked() {
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
            egui::TopBottomPanel::top("bar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.title).strong());
                    ui.separator();
                    ui.label(format!("{:.0} FPS   {:.0} kbit/s   {:.0} ms", self.stats.fps, self.stats.kbps, rtt));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Uzish").clicked() {
                            want_disconnect = true;
                        }
                        let fs_text = if fullscreen { "Oynaga qaytish" } else { "To'liq ekran" };
                        if ui.button(fs_text).clicked() {
                            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(!fullscreen));
                        }
                        ui.checkbox(&mut self.input_on, "Boshqaruv");
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_session(ctx);
        match self.screen {
            Screen::Form => self.ui_form(ctx),
            Screen::Connecting => self.ui_connecting(ctx),
            Screen::Viewing => self.ui_viewing(ctx),
        }
        self.ui_confirm(ctx);
    }
}
