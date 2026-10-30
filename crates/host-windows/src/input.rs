//! Viewer'dan kelgan sichqoncha va klaviatura hodisalarini Windows'da bajarish (SendInput).
//! Eslatma: yuqori huquqli oynalar (UAC, administrator dasturlar) va qulf ekrani oddiy dastur
//! uchun yopiq, ularga kiritish ishlamaydi.

use protocol::InputEvent;
use std::mem;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, KEYBDINPUT, MOUSEINPUT};

const INPUT_MOUSE: u32 = 0;
const INPUT_KEYBOARD: u32 = 1;

const MOVE: u32 = 0x0001;
const LEFTDOWN: u32 = 0x0002;
const LEFTUP: u32 = 0x0004;
const RIGHTDOWN: u32 = 0x0008;
const RIGHTUP: u32 = 0x0010;
const MIDDLEDOWN: u32 = 0x0020;
const MIDDLEUP: u32 = 0x0040;
const WHEEL: u32 = 0x0800;
const HWHEEL: u32 = 0x1000;
const ABSOLUTE: u32 = 0x8000;

const KEY_EXTENDED: u32 = 0x0001;
const KEY_UP: u32 = 0x0002;
const KEY_SCANCODE: u32 = 0x0008;

fn send(input: INPUT) {
    unsafe {
        SendInput(1, &input, mem::size_of::<INPUT>() as i32);
    }
}

fn mouse(dx: i32, dy: i32, data: u32, flags: u32) {
    send(INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    });
}

fn key(scan: u16, extended: bool, up: bool) {
    let mut flags = KEY_SCANCODE;
    if extended {
        flags |= KEY_EXTENDED;
    }
    if up {
        flags |= KEY_UP;
    }
    send(INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    });
}

/// Linux evdev kodi -> (Windows scancode, kengaytirilgan klavishmi).
/// evdev 1..=88 PS/2 set 1 scancode'lariga mos keladi (ESC=1, A=30, F12=88).
fn scancode(code: u32) -> Option<(u16, bool)> {
    Some(match code {
        1..=88 => (code as u16, false),
        96 => (0x1C, true),  // Numpad Enter
        97 => (0x1D, true),  // O'ng Ctrl
        98 => (0x35, true),  // Numpad /
        100 => (0x38, true), // O'ng Alt
        102 => (0x47, true), // Home
        103 => (0x48, true), // Up
        104 => (0x49, true), // PageUp
        105 => (0x4B, true), // Left
        106 => (0x4D, true), // Right
        107 => (0x4F, true), // End
        108 => (0x50, true), // Down
        109 => (0x51, true), // PageDown
        110 => (0x52, true), // Insert
        111 => (0x53, true), // Delete
        125 => (0x5B, true), // Chap Win
        126 => (0x5C, true), // O'ng Win
        127 => (0x5D, true), // Menu
        _ => return None,
    })
}

pub fn apply(ev: InputEvent) {
    match ev {
        // 0..65535 asosiy monitorning butun maydoni: aynan olinayotgan ekran
        InputEvent::PointerMove { x, y } => mouse(i32::from(x), i32::from(y), 0, MOVE | ABSOLUTE),
        InputEvent::Button { button, pressed } => {
            let flags = match (button, pressed) {
                (0, true) => LEFTDOWN,
                (0, false) => LEFTUP,
                (1, true) => RIGHTDOWN,
                (1, false) => RIGHTUP,
                (_, true) => MIDDLEDOWN,
                (_, false) => MIDDLEUP,
            };
            mouse(0, 0, 0, flags);
        }
        InputEvent::Scroll { dx, dy } => {
            // Protokol: musbat = pastga/o'ngga. Windows: musbat = yuqoriga/o'ngga.
            if dy != 0 {
                mouse(0, 0, (-i32::from(dy) * 120) as u32, WHEEL);
            }
            if dx != 0 {
                mouse(0, 0, (i32::from(dx) * 120) as u32, HWHEEL);
            }
        }
        InputEvent::Key { code, pressed } => {
            if let Some((scan, ext)) = scancode(code) {
                key(scan, ext, !pressed);
            }
        }
    }
}
