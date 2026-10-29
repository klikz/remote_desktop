//! Parol asosidagi autentifikatsiya (challenge-response).

use anyhow::{anyhow, bail, Context, Result};
use argon2::Argon2;
use hmac::{Hmac, Mac};
use protocol::{NONCE_LEN, PROOF_LEN, SALT_LEN};
use rand::RngCore;
use sha2::Sha256;
use std::{fs, path::Path};

use crate::{fsutil, hexfmt};

pub const MIN_PASSWORD_LEN: usize = 8;

type HmacSha256 = Hmac<Sha256>;

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b
}

/// Paroldan 32 baytli kalit hosil qiladi (Argon2id).
pub fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let mut out = [0u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow!("argon2 xatosi: {e}"))?;
    Ok(out)
}

fn mac_for(key: &[u8; 32], nonce: &[u8; NONCE_LEN], fingerprint: &[u8; 32]) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC har qanday kalit uzunligini qabul qiladi");
    mac.update(b"rd-auth-v1");
    mac.update(nonce);
    mac.update(fingerprint);
    mac
}

/// Viewer tomonda: isbot hisoblash.
pub fn compute_proof(key: &[u8; 32], nonce: &[u8; NONCE_LEN], fingerprint: &[u8; 32]) -> [u8; PROOF_LEN] {
    let out = mac_for(key, nonce, fingerprint).finalize().into_bytes();
    let mut proof = [0u8; PROOF_LEN];
    proof.copy_from_slice(&out);
    proof
}

/// Host tomonda: isbotni (doimiy vaqtda) tekshirish.
pub fn verify_proof(
    key: &[u8; 32],
    nonce: &[u8; NONCE_LEN],
    fingerprint: &[u8; 32],
    proof: &[u8; PROOF_LEN],
) -> bool {
    mac_for(key, nonce, fingerprint).verify_slice(proof).is_ok()
}

/// Host saqlaydigan ma'lumot: tuz va undan olingan kalit (parolning o'zi saqlanmaydi).
pub struct Credentials {
    pub salt: [u8; SALT_LEN],
    pub key: [u8; 32],
}

impl Credentials {
    pub fn from_password(password: &str) -> Result<Self> {
        if password.chars().count() < MIN_PASSWORD_LEN {
            bail!("parol kamida {MIN_PASSWORD_LEN} belgidan iborat bo'lishi kerak");
        }
        let salt = random_bytes::<SALT_LEN>();
        let key = derive_key(password, &salt)?;
        Ok(Credentials { salt, key })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = format!("{}\n{}\n", hexfmt::encode(&self.salt), hexfmt::encode(&self.key));
        fsutil::write_private(path, text.as_bytes())
            .with_context(|| format!("{} ga yozib bo'lmadi", path.display()))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("{} o'qib bo'lmadi", path.display()))?;
        let mut lines = text.lines();
        let salt = hexfmt::decode(lines.next().unwrap_or(""))
            .and_then(|v| <[u8; SALT_LEN]>::try_from(v).ok())
            .ok_or_else(|| anyhow!("parol fayli buzilgan (tuz)"))?;
        let key = hexfmt::decode(lines.next().unwrap_or(""))
            .and_then(|v| <[u8; 32]>::try_from(v).ok())
            .ok_or_else(|| anyhow!("parol fayli buzilgan (kalit)"))?;
        Ok(Credentials { salt, key })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_password_verifies() {
        let creds = Credentials::from_password("correct horse").unwrap();
        let nonce = random_bytes::<NONCE_LEN>();
        let fp = [5u8; 32];
        let key = derive_key("correct horse", &creds.salt).unwrap();
        let proof = compute_proof(&key, &nonce, &fp);
        assert!(verify_proof(&creds.key, &nonce, &fp, &proof));
    }

    #[test]
    fn wrong_password_fails() {
        let creds = Credentials::from_password("correct horse").unwrap();
        let nonce = random_bytes::<NONCE_LEN>();
        let fp = [5u8; 32];
        let key = derive_key("wrong horse!", &creds.salt).unwrap();
        let proof = compute_proof(&key, &nonce, &fp);
        assert!(!verify_proof(&creds.key, &nonce, &fp, &proof));
    }

    #[test]
    fn proof_is_bound_to_nonce_and_fingerprint() {
        let creds = Credentials::from_password("correct horse").unwrap();
        let nonce = [1u8; NONCE_LEN];
        let fp = [5u8; 32];
        let proof = compute_proof(&creds.key, &nonce, &fp);
        assert!(verify_proof(&creds.key, &nonce, &fp, &proof));
        assert!(!verify_proof(&creds.key, &[2u8; NONCE_LEN], &fp, &proof));
        assert!(!verify_proof(&creds.key, &nonce, &[6u8; 32], &proof));
    }

    #[test]
    fn short_password_rejected() {
        assert!(Credentials::from_password("short").is_err());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("rd-auth-{}", std::process::id()));
        let file = dir.join("password.key");
        let creds = Credentials::from_password("correct horse").unwrap();
        creds.save(&file).unwrap();
        let loaded = Credentials::load(&file).unwrap();
        assert_eq!(loaded.salt, creds.salt);
        assert_eq!(loaded.key, creds.key);
        let _ = fs::remove_dir_all(&dir);
    }
}
