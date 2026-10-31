//! Interfeys uslubi: ranglar, o'lchamlar va qayta ishlatiladigan elementlar (kartalar, maydonlar, tugmalar,
//! kalitlar, "chip"lar). Hamma bo'lim shu yerdagi elementlardan foydalanadi, shunda ko'rinish bir xil bo'ladi.

use egui::{
    Color32, Context, FontId, Frame, Margin, Response, RichText, Rounding, Sense, Stroke, TextBuffer, TextEdit, TextStyle, Ui, Vec2,
    WidgetText,
};

pub const BG: Color32 = Color32::from_rgb(17, 19, 25);
pub const PANEL: Color32 = Color32::from_rgb(23, 26, 33);
pub const CARD: Color32 = Color32::from_rgb(30, 34, 43);
pub const BORDER: Color32 = Color32::from_rgb(46, 51, 63);
/// Kiritish maydonlari, radio va ikkinchi darajali tugmalar chegarasi (kartadan ajralib turishi uchun).
pub const BORDER_STRONG: Color32 = Color32::from_rgb(64, 71, 87);
/// Kiritish maydoni foni.
pub const INPUT: Color32 = Color32::from_rgb(21, 24, 31);
pub const ACCENT: Color32 = Color32::from_rgb(86, 128, 255);
pub const TEXT: Color32 = Color32::from_rgb(226, 230, 239);
pub const DIM: Color32 = Color32::from_rgb(138, 146, 163);
pub const GOOD: Color32 = Color32::from_rgb(72, 199, 128);
pub const WARN: Color32 = Color32::from_rgb(240, 178, 74);
pub const BAD: Color32 = Color32::from_rgb(238, 98, 98);

/// Kiritish maydonlari va tugmalarning balandligi (bir qatorda bir xil ko'rinishi uchun).
pub const FIELD_H: f32 = 34.0;
/// Shakl (forma) qatorlaridagi standart maydon kengligi.
pub const FIELD_W: f32 = 300.0;
/// Shakl qatorlaridagi yorliq ustuni kengligi.
pub const LABEL_W: f32 = 110.0;
const R: f32 = 8.0;

/// Umumiy uslubni o'rnatadi (qorong'i mavzu, yumaloq burchaklar, bir xil o'lchamlar).
pub fn apply(ctx: &Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = INPUT;
    v.faint_bg_color = CARD;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = Rounding::same(R);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;

    let r = Rounding::same(R);
    let idle = Color32::from_rgb(38, 43, 54);
    let hover = Color32::from_rgb(46, 52, 65);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.rounding = r;
    v.widgets.inactive.bg_fill = idle;
    v.widgets.inactive.weak_bg_fill = idle;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.rounding = r;
    // Hover'da chegara faqat biroz yorishadi, qalinligi va o'lchami o'zgarmaydi (maydonlar "sakramaydi"); fokus — ACCENT
    v.widgets.hovered.bg_fill = hover;
    v.widgets.hovered.weak_bg_fill = hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(88, 96, 116));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.hovered.rounding = r;
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.bg_fill = ACCENT.gamma_multiply(0.6);
    v.widgets.active.weak_bg_fill = ACCENT.gamma_multiply(0.6);
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.rounding = r;
    v.widgets.active.expansion = 0.0;
    v.widgets.open.rounding = r;
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.button_padding = Vec2::new(14.0, 7.0);
        s.spacing.interact_size.y = FIELD_H;
        s.spacing.icon_width = 18.0;
        s.spacing.icon_width_inner = 10.0;
        s.spacing.window_margin = Margin::same(18.0);
        s.spacing.slider_width = 220.0;
        // Aylantirish chizig'i kontent ustiga chiqmaydi (kartalar o'ng chetiga tegib qolmaydi)
        s.spacing.scroll = egui::style::ScrollStyle::solid();
        s.text_styles = [
            (TextStyle::Heading, FontId::proportional(20.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
    });
}

pub fn page_frame() -> Frame {
    Frame::none().fill(BG).inner_margin(Margin::symmetric(28.0, 22.0))
}

pub fn side_frame() -> Frame {
    Frame::none().fill(PANEL).stroke(Stroke::new(1.0, BORDER)).inner_margin(Margin::same(14.0))
}

pub fn header_frame() -> Frame {
    Frame::none().fill(PANEL).stroke(Stroke::new(1.0, BORDER)).inner_margin(Margin::symmetric(16.0, 8.0))
}

pub fn bar_frame() -> Frame {
    Frame::none().fill(PANEL).inner_margin(Margin::symmetric(10.0, 5.0))
}

/// Sahifa sarlavhasi va izohlari (hamma bo'limda bir xil).
pub fn page_title(ui: &mut Ui, title: &str, hints: &[&str]) {
    ui.label(RichText::new(title).size(22.0).strong().color(TEXT));
    for h in hints {
        hint(ui, h);
    }
    ui.add_space(14.0);
}

/// Sarlavhali karta. Kengligi qat'iy: ichidagi uzun matn kartani kengaytirmaydi (o'raladi).
pub fn card<R>(ui: &mut Ui, title: Option<&str>, add: impl FnOnce(&mut Ui) -> R) -> R {
    let r = Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::symmetric(18.0, 16.0))
        .show(ui, |ui| {
            let w = ui.available_width();
            ui.set_width(w);
            ui.set_max_width(w);
            if let Some(t) = title {
                ui.label(RichText::new(t).size(11.5).color(DIM).strong());
                ui.add_space(6.0);
            }
            add(ui)
        })
        .inner;
    ui.add_space(12.0);
    r
}

/// Rangli yumaloq belgi (holat uchun).
pub fn chip(ui: &mut Ui, text: impl Into<String>, color: Color32) {
    Frame::none()
        .fill(color.gamma_multiply(0.16))
        .rounding(Rounding::same(20.0))
        .inner_margin(Margin::symmetric(10.0, 4.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).size(12.0).strong().color(color));
        });
}

pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(DIM)).wrap());
}

/// Holat/xabar matni: rangli, kerak bo'lsa o'raladi (kartani kengaytirmaydi).
pub fn note(ui: &mut Ui, text: &str, color: Color32) {
    ui.add(egui::Label::new(RichText::new(text).color(color)).wrap());
}

pub fn dim(text: &str) -> RichText {
    RichText::new(text).color(DIM)
}

/// Standart bir qatorli kiritish maydoni (balandligi tugmalar bilan bir xil, kengligi `FIELD_W`).
pub fn input(text: &mut dyn TextBuffer) -> TextEdit<'_> {
    TextEdit::singleline(text)
        .margin(Margin::symmetric(10.0, 8.0))
        .min_size(Vec2::new(0.0, FIELD_H))
        .vertical_align(egui::Align::Center)
        .desired_width(FIELD_W)
}

/// Ikki ustunli shakl (yorliq | maydon): hamma bo'limda bir xil ustun kengligi va oraliqlar.
pub fn form<R>(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Grid::new(id).num_columns(2).spacing([14.0, 10.0]).min_col_width(LABEL_W).show(ui, add).inner
}

/// Ikkinchi darajali tugma (balandligi maydonlar bilan bir xil).
pub fn button(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    ui.add(egui::Button::new(text).min_size(Vec2::new(0.0, FIELD_H)).rounding(Rounding::same(R)))
}

/// Tanlanadigan "pill" (sifat shablonlari, transport va h.k.).
pub fn pill(ui: &mut Ui, selected: bool, text: &str) -> Response {
    let (fill, stroke, color) = if selected {
        (ACCENT.gamma_multiply(0.22), ACCENT, Color32::WHITE)
    } else {
        (Color32::TRANSPARENT, BORDER_STRONG, TEXT)
    };
    ui.add(
        egui::Button::new(RichText::new(text).color(color))
            .fill(fill)
            .stroke(Stroke::new(1.0, stroke))
            .rounding(Rounding::same(16.0))
            .min_size(Vec2::new(0.0, 32.0)),
    )
}

/// Yoqish/o'chirish kaliti (katakcha o'rniga). Bosilganda `changed()` qaytaradi.
pub fn toggle(ui: &mut Ui, on: &mut bool, text: impl Into<WidgetText>) -> Response {
    ui.horizontal(|ui| {
        let size = Vec2::new(38.0, 22.0);
        let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
        let label = ui.add(egui::Label::new(text).sense(Sense::click()));
        if resp.clicked() || label.clicked() {
            *on = !*on;
            resp.mark_changed();
        }
        if ui.is_rect_visible(rect) {
            let t = ui.ctx().animate_bool_responsive(resp.id, *on);
            let mut bg = lerp_color(Color32::from_rgb(52, 58, 72), ACCENT, t);
            if !ui.is_enabled() {
                bg = bg.gamma_multiply(0.5);
            }
            let hovered = (resp.hovered() || label.hovered()) && ui.is_enabled();
            let stroke = if hovered { Stroke::new(1.0, Color32::from_rgb(110, 120, 142)) } else { Stroke::NONE };
            ui.painter().rect(rect, Rounding::same(size.y / 2.0), bg, stroke);
            let r = size.y / 2.0 - 3.0;
            let x = egui::lerp((rect.left() + r + 3.0)..=(rect.right() - r - 3.0), t);
            ui.painter().circle_filled(egui::pos2(x, rect.center().y), r, Color32::WHITE);
        }
        resp | label
    })
    .inner
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

pub fn tab(ui: &mut Ui, selected: bool, text: &str) -> Response {
    let (fill, color) = if selected { (ACCENT.gamma_multiply(0.22), Color32::WHITE) } else { (Color32::TRANSPARENT, DIM) };
    ui.add(
        egui::Button::new(RichText::new(text).strong().color(color))
            .fill(fill)
            .stroke(Stroke::NONE)
            .rounding(Rounding::same(R))
            .min_size(Vec2::new(0.0, 32.0)),
    )
}

pub fn accent_button(ui: &mut Ui, text: &str, min_size: Vec2, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
            .fill(ACCENT)
            .stroke(Stroke::NONE)
            .rounding(Rounding::same(R))
            .min_size(Vec2::new(min_size.x, min_size.y.max(FIELD_H))),
    )
}

/// Butun kenglikdagi asosiy tugma.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let w = ui.available_width();
    accent_button(ui, text, Vec2::new(w, 40.0), enabled)
}

pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
            .fill(BAD.gamma_multiply(0.85))
            .stroke(Stroke::NONE)
            .rounding(Rounding::same(R))
            .min_size(Vec2::new(0.0, FIELD_H)),
    )
}

pub fn error_banner(ui: &mut Ui, text: &str) {
    Frame::none()
        .fill(BAD.gamma_multiply(0.14))
        .stroke(Stroke::new(1.0, BAD.gamma_multiply(0.6)))
        .rounding(Rounding::same(R))
        .inner_margin(Margin::same(10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).color(BAD)).wrap());
        });
}

/// Shakl jadvalidan tashqaridagi qator yorlig'i: `form` dagi kabi chapga tekislangan, kengligi `LABEL_W`.
pub fn row_label(ui: &mut Ui, text: &str) {
    ui.allocate_ui_with_layout(Vec2::new(LABEL_W, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(Vec2::new(LABEL_W, FIELD_H));
        ui.label(dim(text));
    });
}
