//! "Server orqali" bo'limi: ID bilan ulanish, kontaktlar (onlayn/oflayn), bu kompyuterning relay sozlamalari
//! (ID, qabul qilish rejimi, ishonchli ro'yxat), server rejimi (ulangan clientlar, firewall) va kiruvchi
//! so'rov dialogi. Hammasi `rdcore::relay` ustida.

use super::{ondemand, firewall, theme};
use egui::{Align2, Key, RichText, Ui};
use rdcore::{
    auth::Credentials,
    cert::HostIdentity,
    paths,
    relay::{
        self,
        agent::{Agent, AgentConfig, AgentEvent, Connector, Policy},
        registry::now_secs,
        server::{RelayServer, ServerHandle},
    },
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

/// Kiruvchi so'rovga javob berish muddati.
const ANSWER_SECS: u64 = 30;

// ---------------------------------------------------------------------------------------------
// Fayllar
// ---------------------------------------------------------------------------------------------

fn file(name: &str) -> Option<PathBuf> {
    paths::config_dir().ok().map(|d| d.join(name))
}

fn read_text(name: &str) -> String {
    file(name).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default()
}

fn write_text(name: &str, text: &str) {
    if let Some(p) = file(name) {
        if let Err(e) = rdcore::fsutil::write_private(&p, text.as_bytes()) {
            eprintln!("{name} yozilmadi: {e}");
        }
    }
}

fn read_kv(name: &str) -> HashMap<String, String> {
    read_text(name)
        .lines()
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect()
}

fn computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "Kompyuter".into())
}

fn random_key() -> String {
    const A: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    rdcore::auth::random_bytes::<14>().iter().map(|b| A[*b as usize % A.len()] as char).collect()
}

// ---------------------------------------------------------------------------------------------
// Client sozlamalari
// ---------------------------------------------------------------------------------------------

struct ClientCfg {
    server: String,
    key: String,
    name: String,
    id: String,
    token: String,
    enabled: bool,
    unattended: bool,
}

impl ClientCfg {
    fn load() -> Self {
        let kv = read_kv("relay_client.txt");
        let get = |k: &str| kv.get(k).cloned().unwrap_or_default();
        ClientCfg {
            server: get("server"),
            key: get("key"),
            name: if get("name").is_empty() { computer_name() } else { get("name") },
            id: get("id"),
            token: get("token"),
            enabled: get("enabled") == "1",
            unattended: get("unattended") == "1",
        }
    }

    fn save(&self) {
        write_text(
            "relay_client.txt",
            &format!(
                "server={}\nkey={}\nname={}\nid={}\ntoken={}\nenabled={}\nunattended={}\n",
                self.server, self.key, self.name, self.id, self.token, self.enabled as u8, self.unattended as u8
            ),
        );
    }

    /// `host` yoki `host:port` -> `host:port`
    fn server_addr(&self) -> String {
        let s = self.server.trim();
        if s.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) {
            s.to_string()
        } else {
            format!("{s}:{}", relay::DEFAULT_PORT)
        }
    }
}

struct Contact {
    id: String,
    name: String,
}

fn load_contacts() -> Vec<Contact> {
    read_text("relay_contacts.txt")
        .lines()
        .filter_map(|l| {
            let (id, name) = l.split_once(' ').unwrap_or((l, ""));
            relay::valid_id(id).then(|| Contact { id: id.to_string(), name: name.trim().to_string() })
        })
        .collect()
}

fn save_contacts(c: &[Contact]) {
    write_text("relay_contacts.txt", &c.iter().map(|c| format!("{} {}\n", c.id, c.name)).collect::<String>());
}

fn load_trusted() -> Vec<String> {
    read_text("relay_trusted.txt").lines().map(str::trim).filter(|l| relay::valid_id(l)).map(str::to_string).collect()
}

fn save_trusted(t: &[String]) {
    write_text("relay_trusted.txt", &t.iter().map(|i| format!("{i}\n")).collect::<String>());
}

// ---------------------------------------------------------------------------------------------
// Server (bu kompyuter server bo'lganda)
// ---------------------------------------------------------------------------------------------

struct ServerRunner {
    handle: ServerHandle,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for ServerRunner {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
    }
}

/// `key = None`: ochiq server (kalitsiz ro'yxatdan o'tish).
fn start_server(port: u16, key: Option<&str>) -> Result<ServerRunner, String> {
    let (ready_tx, ready_rx) = mpsc::channel::<Result<ServerHandle, String>>();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let key = key.map(str::to_string);
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
        };
        rt.block_on(async move {
            let setup = (|| -> anyhow::Result<RelayServer> {
                let dir = paths::config_dir()?;
                let identity = HostIdentity::load_or_create(&dir)?;
                let creds = key.as_deref().map(Credentials::from_password).transpose()?;
                RelayServer::bind(SocketAddr::from(([0, 0, 0, 0], port)), &identity, creds, Some(dir.join("relay_registry.txt")))
            })();
            match setup {
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("{e:#}")));
                }
                Ok(server) => {
                    let _ = ready_tx.send(Ok(server.handle()));
                    tokio::select! {
                        _ = server.run() => {}
                        _ = stop_rx => {}
                    }
                }
            }
        });
        rt.shutdown_timeout(Duration::from_secs(2));
    });
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(handle)) => Ok(ServerRunner { handle, stop: Some(stop_tx) }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("server vaqtida ishga tushmadi".into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------------------------

struct Incoming {
    sid: String,
    from: String,
    name: String,
    since: Instant,
}

/// Foydalanuvchi "Ulanish" ni bosdi.
pub struct ConnectRequest {
    pub id: String,
    pub password: String,
}

pub struct RelayPanel {
    cfg: ClientCfg,
    agent: Option<Agent>,
    online: bool,
    status: String,
    contacts: Vec<Contact>,
    presence: HashMap<String, (bool, String)>,
    trusted: Vec<String>,
    incoming: VecDeque<Incoming>,
    target: String,
    password: String,
    error: Option<String>,
    show_key: bool,
    /// Qarovsiz parol maydoni (saqlangach tozalanadi) va uning holati.
    unattended_pw: String,
    unattended_msg: Option<(bool, String)>,
    ondemand: ondemand::Guard,
    // server rejimi
    server_on: bool,
    server_port: String,
    server_key: String,
    /// Kalit bilan himoyalash (standart yoqilgan).
    server_keyed: bool,
    server: Option<ServerRunner>,
    server_error: Option<String>,
    fw_rx: Option<mpsc::Receiver<Result<String, String>>>,
    fw_msg: Option<(bool, String)>,
}

impl RelayPanel {
    pub fn new() -> Self {
        let cfg = ClientCfg::load();
        let srv = read_kv("relay_server.txt");
        let key = read_text("relay_server.pw").trim().to_string();
        let mut p = RelayPanel {
            cfg,
            agent: None,
            online: false,
            status: "o'chirilgan".into(),
            contacts: load_contacts(),
            presence: HashMap::new(),
            trusted: load_trusted(),
            incoming: VecDeque::new(),
            target: String::new(),
            password: String::new(),
            error: None,
            show_key: false,
            unattended_pw: String::new(),
            unattended_msg: None,
            ondemand: ondemand::Guard::default(),
            server_on: srv.get("enabled").is_some_and(|v| v == "1"),
            server_port: srv.get("port").cloned().unwrap_or_else(|| relay::DEFAULT_PORT.to_string()),
            server_key: if key.chars().count() >= 8 { key } else { random_key() },
            server_keyed: srv.get("keyed").map_or(true, |v| v != "0"),
            server: None,
            server_error: None,
            fw_rx: None,
            fw_msg: None,
        };
        if p.server_on {
            p.start_server();
        }
        if p.cfg.enabled {
            p.start_agent();
        }
        p
    }

    // --- tashqi API ---------------------------------------------------------------------------

    pub fn connector(&self) -> Option<Connector> {
        self.agent.as_ref().map(Agent::connector)
    }

    /// "Server orqali" bo'limida ko'rsatiladigan xato (masalan, ID orqali ulanish tugagani sababi).
    pub fn set_error(&mut self, e: Option<String>) {
        self.error = e;
    }

    pub fn is_online(&self) -> bool {
        self.online
    }

    /// Agent yoqilgan (tray'ga yashirinish shu holatda mantiqli).
    pub fn agent_active(&self) -> bool {
        self.agent.is_some()
    }

    pub fn has_incoming(&self) -> bool {
        !self.incoming.is_empty()
    }

    /// Tray uchun qisqa holat.
    pub fn status_line(&self) -> String {
        if self.online && !self.cfg.id.is_empty() {
            format!("Onlayn · ID {}", relay::pretty_id(&self.cfg.id))
        } else if self.agent.is_some() {
            format!("Ulanilmoqda: {}", self.status)
        } else {
            "Oflayn".to_string()
        }
    }

    /// Muvaffaqiyatli ulanishdan keyin ID kontaktlarga saqlanadi.
    pub fn remember_contact(&mut self, id: &str) {
        if self.contacts.iter().any(|c| c.id == id) {
            return;
        }
        let name = self.presence.get(id).map(|p| p.1.clone()).unwrap_or_default();
        self.contacts.push(Contact { id: id.to_string(), name });
        save_contacts(&self.contacts);
        self.subscribe();
    }

    // --- agent --------------------------------------------------------------------------------

    fn start_agent(&mut self) {
        self.agent = None;
        self.online = false;
        if self.cfg.server.trim().is_empty() {
            self.status = "server manzilini kiriting".into();
            return;
        }
        let Ok(known_hosts) = paths::known_hosts_file() else { return };
        let cfg = AgentConfig {
            server: self.cfg.server_addr(),
            key: self.cfg.key.clone(),
            name: self.cfg.name.clone(),
            id: self.cfg.id.clone(),
            token: self.cfg.token.clone(),
            known_hosts,
            host_port: None,
        };
        let policy = Policy {
            unattended: self.cfg.unattended && ondemand::has_password(),
            trusted: self.trusted.iter().cloned().collect::<HashSet<_>>(),
            ensure_host: Some(self.ondemand.ensure_fn()),
        };
        self.agent = Some(Agent::start(cfg, policy));
        self.status = "ulanilmoqda...".into();
        self.subscribe();
    }

    fn stop_agent(&mut self) {
        self.agent = None;
        self.ondemand.stop();
        self.online = false;
        self.status = "o'chirilgan".into();
        self.presence.clear();
        self.incoming.clear();
    }

    fn subscribe(&self) {
        if let Some(a) = &self.agent {
            a.subscribe(self.contacts.iter().map(|c| c.id.clone()).collect());
        }
    }

    fn sync_policy(&self) {
        if let Some(a) = &self.agent {
            let mut p = a.policy.lock().unwrap_or_else(|e| e.into_inner());
            p.unattended = self.cfg.unattended && ondemand::has_password();
            p.trusted = self.trusted.iter().cloned().collect();
        }
    }

    /// Har kadrda chaqiriladi: agent hodisalari, firewall natijasi, javob muddati.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let mut events = vec![];
        if let Some(a) = &self.agent {
            while let Some(e) = a.try_event() {
                events.push(e);
            }
        }
        for e in events {
            match e {
                AgentEvent::Status { online, text } => {
                    self.online = online;
                    self.status = text;
                }
                AgentEvent::Registered { id, token } => {
                    self.cfg.id = id;
                    self.cfg.token = token;
                    self.cfg.save();
                }
                AgentEvent::Presence { id, online, name } => {
                    if let Some(c) = self.contacts.iter_mut().find(|c| c.id == id) {
                        if c.name.is_empty() && !name.is_empty() {
                            c.name = name.clone();
                            save_contacts(&self.contacts);
                        }
                    }
                    self.presence.insert(id, (online, name));
                }
                AgentEvent::Incoming { sid, from, name } => {
                    self.incoming.push_back(Incoming { sid, from, name, since: Instant::now() });
                }
            }
        }
        // javob berilmagan so'rovlar avtomatik rad etiladi
        while self.incoming.front().is_some_and(|i| i.since.elapsed() >= Duration::from_secs(ANSWER_SECS)) {
            if let (Some(i), Some(a)) = (self.incoming.pop_front(), &self.agent) {
                a.reply(&i.sid, false);
            }
        }
        if let Some(rx) = &self.fw_rx {
            if let Ok(r) = rx.try_recv() {
                self.fw_msg = Some(match r {
                    Ok(t) => (true, t),
                    Err(t) => (false, t),
                });
                self.fw_rx = None;
            }
        }
        if self.agent.is_some() || self.server.is_some() || self.fw_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn start_server(&mut self) {
        self.server = None;
        self.server_error = None;
        let Ok(port) = self.server_port.trim().parse::<u16>() else {
            self.server_error = Some("Port 1024 dan 65535 gacha son bo'lishi kerak".into());
            self.server_on = false;
            return;
        };
        if port < 1024 {
            self.server_error = Some("Port 1024 dan 65535 gacha son bo'lishi kerak".into());
            self.server_on = false;
            return;
        }
        write_text("relay_server.pw", &self.server_key);
        match start_server(port, self.server_keyed.then_some(self.server_key.as_str())) {
            Ok(s) => {
                self.server = Some(s);
                // Shu kompyuter ham clientmi: o'z serveriga ulanish sozlamalarini to'ldiramiz
                if self.cfg.server.trim().is_empty() {
                    self.cfg.server = format!("127.0.0.1:{port}");
                    self.cfg.key = if self.server_keyed { self.server_key.clone() } else { String::new() };
                    self.cfg.save();
                }
            }
            Err(e) => {
                self.server_error = Some(e);
                self.server_on = false;
            }
        }
        self.save_server();
    }

    fn save_server(&self) {
        write_text(
            "relay_server.txt",
            &format!("enabled={}\nport={}\nkeyed={}\n", self.server_on as u8, self.server_port.trim(), self.server_keyed as u8),
        );
    }

    // --- UI -----------------------------------------------------------------------------------

    /// Kontaktlar (chap panel uchun): `(id, nom)`, onlayn holati bilan.
    fn contact_label(&self, c: &Contact) -> String {
        if c.name.is_empty() {
            relay::pretty_id(&c.id)
        } else {
            c.name.clone()
        }
    }

    /// `quality`: ulanish kartasidan oldin chiziladigan sifat tanlash kartasi.
    pub fn ui(&mut self, ui: &mut Ui, quality: &mut dyn FnMut(&mut Ui)) -> Option<ConnectRequest> {
        let mut request = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(640.0);
            ui.label(RichText::new("Server orqali").size(20.0).strong());
            theme::hint(ui, "Boshqa kompyuterning 9 xonali ID sini kiriting. Ruxsat berilgach ulanasiz.");
            ui.add_space(12.0);
            quality(ui);
            request = self.card_connect(ui);
            self.card_contacts(ui);
            self.card_this_pc(ui);
        });
        request
    }

    /// "HOST" bo'limi: bu kompyuter relay serveri bo'lishi.
    pub fn ui_server(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(640.0);
            ui.label(RichText::new("HOST: server rejimi").size(20.0).strong());
            theme::hint(ui, "Bu kompyuter boshqa kompyuterlarni bir-biriga ulab beruvchi server bo'ladi (\"Server orqali\" bo'limi uchun).");
            theme::hint(ui, "Eslatma: \"Bu kompyuter\" bo'limi esa shu kompyuterning o'zini P2P uchun ochadi, bu boshqa narsa.");
            ui.add_space(12.0);
            self.card_server(ui);
        });
    }

    pub fn server_running(&self) -> bool {
        self.server.is_some()
    }

    fn card_connect(&mut self, ui: &mut Ui) -> Option<ConnectRequest> {
        let mut request = None;
        theme::card(ui, Some("ID BO'YICHA ULANISH"), |ui| {
            let mut enter = false;
            egui::Grid::new("relay_conn").num_columns(2).spacing([14.0, 10.0]).min_col_width(70.0).show(ui, |ui| {
                ui.label(theme::dim("ID"));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.target)
                        .desired_width(200.0)
                        .hint_text("123 456 789")
                        .font(egui::TextStyle::Monospace),
                );
                enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.end_row();
                ui.label(theme::dim("Parol"));
                ui.add(egui::TextEdit::singleline(&mut self.password).password(true).desired_width(200.0).hint_text("qarovsiz parol (agar kerak bo'lsa)"));
                ui.end_row();
            });
            theme::hint(ui, "Parol: o'sha kompyuterning qarovsiz paroli (ruxsat so'ramasdan ulanish uchun). Bo'sh qoldirsangiz, u kompyuterda ruxsat oynasi chiqadi.");
            ui.add_space(6.0);
            if let Some(e) = &self.error {
                theme::error_banner(ui, e);
                ui.add_space(6.0);
            }
            let id = relay::normalize_id(&self.target);
            let can = relay::valid_id(&id) && self.online;
            if !self.online {
                ui.label(RichText::new("Avval pastda serverga ulaning (\"Bu kompyuter\" kartasi).").color(theme::WARN));
            }
            if theme::accent_button(ui, "Ulanish", egui::vec2(160.0, 34.0), can).clicked() || (enter && can) {
                self.error = None;
                request = Some(ConnectRequest { id, password: self.password.clone() });
            }
        });
        request
    }

    fn card_contacts(&mut self, ui: &mut Ui) {
        let mut pick: Option<String> = None;
        let mut delete: Option<usize> = None;
        theme::card(ui, Some("KONTAKTLAR"), |ui| {
            if self.contacts.is_empty() {
                theme::hint(ui, "Muvaffaqiyatli ulanishdan keyin ID shu yerda saqlanadi.");
            }
            for (i, c) in self.contacts.iter().enumerate() {
                let on = self.presence.get(&c.id).is_some_and(|p| p.0);
                ui.horizontal(|ui| {
                    theme::dot(ui, if on { theme::GOOD } else { theme::DIM });
                    if ui.link(RichText::new(self.contact_label(c)).strong()).clicked() {
                        pick = Some(c.id.clone());
                    }
                    ui.label(RichText::new(relay::pretty_id(&c.id)).monospace().size(12.0).color(theme::DIM));
                    ui.label(theme::dim(if on { "onlayn" } else { "oflayn" }));
                    if ui.small_button("×").clicked() {
                        delete = Some(i);
                    }
                });
            }
        });
        if let Some(id) = pick {
            self.target = relay::pretty_id(&id);
        }
        if let Some(i) = delete {
            self.contacts.remove(i);
            save_contacts(&self.contacts);
            self.subscribe();
        }
    }

    fn card_this_pc(&mut self, ui: &mut Ui) {
        let mut apply = false;
        let mut toggle: Option<bool> = None;
        let mut untrust: Option<usize> = None;
        let mut save_pw = false;
        let mut policy_changed = false;
        theme::card(ui, Some("BU KOMPYUTER (SERVERGA ULANISH)"), |ui| {
            let mut on = self.cfg.enabled;
            if ui.checkbox(&mut on, "Serverga ulanib, ID orqali ulanishga ruxsat berish").changed() {
                toggle = Some(on);
            }
            ui.add_space(4.0);
            egui::Grid::new("relay_cfg").num_columns(2).spacing([14.0, 8.0]).min_col_width(90.0).show(ui, |ui| {
                ui.label(theme::dim("Server"));
                ui.add(egui::TextEdit::singleline(&mut self.cfg.server).desired_width(260.0).hint_text("IP yoki nom (port 7780)"));
                ui.end_row();
                ui.label(theme::dim("Server kaliti"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.cfg.key).hint_text("ixtiyoriy").password(!self.show_key).desired_width(200.0));
                    if ui.small_button(if self.show_key { "Yashirish" } else { "Ko'rsatish" }).clicked() {
                        self.show_key = !self.show_key;
                    }
                });
                ui.end_row();
                ui.label(theme::dim("Nom"));
                ui.add(egui::TextEdit::singleline(&mut self.cfg.name).desired_width(260.0));
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let (color, text) = if self.online {
                    (theme::GOOD, "onlayn".to_string())
                } else {
                    (theme::DIM, self.status.clone())
                };
                theme::chip(ui, if self.online { "onlayn" } else { "oflayn" }, color);
                if !self.online || !self.status.is_empty() {
                    ui.label(theme::dim(&text));
                }
            });
            if !self.cfg.id.is_empty() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(theme::dim("Mening ID'm"));
                    ui.label(RichText::new(relay::pretty_id(&self.cfg.id)).monospace().size(20.0).strong());
                    if ui.small_button("Nusxalash").clicked() {
                        ui.ctx().output_mut(|o| o.copied_text = self.cfg.id.clone());
                    }
                });
            }
            ui.add_space(8.0);
            ui.label(theme::dim("Kiruvchi so'rovlar"));
            if ui.radio_value(&mut self.cfg.unattended, false, "Har safar ruxsat so'rash (30 soniyadan keyin rad etiladi)").changed() {
                policy_changed = true;
            }
            if ui.radio_value(&mut self.cfg.unattended, true, "Qarovsiz: ruxsat so'ramasdan, alohida parol bilan").changed() {
                policy_changed = true;
            }
            if self.cfg.unattended {
                ui.horizontal(|ui| {
                    ui.add_space(22.0);
                    let hint = if ondemand::has_password() { "saqlangan (yangisini kiriting)" } else { "parol (kamida 8 belgi)" };
                    ui.add(egui::TextEdit::singleline(&mut self.unattended_pw).password(true).desired_width(200.0).hint_text(hint));
                    if ui.button("Saqlash").clicked() {
                        save_pw = true;
                    }
                });
                match (&self.unattended_msg, ondemand::has_password()) {
                    (Some((ok, m)), _) => {
                        ui.colored_label(if *ok { theme::GOOD } else { theme::WARN }, m);
                    }
                    (None, true) => {
                        ui.colored_label(theme::GOOD, "Parol saqlangan.");
                    }
                    (None, false) => {
                        ui.colored_label(theme::WARN, "Parol o'rnatilmaguncha so'rovlar har safar ruxsat bilan qabul qilinadi.");
                    }
                }
                theme::hint(
                    ui,
                    "Bu parol faqat ID orqali qarovsiz ulanish uchun. Host paroli va \"Ulanishga ruxsat berish\" dan mustaqil: kompyuter serverga ulangan bo'lsa yetarli.",
                );
            } else {
                theme::hint(ui, "Har bir ulanishga bir martalik parol beriladi; Host kerak bo'lsa o'zi ishga tushadi.");
            }
            if !self.trusted.is_empty() {
                ui.add_space(6.0);
                ui.label(theme::dim("Ishonchli ID'lar (ruxsat so'ralmaydi)"));
                for (i, id) in self.trusted.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(relay::pretty_id(id)).monospace());
                        if ui.small_button("Olib tashlash").clicked() {
                            untrust = Some(i);
                        }
                    });
                }
            }
            ui.add_space(8.0);
            if theme::accent_button(ui, "Saqlash va ulanish", egui::vec2(180.0, 32.0), self.cfg.enabled).clicked() {
                apply = true;
            }
        });
        if let Some(i) = untrust {
            self.trusted.remove(i);
            save_trusted(&self.trusted);
            policy_changed = true;
        }
        if let Some(on) = toggle {
            self.cfg.enabled = on;
            self.cfg.save();
            if on {
                self.start_agent();
            } else {
                self.stop_agent();
            }
        }
        if apply {
            self.cfg.save();
            self.start_agent();
        }
        if save_pw {
            match ondemand::save_password(&self.unattended_pw) {
                Ok(()) => {
                    self.unattended_pw.clear();
                    self.ondemand.stop();
                    self.unattended_msg = Some((true, "Parol saqlandi.".into()));
                    policy_changed = true;
                }
                Err(e) => self.unattended_msg = Some((false, format!("{e:#}"))),
            }
        }
        if policy_changed {
            self.cfg.save();
            self.sync_policy();
        }
    }

    fn card_server(&mut self, ui: &mut Ui) {
        let mut toggle: Option<bool> = None;
        let mut new_key = false;
        let mut open_fw = false;
        theme::card(ui, Some("SERVER REJIMI (BU KOMPYUTER SERVER BO'LADI)"), |ui| {
            let mut on = self.server_on;
            if ui.checkbox(&mut on, "Server rejimini yoqish").changed() {
                toggle = Some(on);
            }
            theme::hint(ui, "Faqat oq (statik) IP li kompyuterda yoqing. Clientlar shu manzil bilan ulanadi.");
            ui.add_space(6.0);
            let running = self.server.is_some();
            let mut keyed = self.server_keyed;
            if ui.add_enabled(!running, egui::Checkbox::new(&mut keyed, "Kalit bilan himoyalash")).changed() {
                self.server_keyed = keyed;
                self.save_server();
            }
            if !self.server_keyed {
                theme::hint(ui, "Kalitsiz: server manzilini bilgan har kim ro'yxatdan o'tadi va server trafigidan foydalanadi. Ulanish baribir qabul qiluvchining ruxsati yoki parolini talab qiladi.");
            }
            ui.add_space(4.0);
            egui::Grid::new("relay_srv").num_columns(2).spacing([14.0, 8.0]).min_col_width(90.0).show(ui, |ui| {
                ui.label(theme::dim("Port (UDP)"));
                ui.add_enabled(!running, egui::TextEdit::singleline(&mut self.server_port).desired_width(80.0));
                ui.end_row();
                if self.server_keyed {
                    ui.label(theme::dim("Server kaliti"));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&self.server_key).monospace());
                        if ui.small_button("Nusxalash").clicked() {
                            ui.ctx().output_mut(|o| o.copied_text = self.server_key.clone());
                        }
                        if ui.add_enabled(!running, egui::Button::new("Yangilash").small()).clicked() {
                            new_key = true;
                        }
                    });
                    ui.end_row();
                }
            });
            if let Some(e) = &self.server_error {
                ui.add_space(6.0);
                theme::error_banner(ui, e);
            }
            if let Some(s) = &self.server {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    theme::chip(ui, "server ishlayapti", theme::GOOD);
                    if ui.button("Firewall'da portni ochish").clicked() {
                        open_fw = true;
                    }
                });
                if let Some((ok, t)) = &self.fw_msg {
                    ui.label(RichText::new(t).color(if *ok { theme::GOOD } else { theme::BAD }));
                }
                ui.add_space(6.0);
                ui.label(theme::dim("Ulangan clientlar"));
                let clients = s.handle.clients();
                if clients.is_empty() {
                    theme::hint(ui, "Hozircha hech kim ro'yxatdan o'tmagan.");
                }
                let now = now_secs();
                for c in clients {
                    ui.horizontal(|ui| {
                        theme::dot(ui, if c.online { theme::GOOD } else { theme::DIM });
                        ui.label(RichText::new(if c.name.is_empty() { "—".to_string() } else { c.name }).strong());
                        ui.label(RichText::new(relay::pretty_id(&c.id)).monospace().size(12.0).color(theme::DIM));
                        let when = if c.online {
                            c.ip.map(|ip| ip.to_string()).unwrap_or_else(|| "onlayn".into())
                        } else {
                            ago(now.saturating_sub(c.last_seen))
                        };
                        ui.label(theme::dim(&when));
                    });
                }
            }
        });
        if new_key {
            self.server_key = random_key();
            write_text("relay_server.pw", &self.server_key);
        }
        if open_fw {
            if let Ok(port) = self.server_port.trim().parse::<u16>() {
                let (tx, rx) = mpsc::channel();
                self.fw_rx = Some(rx);
                self.fw_msg = Some((true, "Firewall sozlanmoqda (ruxsat so'ralishi mumkin)...".into()));
                std::thread::spawn(move || {
                    let _ = tx.send(firewall::open_udp(port));
                });
            }
        }
        if let Some(on) = toggle {
            self.server_on = on;
            if on {
                self.start_server();
            } else {
                self.server = None;
                self.fw_msg = None;
            }
            self.save_server();
        }
    }

    /// Kiruvchi so'rov oynasi (har qanday ekranda ko'rinadi).
    pub fn incoming_dialog(&mut self, ctx: &egui::Context) {
        let Some(i) = self.incoming.front() else { return };
        let left = ANSWER_SECS.saturating_sub(i.since.elapsed().as_secs());
        let who = if i.name.is_empty() { "Noma'lum kompyuter".to_string() } else { i.name.clone() };
        let (sid, from) = (i.sid.clone(), i.from.clone());
        let mut answer: Option<(bool, bool)> = None; // (ruxsat, doim)
        egui::Window::new("Ulanish so'rovi")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(RichText::new(who).size(16.0).strong());
                ui.label(RichText::new(relay::pretty_id(&from)).monospace().color(theme::DIM));
                ui.add_space(4.0);
                ui.label("sizning kompyuteringizni ko'rish va boshqarish uchun ulanmoqchi.");
                theme::hint(ui, &format!("{left} soniyadan keyin avtomatik rad etiladi."));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if theme::accent_button(ui, "Ruxsat berish", egui::vec2(130.0, 32.0), true).clicked() {
                        answer = Some((true, false));
                    }
                    if ui.add(egui::Button::new("Doim ruxsat").min_size(egui::vec2(110.0, 32.0))).clicked() {
                        answer = Some((true, true));
                    }
                    if theme::danger_button(ui, "Rad etish").clicked() {
                        answer = Some((false, false));
                    }
                });
            });
        ctx.request_repaint_after(Duration::from_millis(500));
        if let Some((yes, always)) = answer {
            self.incoming.pop_front();
            if yes && always && !self.trusted.contains(&from) {
                self.trusted.push(from.clone());
                save_trusted(&self.trusted);
                self.sync_policy();
            }
            if let Some(a) = &self.agent {
                a.reply(&sid, yes);
            }
        }
    }
}

fn ago(secs: u64) -> String {
    match secs {
        0..=59 => "hozirgina".into(),
        60..=3599 => format!("{} daqiqa oldin", secs / 60),
        3600..=86_399 => format!("{} soat oldin", secs / 3600),
        _ => format!("{} kun oldin", secs / 86_400),
    }
}
