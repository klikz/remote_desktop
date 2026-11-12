//! Bitta kompyuterda Host va Viewer o'rtasidagi to'liq oqim testlari (127.0.0.1).

use protocol::{Capabilities, MonitorInfo, CODEC_H264};
use rdcore::{
    auth::Credentials,
    cert::HostIdentity,
    client::{connect, ConnectOptions},
    server::Host,
};
use std::net::SocketAddr;

const PASSWORD: &str = "test-password-123";

fn test_caps() -> Capabilities {
    Capabilities {
        codecs: CODEC_H264,
        max_fps: 60,
        monitors: vec![MonitorInfo { id: 0, width: 1920, height: 1080, primary: true }],
    }
}

/// Host'ni 127.0.0.1:0 da ishga tushiradi va manzilini qaytaradi.
async fn start_host() -> SocketAddr {
    let identity = HostIdentity::generate().unwrap();
    let creds = Credentials::from_password(PASSWORD).unwrap();
    let host = Host::bind("127.0.0.1:0".parse().unwrap(), &identity, creds, test_caps()).unwrap();
    let addr = host.local_addr().unwrap();
    tokio::spawn(host.run());
    addr
}

#[tokio::test(flavor = "multi_thread")]
async fn connect_auth_and_ping() {
    let addr = start_host().await;

    let mut session = connect(ConnectOptions {
        addr,
        password: PASSWORD,
    })
    .await
    .expect("ulanish muvaffaqiyatli bo'lishi kerak");

    assert_eq!(session.host_caps, test_caps());
    for _ in 0..3 {
        let rtt = session.ping().await.expect("ping");
        assert!(rtt.as_secs() < 2);
    }
    session.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_is_rejected() {
    let addr = start_host().await;
    let res = connect(ConnectOptions {
        addr,
        password: "definitely-wrong-password",
    })
    .await;
    let err = res.err().expect("noto'g'ri kalit rad etilishi kerak");
    assert!(format!("{err:#}").contains("noto'g'ri kalit"), "xato matni: {err:#}");
    // Mobil klient shu belgi bo'yicha qayta ulanishni to'xtatadi (kalit noto'g'ri bo'lsa urinish befoyda)
    assert!(err.downcast_ref::<rdcore::client::Rejected>().is_some(), "Rejected kutilgan edi: {err:#}");
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_failures_get_blocked() {
    let addr = start_host().await;
    for _ in 0..5 {
        let res = connect(ConnectOptions {
            addr,
            password: "definitely-wrong-password",
        })
        .await;
        assert!(res.is_err());
    }
    // 5 ta xatodan keyin to'g'ri parol bilan ham ulanib bo'lmaydi (IP bloklangan).
    let res = connect(ConnectOptions {
        addr,
        password: PASSWORD,
    })
    .await;
    assert!(res.is_err(), "bloklangan IP ulana olmasligi kerak");
}

/// v2 xabarlari (Feedback, RequestKeyframe) va bitta yozuvga birlashtirilgan kiritish haqiqiy QUIC oqimida
/// to'g'ri ajratiladi: keyingi ping javobi buzilmagan bo'lishi kerak. RTT QUIC'dan olinadi.
#[tokio::test(flavor = "multi_thread")]
async fn batched_input_and_feedback_keep_stream_intact() {
    use protocol::InputEvent;
    let addr = start_host().await;
    let mut session = connect(ConnectOptions { addr, password: PASSWORD })
        .await
        .expect("ulanish");

    let mut batch: Vec<InputEvent> = (0..500u16).map(|i| InputEvent::PointerMove { x: i, y: i }).collect();
    batch.push(InputEvent::Button { button: 0, pressed: true });
    batch.push(InputEvent::Key { code: 30, pressed: true });
    session.send_inputs(batch).await.expect("kiritish");
    session.send_feedback(123_456, 2).await.expect("feedback");
    session.request_keyframe().await.expect("kalit kadr so'rovi");
    for _ in 0..3 {
        session.ping().await.expect("oqim buzilmagan bo'lishi kerak");
    }
    assert!(session.rtt().as_secs() < 2);
    session.close().await;
}
