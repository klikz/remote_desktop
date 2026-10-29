//! Hex yordamchilari (tashqi kutubxonasiz).

pub fn encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

pub fn decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let b = s.as_bytes();
    for i in (0..b.len()).step_by(2) {
        let hi = (b[i] as char).to_digit(16)?;
        let lo = (b[i + 1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
    }
    Some(out)
}

/// Fingerprint ko'rinishi: `AB:CD:EF:...`
pub fn colon(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let data = [0u8, 1, 15, 16, 255, 128];
        assert_eq!(encode(&data), "00010f10ff80");
        assert_eq!(decode("00010f10ff80").unwrap(), data);
        assert_eq!(decode("00010F10FF80").unwrap(), data);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(decode("abc").is_none());
        assert!(decode("zz").is_none());
        assert!(decode("é1").is_none());
        assert_eq!(decode("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn colon_format() {
        assert_eq!(colon(&[0xab, 0x01, 0xff]), "AB:01:FF");
        assert_eq!(colon(&[]), "");
    }
}
