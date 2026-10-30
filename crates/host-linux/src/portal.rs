//! xdg-desktop-portal (ScreenCast + RemoteDesktop) sessiyasi.
//!
//! Sessiya bitta fon vazifasida yashaydi. Ruxsat (restore token) faylda saqlanadi, shuning uchun
//! dialog faqat birinchi marta chiqadi. Sessiya Host ishga tushganda darhol ochiladi (qulflangan ekranda
//! yangi sessiya ochilmaydi, ochilgani esa ishlaydi); yaroqsiz bo'lib qolsa o'zi qayta ochiladi.

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
    SetPhysical(i32, i32),
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

    /// Oqimning haqiqiy kadr o'lchami (piksel). Logik o'lchamdan katta bo'lsa, ekran masshtabli.
    pub async fn set_physical_size(&self, w: i32, h: i32) {
        let _ = self.tx.send(Cmd::SetPhysical(w, h)).await;
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
    // Sessiya Host ishga tushishi bilan (Viewer so'ramasdan) ochiladi: ekran qulflanganda GNOME yangi
    // sessiyani rad etadi, lekin ochilgan sessiya qulfda ham ishlaydi. Ochilmasa (masalan, ekran qulflangan)
    // foydalanuvchi ruxsatni rad etmaguncha qayta uriniladi.
    let mut eager = true;
    let mut backoff = 3u64;
    let mut first: Option<Cmd> = None;
    loop {
        if first.is_none() && !eager {
            match rx.recv().await {
                Some(c @ Cmd::OpenStream(..)) => first = Some(c),
                Some(_) => continue, // sessiya yo'q: kiritish hodisasi e'tiborsiz qoldiriladi
                None => return,
            }
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
            let devices = started.devices();
            eprintln!("[portal] ruxsat etilgan qurilmalar: {devices:?}");
            if !devices.contains(DeviceType::Pointer) || !devices.contains(DeviceType::Keyboard) {
                if saved.is_some() {
                    // Eski ruxsatda faqat ekran berilgan edi: token o'chiriladi va sessiya darhol qayta ochiladi,
                    // shunda GNOME boshqarish ruxsatini ham so'raydi.
                    eprintln!("[portal] sichqoncha/klaviatura ruxsati yo'q: saqlangan ruxsat bekor qilinib, qayta so'raladi");
                    let _ = std::fs::remove_file(&token_path);
                    let _ = session.close().await;
                    return Err(anyhow::Error::new(NoControl));
                }
                eprintln!("[portal] OGOHLANTIRISH: boshqarish ruxsati berilmadi; faqat ko'rish ishlaydi (dialogda \"Boshqarish\" ni yoqing)");
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
                // Foydalanuvchi ruxsatni rad etgan: o'zi so'ramaguncha qayta so'ramaymiz
                if e.downcast_ref::<NoControl>().is_some() {
                    continue; // token o'chirilgan: darhol yangi sessiya (so'rov `first` da saqlanadi)
                }
                let denied = matches!(e.downcast_ref::<ashpd::Error>(), Some(ashpd::Error::Response(_)));
                if let Some(Cmd::OpenStream(_, reply)) = first.take() {
                    let _ = reply.send(Err(e));
                }
                if denied {
                    eager = false;
                } else if eager {
                    tokio::select! {
                        _ = tokio::time::sleep(std::time::Duration::from_secs(backoff)) => {}
                        c = rx.recv() => match c {
                            Some(c @ Cmd::OpenStream(..)) => first = Some(c),
                            Some(_) => {}
                            None => return,
                        },
                    }
                    backoff = (backoff * 2).min(30);
                }
                continue;
            }
        };
        eager = true;
        backoff = 3;
        eprintln!("[portal] sessiya ochildi, monitorlar: {}", mons.len());
        for (i, m) in mons.iter().enumerate() {
            eprintln!("[portal]   monitor {i}: {}x{} @ ({}, {}), node {}", m.w, m.h, m.x, m.y, m.node_id);
        }

        let mut next = first.take();
        // Sessiya aynan shu so'rov uchun ochilgan bo'lsa (yangi), xato bo'lsa qayta urinmaymiz. Ilgari
        // (oldindan) ochilgan sessiya eskirgan bo'lishi mumkin: shunda yangisini ochib, so'rovni qaytaramiz.
        let mut fresh = next.is_some();
        let mut cur = mons[0];
        let mut dbg_last: Option<std::time::Instant> = None;
        let mut phys: Option<(i32, i32)> = None;
        let mut in_err = 0u32;
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
                            // Maqsad: logik koordinata (portal bergan o'lcham).
                            let (lw, lh) = (f64::from(cur.w.max(1)), f64::from(cur.h.max(1)));
                            let (tx, ty) = (f64::from(x) / 65535.0 * lw, f64::from(y) / 65535.0 * lh);
                            // GNOME mutlaq qiymatni 0..logik oraliqda qabul qiladi, lekin masshtabga (s = fizik/logik)
                            // bo'lib qo'llaydi. Shuning uchun s ga ko'paytirib yuboramiz; oraliqdan chiqadigan qismni
                            // (ekranning o'ng/pastki chekkasi) nisbiy siljish bilan qo'shamiz.
                            let (sx, sy) = match phys {
                                Some((pw, ph)) if pw > cur.w && ph > cur.h => (f64::from(pw) / lw, f64::from(ph) / lh),
                                _ => (1.0, 1.0),
                            };
                            let ax = (tx * sx).clamp(0.0, lw - 1.0);
                            let ay = (ty * sy).clamp(0.0, lh - 1.0);
                            let (px, py) = (ax, ay);
                            let mut r = rd.notify_pointer_motion_absolute(&session, cur.node_id, ax, ay).await;
                            let (rx, ry) = (tx - ax / sx, ty - ay / sy);
                            if r.is_ok() && (rx > 0.5 || ry > 0.5) {
                                r = rd.notify_pointer_motion(&session, rx.max(0.0), ry.max(0.0)).await;
                            }
                            note_err(&mut in_err, "ko'rsatkich", &r);
                            if std::env::var_os("RD_DEBUG").is_some()
                                && dbg_last.map_or(true, |t: std::time::Instant| t.elapsed().as_millis() >= 500)
                            {
                                dbg_last = Some(std::time::Instant::now());
                                eprintln!(
                                    "[dbg] viewer=({x},{y}) maqsad=({tx:.0},{ty:.0}) mutlaq=({px:.0},{py:.0}) nisbiy=({rx:.0},{ry:.0}) logik {}x{} masshtab {sx:.3} natija {:?}",
                                    cur.w,
                                    cur.h,
                                    r.as_ref().map(|_| "ok").map_err(|e| e.to_string())
                                );
                            }
                        }
                        InputEvent::Button { button, pressed } => {
                            let code = match button {
                                0 => 0x110, // BTN_LEFT
                                1 => 0x111, // BTN_RIGHT
                                _ => 0x112, // BTN_MIDDLE
                            };
                            let st = if pressed { KeyState::Pressed } else { KeyState::Released };
                            let r = rd.notify_pointer_button(&session, code, st).await;
                            note_err(&mut in_err, "tugma", &r);
                        }
                        InputEvent::Scroll { dx, dy } => {
                            if dy != 0 {
                                let r = rd.notify_pointer_axis_discrete(&session, Axis::Vertical, i32::from(dy)).await;
                                note_err(&mut in_err, "g'ildirak", &r);
                            }
                            if dx != 0 {
                                let _ = rd.notify_pointer_axis_discrete(&session, Axis::Horizontal, i32::from(dx)).await;
                            }
                        }
                        InputEvent::Key { code, pressed } => {
                            let st = if pressed { KeyState::Pressed } else { KeyState::Released };
                            let r = rd.notify_keyboard_keycode(&session, code as i32, st).await;
                            note_err(&mut in_err, "klavisha", &r);
                        }
                    }
                    continue;
                }
                Cmd::SetPhysical(w, h) => {
                    phys = Some((w, h));
                    eprintln!(
                        "[portal] kadr {w}x{h}, logik {}x{}: masshtab {:.3}",
                        cur.w,
                        cur.h,
                        f64::from(w) / f64::from(cur.w.max(1))
                    );
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
                    fresh = false;
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
                    if fresh {
                        let _ = reply.send(Err(anyhow!("PipeWire ulanishini olib bo'lmadi: {e}")));
                    } else {
                        first = Some(Cmd::OpenStream(want, reply)); // yangi sessiyada shu so'rov qaytariladi
                    }
                    break;
                }
            }
        }
        let _ = rd; // sessiya bilan birga tushadi
    }
}

/// Kiritish xatolarining birinchi bir nechtasini logga yozadi (sabab ko'rinishi uchun, log to'lib ketmasin).
fn note_err<T, E: std::fmt::Display>(count: &mut u32, what: &str, r: &Result<T, E>) {
    if let Err(e) = r {
        *count += 1;
        if *count <= 10 {
            eprintln!("[portal] kiritish xatosi ({what}): {e}");
        }
    }
}

/// Saqlangan ruxsatda boshqarish yo'q edi: sessiya yopildi, token o'chirildi, qayta ochish kerak.
#[derive(Debug)]
struct NoControl;

impl std::fmt::Display for NoControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("saqlangan ruxsatda boshqarish yo'q")
    }
}

impl std::error::Error for NoControl {}
