//! Windows Host kutubxonasi: ekran olish, kodlash, kiritish va dastur ichidan boshqariladigan Host.

#![cfg(windows)]

mod capture;
mod input;
mod service;
mod video;

pub use service::{run_cli, PeerInfo, RunningHost};
