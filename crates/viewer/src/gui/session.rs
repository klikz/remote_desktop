//! Bitta ulanish: tarmoq (alohida oqim va tokio runtime), H.264 dekodlash va UI bilan aloqa.

use anyhow::{Context, Result};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use protocol::{video_reason, InputEvent, DEFAULT_PORT};
use rdcore::{
    client::{connect_relayed, connect_with, ConnectOptions, Rejected, Transport, VideoRefused},
    filexfer,
    pacing::{ViewerFeedback, FEEDBACK_INTERVAL},
    relay::agent::Connector,
};
use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, TrySendError},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
pub use rdcore::filexfer::RemoteFile;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Clone)]
pub struct Params {
    pub addr: String,
    pub password: String,
    pub fps: u16,
    pub kbps: u32,
    pub height: u16,
    pub transport: Transport,
    /// Server (relay) orqali ulanish: qabul qiluvchi ID va agent tutqichi (`addr` e'tiborsiz qoldiriladi).
    pub relay: Option<(String, Connector)>,
}

/// Dekodlangan kadr: tekstura uchun tayyor (rang dekoder oqimida o'girilgan, UI oqimi faqat yuklaydi).
pub struct Picture {
    pub image: egui::ColorImage,
}

/// Tarmoq oqimi va UI o'rtasida bo'lishiladigan holat.
#[derive(Default)]
pub struct Shared {
    pub picture: Mutex<Option<Picture>>,
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    pub rtt_us: AtomicU64,
    /// Diagnostika (`RD_DIAG`): dekodlangan kadrlar, dekoder navbati va kalit kadr kutilishi.
    pub decoded: AtomicU64,
    pub queue: AtomicU64,
    pub need_key: std::sync::atomic::AtomicBool,
}

pub enum Event {
    Connected { via_tcp: bool },
    VideoStarted,
    /// Host ekranni ola olmadi (xato matni). Ulanish ochiq: ruxsatni qaytadan berish mumkin.
    VideoRefused(String),
    /// Aloqa uzildi, avtomatik qayta ulanilmoqda (`attempt` 1 dan boshlanadi; `reason` = uzilish sababi).
    Reconnecting { attempt: u32, reason: String },
    /// Ulanish tugadi. `Some` = xato matni.
    Ended(Option<String>),
    /// Host'dagi almashinuv papkasi ro'yxati.
    Files(Result<Vec<RemoteFile>, String>),
    /// Fayl uzatish holati. `result`: `None` = davom etmoqda, `Some` = tugadi (`Ok` = saqlangan yo'l / xabar).
    Transfer { id: u64, done: u64, total: u64, result: Option<Result<String, String>> },
}

enum Cmd {
    ResetPermissions,
    ListFiles,
    SendFile(u64, PathBuf),
    GetFile(u64, String),
    Close,
}

pub struct Handle {
    cmd: UnboundedSender<Cmd>,
    /// Kiritish alohida kanalda: tarmoq sikli navbatdagilarni birdaniga olib, bitta yozuv bilan yuboradi.
    input: UnboundedSender<InputEvent>,
    pub events: mpsc::Receiver<Event>,
    pub shared: Arc<Shared>,
    next_transfer: AtomicU64,
}

impl Handle {
    pub fn list_files(&self) {
        let _ = self.cmd.send(Cmd::ListFiles);
    }
    /// Faylni Host'ga yuboradi. Uzatish identifikatorini qaytaradi (`Event::Transfer`).
    pub fn send_file(&self, path: PathBuf) -> u64 {
        let id = self.next_transfer.fetch_add(1, Ordering::Relaxed);
        let _ = self.cmd.send(Cmd::SendFile(id, path));
        id
    }
    /// Host'dagi faylni yuklab oladi. Uzatish identifikatorini qaytaradi (`Event::Transfer`).
    pub fn get_file(&self, name: String) -> u64 {
        let id = self.next_transfer.fetch_add(1, Ordering::Relaxed);
        let _ = self.cmd.send(Cmd::GetFile(id, name));
        id
    }
    pub fn input(&self, ev: InputEvent) {
        let _ = self.input.send(ev);
    }
    pub fn close(&self) {
        let _ = self.cmd.send(Cmd::Close);
    }
    /// Host'dagi saqlangan ekran ruxsatini bekor qilib, qaytadan so'rashni talab qiladi.
    pub fn reset_permissions(&self) {
        let _ = self.cmd.send(Cmd::ResetPermissions);
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Close);
    }
}

pub fn start(ctx: egui::Context, p: Params) -> Handle {
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (input_tx, input_rx) = unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    let shared = Arc::new(Shared::default());
    let sh = shared.clone();
    std::thread::spawn(move || {
        let ended = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(rt) => rt.block_on(run(p, ctx.clone(), sh, ev_tx.clone(), cmd_rx, input_rx)),
            Err(e) => Err(e.into()),
        };
        let msg = match ended {
            Ok(m) => m,
            Err(e) => Some(format!("{e:#}")),
        };
        let _ = ev_tx.send(Event::Ended(msg));
        ctx.request_repaint();
    });
    Handle { cmd: cmd_tx, input: input_tx, events: ev_rx, shared, next_transfer: AtomicU64::new(1) }
}

/// Uzatish borishini UI'ga yetkazadi (juda tez-tez emas: 100 ms da bir marta).
struct Reporter {
    id: u64,
    ev_tx: mpsc::Sender<Event>,
    ctx: egui::Context,
    last: Instant,
}

impl Reporter {
    fn progress(&mut self, done: u64, total: u64) {
        if done < total && self.last.elapsed() < Duration::from_millis(100) {
            return;
        }
        self.last = Instant::now();
        let _ = self.ev_tx.send(Event::Transfer { id: self.id, done, total, result: None });
        self.ctx.request_repaint();
    }

    fn finish(&self, total: u64, result: Result<String, String>) {
        let done = if result.is_ok() { total } else { 0 };
        let _ = self.ev_tx.send(Event::Transfer { id: self.id, done, total, result: Some(result) });
        self.ctx.request_repaint();
    }
}

/// Fayl buyruqlarini alohida vazifalarda bajaradi (video va boshqa uzatishlar to'xtamaydi).
fn spawn_file_cmd(conn: rdcore::quinn::Connection, ctx: &egui::Context, ev_tx: &mpsc::Sender<Event>, cmd: Cmd) {
    let reporter = |id| Reporter { id, ev_tx: ev_tx.clone(), ctx: ctx.clone(), last: Instant::now() };
    match cmd {
        Cmd::ListFiles => {
            let (ev_tx, ctx) = (ev_tx.clone(), ctx.clone());
            tokio::spawn(async move {
                let r = filexfer::list(&conn).await.map_err(|e| format!("{e:#}"));
                let _ = ev_tx.send(Event::Files(r));
                ctx.request_repaint();
            });
        }
        Cmd::SendFile(id, path) => {
            let mut rep = reporter(id);
            tokio::spawn(async move {
                let total = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
                let r = filexfer::upload(&conn, &path, &mut |d, t| rep.progress(d, t)).await;
                rep.finish(total, r.map(|_| "Host'ga yuborildi".to_string()).map_err(|e| format!("{e:#}")));
            });
        }
        Cmd::GetFile(id, name) => {
            let mut rep = reporter(id);
            tokio::spawn(async move {
                let r = filexfer::download(&conn, &name, &mut |d, t| rep.progress(d, t)).await;
                rep.finish(0, r.map(|p| p.display().to_string()).map_err(|e| format!("{e:#}")));
            });
        }
        _ => {}
    }
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
        .with_context(|| format!("{target} manzilini aniqlab bo'lmadi"))?;
    let first = addrs.next();
    first.with_context(|| format!("{target} uchun manzil topilmadi"))
}

/// Sessiya video bilan ochilgandan keyin aloqa uzilsa, shu vaqt ichida avtomatik qayta ulanishga uriniladi.
const RECONNECT_WINDOW: Duration = Duration::from_secs(90);

/// Dekoder ortda qolganda ham ekran kamida shu oraliqda yangilanadi.
const MAX_SHOW_GAP: Duration = Duration::from_millis(100);

/// Bitta ulanish urinishi qanday tugadi.
enum End {
    /// Foydalanuvchi yopdi.
    Closed,
    /// Qayta urinish befoyda (Host ataylab yopdi, ruxsat berilmadi ...).
    Fatal(String),
    /// Tarmoq uzildi: qayta ulanish mumkin.
    Lost(String),
}

/// Relay serveri rad etdi: vaqtincha sabablar (kompyuter oflayn, agent qayta ulanmoqda, javob kelmadi) qayta uriniladi;
/// qolganlari ("ruxsat berilmadi", "bunday ID yo'q" ...) doimiy.
fn relay_retryable(text: &str) -> bool {
    ["oflayn", "aloqa yo'q", "vaqt tugadi", "oqim ochilmadi"].iter().any(|k| text.contains(k))
}

/// Qayta urinishdan oldingi kutish. `true` = foydalanuvchi yopdi.
async fn wait_or_close(cmd_rx: &mut UnboundedReceiver<Cmd>, dur: Duration) -> bool {
    let sleep = tokio::time::sleep(dur);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            _ = &mut sleep => return false,
            c = cmd_rx.recv() => match c {
                Some(Cmd::Close) | None => return true,
                Some(_) => {}
            },
        }
    }
}

/// Ulanish ochilguncha foydalanuvchi "Bekor qilish" bosishini kutadi (boshqa buyruqlar tashlanadi).
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
    ctx: egui::Context,
    shared: Arc<Shared>,
    ev_tx: mpsc::Sender<Event>,
    mut cmd_rx: UnboundedReceiver<Cmd>,
    mut input_rx: UnboundedReceiver<InputEvent>,
) -> Result<Option<String>> {
    let fb = Arc::new(ViewerFeedback::default());

    // Dekoder oqimi (qayta ulanishlarda ham bitta): eng yangi kadr `shared.picture` ga yoziladi. Xato bo'lsa keyingi
    // kalit kadrgacha kadrlar tashlanadi va Host'dan kalit kadr so'raladi (Host kalit kadrni faqat so'ralganda yuboradi).
    let (ftx, frx) = mpsc::sync_channel::<Vec<u8>>(8);
    {
        let shared = shared.clone();
        let ctx = ctx.clone();
        let fb = fb.clone();
        let (init_tx, init_rx) = mpsc::channel::<Result<(), String>>();
        std::thread::spawn(move || {
            let mut decoder = match Decoder::new() {
                Ok(d) => {
                    let _ = init_tx.send(Ok(()));
                    d
                }
                Err(e) => {
                    let _ = init_tx.send(Err(e.to_string()));
                    return;
                }
            };
            let mut shown = Instant::now();
            while let Ok(data) = frx.recv() {
                fb.dequeued();
                match decoder.decode(&data) {
                    Ok(Some(yuv)) => {
                        shared.decoded.fetch_add(1, Ordering::Relaxed);
                        // Navbatda yangiroq kadr turibdi: bu kadr ekranda darrov almashtiriladi. Rang o'girish (eng
                        // qimmat qadam) o'tkazib yuboriladi, dekoder navbatni tezroq bo'shatadi va kechikish kamayadi.
                        // Ekran baribir kamida `MAX_SHOW_GAP` da bir marta yangilanadi.
                        if fb.queue_len() > 0 && shown.elapsed() < MAX_SHOW_GAP {
                            continue;
                        }
                        shown = Instant::now();
                        let (w, h) = yuv.dimensions();
                        let pixels = super::yuv::to_color32((yuv.y(), yuv.u(), yuv.v()), yuv.strides(), (w, h));
                        let image = egui::ColorImage { size: [w, h], pixels };
                        *shared.picture.lock().unwrap() = Some(Picture { image });
                        ctx.request_repaint();
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("dekodlash xatosi: {e}");
                        fb.request_keyframe();
                    }
                }
            }
        });
        // Dekoder ochilmasa sessiya jimgina yopilib qolmasin: sababi foydalanuvchiga ko'rsatiladi
        if let Ok(Err(e)) = init_rx.recv() {
            anyhow::bail!("H.264 dekoder yaratilmadi: {e}");
        }
    }

    // `ever`: sessiya kamida bir marta to'liq (video bilan) ochilgan; faqat shundan keyin qayta uriniladi.
    let mut ever = false;
    let mut deadline: Option<Instant> = None;
    let mut attempt = 0u32;
    loop {
        let mut up = false;
        let res = run_once(&p, &ctx, &shared, &ev_tx, &ftx, &fb, &mut cmd_rx, &mut input_rx, &mut up).await;
        if up {
            ever = true;
            deadline = None;
            attempt = 0;
        }
        let end = match res {
            Ok(end) => end,
            // Kalit rad etildi / Host videoni rad etdi: qayta urinish befoyda.
            Err(e) if e.downcast_ref::<Rejected>().is_some() || e.downcast_ref::<VideoRefused>().is_some() => return Err(e),
            Err(e) if ever => End::Lost(format!("{e:#}")),
            Err(e) => return Err(e),
        };
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
            return Ok(Some(format!("{reason} (qayta ulanib bo'lmadi)")));
        }
        attempt += 1;
        let _ = ev_tx.send(Event::Reconnecting { attempt, reason });
        ctx.request_repaint();
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
    ctx: &egui::Context,
    shared: &Arc<Shared>,
    ev_tx: &mpsc::Sender<Event>,
    ftx: &mpsc::SyncSender<Vec<u8>>,
    fb: &Arc<ViewerFeedback>,
    cmd_rx: &mut UnboundedReceiver<Cmd>,
    input_rx: &mut UnboundedReceiver<InputEvent>,
    up: &mut bool,
) -> Result<End> {
    let mut session = match &p.relay {
        Some((id, connector)) => {
            // Server qabul qiluvchidan ruxsat so'raydi (u 30 soniya o'ylaydi). Parol kiritilmagan bo'lsa
            // qarovsiz kompyuter ham ruxsat oynasini ko'rsatadi va bir martalik parol beradi.
            let asked = connector.connect_ask(id, p.password.is_empty());
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
            connect_relayed(ConnectOptions { addr: relayed.local, password: &password }).await?
        }
        None => {
            let addr = resolve(&p.addr).await?;
            connect_with(ConnectOptions { addr, password: &p.password }, p.transport).await?
        }
    };
    let _ = ev_tx.send(Event::Connected { via_tcp: session.via_tcp });
    ctx.request_repaint();

    // Host ekranni ola olmasa (ruxsat yo'q yoki noto'g'ri berilgan) ulanish uzilmaydi: foydalanuvchi
    // ruxsatni bekor qilib qaytadan berishi (yoki bekor qilishi) mumkin.
    let mut stream = loop {
        match session.request_video(p.fps, p.kbps, p.height, 0).await {
            Ok(s) => break s,
            Err(e) if e.downcast_ref::<VideoRefused>().map_or(false, |v| v.0 == video_reason::NO_CAPTURE) => {
                let _ = ev_tx.send(Event::VideoRefused(format!("{e:#}")));
                ctx.request_repaint();
                loop {
                    match cmd_rx.recv().await {
                        Some(Cmd::ResetPermissions) => {
                            session.reset_permissions().await?;
                            break;
                        }
                        Some(Cmd::ListFiles | Cmd::SendFile(..) | Cmd::GetFile(..)) => {}
                        Some(Cmd::Close) | None => {
                            session.close().await;
                            return Ok(End::Closed);
                        }
                    }
                }
            }
            Err(e) => return Err(e),
        }
    };
    let _ = ev_tx.send(Event::VideoStarted);
    ctx.request_repaint();
    *up = true;

    // Yangi oqim: eski holat tozalanadi, video kutilayotganda bosilgan tugmalar Host'ga yuborilmaydi.
    fb.reset();
    while input_rx.try_recv().is_ok() {}

    // Kadrlarni qabul qilish. next_frame() ni timeout/select bilan uzib bo'lmaydi (oqim buziladi),
    // shuning uchun u alohida vazifada ishlaydi va kerak bo'lsa butunlay abort qilinadi.
    let rx_shared = shared.clone();
    let rx_fb = fb.clone();
    let ftx = ftx.clone();
    let mut recv_task = tokio::spawn(async move {
        loop {
            match stream.next_frame().await {
                Ok(Some(f)) => {
                    rx_shared.frames.fetch_add(1, Ordering::Relaxed);
                    rx_shared.bytes.fetch_add(f.data.len() as u64, Ordering::Relaxed);
                    rx_fb.frame_received(f.pts_us);
                    if !rx_fb.accept(f.keyframe) {
                        continue;
                    }
                    // Navbatga qo'yishdan oldin sanaymiz: dekoder `dequeued()` ni tezroq chaqirsa ham hisob buzilmaydi
                    rx_fb.queued();
                    match ftx.try_send(f.data) {
                        Ok(()) => {}
                        // Dekoder ulgurmayapti: zanjir uzildi, kalit kadr kerak
                        Err(TrySendError::Full(_)) => {
                            rx_fb.dequeued();
                            rx_fb.request_keyframe();
                        }
                        Err(TrySendError::Disconnected(_)) => return None,
                    }
                }
                Ok(None) => return Some("Host videoni to'xtatdi".to_string()),
                Err(e) => return Some(format!("video: {e:#}")),
            }
        }
    });

    // Feedback va kalit kadr so'rovlari shu taktda yuboriladi; RTT QUIC'dan olinadi (alohida ping yo'q,
    // shuning uchun kiritish hech qachon javob kutib qolmaydi). Uzilishni QUIC idle timeout aniqlaydi.
    let conn = session.connection();
    let closed = conn.closed();
    tokio::pin!(closed);
    let mut tick = tokio::time::interval(FEEDBACK_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut ticks: u32 = 0;
    let end = loop {
        tokio::select! {
            ev = input_rx.recv() => {
                let Some(ev) = ev else { break End::Closed };
                let mut batch = vec![ev];
                while let Ok(ev) = input_rx.try_recv() {
                    batch.push(ev);
                }
                if let Err(e) = session.send_inputs(batch).await {
                    break End::Lost(format!("{e:#}"));
                }
            }
            c = cmd_rx.recv() => match c {
                Some(Cmd::ResetPermissions) => {
                    if let Err(e) = session.reset_permissions().await {
                        break End::Lost(format!("{e:#}"));
                    }
                }
                Some(c @ (Cmd::ListFiles | Cmd::SendFile(..) | Cmd::GetFile(..))) => {
                    spawn_file_cmd(session.connection(), ctx, ev_tx, c);
                }
                Some(Cmd::Close) | None => break End::Closed,
            },
            _ = tick.tick() => {
                if let Some((pts, queue)) = fb.take_feedback() {
                    if let Err(e) = session.send_feedback(pts, queue).await {
                        break End::Lost(format!("{e:#}"));
                    }
                }
                if fb.take_keyframe_request() {
                    if let Err(e) = session.request_keyframe().await {
                        break End::Lost(format!("{e:#}"));
                    }
                }
                ticks = ticks.wrapping_add(1);
                shared.queue.store(fb.queue_len().max(0) as u64, Ordering::Relaxed);
                shared.need_key.store(fb.needs_key(), Ordering::Relaxed);
                if ticks % 20 == 0 {
                    shared.rtt_us.store(session.rtt().as_micros() as u64, Ordering::Relaxed);
                }
            }
            e = &mut closed => break End::Lost(format!("aloqa uzildi: {e}")),
            r = &mut recv_task => break match r {
                Ok(None) => End::Closed,
                // Host videoni o'zi to'xtatdi: qayta ulanish befoyda
                Ok(Some(m)) if m.starts_with("Host videoni") => End::Fatal(m),
                Ok(Some(m)) => End::Lost(m),
                Err(_) => End::Lost("video vazifasi to'xtadi".to_string()),
            },
        }
    };
    recv_task.abort();
    // Host ulanishni ataylab yopgan bo'lsa (masalan "Uzish"), qayta ulanmaymiz.
    let end = match end {
        End::Lost(m) if session.closed_by_peer() => End::Fatal(format!("{m} (Host ulanishni yopdi)")),
        e => e,
    };
    session.close().await;
    Ok(end)
}
