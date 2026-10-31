//! Bitta ulanish: relay orqali ulanish, video kadrlarni qabul qilish va kiritishni yuborish.
//! `crates/viewer/src/gui/session.rs` mantig'i, lekin egui/openh264 siz.

use anyhow::Result;
use protocol::InputEvent;
use rdcore::{
    client::{connect_relayed, ConnectOptions},
    known_hosts::KnownHosts,
    paths,
    relay::agent::Connector,
};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, TrySendError},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Clone)]
pub struct Params {
    pub id: String,
    pub password: String,
    pub fps: u16,
    pub kbps: u32,
    pub height: u16,
    pub connector: Connector,
}

/// Kadr: kalit-kadr belgisi + Annex-B H.264 baytlari.
pub struct Frame {
    pub keyframe: bool,
    pub data: Vec<u8>,
}

#[derive(Default)]
pub struct Stats {
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    pub rtt_us: AtomicU64,
}

pub enum Event {
    /// Birinchi ulanish: fingerprint'ni foydalanuvchi tasdiqlashi kerak (`Sess::answer_confirm`).
    Confirm(String),
    Connected { fingerprint: String, via_tcp: bool },
    VideoStarted,
    /// Ulanish tugadi. `Some` = xato matni.
    Ended(Option<String>),
}

enum Cmd {
    Input(InputEvent),
    Close,
}

pub struct Sess {
    cmd: UnboundedSender<Cmd>,
    events: Mutex<mpsc::Receiver<Event>>,
    frames: Mutex<mpsc::Receiver<Frame>>,
    confirm: Arc<Mutex<Option<mpsc::Sender<bool>>>>,
    pub stats: Arc<Stats>,
}

impl Sess {
    pub fn input(&self, ev: InputEvent) {
        let _ = self.cmd.send(Cmd::Input(ev));
    }

    pub fn close(&self) {
        let _ = self.cmd.send(Cmd::Close);
    }

    pub fn poll_event(&self) -> Option<Event> {
        self.events.lock().ok()?.try_recv().ok()
    }

    /// Kadr kutadi (bloklaydi, eng ko'pi bilan `timeout`).
    pub fn next_frame(&self, timeout: Duration) -> Option<Frame> {
        self.frames.lock().ok()?.recv_timeout(timeout).ok()
    }

    pub fn answer_confirm(&self, accept: bool) {
        if let Some(tx) = self.confirm.lock().ok().and_then(|mut c| c.take()) {
            let _ = tx.send(accept);
        }
    }
}

impl Drop for Sess {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Close);
        // Kutayotgan fingerprint so'rovi bo'lsa, rad etiladi (oqim osilib qolmasligi uchun).
        self.answer_confirm(false);
    }
}

pub fn start(p: Params) -> Arc<Sess> {
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::channel();
    let (frame_tx, frame_rx) = mpsc::sync_channel::<Frame>(8);
    let stats = Arc::new(Stats::default());
    let confirm = Arc::new(Mutex::new(None));
    let sess = Arc::new(Sess {
        cmd: cmd_tx,
        events: Mutex::new(ev_rx),
        frames: Mutex::new(frame_rx),
        confirm: confirm.clone(),
        stats: stats.clone(),
    });
    std::thread::spawn(move || {
        let ended = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(rt) => rt.block_on(run(p, stats, ev_tx.clone(), confirm, frame_tx, cmd_rx)),
            Err(e) => Err(e.into()),
        };
        let msg = match ended {
            Ok(m) => m,
            Err(e) => Some(format!("{e:#}")),
        };
        let _ = ev_tx.send(Event::Ended(msg));
    });
    sess
}

async fn run(
    p: Params,
    stats: Arc<Stats>,
    ev_tx: mpsc::Sender<Event>,
    confirm_slot: Arc<Mutex<Option<mpsc::Sender<bool>>>>,
    frame_tx: mpsc::SyncSender<Frame>,
    mut cmd_rx: UnboundedReceiver<Cmd>,
) -> Result<Option<String>> {
    let known_hosts = KnownHosts::new(paths::known_hosts_file()?);

    // Fingerprint tasdig'i: UI'ga so'rov yuboriladi va javob kutiladi (bu oqim faqat shu ulanishniki).
    let confirm_tx = ev_tx.clone();
    let confirm = move |fp: &str| -> bool {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut slot) = confirm_slot.lock() {
            *slot = Some(tx);
        }
        if confirm_tx.send(Event::Confirm(fp.to_string())).is_err() {
            return false;
        }
        rx.recv().unwrap_or(false)
    };

    // Server qabul qiluvchidan ruxsat so'raydi (u 30 soniya o'ylaydi)
    // Parol kiritilmagan bo'lsa: qarovsiz Host ham ruxsat oynasini ko'rsatadi (xato emas)
    let relayed = p
        .connector
        .connect_ask(&p.id, p.password.is_empty())
        .await
        .map_err(|_| anyhow::anyhow!("relay agenti to'xtagan"))?
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let password = relayed.pw.clone().unwrap_or_else(|| p.password.clone());
    if password.is_empty() {
        anyhow::bail!("Host kalit bermadi: kalitni kiriting");
    }
    let mut session = connect_relayed(
        ConnectOptions { addr: relayed.local, password: &password, known_hosts: &known_hosts, confirm_new_host: &confirm },
        format!("id:{}", p.id),
    )
    .await?;
    let _ = ev_tx.send(Event::Connected { fingerprint: session.fingerprint.clone(), via_tcp: session.via_tcp });

    let mut stream = session.request_video(p.fps, p.kbps, p.height, 0).await?;
    let _ = ev_tx.send(Event::VideoStarted);

    // next_frame() ni timeout/select bilan uzib bo'lmaydi (oqim buziladi): alohida vazifa, kerak bo'lsa abort.
    let rx_stats = stats.clone();
    let mut recv_task = tokio::spawn(async move {
        let mut need_key = false;
        loop {
            match stream.next_frame().await {
                Ok(Some(f)) => {
                    rx_stats.frames.fetch_add(1, Ordering::Relaxed);
                    rx_stats.bytes.fetch_add(f.data.len() as u64, Ordering::Relaxed);
                    if need_key && !f.keyframe {
                        continue;
                    }
                    match frame_tx.try_send(Frame { keyframe: f.keyframe, data: f.data }) {
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
                Some(Cmd::Close) | None => break None,
            },
            _ = ping.tick() => match session.ping().await {
                Ok(d) => stats.rtt_us.store(d.as_micros() as u64, Ordering::Relaxed),
                Err(e) => break Some(format!("aloqa uzildi: {e:#}")),
            },
            r = &mut recv_task => break r.unwrap_or_else(|_| Some("video vazifasi to'xtadi".to_string())),
        }
    };
    recv_task.abort();
    session.close().await;
    Ok(end)
}
