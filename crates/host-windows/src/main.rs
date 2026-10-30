//! Host (Windows) buyruq qatori: `host-windows set-password | run`.

#[cfg(windows)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    host_windows::run_cli().await
}

#[cfg(not(windows))]
fn main() {
    eprintln!("host-windows faqat Windows uchun.");
}
