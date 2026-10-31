//! Relay rejimi: server, ikki client (agent) va Host bitta kompyuterda (127.0.0.1).
//! `RD_CONFIG_DIR` butun jarayon uchun o'rnatiladi, shuning uchun hamma tekshiruv bitta testda ketma-ket.

use protocol::{Capabilities, CODEC_H264};
use rdcore::{
    auth::Credentials,
    cert::HostIdentity,
    client::{connect_relayed, ConnectOptions},
    known_hosts::KnownHosts,
    paths,
    relay::{
        agent::{Agent, AgentConfig, AgentEvent, Policy, Relayed},
        server::RelayServer,
    },
    server::Host,
};
use std::time::Duration;
use tokio::time::{sleep, timeout};

const HOST_PW: &str = "host-password-123";
const SERVER_KEY: &str = "server-key-12345";

fn caps() -> Capabilities {
    Capabilities { codecs: CODEC_H264, max_fps: 60, monitors: vec![] }
}

/// Kutilgan hodisa kelguncha (5 s gacha) kutadi.
async fn wait_event<T>(agent: &Agent, mut f: impl FnMut(AgentEvent) -> Option<T>) -> T {
    timeout(Duration::from_secs(8), async {
        loop {
            while let Some(e) = agent.try_event() {
                if let Some(v) = f(e) {
                    return v;
                }
            }
            sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("kutilgan hodisa kelmadi")
}

fn cfg(server: &str, key: &str, name: &str, dir: &std::path::Path, host_port: Option<u16>) -> AgentConfig {
    AgentConfig {
        server: server.to_string(),
        key: key.to_string(),
        name: name.to_string(),
        id: String::new(),
        token: String::new(),
        known_hosts: dir.join(format!("kh-{name}")),
        host_port,
    }
}

async fn registered(a: &Agent) -> String {
    wait_event(a, |e| match e {
        AgentEvent::Registered { id, .. } => Some(id),
        _ => None,
    })
    .await
}

async fn ping_through(relayed: Relayed, target_id: &str, password: &str, dir: &std::path::Path) {
    let kh = KnownHosts::new(dir.join("kh-viewer"));
    let mut s = connect_relayed(
        ConnectOptions { addr: relayed.local, password, known_hosts: &kh, confirm_new_host: &|_| true },
        format!("id:{target_id}"),
    )
    .await
    .expect("relay orqali ulanish");
    for _ in 0..3 {
        s.ping().await.expect("ping");
    }
    s.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_end_to_end() {
    let dir = std::env::temp_dir().join(format!("rd-relay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("RD_CONFIG_DIR", &dir);

    // B kompyuteridagi Host (doimiy parol faylda, bir martalik parol shu tuz bilan olinadi)
    Credentials::from_password(HOST_PW).unwrap().save(&paths::host_password_file().unwrap()).unwrap();
    let creds = Credentials::load(&paths::host_password_file().unwrap()).unwrap();
    let host = Host::bind("127.0.0.1:0".parse().unwrap(), &HostIdentity::generate().unwrap(), creds, caps()).unwrap();
    let host_port = host.local_addr().unwrap().port();
    tokio::spawn(host.run());

    // Relay serveri
    let srv_creds = Credentials::from_password(SERVER_KEY).unwrap();
    let server = RelayServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        &HostIdentity::generate().unwrap(),
        Some(srv_creds),
        Some(dir.join("registry.txt")),
    )
    .unwrap();
    let server_addr = format!("127.0.0.1:{}", server.local_addr().unwrap().port());
    let handle = server.handle();
    tokio::spawn(server.run());

    // Noto'g'ri server kaliti rad etiladi
    let bad = Agent::start(cfg(&server_addr, "wrong-key-12345", "bad", &dir, None), Policy::default());
    wait_event(&bad, |e| match e {
        AgentEvent::Status { text, .. } if text.contains("rad etdi") => Some(()),
        _ => None,
    })
    .await;
    drop(bad);

    // A (ulanuvchi) va B (qabul qiluvchi)
    let a = Agent::start(cfg(&server_addr, SERVER_KEY, "A", &dir, None), Policy::default());
    let b = Agent::start(cfg(&server_addr, SERVER_KEY, "B", &dir, Some(host_port)), Policy::default());
    let (a_id, b_id) = (registered(&a).await, registered(&b).await);
    assert_ne!(a_id, b_id);
    assert!(rdcore::relay::valid_id(&a_id));
    assert_eq!(handle.clients().iter().filter(|c| c.online).count(), 2);

    // Onlayn holat obunasi
    a.subscribe(vec![b_id.clone()]);
    wait_event(&a, |e| match e {
        AgentEvent::Presence { id, online: true, .. } if id == b_id => Some(()),
        _ => None,
    })
    .await;

    // 1) Dialogli rejim: B qabul qiladi, bir martalik parol bilan ulanadi
    let rx = a.connect(&b_id);
    let (sid, from) = wait_event(&b, |e| match e {
        AgentEvent::Incoming { sid, from, .. } => Some((sid, from)),
        _ => None,
    })
    .await;
    assert_eq!(from, a_id);
    b.reply(&sid, true);
    let relayed = timeout(Duration::from_secs(8), rx).await.unwrap().unwrap().expect("ruxsat berilishi kerak");
    let pw = relayed.pw.clone().expect("dialogli rejimda bir martalik parol keladi");
    ping_through(relayed, &b_id, &pw, &dir).await;

    // Bir martalik parol bir marta ishlaydi: ikkinchi urinish rad etiladi
    let rx = a.connect(&b_id);
    let (sid, _) = wait_event(&b, |e| match e {
        AgentEvent::Incoming { sid, from, .. } => Some((sid, from)),
        _ => None,
    })
    .await;
    b.reply(&sid, true);
    let relayed = timeout(Duration::from_secs(8), rx).await.unwrap().unwrap().unwrap();
    let kh = KnownHosts::new(dir.join("kh-viewer"));
    let again = connect_relayed(
        ConnectOptions { addr: relayed.local, password: &pw, known_hosts: &kh, confirm_new_host: &|_| true },
        format!("id:{b_id}"),
    )
    .await;
    assert!(again.is_err(), "eski bir martalik parol ishlamasligi kerak");

    // 2) Rad etish
    let rx = a.connect(&b_id);
    let (sid, _) = wait_event(&b, |e| match e {
        AgentEvent::Incoming { sid, from, .. } => Some((sid, from)),
        _ => None,
    })
    .await;
    b.reply(&sid, false);
    let err = timeout(Duration::from_secs(8), rx).await.unwrap().unwrap().unwrap_err();
    assert!(err.contains("ruxsat berilmadi"), "{err}");

    // 3) Qarovsiz rejim: dialogsiz, Host'ning doimiy paroli bilan
    b.policy.lock().unwrap().unattended = true;
    let relayed = timeout(Duration::from_secs(8), a.connect(&b_id)).await.unwrap().unwrap().expect("qarovsiz qabul");
    assert!(relayed.pw.is_none());
    ping_through(relayed, &b_id, HOST_PW, &dir).await;

    // 4) Yo'q ID va oflayn
    let err = a.connect("999999999").await.unwrap().unwrap_err();
    assert!(err.contains("bunday ID yo'q"), "{err}");
    let err = a.connect(&a_id).await.unwrap().unwrap_err();
    assert!(err.contains("o'zingizga"), "{err}");

    drop(b);
    wait_event(&a, |e| match e {
        AgentEvent::Presence { id, online: false, .. } if id == b_id => Some(()),
        _ => None,
    })
    .await;
    let err = a.connect(&b_id).await.unwrap().unwrap_err();
    assert!(err.contains("oflayn"), "{err}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Ochiq (kalitsiz) server: bo'sh kalit bilan ulanadi; kalitli server bo'sh kalitni rad etadi;
/// yo'q ID ga ketma-ket urinishlar vaqtincha bloklanadi.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn open_server_and_key_policy() {
    let dir = std::env::temp_dir().join(format!("rd-relay-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let start = |creds: Option<Credentials>, reg: &str| {
        let server = RelayServer::bind(
            "127.0.0.1:0".parse().unwrap(),
            &HostIdentity::generate().unwrap(),
            creds,
            Some(dir.join(reg)),
        )
        .unwrap();
        let addr = format!("127.0.0.1:{}", server.local_addr().unwrap().port());
        tokio::spawn(server.run());
        addr
    };

    // Ochiq server: kalit bo'sh
    let open = start(None, "open.txt");
    let a = Agent::start(cfg(&open, "", "A", &dir, None), Policy::default());
    let b = Agent::start(cfg(&open, "ixtiyoriy", "B", &dir, None), Policy::default());
    let (_a_id, _b_id) = (registered(&a).await, registered(&b).await);

    // Yo'q ID ga 10 marta urinish: keyin bloklanadi
    for _ in 0..10 {
        let err = a.connect("999999999").await.unwrap().unwrap_err();
        assert!(err.contains("bunday ID yo'q"), "{err}");
    }
    let err = a.connect("999999999").await.unwrap().unwrap_err();
    assert!(err.contains("juda ko'p"), "{err}");

    // Kalitli server bo'sh kalitni rad etadi
    let keyed = start(Some(Credentials::from_password(SERVER_KEY).unwrap()), "keyed.txt");
    let c = Agent::start(cfg(&keyed, "", "C", &dir, None), Policy::default());
    wait_event(&c, |e| match e {
        AgentEvent::Status { text, .. } if text.contains("kalit talab qiladi") => Some(()),
        _ => None,
    })
    .await;

    let _ = std::fs::remove_dir_all(&dir);
}
