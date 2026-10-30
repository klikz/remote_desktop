//! Umumiy mantiq: transport (QUIC), autentifikatsiya, ishonch (fingerprint).
//!
//! Xavfsizlik modeli:
//! - Trafik QUIC/TLS 1.3 bilan shifrlangan.
//! - Host self-signed sertifikat ishlatadi. Viewer birinchi ulanishda fingerprint'ni tasdiqlaydi
//!   (`known_hosts`), keyingi ulanishlarda fingerprint o'zgarsa ulanish rad etiladi.
//! - Parol hech qachon yuborilmaydi: Viewer `HMAC(argon2(parol), nonce || fingerprint)` isbotini yuboradi.
//!   Fingerprint isbotga bog'langani uchun boshqa sertifikatli vositachi (MITM) isbotni to'g'ri hisoblay olmaydi.
//! - Noto'g'ri urinishlar IP bo'yicha cheklanadi (`ratelimit`).

pub use protocol;
pub use quinn;

pub mod auth;
pub mod cert;
pub mod client;
pub mod framing;
pub mod fsutil;
pub mod hexfmt;
pub mod known_hosts;
pub mod onetime;
pub mod paths;
pub mod ratelimit;
pub mod relay;
pub mod server;
pub mod tcp_tunnel;
pub mod tls;
