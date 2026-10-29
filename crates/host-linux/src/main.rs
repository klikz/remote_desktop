//! Host (Ubuntu, GNOME Wayland): ulanish, autentifikatsiya va ekran videosini yuborish.
//! Kiritishni (sichqoncha/klaviatura) bajarish keyingi bosqichda qo'shiladi.

#[cfg(target_os = "linux")]
mod capture;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod portal;

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    linux::run().await
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("host-linux faqat Linux uchun.");
}
