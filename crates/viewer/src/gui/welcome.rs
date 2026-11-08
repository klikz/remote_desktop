//! Dastur ochilganda ko'rsatiladigan dialog: to'lov ma'lumoti (text, phone, card) va reklama (rasm + matn).
//! Mazmun har ochilishda serverdan olinadi. Oxirgi muvaffaqiyatli javob config papkasida saqlanadi va
//! server bilan aloqa bo'lmaganda shu kesh ko'rsatiladi.

use super::theme;
use egui::{Align2, Context, RichText, TextureHandle, TextureOptions};
use std::{
    io::Read,
    path::PathBuf,
    sync::mpsc,
    time::Duration,
};

const API: &str = "http://cloud.hubnode.uz:8780/api/public";
const TIMEOUT: Duration = Duration::from_secs(6);
/// Rasm baytlari uchun yuqori chegara (noto'g'ri javob xotirani to'ldirmasin).
const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
/// Tekstura shu o'lchamdan katta bo'lmaydi (HiDPI ekranda ham tiniq, lekin og'ir emas).
const MAX_TEX_SIDE: u32 = 900;
const CACHE_FILE: &str = "welcome_cache.json";
const BANNER_FILE: &str = "welcome_banner.bin";

#[derive(Default, Clone)]
struct Info {
    text: String,
    phone: String,
    card: String,
    ad_text: String,
}

enum Msg {
    Payment(Option<(String, String, String)>),
    AdText(Option<String>),
    /// Asl baytlar (keshga yoziladi) va dekodlangan rasm.
    Banner(Option<(Vec<u8>, egui::ColorImage)>),
}

pub struct Welcome {
    info: Info,
    banner: Option<TextureHandle>,
    rx: mpsc::Receiver<Msg>,
    /// Hali javobi kelmagan so'rovlar soni.
    pending: u8,
}

fn cache_path(name: &str) -> Option<PathBuf> {
    rdcore::paths::config_dir().ok().map(|d| d.join(name))
}

fn load_cache() -> Info {
    let Some(text) = cache_path(CACHE_FILE).and_then(|p| std::fs::read_to_string(p).ok()) else { return Info::default() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return Info::default() };
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    Info { text: s("text"), phone: s("phone"), card: s("card"), ad_text: s("ad_text") }
}

fn save_cache(info: &Info) {
    let Some(p) = cache_path(CACHE_FILE) else { return };
    let v = serde_json::json!({ "text": info.text, "phone": info.phone, "card": info.card, "ad_text": info.ad_text });
    if let Err(e) = rdcore::fsutil::write_private(&p, v.to_string().as_bytes()) {
        eprintln!("{CACHE_FILE} yozilmadi: {e}");
    }
}

/// PNG/JPEG baytlarini teksturaga tayyorlaydi; katta rasm proporsiyasi saqlangan holda kichraytiriladi.
fn decode_image(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    if w.max(h) > MAX_TEX_SIDE {
        let k = MAX_TEX_SIDE as f32 / w.max(h) as f32;
        let (nw, nh) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
        img = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
    }
    let size = [img.width() as usize, img.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()))
}

/// Oddiy HTTP/1.1 GET (`http://host:port/path`). Faqat 200 javob tanasini qaytaradi.
/// `Content-Length`, `chunked` va ulanish yopilguncha o'qish qo'llab-quvvatlanadi.
fn http_get(url: &str, limit: u64) -> Option<Vec<u8>> {
    use std::{io::Write, net::{TcpStream, ToSocketAddrs}};
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = rest.split_once('/').map(|(a, p)| (a, format!("/{p}"))).unwrap_or((rest, "/".into()));
    let host = authority.rsplit_once(':').map_or(authority, |(h, _)| h);
    let addr_str = if authority.contains(':') { authority.to_string() } else { format!("{authority}:80") };
    let addr = addr_str.to_socket_addrs().ok()?.next()?;
    let mut s = TcpStream::connect_timeout(&addr, TIMEOUT).ok()?;
    s.set_read_timeout(Some(TIMEOUT)).ok()?;
    s.set_write_timeout(Some(TIMEOUT)).ok()?;
    write!(s, "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: MS-Remote\r\nAccept: */*\r\nConnection: close\r\n\r\n").ok()?;

    let mut raw = Vec::new();
    s.take(limit + 64 * 1024).read_to_end(&mut raw).ok()?;
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let body = &raw[split + 4..];
    let mut lines = head.lines();
    let status = lines.next()?.split_whitespace().nth(1)?;
    if status != "200" {
        return None;
    }
    let header = |name: &str| lines.clone().find_map(|l| l.strip_prefix(name).map(|v| v.trim_start_matches(':').trim().to_string()));
    let out = if header("transfer-encoding").is_some_and(|v| v.contains("chunked")) {
        dechunk(body)?
    } else if let Some(n) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        body.get(..n)?.to_vec()
    } else {
        body.to_vec()
    };
    (out.len() as u64 <= limit).then_some(out)
}

fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b.windows(2).position(|w| w == b"\r\n")?;
        let size_str = std::str::from_utf8(&b[..eol]).ok()?;
        let size = usize::from_str_radix(size_str.split(';').next()?.trim(), 16).ok()?;
        b = &b[eol + 2..];
        if size == 0 {
            return Some(out);
        }
        out.extend_from_slice(b.get(..size)?);
        b = b.get(size + 2..)?;
    }
}

/// JSON javob: `"result" == "ok"` bo'lsagina qabul qilinadi.
fn get_json(path: &str) -> Option<serde_json::Value> {
    let body = http_get(&format!("{API}{path}"), 256 * 1024)?;
    let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
    (v.get("result").and_then(|r| r.as_str()) == Some("ok")).then_some(v)
}

fn field(v: &serde_json::Value, k: &str) -> String {
    match v.get(k) {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn fetch_payment() -> Option<(String, String, String)> {
    let v = get_json("/payment-info")?;
    Some((field(&v, "text"), field(&v, "phone"), field(&v, "card")))
}

fn fetch_ad_text() -> Option<String> {
    get_json("/ad-text").map(|v| field(&v, "text"))
}

fn fetch_banner() -> Option<(Vec<u8>, egui::ColorImage)> {
    let bytes = http_get(&format!("{API}/banner-image"), MAX_IMAGE_BYTES)?;
    let img = decode_image(&bytes)?;
    Some((bytes, img))
}

impl Welcome {
    pub fn new(ctx: &Context) -> Welcome {
        let info = load_cache();
        let banner = cache_path(BANNER_FILE)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| decode_image(&b))
            .map(|img| ctx.load_texture("welcome_banner", img, TextureOptions::LINEAR));

        let (tx, rx) = mpsc::channel();
        let jobs: [fn() -> Msg; 3] = [
            || Msg::Payment(fetch_payment()),
            || Msg::AdText(fetch_ad_text()),
            || Msg::Banner(fetch_banner()),
        ];
        for job in jobs {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            std::thread::spawn(move || {
                let _ = tx.send(job());
                ctx.request_repaint();
            });
        }
        Welcome { info, banner, rx, pending: jobs.len() as u8 }
    }

    fn poll(&mut self, ctx: &Context) {
        while let Ok(msg) = self.rx.try_recv() {
            self.pending = self.pending.saturating_sub(1);
            match msg {
                Msg::Payment(Some((text, phone, card))) => {
                    (self.info.text, self.info.phone, self.info.card) = (text, phone, card);
                    save_cache(&self.info);
                }
                Msg::AdText(Some(t)) => {
                    self.info.ad_text = t;
                    save_cache(&self.info);
                }
                Msg::Banner(Some((bytes, img))) => {
                    if let Some(p) = cache_path(BANNER_FILE) {
                        let _ = rdcore::fsutil::write_private(&p, &bytes);
                    }
                    match &mut self.banner {
                        Some(t) => t.set(img, TextureOptions::LINEAR),
                        None => self.banner = Some(ctx.load_texture("welcome_banner", img, TextureOptions::LINEAR)),
                    }
                }
                // Javob kelmadi: kesh (bo'lsa) qoladi.
                Msg::Payment(None) | Msg::AdText(None) | Msg::Banner(None) => {}
            }
        }
    }

    fn has_content(&self) -> bool {
        let i = &self.info;
        self.banner.is_some() || [&i.text, &i.phone, &i.card, &i.ad_text].iter().any(|s| !s.is_empty())
    }

    /// Dialogni chizadi. Yopilsa (yoki ko'rsatadigan hech narsa qolmasa) `false` qaytaradi.
    pub fn show(&mut self, ctx: &Context) -> bool {
        self.poll(ctx);
        let loading = self.pending > 0;
        if !loading && !self.has_content() {
            return false;
        }
        let mut open = true;
        let screen = ctx.screen_rect();
        egui::Window::new("Ulan")
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([440.0, 0.0])
            .show(ctx, |ui| {
                // `fixed_size` balandligi 0: ScrollArea o'z balandligini mazmundan oladi (oyna balandligidan oshmaydi)
                let max_h = (screen.height() - 120.0).max(160.0);
                egui::ScrollArea::vertical().max_height(max_h).min_scrolled_height(max_h).show(ui, |ui| {
                    if !self.has_content() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(30.0);
                            ui.add(egui::Spinner::new().size(28.0).color(theme::ACCENT));
                            ui.add_space(8.0);
                            ui.label(theme::dim("Yuklanmoqda..."));
                            ui.add_space(30.0);
                        });
                    }
                    self.body(ui, screen.height());
                });
                ui.add_space(12.0);
                if theme::accent_button(ui, "Davom etish", egui::vec2(ui.available_width(), 34.0), true).clicked() {
                    open = false;
                }
            });
        open
    }

    fn body(&self, ui: &mut egui::Ui, screen_h: f32) {
        let i = &self.info;
        if !i.text.is_empty() {
            ui.label(RichText::new(&i.text).size(16.0).strong());
            ui.add_space(8.0);
        }
        for (value, shown) in [(&i.phone, i.phone.clone()), (&i.card, spaced(&i.card))] {
            if value.is_empty() {
                continue;
            }
            ui.label(RichText::new(shown).monospace().size(15.0));
            ui.add_space(4.0);
        }
        if let Some(tex) = &self.banner {
            ui.add_space(8.0);
            let [w, h] = tex.size();
            let max_w = ui.available_width();
            let max_h = 320.0_f32.min(screen_h * 0.45).max(120.0);
            let k = (max_w / w as f32).min(max_h / h as f32).min(1.0);
            let size = egui::vec2(w as f32 * k, h as f32 * k);
            ui.vertical_centered(|ui| {
                ui.add(egui::Image::new((tex.id(), size)).fit_to_exact_size(size).rounding(10.0));
            });
        }
        if !i.ad_text.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new(&i.ad_text).size(14.0));
        }
    }
}

/// Faqat raqamlardan iborat bo'lsa 4 tadan guruhlaydi: `9860176619947672` -> `9860 1766 1994 7672`
fn spaced(n: &str) -> String {
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
        return n.to_string();
    }
    n.chars().collect::<Vec<_>>().chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}
