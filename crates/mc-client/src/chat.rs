//! The chat on a server: the messages `ChatComponent` keeps, and `ChatScreen`, which T
//! opens (and / opens with a slash) to type a message or a command, with Enter sending it,
//! Up and Down going through what was sent before, and Page Up and Down or the wheel
//! scrolling.

use mc_render::hud::chat::{self as hud_chat, Line};
use mc_render::hud::menu::Field;
use std::time::Instant;

#[derive(Default)]
pub struct Chat {
    /// Lines, newest first, as `ChatComponent.trimmedMessages`.
    lines: Vec<Line>,
    /// What was sent, oldest first (`recentChat`).
    history: Vec<String>,
    /// Where Up and Down are in the history, and what was typed before moving into it.
    history_pos: usize,
    history_buffer: String,
    /// The chat screen's box, while it's open.
    input: Option<Field>,
    scroll: usize,
    new_since_scroll: bool,
    /// When the box was last edited, which restarts its cursor's blink.
    edited: Option<Instant>,
}

/// What a key in the open chat asks the game to do.
pub enum Key {
    /// Send this, a message or a `/` command, and close the chat.
    Send(String),
    Close,
    None,
}

impl Chat {
    pub fn is_open(&self) -> bool {
        self.input.is_some()
    }

    /// `Gui.openChatScreen`, with `prefix` already typed (`/` for the command key).
    pub fn open(&mut self, prefix: &str) {
        self.input = Some(Field::new(prefix));
        self.history_pos = self.history.len();
        self.edited = Some(Instant::now());
    }

    /// `ChatScreen.removed`: closing scrolls back to the newest line.
    pub fn close(&mut self) {
        self.input = None;
        (self.scroll, self.new_since_scroll) = (0, false);
    }

    /// `ChatComponent.addMessage`, wrapped to the chat's width as `addMessageToDisplayQueue`
    /// does, at `tick`. Scrolled up, the view stays on the same lines.
    pub fn add(
        &mut self,
        art: &mc_render::hud::HudArt,
        spans: &[mc_protocol::chat::Span],
        tick: i32,
    ) {
        for spans in hud_chat::wrap(art, spans, hud_chat::WIDTH) {
            if self.is_open() && self.scroll > 0 {
                self.new_since_scroll = true;
                self.scroll_by(1);
            }
            self.lines.insert(0, Line { spans, added: tick });
        }
        self.lines.truncate(hud_chat::MAX_LINES);
    }

    /// `ChatComponent.scrollChat`: up by `lines` (down when negative), within the lines.
    pub fn scroll_by(&mut self, lines: i32) {
        let max = self.lines.len().saturating_sub(hud_chat::OPEN_LINES) as i32;
        let scroll = (self.scroll as i32 + lines).min(max);
        if scroll <= 0 {
            (self.scroll, self.new_since_scroll) = (0, false);
        } else {
            self.scroll = scroll as usize;
        }
    }

    /// A key pressed with the chat open: `ChatScreen.keyPressed` and its `EditBox`.
    pub fn key(&mut self, key: &winit::keyboard::Key, text: Option<&str>) -> Key {
        use winit::keyboard::{Key as K, NamedKey as N};
        let Some(field) = &mut self.input else {
            return Key::None;
        };
        self.edited = Some(Instant::now());
        match key {
            K::Named(N::Escape) => return Key::Close,
            K::Named(N::Enter) => {
                // `handleChatInput`: what's left after normalizing, remembered if new.
                let message = mc_protocol::chat::normalize(&field.value);
                if !message.is_empty() && self.history.last() != Some(&message) {
                    self.history.push(message.clone());
                    if self.history.len() > 100 {
                        self.history.remove(0);
                    }
                }
                return if message.is_empty() {
                    Key::Close
                } else {
                    Key::Send(message)
                };
            }
            K::Named(N::ArrowUp) => self.move_in_history(-1),
            K::Named(N::ArrowDown) => self.move_in_history(1),
            K::Named(N::PageUp) => self.scroll_by(hud_chat::OPEN_LINES as i32 - 1),
            K::Named(N::PageDown) => self.scroll_by(-(hud_chat::OPEN_LINES as i32) + 1),
            K::Named(N::Backspace) => field.backspace(),
            K::Named(N::Delete) => field.delete(),
            K::Named(N::ArrowLeft) => field.step(-1),
            K::Named(N::ArrowRight) => field.step(1),
            K::Named(N::Home) => field.home(),
            K::Named(N::End) => field.end(),
            _ => {
                // `ChatScreen.init`: `setMaxLength(256)`.
                field.insert(text.unwrap_or(""), 256)
            }
        }
        Key::None
    }

    /// `ChatScreen.moveInHistory`: past the newest entry is what was being typed.
    fn move_in_history(&mut self, dir: i32) {
        let Some(field) = &mut self.input else { return };
        let max = self.history.len();
        let pos = (self.history_pos as i32 + dir).clamp(0, max as i32) as usize;
        if pos == self.history_pos {
            return;
        }
        if pos == max {
            *field = Field::new(&self.history_buffer);
        } else {
            if self.history_pos == max {
                self.history_buffer = field.value.clone();
            }
            *field = Field::new(&self.history[pos]);
        }
        self.history_pos = pos;
    }

    /// What the HUD draws at `tick`.
    pub fn view(&self, tick: i32) -> hud_chat::Chat {
        hud_chat::Chat {
            lines: self.lines.clone(),
            tick,
            scroll: self.scroll,
            new_since_scroll: self.new_since_scroll,
            input: self.input.clone(),
            // `TextCursorUtils.isCursorVisible`: on for 300 ms, off for 300.
            cursor_on: self
                .edited
                .is_some_and(|t| (t.elapsed().as_millis() / 300).is_multiple_of(2)),
        }
    }
}
