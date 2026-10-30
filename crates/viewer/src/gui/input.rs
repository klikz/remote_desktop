//! egui hodisalarini Host'ga yuboriladigan `InputEvent` larga aylantiradi.

use super::keys;
use egui::{Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect, Vec2};
use protocol::InputEvent;
use std::collections::HashSet;

#[derive(Default)]
pub struct Forwarder {
    keys: HashSet<u32>,
    buttons: [bool; 3],
    mods: (bool, bool, bool), // shift, ctrl, alt
    last_pos: Option<(u16, u16)>,
    wheel: Vec2,
}

fn frac(rect: Rect, p: Pos2) -> Option<(u16, u16)> {
    if !rect.contains(p) || rect.width() < 1.0 || rect.height() < 1.0 {
        return None;
    }
    let fx = ((p.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
    let fy = ((p.y - rect.min.y) / rect.height()).clamp(0.0, 1.0);
    Some(((fx * 65535.0) as u16, (fy * 65535.0) as u16))
}

impl Forwarder {
    /// `rect`: video chizilgan to'rtburchak (egui nuqtalarida). `hover`: sichqoncha hozir qayerda.
    pub fn process(
        &mut self,
        events: &[Event],
        modifiers: Modifiers,
        focused: bool,
        rect: Rect,
        hover: Option<Pos2>,
        out: &mut dyn FnMut(InputEvent),
    ) {
        if !focused {
            self.release_all(out);
            return;
        }
        self.sync_modifiers(modifiers, out);
        let inside = hover.map_or(false, |p| rect.contains(p));

        for ev in events {
            match ev {
                Event::PointerMoved(p) => self.move_to(rect, *p, out),
                Event::PointerButton { pos, button, pressed, .. } => {
                    let i = match button {
                        PointerButton::Primary => 0,
                        PointerButton::Secondary => 1,
                        PointerButton::Middle => 2,
                        _ => continue,
                    };
                    if *pressed && !rect.contains(*pos) {
                        continue;
                    }
                    if *pressed {
                        self.move_to(rect, *pos, out);
                    }
                    if self.buttons[i] != *pressed {
                        self.buttons[i] = *pressed;
                        out(InputEvent::Button { button: i as u8, pressed: *pressed });
                    }
                }
                Event::MouseWheel { unit, delta, .. } if inside => {
                    let lines = match unit {
                        MouseWheelUnit::Line => *delta,
                        MouseWheelUnit::Point => *delta / 50.0,
                        MouseWheelUnit::Page => *delta * 3.0,
                    };
                    self.wheel += lines;
                    let (sx, sy) = (self.wheel.x.trunc(), self.wheel.y.trunc());
                    self.wheel -= Vec2::new(sx, sy);
                    // egui: musbat = yuqoriga/chapga; portal: musbat = pastga/o'ngga
                    if sx != 0.0 || sy != 0.0 {
                        out(InputEvent::Scroll { dx: -(sx as i16), dy: -(sy as i16) });
                    }
                }
                Event::Key { key, physical_key, pressed, repeat, .. } => {
                    if *repeat {
                        continue;
                    }
                    let name = physical_key.unwrap_or(*key).name();
                    if let Some(code) = keys::evdev(name) {
                        self.key(code, *pressed, out);
                    }
                }
                // egui Ctrl+C/X/V ni klavish o'rniga shu hodisalar bilan beradi
                Event::Copy => self.tap(keys::KEY_C, out),
                Event::Cut => self.tap(keys::KEY_X, out),
                Event::Paste(_) => self.tap(keys::KEY_V, out),
                _ => {}
            }
        }
    }

    fn move_to(&mut self, rect: Rect, p: Pos2, out: &mut dyn FnMut(InputEvent)) {
        if let Some(pos) = frac(rect, p) {
            if self.last_pos != Some(pos) {
                self.last_pos = Some(pos);
                out(InputEvent::PointerMove { x: pos.0, y: pos.1 });
            }
        }
    }

    fn key(&mut self, code: u32, pressed: bool, out: &mut dyn FnMut(InputEvent)) {
        let changed = if pressed { self.keys.insert(code) } else { self.keys.remove(&code) };
        if changed {
            out(InputEvent::Key { code, pressed });
        }
    }

    fn tap(&mut self, code: u32, out: &mut dyn FnMut(InputEvent)) {
        if !self.keys.contains(&code) {
            out(InputEvent::Key { code, pressed: true });
            out(InputEvent::Key { code, pressed: false });
        }
    }

    fn sync_modifiers(&mut self, m: Modifiers, out: &mut dyn FnMut(InputEvent)) {
        let now = (m.shift, m.ctrl, m.alt);
        if now.0 != self.mods.0 {
            self.key(keys::LEFT_SHIFT, now.0, out);
        }
        if now.1 != self.mods.1 {
            self.key(keys::LEFT_CTRL, now.1, out);
        }
        if now.2 != self.mods.2 {
            self.key(keys::LEFT_ALT, now.2, out);
        }
        self.mods = now;
    }

    /// Bosilgan hamma narsani qo'yib yuboradi (fokus yo'qolganda yoki uzilganda).
    pub fn release_all(&mut self, out: &mut dyn FnMut(InputEvent)) {
        for code in self.keys.drain() {
            out(InputEvent::Key { code, pressed: false });
        }
        for (i, b) in self.buttons.iter_mut().enumerate() {
            if *b {
                *b = false;
                out(InputEvent::Button { button: i as u8, pressed: false });
            }
        }
        self.mods = (false, false, false);
    }
}
