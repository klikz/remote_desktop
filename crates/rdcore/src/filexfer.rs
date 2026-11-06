//! Fayl almashinuvi: Viewer va Host o'rtasida har fayl uchun alohida QUIC oqimi (video va control'ga halaqit bermaydi).
//!
//! Xavfsizlik: Host faqat o'zining almashinuv papkasida (`<Yuklamalar>/MS Remote`) ishlaydi. Fayl nomida yo'l
//! bo'lishi mumkin emas, mavjud fayl ustiga yozilmaydi (nom `fayl (1).txt` ko'rinishida o'zgartiriladi).
//! `RD_NO_FILES=1` bo'lsa, Host fayl almashinuvini rad etadi.
//!
//! Oqim: birinchi bayt = amal. `PUT`: `nom_uzunligi(2) nom hajm(8) ma'lumot` -> `holat(1)`.
//! `LIST`: -> `holat(1) soni(4) [nom_uzunligi(2) nom hajm(8)]...`. `GET`: `nom_uzunligi(2) nom` -> `holat(1) hajm(8) ma'lumot`.

use anyhow::{anyhow, bail, Context, Result};
use quinn::{Connection, RecvStream, SendStream};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinHandle,
    time::timeout,
};

const OP_PUT: u8 = 1;
const OP_LIST: u8 = 2;
const OP_GET: u8 = 3;

const ST_OK: u8 = 0;
const ST_BAD_NAME: u8 = 1;
const ST_TOO_BIG: u8 = 2;
const ST_NOT_FOUND: u8 = 3;
const ST_IO: u8 = 4;
const ST_DISABLED: u8 = 5;

/// Almashinuv papkasining nomi.
pub const FOLDER: &str = "MS Remote";
/// Bitta faylning eng katta hajmi.
pub const MAX_FILE_LEN: u64 = 16 << 30;
const MAX_NAME_LEN: usize = 255;
const MAX_LIST: usize = 1000;
const CHUNK: usize = 64 * 1024;
/// Fayl oqimlari ustuvorligi: video (Host'da 10) va kiritishdan past, katta fayl ekran/boshqaruvni bosmasin.
const PRIORITY: i32 = -10;
/// Oqim ichida keyingi baytni kutish muddati (tarmoq uzilsa osilib qolmaslik uchun).
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

fn describe(st: u8) -> &'static str {
    match st {
        ST_OK => "ok",
        ST_BAD_NAME => "fayl nomi yaroqsiz",
        ST_TOO_BIG => "fayl juda katta",
        ST_NOT_FOUND => "fayl topilmadi",
        ST_IO => "fayl bilan ishlashda xato",
        ST_DISABLED => "Host fayl almashinuvini o'chirib qo'ygan",
        _ => "noma'lum sabab",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub name: String,
    pub size: u64,
}

/// Shu kompyuterdagi almashinuv papkasi (yo'q bo'lsa yaratiladi). `RD_SHARED_DIR` bilan boshqa papka berish mumkin.
pub fn shared_dir() -> Result<PathBuf> {
    let dir = match std::env::var_os("RD_SHARED_DIR").filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => match dirs::download_dir().or_else(dirs::home_dir) {
            Some(base) => base.join(FOLDER),
            None => crate::paths::config_dir()?.join("shared"),
        },
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("{} papkasini yaratib bo'lmadi", dir.display()))?;
    Ok(dir)
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Nom papkadan chiqib keta olmaydigan, ikkala tizimda (Windows/Linux) yaroqli oddiy fayl nomimi.
pub fn valid_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN || name == "." || name == ".." {
        return false;
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return false;
    }
    if name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
        return false;
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !RESERVED.contains(&stem.as_str())
}

/// Ixtiyoriy fayl nomini yaroqli nomga aylantiradi (taqiqlangan belgilar `_` ga almashadi).
pub fn clean_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    out = out.trim_end_matches([' ', '.']).to_string();
    while out.len() > MAX_NAME_LEN {
        out.pop();
    }
    if out.is_empty() {
        out = "fayl".to_string();
    }
    if !valid_name(&out) {
        out.insert(0, '_');
    }
    out
}

/// `dir/name` band bo'lsa, `name (1).ext`, `name (2).ext`... dan birinchi bo'shini qaytaradi.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1..10_000 {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} ({}){ext}", rand::random::<u32>()))
}

fn part_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.{:08x}.part", rand::random::<u32>()))
}

fn disabled() -> bool {
    std::env::var_os("RD_NO_FILES").is_some()
}

// ---------------------------------------------------------------------------------------------
// Oqimdan o'qish/yozish yordamchilari
// ---------------------------------------------------------------------------------------------

async fn read_exact(recv: &mut RecvStream, buf: &mut [u8]) -> Result<()> {
    timeout(IDLE_TIMEOUT, recv.read_exact(buf))
        .await
        .map_err(|_| anyhow!("ma'lumot kelmadi (vaqt tugadi)"))?
        .map_err(|e| anyhow!("oqim uzildi: {e}"))
}

async fn read_u8(recv: &mut RecvStream) -> Result<u8> {
    let mut b = [0u8; 1];
    read_exact(recv, &mut b).await?;
    Ok(b[0])
}

async fn read_u16(recv: &mut RecvStream) -> Result<u16> {
    let mut b = [0u8; 2];
    read_exact(recv, &mut b).await?;
    Ok(u16::from_be_bytes(b))
}

async fn read_u32(recv: &mut RecvStream) -> Result<u32> {
    let mut b = [0u8; 4];
    read_exact(recv, &mut b).await?;
    Ok(u32::from_be_bytes(b))
}

async fn read_u64(recv: &mut RecvStream) -> Result<u64> {
    let mut b = [0u8; 8];
    read_exact(recv, &mut b).await?;
    Ok(u64::from_be_bytes(b))
}

async fn read_name(recv: &mut RecvStream) -> Result<String> {
    let len = read_u16(recv).await? as usize;
    if len == 0 || len > MAX_NAME_LEN {
        bail!("nom uzunligi noto'g'ri");
    }
    let mut buf = vec![0u8; len];
    read_exact(recv, &mut buf).await?;
    String::from_utf8(buf).map_err(|_| anyhow!("nom UTF-8 emas"))
}

async fn write_all(send: &mut SendStream, data: &[u8]) -> Result<()> {
    timeout(IDLE_TIMEOUT, send.write_all(data))
        .await
        .map_err(|_| anyhow!("yuborib bo'lmadi (vaqt tugadi)"))?
        .map_err(|e| anyhow!("oqim uzildi: {e}"))
}

/// Oqimni tugatadi va qarshi tomon o'qib bo'lguncha kutadi.
async fn finish(mut send: SendStream) {
    let _ = send.finish();
    let _ = timeout(Duration::from_secs(5), send.stopped()).await;
}

// ---------------------------------------------------------------------------------------------
// Host tomoni
// ---------------------------------------------------------------------------------------------

/// Ulanishdagi fayl so'rovlarini qabul qiladi. Ulanish yopilganda yoki qaytgan vazifa to'xtatilganda tugaydi.
pub fn spawn_server(conn: Connection) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Ok((send, recv)) = conn.accept_bi().await {
            let _ = send.set_priority(PRIORITY);
            tokio::spawn(async move {
                if let Err(e) = handle(send, recv).await {
                    eprintln!("[host] fayl almashinuvi: {e:#}");
                }
            });
        }
    })
}

async fn handle(mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let op = read_u8(&mut recv).await?;
    if disabled() {
        write_all(&mut send, &[ST_DISABLED]).await?;
        finish(send).await;
        return Ok(());
    }
    match op {
        OP_PUT => serve_put(send, recv).await,
        OP_LIST => serve_list(send).await,
        OP_GET => serve_get(send, recv).await,
        other => bail!("noma'lum fayl amali: {other}"),
    }
}

async fn serve_put(mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let name = read_name(&mut recv).await?;
    let size = read_u64(&mut recv).await?;
    let status = if !valid_name(&name) {
        ST_BAD_NAME
    } else if size > MAX_FILE_LEN {
        ST_TOO_BIG
    } else {
        ST_OK
    };
    if status != ST_OK {
        write_all(&mut send, &[status]).await?;
        let _ = recv.stop(0u32.into());
        finish(send).await;
        return Ok(());
    }

    let dir = shared_dir()?;
    let tmp = part_path(&dir, &name);
    let result = receive_into(&mut recv, &tmp, size).await;
    let result = match result {
        Ok(()) => tokio::fs::rename(&tmp, unique_path(&dir, &name)).await.map_err(Into::into),
        Err(e) => Err(e),
    };
    let status = match &result {
        Ok(()) => ST_OK,
        Err(_) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            ST_IO
        }
    };
    let _ = write_all(&mut send, &[status]).await;
    finish(send).await;
    if result.is_ok() {
        println!("[host] fayl qabul qilindi: {name} ({size} bayt)");
    }
    result
}

async fn receive_into(recv: &mut RecvStream, path: &Path, size: u64) -> Result<()> {
    let mut file = File::create(path).await.with_context(|| format!("{} yaratilmadi", path.display()))?;
    let mut buf = vec![0u8; CHUNK];
    let mut left = size;
    while left > 0 {
        let want = left.min(CHUNK as u64) as usize;
        let n = match timeout(IDLE_TIMEOUT, recv.read(&mut buf[..want])).await {
            Err(_) => bail!("ma'lumot kelmadi (vaqt tugadi)"),
            Ok(Err(e)) => bail!("oqim uzildi: {e}"),
            Ok(Ok(None)) => bail!("fayl to'liq kelmadi"),
            Ok(Ok(Some(n))) => n,
        };
        file.write_all(&buf[..n]).await?;
        left -= n as u64;
    }
    file.flush().await?;
    Ok(())
}

async fn serve_list(mut send: SendStream) -> Result<()> {
    let files = list_local().await.unwrap_or_default();
    let mut out = Vec::with_capacity(5 + files.len() * 32);
    out.push(ST_OK);
    out.extend_from_slice(&(files.len() as u32).to_be_bytes());
    for f in &files {
        out.extend_from_slice(&(f.name.len() as u16).to_be_bytes());
        out.extend_from_slice(f.name.as_bytes());
        out.extend_from_slice(&f.size.to_be_bytes());
    }
    write_all(&mut send, &out).await?;
    finish(send).await;
    Ok(())
}

/// Shu kompyuterdagi almashinuv papkasidagi fayllar (nom bo'yicha tartiblangan).
pub async fn list_local() -> Result<Vec<RemoteFile>> {
    let dir = shared_dir()?;
    let mut rd = tokio::fs::read_dir(&dir).await?;
    let mut files = vec![];
    while let Some(entry) = rd.next_entry().await? {
        let Ok(meta) = entry.metadata().await else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else { continue };
        if !valid_name(&name) || name.ends_with(".part") {
            continue;
        }
        files.push(RemoteFile { name, size: meta.len() });
        if files.len() >= MAX_LIST {
            break;
        }
    }
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(files)
}

async fn serve_get(mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let name = read_name(&mut recv).await?;
    if !valid_name(&name) {
        write_all(&mut send, &[ST_BAD_NAME]).await?;
        finish(send).await;
        return Ok(());
    }
    let path = shared_dir()?.join(&name);
    let opened = async {
        let file = File::open(&path).await?;
        let meta = file.metadata().await?;
        if !meta.is_file() {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound));
        }
        Ok((file, meta.len()))
    }
    .await;
    let (mut file, size) = match opened {
        Ok(x) => x,
        Err(e) => {
            let st = if e.kind() == std::io::ErrorKind::NotFound { ST_NOT_FOUND } else { ST_IO };
            write_all(&mut send, &[st]).await?;
            finish(send).await;
            return Ok(());
        }
    };
    let mut head = vec![ST_OK];
    head.extend_from_slice(&size.to_be_bytes());
    write_all(&mut send, &head).await?;
    let mut buf = vec![0u8; CHUNK];
    let mut left = size;
    while left > 0 {
        let want = left.min(CHUNK as u64) as usize;
        let n = file.read(&mut buf[..want]).await?;
        if n == 0 {
            bail!("fayl o'qish paytida qisqardi: {name}");
        }
        write_all(&mut send, &buf[..n]).await?;
        left -= n as u64;
    }
    finish(send).await;
    println!("[host] fayl yuborildi: {name} ({size} bayt)");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Viewer tomoni
// ---------------------------------------------------------------------------------------------

/// Host'dagi almashinuv papkasi ro'yxati.
pub async fn list(conn: &Connection) -> Result<Vec<RemoteFile>> {
    let (mut send, mut recv) = conn.open_bi().await.context("oqim ochib bo'lmadi")?;
    write_all(&mut send, &[OP_LIST]).await?;
    let _ = send.finish();
    let st = read_u8(&mut recv).await?;
    if st != ST_OK {
        bail!("{}", describe(st));
    }
    let count = (read_u32(&mut recv).await? as usize).min(MAX_LIST);
    let mut files = Vec::with_capacity(count);
    for _ in 0..count {
        let name = read_name(&mut recv).await?;
        let size = read_u64(&mut recv).await?;
        files.push(RemoteFile { name, size });
    }
    Ok(files)
}

/// Faylni Host'ning almashinuv papkasiga yuboradi. `progress(yuborilgan, jami)`.
pub async fn upload(conn: &Connection, path: &Path, progress: &mut (dyn FnMut(u64, u64) + Send)) -> Result<()> {
    let meta = tokio::fs::metadata(path).await.with_context(|| format!("{} o'qilmadi", path.display()))?;
    if !meta.is_file() {
        bail!("bu fayl emas: {}", path.display());
    }
    let size = meta.len();
    if size > MAX_FILE_LEN {
        bail!("fayl juda katta (eng ko'pi {} GB)", MAX_FILE_LEN >> 30);
    }
    let name = clean_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let mut file = File::open(path).await.with_context(|| format!("{} ochilmadi", path.display()))?;

    let (mut send, mut recv) = conn.open_bi().await.context("oqim ochib bo'lmadi")?;
    let _ = send.set_priority(PRIORITY);
    let mut head = vec![OP_PUT];
    head.extend_from_slice(&(name.len() as u16).to_be_bytes());
    head.extend_from_slice(name.as_bytes());
    head.extend_from_slice(&size.to_be_bytes());

    // Host rad etsa (nom/hajm/o'chirilgan) oqimni to'xtatadi: yozish xatosi bo'lsa, sababini holat baytidan o'qiymiz.
    let sent = async {
        write_all(&mut send, &head).await?;
        let mut buf = vec![0u8; CHUNK];
        let mut done = 0u64;
        progress(0, size);
        while done < size {
            let want = (size - done).min(CHUNK as u64) as usize;
            let n = file.read(&mut buf[..want]).await?;
            if n == 0 {
                bail!("fayl yuborish paytida o'zgardi");
            }
            write_all(&mut send, &buf[..n]).await?;
            done += n as u64;
            progress(done, size);
        }
        Ok(())
    }
    .await;
    if sent.is_ok() {
        let _ = send.finish();
    }
    let status = timeout(Duration::from_secs(60), read_u8(&mut recv)).await;
    match (sent, status) {
        (Ok(()), Ok(Ok(ST_OK))) => Ok(()),
        (_, Ok(Ok(st))) if st != ST_OK => bail!("Host qabul qilmadi: {}", describe(st)),
        (Err(e), _) => Err(e),
        (Ok(()), Ok(Err(e))) => Err(e),
        (Ok(()), Err(_)) => bail!("Host tasdiqlamadi (vaqt tugadi)"),
        (Ok(()), Ok(Ok(_))) => unreachable!(),
    }
}

/// Host'ning almashinuv papkasidagi faylni shu kompyuterning almashinuv papkasiga yuklab oladi. Saqlangan yo'lni qaytaradi.
pub async fn download(conn: &Connection, name: &str, progress: &mut (dyn FnMut(u64, u64) + Send)) -> Result<PathBuf> {
    if !valid_name(name) {
        bail!("fayl nomi yaroqsiz");
    }
    let (mut send, mut recv) = conn.open_bi().await.context("oqim ochib bo'lmadi")?;
    let mut req = vec![OP_GET];
    req.extend_from_slice(&(name.len() as u16).to_be_bytes());
    req.extend_from_slice(name.as_bytes());
    write_all(&mut send, &req).await?;
    let _ = send.finish();

    let st = read_u8(&mut recv).await?;
    if st != ST_OK {
        bail!("Host yubormadi: {}", describe(st));
    }
    let size = read_u64(&mut recv).await?;
    if size > MAX_FILE_LEN {
        bail!("fayl juda katta");
    }
    let dir = shared_dir()?;
    let tmp = part_path(&dir, name);
    let result = async {
        let mut file = File::create(&tmp).await.with_context(|| format!("{} yaratilmadi", tmp.display()))?;
        let mut buf = vec![0u8; CHUNK];
        let mut done = 0u64;
        progress(0, size);
        while done < size {
            let want = (size - done).min(CHUNK as u64) as usize;
            let n = match timeout(IDLE_TIMEOUT, recv.read(&mut buf[..want])).await {
                Err(_) => bail!("ma'lumot kelmadi (vaqt tugadi)"),
                Ok(Err(e)) => bail!("oqim uzildi: {e}"),
                Ok(Ok(None)) => bail!("fayl to'liq kelmadi"),
                Ok(Ok(Some(n))) => n,
            };
            file.write_all(&buf[..n]).await?;
            done += n as u64;
            progress(done, size);
        }
        file.flush().await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    match result {
        Ok(()) => {
            let dest = unique_path(&dir, name);
            tokio::fs::rename(&tmp, &dest).await.context("faylni saqlab bo'lmadi")?;
            Ok(dest)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("hisobot.pdf"));
        assert!(valid_name("Ma'lumot (1).txt"));
        for bad in ["", ".", "..", "a/b", "a\\b", "C:x", "x.", "x ", "con", "NUL.txt", "a\nb"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn clean_makes_valid() {
        for raw in ["a/b.txt", "con", "x.", "", "a:b*c", "   "] {
            assert!(valid_name(&clean_name(raw)), "{raw:?} -> {:?}", clean_name(raw));
        }
        assert_eq!(clean_name("a/b.txt"), "a_b.txt");
    }

    #[test]
    fn unique_names() {
        let dir = std::env::temp_dir().join(format!("rd-filexfer-{}", rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), b"1").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (1).txt"));
        std::fs::write(dir.join("a (1).txt"), b"1").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (2).txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
