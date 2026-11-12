//! Ulanish kalitlarini xavfsiz saqlash. Kalit faqat foydalanuvchi shu ulanish uchun "Saqlash" ni belgilagan bo'lsa
//! saqlanadi; har bir ulanishning (P2P manzili yoki ID) o'z yozuvi bor.
//!
//! - Windows: DPAPI (`CryptProtectData`). Kalit shu Windows foydalanuvchisining hisobiga bog'lab shifrlanadi va
//!   `saved_keys.txt` ga yoziladi: faylni boshqa foydalanuvchi yoki boshqa kompyuter ocha olmaydi.
//! - Linux: tizim kalit zanjiri (GNOME Keyring / KWallet) `secret-tool` orqali. U o'rnatilmagan bo'lsa saqlash
//!   taklif qilinmaydi (`available()` = `false`): kalit hech qachon ochiq matnda diskka yozilmaydi.

/// P2P ulanish yozuvi (`manzil:port`).
pub fn p2p_key(addr: &str) -> String {
    format!("p2p:{}", addr.trim().to_ascii_lowercase())
}

/// Server orqali ulanish yozuvi (9 xonali ID).
pub fn id_key(id: &str) -> String {
    format!("id:{id}")
}

/// Shu tizimda xavfsiz saqlash imkoni bormi (bo'lmasa "Saqlash" belgisi o'chirilgan holda ko'rsatiladi).
pub fn available() -> bool {
    imp::available()
}

/// Shu ulanish uchun saqlangan kalit.
pub fn get(conn: &str) -> Option<String> {
    imp::get(conn).filter(|p| !p.is_empty())
}

/// Kalitni saqlaydi (oldingisi almashtiriladi). Bo'sh kalit saqlanmaydi: yozuv o'chiriladi.
pub fn set(conn: &str, password: &str) -> Result<(), String> {
    if password.is_empty() {
        delete(conn);
        return Ok(());
    }
    imp::set(conn, password)
}

/// Shu ulanishning saqlangan kalitini o'chiradi (bo'lmasa hech narsa qilmaydi).
pub fn delete(conn: &str) {
    imp::delete(conn);
}

#[cfg(windows)]
mod imp {
    use rdcore::{fsutil, hexfmt, paths};
    use std::{path::PathBuf, ptr};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB},
    };

    /// Oyna ko'rsatilmasin (xizmat/fon rejimida ham ishlaydi).
    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
    /// Qo'shimcha entropiya: boshqa dastur shu foydalanuvchi nomidan tasodifan ochib qo'ymasin.
    const ENTROPY: &[u8] = b"ulan-remote-desktop/saved-key/v1";

    fn file() -> Option<PathBuf> {
        paths::config_dir().ok().map(|d| d.join("saved_keys.txt"))
    }

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
    }

    /// `protect = true`: shifrlash, aks holda ochish.
    fn crypt(data: &[u8], protect: bool) -> Option<Vec<u8>> {
        let input = blob(data);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: ptr::null_mut() };
        // SAFETY: kirish bloblari chaqiruv davomida yashaydi; chiqish buferini Windows ajratadi va u nusxa olingach
        // `LocalFree` bilan bo'shatiladi.
        unsafe {
            let ok = if protect {
                CryptProtectData(&input, ptr::null(), &entropy, ptr::null(), ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
            } else {
                CryptUnprotectData(&input, ptr::null_mut(), &entropy, ptr::null(), ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
            };
            if ok == 0 || out.pbData.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
            // Ochilgan kalit xotirada qolib ketmasin
            std::ptr::write_bytes(out.pbData, 0, out.cbData as usize);
            LocalFree(out.pbData.cast());
            Some(bytes)
        }
    }

    /// Fayl: har qatorda `yozuv<TAB>shifrlangan kalit (hex)`.
    fn load() -> Vec<(String, String)> {
        let Some(text) = file().and_then(|p| std::fs::read_to_string(p).ok()) else { return vec![] };
        text.lines().filter_map(|l| l.split_once('\t')).map(|(k, v)| (k.to_string(), v.trim().to_string())).collect()
    }

    fn store(entries: &[(String, String)]) -> Result<(), String> {
        let path = file().ok_or("sozlama papkasi topilmadi")?;
        let text: String = entries.iter().map(|(k, v)| format!("{k}\t{v}\n")).collect();
        fsutil::write_private(&path, text.as_bytes()).map_err(|e| format!("kalit saqlanmadi: {e}"))
    }

    fn clean(conn: &str) -> String {
        conn.replace(['\t', '\n', '\r'], " ")
    }

    pub fn available() -> bool {
        true
    }

    pub fn get(conn: &str) -> Option<String> {
        let conn = clean(conn);
        let (_, hex) = load().into_iter().find(|(k, _)| *k == conn)?;
        String::from_utf8(crypt(&hexfmt::decode(&hex)?, false)?).ok()
    }

    pub fn set(conn: &str, password: &str) -> Result<(), String> {
        let conn = clean(conn);
        let enc = crypt(password.as_bytes(), true).ok_or("kalitni shifrlab bo'lmadi (DPAPI)")?;
        let mut entries = load();
        entries.retain(|(k, _)| *k != conn);
        entries.push((conn, hexfmt::encode(&enc)));
        store(&entries)
    }

    pub fn delete(conn: &str) {
        let conn = clean(conn);
        let mut entries = load();
        let before = entries.len();
        entries.retain(|(k, _)| *k != conn);
        if entries.len() != before {
            let _ = store(&entries);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn dpapi_roundtrip_and_tamper() {
            let enc = crypt("maxfiy-kalit-123".as_bytes(), true).expect("shifrlash");
            assert!(!enc.windows(6).any(|w| w == b"maxfiy"), "ochiq matn shifrlangan blobda ko'rinmasligi kerak");
            assert_eq!(crypt(&enc, false).unwrap(), b"maxfiy-kalit-123");
            let mut bad = enc.clone();
            let last = bad.len() - 1;
            bad[last] ^= 0xff;
            assert!(crypt(&bad, false).is_none(), "buzilgan blob ochilmasligi kerak");
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::{
        io::Write,
        process::{Command, Stdio},
        sync::OnceLock,
    };

    const SERVICE: &str = "ulan-remote-desktop";

    fn tool() -> Command {
        Command::new("secret-tool")
    }

    /// `secret-tool` bor va kalit zanjiri javob beradimi (bir marta tekshiriladi).
    pub fn available() -> bool {
        static OK: OnceLock<bool> = OnceLock::new();
        *OK.get_or_init(|| {
            tool()
                .args(["search", "service", SERVICE])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok()
        })
    }

    pub fn get(conn: &str) -> Option<String> {
        if !available() {
            return None;
        }
        let out = tool().args(["lookup", "service", SERVICE, "connection", conn]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8(out.stdout).ok().map(|s| s.trim_end_matches(['\n', '\r']).to_string())
    }

    pub fn set(conn: &str, password: &str) -> Result<(), String> {
        if !available() {
            return Err("tizim kalit zanjiri topilmadi (libsecret-tools o'rnating)".into());
        }
        // Kalit buyruq satrida emas, stdin orqali beriladi (jarayonlar ro'yxatida ko'rinmaydi)
        let mut child = tool()
            .args(["store", "--label", &format!("Ulan: {conn}"), "service", SERVICE, "connection", conn])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("secret-tool ishga tushmadi: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(password.as_bytes()).map_err(|e| format!("kalit saqlanmadi: {e}"))?;
        }
        match child.wait() {
            Ok(s) if s.success() => Ok(()),
            _ => Err("kalit zanjiriga saqlab bo'lmadi".into()),
        }
    }

    pub fn delete(conn: &str) {
        if available() {
            let _ = tool()
                .args(["clear", "service", SERVICE, "connection", conn])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}
