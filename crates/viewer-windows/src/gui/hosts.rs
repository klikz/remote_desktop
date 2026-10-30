//! Saqlangan Host'lar ro'yxati (`hosts.txt`, konfiguratsiya papkasida). Parol saqlanmaydi.
//! Format: har qatorda `nom<TAB>manzil<TAB>fps<TAB>kbps<TAB>balandlik<TAB>boshqaruv(1/0)`.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct SavedHost {
    pub name: String,
    pub addr: String,
    pub fps: u16,
    pub kbps: u32,
    pub height: u16,
    pub input: bool,
}

fn file() -> Option<PathBuf> {
    rdcore::paths::config_dir().ok().map(|d| d.join("hosts.txt"))
}

pub fn load() -> Vec<SavedHost> {
    let Some(path) = file() else { return vec![] };
    let Ok(text) = std::fs::read_to_string(path) else { return vec![] };
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() < 6 || f[1].trim().is_empty() {
                return None;
            }
            Some(SavedHost {
                name: f[0].to_string(),
                addr: f[1].to_string(),
                fps: f[2].parse().unwrap_or(30),
                kbps: f[3].parse().unwrap_or(8000),
                height: f[4].parse().unwrap_or(1080),
                input: f[5] != "0",
            })
        })
        .collect()
}

pub fn save(hosts: &[SavedHost]) -> std::io::Result<()> {
    let Some(path) = file() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let text: String = hosts
        .iter()
        .map(|h| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                clean(&h.name),
                clean(&h.addr),
                h.fps,
                h.kbps,
                h.height,
                u8::from(h.input)
            )
        })
        .collect();
    std::fs::write(path, text)
}
