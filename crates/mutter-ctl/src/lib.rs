//! Mutter'ning ichki D-Bus API'si (`org.gnome.Mutter.RemoteDesktop` + `org.gnome.Mutter.ScreenCast`)
//! orqali ekran oqimi va kiritish. xdg-desktop-portal'ga muqobil: ruxsat dialogi yo'q.
//!
//! Qulflangan ekranda GNOME bu sessiyani yopadi, agar "Allow Locked Remote Desktop" kengaytmasi
//! (`inhibit_remote_access` ni o'chiradi) yoqilmagan bo'lsa. Sessiya yopilsa, keyingi `open_stream`
//! yangisini ochadi. Faqat asosiy monitor qo'llanadi (bo'sh connector).
#![cfg(target_os = "linux")]

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use protocol::InputEvent;
use std::{collections::HashMap, time::Duration};
use tokio::sync::{mpsc, oneshot};
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue, Value},
    Connection, Proxy,
};

const RD_DEST: &str = "org.gnome.Mutter.RemoteDesktop";
const RD_PATH: &str = "/org/gnome/Mutter/RemoteDesktop";
const RD_IFACE: &str = "org.gnome.Mutter.RemoteDesktop";
const RD_SESSION_IFACE: &str = "org.gnome.Mutter.RemoteDesktop.Session";
const SC_DEST: &str = "org.gnome.Mutter.ScreenCast";
const SC_PATH: &str = "/org/gnome/Mutter/ScreenCast";
const SC_IFACE: &str = "org.gnome.Mutter.ScreenCast";
const SC_SESSION_IFACE: &str = "org.gnome.Mutter.ScreenCast.Session";
const SC_STREAM_IFACE: &str = "org.gnome.Mutter.ScreenCast.Stream";

/// Ochilgan oqim: PipeWire node (odatiy PipeWire ulanishida, fd kerak emas) va o'lcham (noma'lum bo'lsa 0).
#[derive(Debug, Clone, Copy)]
pub struct Stream {
    pub node_id: u32,
    pub width: i32,
    pub height: i32,
}

enum Cmd {
    Open(oneshot::Sender<Result<Stream>>),
    Input(InputEvent),
    SetPhysical(i32, i32),
    /// Sessiya (avlod raqami bilan) Mutter tomonidan yopildi.
    Closed(u64),
}

#[derive(Clone)]
pub struct MutterHandle {
    tx: mpsc::Sender<Cmd>,
}

/// Mutter RemoteDesktop xizmati mavjudmi va sessiya yaratiladimi (avtomatik tanlash uchun).
pub async fn available() -> Result<()> {
    let conn = Connection::session().await.context("session D-Bus'ga ulanib bo'lmadi")?;
    let rd = Proxy::new(&conn, RD_DEST, RD_PATH, RD_IFACE).await?;
    let path: OwnedObjectPath = rd.call("CreateSession", &()).await.context("RemoteDesktop.CreateSession")?;
    if let Ok(s) = Proxy::new(&conn, RD_DEST, path, RD_SESSION_IFACE).await {
        let _: zbus::Result<()> = s.call("Stop", &()).await;
    }
    Ok(())
}

impl MutterHandle {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel(512);
        tokio::spawn(task(tx.clone(), rx));
        MutterHandle { tx }
    }

    /// Ko'rsatkich harakati zich: navbat to'lsa tashlanadi; tugma va klavishlar yo'qolmaydi.
    pub async fn input(&self, ev: InputEvent) {
        if matches!(ev, InputEvent::PointerMove { .. }) {
            let _ = self.tx.try_send(Cmd::Input(ev));
        } else {
            let _ = self.tx.send(Cmd::Input(ev)).await;
        }
    }

    /// Oqimning haqiqiy kadr o'lchami (piksel): mutlaq ko'rsatkich koordinatalari shu fazoda.
    pub async fn set_physical_size(&self, w: i32, h: i32) {
        let _ = self.tx.send(Cmd::SetPhysical(w, h)).await;
    }

    pub async fn open_stream(&self) -> Result<Stream> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx.send(Cmd::Open(reply_tx)).await.map_err(|_| anyhow!("mutter vazifasi to'xtagan"))?;
        reply_rx.await.map_err(|_| anyhow!("mutter vazifasi javob bermadi"))?
    }
}

struct Sess {
    rd: Proxy<'static>,
    stream_path: String,
    stream: Stream,
    gen: u64,
}

async fn open_session(conn: &Connection, gen: u64, tx: &mpsc::Sender<Cmd>) -> Result<Sess> {
    let rd = Proxy::new(conn, RD_DEST, RD_PATH, RD_IFACE).await?;
    let rd_path: OwnedObjectPath = rd.call("CreateSession", &()).await.context("RemoteDesktop.CreateSession")?;
    let rd_session = Proxy::new(conn, RD_DEST, rd_path, RD_SESSION_IFACE).await?;

    let built = build(conn, gen, tx, &rd_session).await;
    if built.is_err() {
        let _: zbus::Result<()> = rd_session.call("Stop", &()).await;
    }
    let (stream_path, stream) = built?;
    Ok(Sess { rd: rd_session, stream_path, stream, gen })
}

async fn build(
    conn: &Connection,
    gen: u64,
    tx: &mpsc::Sender<Cmd>,
    rd_session: &Proxy<'static>,
) -> Result<(String, Stream)> {
    let session_id: String = rd_session.get_property("SessionId").await.context("SessionId o'qilmadi")?;

    let sc = Proxy::new(conn, SC_DEST, SC_PATH, SC_IFACE).await?;
    let mut opts: HashMap<&str, Value> = HashMap::new();
    opts.insert("remote-desktop-session-id", Value::from(session_id.as_str()));
    let sc_path: OwnedObjectPath = sc.call("CreateSession", &(opts,)).await.context("ScreenCast.CreateSession")?;
    let sc_session = Proxy::new(conn, SC_DEST, sc_path, SC_SESSION_IFACE).await?;

    // Bo'sh connector = asosiy monitor; cursor-mode 1 = kursor kadrga chiziladi.
    let mut ropts: HashMap<&str, Value> = HashMap::new();
    ropts.insert("cursor-mode", Value::from(1u32));
    let stream_path: OwnedObjectPath =
        sc_session.call("RecordMonitor", &("", ropts)).await.context("ScreenCast.RecordMonitor")?;
    let stream_proxy = Proxy::new(conn, SC_DEST, stream_path.clone(), SC_STREAM_IFACE).await?;
    let mut added = stream_proxy.receive_signal("PipeWireStreamAdded").await?;
    let mut closed = rd_session.receive_signal("Closed").await?;

    let () = rd_session.call("Start", &()).await.context("RemoteDesktop.Session.Start")?;
    let node: u32 = tokio::time::timeout(Duration::from_secs(5), added.next())
        .await
        .context("PipeWireStreamAdded 5 soniyada kelmadi")?
        .ok_or_else(|| anyhow!("signal oqimi yopildi"))?
        .body()
        .deserialize()
        .context("PipeWireStreamAdded o'qilmadi")?;

    // O'lcham: Parameters["size"] (iloji bo'lsa). Bo'lmasa 0: keyin set_physical_size to'ldiradi.
    let (mut width, mut height) = (0, 0);
    if let Ok(params) = stream_proxy.get_property::<HashMap<String, OwnedValue>>("Parameters").await {
        if let Some(v) = params.get("size") {
            if let Ok((w, h)) = <(i32, i32)>::try_from(v.try_clone()?) {
                (width, height) = (w, h);
            }
        }
    }

    let tx = tx.clone();
    tokio::spawn(async move {
        closed.next().await;
        let _ = tx.send(Cmd::Closed(gen)).await;
    });
    Ok((stream_path.to_string(), Stream { node_id: node, width, height }))
}

async fn task(tx: mpsc::Sender<Cmd>, mut rx: mpsc::Receiver<Cmd>) {
    let conn = match Connection::session().await {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("[mutter] session D-Bus'ga ulanib bo'lmadi: {e}");
            None
        }
    };
    let mut sess: Option<Sess> = None;
    let mut gen = 0u64;
    let mut phys: Option<(i32, i32)> = None;
    let mut in_err = 0u32;

    while let Some(cmd) = rx.recv().await {
        match cmd {
            Cmd::Open(reply) => {
                let Some(conn) = &conn else {
                    let _ = reply.send(Err(anyhow!("session D-Bus yo'q")));
                    continue;
                };
                if sess.is_none() {
                    gen += 1;
                    match open_session(conn, gen, &tx).await {
                        Ok(s) => {
                            eprintln!(
                                "[mutter] sessiya ochildi: node {}, o'lcham {}x{}",
                                s.stream.node_id, s.stream.width, s.stream.height
                            );
                            sess = Some(s);
                        }
                        Err(e) => {
                            eprintln!("[mutter] sessiyani ochib bo'lmadi: {e:#}");
                            let _ = reply.send(Err(e));
                            continue;
                        }
                    }
                }
                let _ = reply.send(Ok(sess.as_ref().map(|s| s.stream).expect("sessiya bor")));
            }
            Cmd::SetPhysical(w, h) => phys = Some((w, h)),
            Cmd::Closed(g) => {
                if sess.as_ref().map_or(false, |s| s.gen == g) {
                    eprintln!("[mutter] sessiya Mutter tomonidan yopildi (ekran qulflangan bo'lishi mumkin)");
                    sess = None;
                }
            }
            Cmd::Input(ev) => {
                let Some(s) = &sess else { continue };
                let r: zbus::Result<()> = match ev {
                    InputEvent::PointerMove { x, y } => {
                        // Mutlaq koordinata oqim (kadr) pikselida: kadr o'lchami ma'lum bo'lishi kerak.
                        let (w, h) = match phys {
                            Some(p) => p,
                            None => (s.stream.width, s.stream.height),
                        };
                        if w <= 0 || h <= 0 {
                            continue;
                        }
                        let ax = (f64::from(x) / 65535.0 * f64::from(w)).clamp(0.0, f64::from(w - 1));
                        let ay = (f64::from(y) / 65535.0 * f64::from(h)).clamp(0.0, f64::from(h - 1));
                        s.rd.call("NotifyPointerMotionAbsolute", &(s.stream_path.as_str(), ax, ay)).await
                    }
                    InputEvent::Button { button, pressed } => {
                        let code: i32 = match button {
                            0 => 0x110, // BTN_LEFT
                            1 => 0x111, // BTN_RIGHT
                            _ => 0x112, // BTN_MIDDLE
                        };
                        s.rd.call("NotifyPointerButton", &(code, pressed)).await
                    }
                    InputEvent::Scroll { dx, dy } => {
                        let mut r = Ok(());
                        if dy != 0 {
                            r = s.rd.call("NotifyPointerAxisDiscrete", &(0u32, i32::from(dy))).await;
                        }
                        if dx != 0 {
                            let _: zbus::Result<()> = s.rd.call("NotifyPointerAxisDiscrete", &(1u32, i32::from(dx))).await;
                        }
                        r
                    }
                    InputEvent::Key { code, pressed } => s.rd.call("NotifyKeyboardKeycode", &(code, pressed)).await,
                };
                if let Err(e) = r {
                    in_err += 1;
                    if in_err <= 10 {
                        eprintln!("[mutter] kiritish xatosi: {e}");
                    }
                }
            }
        }
    }
    if let Some(s) = sess {
        let _: zbus::Result<()> = s.rd.call("Stop", &()).await;
    }
}
