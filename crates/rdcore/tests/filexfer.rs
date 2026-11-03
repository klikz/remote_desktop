//! Fayl almashinuvi: Viewer Host'ga fayl yuboradi, ro'yxatni oladi va qaytarib yuklab oladi (127.0.0.1).

use protocol::{Capabilities, CODEC_H264};
use rdcore::{
    auth::Credentials,
    cert::HostIdentity,
    client::{connect, ConnectOptions},
    filexfer,
    known_hosts::KnownHosts,
    server::{Handler, Host},
};
use std::sync::Arc;

const PASSWORD: &str = "test-password-123";

#[tokio::test(flavor = "multi_thread")]
async fn upload_list_download() {
    let dir = std::env::temp_dir().join(format!("rd-filexfer-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("RD_SHARED_DIR", dir.join("shared"));

    let identity = HostIdentity::generate().unwrap();
    let creds = Credentials::from_password(PASSWORD).unwrap();
    let caps = Capabilities { codecs: CODEC_H264, max_fps: 60, monitors: vec![] };
    let host = Host::bind("127.0.0.1:0".parse().unwrap(), &identity, creds, caps).unwrap();
    let addr = host.local_addr().unwrap();
    let handler: Handler = Arc::new(|io| {
        Box::pin(async move {
            let files = filexfer::spawn_server(io.conn.clone());
            rdcore::server::default_session(io).await;
            files.abort();
        })
    });
    tokio::spawn(host.run_with(handler));

    let kh = KnownHosts::new(dir.join("known_hosts"));
    let session = connect(ConnectOptions { addr, password: PASSWORD, known_hosts: &kh, confirm_new_host: &|_| true })
        .await
        .unwrap();
    let conn = session.connection();

    // 300 000 bayt: bir necha bo'lak (64 KiB) bo'lib o'tadi
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let src = dir.join("hisobot.bin");
    std::fs::write(&src, &data).unwrap();

    let mut last = (0, 0);
    filexfer::upload(&conn, &src, &mut |d, t| last = (d, t)).await.unwrap();
    assert_eq!(last, (300_000, 300_000));
    assert_eq!(std::fs::read(dir.join("shared/hisobot.bin")).unwrap(), data);

    let list = filexfer::list(&conn).await.unwrap();
    assert_eq!(list, vec![filexfer::RemoteFile { name: "hisobot.bin".into(), size: 300_000 }]);

    // Mavjud nom ustiga yozilmaydi: "hisobot (1).bin" bo'ladi
    let saved = filexfer::download(&conn, "hisobot.bin", &mut |_, _| {}).await.unwrap();
    assert_eq!(saved.file_name().unwrap(), "hisobot (1).bin");
    assert_eq!(std::fs::read(&saved).unwrap(), data);

    // Yo'l chiqib ketish va mavjud bo'lmagan fayl rad etiladi
    assert!(filexfer::download(&conn, "../known_hosts", &mut |_, _| {}).await.is_err());
    let err = filexfer::download(&conn, "yoq.txt", &mut |_, _| {}).await.unwrap_err();
    assert!(format!("{err:#}").contains("topilmadi"), "{err:#}");

    // Bo'sh fayl ham o'tadi
    let empty = dir.join("bosh.txt");
    std::fs::write(&empty, b"").unwrap();
    filexfer::upload(&conn, &empty, &mut |_, _| {}).await.unwrap();
    assert!(dir.join("shared/bosh.txt").exists());

    session.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}
