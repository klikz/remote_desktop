//! Umumiy mantiq: transport (QUIC) va autentifikatsiya.
//!
//! Xavfsizlik modeli:
//! - Trafik QUIC/TLS 1.3 bilan shifrlangan.
//! - Host self-signed sertifikat ishlatadi. Sertifikat foydalanuvchiga ko'rsatilmaydi va saqlanmaydi:
//!   Viewer Host'ning haqiqiyligini alohida tekshirmaydi (soxta Host o'zini haqiqiydek ko'rsata oladi).
//! - Parol hech qachon yuborilmaydi: Viewer `HMAC(argon2(parol), nonce || sertifikat xeshi)` isbotini yuboradi.
//!   Xesh isbotga bog'langani uchun boshqa sertifikatli vositachi (MITM) isbotni haqiqiy Host'ga uzata olmaydi.
//! - Noto'g'ri urinishlar IP bo'yicha cheklanadi (`ratelimit`).

pub use protocol;
pub use quinn;

pub mod auth;
pub mod cert;
pub mod client;
pub mod framing;
pub mod filexfer;
pub mod fsutil;
pub mod hexfmt;
pub mod onetime;
pub mod pacing;
pub mod paths;
pub mod ratelimit;
pub mod relay;
pub mod server;
pub mod tcp_tunnel;
pub mod tls;
