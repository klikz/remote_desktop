//! Host va Viewer o'rtasidagi umumiy protokol.
//!
//! Format qo'lda yozilgan binary (tashqi kutubxonasiz), barcha butun sonlar big-endian.
//! Har xabar frame ichida yuboriladi: `u32 uzunlik` + `payload` (payload birinchi bayti: xabar turi).
//! Bitta QUIC ulanish ichida alohida oqimlar (kanallar) ishlatiladi.

use std::fmt;

/// Protokol versiyasi. Mos kelmasa, ulanish handshake'da rad etiladi.
pub const PROTOCOL_VERSION: u16 = 1;
/// Host tinglaydigan standart port (UDP).
pub const DEFAULT_PORT: u16 = 7777;
/// QUIC ALPN identifikatori.
pub const ALPN: &[u8] = b"rd/1";
/// TLS server nomi (sertifikat tekshiruvi ishlatilmaydi, faqat handshake uchun).
pub const SERVER_NAME: &str = "remote-desktop";
/// Bitta control frame'ning maksimal hajmi (xotirani suiiste'mol qilishdan himoya).
pub const MAX_FRAME_LEN: usize = 1 << 20;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 32;
pub const PROOF_LEN: usize = 32;
/// `Capabilities::codecs` bitmaskidagi bitlar.
pub const CODEC_H264: u8 = 1;

/// `AuthResult::reason` kodlari.
pub mod reason {
    pub const OK: u8 = 0;
    pub const BAD_PASSWORD: u8 = 1;
    pub const BLOCKED: u8 = 2;
    pub const VERSION_MISMATCH: u8 = 3;
    pub const REJECTED: u8 = 4;

    pub fn describe(code: u8) -> &'static str {
        match code {
            OK => "ok",
            BAD_PASSWORD => "noto'g'ri parol",
            BLOCKED => "ko'p noto'g'ri urinish, vaqtincha bloklangan",
            VERSION_MISMATCH => "protokol versiyasi mos kelmaydi",
            REJECTED => "Host ulanishni rad etdi",
            _ => "noma'lum sabab",
        }
    }
}

/// `VideoFailed::reason` kodlari.
pub mod video_reason {
    pub const NO_CAPTURE: u8 = 1;
    pub const NO_ENCODER: u8 = 2;
    pub const BUSY: u8 = 3;
    pub const BAD_REQUEST: u8 = 4;

    pub fn describe(code: u8) -> &'static str {
        match code {
            NO_CAPTURE => "ekranni olib bo'lmadi (ekran qulflangan yoki portal ruxsat bermadi)",
            NO_ENCODER => "H.264 kodlagich topilmadi",
            BUSY => "video allaqachon yuborilmoqda",
            BAD_REQUEST => "noto'g'ri so'rov",
            _ => "noma'lum sabab",
        }
    }
}

/// Video kadr sarlavhasi uzunligi (baytlarda).
pub const VIDEO_HEADER_LEN: usize = 13;
/// Bitta kodlangan kadrning maksimal hajmi.
pub const MAX_VIDEO_FRAME_LEN: usize = 16 << 20;

/// Video oqimidagi har kadr oldidan keladi: `bayroqlar(1) + pts_us(8) + uzunlik(4)`, so'ng kadr baytlari.
/// Bayroqlar: bit0 = kalit kadr (IDR).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoHeader {
    pub keyframe: bool,
    /// Taqdimot vaqti, mikrosekundlarda (Host soatiga nisbatan).
    pub pts_us: u64,
    /// Kadr baytlari soni.
    pub len: u32,
}

impl VideoHeader {
    pub fn encode(&self) -> [u8; VIDEO_HEADER_LEN] {
        let mut out = [0u8; VIDEO_HEADER_LEN];
        out[0] = u8::from(self.keyframe);
        out[1..9].copy_from_slice(&self.pts_us.to_be_bytes());
        out[9..13].copy_from_slice(&self.len.to_be_bytes());
        out
    }

    pub fn decode(b: &[u8; VIDEO_HEADER_LEN]) -> Result<VideoHeader, DecodeError> {
        let mut pts = [0u8; 8];
        pts.copy_from_slice(&b[1..9]);
        let len = u32::from_be_bytes([b[9], b[10], b[11], b[12]]);
        if len as usize > MAX_VIDEO_FRAME_LEN {
            return Err(DecodeError::BadFrameLength(len as usize));
        }
        Ok(VideoHeader {
            keyframe: b[0] & 1 == 1,
            pts_us: u64::from_be_bytes(pts),
            len,
        })
    }
}

/// Ulanish ichidagi kanallar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Handshake, sozlamalar, ping.
    Control,
    /// Kodlangan video kadrlar (Host -> Viewer).
    Video,
    /// Sichqoncha va klaviatura (Viewer -> Host).
    Input,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

/// Host e'lon qiladigan imkoniyatlar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// Qo'llab-quvvatlanadigan kodeklar bitmaski (`CODEC_*`).
    pub codecs: u8,
    pub max_fps: u16,
    pub monitors: Vec<MonitorInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Viewer -> Host: birinchi xabar.
    ClientHello { version: u16 },
    /// Host -> Viewer: parol xeshlash tuzi va bir martalik nonce.
    ServerHello {
        version: u16,
        salt: [u8; SALT_LEN],
        nonce: [u8; NONCE_LEN],
    },
    /// Viewer -> Host: parolni bilishini isbotlaydi (parolning o'zi yuborilmaydi).
    AuthProof { proof: [u8; PROOF_LEN] },
    /// Host -> Viewer: autentifikatsiya natijasi.
    AuthResult { ok: bool, reason: u8 },
    /// Host -> Viewer: imkoniyatlar.
    Capabilities(Capabilities),
    Ping { id: u64, sent_ms: u64 },
    Pong { id: u64, sent_ms: u64 },
    /// Ulanishni tartibli yopish.
    Bye { reason: u8 },
    /// Viewer -> Host: videoni boshlashni so'raydi. `max_height`: 0 = asl o'lcham, aks holda shu balandlikka kichraytiriladi.
    VideoRequest { fps: u16, kbps: u32, max_height: u16, monitor: u8 },
    /// Host -> Viewer: video boshlandi. Kadrlar Host ochgan bir yo'nalishli oqimda keladi.
    VideoStarted { codec: u8, fps: u16, kbps: u32 },
    /// Host -> Viewer: videoni boshlab bo'lmadi (`video_reason`).
    VideoFailed { reason: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Ma'lumot tugab qoldi.
    Truncated,
    UnknownTag(u8),
    BadValue,
    /// Frame uzunligi 0 yoki `MAX_FRAME_LEN` dan katta.
    BadFrameLength(usize),
    /// Xabar tugagandan keyin ortiqcha baytlar qolgan.
    TrailingBytes,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "xabar to'liq emas"),
            DecodeError::UnknownTag(t) => write!(f, "noma'lum xabar turi: {t}"),
            DecodeError::BadValue => write!(f, "noto'g'ri qiymat"),
            DecodeError::BadFrameLength(n) => write!(f, "noto'g'ri frame uzunligi: {n}"),
            DecodeError::TrailingBytes => write!(f, "xabar oxirida ortiqcha baytlar"),
        }
    }
}

impl std::error::Error for DecodeError {}

const TAG_CLIENT_HELLO: u8 = 1;
const TAG_SERVER_HELLO: u8 = 2;
const TAG_AUTH_PROOF: u8 = 3;
const TAG_AUTH_RESULT: u8 = 4;
const TAG_CAPABILITIES: u8 = 5;
const TAG_PING: u8 = 6;
const TAG_PONG: u8 = 7;
const TAG_BYE: u8 = 8;
const TAG_VIDEO_REQUEST: u8 = 9;
const TAG_VIDEO_STARTED: u8 = 10;
const TAG_VIDEO_FAILED: u8 = 11;

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Truncated)?;
        if end > self.buf.len() {
            return Err(DecodeError::Truncated);
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_be_bytes(a))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let b = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(b);
        Ok(a)
    }

    fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::BadValue),
        }
    }

    fn finish(&self) -> Result<(), DecodeError> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

impl Message {
    /// Xabarni (frame uzunligisiz) baytlarga aylantiradi.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Message::ClientHello { version } => {
                out.push(TAG_CLIENT_HELLO);
                out.extend_from_slice(&version.to_be_bytes());
            }
            Message::ServerHello { version, salt, nonce } => {
                out.push(TAG_SERVER_HELLO);
                out.extend_from_slice(&version.to_be_bytes());
                out.extend_from_slice(salt);
                out.extend_from_slice(nonce);
            }
            Message::AuthProof { proof } => {
                out.push(TAG_AUTH_PROOF);
                out.extend_from_slice(proof);
            }
            Message::AuthResult { ok, reason } => {
                out.push(TAG_AUTH_RESULT);
                out.push(u8::from(*ok));
                out.push(*reason);
            }
            Message::Capabilities(c) => {
                out.push(TAG_CAPABILITIES);
                out.push(c.codecs);
                out.extend_from_slice(&c.max_fps.to_be_bytes());
                let monitors: Vec<&MonitorInfo> = c.monitors.iter().take(255).collect();
                out.push(monitors.len() as u8);
                for m in monitors {
                    out.extend_from_slice(&m.id.to_be_bytes());
                    out.extend_from_slice(&m.width.to_be_bytes());
                    out.extend_from_slice(&m.height.to_be_bytes());
                    out.push(u8::from(m.primary));
                }
            }
            Message::Ping { id, sent_ms } => {
                out.push(TAG_PING);
                out.extend_from_slice(&id.to_be_bytes());
                out.extend_from_slice(&sent_ms.to_be_bytes());
            }
            Message::Pong { id, sent_ms } => {
                out.push(TAG_PONG);
                out.extend_from_slice(&id.to_be_bytes());
                out.extend_from_slice(&sent_ms.to_be_bytes());
            }
            Message::Bye { reason } => {
                out.push(TAG_BYE);
                out.push(*reason);
            }
            Message::VideoRequest { fps, kbps, max_height, monitor } => {
                out.push(TAG_VIDEO_REQUEST);
                out.extend_from_slice(&fps.to_be_bytes());
                out.extend_from_slice(&kbps.to_be_bytes());
                out.extend_from_slice(&max_height.to_be_bytes());
                out.push(*monitor);
            }
            Message::VideoStarted { codec, fps, kbps } => {
                out.push(TAG_VIDEO_STARTED);
                out.push(*codec);
                out.extend_from_slice(&fps.to_be_bytes());
                out.extend_from_slice(&kbps.to_be_bytes());
            }
            Message::VideoFailed { reason } => {
                out.push(TAG_VIDEO_FAILED);
                out.push(*reason);
            }
        }
        out
    }

    /// `encode` natijasini qaytaradi. Butun bufer iste'mol qilinishi shart.
    pub fn decode(buf: &[u8]) -> Result<Message, DecodeError> {
        let mut r = Reader::new(buf);
        let tag = r.u8()?;
        let msg = match tag {
            TAG_CLIENT_HELLO => Message::ClientHello { version: r.u16()? },
            TAG_SERVER_HELLO => Message::ServerHello {
                version: r.u16()?,
                salt: r.array::<SALT_LEN>()?,
                nonce: r.array::<NONCE_LEN>()?,
            },
            TAG_AUTH_PROOF => Message::AuthProof {
                proof: r.array::<PROOF_LEN>()?,
            },
            TAG_AUTH_RESULT => Message::AuthResult {
                ok: r.bool()?,
                reason: r.u8()?,
            },
            TAG_CAPABILITIES => {
                let codecs = r.u8()?;
                let max_fps = r.u16()?;
                let count = r.u8()? as usize;
                let mut monitors = Vec::with_capacity(count);
                for _ in 0..count {
                    monitors.push(MonitorInfo {
                        id: r.u32()?,
                        width: r.u32()?,
                        height: r.u32()?,
                        primary: r.bool()?,
                    });
                }
                Message::Capabilities(Capabilities {
                    codecs,
                    max_fps,
                    monitors,
                })
            }
            TAG_PING => Message::Ping {
                id: r.u64()?,
                sent_ms: r.u64()?,
            },
            TAG_PONG => Message::Pong {
                id: r.u64()?,
                sent_ms: r.u64()?,
            },
            TAG_BYE => Message::Bye { reason: r.u8()? },
            TAG_VIDEO_REQUEST => Message::VideoRequest {
                fps: r.u16()?,
                kbps: r.u32()?,
                max_height: r.u16()?,
                monitor: r.u8()?,
            },
            TAG_VIDEO_STARTED => Message::VideoStarted {
                codec: r.u8()?,
                fps: r.u16()?,
                kbps: r.u32()?,
            },
            TAG_VIDEO_FAILED => Message::VideoFailed { reason: r.u8()? },
            other => return Err(DecodeError::UnknownTag(other)),
        };
        r.finish()?;
        Ok(msg)
    }
}

/// Xabarni frame ko'rinishiga o'raydi: `u32 uzunlik` + payload.
pub fn encode_frame(msg: &Message) -> Vec<u8> {
    let payload = msg.encode();
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&payload);
    out
}

/// Bufer boshidan bitta frame'ni o'qiydi.
/// `Ok(None)`: ma'lumot hali to'liq emas. `Ok(Some((xabar, iste'mol qilingan baytlar)))`.
pub fn decode_frame(buf: &[u8]) -> Result<Option<(Message, usize)>, DecodeError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len == 0 || len > MAX_FRAME_LEN {
        return Err(DecodeError::BadFrameLength(len));
    }
    let total = 4 + len;
    if buf.len() < total {
        return Ok(None);
    }
    let msg = Message::decode(&buf[4..total])?;
    Ok(Some((msg, total)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<Message> {
        vec![
            Message::ClientHello { version: 1 },
            Message::ServerHello {
                version: 1,
                salt: [7; SALT_LEN],
                nonce: [9; NONCE_LEN],
            },
            Message::AuthProof { proof: [3; PROOF_LEN] },
            Message::AuthResult { ok: true, reason: reason::OK },
            Message::AuthResult { ok: false, reason: reason::BAD_PASSWORD },
            Message::Capabilities(Capabilities {
                codecs: CODEC_H264,
                max_fps: 60,
                monitors: vec![],
            }),
            Message::Capabilities(Capabilities {
                codecs: CODEC_H264,
                max_fps: 30,
                monitors: vec![
                    MonitorInfo { id: 0, width: 1920, height: 1080, primary: true },
                    MonitorInfo { id: 1, width: 2560, height: 1440, primary: false },
                ],
            }),
            Message::Ping { id: u64::MAX, sent_ms: 123456789 },
            Message::Pong { id: 42, sent_ms: 0 },
            Message::Bye { reason: 0 },
            Message::VideoRequest { fps: 30, kbps: 4000, max_height: 1080, monitor: 1 },
            Message::VideoStarted { codec: CODEC_H264, fps: 60, kbps: u32::MAX },
            Message::VideoFailed { reason: video_reason::NO_CAPTURE },
        ]
    }

    #[test]
    fn roundtrip_all_messages() {
        for m in samples() {
            let enc = m.encode();
            assert_eq!(Message::decode(&enc).unwrap(), m, "{m:?}");
        }
    }

    #[test]
    fn frame_roundtrip() {
        for m in samples() {
            let f = encode_frame(&m);
            let (got, used) = decode_frame(&f).unwrap().unwrap();
            assert_eq!(got, m);
            assert_eq!(used, f.len());
        }
    }

    #[test]
    fn frame_incomplete_returns_none() {
        let f = encode_frame(&Message::Ping { id: 1, sent_ms: 2 });
        for cut in 0..f.len() {
            assert_eq!(decode_frame(&f[..cut]).unwrap(), None, "cut={cut}");
        }
    }

    #[test]
    fn two_frames_back_to_back() {
        let a = Message::Ping { id: 1, sent_ms: 1 };
        let b = Message::Pong { id: 1, sent_ms: 1 };
        let mut buf = encode_frame(&a);
        buf.extend(encode_frame(&b));
        let (m1, n1) = decode_frame(&buf).unwrap().unwrap();
        assert_eq!(m1, a);
        let (m2, n2) = decode_frame(&buf[n1..]).unwrap().unwrap();
        assert_eq!(m2, b);
        assert_eq!(n1 + n2, buf.len());
    }

    #[test]
    fn truncated_payload_is_error() {
        for m in samples() {
            let enc = m.encode();
            for cut in 0..enc.len() {
                assert!(Message::decode(&enc[..cut]).is_err(), "{m:?} cut={cut}");
            }
        }
    }

    #[test]
    fn unknown_tag_is_error() {
        assert_eq!(Message::decode(&[200]), Err(DecodeError::UnknownTag(200)));
        assert_eq!(Message::decode(&[0]), Err(DecodeError::UnknownTag(0)));
    }

    #[test]
    fn trailing_bytes_are_error() {
        let mut enc = Message::Bye { reason: 0 }.encode();
        enc.push(0);
        assert_eq!(Message::decode(&enc), Err(DecodeError::TrailingBytes));
    }

    #[test]
    fn bad_bool_is_error() {
        assert_eq!(
            Message::decode(&[TAG_AUTH_RESULT, 2, 0]),
            Err(DecodeError::BadValue)
        );
    }

    #[test]
    fn bad_frame_length_is_error() {
        assert_eq!(
            decode_frame(&[0, 0, 0, 0, 1]),
            Err(DecodeError::BadFrameLength(0))
        );
        let big = (MAX_FRAME_LEN as u32 + 1).to_be_bytes();
        assert_eq!(
            decode_frame(&[big[0], big[1], big[2], big[3]]),
            Err(DecodeError::BadFrameLength(MAX_FRAME_LEN + 1))
        );
    }

    #[test]
    fn capabilities_monitor_count_is_capped() {
        let monitors = (0..300)
            .map(|i| MonitorInfo { id: i, width: 1, height: 1, primary: false })
            .collect();
        let m = Message::Capabilities(Capabilities { codecs: 1, max_fps: 1, monitors });
        match Message::decode(&m.encode()).unwrap() {
            Message::Capabilities(c) => assert_eq!(c.monitors.len(), 255),
            _ => panic!("kutilmagan xabar"),
        }
    }

    #[test]
    fn video_header_roundtrip() {
        for (key, pts, len) in [(true, 0u64, 0u32), (false, u64::MAX, 1234), (true, 42, MAX_VIDEO_FRAME_LEN as u32)] {
            let h = VideoHeader { keyframe: key, pts_us: pts, len };
            assert_eq!(VideoHeader::decode(&h.encode()).unwrap(), h);
        }
    }

    #[test]
    fn video_header_rejects_huge_frames() {
        let h = VideoHeader { keyframe: true, pts_us: 0, len: MAX_VIDEO_FRAME_LEN as u32 + 1 };
        assert_eq!(
            VideoHeader::decode(&h.encode()),
            Err(DecodeError::BadFrameLength(MAX_VIDEO_FRAME_LEN + 1))
        );
    }

    #[test]
    fn video_reason_descriptions_exist() {
        for code in 0..=5u8 {
            assert!(!video_reason::describe(code).is_empty());
        }
    }

    #[test]
    fn reason_descriptions_exist() {
        for code in 0..=5u8 {
            assert!(!reason::describe(code).is_empty());
        }
    }
}
