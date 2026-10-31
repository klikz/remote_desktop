//! Android client yadrosi: `rdcore` ustidagi UI'siz sessiya va JNI sirt.
//! Video dekodlash Kotlin tomonda (MediaCodec): bu yerda faqat Annex-B kadrlar uzatiladi.

mod jni_api;
mod session;

pub use session::*;
