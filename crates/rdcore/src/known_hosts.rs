//! Ishonchli Host'lar ro'yxati (SSH'dagi known_hosts kabi).
//! Fayl formati: har qatorda `manzil fingerprint`.

use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Trust {
    /// Manzil ro'yxatda va fingerprint mos.
    Known,
    /// Manzil ro'yxatda yo'q (birinchi ulanish).
    Unknown,
    /// Manzil ro'yxatda bor, lekin fingerprint boshqa. Xavfli holat!
    Changed { expected: String },
}

pub struct KnownHosts {
    path: PathBuf,
}

impl KnownHosts {
    pub fn new(path: PathBuf) -> Self {
        KnownHosts { path }
    }

    pub fn check(&self, addr: &str, fingerprint: &str) -> io::Result<Trust> {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Trust::Unknown),
            Err(e) => return Err(e),
        };
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            if let (Some(a), Some(fp)) = (parts.next(), parts.next()) {
                if a == addr {
                    return Ok(if fp.eq_ignore_ascii_case(fingerprint) {
                        Trust::Known
                    } else {
                        Trust::Changed {
                            expected: fp.to_string(),
                        }
                    });
                }
            }
        }
        Ok(Trust::Unknown)
    }

    pub fn add(&self, addr: &str, fingerprint: &str) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{addr} {fingerprint}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rd-kh-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("known_hosts")
    }

    #[test]
    fn unknown_then_known() {
        let kh = KnownHosts::new(temp_file("a"));
        assert_eq!(kh.check("1.2.3.4:7777", "AA:BB").unwrap(), Trust::Unknown);
        kh.add("1.2.3.4:7777", "AA:BB").unwrap();
        assert_eq!(kh.check("1.2.3.4:7777", "AA:BB").unwrap(), Trust::Known);
        assert_eq!(kh.check("1.2.3.4:7777", "aa:bb").unwrap(), Trust::Known);
    }

    #[test]
    fn changed_fingerprint_detected() {
        let kh = KnownHosts::new(temp_file("b"));
        kh.add("1.2.3.4:7777", "AA:BB").unwrap();
        assert_eq!(
            kh.check("1.2.3.4:7777", "CC:DD").unwrap(),
            Trust::Changed { expected: "AA:BB".to_string() }
        );
    }

    #[test]
    fn other_addresses_are_independent() {
        let kh = KnownHosts::new(temp_file("c"));
        kh.add("1.2.3.4:7777", "AA:BB").unwrap();
        assert_eq!(kh.check("5.6.7.8:7777", "AA:BB").unwrap(), Trust::Unknown);
        kh.add("5.6.7.8:7777", "CC:DD").unwrap();
        assert_eq!(kh.check("5.6.7.8:7777", "CC:DD").unwrap(), Trust::Known);
        assert_eq!(kh.check("1.2.3.4:7777", "AA:BB").unwrap(), Trust::Known);
    }
}
