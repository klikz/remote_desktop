//! Bitta kompyuterda Host va Viewer o'rtasidagi to'liq oqim testlari (127.0.0.1).

use protocol::{Capabilities, MonitorInfo, CODEC_H264};
use rdcore::{
    auth::Credentials,
    cert::HostIdentity,
    client::{connect, ConnectOptions},
    hexfmt,
    known_hosts::KnownHosts,
    server::Host,
};
use std::{net::SocketAddr, path::PathBuf, sync::atomic::{AtomicUsize, Ordering}};

const PASSWORD: &str = "test-password-123";

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rd-loop-{}-{}-{}", name, std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn test_caps() -> Capabilities {
    Capabilities {
        codecs: CODEC_H264,
        max_fps: 60,
        monitors: vec![MonitorInfo { id: 0, width: 1920, height: 1080, primary: true }],
    }
}

/// Host'ni 127.0.0.1:0 da ishga tushiradi. (manzil, fingerprint) qaytaradi.
async fn start_host() -> (SocketAddr, [u8; 32]) {
    let identity = HostIdentity::generate().unwrap();
    let creds = Credentials::from_password(PASSWORD).unwrap();
    let host = Host::bind("127.0.0.1:0".parse().unwrap(), &identity, creds, test_caps()).unwrap();
    let addr = host.local_addr().unwrap();
    let fp = host.fingerprint();
    tokio::spawn(host.run());
    (addr, fp)
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_auth_and_ping() {
    let (addr, fp) = start_host().await;
    let kh = KnownHosts::new(temp_dir("ok").join("known_hosts"));

    let mut session = connect(ConnectOptions {
        addr,
        password: PASSWORD,
        known_hosts: &kh,
        confirm_new_host: &|_| true,
    })
    .await
    .expect("ulanish muvaffaqiyatli bo'lishi kerak");

    assert_eq!(session.fingerprint, hexfmt::colon(&fp));
    assert_eq!(session.host_caps, test_caps());
    for _ in 0..3 {
        let rtt = session.ping().await.expect("ping");
        assert!(rtt.as_secs() < 2);
    }
    session.close().await;

    // Ikkinchi ulanishda fingerprint allaqachon saqlangan: tasdiq so'ralmaydi.
    let session2 = connect(ConnectOptions {
        addr,
        password: PASSWORD,
        known_hosts: &kh,
        confirm_new_host: &|_| panic!("saqlangan Host uchun tasdiq so'ralmasligi kerak"),
    })
    .await
    .expect("ikkinchi ulanish");
    session2.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_is_rejected() {
    let (addr, _) = start_host().await;
    let kh = KnownHosts::new(temp_dir("badpw").join("known_hosts"));
    let res = connect(ConnectOptions {
        addr,
        password: "definitely-wrong-password",
        known_hosts: &kh,
        confirm_new_host: &|_| true,
    })
    .await;
    let err = res.err().expect("noto'g'ri parol rad etilishi kerak");
    assert!(format!("{err:#}").contains("noto'g'ri parol"), "xato matni: {err:#}");
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_failures_get_blocked() {
    let (addr, _) = start_host().await;
    let kh = KnownHosts::new(temp_dir("block").join("known_hosts"));
    for _ in 0..5 {
        let res = connect(ConnectOptions {
            addr,
            password: "definitely-wrong-password",
            known_hosts: &kh,
            confirm_new_host: &|_| true,
        })
        .await;
        assert!(res.is_err());
    }
    // 5 ta xatodan keyin to'g'ri parol bilan ham ulanib bo'lmaydi (IP bloklangan).
    let res = connect(ConnectOptions {
        addr,
        password: PASSWORD,
        known_hosts: &kh,
        confirm_new_host: &|_| true,
    })
    .await;
    assert!(res.is_err(), "bloklangan IP ulana olmasligi kerak");
}

#[tokio::test(flavor = "multi_thread")]
async fn unconfirmed_new_host_is_refused() {
    let (addr, _) = start_host().await;
    let kh = KnownHosts::new(temp_dir("noconfirm").join("known_hosts"));
    let res = connect(ConnectOptions {
        addr,
        password: PASSWORD,
        known_hosts: &kh,
        confirm_new_host: &|_| false,
    })
    .await;
    assert!(res.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn changed_fingerprint_is_refused() {
    let (addr, _) = start_host().await;
    let kh = KnownHosts::new(temp_dir("changed").join("known_hosts"));
    // Bu manzil uchun boshqa fingerprint saqlangan (masalan, vositachi sertifikati).
    kh.add(&addr.to_string(), "00:11:22:33").unwrap();
    let res = connect(ConnectOptions {
        addr,
        password: PASSWORD,
        known_hosts: &kh,
        confirm_new_host: &|_| true,
    })
    .await;
    let err = res.err().expect("o'zgargan fingerprint rad etilishi kerak");
    assert!(format!("{err:#}").contains("fingerprint"), "xato matni: {err:#}");
}
