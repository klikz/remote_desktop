//! Server ro'yxati: ID -> (token, nom, oxirgi ko'rilgan vaqt). Matn fayl: `id token oxirgi_vaqt nom...`.

use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{auth, fsutil, hexfmt};

use super::ID_LEN;

#[derive(Clone, Debug)]
pub struct Entry {
    pub token: String,
    pub name: String,
    pub last_seen: u64,
}

pub struct Registry {
    path: Option<PathBuf>,
    map: HashMap<String, Entry>,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 9 xonali tasodifiy ID (birinchi raqam 0 emas), modul og'ishisiz.
fn random_id() -> String {
    const SPAN: u64 = 9 * 10u64.pow(ID_LEN as u32 - 1);
    loop {
        let v = u64::from_le_bytes(auth::random_bytes::<8>());
        if v < u64::MAX - u64::MAX % SPAN {
            return (10u64.pow(ID_LEN as u32 - 1) + v % SPAN).to_string();
        }
    }
}

impl Registry {
    /// `path = None`: faqat xotirada (testlar).
    pub fn load(path: Option<PathBuf>) -> Self {
        let mut map = HashMap::new();
        if let Some(text) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            for line in text.lines() {
                let mut it = line.splitn(4, ' ');
                if let (Some(id), Some(token), Some(seen)) = (it.next(), it.next(), it.next()) {
                    map.insert(
                        id.to_string(),
                        Entry {
                            token: token.to_string(),
                            last_seen: seen.parse().unwrap_or(0),
                            name: it.next().unwrap_or("").to_string(),
                        },
                    );
                }
            }
        }
        Registry { path, map }
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let mut ids: Vec<_> = self.map.keys().collect();
        ids.sort();
        let mut text = String::new();
        for id in ids {
            let e = &self.map[id];
            text.push_str(&format!("{id} {} {} {}\n", e.token, e.last_seen, e.name));
        }
        fsutil::write_private(path, text.as_bytes()).context("ro'yxat fayli yozilmadi")
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.map.get(id)
    }

    pub fn ids(&self) -> Vec<String> {
        let mut v: Vec<String> = self.map.keys().cloned().collect();
        v.sort();
        v
    }

    /// Yangi ID va token beradi.
    pub fn register(&mut self, name: &str) -> (String, String) {
        let id = loop {
            let c = random_id();
            if !self.map.contains_key(&c) {
                break c;
            }
        };
        let token = hexfmt::encode(&auth::random_bytes::<16>());
        self.map.insert(id.clone(), Entry { token: token.clone(), name: name.to_string(), last_seen: now_secs() });
        let _ = self.save();
        (id, token)
    }

    /// Nom va oxirgi ko'rilgan vaqtni yangilaydi.
    pub fn touch(&mut self, id: &str, name: Option<&str>) {
        if let Some(e) = self.map.get_mut(id) {
            e.last_seen = now_secs();
            if let Some(n) = name {
                e.name = n.to_string();
            }
            let _ = self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_nine_digits_and_unique() {
        let mut r = Registry::load(None);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let (id, _) = r.register("x");
            assert!(super::super::valid_id(&id), "{id}");
            assert!(!id.starts_with('0'));
            assert!(seen.insert(id));
        }
    }

    #[test]
    fn save_and_reload() {
        let dir = std::env::temp_dir().join(format!("rd-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("reg.txt");
        let mut r = Registry::load(Some(file.clone()));
        let (id, token) = r.register("Ali PC");
        let r2 = Registry::load(Some(file));
        let e = r2.get(&id).unwrap();
        assert_eq!(e.token, token);
        assert_eq!(e.name, "Ali PC");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
