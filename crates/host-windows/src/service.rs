//! Windows Host: CLI, dastur ichidan boshqariladigan `RunningHost` va sessiya mantiqi
//! (Linux Host bilan bir xil protokol).

use crate::{input, video};
use anyhow::{anyhow, bail, Context, Result};
use protocol::{video_reason, Capabilities, Message, VideoHeader, CODEC_H264, DEFAULT_PORT};
use rdcore::{
    auth::Credentials,
    cert::HostIdentity,
    framing::{read_message, write_message},
    hexfmt, paths,
    server::{Handler, Host, SessionIo},
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{sync::mpsc, task::JoinHandle};
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};

const USAGE: &str = "\
host-windows: Remote Desktop Host (Windows)

Buyruqlar:
  host-windows set-password            Ulanish parolini o'rnatish (kamida 8 belgi)
  host-windows run [--bind IP] [--port PORT]
                                       Host'ni ishga tushirish
                                       (standart: --bind 0.0.0.0, port: UI'da saqlangan yoki 7777)

Birinchi ishga tushirishda Windows xavfsizlik devori (Firewall) so'raydi: \"Ruxsat berish\" ni bosing.
Dasturning oynali versiyasida (`viewer`) \"Bu kompyuter\" bo'limidan ham yoqish mumkin.
";

fn dpi_aware() {
    // Koordinatalar va o'lchamlar fizik pikselda bo'lishi uchun (ekran masshtabi 125% va h.k.).
    // Jarayonda allaqachon o'rnatilgan bo'lsa, bu chaqiruv e'tiborsiz qoladi.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub async fn run_cli() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("set-password") => set_password(),
        Some("run") => run_host(&args[1..]).await,
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    }
}

fn set_password() -> Result<()> {
    let p1 = rpassword::prompt_password("Yangi parol (kamida 8 belgi): ")?;
    let p2 = rpassword::prompt_password("Parolni takrorlang: ")?;
    if p1 != p2 {
        bail!("parollar mos kelmadi");
    }
    let creds = Credentials::from_password(&p1)?;
    let path = paths::host_password_file()?;
    creds.save(&path)?;
    println!("Parol saqlandi: {}", path.display());
    Ok(())
}

async fn run_host(args: &[String]) -> Result<()> {
    dpi_aware();
    let mut ip: IpAddr = "0.0.0.0".parse()?;
    let mut port: u16 = paths::read_host_port().unwrap_or(DEFAULT_PORT);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--bind" => ip = it.next().context("--bind uchun qiymat kerak")?.parse().context("noto'g'ri IP")?,
            "--port" => port = it.next().context("--port uchun qiymat kerak")?.parse().context("noto'g'ri port")?,
            other => bail!("noma'lum argument: {other}\n\n{USAGE}"),
        }
    }
    let host = bind_host(ip, port, None)?;
    let _tcp = host.spawn_tcp_tunnel();
    println!("Host ishga tushdi: {} (UDP)", host.local_addr()?);
    println!("Fingerprint: {}", hexfmt::colon(&host.fingerprint()));
    println!("Viewer birinchi ulanishda shu fingerprint'ni tasdiqlaydi. To'xtatish: Ctrl+C");
    host.run_with(make_handler(Arc::new(Shared::default()))).await
}

/// Sertifikat va parolni o'qib, UDP portni ochadi. Tokio runtime ichida chaqirilishi kerak.
fn bind_host(ip: IpAddr, port: u16, creds: Option<Credentials>) -> Result<Host> {
    let dir = paths::config_dir()?;
    let identity = HostIdentity::load_or_create(&dir)?;
    let creds = match creds {
        Some(c) => c,
        None => Credentials::load(&paths::host_password_file()?)
            .context("parol o'rnatilmagan: \"Bu kompyuter\" bo'limida yoki `host-windows set-password` bilan o'rnating")?,
    };
    let caps = Capabilities { codecs: CODEC_H264, max_fps: 60, monitors: vec![] };
    Host::bind(SocketAddr::new(ip, port), &identity, creds, caps)
}

// ---------------------------------------------------------------------------------------------
// Dastur ichidan boshqarish
// ---------------------------------------------------------------------------------------------

struct Peer {
    id: u64,
    ip: IpAddr,
    via_tcp: bool,
    since: Instant,
    conn: rdcore::quinn::Connection,
}

#[derive(Default)]
struct Shared {
    peers: Mutex<Vec<Peer>>,
    next_id: AtomicU64,
}

/// Hozir ulangan Viewer haqida ma'lumot.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub id: u64,
    pub ip: IpAddr,
    /// TCP tunnel orqali ulangan.
    pub via_tcp: bool,
    pub since: Instant,
}

/// Dastur ichida ishlayotgan Host. `stop()` yoki `drop` uni to'xtatadi va portni bo'shatadi.
pub struct RunningHost {
    pub port: u16,
    pub fingerprint: String,
    shared: Arc<Shared>,
    stop_tx: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl RunningHost {
    pub fn start(port: u16) -> Result<RunningHost> {
        Self::start_with("0.0.0.0".parse().unwrap(), port, None)
    }

    /// Qarovsiz (ID orqali) ulanish uchun: faqat loopback'da, bo'sh portda, alohida parol bilan.
    pub fn start_dedicated(creds: Credentials) -> Result<RunningHost> {
        Self::start_with("127.0.0.1".parse().unwrap(), 0, Some(creds))
    }

    fn start_with(ip: IpAddr, port: u16, creds: Option<Credentials>) -> Result<RunningHost> {
        dpi_aware();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<std::result::Result<(String, u16), String>>();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let shared = Arc::new(Shared::default());
        let sh = shared.clone();
        let thread = std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("tokio: {e}")));
                    return;
                }
            };
            rt.block_on(async {
                let host = match bind_host(ip, port, creds) {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = ready_tx.send(Err(format!("{e:#}")));
                        return;
                    }
                };
                let _tcp = host.spawn_tcp_tunnel();
                let bound = host.local_addr().map(|a| a.port()).unwrap_or(port);
                let _ = ready_tx.send(Ok((hexfmt::colon(&host.fingerprint()), bound)));
                tokio::select! {
                    r = host.run_with(make_handler(sh.clone())) => {
                        if let Err(e) = r {
                            eprintln!("[host] to'xtadi: {e:#}");
                        }
                    }
                    _ = stop_rx => {}
                }
                // Ulanganlarga yopilish sababini aytib, ularni tartib bilan uzamiz
                for p in sh.peers.lock().unwrap().iter() {
                    p.conn.close(0u32.into(), b"host stopped");
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            });
            rt.shutdown_timeout(Duration::from_secs(2));
        });
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok((fingerprint, port))) => Ok(RunningHost { port, fingerprint, shared, stop_tx: Some(stop_tx), thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(anyhow!(e))
            }
            Err(_) => Err(anyhow!("Host ishga tushmadi (vaqt tugadi)")),
        }
    }

    pub fn peers(&self) -> Vec<PeerInfo> {
        self.shared.peers.lock().unwrap().iter().map(|p| PeerInfo { id: p.id, ip: p.ip, via_tcp: p.via_tcp, since: p.since }).collect()
    }

    /// Ulangan Viewer'ni uzadi.
    pub fn kick(&self, id: u64) {
        if let Some(p) = self.shared.peers.lock().unwrap().iter().find(|p| p.id == id) {
            p.conn.close(0u32.into(), b"kicked");
        }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for RunningHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn make_handler(shared: Arc<Shared>) -> Handler {
    Arc::new(move |io| {
        let sh = shared.clone();
        Box::pin(session(io, sh))
    })
}

struct Video {
    task: JoinHandle<()>,
    stop: Arc<AtomicBool>,
}

impl Video {
    fn running(&self) -> bool {
        !self.task.is_finished()
    }
    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.task.abort();
    }
}

/// Bitta Viewer sessiyasi: control xabarlari, video va kiritish.
async fn session(mut io: SessionIo, shared: Arc<Shared>) {
    let id = shared.next_id.fetch_add(1, Ordering::Relaxed);
    let (real_ip, via_tcp) = rdcore::tcp_tunnel::resolve(io.peer);
    shared.peers.lock().unwrap().push(Peer { id, ip: real_ip, via_tcp, since: Instant::now(), conn: io.conn.clone() });
    println!("[host] {real_ip}: ulandi{}", if via_tcp { " (TCP)" } else { "" });
    let mut video: Option<Video> = None;
    loop {
        match read_message(&mut io.recv).await {
            Ok(Message::Ping { id, sent_ms }) => {
                if write_message(&mut io.send, &Message::Pong { id, sent_ms }).await.is_err() {
                    break;
                }
            }
            Ok(Message::VideoRequest { fps, kbps, max_height, .. }) => {
                let reply = if video.as_ref().map_or(false, Video::running) {
                    Message::VideoFailed { reason: video_reason::BUSY }
                } else {
                    let (fps, kbps) = (fps.clamp(5, 60), kbps.clamp(500, 50_000));
                    video = Some(start_video(&io, fps, kbps, max_height));
                    println!("[host] video boshlandi: {fps} FPS, {kbps} kbit/s, balandlik {max_height}");
                    Message::VideoStarted { codec: CODEC_H264, fps, kbps }
                };
                if write_message(&mut io.send, &reply).await.is_err() {
                    break;
                }
            }
            Ok(Message::Input(ev)) => input::apply(ev),
            Ok(Message::Bye { .. }) | Err(_) => break,
            Ok(_) => {}
        }
    }
    if let Some(v) = video {
        v.stop();
    }
    shared.peers.lock().unwrap().retain(|p| p.id != id);
    println!("[host] {real_ip}: uzildi");
    io.conn.close(0u32.into(), b"bye");
}

fn start_video(io: &SessionIo, fps: u16, kbps: u32, max_height: u16) -> Video {
    let stop = Arc::new(AtomicBool::new(false));
    let conn = io.conn.clone();
    let stop2 = stop.clone();
    let task = tokio::spawn(async move {
        let (tx, mut rx) = mpsc::channel::<video::EncodedFrame>(8);
        let (etx, mut erx) = mpsc::unbounded_channel::<String>();
        let thread = video::spawn(u32::from(fps), kbps, u32::from(max_height), tx, stop2.clone(), etx);

        let mut stream = match conn.open_uni().await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[host] video oqimini ochib bo'lmadi: {e}");
                stop2.store(true, Ordering::Relaxed);
                return;
            }
        };
        let mut sent: u64 = 0;
        loop {
            tokio::select! {
                frame = rx.recv() => {
                    let Some(f) = frame else { break };
                    let header = VideoHeader { keyframe: f.keyframe, pts_us: f.pts_us, len: f.data.len() as u32 }.encode();
                    if stream.write_all(&header).await.is_err() || stream.write_all(&f.data).await.is_err() {
                        break; // Viewer uzildi
                    }
                    sent += 1;
                }
                err = erx.recv() => {
                    if let Some(m) = err {
                        eprintln!("[host] ekran/kodlash xatosi: {m}");
                    }
                    break;
                }
            }
        }
        stop2.store(true, Ordering::Relaxed);
        let _ = stream.finish();
        let _ = thread.join();
        println!("[host] video to'xtadi ({sent} kadr yuborildi)");
    });
    Video { task, stop }
}
