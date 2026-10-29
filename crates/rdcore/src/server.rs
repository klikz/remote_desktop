//! Host tomoni: ulanishlarni qabul qilish va autentifikatsiya.

use anyhow::{bail, Context, Result};
use protocol::{reason, Capabilities, Message, PROTOCOL_VERSION};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::time::timeout;

use crate::{
    auth::{self, Credentials},
    cert::HostIdentity,
    framing::{read_message, write_message},
    ratelimit::RateLimiter,
    tls,
};

const STEP_TIMEOUT: Duration = Duration::from_secs(10);

struct State {
    creds: Credentials,
    fingerprint: [u8; 32],
    caps: Capabilities,
    limiter: RateLimiter,
}

pub struct Host {
    endpoint: quinn::Endpoint,
    state: Arc<State>,
}

impl Host {
    /// `bind` manzilida UDP port ochadi. Tokio runtime ichida chaqirilishi kerak.
    pub fn bind(bind: SocketAddr, identity: &HostIdentity, creds: Credentials, caps: Capabilities) -> Result<Self> {
        let server_config = tls::server_config(identity)?;
        let endpoint = quinn::Endpoint::server(server_config, bind)
            .with_context(|| format!("{bind} manzilini tinglab bo'lmadi"))?;
        Ok(Host {
            endpoint,
            state: Arc::new(State {
                creds,
                fingerprint: identity.fingerprint(),
                caps,
                limiter: RateLimiter::standard(),
            }),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    pub fn fingerprint(&self) -> [u8; 32] {
        self.state.fingerprint
    }

    /// Ulanishlarni qabul qilish sikli (to'xtamaydi).
    pub async fn run(self) -> Result<()> {
        while let Some(incoming) = self.endpoint.accept().await {
            let ip = incoming.remote_address().ip();
            if self.state.limiter.is_blocked(ip) {
                eprintln!("[host] {ip}: bloklangan, ulanish rad etildi");
                incoming.refuse();
                continue;
            }
            let state = self.state.clone();
            tokio::spawn(async move {
                if let Err(e) = serve(incoming, &state, ip).await {
                    eprintln!("[host] {ip}: {e:#}");
                }
            });
        }
        Ok(())
    }
}

/// Oqimni tartib bilan yopadi: Viewer oxirgi xabarni o'qib bo'lguncha kutadi.
async fn graceful_reject(mut send: quinn::SendStream, conn: &quinn::Connection) {
    let _ = send.finish();
    let _ = timeout(Duration::from_secs(2), send.stopped()).await;
    conn.close(1u32.into(), b"rejected");
}

async fn serve(incoming: quinn::Incoming, state: &State, ip: IpAddr) -> Result<()> {
    let connecting = incoming.accept().context("ulanishni qabul qilib bo'lmadi")?;
    let conn = timeout(STEP_TIMEOUT, connecting)
        .await
        .context("handshake vaqti tugadi")?
        .context("handshake muvaffaqiyatsiz")?;

    let (mut send, mut recv) = timeout(STEP_TIMEOUT, conn.accept_bi())
        .await
        .context("control oqimi ochilmadi (vaqt tugadi)")?
        .context("control oqimi ochilmadi")?;

    let hello = timeout(STEP_TIMEOUT, read_message(&mut recv))
        .await
        .context("ClientHello kelmadi (vaqt tugadi)")??;
    let Message::ClientHello { version } = hello else {
        bail!("birinchi xabar ClientHello emas");
    };
    if version != PROTOCOL_VERSION {
        write_message(
            &mut send,
            &Message::AuthResult { ok: false, reason: reason::VERSION_MISMATCH },
        )
        .await?;
        graceful_reject(send, &conn).await;
        bail!("protokol versiyasi mos kelmaydi (viewer: {version}, host: {PROTOCOL_VERSION})");
    }

    let nonce = auth::random_bytes::<{ protocol::NONCE_LEN }>();
    write_message(
        &mut send,
        &Message::ServerHello { version: PROTOCOL_VERSION, salt: state.creds.salt, nonce },
    )
    .await?;

    let proof_msg = timeout(STEP_TIMEOUT, read_message(&mut recv))
        .await
        .context("AuthProof kelmadi (vaqt tugadi)")??;
    let Message::AuthProof { proof } = proof_msg else {
        bail!("AuthProof kutilgan edi");
    };

    if !auth::verify_proof(&state.creds.key, &nonce, &state.fingerprint, &proof) {
        state.limiter.record_failure(ip);
        write_message(
            &mut send,
            &Message::AuthResult { ok: false, reason: reason::BAD_PASSWORD },
        )
        .await?;
        graceful_reject(send, &conn).await;
        bail!("noto'g'ri parol");
    }
    state.limiter.record_success(ip);

    write_message(&mut send, &Message::AuthResult { ok: true, reason: reason::OK }).await?;
    write_message(&mut send, &Message::Capabilities(state.caps.clone())).await?;
    eprintln!("[host] {ip}: ulandi");

    loop {
        match read_message(&mut recv).await {
            Ok(Message::Ping { id, sent_ms }) => {
                write_message(&mut send, &Message::Pong { id, sent_ms }).await?;
            }
            Ok(Message::Bye { .. }) => break,
            Ok(other) => bail!("kutilmagan xabar: {other:?}"),
            Err(_) => break, // ulanish uzildi
        }
    }
    conn.close(0u32.into(), b"bye");
    eprintln!("[host] {ip}: uzildi");
    Ok(())
}
