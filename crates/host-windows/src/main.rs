//! Host (Windows): ulanishni qabul qiladi, ekran videosini yuboradi va sichqoncha/klaviaturani bajaradi.

#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod input;
#[cfg(windows)]
mod service;
#[cfg(windows)]
mod video;

#[cfg(windows)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    service::run().await
}

#[cfg(not(windows))]
fn main() {
    eprintln!("host-windows faqat Windows uchun.");
}
