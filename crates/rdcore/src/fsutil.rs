//! Fayl yordamchilari.

use std::{fs, io, io::Write, path::Path};

/// Faylni faqat egasi o'qiy oladigan qilib yozadi (Unix: 0600). Yozish atomik: avval yonidagi vaqtinchalik
/// faylga yoziladi, so'ng nomi almashtiriladi. Jarayon yozish o'rtasida to'xtab qolsa (o'chirish, tok uzilishi)
/// eski fayl butun qoladi: aks holda sertifikat, kalit yoki relay ro'yxati yarim yozilgan holda qolardi.
pub fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Bir jarayondagi parallel yozuvlar bir-birining vaqtinchalik faylini bosmasin
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(format!(".{}-{seq}.tmp", std::process::id()));
    let tmp = path.with_file_name(tmp_name);
    let atomic = write_file(&tmp, data).and_then(|()| fs::rename(&tmp, path));
    if atomic.is_ok() {
        return Ok(());
    }
    // Nomni almashtirib bo'lmadi (Windows: fayl boshqa jarayonda ochiq) yoki papkaga yangi fayl yozib bo'lmaydi:
    // to'g'ridan-to'g'ri ustiga yoziladi.
    let _ = fs::remove_file(&tmp);
    write_file(path, data)
}

fn write_file(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(data)?;
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_overwrites() {
        let dir = std::env::temp_dir().join(format!("rd-fsutil-{}", std::process::id()));
        let file = dir.join("sub").join("secret.bin");
        write_private(&file, b"one").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"one");
        write_private(&file, b"2").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"2");
        // vaqtinchalik fayl qolmaydi
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
