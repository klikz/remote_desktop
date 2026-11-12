//! Bitta ulanish: relay orqali ulanish, video kadrlarni qabul qilish va kiritishni yuborish.
//! `crates/viewer/src/gui/session.rs` mantig'i, lekin egui/openh264 siz.
//!
//! Aloqa uzilsa (tarmoq o'zgardi, vaqtincha internet yo'q, Host qayta ishga tushdi) sessiya darrov tugamaydi:
//! `RECONNECT_WINDOW` ichida avtomatik qayta ulanishga urinadi (`Event::Reconnecting`). Doimiy xatolar
//! (kalit noto'g'ri, Host ulanishni uzdi, foydalanuvchi yopdi) darrov tugatadi.

use anyhow::Result;
use protocol::{InputEvent, DEFAULT_PORT};
use rdcore::{
    client::{connect_relayed, connect_with, ConnectOptions, Rejected, Transport, VideoRefused},
    pacing::{ViewerFeedback, FEEDBACK_INTERVAL},
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
    Connected { via_tcp: bool },
    VideoStarted,
    /// Aloqa uzildi, avtomatik qayta ulanilmoqda (`attempt` 1 dan boshlanadi; `reason` = uzilish sababi).
    Reconnecting { attempt: u32, reason: String },
    /// Ulanish tugadi. `Some` = xato matni.
    Ended(Option<String>),
}

enum Cmd {
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
    /// Kiritish alohida kanalda: tarmoq sikli navbatdagilarni birdaniga olib, bitta yozuv bilan yuboradi.
    input: UnboundedSender<InputEvent>,
    events: Mutex<mpsc::Receiver<Event>>,
    frames: Mutex<mpsc::Receiver<Frame>>,
    /// `wait_frame` olgan, lekin hali dekoder buferiga ko'chirilmagan kadr.
    pending: Mutex<Option<Frame>>,
    fb: Arc<ViewerFeedback>,
    pub stats: Arc<Stats>,
}

impl Sess {
    pub fn input(&self, ev: InputEvent) {
        let _ = self.input.send(ev);
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

    /// Kadr kutadi (bloklaydi, eng ko'pi bilan `timeout`). Kadr `copy_frame` yoki `discard_frame` gacha saqlanadi.
    /// Natija: (baytlar soni, kalit kadrmi).
    pub fn wait_frame(&self, timeout: Duration) -> Option<(usize, bool)> {
        let f = self.frames.lock().ok()?.recv_timeout(timeout).ok()?;
        self.fb.dequeued();
        let meta = (f.data.len(), f.keyframe);
        if let Ok(mut p) = self.pending.lock() {
            *p = Some(f);
        }
        Some(meta)
    }

    /// Saqlangan kadrni `dst` ga (dekoderning kirish buferiga) ko'chiradi. Sig'masa kadr tashlanadi, kalit kadr
    /// so'raladi va `None` qaytadi.
    pub fn copy_frame(&self, dst: &mut [u8]) -> Option<usize> {
        let f = self.pending.lock().ok()?.take()?;
        if f.data.len() > dst.len() {
            self.fb.request_keyframe();
            return None;
        }
        dst[..f.data.len()].copy_from_slice(&f.data);
        Some(f.data.len())
    }

    /// Saqlangan kadrni tashlaydi. `chain_broken`: dekoder kadrni ololmadi (zanjir uzildi), kalit kadr so'raladi;
    /// `false`: dekoder baribir kalit kadrni kutayotgan edi.
    pub fn discard_frame(&self, chain_broken: bool) {
        if let Ok(mut p) = self.pending.lock() {
            p.take();
        }
        if chain_broken {
            self.fb.request_keyframe();
        }
    }

    /// Dekoder xatosi yoki qayta yaratildi: keyingi kalit kadrgacha kadrlar tashlanadi va Host'dan so'raladi.
    pub fn request_keyframe(&self) {
        self.fb.request_keyframe();
    }
}

impl Drop for Sess {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Close);
    }
}

pub fn start(p: Params) -> Arc<Sess> {
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (input_tx, input_rx) = unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    let (frame_tx, frame_rx) = mpsc::sync_channel::<Frame>(8);
    let stats = Arc::new(Stats::default());
    let fb = Arc::new(ViewerFeedback::default());
    let sess = Arc::new(Sess {
        cmd: cmd_tx,
        input: input_tx,
        events: Mutex::new(ev_rx),
        frames: Mutex::new(frame_rx),
        pending: Mutex::new(None),
        fb: fb.clone(),
        stats: stats.clone(),
    });
    std::thread::spawn(move || {
        let ended = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(rt) => rt.block_on(run(p, stats, ev_tx.clone(), frame_tx, cmd_rx, input_rx, fb)),
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
    frame_tx: mpsc::SyncSender<Frame>,
    mut cmd_rx: UnboundedReceiver<Cmd>,
    mut input_rx: UnboundedReceiver<InputEvent>,
    fb: Arc<ViewerFeedback>,
) -> Result<Option<String>> {
    // `ever`: sessiya kamida bir marta to'liq (video bilan) ochilgan; faqat shundan keyin qayta uriniladi.
    let mut ever = false;
    let mut deadline: Option<Instant> = None;
    let mut attempt = 0u32;
    loop {
        let mut up = false;
        let end = match run_once(&p, &stats, &ev_tx, &frame_tx, &mut cmd_rx, &mut input_rx, &fb, &mut up)
            .await
        {
            Ok(end) => end,
            // Kalit rad etildi / Host videoni rad etdi: qayta urinish befoyda.
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
    frame_tx: &mpsc::SyncSender<Frame>,
    cmd_rx: &mut UnboundedReceiver<Cmd>,
    input_rx: &mut UnboundedReceiver<InputEvent>,
    fb: &Arc<ViewerFeedback>,
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
            connect_relayed(ConnectOptions { addr: relayed.local, password: &password }).await?
        }
        None => {
            if p.password.is_empty() {
                return Ok(End::Fatal("kalitni kiriting".into()));
            }
            let addr = resolve(&p.addr).await?;
            connect_with(ConnectOptions { addr, password: &p.password }, Transport::Auto).await?
        }
    };
    let _ = ev_tx.send(Event::Connected { via_tcp: session.via_tcp });

    let mut stream = session.request_video(p.fps, p.kbps, p.height, 0).await?;
    let _ = ev_tx.send(Event::VideoStarted);
    *up = true;
    // Yangi oqim: eski holat tozalanadi, uzilish paytida bosilgan tugmalar Host'ga yuborilmaydi.
    fb.reset();
    while input_rx.try_recv().is_ok() {}

    // next_frame() ni timeout/select bilan uzib bo'lmaydi (oqim buziladi): alohida vazifa, kerak bo'lsa abort.
    let rx_stats = stats.clone();
    let rx_fb = fb.clone();
    let frame_tx = frame_tx.clone();
    let mut recv_task = tokio::spawn(async move {
        loop {
            match stream.next_frame().await {
                Ok(Some(f)) => {
                    rx_stats.frames.fetch_add(1, Ordering::Relaxed);
                    rx_stats.bytes.fetch_add(f.data.len() as u64, Ordering::Relaxed);
                    rx_fb.frame_received(f.pts_us);
                    if !rx_fb.accept(f.keyframe) {
                        continue;
                    }
                    // Navbatga qo'yishdan oldin sanaymiz: dekoder `dequeued()` ni tezroq chaqirsa hisob +1 ga
                    // siljib qolardi (4 marta siljisa Host kodlashni butunlay to'xtatadi: video qotadi).
                    rx_fb.queued();
                    match frame_tx.try_send(Frame { keyframe: f.keyframe, data: f.data }) {
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

    // Feedback, kalit kadr so'rovi va RTT shu taktda; alohida ping yo'q (kiritish javob kutib qolmaydi).
    // Uzilishni QUIC idle timeout aniqlaydi.
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
                Some(Cmd::NetworkChanged) => break End::Lost("tarmoq o'zgardi".into()),
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
                if ticks % 20 == 0 {
                    stats.rtt_us.store(session.rtt().as_micros() as u64, Ordering::Relaxed);
                }
            }
            e = &mut closed => break End::Lost(format!("aloqa uzildi: {e}")),
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
