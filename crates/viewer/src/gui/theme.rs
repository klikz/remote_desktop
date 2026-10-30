//! Interfeys uslubi: ranglar, o'lchamlar va qayta ishlatiladigan elementlar (kartalar, "chip"lar, tugmalar).

use egui::{Color32, Context, FontId, Frame, Margin, Response, RichText, Rounding, Sense, Stroke, TextStyle, Ui, Vec2};

pub const BG: Color32 = Color32::from_rgb(17, 19, 25);
pub const PANEL: Color32 = Color32::from_rgb(23, 26, 33);
pub const CARD: Color32 = Color32::from_rgb(30, 34, 43);
pub const BORDER: Color32 = Color32::from_rgb(46, 51, 63);
pub const ACCENT: Color32 = Color32::from_rgb(86, 128, 255);
pub const TEXT: Color32 = Color32::from_rgb(226, 230, 239);
pub const DIM: Color32 = Color32::from_rgb(138, 146, 163);
pub const GOOD: Color32 = Color32::from_rgb(72, 199, 128);
pub const WARN: Color32 = Color32::from_rgb(240, 178, 74);
pub const BAD: Color32 = Color32::from_rgb(238, 98, 98);

/// Umumiy uslubni o'rnatadi (qorong'i mavzu, yumaloq burchaklar, ixcham oraliqlar).
pub fn apply(ctx: &Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(13, 15, 20);
    v.faint_bg_color = CARD;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_rounding = Rounding::same(12.0);
    v.menu_rounding = Rounding::same(8.0);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;

    let r = Rounding::same(8.0);
    let idle = Color32::from_rgb(38, 43, 54);
    let hover = Color32::from_rgb(48, 54, 68);
    v.widgets.noninteractive.bg_fill = CARD;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.rounding = r;
    v.widgets.inactive.bg_fill = idle;
    v.widgets.inactive.weak_bg_fill = idle;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.rounding = r;
    v.widgets.hovered.bg_fill = hover;
    v.widgets.hovered.weak_bg_fill = hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.7));
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    v.widgets.hovered.rounding = r;
    v.widgets.active.bg_fill = ACCENT.gamma_multiply(0.6);
    v.widgets.active.weak_bg_fill = ACCENT.gamma_multiply(0.6);
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(2.0, Color32::WHITE);
    v.widgets.active.rounding = r;
    v.widgets.open.rounding = r;
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 7.0);
        s.spacing.button_padding = Vec2::new(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.window_margin = Margin::same(16.0);
        s.spacing.slider_width = 200.0;
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
    Frame::none().fill(BG).inner_margin(Margin::symmetric(24.0, 18.0))
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

/// Sarlavhali karta.
pub fn card<R>(ui: &mut Ui, title: Option<&str>, add: impl FnOnce(&mut Ui) -> R) -> R {
    let r = Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .rounding(Rounding::same(10.0))
        .inner_margin(Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(t) = title {
                ui.label(RichText::new(t).size(11.0).color(DIM).strong());
                ui.add_space(4.0);
            }
            add(ui)
        })
        .inner;
    ui.add_space(10.0);
    r
}

/// Rangli yumaloq belgi (holat uchun).
pub fn chip(ui: &mut Ui, text: impl Into<String>, color: Color32) {
    Frame::none()
        .fill(color.gamma_multiply(0.16))
        .rounding(Rounding::same(20.0))
        .inner_margin(Margin::symmetric(9.0, 3.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).size(12.0).strong().color(color));
        });
}

pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(DIM));
}

pub fn dim(text: &str) -> RichText {
    RichText::new(text).color(DIM)
}

pub fn tab(ui: &mut Ui, selected: bool, text: &str) -> Response {
    let (fill, color) = if selected { (ACCENT.gamma_multiply(0.22), Color32::WHITE) } else { (Color32::TRANSPARENT, DIM) };
    ui.add(
        egui::Button::new(RichText::new(text).strong().color(color))
            .fill(fill)
            .stroke(Stroke::NONE)
            .rounding(Rounding::same(8.0)),
    )
}

pub fn accent_button(ui: &mut Ui, text: &str, min_size: Vec2, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
            .fill(ACCENT)
            .rounding(Rounding::same(8.0))
            .min_size(min_size),
    )
}

/// Butun kenglikdagi asosiy tugma.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let w = ui.available_width();
    accent_button(ui, text, Vec2::new(w, 38.0), enabled)
}

pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
            .fill(BAD.gamma_multiply(0.85))
            .rounding(Rounding::same(8.0)),
    )
}

pub fn error_banner(ui: &mut Ui, text: &str) {
    Frame::none()
        .fill(BAD.gamma_multiply(0.14))
        .stroke(Stroke::new(1.0, BAD.gamma_multiply(0.6)))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::same(10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(BAD));
        });
}
