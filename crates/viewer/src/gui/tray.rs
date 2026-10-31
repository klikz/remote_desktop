//! Tizim paneli (tray) ikonkasi (Windows). Oyna yopilganda/pastga tushirilganda agent ishlashda qolishi
//! uchun dastur trayga yashirinadi. Ikonka: bosilsa oyna ochiladi, menyuda "Ochish" va "Chiqish".

use std::sync::mpsc::{channel, Receiver};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use std::sync::atomic::{AtomicBool, Ordering};

/// Oyna tray ikonkasi orqali ochildi (`update` hidden bayrog'ini tushiradi).
static SHOWN: AtomicBool = AtomicBool::new(false);

extern "system" {
    fn FindWindowW(class: *const u16, title: *const u16) -> isize;
    fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
    fn SetForegroundWindow(hwnd: isize) -> i32;
}

/// Yashirin oyna `update` chaqirmaydi, shuning uchun oynani to'g'ridan-to'g'ri Win32 orqali ochamiz.
fn show_window() {
    let title: Vec<u16> = "MS Remote\0".encode_utf16().collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd != 0 {
            ShowWindow(hwnd, 9); // SW_RESTORE
            ShowWindow(hwnd, 5); // SW_SHOW
            SetForegroundWindow(hwnd);
        }
    }
    SHOWN.store(true, Ordering::SeqCst);
}

pub enum Cmd {
    Show,
    Quit,
    /// Oyna allaqachon ochilgan.
    Shown,
}

pub struct Tray {
    icon: TrayIcon,
    status: MenuItem,
    rx: Receiver<Cmd>,
    last: String,
}

/// 32x32: dastur ikonkasi (`icon_draw`).
fn make_icon() -> Option<Icon> {
    const N: u32 = 32;
    Icon::from_rgba(crate::icon_draw::icon_rgba(N), N, N).ok()
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
            .with_tooltip("MS Remote")
            .with_icon(make_icon()?)
            .build()
            .ok()?;

        let (_tx, rx) = channel();
        {
            let ctx = ctx.clone();
            MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                if e.id == show_id {
                    show_window();
                } else if e.id == quit_id {
                    std::process::exit(0);
                }
                ctx.request_repaint();
            }));
        }
        {
            let ctx = ctx.clone();
            TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                    show_window();
                    ctx.request_repaint();
                }
            }));
        }
        Some(Tray { icon, status, rx, last: String::new() })
    }

    pub fn poll(&self) -> Vec<Cmd> {
        let mut v: Vec<Cmd> = self.rx.try_iter().collect();
        if SHOWN.swap(false, Ordering::SeqCst) {
            v.push(Cmd::Shown);
        }
        v
    }

    /// Holat matni menyuda va tooltip'da (o'zgargandagina yangilanadi).
    pub fn set_status(&mut self, text: &str) {
        if self.last != text {
            self.last = text.to_string();
            self.status.set_text(text);
            let _ = self.icon.set_tooltip(Some(format!("MS Remote · {text}")));
        }
    }
}
