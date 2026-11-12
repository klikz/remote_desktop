//! Viewer tomoni: Host'ga ulanish va autentifikatsiya.

use anyhow::{anyhow, bail, Context, Result};
use protocol::{
    reason, video_reason, Capabilities, Message, VideoHeader, PROTOCOL_VERSION, VIDEO_HEADER_LEN,
};
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::time::timeout;

use crate::{
    auth,
    cert::sha256,
    framing::{read_message, write_message},
    tcp_tunnel, tls,
};

const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// `Transport::Auto`: UDP javobini shuncha kutib, so'ng TCP'ga o'tiladi.
const AUTO_UDP_TIMEOUT: Duration = Duration::from_secs(4);
/// Portal/kodlagich ishga tushishi uchun ko'proq vaqt beriladi.
const VIDEO_START_TIMEOUT: Duration = Duration::from_secs(20);

pub struct ConnectOptions<'a> {
    pub addr: SocketAddr,
    pub password: &'a str,
}

/// Ulanish transporti: QUIC (UDP) yoki UDP bloklangan tarmoqlar uchun TCP orqali tunnel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Transport {
    /// Avval UDP, Host'ga yetib bo'lmasa TCP.
    #[default]
    Auto,
    Udp,
    Tcp,
}

/// Host'ga (UDP orqali) yetib bo'lmadi: `Transport::Auto` shunda TCP'ga o'tadi.
#[derive(Debug)]
pub struct Unreachable(pub String);

impl std::fmt::Display for Unreachable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unreachable {}

/// Host ulanishni doimiy rad etdi (kalit noto'g'ri, versiya mos emas): qayta urinish foyda bermaydi.
#[derive(Debug)]
pub struct Rejected(pub String);

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Rejected {}

pub struct Session {
    /// Ulanish TCP tunnel orqali o'rnatilganmi.
    pub via_tcp: bool,
    endpoint: quinn::Endpoint,
    conn: quinn::Connection,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    next_ping: u64,
    pub host_caps: Capabilities,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Tanlangan transport bilan ulanadi.
pub async fn connect_with(opts: ConnectOptions<'_>, transport: Transport) -> Result<Session> {
    match transport {
        Transport::Udp => connect(opts).await,
        Transport::Tcp => connect_via_tcp(opts).await,
        Transport::Auto => {
            let again = ConnectOptions { addr: opts.addr, password: opts.password };
            match connect_timed(opts, AUTO_UDP_TIMEOUT).await {
                Err(e) if e.downcast_ref::<Unreachable>().is_some() => {
                    eprintln!("UDP orqali ulanib bo'lmadi, TCP sinab ko'riladi...");
                    connect_via_tcp(again).await.map_err(|e2| anyhow!("UDP: {e}\nTCP: {e2:#}"))
                }
                other => other,
            }
        }
    }
}

/// QUIC paketlarini TCP orqali tashib ulanadi (qarang: `tcp_tunnel`).
pub async fn connect_via_tcp(opts: ConnectOptions<'_>) -> Result<Session> {
    let local = tcp_tunnel::connect(opts.addr).await?;
    let opts = ConnectOptions { addr: local, ..opts };
    let mut s = connect_timed(opts, STEP_TIMEOUT).await?;
    s.via_tcp = true;
    Ok(s)
}

/// Relay serveri orqali ochilgan lokal UDP ko'prikka (`opts.addr`) ulanadi.
pub async fn connect_relayed(opts: ConnectOptions<'_>) -> Result<Session> {
    let mut s = connect_timed(opts, STEP_TIMEOUT).await?;
    s.via_tcp = true;
    Ok(s)
}

pub async fn connect(opts: ConnectOptions<'_>) -> Result<Session> {
    connect_timed(opts, STEP_TIMEOUT).await
}

async fn connect_timed(opts: ConnectOptions<'_>, handshake_timeout: Duration) -> Result<Session> {
    let bind: SocketAddr = if opts.addr.is_ipv6() {
        "[::]:0".parse()?
    } else {
        "0.0.0.0:0".parse()?
    };
    let endpoint = quinn::Endpoint::client(bind).context("klient portini ochib bo'lmadi")?;

    let seen: tls::SeenCert = Arc::new(Mutex::new(None));
    let cfg = tls::client_config(seen.clone())?;
    let connecting = endpoint
        .connect_with(cfg, opts.addr, protocol::SERVER_NAME)
        .context("ulanishni boshlab bo'lmadi")?;
    let shown = opts.addr;
    let conn = timeout(handshake_timeout, connecting)
        .await
        .map_err(|_| {
            anyhow::Error::new(Unreachable(format!("{shown} ga ulanish vaqti tugadi (Host ishlayaptimi, port ochiqmi?)")))
        })?
        .context("QUIC ulanishi muvaffaqiyatsiz")?;

    // Sertifikat xeshi parol isbotiga bog'lanadi: vositachi isbotni haqiqiy Host'ga uzata olmaydi.
    let cert_der = seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or_else(|| anyhow!("Host sertifikati olinmadi"))?;
    let fp_bytes = sha256(&cert_der);

    let (mut send, mut recv) = conn.open_bi().await.context("control oqimini ochib bo'lmadi")?;
    write_message(&mut send, &Message::ClientHello { version: PROTOCOL_VERSION }).await?;

    let (salt, nonce) = match timeout(STEP_TIMEOUT, read_message(&mut recv))
        .await
        .context("Host javob bermadi (vaqt tugadi)")??
    {
        Message::ServerHello { salt, nonce, .. } => (salt, nonce),
        Message::AuthResult { ok: false, reason: r } => {
            return Err(Rejected(format!("Host rad etdi: {}", reason::describe(r))).into())
        }
        other => bail!("kutilmagan xabar: {other:?}"),
    };

    let derived = auth::derive_key(opts.password, &salt)?;
    let proof = auth::compute_proof(&derived, &nonce, &fp_bytes);
    write_message(&mut send, &Message::AuthProof { proof }).await?;

    match timeout(STEP_TIMEOUT, read_message(&mut recv))
        .await
        .context("autentifikatsiya natijasi kelmadi (vaqt tugadi)")??
    {
        Message::AuthResult { ok: true, .. } => {}
        Message::AuthResult { ok: false, reason: r } => {
            return Err(Rejected(format!("autentifikatsiya rad etildi: {}", reason::describe(r))).into())
        }
        other => bail!("kutilmagan xabar: {other:?}"),
    }

    let host_caps = match timeout(STEP_TIMEOUT, read_message(&mut recv))
        .await
        .context("Capabilities kelmadi (vaqt tugadi)")??
    {
        Message::Capabilities(c) => c,
        other => bail!("Capabilities kutilgan edi, keldi: {other:?}"),
    };

    Ok(Session {
        via_tcp: false,
        endpoint,
        conn,
        send,
        recv,
        next_ping: 1,
        host_caps,
    })
}

/// Host yuborayotgan video oqimi.
pub struct VideoStream {
    recv: quinn::RecvStream,
    pub codec: u8,
    pub fps: u16,
    pub kbps: u32,
}

pub struct VideoFrame {
    pub keyframe: bool,
    pub pts_us: u64,
    /// H.264 (Annex B) baytlari.
    pub data: Vec<u8>,
}

impl VideoStream {
    /// Keyingi kadr. Host oqimni tugatsa `Ok(None)`.
    pub async fn next_frame(&mut self) -> Result<Option<VideoFrame>> {
        let mut hdr = [0u8; VIDEO_HEADER_LEN];
        match self.recv.read_exact(&mut hdr).await {
            Ok(()) => {}
            Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        let h = VideoHeader::decode(&hdr)?;
        let mut data = vec![0u8; h.len as usize];
        self.recv
            .read_exact(&mut data)
            .await
            .context("video kadri to'liq kelmadi")?;
        Ok(Some(VideoFrame {
            keyframe: h.keyframe,
            pts_us: h.pts_us,
            data,
        }))
    }
}

/// Host videoni boshlashdan bosh tortdi (`video_reason` kodi).
#[derive(Debug)]
pub struct VideoRefused(pub u8);

impl std::fmt::Display for VideoRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Host videoni boshlay olmadi: {}", video_reason::describe(self.0))
    }
}

impl std::error::Error for VideoRefused {}

impl Session {
    /// Ulanish tutqichi: fayl almashinuvi kabi qo'shimcha oqimlar ochish uchun (`filexfer`).
    pub fn connection(&self) -> quinn::Connection {
        self.conn.clone()
    }

    /// Host ulanishni ataylab yopdimi (`Uzish`, Host to'xtadi): qayta ulanish foyda bermaydi.
    /// Tarmoq uzilishi/timeout (`TimedOut`, `Reset` ...) bunga kirmaydi.
    pub fn closed_by_peer(&self) -> bool {
        matches!(self.conn.close_reason(), Some(quinn::ConnectionError::ApplicationClosed(_)))
    }

    /// Host'dan saqlangan ekran ruxsatini bekor qilib, qaytadan so'rashni talab qiladi.
    pub async fn reset_permissions(&mut self) -> Result<()> {
        write_message(&mut self.send, &Message::ResetPermissions).await?;
        Ok(())
    }

    /// Host'ga sichqoncha/klaviatura hodisasini yuboradi.
    pub async fn send_input(&mut self, ev: protocol::InputEvent) -> Result<()> {
        write_message(&mut self.send, &Message::Input(ev)).await?;
        Ok(())
    }

    /// Bir nechta kiritish hodisasini bitta yozuv bilan yuboradi (ketma-ket sichqoncha harakatlaridan faqat
    /// oxirgisi qoladi): kamroq paket, kamroq kechikish.
    pub async fn send_inputs(&mut self, events: impl IntoIterator<Item = protocol::InputEvent>) -> Result<()> {
        let mut buf = Vec::new();
        for ev in crate::pacing::coalesce_inputs(events) {
            buf.extend_from_slice(&protocol::encode_frame(&Message::Input(ev)));
        }
        if !buf.is_empty() {
            self.send.write_all(&buf).await?;
        }
        Ok(())
    }

    /// Host'dan kalit kadr (IDR) so'raydi.
    pub async fn request_keyframe(&mut self) -> Result<()> {
        write_message(&mut self.send, &Message::RequestKeyframe).await
    }

    /// Oxirgi qabul qilingan kadr va dekoder navbati haqida Host'ga xabar beradi (`pacing`).
    pub async fn send_feedback(&mut self, last_pts_us: u64, queue: u8) -> Result<()> {
        write_message(&mut self.send, &Message::Feedback { last_pts_us, queue }).await
    }

    /// QUIC o'lchagan aylanish vaqti (RTT): alohida ping kerak emas.
    pub fn rtt(&self) -> Duration {
        self.conn.rtt()
    }

    /// Hostdan videoni boshlashni so'raydi va kadrlar oqimini qaytaradi.
    pub async fn request_video(&mut self, fps: u16, kbps: u32, max_height: u16, monitor: u8) -> Result<VideoStream> {
        write_message(&mut self.send, &Message::VideoRequest { fps, kbps, max_height, monitor }).await?;
        match timeout(VIDEO_START_TIMEOUT, read_message(&mut self.recv))
            .await
            .context("Host videoni boshlamadi (vaqt tugadi)")??
        {
            Message::VideoStarted { codec, fps, kbps } => {
                let recv = timeout(VIDEO_START_TIMEOUT, self.conn.accept_uni())
                    .await
                    .context("video oqimi kelmadi (vaqt tugadi)")?
                    .context("video oqimini qabul qilib bo'lmadi")?;
                Ok(VideoStream { recv, codec, fps, kbps })
            }
            Message::VideoFailed { reason } => Err(VideoRefused(reason).into()),
            other => bail!("kutilmagan javob: {other:?}"),
        }
    }

    /// Ping yuboradi va aylanish vaqtini (RTT) qaytaradi.
    pub async fn ping(&mut self) -> Result<Duration> {
        self.ping_within(STEP_TIMEOUT).await
    }

    /// `ping`, lekin Pong uchun `wait` gacha kutadi. Pong'dan oldin kelgan boshqa xabarlar (yoki eski Pong'lar)
    /// o'tkazib yuboriladi. Vaqt tugasa oqim buzilgan hisoblanadi: sessiyani tugatish kerak.
    pub async fn ping_within(&mut self, wait: Duration) -> Result<Duration> {
        let id = self.next_ping;
        self.next_ping += 1;
        let start = Instant::now();
        write_message(&mut self.send, &Message::Ping { id, sent_ms: now_ms() }).await?;
        let recv = &mut self.recv;
        timeout(wait, async move {
            loop {
                if let Message::Pong { id: got, .. } = read_message(recv).await? {
                    if got == id {
                        return Ok::<_, anyhow::Error>(start.elapsed());
                    }
                }
            }
        })
        .await
        .context("Pong kelmadi (vaqt tugadi)")?
    }

    /// Ulanishni tartibli yopadi.
    pub async fn close(mut self) {
        let _ = write_message(&mut self.send, &Message::Bye { reason: 0 }).await;
        let _ = self.send.finish();
        let _ = timeout(Duration::from_secs(2), self.send.stopped()).await;
        self.conn.close(0u32.into(), b"bye");
        let _ = timeout(Duration::from_secs(2), self.endpoint.wait_idle()).await;
    }
}
