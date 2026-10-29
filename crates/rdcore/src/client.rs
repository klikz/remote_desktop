//! Viewer tomoni: Host'ga ulanish, fingerprint tekshiruvi va autentifikatsiya.

use anyhow::{anyhow, bail, Context, Result};
use protocol::{reason, Capabilities, Message, PROTOCOL_VERSION};
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
    hexfmt,
    known_hosts::{KnownHosts, Trust},
    tls,
};

const STEP_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ConnectOptions<'a> {
    pub addr: SocketAddr,
    pub password: &'a str,
    pub known_hosts: &'a KnownHosts,
    /// Birinchi ulanishda fingerprint ko'rsatiladi. `true` qaytarilsa, Host ishonchli deb saqlanadi.
    pub confirm_new_host: &'a dyn Fn(&str) -> bool,
}

pub struct Session {
    endpoint: quinn::Endpoint,
    conn: quinn::Connection,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    next_ping: u64,
    pub host_caps: Capabilities,
    /// Host sertifikati fingerprint'i (`AB:CD:...`).
    pub fingerprint: String,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub async fn connect(opts: ConnectOptions<'_>) -> Result<Session> {
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
    let conn = timeout(STEP_TIMEOUT, connecting)
        .await
        .map_err(|_| anyhow!("{} ga ulanish vaqti tugadi (Host ishlayaptimi, UDP port ochiqmi?)", opts.addr))?
        .context("QUIC ulanishi muvaffaqiyatsiz")?;

    // Fingerprint tekshiruvi: hech qanday maxfiy ma'lumot yuborilmasdan oldin.
    let cert_der = seen
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or_else(|| anyhow!("Host sertifikati olinmadi"))?;
    let fp_bytes = sha256(&cert_der);
    let fp = hexfmt::colon(&fp_bytes);
    let key = opts.addr.to_string();
    match opts.known_hosts.check(&key, &fp)? {
        Trust::Known => {}
        Trust::Unknown => {
            if !(opts.confirm_new_host)(&fp) {
                bail!("Host fingerprint'i tasdiqlanmadi, ulanish bekor qilindi");
            }
            opts.known_hosts.add(&key, &fp)?;
        }
        Trust::Changed { expected } => {
            bail!(
                "DIQQAT: {key} uchun Host fingerprint'i o'zgargan!\n  saqlangan: {expected}\n  hozirgi:   {fp}\nBu vositachi hujumi (MITM) belgisi bo'lishi mumkin. Host sertifikatini o'zingiz almashtirgan bo'lsangiz, known_hosts faylidan shu qatorni o'chiring."
            );
        }
    }

    let (mut send, mut recv) = conn.open_bi().await.context("control oqimini ochib bo'lmadi")?;
    write_message(&mut send, &Message::ClientHello { version: PROTOCOL_VERSION }).await?;

    let (salt, nonce) = match timeout(STEP_TIMEOUT, read_message(&mut recv))
        .await
        .context("Host javob bermadi (vaqt tugadi)")??
    {
        Message::ServerHello { salt, nonce, .. } => (salt, nonce),
        Message::AuthResult { ok: false, reason: r } => bail!("Host rad etdi: {}", reason::describe(r)),
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
        Message::AuthResult { ok: false, reason: r } => bail!("autentifikatsiya rad etildi: {}", reason::describe(r)),
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
        endpoint,
        conn,
        send,
        recv,
        next_ping: 1,
        host_caps,
        fingerprint: fp,
    })
}

impl Session {
    /// Ping yuboradi va aylanish vaqtini (RTT) qaytaradi.
    pub async fn ping(&mut self) -> Result<Duration> {
        let id = self.next_ping;
        self.next_ping += 1;
        let start = Instant::now();
        write_message(&mut self.send, &Message::Ping { id, sent_ms: now_ms() }).await?;
        match timeout(STEP_TIMEOUT, read_message(&mut self.recv))
            .await
            .context("Pong kelmadi (vaqt tugadi)")??
        {
            Message::Pong { id: got, .. } if got == id => Ok(start.elapsed()),
            other => bail!("kutilmagan javob: {other:?}"),
        }
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
