//! Client agenti: relay serveriga doimiy ulanadi (dynamic IP va NAT muammo emas), ID oladi, boshqa
//! clientlardan kelgan so'rovlarni qabul qiladi yoki rad etadi va o'zi boshqa ID ga ulanish uchun
//! ma'lumot oqimini ochadi. Alohida OS oqimida, o'z tokio runtime'ida ishlaydi.

use anyhow::{anyhow, bail, Context, Result};
use protocol::{DEFAULT_PORT, SERVER_NAME};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{mpsc as std_mpsc, Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::UdpSocket,
    sync::{mpsc, oneshot},
    time::{sleep, timeout},
};

use super::{
    msg::{read_msg, write_msg, RMsg},
    ALPN, MAX_BIDI,
};
use crate::{
    auth::{self, Credentials},
    cert::sha256,
    hexfmt,
    known_hosts::{KnownHosts, Trust},
    onetime, paths, tcp_tunnel, tls,
};

const STEP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct AgentConfig {
    /// `host:port`
    pub server: String,
    /// Serverga kirish kaliti (server egasi beradi).
    pub key: String,
    pub name: String,
    /// Oldingi ro'yxatdan o'tishda berilgan ID va token (bo'sh: yangi ID olinadi).
    pub id: String,
    pub token: String,
    pub known_hosts: PathBuf,
    /// Mahalliy Host'ning UDP porti (`None`: saqlangan port yoki standart).
    pub host_port: Option<u16>,
}

/// Kiruvchi so'rovlarga munosabat (UI o'zgartirib turadi).
#[derive(Default)]
pub struct Policy {
    /// Qarovsiz: dialogsiz qabul qilinadi, Viewer doimiy parolni kiritadi.
    pub unattended: bool,
    /// Ishonchli ID'lar: dialogsiz, bir martalik parol bilan qabul qilinadi.
    pub trusted: HashSet<String>,
}

#[derive(Debug)]
pub enum AgentEvent {
    Status { online: bool, text: String },
    Registered { id: String, token: String },
    Presence { id: String, online: bool, name: String },
    /// Dialog kerak: `Agent::reply` bilan javob beriladi.
    Incoming { sid: String, from: String, name: String },
}

/// Ochilgan ma'lumot kanali: `local` ga (UDP) oddiy Host manzili sifatida ulaniladi.
#[derive(Debug)]
pub struct Relayed {
    pub local: SocketAddr,
    /// Bir martalik parol; `None` bo'lsa Host'ning doimiy paroli kerak.
    pub pw: Option<String>,
}

enum Cmd {
    Subscribe(Vec<String>),
    Reply { sid: String, accept: bool },
    Connect { target: String, tx: oneshot::Sender<Result<Relayed, String>> },
}

pub struct Agent {
    cmd: mpsc::UnboundedSender<Cmd>,
    events: std_mpsc::Receiver<AgentEvent>,
    pub policy: Arc<Mutex<Policy>>,
}

impl Agent {
    /// Agentni ishga tushiradi. `Agent` tushirilganda to'xtaydi.
    pub fn start(cfg: AgentConfig, policy: Policy) -> Agent {
        let (cmd, cmd_rx) = mpsc::unbounded_channel();
        let (ev_tx, events) = std_mpsc::channel();
        let policy = Arc::new(Mutex::new(policy));
        let p = policy.clone();
        std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() else {
                let _ = ev_tx.send(AgentEvent::Status { online: false, text: "runtime ochilmadi".into() });
                return;
            };
            rt.block_on(run(cfg, p, cmd_rx, ev_tx));
            rt.shutdown_timeout(Duration::from_secs(2));
        });
        Agent { cmd, events, policy }
    }

    pub fn try_event(&self) -> Option<AgentEvent> {
        self.events.try_recv().ok()
    }

    /// Onlayn holatini kuzatiladigan ID'lar ro'yxati (almashtiriladi).
    pub fn subscribe(&self, ids: Vec<String>) {
        let _ = self.cmd.send(Cmd::Subscribe(ids));
    }

    /// Kiruvchi so'rovga javob (dialog natijasi).
    pub fn reply(&self, sid: &str, accept: bool) {
        let _ = self.cmd.send(Cmd::Reply { sid: sid.to_string(), accept });
    }

    /// `target` ID ga ulanish so'raydi. Natija: lokal UDP manzil (`client::connect_relayed` ga beriladi).
    pub fn connect(&self, target: &str) -> oneshot::Receiver<Result<Relayed, String>> {
        self.connector().connect(target)
    }

    /// Boshqa oqimlarga (masalan, sessiya oqimiga) berish uchun nusxalanadigan tutqich.
    pub fn connector(&self) -> Connector {
        Connector(self.cmd.clone())
    }
}

#[derive(Clone)]
pub struct Connector(mpsc::UnboundedSender<Cmd>);

impl Connector {
    /// Yuborib bo'lmasa (agent to'xtagan) qaytgan `Receiver` xato beradi.
    pub fn connect(&self, target: &str) -> oneshot::Receiver<Result<Relayed, String>> {
        let (tx, rx) = oneshot::channel();
        let _ = self.0.send(Cmd::Connect { target: target.to_string(), tx });
        rx
    }
}

enum Exit {
    Stopped,
    Disconnected,
}

async fn run(
    cfg: AgentConfig,
    policy: Arc<Mutex<Policy>>,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
    ev: std_mpsc::Sender<AgentEvent>,
) {
    let mut subs: Vec<String> = vec![];
    let mut cfg = cfg;
    let mut backoff = 2u64;
    loop {
        let _ = ev.send(AgentEvent::Status { online: false, text: format!("{} ga ulanilmoqda...", cfg.server) });
        let started = std::time::Instant::now();
        let why = match session(&mut cfg, &policy, &mut cmds, &ev, &mut subs).await {
            Ok(Exit::Stopped) => return,
            Ok(Exit::Disconnected) => "server bilan aloqa uzildi".to_string(),
            Err(e) => format!("{e:#}"),
        };
        if started.elapsed() > Duration::from_secs(30) {
            backoff = 2;
        }
        let _ = ev.send(AgentEvent::Status { online: false, text: format!("{why}. {backoff} s dan keyin qayta uriniladi") });
        let wait = sleep(Duration::from_secs(backoff));
        tokio::pin!(wait);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                c = cmds.recv() => match c {
                    None => return,
                    Some(Cmd::Connect { tx, .. }) => {
                        let _ = tx.send(Err("relay server bilan aloqa yo'q".into()));
                    }
                    Some(Cmd::Subscribe(v)) => subs = v,
                    Some(Cmd::Reply { .. }) => {}
                },
            }
        }
        backoff = (backoff * 2).min(20);
    }
}

async fn session(
    cfg: &mut AgentConfig,
    policy: &Arc<Mutex<Policy>>,
    cmds: &mut mpsc::UnboundedReceiver<Cmd>,
    ev: &std_mpsc::Sender<AgentEvent>,
    subs: &mut Vec<String>,
) -> Result<Exit> {
    let addr = tokio::net::lookup_host(cfg.server.as_str())
        .await
        .with_context(|| format!("{} manzili topilmadi", cfg.server))?
        .next()
        .ok_or_else(|| anyhow!("{} manzili topilmadi", cfg.server))?;
    let bind: SocketAddr = if addr.is_ipv6() { "[::]:0".parse()? } else { "0.0.0.0:0".parse()? };
    let endpoint = quinn::Endpoint::client(bind).context("klient portini ochib bo'lmadi")?;
    let seen: tls::SeenCert = Arc::new(Mutex::new(None));
    let ccfg = tls::client_config_alpn(seen.clone(), ALPN, MAX_BIDI)?;
    let conn = timeout(
        STEP_TIMEOUT,
        endpoint.connect_with(ccfg, addr, SERVER_NAME).context("ulanishni boshlab bo'lmadi")?,
    )
    .await
    .map_err(|_| anyhow!("{} javob bermadi (server ishlayaptimi, port ochiqmi?)", cfg.server))?
    .context("QUIC ulanishi muvaffaqiyatsiz")?;

    // Server fingerprint'i: birinchi ulanishda saqlanadi, keyin o'zgarsa ulanish rad etiladi.
    let cert = seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or_else(|| anyhow!("server sertifikati olinmadi"))?;
    let fp_bytes = sha256(&cert);
    let fp = hexfmt::colon(&fp_bytes);
    let kh = KnownHosts::new(cfg.known_hosts.clone());
    let key = format!("relay:{}", cfg.server);
    match kh.check(&key, &fp)? {
        Trust::Known => {}
        Trust::Unknown => kh.add(&key, &fp)?,
        Trust::Changed { expected } => bail!(
            "DIQQAT: relay server fingerprint'i o'zgargan!\n  saqlangan: {expected}\n  hozirgi:   {fp}\nKnown_hosts faylidan `{key}` qatorini o'chirmaguncha ulanilmaydi."
        ),
    }

    let (mut send, mut recv) = conn.open_bi().await.context("control oqimini ochib bo'lmadi")?;
    write_msg(&mut send, &RMsg::Hello { id: cfg.id.clone(), token: cfg.token.clone(), name: cfg.name.clone() }).await?;
    // Javob: `Challenge` (kalitli server) yoki to'g'ridan-to'g'ri `Ok` (ochiq server, kalit kerak emas)
    let mut first = timeout(STEP_TIMEOUT, read_msg(&mut recv)).await.context("server javob bermadi")??;
    if let RMsg::Challenge { salt, nonce } = first {
        if cfg.key.is_empty() {
            bail!("bu server kalit talab qiladi: server kalitini kiriting");
        }
        let derived = auth::derive_key(&cfg.key, &salt)?;
        write_msg(&mut send, &RMsg::Proof { proof: auth::compute_proof(&derived, &nonce, &fp_bytes) }).await?;
        first = timeout(STEP_TIMEOUT, read_msg(&mut recv)).await.context("server javob bermadi")??;
    }
    match first {
        RMsg::Ok { id, token } => {
            if id != cfg.id || token != cfg.token {
                cfg.id = id.clone();
                cfg.token = token.clone();
                let _ = ev.send(AgentEvent::Registered { id, token });
            }
        }
        RMsg::Err { text } => bail!("server rad etdi: {text}"),
        other => bail!("kutilmagan xabar: {other:?}"),
    }
    let _ = ev.send(AgentEvent::Status { online: true, text: format!("serverga ulangan (fingerprint {})", &fp[..23.min(fp.len())]) });
    if !subs.is_empty() {
        write_msg(&mut send, &RMsg::Sub(subs.clone())).await?;
    }

    let (in_tx, mut in_rx) = mpsc::unbounded_channel::<RMsg>();
    let reader = tokio::spawn(async move {
        while let Ok(m) = read_msg(&mut recv).await {
            if in_tx.send(m).is_err() {
                break;
            }
        }
    });

    let mut connects: HashMap<String, oneshot::Sender<Result<Relayed, String>>> = HashMap::new();
    let mut pending_in: HashMap<String, String> = HashMap::new();
    let exit = loop {
        tokio::select! {
            m = in_rx.recv() => {
                let Some(m) = m else { break Exit::Disconnected };
                match m {
                    RMsg::Pres { id, online, name } => {
                        let _ = ev.send(AgentEvent::Presence { id, online, name });
                    }
                    RMsg::Incoming { sid, from, name } => {
                        let (auto, unattended) = {
                            let p = policy.lock().unwrap_or_else(|e| e.into_inner());
                            (p.unattended || p.trusted.contains(&from), p.unattended)
                        };
                        if auto {
                            accept(&conn, &mut send, &sid, &from, !unattended, cfg.host_port).await?;
                        } else {
                            pending_in.insert(sid.clone(), from.clone());
                            let _ = ev.send(AgentEvent::Incoming { sid, from, name });
                        }
                    }
                    RMsg::Result { req, ok, sid, pw, text } => {
                        if let Some(tx) = connects.remove(&req) {
                            if ok {
                                let conn = conn.clone();
                                tokio::spawn(async move {
                                    let _ = tx.send(open_viewer_side(&conn, &sid, pw).await.map_err(|e| format!("{e:#}")));
                                });
                            } else {
                                let _ = tx.send(Err(text));
                            }
                        }
                    }
                    _ => {}
                }
            }
            c = cmds.recv() => match c {
                None => break Exit::Stopped,
                Some(Cmd::Subscribe(v)) => {
                    *subs = v.clone();
                    write_msg(&mut send, &RMsg::Sub(v)).await?;
                }
                Some(Cmd::Reply { sid, accept: yes }) => {
                    if let Some(from) = pending_in.remove(&sid) {
                        if yes {
                            accept(&conn, &mut send, &sid, &from, true, cfg.host_port).await?;
                        } else {
                            write_msg(&mut send, &RMsg::Reply { sid, accept: false, pw: None }).await?;
                        }
                    }
                }
                Some(Cmd::Connect { target, tx }) => {
                    let req = hexfmt::encode(&auth::random_bytes::<6>());
                    connects.insert(req.clone(), tx);
                    write_msg(&mut send, &RMsg::Connect { target, req }).await?;
                }
            },
            _ = conn.closed() => break Exit::Disconnected,
        }
    };
    reader.abort();
    conn.close(0u32.into(), b"bye");
    // Yopish xabari serverga yetib borishi uchun (aks holda u faqat idle timeout'dan keyin sezadi)
    let _ = timeout(Duration::from_secs(2), endpoint.wait_idle()).await;
    Ok(exit)
}

/// Kiruvchi so'rovni qabul qiladi: kerak bo'lsa bir martalik parol yaratadi, serverga javob beradi va
/// ma'lumot oqimini mahalliy Host'ga ulaydi.
async fn accept(
    conn: &quinn::Connection,
    send: &mut quinn::SendStream,
    sid: &str,
    from: &str,
    one_time: bool,
    host_port: Option<u16>,
) -> Result<()> {
    let pw = if one_time {
        let made = Credentials::load(&paths::host_password_file()?)
            .context("Host paroli o'rnatilmagan")
            .and_then(|c| onetime::create(&c.salt));
        match made {
            Ok(p) => Some(p),
            Err(e) => {
                write_msg(send, &RMsg::Reply { sid: sid.to_string(), accept: false, pw: None }).await?;
                eprintln!("[agent] bir martalik parol yaratilmadi: {e:#}");
                return Ok(());
            }
        }
    } else {
        None
    };
    write_msg(send, &RMsg::Reply { sid: sid.to_string(), accept: true, pw }).await?;
    let (conn, sid, from) = (conn.clone(), sid.to_string(), from.to_string());
    tokio::spawn(async move {
        if let Err(e) = open_host_side(&conn, &sid, &from, host_port).await {
            eprintln!("[agent] ma'lumot oqimi ochilmadi: {e:#}");
        }
    });
    Ok(())
}

/// Qabul qiluvchi tomon: oqim mahalliy Host'ning UDP portiga yo'naltiriladi (TCP tunnel serveri kabi).
async fn open_host_side(conn: &quinn::Connection, sid: &str, from: &str, host_port: Option<u16>) -> Result<()> {
    let (mut s, r) = conn.open_bi().await.context("oqim ochilmadi")?;
    write_msg(&mut s, &RMsg::Data { sid: sid.to_string() }).await?;
    // Host manba manzilni ko'radi: har bir masofaviy ID o'z loopback manzilidan keladi (brute-force cheklovi ID bo'yicha)
    let lo = IpAddr::V4(tcp_tunnel::loopback_for_key(from));
    let udp = match UdpSocket::bind(SocketAddr::new(lo, 0)).await {
        Ok(s) => s,
        Err(_) => UdpSocket::bind("127.0.0.1:0").await.context("lokal UDP port ochilmadi")?,
    };
    let port = host_port.or_else(paths::read_host_port).unwrap_or(DEFAULT_PORT);
    tcp_tunnel::pump_io(r, s, udp, Some(SocketAddr::from(([127, 0, 0, 1], port))), false).await;
    Ok(())
}

/// Ulanuvchi tomon: oqim ochiladi va lokal UDP manzilga ko'prik quriladi.
async fn open_viewer_side(conn: &quinn::Connection, sid: &str, pw: Option<String>) -> Result<Relayed> {
    let (mut s, r) = conn.open_bi().await.context("oqim ochilmadi")?;
    write_msg(&mut s, &RMsg::Data { sid: sid.to_string() }).await?;
    let udp = UdpSocket::bind("127.0.0.1:0").await.context("lokal UDP port ochilmadi")?;
    let local = udp.local_addr()?;
    tokio::spawn(tcp_tunnel::pump_io(r, s, udp, None, true));
    Ok(Relayed { local, pw })
}
