//! Server (relay) rejimi: clientlar serverga doimiy ulanib turadi, 9 xonali ID oladi va bir-biriga ID orqali
//! ulanadi. Server faqat ikki clientning ma'lumot oqimlarini ulab beradi; ichkarida hozirgi QUIC/TLS/parol
//! protokoli (`server.rs`/`client.rs`) o'zgarishsiz ishlaydi, shuning uchun server trafikni o'qiy olmaydi.
//!
//! ```text
//! A: quinn -> 127.0.0.1:P (UDP) -> [QUIC oqimi] -> RELAY SERVER -> [QUIC oqimi] -> 127.x (UDP) -> B Host (QUIC)
//! ```

pub mod agent;
pub mod msg;
pub mod registry;
pub mod server;

/// Relay serveri uchun ALPN (Host/Viewer ning `rd/1` idan alohida).
pub const ALPN: &[u8] = b"rd-relay/1";
pub const DEFAULT_PORT: u16 = 7780;
/// Bitta server ulanishidagi parallel oqimlar (1 control + sessiyalar).
pub const MAX_BIDI: u32 = 64;
/// ID uzunligi (raqamlar).
pub const ID_LEN: usize = 9;

/// ID to'g'ri formatdami (9 ta raqam).
pub fn valid_id(s: &str) -> bool {
    s.len() == ID_LEN && s.bytes().all(|b| b.is_ascii_digit())
}

/// `123456789` -> `123 456 789`
pub fn pretty_id(id: &str) -> String {
    if id.len() == ID_LEN {
        format!("{} {} {}", &id[..3], &id[3..6], &id[6..])
    } else {
        id.to_string()
    }
}

/// Foydalanuvchi kiritgan ID'dan bo'shliq va chiziqchalarni olib tashlaydi.
pub fn normalize_id(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_helpers() {
        assert!(valid_id("123456789"));
        assert!(!valid_id("12345678"));
        assert!(!valid_id("12345678a"));
        assert_eq!(pretty_id("123456789"), "123 456 789");
        assert_eq!(normalize_id("123 456-789"), "123456789");
    }
}
