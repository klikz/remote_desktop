//! xdg-desktop-portal (ScreenCast + RemoteDesktop) sessiyasi.
//!
//! Sessiya bitta fon vazifasida yashaydi. Ruxsat (restore token) faylda saqlanadi, shuning uchun
//! dialog faqat birinchi marta chiqadi. Sessiya yaroqsiz bo'lib qolsa (masalan ekran qulflansa),
//! keyingi so'rovda qayta ochiladi.

use anyhow::{anyhow, Context, Result};
use ashpd::desktop::{
    remote_desktop::{DeviceType, RemoteDesktop},
    screencast::{CursorMode, Screencast, SourceType},
    PersistMode,
};
use std::{os::fd::OwnedFd, path::PathBuf};
use tokio::sync::{mpsc, oneshot};

pub struct StreamInfo {
    /// PipeWire ulanishi (fd). Egalikni kodlagich oladi.
    pub fd: OwnedFd,
    pub node_id: u32,
}

enum Cmd {
    OpenStream(oneshot::Sender<Result<StreamInfo>>),
}

#[derive(Clone)]
pub struct PortalHandle {
    tx: mpsc::Sender<Cmd>,
}

impl PortalHandle {
    pub fn spawn(token_path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel(8);
        tokio::spawn(portal_task(token_path, rx));
        PortalHandle { tx }
    }

    /// Ekran oqimi uchun PipeWire ulanishini so'raydi (kerak bo'lsa sessiyani ochadi).
    pub async fn open_stream(&self) -> Result<StreamInfo> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(Cmd::OpenStream(reply_tx))
            .await
            .map_err(|_| anyhow!("portal vazifasi to'xtagan"))?;
        reply_rx.await.map_err(|_| anyhow!("portal vazifasi javob bermadi"))?
    }
}

fn load_token(path: &PathBuf) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Token har sessiyada yangilanadi, shuning uchun darhol va xavfsiz (vaqtincha fayl + rename) yoziladi.
fn save_token(path: &PathBuf, token: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    rdcore::fsutil::write_private(&tmp, token.as_bytes()).context("token faylini yozib bo'lmadi")?;
    std::fs::rename(&tmp, path).context("token faylini almashtirib bo'lmadi")?;
    Ok(())
}

async fn portal_task(token_path: PathBuf, mut rx: mpsc::Receiver<Cmd>) {
    let mut pending: Option<Cmd> = None;
    loop {
        let first = match pending.take() {
            Some(c) => c,
            None => match rx.recv().await {
                Some(c) => c,
                None => return,
            },
        };

        let setup = async {
            let rd = RemoteDesktop::new()
                .await
                .context("RemoteDesktop portaliga ulanib bo'lmadi")?;
            let sc = Screencast::new()
                .await
                .context("ScreenCast portaliga ulanib bo'lmadi")?;
            let session = rd
                .create_session()
                .await
                .context("portal sessiyasi yaratilmadi (ekran qulflangan yoki o'chgan bo'lishi mumkin)")?;
            let saved = load_token(&token_path);
            rd.select_devices(
                &session,
                DeviceType::Keyboard | DeviceType::Pointer,
                saved.as_deref(),
                PersistMode::ExplicitlyRevoked,
            )
            .await
            .context("select_devices xatosi")?;
            sc.select_sources(
                &session,
                CursorMode::Embedded,
                SourceType::Monitor.into(),
                false,
                None,
                PersistMode::DoNot,
            )
            .await
            .context("select_sources xatosi")?;
            let started = rd
                .start(&session, None)
                .await
                .context("portal start xatosi")?
                .response()
                .context("portal ruxsat bermadi")?;
            if let Some(t) = started.restore_token() {
                save_token(&token_path, t)?;
            }
            let node = started
                .streams()
                .unwrap_or_default()
                .first()
                .map(|s| s.pipe_wire_node_id())
                .context("portal oqim bermadi")?;
            Ok::<_, anyhow::Error>((rd, sc, session, node))
        }
        .await;

        let (rd, sc, session, node) = match setup {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[portal] sessiyani ochib bo'lmadi: {e:#}");
                let Cmd::OpenStream(reply) = first;
                let _ = reply.send(Err(e));
                continue;
            }
        };
        eprintln!("[portal] sessiya ochildi (PipeWire node {node})");

        let mut next = Some(first);
        loop {
            let cmd = match next.take() {
                Some(c) => c,
                None => match rx.recv().await {
                    Some(c) => c,
                    None => {
                        let _ = session.close().await;
                        return;
                    }
                },
            };
            let Cmd::OpenStream(reply) = cmd;
            match sc.open_pipe_wire_remote(&session).await {
                Ok(fd) => {
                    let _ = reply.send(Ok(StreamInfo { fd, node_id: node }));
                }
                Err(e) => {
                    eprintln!("[portal] sessiya yaroqsiz, qayta ochiladi: {e}");
                    let _ = reply.send(Err(anyhow!("PipeWire ulanishini olib bo'lmadi: {e}")));
                    break;
                }
            }
        }
        let _ = rd; // sessiya bilan birga tushadi
    }
}
