//! Relay serveri: clientlarni ro'yxatdan o'tkazadi, onlayn holatni yuritadi, ulanish so'rovlarini
//! uzatadi va ruxsat berilgan ikki clientning ma'lumot oqimlarini bir-biriga ulaydi.

use anyhow::{bail, Context, Result};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::Duration,
};
use tokio::{
    io::copy_bidirectional,
    sync::{mpsc, oneshot},
    time::{sleep, timeout},
};

use super::{
    msg::{clean, read_msg, write_msg, RMsg},
    registry::Registry,
    valid_id, ALPN, MAX_BIDI,
};
use crate::{
    auth::{self, Credentials},
    cert::HostIdentity,
    hexfmt,
    ratelimit::RateLimiter,
    tls,
};

const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// Qabul qiluvchi shuncha vaqt javob bermasa, so'rov bekor qilinadi (dialog 30 s, zaxira bilan).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(35);
/// Ruxsatdan keyin ikkala oqim shuncha vaqt ichida kelishi kerak.
const DATA_WINDOW: Duration = Duration::from_secs(40);

type Duplex = tokio::io::Join<quinn::RecvStream, quinn::SendStream>;

struct Online {
    name: String,
    tx: mpsc::UnboundedSender<RMsg>,
    conn: quinn::Connection,
    subs: HashSet<String>,
    ip: IpAddr,
    gen: u64,
}

struct Sess {
    a: String,
    b: String,
    req: String,
    accepted: bool,
}

struct St {
    /// `None`: ochiq server (kalitsiz ro'yxatdan o'tish mumkin).
    creds: Option<Credentials>,
    /// Yo'q ID ga urinishlar (ID ni taxmin qilishdan himoya), IP bo'yicha.
    probes: RateLimiter,
    fingerprint: [u8; 32],
    limiter: RateLimiter,
    reg: Mutex<Registry>,
    online: Mutex<HashMap<String, Online>>,
    sessions: Mutex<HashMap<String, Sess>>,
    rendezvous: Mutex<HashMap<String, oneshot::Sender<Duplex>>>,
    gen: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Clone, Debug)]
pub struct ClientInfo {
    pub id: String,
    pub name: String,
    pub online: bool,
    /// Oxirgi marta ko'ringan vaqt (unix soniya).
    pub last_seen: u64,
    pub ip: Option<IpAddr>,
}

/// Serverning holatini UI'ga ko'rsatish uchun (`RelayServer::run` dan oldin olinadi).
#[derive(Clone)]
pub struct ServerHandle(Arc<St>);

impl ServerHandle {
    pub fn clients(&self) -> Vec<ClientInfo> {
        let ids = lock(&self.0.reg).ids();
        let online = lock(&self.0.online);
        let reg = lock(&self.0.reg);
        let mut v: Vec<ClientInfo> = ids
            .into_iter()
            .filter_map(|id| {
                let e = reg.get(&id)?;
                let o = online.get(&id);
                Some(ClientInfo {
                    name: o.map_or_else(|| e.name.clone(), |o| o.name.clone()),
                    online: o.is_some(),
                    last_seen: e.last_seen,
                    ip: o.map(|o| o.ip),
                    id,
                })
            })
            .collect();
        v.sort_by(|a, b| b.online.cmp(&a.online).then(b.last_seen.cmp(&a.last_seen)));
        v
    }

    pub fn fingerprint(&self) -> [u8; 32] {
        self.0.fingerprint
    }
}

pub struct RelayServer {
    endpoint: quinn::Endpoint,
    st: Arc<St>,
}

impl RelayServer {
    /// `registry_path = None`: ro'yxat faqat xotirada. Tokio runtime ichida chaqirilishi kerak.
    pub fn bind(
        bind: SocketAddr,
        identity: &HostIdentity,
        creds: Option<Credentials>,
        registry_path: Option<PathBuf>,
    ) -> Result<Self> {
        let cfg = tls::server_config_alpn(identity, ALPN, MAX_BIDI)?;
        let endpoint = quinn::Endpoint::server(cfg, bind).with_context(|| format!("{bind} manzilini tinglab bo'lmadi"))?;
        Ok(RelayServer {
            endpoint,
            st: Arc::new(St {
                creds,
                probes: RateLimiter::new(10, Duration::from_secs(600), Duration::from_secs(900)),
                fingerprint: identity.fingerprint(),
                limiter: RateLimiter::standard(),
                reg: Mutex::new(Registry::load(registry_path)),
                online: Mutex::new(HashMap::new()),
                sessions: Mutex::new(HashMap::new()),
                rendezvous: Mutex::new(HashMap::new()),
                gen: AtomicU64::new(1),
            }),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    pub fn handle(&self) -> ServerHandle {
        ServerHandle(self.st.clone())
    }

    /// Ulanishlarni qabul qiladi (to'xtamaydi).
    pub async fn run(self) -> Result<()> {
        while let Some(incoming) = self.endpoint.accept().await {
            let ip = incoming.remote_address().ip();
            if self.st.limiter.is_blocked(ip) {
                eprintln!("[server] {ip}: bloklangan, ulanish rad etildi");
                incoming.refuse();
                continue;
            }
            let st = self.st.clone();
            tokio::spawn(async move {
                if let Err(e) = client_task(incoming, st, ip).await {
                    eprintln!("[server] {ip}: {e:#}");
                }
            });
        }
        Ok(())
    }
}

impl St {
    fn send_to(&self, id: &str, m: RMsg) -> bool {
        lock(&self.online).get(id).is_some_and(|o| o.tx.send(m).is_ok())
    }

    fn pres(&self, id: &str) -> RMsg {
        let online = lock(&self.online);
        match online.get(id) {
            Some(o) => RMsg::Pres { id: id.to_string(), online: true, name: o.name.clone() },
            None => {
                let name = lock(&self.reg).get(id).map(|e| e.name.clone()).unwrap_or_default();
                RMsg::Pres { id: id.to_string(), online: false, name }
            }
        }
    }

    /// `id` holati o'zgardi: unga obuna bo'lgan hamma clientga xabar beriladi.
    fn notify_presence(&self, id: &str) {
        let m = self.pres(id);
        for o in lock(&self.online).values() {
            if o.subs.contains(id) {
                let _ = o.tx.send(m.clone());
            }
        }
    }
}

async fn client_task(incoming: quinn::Incoming, st: Arc<St>, ip: IpAddr) -> Result<()> {
    let connecting = incoming.accept().context("ulanishni qabul qilib bo'lmadi")?;
    let conn = timeout(STEP_TIMEOUT, connecting)
        .await
        .context("handshake vaqti tugadi")?
        .context("handshake muvaffaqiyatsiz")?;
    let (mut send, mut recv) = timeout(STEP_TIMEOUT, conn.accept_bi())
        .await
        .context("control oqimi ochilmadi (vaqt tugadi)")?
        .context("control oqimi ochilmadi")?;

    let RMsg::Hello { id, token, name } = timeout(STEP_TIMEOUT, read_msg(&mut recv))
        .await
        .context("HELLO kelmadi (vaqt tugadi)")??
    else {
        bail!("birinchi xabar HELLO emas");
    };

    // Kalitli server: parol isboti (fingerprint'ga bog'langan). Ochiq serverda bu bosqich yo'q.
    if let Some(creds) = &st.creds {
        let nonce = auth::random_bytes::<{ protocol::NONCE_LEN }>();
        write_msg(&mut send, &RMsg::Challenge { salt: creds.salt, nonce }).await?;
        let RMsg::Proof { proof } = timeout(STEP_TIMEOUT, read_msg(&mut recv))
            .await
            .context("PROOF kelmadi (vaqt tugadi)")??
        else {
            bail!("PROOF kutilgan edi");
        };
        if !auth::verify_proof(&creds.key, &nonce, &st.fingerprint, &proof) {
            st.limiter.record_failure(ip);
            let _ = write_msg(&mut send, &RMsg::Err { text: "server kaliti noto'g'ri".into() }).await;
            finish_and_close(send, &conn).await;
            bail!("server kaliti noto'g'ri");
        }
        st.limiter.record_success(ip);
    }

    let name = match clean(&name) {
        n if n.is_empty() => "Noma'lum".to_string(),
        n => n,
    };
    // Shaxsiyat: ID+token mos kelsa shu ID; ID yo'q yoki ro'yxatda topilmasa yangisi; token mos kelmasa rad.
    let ident = {
        let mut reg = lock(&st.reg);
        if id.is_empty() {
            Some(reg.register(&name))
        } else {
            match reg.get(&id).map(|e| e.token == token) {
                None => Some(reg.register(&name)),
                Some(true) => {
                    reg.touch(&id, Some(&name));
                    Some((id, token))
                }
                Some(false) => None,
            }
        }
    };
    let Some((id, token)) = ident else {
        let _ = write_msg(&mut send, &RMsg::Err { text: "ID uchun token mos kelmadi".into() }).await;
        finish_and_close(send, &conn).await;
        bail!("ID tokeni mos kelmadi");
    };
    write_msg(&mut send, &RMsg::Ok { id: id.clone(), token }).await?;

    let (tx, mut rx) = mpsc::unbounded_channel::<RMsg>();
    let gen = st.gen.fetch_add(1, Ordering::SeqCst);
    let old = lock(&st.online).insert(
        id.clone(),
        Online { name, tx, conn: conn.clone(), subs: HashSet::new(), ip, gen },
    );
    if let Some(old) = old {
        old.conn.close(2u32.into(), b"replaced");
    }
    eprintln!("[server] {id} ({ip}): onlayn");
    st.notify_presence(&id);

    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if write_msg(&mut send, &m).await.is_err() {
                break;
            }
        }
        let _ = send.finish();
    });
    let data_task = {
        let (st, conn, id) = (st.clone(), conn.clone(), id.clone());
        tokio::spawn(async move {
            while let Ok((s, r)) = conn.accept_bi().await {
                let (st, id) = (st.clone(), id.clone());
                tokio::spawn(async move {
                    if let Err(e) = handle_data(&st, &id, s, r).await {
                        eprintln!("[server] {id}: ma'lumot oqimi: {e:#}");
                    }
                });
            }
        })
    };

    loop {
        tokio::select! {
            m = read_msg(&mut recv) => match m {
                Ok(m) => handle(&st, &id, m),
                Err(_) => break,
            },
            _ = conn.closed() => break,
        }
    }

    writer.abort();
    data_task.abort();
    let removed = {
        let mut on = lock(&st.online);
        if on.get(&id).map(|o| o.gen) == Some(gen) {
            on.remove(&id);
            true
        } else {
            false
        }
    };
    if removed {
        lock(&st.reg).touch(&id, None);
        st.notify_presence(&id);
        eprintln!("[server] {id}: oflayn");
    }
    conn.close(0u32.into(), b"bye");
    Ok(())
}

async fn finish_and_close(mut send: quinn::SendStream, conn: &quinn::Connection) {
    let _ = send.finish();
    let _ = timeout(Duration::from_secs(2), send.stopped()).await;
    conn.close(1u32.into(), b"rejected");
}

fn fail(st: &St, to: &str, req: &str, text: &str) {
    st.send_to(
        to,
        RMsg::Result { req: req.to_string(), ok: false, sid: String::new(), pw: None, text: text.to_string() },
    );
}

fn handle(st: &Arc<St>, id: &str, m: RMsg) {
    match m {
        RMsg::Sub(ids) => {
            let ids: HashSet<String> = ids.into_iter().filter(|i| valid_id(i)).collect();
            let msgs: Vec<RMsg> = ids.iter().map(|i| st.pres(i)).collect();
            if let Some(o) = lock(&st.online).get_mut(id) {
                o.subs = ids;
                for m in msgs {
                    let _ = o.tx.send(m);
                }
            }
        }
        RMsg::Connect { target, req, ask } => {
            if !valid_id(&target) {
                return fail(st, id, &req, "ID noto'g'ri (9 ta raqam)");
            }
            if target == id {
                return fail(st, id, &req, "o'zingizga ulanib bo'lmaydi");
            }
            // ID ni taxmin qilib topishdan himoya: yo'q ID ga ketma-ket urinishlar vaqtincha bloklanadi
            let ip = lock(&st.online).get(id).map(|o| o.ip);
            if ip.is_some_and(|ip| st.probes.is_blocked(ip)) {
                return fail(st, id, &req, "juda ko'p noto'g'ri ID kiritildi, keyinroq urinib ko'ring");
            }
            if lock(&st.reg).get(&target).is_none() {
                if let Some(ip) = ip {
                    st.probes.record_failure(ip);
                }
                return fail(st, id, &req, "bunday ID yo'q");
            }
            let my_name = lock(&st.online).get(id).map(|o| o.name.clone()).unwrap_or_default();
            let sid = hexfmt::encode(&auth::random_bytes::<8>());
            lock(&st.sessions).insert(
                sid.clone(),
                Sess { a: id.to_string(), b: target.clone(), req: req.clone(), accepted: false },
            );
            if !st.send_to(&target, RMsg::Incoming { sid: sid.clone(), from: id.to_string(), name: my_name, ask }) {
                lock(&st.sessions).remove(&sid);
                return fail(st, id, &req, "kompyuter oflayn");
            }
            let st = st.clone();
            tokio::spawn(async move {
                sleep(REQUEST_TIMEOUT).await;
                let expired = {
                    let mut s = lock(&st.sessions);
                    match s.get(&sid) {
                        Some(x) if !x.accepted => s.remove(&sid),
                        _ => None,
                    }
                };
                if let Some(x) = expired {
                    fail(&st, &x.a, &x.req, "javob berilmadi (vaqt tugadi)");
                    return;
                }
                sleep(DATA_WINDOW).await;
                lock(&st.sessions).remove(&sid);
            });
        }
        RMsg::Reply { sid, accept, pw } => {
            let found = {
                let mut s = lock(&st.sessions);
                match s.get_mut(&sid) {
                    Some(x) if x.b == id && !x.accepted => {
                        if accept {
                            x.accepted = true;
                            Some((x.a.clone(), x.req.clone()))
                        } else {
                            let x = s.remove(&sid).expect("topilgan");
                            Some((x.a, x.req))
                        }
                    }
                    _ => None,
                }
            };
            if let Some((a, req)) = found {
                if accept {
                    st.send_to(&a, RMsg::Result { req, ok: true, sid, pw, text: String::new() });
                } else {
                    fail(st, &a, &req, "ruxsat berilmadi");
                }
            }
        }
        _ => {}
    }
}

/// Client ochgan ma'lumot oqimi: sessiya ruxsat etilgan bo'lsa, ikkinchi tomon oqimi bilan juftlanadi.
async fn handle_data(st: &St, cid: &str, send: quinn::SendStream, mut recv: quinn::RecvStream) -> Result<()> {
    let RMsg::Data { sid } = timeout(STEP_TIMEOUT, read_msg(&mut recv))
        .await
        .context("DATA kelmadi (vaqt tugadi)")??
    else {
        bail!("DATA kutilgan edi");
    };
    // Qabul qiluvchi `Reply` ni control oqimida, ma'lumot oqimini esa darhol keyin yuboradi: ular boshqa
    // oqimlar bo'lgani uchun tartibi kafolatlanmagan, shuning uchun ruxsat belgisi biroz kutiladi.
    let mut waited = 0;
    loop {
        {
            let s = lock(&st.sessions);
            let x = s.get(&sid).context("sessiya topilmadi")?;
            if x.a != cid && x.b != cid {
                bail!("sessiyaga ruxsat yo'q");
            }
            if x.accepted {
                break;
            }
        }
        waited += 1;
        if waited > 100 {
            bail!("sessiyaga ruxsat yo'q");
        }
        sleep(Duration::from_millis(30)).await;
    }
    let mine = tokio::io::join(recv, send);
    let rx = {
        let mut r = lock(&st.rendezvous);
        match r.remove(&sid) {
            Some(tx) => {
                drop(r);
                let _ = tx.send(mine);
                return Ok(());
            }
            None => {
                let (tx, rx) = oneshot::channel();
                r.insert(sid.clone(), tx);
                rx
            }
        }
    };
    let mut other = match timeout(DATA_WINDOW, rx).await {
        Ok(Ok(o)) => o,
        _ => {
            lock(&st.rendezvous).remove(&sid);
            bail!("ikkinchi tomon oqimi kelmadi");
        }
    };
    let mut mine = mine;
    eprintln!("[server] sessiya {sid}: ulandi");
    let _ = copy_bidirectional(&mut mine, &mut other).await;
    lock(&st.sessions).remove(&sid);
    eprintln!("[server] sessiya {sid}: tugadi");
    Ok(())
}
