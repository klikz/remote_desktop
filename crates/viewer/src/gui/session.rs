//! Bitta ulanish: tarmoq (alohida oqim va tokio runtime), H.264 dekodlash va UI bilan aloqa.

use anyhow::{Context, Result};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use protocol::{video_reason, InputEvent, DEFAULT_PORT};
use rdcore::{
    client::{connect_relayed, connect_with, ConnectOptions, Transport, VideoRefused},
    filexfer,
    relay::agent::Connector,
    known_hosts::KnownHosts,
    paths,
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

/// Dekodlangan kadr (RGB, 3 bayt/piksel).
pub struct Picture {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
}

/// Tarmoq oqimi va UI o'rtasida bo'lishiladigan holat.
#[derive(Default)]
pub struct Shared {
    pub picture: Mutex<Option<Picture>>,
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    pub rtt_us: AtomicU64,
}

pub enum Event {
    /// Birinchi ulanish: fingerprint'ni foydalanuvchi tasdiqlashi kerak.
    Confirm(String, mpsc::Sender<bool>),
    Connected { fingerprint: String, via_tcp: bool },
    VideoStarted,
    /// Host ekranni ola olmadi (xato matni). Ulanish ochiq: ruxsatni qaytadan berish mumkin.
    VideoRefused(String),
    /// Ulanish tugadi. `Some` = xato matni.
    Ended(Option<String>),
    /// Host'dagi almashinuv papkasi ro'yxati.
    Files(Result<Vec<RemoteFile>, String>),
    /// Fayl uzatish holati. `result`: `None` = davom etmoqda, `Some` = tugadi (`Ok` = saqlangan yo'l / xabar).
    Transfer { id: u64, done: u64, total: u64, result: Option<Result<String, String>> },
}

enum Cmd {
    Input(InputEvent),
    ResetPermissions,
    ListFiles,
    SendFile(u64, PathBuf),
    GetFile(u64, String),
    Close,
}

pub struct Handle {
    cmd: UnboundedSender<Cmd>,
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
        let _ = self.cmd.send(Cmd::Input(ev));
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
    let (ev_tx, ev_rx) = mpsc::channel();
    let shared = Arc::new(Shared::default());
    let sh = shared.clone();
    std::thread::spawn(move || {
        let ended = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(rt) => rt.block_on(run(p, ctx.clone(), sh, ev_tx.clone(), cmd_rx)),
            Err(e) => Err(e.into()),
        };
        let msg = match ended {
            Ok(m) => m,
            Err(e) => Some(format!("{e:#}")),
        };
        let _ = ev_tx.send(Event::Ended(msg));
        ctx.request_repaint();
    });
    Handle { cmd: cmd_tx, events: ev_rx, shared, next_transfer: AtomicU64::new(1) }
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

async fn run(
    p: Params,
    ctx: egui::Context,
    shared: Arc<Shared>,
    ev_tx: mpsc::Sender<Event>,
    mut cmd_rx: UnboundedReceiver<Cmd>,
) -> Result<Option<String>> {
    let known_hosts = KnownHosts::new(paths::known_hosts_file()?);

    // Fingerprint tasdig'i: UI'ga so'rov yuboriladi va javob kutiladi (bu oqim faqat shu ulanishniki).
    let confirm_tx = ev_tx.clone();
    let confirm_ctx = ctx.clone();
    let confirm = move |fp: &str| -> bool {
        let (tx, rx) = mpsc::channel();
        if confirm_tx.send(Event::Confirm(fp.to_string(), tx)).is_err() {
            return false;
        }
        confirm_ctx.request_repaint();
        rx.recv().unwrap_or(false)
    };

    let mut session = match &p.relay {
        Some((id, connector)) => {
            // Server qabul qiluvchidan ruxsat so'raydi (u 30 soniya o'ylaydi). Parol kiritilmagan bo'lsa
            // qarovsiz kompyuter ham ruxsat oynasini ko'rsatadi va bir martalik parol beradi.
            let relayed = connector
                .connect_ask(id, p.password.is_empty())
                .await
                .map_err(|_| anyhow::anyhow!("relay agenti to'xtagan"))?
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let password = relayed.pw.clone().unwrap_or_else(|| p.password.clone());
            if password.is_empty() {
                anyhow::bail!("Host kalit bermadi: kalitni kiriting");
            }
            connect_relayed(
                ConnectOptions { addr: relayed.local, password: &password, known_hosts: &known_hosts, confirm_new_host: &confirm },
                format!("id:{id}"),
            )
            .await?
        }
        None => {
            let addr = resolve(&p.addr).await?;
            connect_with(
                ConnectOptions { addr, password: &p.password, known_hosts: &known_hosts, confirm_new_host: &confirm },
                p.transport,
            )
            .await?
        }
    };
    let _ = ev_tx.send(Event::Connected { fingerprint: session.fingerprint.clone(), via_tcp: session.via_tcp });
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
                        Some(Cmd::Input(_) | Cmd::ListFiles | Cmd::SendFile(..) | Cmd::GetFile(..)) => {}
                        Some(Cmd::Close) | None => {
                            session.close().await;
                            return Ok(None);
                        }
                    }
                }
            }
            Err(e) => return Err(e),
        }
    };
    let _ = ev_tx.send(Event::VideoStarted);
    ctx.request_repaint();

    // Dekoder oqimi: eng yangi kadr `shared.picture` ga yoziladi.
    let (ftx, frx) = mpsc::sync_channel::<Vec<u8>>(8);
    {
        let shared = shared.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut decoder = match Decoder::new() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("dekoder yaratilmadi: {e}");
                    return;
                }
            };
            while let Ok(data) = frx.recv() {
                match decoder.decode(&data) {
                    Ok(Some(yuv)) => {
                        let (w, h) = yuv.dimensions();
                        let mut rgb = vec![0u8; w * h * 3];
                        yuv.write_rgb8(&mut rgb);
                        *shared.picture.lock().unwrap() = Some(Picture { w, h, rgb });
                        ctx.request_repaint();
                    }
                    Ok(None) => {}
                    Err(e) => eprintln!("dekodlash xatosi: {e}"),
                }
            }
        });
    }

    // Kadrlarni qabul qilish. next_frame() ni timeout/select bilan uzib bo'lmaydi (oqim buziladi),
    // shuning uchun u alohida vazifada ishlaydi va kerak bo'lsa butunlay abort qilinadi.
    let rx_shared = shared.clone();
    let mut recv_task = tokio::spawn(async move {
        let mut need_key = false;
        loop {
            match stream.next_frame().await {
                Ok(Some(f)) => {
                    rx_shared.frames.fetch_add(1, Ordering::Relaxed);
                    rx_shared.bytes.fetch_add(f.data.len() as u64, Ordering::Relaxed);
                    if need_key && !f.keyframe {
                        continue;
                    }
                    match ftx.try_send(f.data) {
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
                        break Some(format!("{e:#}"));
                    }
                }
                Some(Cmd::ResetPermissions) => {
                    if let Err(e) = session.reset_permissions().await {
                        break Some(format!("{e:#}"));
                    }
                }
                Some(c @ (Cmd::ListFiles | Cmd::SendFile(..) | Cmd::GetFile(..))) => {
                    spawn_file_cmd(session.connection(), &ctx, &ev_tx, c);
                }
                Some(Cmd::Close) | None => break None,
            },
            _ = ping.tick() => match session.ping().await {
                Ok(d) => shared.rtt_us.store(d.as_micros() as u64, Ordering::Relaxed),
                Err(e) => break Some(format!("aloqa uzildi: {e:#}")),
            },
            r = &mut recv_task => break r.unwrap_or_else(|_| Some("video vazifasi to'xtadi".to_string())),
        }
    };
    recv_task.abort();
    session.close().await;
    Ok(end)
}
