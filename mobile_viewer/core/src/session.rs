//! Bitta ulanish: relay orqali ulanish, video kadrlarni qabul qilish va kiritishni yuborish.
//! `crates/viewer/src/gui/session.rs` mantig'i, lekin egui/openh264 siz.
//!
//! Aloqa uzilsa (tarmoq o'zgardi, vaqtincha internet yo'q, Host qayta ishga tushdi) sessiya darrov tugamaydi:
//! `RECONNECT_WINDOW` ichida avtomatik qayta ulanishga urinadi (`Event::Reconnecting`). Doimiy xatolar
//! (kalit noto'g'ri, fingerprint o'zgargan, Host ulanishni uzdi, foydalanuvchi yopdi) darrov tugatadi.

use anyhow::Result;
use protocol::{InputEvent, DEFAULT_PORT};
use rdcore::{
    client::{connect_relayed, connect_with, ConnectOptions, Rejected, Transport, VideoRefused},
    known_hosts::KnownHosts,
    paths,
    relay::agent::Connector,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, TrySendError},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// Uzilgandan keyin shuncha vaqt davomida qayta ulanishga urinadi.
const RECONNECT_WINDOW: Duration = Duration::from_secs(90);
/// Pong shuncha vaqt kelmasa aloqa uzilgan hisoblanadi (mobil tarmoqdagi qisqa uzilishlarga chidamli).
const PONG_WAIT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub struct Params {
    /// Server (relay) orqali ulanishda qabul qiluvchi ID.
    pub id: String,
    /// P2P (to'g'ridan-to'g'ri) ulanishda Host manzili: `IP`, `IP:port` yoki `nom[:port]`.
    pub addr: String,
    pub password: String,
    pub fps: u16,
    pub kbps: u32,
    pub height: u16,
    /// `Some` = server orqali, `None` = P2P (`addr`).
    pub connector: Option<Connector>,
}

/// Kadr: kalit-kadr belgisi + Annex-B H.264 baytlari.
pub struct Frame {
    pub keyframe: bool,
    pub data: Vec<u8>,
}

#[derive(Default)]
pub struct Stats {
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    pub rtt_us: AtomicU64,
}

pub enum Event {
    /// Birinchi ulanish: fingerprint'ni foydalanuvchi tasdiqlashi kerak (`Sess::answer_confirm`).
    Confirm(String),
    Connected { fingerprint: String, via_tcp: bool },
    VideoStarted,
    /// Aloqa uzildi, avtomatik qayta ulanilmoqda (`attempt` 1 dan boshlanadi; `reason` = uzilish sababi).
    Reconnecting { attempt: u32, reason: String },
    /// Ulanish tugadi. `Some` = xato matni.
    Ended(Option<String>),
}

enum Cmd {
    Input(InputEvent),
    Close,
    /// Qurilma tarmog'i o'zgardi: eski ulanish yaroqsiz, darrov yangisini ochish kerak.
    NetworkChanged,
}

/// Bitta urinish natijasi.
enum End {
    /// Foydalanuvchi yopdi.
    Closed,
    /// Doimiy: qayta urinish foyda bermaydi (matn foydalanuvchiga ko'rsatiladi).
    Fatal(String),
    /// Vaqtincha uzilish: qayta urinish mumkin.
    Lost(String),
}

pub struct Sess {
    cmd: UnboundedSender<Cmd>,
    events: Mutex<mpsc::Receiver<Event>>,
    frames: Mutex<mpsc::Receiver<Frame>>,
    confirm: Arc<Mutex<Option<mpsc::Sender<bool>>>>,
    pub stats: Arc<Stats>,
}

impl Sess {
    pub fn input(&self, ev: InputEvent) {
        let _ = self.cmd.send(Cmd::Input(ev));
    }

    pub fn close(&self) {
        let _ = self.cmd.send(Cmd::Close);
    }

    /// Tarmoq (Wi-Fi / mobil internet) o'zgardi: ulanish darrov qayta o'rnatiladi.
    pub fn network_changed(&self) {
        let _ = self.cmd.send(Cmd::NetworkChanged);
    }

    pub fn poll_event(&self) -> Option<Event> {
        self.events.lock().ok()?.try_recv().ok()
    }

    /// Kadr kutadi (bloklaydi, eng ko'pi bilan `timeout`).
    pub fn next_frame(&self, timeout: Duration) -> Option<Frame> {
        self.frames.lock().ok()?.recv_timeout(timeout).ok()
    }

    pub fn answer_confirm(&self, accept: bool) {
        if let Some(tx) = self.confirm.lock().ok().and_then(|mut c| c.take()) {
            let _ = tx.send(accept);
        }
    }
}

impl Drop for Sess {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Close);
        // Kutayotgan fingerprint so'rovi bo'lsa, rad etiladi (oqim osilib qolmasligi uchun).
        self.answer_confirm(false);
    }
}

pub fn start(p: Params) -> Arc<Sess> {
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    let (frame_tx, frame_rx) = mpsc::sync_channel::<Frame>(8);
    let stats = Arc::new(Stats::default());
    let confirm = Arc::new(Mutex::new(None));
    let sess = Arc::new(Sess {
        cmd: cmd_tx,
        events: Mutex::new(ev_rx),
        frames: Mutex::new(frame_rx),
        confirm: confirm.clone(),
        stats: stats.clone(),
    });
    std::thread::spawn(move || {
        let ended = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(rt) => rt.block_on(run(p, stats, ev_tx.clone(), confirm, frame_tx, cmd_rx)),
            Err(e) => Err(e.into()),
        };
        let msg = match ended {
            Ok(m) => m,
            Err(e) => Some(format!("{e:#}")),
        };
        let _ = ev_tx.send(Event::Ended(msg));
    });
    sess
}

async fn resolve(target: &str) -> Result<SocketAddr> {
    let target = target.trim();
    if let Ok(a) = target.parse::<SocketAddr>() {
        return Ok(a);
    }
    if let Ok(ip) = target.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    let with_port = if target.contains(':') { target.to_string() } else { format!("{target}:{DEFAULT_PORT}") };
    let mut addrs = tokio::net::lookup_host(with_port)
        .await
        .map_err(|e| anyhow::anyhow!("{target} manzilini aniqlab bo'lmadi: {e}"))?;
    addrs.next().ok_or_else(|| anyhow::anyhow!("{target} uchun manzil topilmadi"))
}

/// Relay serveri rad etdi: vaqtincha sabablar (kompyuter oflayn, agent qayta ulanmoqda, javob kelmadi) qayta uriniladi;
/// qolganlari ("ruxsat berilmadi", "bunday ID yo'q" ...) doimiy.
fn relay_retryable(text: &str) -> bool {
    ["oflayn", "aloqa yo'q", "vaqt tugadi", "oqim ochilmadi"].iter().any(|k| text.contains(k))
}

/// Qayta urinishdan oldingi kutish. `true` = foydalanuvchi yopdi. Tarmoq o'zgarsa kutish qisqartiriladi.
async fn wait_or_close(cmd_rx: &mut UnboundedReceiver<Cmd>, dur: Duration) -> bool {
    let sleep = tokio::time::sleep(dur);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return false,
            c = cmd_rx.recv() => match c {
                Some(Cmd::Close) | None => return true,
                Some(Cmd::NetworkChanged) => return false,
                Some(Cmd::Input(_)) => {}
            },
        }
    }
}

/// Ulanish ochilguncha foydalanuvchi "yopish" bosishini kutadi (boshqa buyruqlar tashlanadi).
async fn wait_close(cmd_rx: &mut UnboundedReceiver<Cmd>) {
    loop {
        match cmd_rx.recv().await {
            Some(Cmd::Close) | None => return,
            Some(_) => {}
        }
    }
}

async fn run(
    p: Params,
    stats: Arc<Stats>,
    ev_tx: mpsc::Sender<Event>,
    confirm_slot: Arc<Mutex<Option<mpsc::Sender<bool>>>>,
    frame_tx: mpsc::SyncSender<Frame>,
    mut cmd_rx: UnboundedReceiver<Cmd>,
) -> Result<Option<String>> {
    let known_hosts = KnownHosts::new(paths::known_hosts_file()?);

    // Fingerprint tasdig'i: UI'ga so'rov yuboriladi va javob kutiladi (bu oqim faqat shu ulanishniki).
    let confirm_tx = ev_tx.clone();
    let confirm = move |fp: &str| -> bool {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut slot) = confirm_slot.lock() {
            *slot = Some(tx);
        }
        if confirm_tx.send(Event::Confirm(fp.to_string())).is_err() {
            return false;
        }
        rx.recv().unwrap_or(false)
    };

    // `ever`: sessiya kamida bir marta to'liq (video bilan) ochilgan; faqat shundan keyin qayta uriniladi.
    let mut ever = false;
    let mut deadline: Option<Instant> = None;
    let mut attempt = 0u32;
    loop {
        let mut up = false;
        let end = match run_once(&p, &stats, &ev_tx, &confirm, &known_hosts, &frame_tx, &mut cmd_rx, &mut up).await {
            Ok(end) => end,
            // Kalit rad etildi / fingerprint o'zgargan / Host videoni rad etdi: qayta urinish befoyda.
            Err(e) if e.downcast_ref::<Rejected>().is_some() || e.downcast_ref::<VideoRefused>().is_some() => return Err(e),
            Err(e) if ever => End::Lost(format!("{e:#}")),
            Err(e) => return Err(e),
        };
        if up {
            ever = true;
            deadline = None;
            attempt = 0;
        }
        let reason = match end {
            End::Closed => return Ok(None),
            End::Fatal(m) => return Ok(Some(m)),
            End::Lost(m) => m,
        };
        if !ever {
            return Ok(Some(reason));
        }
        let deadline = *deadline.get_or_insert_with(|| Instant::now() + RECONNECT_WINDOW);
        if Instant::now() >= deadline {
            return Ok(Some(format!("{reason}\n(qayta ulanib bo'lmadi)")));
        }
        attempt += 1;
        let _ = ev_tx.send(Event::Reconnecting { attempt, reason });
        // 1, 2, 4, 8, 15, 15 ... soniya
        let wait = Duration::from_secs((1u64 << (attempt - 1).min(4)).min(15));
        if wait_or_close(&mut cmd_rx, wait).await {
            return Ok(None);
        }
    }
}

/// Bitta ulanish urinishi: ulanadi, video so'raydi va uzilguncha ishlaydi. `up` = video boshlandi.
#[allow(clippy::too_many_arguments)]
async fn run_once(
    p: &Params,
    stats: &Arc<Stats>,
    ev_tx: &mpsc::Sender<Event>,
    confirm: &dyn Fn(&str) -> bool,
    known_hosts: &KnownHosts,
    frame_tx: &mpsc::SyncSender<Frame>,
    cmd_rx: &mut UnboundedReceiver<Cmd>,
    up: &mut bool,
) -> Result<End> {
    let mut session = match &p.connector {
        Some(connector) => {
            // Server qabul qiluvchidan ruxsat so'raydi (u 30 soniya o'ylaydi)
            // Parol kiritilmagan bo'lsa: qarovsiz Host ham ruxsat oynasini ko'rsatadi (xato emas)
            let asked = connector.connect_ask(&p.id, p.password.is_empty());
            let relayed = tokio::select! {
                r = asked => match r {
                    Err(_) => return Ok(End::Lost("relay agenti bilan aloqa uzildi".into())),
                    Ok(Err(text)) => return Ok(if relay_retryable(&text) { End::Lost(text) } else { End::Fatal(text) }),
                    Ok(Ok(relayed)) => relayed,
                },
                _ = wait_close(cmd_rx) => return Ok(End::Closed),
            };
            let password = relayed.pw.clone().unwrap_or_else(|| p.password.clone());
            if password.is_empty() {
                return Ok(End::Fatal("Host kalit bermadi: kalitni kiriting".into()));
            }
            connect_relayed(
                ConnectOptions { addr: relayed.local, password: &password, known_hosts, confirm_new_host: confirm },
                format!("id:{}", p.id),
            )
            .await?
        }
        None => {
            if p.password.is_empty() {
                return Ok(End::Fatal("kalitni kiriting".into()));
            }
            let addr = resolve(&p.addr).await?;
            connect_with(
                ConnectOptions { addr, password: &p.password, known_hosts, confirm_new_host: confirm },
                Transport::Auto,
            )
            .await?
        }
    };
    let _ = ev_tx.send(Event::Connected { fingerprint: session.fingerprint.clone(), via_tcp: session.via_tcp });

    let mut stream = session.request_video(p.fps, p.kbps, p.height, 0).await?;
    let _ = ev_tx.send(Event::VideoStarted);
    *up = true;

    // next_frame() ni timeout/select bilan uzib bo'lmaydi (oqim buziladi): alohida vazifa, kerak bo'lsa abort.
    let rx_stats = stats.clone();
    let frame_tx = frame_tx.clone();
    let mut recv_task = tokio::spawn(async move {
        let mut need_key = false;
        loop {
            match stream.next_frame().await {
                Ok(Some(f)) => {
                    rx_stats.frames.fetch_add(1, Ordering::Relaxed);
                    rx_stats.bytes.fetch_add(f.data.len() as u64, Ordering::Relaxed);
                    if need_key && !f.keyframe {
                        continue;
                    }
                    match frame_tx.try_send(Frame { keyframe: f.keyframe, data: f.data }) {
                        Ok(()) => need_key = false,
                        Err(TrySendError::Full(_)) => need_key = true,
                        Err(TrySendError::Disconnected(_)) => return None,
                    }
                }
                Ok(None) => return Some("Host videoni to'xtatdi".to_string()),
                Err(e) => return Some(format!("video: {e:#}")),
            }
        }
    });

    let mut ping = tokio::time::interval(Duration::from_secs(2));
    let end = loop {
        tokio::select! {
            c = cmd_rx.recv() => match c {
                Some(Cmd::Input(ev)) => {
                    if let Err(e) = session.send_input(ev).await {
                        break End::Lost(format!("{e:#}"));
                    }
                }
                Some(Cmd::NetworkChanged) => break End::Lost("tarmoq o'zgardi".into()),
                Some(Cmd::Close) | None => break End::Closed,
            },
            _ = ping.tick() => match session.ping_within(PONG_WAIT).await {
                Ok(d) => stats.rtt_us.store(d.as_micros() as u64, Ordering::Relaxed),
                Err(e) => break End::Lost(format!("aloqa uzildi: {e:#}")),
            },
            r = &mut recv_task => break match r {
                Ok(None) => End::Closed,
                Ok(Some(m)) => End::Lost(m),
                Err(_) => End::Lost("video vazifasi to'xtadi".to_string()),
            },
        }
    };
    recv_task.abort();
    // Host ulanishni ataylab yopgan bo'lsa (masalan "Uzish"), qayta ulanmaymiz.
    let end = match end {
        End::Lost(m) if session.closed_by_peer() => End::Fatal(format!("{m}\n(Host ulanishni yopdi)")),
        e => e,
    };
    session.close().await;
    Ok(end)
}
