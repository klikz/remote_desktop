//! QUIC (UDP) paketlarini TCP orqali tashish: UDP bloklangan tarmoqlar uchun zaxira yo'l.
//!
//! Mavjud QUIC ulanishiga tegilmaydi. Shifrlash, fingerprint tekshiruvi va parol autentifikatsiyasi
//! avvalgidek QUIC ichida bajariladi, TCP esa faqat tayyor (shifrlangan) datagrammalarni tashiydi.
//!
//! ```text
//! Viewer: quinn -> 127.0.0.1:P (UDP) -> [TCP] -> Host TCP port -> 127.x.y.z (UDP) -> Host QUIC portiga
//! ```
//!
//! TCP oqimi formati: boshida 4 bayt `RDT1`, so'ng har datagramma uchun `u16` (big-endian) uzunlik + baytlar.
//!
//! Host QUIC'dagi ulanish chastotasi cheklovi (`RateLimiter`) IP bo'yicha ishlaydi. TCP orqali kelganlar
//! uchun har bir haqiqiy IP o'zining doimiy `127.x.y.z` manziliga xaritalanadi, shunda cheklov har bir
//! tashqi IP uchun alohida ishlaydi. Haqiqiy manzilni `resolve` qaytaradi.

use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    task::JoinHandle,
    time::{sleep, timeout},
};

const MAGIC: &[u8; 4] = b"RDT1";
/// Shu vaqt davomida hech narsa kelmasa, tunnel yopiladi (QUIC keepalive har 5 soniyada).
const IDLE: Duration = Duration::from_secs(45);
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TUNNELS: usize = 64;
const MAX_PER_IP: usize = 4;

/// `127.x.y.z` (tunnel) manzil -> haqiqiy Viewer IP'si.
fn registry() -> &'static Mutex<HashMap<IpAddr, IpAddr>> {
    static REG: OnceLock<Mutex<HashMap<IpAddr, IpAddr>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Sessiyadagi `peer` manzilidan haqiqiy IP'ni va TCP tunnel orqali kelgan-kelmaganini aniqlaydi.
pub fn resolve(peer: IpAddr) -> (IpAddr, bool) {
    match registry().lock().unwrap_or_else(|e| e.into_inner()).get(&peer) {
        Some(real) => (*real, true),
        None => (peer, false),
    }
}

fn loopback_for(real: IpAddr) -> Ipv4Addr {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    real.hash(&mut h);
    let v = h.finish();
    Ipv4Addr::new(127, ((v >> 8) & 0xff) as u8, ((v >> 16) & 0xff) as u8, 2 + ((v >> 24) % 252) as u8)
}

/// Datagrammalarni UDP soketi va TCP oqimi o'rtasida ikki tomonlama uzatadi.
/// `learn`: UDP tomondagi manzilni oxirgi kelgan paketdan o'rganadi (Viewer tomoni: quinn manzili).
async fn pump(tcp: TcpStream, udp: UdpSocket, dst: Option<SocketAddr>, learn: bool) {
    let _ = tcp.set_nodelay(true);
    let (rd, wr) = tcp.into_split();
    pump_io(rd, wr, udp, dst, learn).await
}

/// `pump` har qanday oqim juftligi uchun (TCP yoki relay serveriga ochilgan QUIC oqimi).
pub(crate) async fn pump_io<R, W>(mut rd: R, mut wr: W, udp: UdpSocket, dst: Option<SocketAddr>, learn: bool)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let udp = Arc::new(udp);
    let dst = Arc::new(Mutex::new(dst));

    let (u, d) = (udp.clone(), dst.clone());
    let mut up = tokio::spawn(async move {
        let mut buf = vec![0u8; 65536];
        loop {
            match timeout(IDLE, u.recv_from(&mut buf)).await {
                Err(_) => break,
                // Windows: yopilgan UDP portdan ICMP xatosi kelsa, recv_from shu xatoni qaytaradi.
                Ok(Err(e)) if e.kind() == ErrorKind::ConnectionReset => continue,
                Ok(Err(_)) => break,
                Ok(Ok((n, src))) => {
                    if learn {
                        *d.lock().unwrap_or_else(|e| e.into_inner()) = Some(src);
                    }
                    let mut frame = Vec::with_capacity(n + 2);
                    frame.extend_from_slice(&(n as u16).to_be_bytes());
                    frame.extend_from_slice(&buf[..n]);
                    if wr.write_all(&frame).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut down = tokio::spawn(async move {
        let mut hdr = [0u8; 2];
        let mut data = vec![0u8; 65536];
        loop {
            // Vaqt tugasa yoki xato bo'lsa sikl tugaydi (to'liq o'qilmagan holat keyin ishlatilmaydi).
            match timeout(IDLE, rd.read_exact(&mut hdr)).await {
                Ok(Ok(_)) => {}
                _ => break,
            }
            let n = u16::from_be_bytes(hdr) as usize;
            if rd.read_exact(&mut data[..n]).await.is_err() {
                break;
            }
            let target = *dst.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = target {
                let _ = udp.send_to(&data[..n], t).await;
            }
        }
    });

    tokio::select! {
        _ = &mut up => {}
        _ = &mut down => {}
    }
    up.abort();
    down.abort();
}

// ---------------------------------------------------------------------------------------------
// Viewer tomoni
// ---------------------------------------------------------------------------------------------

/// Host'ning TCP portiga ulanadi va lokal UDP manzilni qaytaradi: quinn'ni shu manzilga ulash kerak.
pub async fn connect(remote: SocketAddr) -> Result<SocketAddr> {
    let mut tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect(remote))
        .await
        .map_err(|_| anyhow::anyhow!("{remote} ga TCP ulanish vaqti tugadi (Host ishlayaptimi, TCP port ochiqmi?)"))?
        .with_context(|| format!("{remote} ga TCP orqali ulanib bo'lmadi"))?;
    tcp.write_all(MAGIC).await.context("TCP tunnel salomi yuborilmadi")?;
    let udp = UdpSocket::bind("127.0.0.1:0").await.context("lokal UDP portni ochib bo'lmadi")?;
    let local = udp.local_addr()?;
    tokio::spawn(pump(tcp, udp, None, true));
    Ok(local)
}

// ---------------------------------------------------------------------------------------------
// Host tomoni
// ---------------------------------------------------------------------------------------------

/// Ishlayotgan TCP tunnel serveri. `Drop` qilinsa to'xtaydi.
pub struct TunnelServer {
    task: JoinHandle<()>,
}

impl Drop for TunnelServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// `listen` manzilida TCP port ochadi; kelgan datagrammalar `host_udp` (Host'ning QUIC porti) ga uzatiladi.
/// Tokio runtime ichida chaqirilishi kerak.
pub fn spawn_server(listen: SocketAddr, host_udp: SocketAddr) -> Result<TunnelServer> {
    let std_listener = std::net::TcpListener::bind(listen).with_context(|| format!("TCP {listen} ni tinglab bo'lmadi"))?;
    std_listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(std_listener)?;
    // 0.0.0.0 ga ulangan Host'ga lokal manzil orqali boriladi.
    let target = if host_udp.ip().is_unspecified() {
        SocketAddr::new(Ipv4Addr::LOCALHOST.into(), host_udp.port())
    } else {
        host_udp
    };
    let counts: Arc<Mutex<HashMap<IpAddr, usize>>> = Arc::new(Mutex::new(HashMap::new()));

    let task = tokio::spawn(async move {
        loop {
            let (tcp, peer) = match listener.accept().await {
                Ok(x) => x,
                Err(_) => {
                    sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let ip = peer.ip();
            {
                let mut c = counts.lock().unwrap_or_else(|e| e.into_inner());
                let total: usize = c.values().sum();
                let mine = c.get(&ip).copied().unwrap_or(0);
                if total >= MAX_TUNNELS || mine >= MAX_PER_IP {
                    continue; // `tcp` shu yerda yopiladi
                }
                *c.entry(ip).or_insert(0) += 1;
            }
            let counts = counts.clone();
            tokio::spawn(async move {
                let lo = IpAddr::V4(loopback_for(ip));
                serve_one(tcp, ip, lo, target).await;
                let mut c = counts.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(n) = c.get_mut(&ip) {
                    *n -= 1;
                    if *n == 0 {
                        c.remove(&ip);
                        registry().lock().unwrap_or_else(|e| e.into_inner()).remove(&lo);
                    }
                }
            });
        }
    });
    Ok(TunnelServer { task })
}

async fn serve_one(mut tcp: TcpStream, real: IpAddr, lo: IpAddr, target: SocketAddr) {
    let mut magic = [0u8; 4];
    match timeout(HELLO_TIMEOUT, tcp.read_exact(&mut magic)).await {
        Ok(Ok(_)) if &magic == MAGIC => {}
        _ => return,
    }
    // Doimiy 127.x.y.z manzil; bog'lab bo'lmasa, oddiy 127.0.0.1 ishlatiladi.
    let (udp, lo) = match UdpSocket::bind(SocketAddr::new(lo, 0)).await {
        Ok(s) => (s, lo),
        Err(_) => match UdpSocket::bind("127.0.0.1:0").await {
            Ok(s) => (s, IpAddr::V4(Ipv4Addr::LOCALHOST)),
            Err(_) => return,
        },
    };
    registry().lock().unwrap_or_else(|e| e.into_inner()).insert(lo, real);
    eprintln!("[host] {real}: TCP tunnel ochildi");
    pump(tcp, udp, Some(target), false).await;
    eprintln!("[host] {real}: TCP tunnel yopildi");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_is_stable_and_not_localhost() {
        let ip: IpAddr = "5.133.121.155".parse().unwrap();
        let a = loopback_for(ip);
        assert_eq!(a, loopback_for(ip));
        assert_eq!(a.octets()[0], 127);
        assert_ne!(a, Ipv4Addr::LOCALHOST);
        assert!(a.octets()[3] >= 2);
    }

    #[tokio::test]
    async fn datagrams_roundtrip_through_tunnel() {
        // "Host" UDP echo serveri
        let echo = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let echo_addr = echo.local_addr().unwrap();
        tokio::spawn(async move {
            let mut b = [0u8; 2048];
            while let Ok((n, src)) = echo.recv_from(&mut b).await {
                let _ = echo.send_to(&b[..n], src).await;
            }
        });
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let listen: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let _server = spawn_server(listen, echo_addr).unwrap();

        let local = connect(listen).await.unwrap();
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        client.send_to(b"salom", local).await.unwrap();
        let mut buf = [0u8; 64];
        let (n, _) = timeout(Duration::from_secs(3), client.recv_from(&mut buf)).await.unwrap().unwrap();
        assert_eq!(&buf[..n], b"salom");
    }
}
