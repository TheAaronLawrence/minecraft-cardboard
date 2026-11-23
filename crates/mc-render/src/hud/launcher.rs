//! The launcher's window, in the style of Mojang's Minecraft Launcher: a dark sidebar with
//! the player's name and the game, a header with the Play tab, the logo over the menu
//! background, and a bar along the bottom with the version and a big green Play button.
//! While the game installs or updates, a bar along the bottom fills.

use super::menu::{Field, LABEL};
use super::{GuiMesh, HudArt};

/// Depths: the panels over the background, then the boxes on them.
const PANEL: f32 = -100.0;
const BOX: f32 = -50.0;

const SIDEBAR: [u8; 4] = [30, 30, 30, 255];
const HEADER: [u8; 4] = [38, 38, 38, 255];
const BOTTOM: [u8; 4] = [36, 36, 36, 255];
const SELECTED: [u8; 4] = [52, 52, 52, 255];
const GREEN: [u8; 4] = [0, 133, 66, 255];
const GREEN_HOVER: [u8; 4] = [0, 110, 55, 255];
const DISABLED: [u8; 4] = [70, 70, 70, 255];
const WHITE: [f32; 3] = [1.0; 3];
const GREY: [f32; 3] = [150.0 / 255.0; 3];
const RED: [f32; 3] = [1.0, 85.0 / 255.0, 85.0 / 255.0];

/// What the launcher shows.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Launcher {
    pub name: Field,
    /// The name box has the focus, and takes typing.
    pub focused: bool,
    /// Play has the keyboard focus, outlined in white.
    pub play_focused: bool,
    /// The cursor is in the visible half of its blink.
    pub cursor_on: bool,
    /// Play works: the name is a valid player name.
    pub play_active: bool,
    /// Where the mouse is, in GUI pixels, to highlight what's under it.
    pub mouse: Option<[f32; 2]>,
    /// Shown in red over the bottom bar, such as when the game wouldn't start.
    pub error: Option<String>,
    /// The big button's label; empty is "PLAY".
    pub button: String,
    /// What the launcher is doing, such as downloading the game, over the bottom bar.
    pub status: Option<String>,
    /// How far a download has got, from 0 to 1, as a bar along the top of the bottom bar.
    pub progress: Option<f32>,
    /// The installed release's tag.
    pub installed: Option<String>,
}

/// Where the launcher's controls are: x, y, width and height in GUI pixels.
pub struct Layout {
    pub name: [i32; 4],
    pub play: [i32; 4],
}

const SIDEBAR_WIDTH: i32 = 96;
const HEADER_HEIGHT: i32 = 34;
const BOTTOM_HEIGHT: i32 = 48;

pub fn layout(width: i32, height: i32) -> Layout {
    let centre = (SIDEBAR_WIDTH + width) / 2;
    Layout {
        name: [6, 22, SIDEBAR_WIDTH - 12, 16],
        play: [centre - 55, height - BOTTOM_HEIGHT + 10, 110, 28],
    }
}

pub fn contains([x, y, w, h]: [i32; 4], [mx, my]: [f32; 2]) -> bool {
    mx >= x as f32 && my >= y as f32 && mx < (x + w) as f32 && my < (y + h) as f32
}

/// The launcher's quads, for a GUI `width` × `height` GUI pixels.
pub fn launcher(art: &HudArt, l: &Launcher, width: u32, height: u32) -> GuiMesh {
    let (w, h) = (width as i32, height as i32);
    let mut mesh = GuiMesh::default();
    let layout = layout(w, h);
    let text = |mesh: &mut GuiMesh, t: &str, x: i32, y: i32, scale: f32, rgb: [f32; 3]| {
        mesh.glyphs(art, t, x as f32, y as f32, LABEL, scale, rgb);
    };
    let text_width = |t: &str| GuiMesh::text_width(art, t) as i32;

    // The hero: the menu background with the logo, between the header and the bottom bar.
    mesh.background(art, w, h);
    let hero_top = HEADER_HEIGHT;
    let hero_height = h - HEADER_HEIGHT - BOTTOM_HEIGHT;
    let centre = (SIDEBAR_WIDTH + w) / 2;
    mesh.logo_at(art, centre, hero_top + (hero_height - 58) / 2);

    // The sidebar: who's playing, then the game, selected.
    mesh.rect(art, [0, 0, SIDEBAR_WIDTH, h], PANEL, SIDEBAR);
    text(&mut mesh, "PLAYER NAME", 6, 10, 1.0, GREY);
    let [x, y, bw, bh] = layout.name;
    let border = if l.focused { GREEN } else { [80, 80, 80, 255] };
    mesh.rect(art, [x, y, bw, bh], BOX, border);
    mesh.rect(
        art,
        [x + 1, y + 1, bw - 2, bh - 2],
        BOX + 1.0,
        [16, 16, 16, 255],
    );
    mesh.field_text(
        art,
        &l.name,
        layout.name,
        l.focused && l.cursor_on,
        false,
        WHITE,
    );
    mesh.rect(art, [0, 50, SIDEBAR_WIDTH, 26], BOX, SELECTED);
    mesh.rect(art, [0, 50, 2, 26], BOX + 1.0, GREEN);
    text(&mut mesh, "MINECRAFT", 8, 53, 1.0, GREY);
    text(&mut mesh, "Cardboard", 8, 64, 1.0, WHITE);
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    text(&mut mesh, &version, 6, h - 12, 1.0, GREY);

    // The header: the game's name, and the Play tab underlined.
    mesh.rect(
        art,
        [SIDEBAR_WIDTH, 0, w - SIDEBAR_WIDTH, HEADER_HEIGHT],
        PANEL,
        HEADER,
    );
    text(
        &mut mesh,
        "MINECRAFT: CARDBOARD",
        SIDEBAR_WIDTH + 10,
        6,
        1.0,
        WHITE,
    );
    text(&mut mesh, "Play", SIDEBAR_WIDTH + 10, 21, 1.0, WHITE);
    let tab = [
        SIDEBAR_WIDTH + 8,
        HEADER_HEIGHT - 3,
        text_width("Play") + 3,
        2,
    ];
    mesh.rect(art, tab, BOX, GREEN);

    // The bottom bar: the version, and Play.
    let bar = [
        SIDEBAR_WIDTH,
        h - BOTTOM_HEIGHT,
        w - SIDEBAR_WIDTH,
        BOTTOM_HEIGHT,
    ];
    mesh.rect(art, bar, PANEL, BOTTOM);
    if let Some(progress) = l.progress {
        let [x, y, w, _] = bar;
        let filled = (w as f32 * progress.clamp(0.0, 1.0)) as i32;
        mesh.rect(art, [x, y, w, 2], BOX, [60, 60, 60, 255]);
        mesh.rect(art, [x, y, filled, 2], BOX + 1.0, GREEN);
    }
    let left = SIDEBAR_WIDTH + 10;
    let (label, tag) = match &l.installed {
        Some(tag) => ("Latest release", tag.as_str()),
        None => ("Not installed", ""),
    };
    text(&mut mesh, label, left, h - BOTTOM_HEIGHT + 14, 1.0, WHITE);
    text(&mut mesh, tag, left, h - BOTTOM_HEIGHT + 26, 1.0, GREY);
    let hovered = l.mouse.is_some_and(|m| contains(layout.play, m));
    let color = match (l.play_active, hovered) {
        (false, _) => DISABLED,
        (true, false) => GREEN,
        (true, true) => GREEN_HOVER,
    };
    mesh.rect(art, layout.play, BOX, color);
    let [px, py, pw, ph] = layout.play;
    if l.play_focused {
        mesh.rect(art, [px - 1, py - 1, pw + 2, ph + 2], BOX - 1.0, [255; 4]);
    }
    let label = if l.button.is_empty() {
        "PLAY"
    } else {
        &l.button
    };
    let label_x = px + (pw - text_width(label) * 2) / 2;
    let rgb = if l.play_active { WHITE } else { GREY };
    text(&mut mesh, label, label_x, py + (ph - 14) / 2, 2.0, rgb);

    let message = match (&l.error, &l.status) {
        (Some(error), _) => Some((error, RED)),
        (None, Some(status)) => Some((status, WHITE)),
        (None, None) => None,
    };
    if let Some((message, rgb)) = message {
        let lines = art.wrap(message, w - SIDEBAR_WIDTH - 20);
        let bottom = h - BOTTOM_HEIGHT - 4;
        for (i, line) in lines.iter().enumerate() {
            let y = bottom - (lines.len() - i) as i32 * 9;
            let x = centre - text_width(line) / 2;
            mesh.shadowed(art, line, x as f32, y as f32, LABEL, 1.0, rgb);
        }
    }
    mesh
}
