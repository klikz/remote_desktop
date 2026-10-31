//! Tizim paneli (tray) ikonkasi (Windows). Oyna yopilganda/pastga tushirilganda agent ishlashda qolishi
//! uchun dastur trayga yashirinadi. Ikonka: bosilsa oyna ochiladi, menyuda "Ochish" va "Chiqish".

use std::sync::mpsc::{channel, Receiver};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

pub enum Cmd {
    Show,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    status: MenuItem,
    rx: Receiver<Cmd>,
    last: String,
}

/// 32x32: ko'k doira ichida oq monitor belgisi.
fn make_icon() -> Option<Icon> {
    const N: u32 = 32;
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let inside = dx * dx + dy * dy <= 15.0 * 15.0;
            let screen = (8..24).contains(&x) && (9..21).contains(&y) && !((10..22).contains(&x) && (11..19).contains(&y));
            let stand = (13..19).contains(&x) && (22..24).contains(&y);
            let px = if !inside {
                [0, 0, 0, 0]
            } else if screen || stand {
                [255, 255, 255, 255]
            } else {
                [56, 120, 230, 255]
            };
            rgba.extend_from_slice(&px);
        }
    }
    Icon::from_rgba(rgba, N, N).ok()
}

impl Tray {
    pub fn new(ctx: &egui::Context) -> Option<Tray> {
        let menu = Menu::new();
        let status = MenuItem::new("Oflayn", false, None);
        let show = MenuItem::new("Ochish", true, None);
        let quit = MenuItem::new("Chiqish", true, None);
        menu.append_items(&[&status, &show, &quit]).ok()?;
        let (show_id, quit_id): (MenuId, MenuId) = (show.id().clone(), quit.id().clone());
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("Remote Desktop")
            .with_icon(make_icon()?)
            .build()
            .ok()?;

        let (tx, rx) = channel();
        {
            let (tx, ctx) = (tx.clone(), ctx.clone());
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                let cmd = if e.id == show_id {
                    Cmd::Show
                } else if e.id == quit_id {
                    Cmd::Quit
                } else {
                    return;
                };
                let _ = tx.send(cmd);
                ctx.request_repaint();
            }));
        }
        {
            let ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    let _ = tx.send(Cmd::Show);
                    ctx.request_repaint();
                }
            }));
        }
        Some(Tray { icon, status, rx, last: String::new() })
    }

    pub fn poll(&self) -> Vec<Cmd> {
        self.rx.try_iter().collect()
    }

    /// Holat matni menyuda va tooltip'da (o'zgargandagina yangilanadi).
    pub fn set_status(&mut self, text: &str) {
        if self.last != text {
            self.last = text.to_string();
            self.status.set_text(text);
            let _ = self.icon.set_tooltip(Some(format!("Remote Desktop · {text}")));
        }
    }
}
