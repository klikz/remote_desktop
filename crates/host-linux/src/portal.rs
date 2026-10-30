//! xdg-desktop-portal (ScreenCast + RemoteDesktop) sessiyasi.
//!
//! Sessiya bitta fon vazifasida yashaydi. Ruxsat (restore token) faylda saqlanadi, shuning uchun
//! dialog faqat birinchi marta chiqadi. Sessiya yaroqsiz bo'lib qolsa (masalan ekran qulflansa),
//! keyingi so'rovda qayta ochiladi.

use anyhow::{anyhow, Context, Result};
use ashpd::desktop::{
    remote_desktop::{Axis, DeviceType, KeyState, RemoteDesktop},
    screencast::{CursorMode, Screencast, SourceType},
    PersistMode,
};
use protocol::InputEvent;
use std::{os::fd::OwnedFd, path::PathBuf};
use tokio::sync::{mpsc, oneshot};

pub struct StreamInfo {
    /// PipeWire ulanishi (fd). Egalikni kodlagich oladi.
    pub fd: OwnedFd,
    pub node_id: u32,
    /// Tanlangan monitor tartib raqami, monitorlar soni va tanlangan monitor o'lchami.
    pub index: usize,
    pub count: usize,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy)]
struct Mon {
    node_id: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

enum Cmd {
    OpenStream(usize, oneshot::Sender<Result<StreamInfo>>),
    Input(InputEvent),
}

#[derive(Clone)]
pub struct PortalHandle {
    tx: mpsc::Sender<Cmd>,
}

impl PortalHandle {
    pub fn spawn(token_path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel(512);
        tokio::spawn(portal_task(token_path, rx));
        PortalHandle { tx }
    }

    /// Kiritish hodisasini portalga uzatadi. Ko'rsatkich harakati zich bo'lgani uchun navbat
    /// to'lsa tashlab yuboriladi; tugma va klavishlar esa yo'qolmaydi.
    pub async fn input(&self, ev: InputEvent) {
        if matches!(ev, InputEvent::PointerMove { .. }) {
            let _ = self.tx.try_send(Cmd::Input(ev));
        } else {
            let _ = self.tx.send(Cmd::Input(ev)).await;
        }
    }

    /// Ekran oqimi uchun PipeWire ulanishini so'raydi (kerak bo'lsa sessiyani ochadi).
    pub async fn open_stream(&self, monitor: usize) -> Result<StreamInfo> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(Cmd::OpenStream(monitor, reply_tx))
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

        if matches!(first, Cmd::Input(_)) {
            continue; // sessiya yo'q: kiritish hodisasi e'tiborsiz qoldiriladi
        }

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
            let mut mons: Vec<Mon> = started
                .streams()
                .unwrap_or_default()
                .iter()
                .map(|s| {
                    let (x, y) = s.position().unwrap_or((0, 0));
                    let (w, h) = s.size().unwrap_or((0, 0));
                    Mon { node_id: s.pipe_wire_node_id(), x, y, w, h }
                })
                .collect();
            if mons.is_empty() {
                return Err(anyhow!("portal oqim bermadi"));
            }
            // Chapdan o'ngga, so'ng tepadan pastga: tartib barqaror bo'lsin
            mons.sort_by_key(|m| (m.x, m.y));
            Ok::<_, anyhow::Error>((rd, sc, session, mons))
        }
        .await;

        let (rd, sc, session, mons) = match setup {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[portal] sessiyani ochib bo'lmadi: {e:#}");
                if let Cmd::OpenStream(_, reply) = first {
                    let _ = reply.send(Err(e));
                }
                continue;
            }
        };
        eprintln!("[portal] sessiya ochildi, monitorlar: {}", mons.len());
        for (i, m) in mons.iter().enumerate() {
            eprintln!("[portal]   monitor {i}: {}x{} @ ({}, {}), node {}", m.w, m.h, m.x, m.y, m.node_id);
        }

        let mut next = Some(first);
        let mut cur = mons[0];
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
            let (want, reply) = match cmd {
                Cmd::Input(ev) => {
                    match ev {
                        InputEvent::PointerMove { x, y } => {
                            let px = f64::from(x) / 65535.0 * f64::from(cur.w.max(1));
                            let py = f64::from(y) / 65535.0 * f64::from(cur.h.max(1));
                            let _ = rd.notify_pointer_motion_absolute(&session, cur.node_id, px, py).await;
                        }
                        InputEvent::Button { button, pressed } => {
                            let code = match button {
                                0 => 0x110, // BTN_LEFT
                                1 => 0x111, // BTN_RIGHT
                                _ => 0x112, // BTN_MIDDLE
                            };
                            let st = if pressed { KeyState::Pressed } else { KeyState::Released };
                            let _ = rd.notify_pointer_button(&session, code, st).await;
                        }
                        InputEvent::Scroll { dx, dy } => {
                            if dy != 0 {
                                let _ = rd.notify_pointer_axis_discrete(&session, Axis::Vertical, i32::from(dy)).await;
                            }
                            if dx != 0 {
                                let _ = rd.notify_pointer_axis_discrete(&session, Axis::Horizontal, i32::from(dx)).await;
                            }
                        }
                        InputEvent::Key { code, pressed } => {
                            let st = if pressed { KeyState::Pressed } else { KeyState::Released };
                            let _ = rd.notify_keyboard_keycode(&session, code as i32, st).await;
                        }
                    }
                    continue;
                }
                Cmd::OpenStream(w, r) => (w, r),
            };
            let index = want.min(mons.len() - 1);
            let m = mons[index];
            cur = m;
            // Sinov uchun: RD_POINTER_SIZE=3840x2160 koordinata fazosini qo'lda belgilaydi
            if let Some((w, h)) = std::env::var("RD_POINTER_SIZE").ok().and_then(|v| {
                let (w, h) = v.split_once('x')?;
                Some((w.trim().parse::<i32>().ok()?, h.trim().parse::<i32>().ok()?))
            }) {
                cur.w = w;
                cur.h = h;
            }
            eprintln!("[portal] kiritish koordinata fazosi: {}x{}", cur.w, cur.h);
            match sc.open_pipe_wire_remote(&session).await {
                Ok(fd) => {
                    let _ = reply.send(Ok(StreamInfo {
                        fd,
                        node_id: m.node_id,
                        index,
                        count: mons.len(),
                        width: m.w,
                        height: m.h,
                    }));
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
