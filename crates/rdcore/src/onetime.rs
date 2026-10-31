//! Bir martalik ulanish paroli. Relay orqali ruxsat berilganda Host egasi tomonidagi agent yaratadi,
//! `serve()` esa doimiy parol bilan birga shuni ham qabul qiladi. Kalit doimiy parolning tuzi bilan
//! olinadi (Viewer tuzni `ServerHello` dan oladi), shuning uchun protokol o'zgarmaydi.
//!
//! Fayl: 1-qator kalit (hex), 2-qator amal qilish muddati (unix soniya). Bitta ishlatilgach o'chadi.

use anyhow::{Context, Result};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{auth, fsutil, hexfmt, paths};

/// Parol shuncha soniya amal qiladi.
pub const TTL_SECS: u64 = 60;
const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Yangi parol yaratadi, kalitini faylga yozadi va parolni qaytaradi.
pub fn create(salt: &[u8; protocol::SALT_LEN]) -> Result<String> {
    let raw = auth::random_bytes::<16>();
    let pw: String = raw.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect();
    let key = auth::derive_key(&pw, salt)?;
    let text = format!("{}\n{}\n", hexfmt::encode(&key), now() + TTL_SECS);
    fsutil::write_private(&paths::onetime_file()?, text.as_bytes()).context("bir martalik kalit yozilmadi")?;
    Ok(pw)
}

/// Amal qilayotgan kalit (bo'lsa).
pub fn load_key() -> Option<[u8; 32]> {
    let text = std::fs::read_to_string(paths::onetime_file().ok()?).ok()?;
    let mut lines = text.lines();
    let key = hexfmt::decode(lines.next()?).and_then(|v| <[u8; 32]>::try_from(v).ok())?;
    let until: u64 = lines.next()?.trim().parse().ok()?;
    (now() <= until).then_some(key)
}

/// Ishlatilgan parolni o'chiradi.
pub fn consume() {
    if let Ok(p) = paths::onetime_file() {
        let _ = std::fs::remove_file(p);
    }
}
