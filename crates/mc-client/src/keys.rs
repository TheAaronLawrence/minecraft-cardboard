//! The key bindings: which key or mouse button each of vanilla's key mappings is on, saved
//! in `options.txt` as `key_<mapping>:<key>`, and what the held keys and buttons mean.

use crate::walk;
use mc_physics::Input;
use mc_render::hud::keys::{Key, MAPPINGS, UNBOUND, index};
use std::collections::HashSet;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// Each mapping's key, in `keys::MAPPINGS`' order.
#[derive(Clone, PartialEq, Debug)]
pub struct Bindings(pub Vec<Key>);

impl Default for Bindings {
    fn default() -> Self {
        Self(mc_render::hud::keys::defaults())
    }
}

impl Bindings {
    /// `Options.load`'s `key_` lines: each mapping's saved key, or its default.
    pub fn load(option: impl Fn(&str) -> Option<String>) -> Self {
        let keys = MAPPINGS.iter().map(|m| {
            option(&format!("key_{}", m.name))
                .and_then(|v| Key::parse(v.trim()))
                .unwrap_or(m.default)
        });
        Self(keys.collect())
    }

    /// `Options.save`'s `key_` lines.
    pub fn saved(&self) -> Vec<(String, String)> {
        let lines = MAPPINGS.iter().zip(&self.0);
        lines
            .map(|(m, k)| (format!("key_{}", m.name), k.name()))
            .collect()
    }

    /// `KeyMapping.matches`: whether `key` is the mapping called `name`'s, and it's bound.
    pub fn is(&self, name: &str, key: Key) -> bool {
        key != UNBOUND && self.0[index(name)] == key
    }

    /// `KeyMapping.isDown`: whether the mapping's key is held.
    pub fn down(&self, name: &str, held: &HashSet<Key>) -> bool {
        let key = self.0[index(name)];
        key != UNBOUND && held.contains(&key)
    }

    /// The movement mappings held, as `KeyboardInput.tick` reads them: Sneak and Sprint as
    /// their `ToggleKeyMapping`s have them.
    pub fn input(&self, held: &HashSet<Key>, toggles: &Toggles) -> Input {
        let down = |name| self.down(name, held);
        Input {
            forward: down("key.forward"),
            backward: down("key.back"),
            left: down("key.left"),
            right: down("key.right"),
            jump: down("key.jump"),
            sneak: toggles.sneak.down,
            sprint: toggles.sprint.down,
        }
    }

    /// Attack and Use down, held or toggled.
    pub fn buttons(toggles: &Toggles) -> walk::Buttons {
        walk::Buttons {
            attack: toggles.attack.down,
            use_item: toggles.use_item.down,
        }
    }

    /// The hotbar slot (0 to 8) whose mapping `key` is.
    pub fn hotbar(&self, key: Key) -> Option<u8> {
        (0..9u8).find(|n| self.is(&format!("key.hotbar.{}", n + 1), key))
    }
}

/// `ToggleKeyMapping`: a mapping that's down while its key is held, or with its option on,
/// flips each time its key is pressed (auto-repeats too, as `KeyboardHandler.keyPress` sets
/// keys for them). A screen lets go of it, and one that let go of it while it was toggled
/// on turns it back on when it closes, if its key is on the keyboard.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct ToggleKey {
    /// The option: toggle rather than hold.
    pub toggle: bool,
    pub down: bool,
    released_by_screen: bool,
}

impl ToggleKey {
    /// `ToggleKeyMapping.setDown`.
    pub fn set_down(&mut self, down: bool) {
        if !self.toggle {
            self.down = down;
        } else if down {
            self.down = !self.down;
        }
    }

    /// `ToggleKeyMapping.release`: a screen opened.
    fn release(&mut self) {
        if self.toggle && self.down || self.released_by_screen {
            self.released_by_screen = true;
        }
        self.down = false;
    }

    /// `KeyMapping.restoreToggleStatesOnScreenClosed`, for a mapping on `key` that restores
    /// its state (`ToggleKeyMapping.shouldRestore`: all but Use).
    fn restore(&mut self, key: Key, restores: bool) {
        let keyboard = matches!(key, Key::Keyboard(_));
        if restores && self.toggle && keyboard && self.released_by_screen {
            self.set_down(true);
        }
        self.released_by_screen = false;
    }
}

/// Vanilla's toggleable mappings (`ToggleKeyMapping`): Sneak, Sprint, Attack and Use.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Toggles {
    pub sneak: ToggleKey,
    pub sprint: ToggleKey,
    pub attack: ToggleKey,
    pub use_item: ToggleKey,
}

impl Toggles {
    /// `KeyMapping.set`: `key` pressed (or repeated) or let go, for each mapping it's on.
    pub fn set(&mut self, b: &Bindings, key: Key, down: bool) {
        if b.is("key.sneak", key) {
            self.sneak.set_down(down);
        }
        if b.is("key.sprint", key) {
            self.sprint.set_down(down);
        }
        if b.is("key.attack", key) {
            self.attack.set_down(down);
        }
        if b.is("key.use", key) {
            self.use_item.set_down(down);
        }
    }

    /// `KeyMapping.releaseAll`, as `Gui.setScreen` does when a screen opens.
    pub fn release(&mut self) {
        self.sneak.release();
        self.sprint.release();
        self.attack.release();
        self.use_item.release();
    }

    /// `KeyMapping.restoreToggleStatesOnScreenClosed`, as the game takes the mouse again.
    pub fn restore(&mut self, b: &Bindings) {
        self.sneak.restore(b.0[index("key.sneak")], true);
        self.sprint.restore(b.0[index("key.sprint")], true);
        self.attack.restore(b.0[index("key.attack")], true);
        self.use_item.restore(b.0[index("key.use")], false);
    }
}

/// `InputConstants.getKey`: a key by its USB HID scancode, as SDL numbers them.
pub fn from_code(code: KeyCode) -> Key {
    use KeyCode as K;
    let scancode = match code {
        K::KeyA => 4,
        K::KeyB => 5,
        K::KeyC => 6,
        K::KeyD => 7,
        K::KeyE => 8,
        K::KeyF => 9,
        K::KeyG => 10,
        K::KeyH => 11,
        K::KeyI => 12,
        K::KeyJ => 13,
        K::KeyK => 14,
        K::KeyL => 15,
        K::KeyM => 16,
        K::KeyN => 17,
        K::KeyO => 18,
        K::KeyP => 19,
        K::KeyQ => 20,
        K::KeyR => 21,
        K::KeyS => 22,
        K::KeyT => 23,
        K::KeyU => 24,
        K::KeyV => 25,
        K::KeyW => 26,
        K::KeyX => 27,
        K::KeyY => 28,
        K::KeyZ => 29,
        K::Digit1 => 30,
        K::Digit2 => 31,
        K::Digit3 => 32,
        K::Digit4 => 33,
        K::Digit5 => 34,
        K::Digit6 => 35,
        K::Digit7 => 36,
        K::Digit8 => 37,
        K::Digit9 => 38,
        K::Digit0 => 39,
        K::Enter => 40,
        K::Escape => 41,
        K::Backspace => 42,
        K::Tab => 43,
        K::Space => 44,
        K::Minus => 45,
        K::Equal => 46,
        K::BracketLeft => 47,
        K::BracketRight => 48,
        K::Backslash => 49,
        K::Semicolon => 51,
        K::Quote => 52,
        K::Backquote => 53,
        K::Comma => 54,
        K::Period => 55,
        K::Slash => 56,
        K::CapsLock => 57,
        K::F1 => 58,
        K::F2 => 59,
        K::F3 => 60,
        K::F4 => 61,
        K::F5 => 62,
        K::F6 => 63,
        K::F7 => 64,
        K::F8 => 65,
        K::F9 => 66,
        K::F10 => 67,
        K::F11 => 68,
        K::F12 => 69,
        K::PrintScreen => 70,
        K::ScrollLock => 71,
        K::Pause => 72,
        K::Insert => 73,
        K::Home => 74,
        K::PageUp => 75,
        K::Delete => 76,
        K::End => 77,
        K::PageDown => 78,
        K::ArrowRight => 79,
        K::ArrowLeft => 80,
        K::ArrowDown => 81,
        K::ArrowUp => 82,
        K::NumLock => 83,
        K::NumpadDivide => 84,
        K::NumpadMultiply => 85,
        K::NumpadSubtract => 86,
        K::NumpadAdd => 87,
        K::NumpadEnter => 88,
        K::Numpad1 => 89,
        K::Numpad2 => 90,
        K::Numpad3 => 91,
        K::Numpad4 => 92,
        K::Numpad5 => 93,
        K::Numpad6 => 94,
        K::Numpad7 => 95,
        K::Numpad8 => 96,
        K::Numpad9 => 97,
        K::Numpad0 => 98,
        K::NumpadDecimal => 99,
        K::IntlBackslash => 100,
        K::ContextMenu => 101,
        K::NumpadEqual => 103,
        K::F13 => 104,
        K::F14 => 105,
        K::F15 => 106,
        K::F16 => 107,
        K::F17 => 108,
        K::F18 => 109,
        K::F19 => 110,
        K::F20 => 111,
        K::F21 => 112,
        K::F22 => 113,
        K::F23 => 114,
        K::F24 => 115,
        K::NumpadComma => 133,
        K::IntlRo => 135,
        K::KanaMode => 136,
        K::IntlYen => 137,
        K::Convert => 138,
        K::NonConvert => 139,
        K::Lang1 => 144,
        K::Lang2 => 145,
        K::ControlLeft => 224,
        K::ShiftLeft => 225,
        K::AltLeft => 226,
        K::SuperLeft => 227,
        K::ControlRight => 228,
        K::ShiftRight => 229,
        K::AltRight => 230,
        K::SuperRight => 231,
        _ => 0,
    };
    Key::Keyboard(scancode)
}

/// A mouse button by SDL's number: left 1, middle 2, right 3, back 4, forward 5.
pub fn from_mouse(button: MouseButton) -> Key {
    Key::Mouse(match button {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
        MouseButton::Back => 4,
        MouseButton::Forward => 5,
        MouseButton::Other(n) => n as i32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rebound mapping moves the walking to its new key, saves under vanilla's names and
    /// loads back; an unbound one never matches.
    #[test]
    fn rebound_keys_walk_save_and_load() {
        let mut b = Bindings::default();
        b.0[index("key.forward")] = from_code(KeyCode::ArrowUp);
        b.0[index("key.jump")] = UNBOUND;
        let held = HashSet::from([from_code(KeyCode::KeyW), from_code(KeyCode::ArrowUp)]);
        let toggles = Toggles::default();
        assert!(b.input(&held, &toggles).forward);
        let held = HashSet::from([from_code(KeyCode::KeyW), UNBOUND]);
        let input = b.input(&held, &toggles);
        assert!(!input.forward && !input.jump);
        let saved = b.saved();
        assert!(saved.contains(&("key_key.forward".into(), "key.keyboard.up".into())));
        assert!(saved.contains(&("key_key.attack".into(), "key.mouse.left".into())));
        let option = |name: &str| {
            saved
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(Bindings::load(option), b);
        assert_eq!(b.hotbar(from_code(KeyCode::Digit3)), Some(2));
    }

    /// Held, Sneak follows its key; toggled, each press flips it and letting go doesn't. A
    /// screen opened while it's on lets go of it and turns it back on when it closes, but
    /// not a second time, and not when it was off.
    #[test]
    fn toggle_keys_flip_and_come_back_after_screens() {
        let b = Bindings::default();
        let shift = from_code(KeyCode::ShiftLeft);
        let mut t = Toggles::default();
        t.set(&b, shift, true);
        assert!(t.sneak.down && !t.sprint.down);
        t.set(&b, shift, false);
        assert!(!t.sneak.down);
        t.sneak.toggle = true;
        t.set(&b, shift, true);
        t.set(&b, shift, false);
        assert!(t.sneak.down, "letting go keeps a toggle on");
        t.release();
        assert!(!t.sneak.down, "a screen lets go");
        t.restore(&b);
        assert!(t.sneak.down, "closing it turns the toggle back on");
        t.restore(&b);
        assert!(t.sneak.down);
        t.set(&b, shift, true);
        t.release();
        t.restore(&b);
        assert!(!t.sneak.down, "a toggle that was off stays off");
        let mut mouse = b.clone();
        mouse.0[index("key.sneak")] = Key::Mouse(4);
        t.set(&mouse, Key::Mouse(4), true);
        t.release();
        t.restore(&mouse);
        assert!(!t.sneak.down, "a mouse button's toggle isn't restored");
        // Use, even on a keyboard key, is never restored; Attack is.
        let mut keyboard = b.clone();
        let (f, g) = (from_code(KeyCode::KeyF), from_code(KeyCode::KeyG));
        (
            keyboard.0[index("key.attack")],
            keyboard.0[index("key.use")],
        ) = (f, g);
        (t.attack.toggle, t.use_item.toggle) = (true, true);
        t.set(&keyboard, f, true);
        t.set(&keyboard, g, true);
        assert!(Bindings::buttons(&t).attack && Bindings::buttons(&t).use_item);
        t.release();
        t.restore(&keyboard);
        assert!(t.attack.down && !t.use_item.down);
    }
}
