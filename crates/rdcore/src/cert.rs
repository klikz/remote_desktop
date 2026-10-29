//! Host sertifikati (self-signed) va fingerprint.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

use crate::fsutil;

pub struct HostIdentity {
    /// DER ko'rinishidagi sertifikat.
    pub cert_der: Vec<u8>,
    /// DER (PKCS#8) ko'rinishidagi maxfiy kalit.
    pub key_der: Vec<u8>,
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let d = Sha256::digest(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

impl HostIdentity {
    /// `dir` ichida `cert.der` va `key.der` bor bo'lsa o'qiydi, yo'q bo'lsa yangisini yaratadi.
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let cert_path = dir.join("cert.der");
        let key_path = dir.join("key.der");
        if cert_path.exists() && key_path.exists() {
            return Ok(HostIdentity {
                cert_der: fs::read(&cert_path).with_context(|| format!("{} o'qib bo'lmadi", cert_path.display()))?,
                key_der: fs::read(&key_path).with_context(|| format!("{} o'qib bo'lmadi", key_path.display()))?,
            });
        }
        let id = Self::generate()?;
        fsutil::write_private(&cert_path, &id.cert_der).context("sertifikatni saqlab bo'lmadi")?;
        fsutil::write_private(&key_path, &id.key_der).context("kalitni saqlab bo'lmadi")?;
        Ok(id)
    }

    pub fn generate() -> Result<Self> {
        let ck = rcgen::generate_simple_self_signed(vec![protocol::SERVER_NAME.to_string()])
            .context("sertifikat yaratib bo'lmadi")?;
        Ok(HostIdentity {
            cert_der: ck.cert.der().as_ref().to_vec(),
            key_der: ck.key_pair.serialize_der(),
        })
    }

    pub fn fingerprint(&self) -> [u8; 32] {
        sha256(&self.cert_der)
    }
}
