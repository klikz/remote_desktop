// Ulan logotipi: `assets/logo/ulan-256.png` (scripts/make-icons.py yasaydi) dastur ichiga joylanadi.
// Oyna va tray ikonkasi uchun ishlatiladi; exe ikonkasi (.ico) `build.rs` orqali qo'shiladi.

/// `n x n` RGBA (to'g'ri alfa) rasm.
#[allow(dead_code)]
pub fn icon_rgba(n: u32) -> Vec<u8> {
    let img = image::load_from_memory_with_format(include_bytes!("../../../assets/logo/ulan-256.png"), image::ImageFormat::Png)
        .expect("ulan-256.png")
        .to_rgba8();
    if n == 256 {
        return img.into_raw();
    }
    image::imageops::resize(&img, n, n, image::imageops::FilterType::Lanczos3).into_raw()
}
