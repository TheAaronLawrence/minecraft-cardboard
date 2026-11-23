//! The pause screen, as vanilla's `PauseScreen.createPauseMenu` lays it out over the HUD:
//! "Game Menu", Back to Game, Options, the buttons for screens this game doesn't have yet
//! (shown inactive), and Disconnect, or Save and Quit to Title in a world of our own.

use super::{Button, GuiMesh, HudArt, SCREEN};

const INWORLD_BACKGROUND: &str = "minecraft:gui/inworld_menu_background";

/// The texture drawn whole, by its full id.
pub(crate) const TEXTURES: [&str; 1] = [INWORLD_BACKGROUND];

/// The icon buttons' sprites, left to right: Report Bugs, Give Feedback, the friends list
/// and Player Reporting.
pub(crate) const ICONS: [&str; 4] = [
    "pause_menu/bug",
    "pause_menu/social_interactions",
    "friends/friends",
    "pause_menu/player_reporting",
];

/// What the pause screen shows.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Pause {
    /// The world is our own (singleplayer), so leaving is "Save and Quit to Title".
    pub local: bool,
    /// Where the mouse is, in GUI pixels, to highlight what's under it.
    pub mouse: Option<[f32; 2]>,
    /// The button clicked or moved to with the keyboard, highlighted.
    pub focused: Option<Action>,
}

/// What a pause screen button does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    ReturnToGame,
    Options,
    /// Disconnect, or Save and Quit to Title.
    Leave,
    /// A screen this game doesn't have yet.
    None,
}

/// The grid's size: two columns of 98-wide buttons padded 4 left and right, and rows padded
/// 4 above (50 above the first): Back to Game, Advancements and Statistics, the icons,
/// Options and World Options, and Disconnect.
const GRID: (i32, i32) = (2 * (98 + 8), (20 + 50) + 4 * (20 + 4));

/// The pause screen's buttons in a GUI `width` × `height` GUI pixels: what each does,
/// where it is, whether it works, and its icon if it's an icon button. The grid sits half
/// way across and a quarter of the way down (`FrameLayout.alignInRectangle`).
pub fn buttons(
    p: &Pause,
    width: i32,
    height: i32,
) -> Vec<(Action, Button, bool, Option<&'static str>)> {
    let x = (width - GRID.0) / 2;
    let y = ((height - GRID.1) as f32 * 0.25) as i32;
    let button = |bx, by, w, label| Button {
        rect: [x + bx, y + by, w, 20],
        label,
    };
    let leave = if p.local {
        "menu.returnToMenu"
    } else {
        "menu.disconnect"
    };
    let mut buttons = vec![
        (
            Action::ReturnToGame,
            button(4, 50, 204, "menu.returnToGame"),
            true,
            None,
        ),
        (
            Action::None,
            button(4, 74, 98, "gui.advancements"),
            false,
            None,
        ),
        (Action::None, button(110, 74, 98, "gui.stats"), false, None),
    ];
    // The icons: a row of four 20 wide, 4 apart, centred in the grid's width.
    let icons_x = 4 + (204 - (4 * 20 + 3 * 4)) / 2;
    for (i, icon) in ICONS.into_iter().enumerate() {
        let b = button(icons_x + i as i32 * 24, 98, 20, "");
        buttons.push((Action::None, b, false, Some(icon)));
    }
    buttons.extend([
        (
            Action::Options,
            button(4, 122, 98, "menu.options"),
            true,
            None,
        ),
        (
            Action::None,
            button(110, 122, 98, "options.worldOptions.button"),
            false,
            None,
        ),
        (Action::Leave, button(4, 146, 204, leave), true, None),
    ]);
    buttons
}

impl GuiMesh {
    /// `Screen.extractMenuBackground` over the world: the in-world menu background tiled.
    pub(crate) fn inworld_background(&mut self, art: &HudArt, width: i32, height: i32) {
        let tile = art.named[INWORLD_BACKGROUND];
        for y in (0..height).step_by(32) {
            for x in (0..width).step_by(32) {
                self.blit(x as f32, y as f32, SCREEN, 32.0, 32.0, tile, [1.0; 4]);
            }
        }
    }

    /// `PauseScreen.extractBackground`'s in-world menu background over the HUD (vanilla
    /// blurs what's behind it first, which this doesn't), then the title and buttons.
    pub(crate) fn pause(&mut self, art: &HudArt, p: &Pause, width: i32, height: i32) {
        self.inworld_background(art, width, height);
        let title = art
            .lang
            .get("menu.game")
            .map_or("menu.game", String::as_str);
        let x = width / 2 - GuiMesh::text_width(art, title) as i32 / 2;
        self.shadowed(art, title, x as f32, 40.0, SCREEN + 2.0, 1.0, [1.0; 3]);
        for (action, button, active, icon) in buttons(p, width, height) {
            let focused = active && p.focused == Some(action);
            self.button(art, &button, active, p.mouse, focused);
            // `SpriteIconButton.CenteredIcon`: a 15 × 15 sprite in the middle.
            if let Some(icon) = icon {
                let [bx, by, w, h] = button.rect;
                let (ix, iy) = (bx + w / 2 - 7, by + h / 2 - 7);
                let rect = art.named[icon];
                self.blit(
                    ix as f32,
                    iy as f32,
                    SCREEN + 2.0,
                    15.0,
                    15.0,
                    rect,
                    [1.0; 4],
                );
            }
        }
    }
}
