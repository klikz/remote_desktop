//! Fayl yordamchilari.

use std::{fs, io, io::Write, path::Path};

/// Faylni faqat egasi o'qiy oladigan qilib yozadi (Unix'da 0600). Mavjud bo'lsa, ustiga yozadi.
pub fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(data)?;
    f.flush()
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
