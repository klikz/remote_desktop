//! Windows Host kutubxonasi: ekran olish, kodlash, kiritish va dastur ichidan boshqariladigan Host.

#![cfg(windows)]

mod capture;
mod desktop;
mod input;
mod service;
mod video;
mod winservice;

pub use service::{run_cli, PeerInfo, RunningHost};
