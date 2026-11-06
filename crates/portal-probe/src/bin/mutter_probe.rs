//! Go/no-go tekshiruv: Mutter'ning ichki D-Bus API'si (`org.gnome.Mutter.RemoteDesktop/ScreenCast`)
//! ekran QULFLANGANDA ham ekran oqimi va kiritishni beradimi? (PLAN.md 9.8)
//!
//! Portal'dan farqi: xdg-desktop-portal oraliqda yo'q, shuning uchun ruxsat dialogi ham yo'q.
//!
//! Foydalanish (GNOME Wayland sessiyasi ichida, oddiy foydalanuvchi sifatida):
//!   mutter-probe [--watch SEKUND] [--lock-after SEKUND] [--frames PAPKA]
//!
//!   --watch N        sessiyani N soniya ochiq ushlaydi; har 2 soniyada kursorni 1 px surib ok/xato chiqaradi
//!   --lock-after N   N soniyadan keyin o'zi `loginctl lock-session` ni ishga tushiradi
//!   --frames PAPKA   `gst-launch-1.0` bilan har soniyada bitta kadrni PAPKA/fNNN.jpg ga saqlaydi
//!                    (qulf paytida kadrlar davom etishi va qulf ekrani ko'rinishini tekshirish uchun)
//!
//! Sinov tartibi: `mutter-probe --watch 40 --lock-after 10 --frames /tmp/mp`. Natija: kiritish xatosi
//! qaysi soniyadan boshlangani, kadrlar qulfdan keyin ham yozilib turgani, qulfda sessiya `Closed` bo'lgani.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("mutter-probe faqat Linux uchun.");
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    imp::run(std::env::args().skip(1).collect()).await
}

#[cfg(target_os = "linux")]
mod imp {
    use anyhow::{anyhow, Context, Result};
    use futures_util::StreamExt;
    use std::{
        collections::HashMap,
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };
    use zbus::{
        zvariant::{ObjectPath, OwnedObjectPath, Value},
        Connection, Proxy,
    };

    const RD_DEST: &str = "org.gnome.Mutter.RemoteDesktop";
    const SC_DEST: &str = "org.gnome.Mutter.ScreenCast";

    async fn proxy<'a>(c: &'a Connection, dest: &'static str, path: &'a str, iface: &'static str) -> Result<Proxy<'a>> {
        Proxy::new(c, dest, path, iface)
            .await
            .with_context(|| format!("{iface} proksisini ochib bo'lmadi"))
    }

    pub async fn run(args: Vec<String>) -> Result<()> {
        let mut watch = 20u64;
        let mut lock_after: Option<u64> = None;
        let mut frames: Option<String> = None;
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--watch" => watch = it.next().context("--watch uchun qiymat kerak")?.parse().context("noto'g'ri son")?,
                "--lock-after" => {
                    lock_after = Some(it.next().context("--lock-after uchun qiymat kerak")?.parse().context("noto'g'ri son")?)
                }
                "--frames" => frames = Some(it.next().context("--frames uchun papka kerak")?.clone()),
                other => anyhow::bail!("noma'lum argument: {other}"),
            }
        }

        let conn = Connection::session().await.context("session D-Bus'ga ulanib bo'lmadi")?;

        // 1) RemoteDesktop sessiyasi (hali Start qilinmaydi: avval ScreenCast bog'lanadi)
        let rd = proxy(&conn, RD_DEST, "/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop").await?;
        let rd_path: OwnedObjectPath = rd
            .call("CreateSession", &())
            .await
            .context("RemoteDesktop.CreateSession xatosi (GNOME Wayland sessiyasidamisiz?)")?;
        let rd_session = proxy(&conn, RD_DEST, rd_path.as_str(), "org.gnome.Mutter.RemoteDesktop.Session").await?;
        let session_id: String = rd_session.get_property("SessionId").await.context("SessionId o'qilmadi")?;
        println!("RemoteDesktop sessiyasi: {rd_path} (id {session_id})");

        // 2) ScreenCast sessiyasi shu RemoteDesktop sessiyasiga bog'lanadi
        let sc = proxy(&conn, SC_DEST, "/org/gnome/Mutter/ScreenCast", "org.gnome.Mutter.ScreenCast").await?;
        let mut opts: HashMap<&str, Value> = HashMap::new();
        opts.insert("remote-desktop-session-id", Value::from(session_id.as_str()));
        let sc_path: OwnedObjectPath = sc.call("CreateSession", &(opts,)).await.context("ScreenCast.CreateSession xatosi")?;
        let sc_session = proxy(&conn, SC_DEST, sc_path.as_str(), "org.gnome.Mutter.ScreenCast.Session").await?;

        // Bo'sh connector = asosiy monitor. cursor-mode 1 = kursor kadrga chiziladi (Embedded).
        let mut ropts: HashMap<&str, Value> = HashMap::new();
        ropts.insert("cursor-mode", Value::from(1u32));
        let stream_path: OwnedObjectPath = sc_session
            .call("RecordMonitor", &("", ropts))
            .await
            .context("RecordMonitor xatosi")?;
        println!("Oqim obyekti: {stream_path}");
        let stream = proxy(&conn, SC_DEST, stream_path.as_str(), "org.gnome.Mutter.ScreenCast.Stream").await?;
        let mut added = stream.receive_signal("PipeWireStreamAdded").await?;
        let mut closed = sc_session.receive_signal("Closed").await?;
        let mut rd_closed = rd_session.receive_signal("Closed").await?;

        // 3) Start (RemoteDesktop.Start bog'langan ScreenCast'ni ham ishga tushiradi)
        let t0 = Instant::now();
        let () = rd_session.call("Start", &()).await.context("RemoteDesktop.Session.Start xatosi")?;
        let node: u32 = tokio::time::timeout(Duration::from_secs(5), added.next())
            .await
            .context("PipeWireStreamAdded 5 soniyada kelmadi")?
            .ok_or_else(|| anyhow!("signal oqimi yopildi"))?
            .body()
            .deserialize()
            .context("PipeWireStreamAdded o'qilmadi")?;
        println!("Sessiya ochildi ({:.2} s), PipeWire node {node}", t0.elapsed().as_secs_f64());

        let mut gst: Option<Child> = None;
        if let Some(dir) = &frames {
            std::fs::create_dir_all(dir).ok();
            let child = Command::new("gst-launch-1.0")
                .args([
                    "-q",
                    "pipewiresrc",
                    &format!("path={node}"),
                    "!",
                    "videoconvert",
                    "!",
                    "videorate",
                    "!",
                    "video/x-raw,framerate=1/1",
                    "!",
                    "jpegenc",
                    "!",
                    "multifilesink",
                    &format!("location={dir}/f%03d.jpg"),
                ])
                .stdout(Stdio::null())
                .spawn()
                .context("gst-launch-1.0 ishga tushmadi (gstreamer1.0-tools va gstreamer1.0-pipewire kerak)")?;
            println!("Kadrlar {dir}/fNNN.jpg ga yoziladi (soniyada 1 ta)");
            gst = Some(child);
        }

        let stream_p = ObjectPath::try_from(stream_path.as_str())?;
        let start = Instant::now();
        let mut locked = false;
        let mut first_err: Option<u64> = None;
        println!("Kuzatuv: {watch} soniya");
        while start.elapsed() < Duration::from_secs(watch) {
            let t = start.elapsed().as_secs();
            if let (Some(l), false) = (lock_after, locked) {
                if t >= l {
                    locked = true;
                    println!("t={t:>3}s: `loginctl lock-session` ishga tushirilmoqda");
                    let _ = Command::new("loginctl").arg("lock-session").status();
                }
            }
            // Mutlaq harakat: oqim obyekti yo'li bilan; keyin 1 px nisbiy
            let a: zbus::Result<()> = rd_session
                .call("NotifyPointerMotionAbsolute", &(&stream_p, 100.0f64, 100.0f64))
                .await;
            let b: zbus::Result<()> = rd_session.call("NotifyPointerMotionRelative", &(1.0f64, 0.0f64)).await;
            match (a, b) {
                (Ok(()), Ok(())) => println!("t={t:>3}s: kiritish ok"),
                (Err(e), _) | (_, Err(e)) => {
                    first_err.get_or_insert(t);
                    println!("t={t:>3}s: kiritish XATO: {e}");
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                s = closed.next() => { if s.is_some() { println!("t={:>3}s: ScreenCast sessiyasi YOPILDI (Closed)", start.elapsed().as_secs()); } }
                s = rd_closed.next() => { if s.is_some() { println!("t={:>3}s: RemoteDesktop sessiyasi YOPILDI (Closed)", start.elapsed().as_secs()); } }
            }
        }

        if let Some(mut c) = gst.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _: zbus::Result<()> = rd_session.call("Stop", &()).await;
        println!(
            "NATIJA: {}",
            match (lock_after, first_err) {
                (Some(l), None) => format!("qulf t={l}s da berildi, kiritish xatosiz davom etdi (kadrlarni tekshiring)"),
                (Some(l), Some(e)) => format!("qulf t={l}s da berildi, kiritish xatosi t={e}s dan boshlandi"),
                (None, None) => "kiritish xatosiz ishladi".to_string(),
                (None, Some(e)) => format!("kiritish xatosi t={e}s dan boshlandi (qulfsiz!)"),
            }
        );
        Ok(())
    }
}
