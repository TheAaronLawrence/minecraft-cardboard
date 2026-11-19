//! The game's menu screens, over the panorama and the menu background as vanilla draws a
//! screen with no world behind it: the title screen (`TitleScreen`, with "Cardboard" under
//! the logo where vanilla has its edition), Direct Connection (`DirectJoinServerScreen`),
//! `ConnectScreen` while joining, and `DisconnectedScreen` when joining fails or the
//! connection is lost. Also the text box (`EditBox`) they and the launcher type into.

use super::{Button, GuiMesh, HudArt};

const PANORAMA: &str = "minecraft:gui/title/background/panorama_0";
const MENU_BACKGROUND: &str = "minecraft:gui/menu_background";
pub(crate) const LOGO: &str = "minecraft:gui/title/minecraft";

/// The textures the menus draw whole, by their full ids.
pub(crate) const TEXTURES: [&str; 3] = [PANORAMA, MENU_BACKGROUND, LOGO];

/// `EditBox.SPRITES`: unfocused and focused.
pub(crate) const TEXT_FIELD: [&str; 2] = ["widget/text_field", "widget/text_field_highlighted"];

/// Depths: the background behind everything, then widgets, then their text.
const BACKGROUND: f32 = -450.0;
const WIDGET: f32 = 0.0;
pub(crate) const LABEL: f32 = 200.0;

/// `EditBox`'s text colour (0xE0E0E0) and the labels' (0xA0A0A0).
const FIELD_TEXT: [f32; 3] = [224.0 / 255.0; 3];
const GREY: [f32; 3] = [160.0 / 255.0; 3];
const WHITE: [f32; 3] = [1.0; 3];

/// A text box's text and where its cursor is, in characters, edited as `EditBox` edits.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Field {
    pub value: String,
    pub cursor: usize,
}

impl Field {
    pub fn new(value: &str) -> Self {
        Self {
            value: value.to_owned(),
            cursor: value.chars().count(),
        }
    }

    fn edit(&mut self, f: impl FnOnce(&mut Vec<char>, usize) -> usize) {
        let mut chars: Vec<char> = self.value.chars().collect();
        let cursor = self.cursor.min(chars.len());
        self.cursor = f(&mut chars, cursor);
        self.value = chars.into_iter().collect();
    }

    /// Types `text` at the cursor, keeping `StringUtil.isAllowedChatCharacter`s, up to
    /// `max` characters in all.
    pub fn insert(&mut self, text: &str, max: usize) {
        self.edit(|chars, mut cursor| {
            for c in text
                .chars()
                .filter(|&c| c >= ' ' && c != '\u{a7}' && c != '\u{7f}')
            {
                if chars.len() < max {
                    chars.insert(cursor, c);
                    cursor += 1;
                }
            }
            cursor
        });
    }

    pub fn backspace(&mut self) {
        self.edit(|chars, cursor| match cursor {
            0 => 0,
            _ => {
                chars.remove(cursor - 1);
                cursor - 1
            }
        });
    }

    pub fn delete(&mut self) {
        self.edit(|chars, cursor| {
            if cursor < chars.len() {
                chars.remove(cursor);
            }
            cursor
        });
    }

    /// Moves the cursor by `steps` characters, staying inside the text.
    pub fn step(&mut self, steps: i32) {
        self.edit(|chars, cursor| (cursor as i32 + steps).clamp(0, chars.len() as i32) as usize);
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.value.chars().count();
    }
}

/// Which menu screen shows.
#[derive(Clone, Default, PartialEq, Debug)]
pub enum Screen {
    #[default]
    Title,
    DirectConnect,
    /// `ConnectScreen` (or a world loading): its message's translation key.
    Connecting(&'static str),
    /// `DisconnectedScreen`: its title's translation key and the reason.
    Disconnected {
        title: &'static str,
        reason: String,
    },
}

/// What the menus show.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Menu {
    pub screen: Screen,
    /// Direct Connection's server address, which is always focused.
    pub address: Field,
    /// The cursor is in the visible half of its blink.
    pub cursor_on: bool,
    /// Where the mouse is, in GUI pixels, to highlight what's under it.
    pub mouse: Option<[f32; 2]>,
}

/// What a menu button does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Singleplayer,
    Multiplayer,
    Options,
    Quit,
    Join,
    /// Cancel, and Back to Server List: Direct Connection goes back to the title screen;
    /// the others go back to Direct Connection.
    Back,
}

/// Direct Connection's box, 200 wide at y 116.
pub fn address_rect(width: i32) -> [i32; 4] {
    [width / 2 - 100, 116, 200, 20]
}

/// `ServerAddress.isValidAddress`, as far as it matters here: a host, then a port if any.
pub fn valid_address(address: &str) -> bool {
    let address = address.trim();
    match address.rsplit_once(':') {
        _ if address.is_empty() => false,
        Some((host, port)) if !host.contains(':') => {
            !host.is_empty() && port.parse::<u16>().is_ok()
        }
        _ => true,
    }
}

/// The current screen's buttons in a GUI `width` × `height` GUI pixels: what each does,
/// where it is, and whether it works.
pub fn buttons(menu: &Menu, art: &HudArt, width: i32, height: i32) -> Vec<(Action, Button, bool)> {
    let button = |x, y, w, label| Button {
        rect: [x, y, w, 20],
        label,
    };
    let left = width / 2 - 100;
    match &menu.screen {
        // `TitleScreen.init`: Singleplayer and Multiplayer from a quarter of the way down
        // plus 48, 24 apart; with no Realms or icon buttons, Options and Quit Game follow
        // 12 further down, as the title screen placed them before those came.
        Screen::Title => {
            let top = height / 4 + 48;
            vec![
                (
                    Action::Singleplayer,
                    button(left, top, 200, "menu.singleplayer"),
                    true,
                ),
                (
                    Action::Multiplayer,
                    button(left, top + 24, 200, "menu.multiplayer"),
                    true,
                ),
                (
                    Action::Options,
                    button(left, top + 60, 98, "menu.options"),
                    false,
                ),
                (
                    Action::Quit,
                    button(width / 2 + 2, top + 60, 98, "menu.quit"),
                    true,
                ),
            ]
        }
        // `DirectJoinServerScreen.init`.
        Screen::DirectConnect => {
            let join_active = valid_address(&menu.address.value);
            vec![
                (
                    Action::Join,
                    button(left, height / 4 + 108, 200, "selectServer.select"),
                    join_active,
                ),
                (
                    Action::Back,
                    button(left, height / 4 + 132, 200, "gui.cancel"),
                    true,
                ),
            ]
        }
        // `ConnectScreen.init`.
        Screen::Connecting(_) => {
            vec![(
                Action::Back,
                button(left, height / 4 + 132, 200, "gui.cancel"),
                true,
            )]
        }
        Screen::Disconnected { reason, .. } => {
            let layout = disconnected_layout(art, reason, width, height);
            vec![(
                Action::Back,
                button(left, layout.button_y, 200, "gui.toMenu"),
                true,
            )]
        }
    }
}

/// `DisconnectedScreen`'s column, centred on the screen: the title and the reason padded
/// 10 all round, then the button padded 2.
struct DisconnectedLayout {
    title_y: i32,
    reason_y: i32,
    lines: Vec<String>,
    button_y: i32,
}

fn disconnected_layout(art: &HudArt, reason: &str, width: i32, height: i32) -> DisconnectedLayout {
    // `MultiLineTextWidget.setMaxWidth(width - 50)`.
    let lines = art.wrap(reason, width - 50);
    let reason_height = lines.len() as i32 * 9;
    let total = (9 + 20) + (reason_height + 20) + (20 + 4);
    let top = (height - total) / 2;
    DisconnectedLayout {
        title_y: top + 10,
        reason_y: top + 29 + 10,
        button_y: top + 29 + reason_height + 20 + 2,
        lines,
    }
}

/// The menus' quads, for a GUI `width` × `height` GUI pixels.
pub fn menu(art: &HudArt, m: &Menu, width: u32, height: u32) -> GuiMesh {
    let (w, h) = (width as i32, height as i32);
    let mut mesh = GuiMesh::default();
    mesh.background(art, w, h);
    let lang = |key: &str| art.lang.get(key).cloned().unwrap_or_else(|| key.to_owned());
    let centred = |mesh: &mut GuiMesh, text: &str, y: i32, rgb: [f32; 3]| {
        let x = w / 2 - GuiMesh::text_width(art, text) as i32 / 2;
        mesh.shadowed(art, text, x as f32, y as f32, LABEL, 1.0, rgb);
    };
    match &m.screen {
        Screen::Title => {
            mesh.logo(art, w);
            // `TitleScreen`'s version line, bottom left.
            let version = format!("minecraft-cardboard {}", env!("CARGO_PKG_VERSION"));
            mesh.shadowed(art, &version, 2.0, (h - 10) as f32, LABEL, 1.0, WHITE);
        }
        Screen::DirectConnect => {
            centred(&mut mesh, &lang("selectServer.direct"), 20, WHITE);
            let label = lang("addServer.enterIp");
            mesh.shadowed(art, &label, (w / 2 - 99) as f32, 100.0, LABEL, 1.0, GREY);
            mesh.edit_box(art, &m.address, address_rect(w), true, m.cursor_on);
        }
        Screen::Connecting(key) => centred(&mut mesh, &lang(key), h / 2 - 50, WHITE),
        Screen::Disconnected { title, reason } => {
            let layout = disconnected_layout(art, reason, w, h);
            centred(&mut mesh, &lang(title), layout.title_y, GREY);
            for (i, line) in layout.lines.iter().enumerate() {
                centred(&mut mesh, line, layout.reason_y + i as i32 * 9, WHITE);
            }
        }
    }
    for (_, button, active) in buttons(m, art, w, h) {
        mesh.button(art, &button, active, m.mouse);
    }
    mesh
}

fn width(art: &HudArt, chars: &[char]) -> i32 {
    chars
        .iter()
        .filter_map(|c| art.glyphs.get(c))
        .map(|&(_, advance)| advance as i32)
        .sum()
}

/// The characters of a box that show: from the first one that keeps the cursor in view
/// (`EditBox.scrollTo`), as many as fit (`Font.plainSubstrByWidth`).
fn shown(art: &HudArt, field: &Field, inner: i32) -> std::ops::Range<usize> {
    let chars: Vec<char> = field.value.chars().collect();
    let cursor = field.cursor.min(chars.len());
    let mut start = 0;
    while width(art, &chars[start..cursor]) > inner {
        start += 1;
    }
    let mut end = start;
    while end < chars.len() && width(art, &chars[start..=end]) <= inner {
        end += 1;
    }
    start..end
}

/// `EditBox.findClickedPositionInText`: the cursor position a click at GUI x `x` gives in
/// a box at `rect`.
pub fn clicked_position(art: &HudArt, field: &Field, rect: [i32; 4], x: f32) -> usize {
    let inner = rect[2] - 8;
    let chars: Vec<char> = field.value.chars().collect();
    let range = shown(art, field, inner);
    let within = ((x as i32) - (rect[0] + 4)).min(inner);
    let mut position = range.start;
    while position < range.end && width(art, &chars[range.start..=position]) <= within {
        position += 1;
    }
    position
}

impl GuiMesh {
    /// `Screen.extractPanorama`, whose six faces are each one colour in 26.3's resources,
    /// then `extractMenuBackground`: the menu background tiled at 32 GUI pixels a tile.
    pub(crate) fn background(&mut self, art: &HudArt, w: i32, h: i32) {
        let [u0, v0, u1, v1] = art.named[PANORAMA];
        let (u, v) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
        self.blit(
            0.0,
            0.0,
            BACKGROUND,
            w as f32,
            h as f32,
            [u, v, u, v],
            [1.0; 4],
        );
        let tile = art.named[MENU_BACKGROUND];
        for y in (0..h).step_by(32) {
            for x in (0..w).step_by(32) {
                self.blit(
                    x as f32,
                    y as f32,
                    BACKGROUND + 1.0,
                    32.0,
                    32.0,
                    tile,
                    [1.0; 4],
                );
            }
        }
    }

    /// `LogoRenderer.extractRenderState`'s logo, 256 × 44 of the 256 × 64 texture, 30 down;
    /// where vanilla puts its edition, 7 up into the logo, "Cardboard" at twice the font's
    /// size, outlined in black as the edition's letters are.
    pub(crate) fn logo(&mut self, art: &HudArt, w: i32) {
        self.logo_at(art, w / 2, 30);
    }

    /// The logo with "Cardboard" under it, centred on `centre`, its top at `top`.
    pub(crate) fn logo_at(&mut self, art: &HudArt, centre: i32, top: i32) {
        let [u0, v0, u1, v1] = art.named[LOGO];
        let logo = [u0, v0, u1, v0 + (v1 - v0) * 44.0 / 64.0];
        let x = (centre - 128) as f32;
        self.blit(x, top as f32, WIDGET, 256.0, 44.0, logo, [1.0; 4]);
        let text = "Cardboard";
        let text_x = (centre - GuiMesh::text_width(art, text) as i32) as f32;
        let text_y = (top + 44 - 7) as f32;
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (1.0, 1.0)] {
            self.glyphs(
                art,
                text,
                text_x + dx,
                text_y + dy,
                WIDGET + 1.0,
                2.0,
                [0.0; 3],
            );
        }
        self.glyphs(art, text, text_x, text_y, WIDGET + 2.0, 2.0, [0.86; 3]);
    }

    /// `EditBox.extractWidgetRenderState` for a bordered box: its sprite, then its text.
    fn edit_box(
        &mut self,
        art: &HudArt,
        field: &Field,
        rect: [i32; 4],
        focused: bool,
        cursor: bool,
    ) {
        let [x, y, w, h] = rect;
        let sprite = art.named[TEXT_FIELD[focused as usize]];
        self.blit(
            x as f32, y as f32, WIDGET, w as f32, h as f32, sprite, [1.0; 4],
        );
        self.field_text(art, field, rect, cursor, true, FIELD_TEXT);
    }

    /// A box's text, shadowed or not, with no selection: the text before the cursor, the
    /// cursor (a bar inside the text, `_` at its end, `TextCursorUtils`), then the rest.
    pub(crate) fn field_text(
        &mut self,
        art: &HudArt,
        field: &Field,
        rect: [i32; 4],
        cursor: bool,
        shadow: bool,
        rgb: [f32; 3],
    ) {
        let [x, y, w, h] = rect;
        let text = |mesh: &mut Self, t: &str, x: i32, y: i32| {
            if shadow {
                mesh.shadowed(art, t, x as f32, y as f32, LABEL, 1.0, rgb);
            } else {
                mesh.glyphs(art, t, x as f32, y as f32, LABEL, 1.0, rgb);
            }
        };
        let chars: Vec<char> = field.value.chars().collect();
        let range = shown(art, field, w - 8);
        let displayed: String = chars[range.clone()].iter().collect();
        let relative = field.cursor.saturating_sub(range.start);
        let on_screen = field.cursor >= range.start && relative <= displayed.chars().count();
        let (text_x, text_y) = (x + 4, y + (h - 8) / 2);
        let mut draw_x = text_x;
        let split = displayed
            .char_indices()
            .nth(relative)
            .map_or(displayed.len(), |(i, _)| i);
        if !displayed.is_empty() {
            let before = if on_screen {
                &displayed[..split]
            } else {
                &displayed
            };
            text(self, before, draw_x, text_y);
            draw_x += width(art, &before.chars().collect::<Vec<_>>()) + 1;
        }
        let insert = field.cursor < chars.len();
        let mut cursor_x = draw_x;
        if insert {
            cursor_x = draw_x - 1;
            draw_x -= 1;
        }
        if on_screen && split < displayed.len() {
            text(self, &displayed[split..], draw_x, text_y);
        }
        if cursor && on_screen {
            if insert {
                let rgba = rgb.map(|c| (c * 255.0) as u8);
                let color = [rgba[0], rgba[1], rgba[2], 255];
                self.rect(art, [cursor_x, text_y - 1, 1, 11], LABEL + 1.0, color);
            } else {
                text(self, "_", cursor_x, text_y);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Field, valid_address};

    /// Typing, deleting and moving edit at the cursor, and typing stops at the limit and
    /// drops characters chat doesn't allow.
    #[test]
    fn fields_edit_at_the_cursor() {
        let mut f = Field::new("lochost");
        f.step(-4);
        f.insert("al", 9);
        assert_eq!((f.value.as_str(), f.cursor), ("localhost", 5));
        f.insert("xyz", 10);
        assert_eq!((f.value.as_str(), f.cursor), ("localxhost", 6));
        f.backspace();
        f.delete();
        assert_eq!((f.value.as_str(), f.cursor), ("localost", 5));
        f.end();
        f.insert(":2\u{a7}5\u{7}565", 64);
        assert_eq!(f.value, "localost:25565");
        f.home();
        f.backspace();
        f.step(-3);
        assert_eq!((f.value.as_str(), f.cursor), ("localost:25565", 0));
    }

    #[test]
    fn addresses_need_a_host_and_a_valid_port() {
        for ok in [
            "localhost",
            "example.com:25565",
            " 10.0.0.2:1 ",
            "[::1]:25565",
        ] {
            assert!(valid_address(ok), "{ok}");
        }
        for bad in [
            "",
            "   ",
            ":25565",
            "localhost:",
            "localhost:99999",
            "host:port",
        ] {
            assert!(!valid_address(bad), "{bad}");
        }
    }
}
