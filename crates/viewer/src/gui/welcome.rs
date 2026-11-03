//! Dastur ochilganda ko'rsatiladigan dialog: rasm, dastur haqida ikki qator va donat uchun karta raqamlari.
//! Matnlar `Info` ichida: keyinchalik ularni URL'dan olish uchun faqat `Info::load` ni almashtirish kifoya.

use super::theme;
use egui::{epaint::Vertex, Align2, Color32, Context, RichText, TextureHandle, TextureOptions};

/// Dialog mazmuni. Hozircha dasturga o'rnatilgan; keyin `INFO_URL` dan olinadi.
pub struct Info {
    pub line1: String,
    pub line2: String,
    /// Donat uchun kartalar: (nomi, raqami).
    pub cards: Vec<(String, String)>,
}

/// Mazmun olinadigan manzil (hozircha belgilanmagan: o'rnatilgan matn ishlatiladi).
pub const INFO_URL: Option<&str> = None;

impl Info {
    pub fn load() -> Info {
        // INFO_URL belgilanganda shu yerda yuklanadi; yuklanmasa, quyidagi standart matn qoladi.
        Info {
            line1: "MS Remote: boshqa kompyuterni uzoqdan, tez va xavfsiz boshqaring.".into(),
            line2: "Shifrlangan QUIC ulanishi, ID orqali yoki to'g'ridan-to'g'ri (P2P) ulanish, fayl almashinuvi. Bepul va ochiq.".into(),
            cards: vec![("Humo".into(), "9860176619947672".into()), ("Visa".into(), "4023060145526470".into())],
        }
    }
}

pub struct Welcome {
    info: Info,
    icon: TextureHandle,
}

impl Welcome {
    pub fn new(ctx: &Context) -> Welcome {
        let n = 192;
        let img = egui::ColorImage::from_rgba_unmultiplied([n as usize, n as usize], &crate::icon_draw::icon_rgba(n));
        Welcome { info: Info::load(), icon: ctx.load_texture("welcome_icon", img, TextureOptions::LINEAR) }
    }

    /// Dialogni chizadi. Yopilsa `false` qaytaradi.
    pub fn show(&self, ctx: &Context) -> bool {
        let mut open = true;
        egui::Window::new("MS Remote")
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([440.0, 0.0])
            .show(ctx, |ui| {
                self.banner(ui);
                ui.add_space(12.0);
                ui.label(RichText::new(&self.info.line1).size(16.0).strong());
                ui.add_space(4.0);
                ui.label(theme::dim(&self.info.line2));
                ui.add_space(10.0);
                ui.label(RichText::new("Loyihani qo'llab-quvvatlash (donat) uchun karta raqamlari:").size(13.0));
                ui.add_space(4.0);
                for (name, number) in &self.info.cards {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{name}:")).strong());
                        ui.label(RichText::new(spaced(number)).monospace().size(15.0));
                        if theme::button(ui, "Nusxa").clicked() {
                            ui.ctx().output_mut(|o| o.copied_text = number.clone());
                        }
                    });
                }
                ui.add_space(12.0);
                if theme::accent_button(ui, "Davom etish", egui::vec2(ui.available_width(), 34.0), true).clicked() {
                    open = false;
                }
            });
        open
    }

    /// Rasm: ikki rangli gradient fon, yarim shaffof doiralar va dastur belgisi.
    fn banner(&self, ui: &mut egui::Ui) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 150.0), egui::Sense::hover());
        let p = ui.painter_at(rect);
        let (top, bottom) = (Color32::from_rgb(52, 84, 214), Color32::from_rgb(138, 70, 210));
        let mut mesh = egui::Mesh::default();
        let uv = egui::pos2(0.0, 0.0);
        for (pos, color) in [(rect.left_top(), top), (rect.right_top(), top), (rect.right_bottom(), bottom), (rect.left_bottom(), bottom)] {
            mesh.vertices.push(Vertex { pos, uv, color });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        p.add(egui::Shape::mesh(mesh));
        for (dx, dy, r, a) in [(0.12, 0.25, 46.0, 40u8), (0.86, 0.2, 30.0, 34), (0.78, 0.85, 58.0, 28), (0.2, 0.9, 24.0, 30)] {
            let c = rect.min + egui::vec2(rect.width() * dx, rect.height() * dy);
            p.circle_filled(c, r, Color32::from_white_alpha(a));
        }
        let icon = egui::Rect::from_center_size(rect.center(), egui::vec2(96.0, 96.0));
        p.image(self.icon.id(), icon, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// `9860176619947672` -> `9860 1766 1994 7672`
fn spaced(n: &str) -> String {
    n.chars().collect::<Vec<_>>().chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ")
}
