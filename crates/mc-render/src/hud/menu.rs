//! The game's menu screens, over the panorama and the menu background as vanilla draws a
//! screen with no world behind it: the title screen (`TitleScreen`, with "Cardboard" under
//! the logo where vanilla has its edition), Direct Connection (`DirectJoinServerScreen`),
//! `ConnectScreen` while joining, and `DisconnectedScreen` when joining fails or the
//! connection is lost; for singleplayer, Select World (`SelectWorldScreen`) and Create New
//! World (`CreateWorldScreen`). Also the text box (`EditBox`) they and the launcher type
//! into.

use super::{Button, GuiMesh, HudArt};

const PANORAMA: &str = "minecraft:gui/title/background/panorama_0";
const MENU_BACKGROUND: &str = "minecraft:gui/menu_background";
pub(crate) const LOGO: &str = "minecraft:gui/title/minecraft";

pub(crate) const HEADER_SEPARATOR: &str = "minecraft:gui/header_separator";
pub(crate) const FOOTER_SEPARATOR: &str = "minecraft:gui/footer_separator";
const TAB_HEADER_BACKGROUND: &str = "minecraft:gui/tab_header_background";

/// The textures the menus draw whole, by their full ids.
pub(crate) const TEXTURES: [&str; 6] = [
    PANORAMA,
    MENU_BACKGROUND,
    LOGO,
    HEADER_SEPARATOR,
    FOOTER_SEPARATOR,
    TAB_HEADER_BACKGROUND,
];

/// `MenuTabBar.MenuTabButton.SPRITES`: selected or not, each plain then highlighted.
pub(crate) const TABS: [&str; 4] = [
    "widget/tab",
    "widget/tab_highlighted",
    "widget/tab_selected",
    "widget/tab_selected_highlighted",
];

/// Create New World's tabs' titles.
const TAB_TITLES: [&str; 3] = [
    "createWorld.tab.game.title",
    "createWorld.tab.world.title",
    "createWorld.tab.more.title",
];

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
/// The world list's second and third lines (0x808080).
const LIST_GREY: [f32; 3] = [128.0 / 255.0; 3];
/// `EditBox.DEFAULT_HINT_STYLE`, dark grey (0x555555).
const HINT: [f32; 3] = [85.0 / 255.0; 3];
/// Hardcore Mode in the world list (0xFF0000).
const RED: [f32; 3] = [1.0, 0.0, 0.0];

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
    SelectWorld,
    CreateWorld,
    /// `ConnectScreen` (or a world loading): its message, or the translation key of it.
    Connecting(String),
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
    /// Direct Connection's server address.
    pub address: Field,
    /// Select World's search box, which filters the list.
    pub search: Field,
    /// The cursor is in the visible half of its blink.
    pub cursor_on: bool,
    /// Where the mouse is, in GUI pixels, to highlight what's under it.
    pub mouse: Option<[f32; 2]>,
    /// Select World's list, most recently played first, and which is selected.
    pub worlds: Vec<WorldRow>,
    pub selected: Option<usize>,
    /// What Create New World will create.
    pub new_world: NewWorld,
    /// What has the focus, highlighted: the widget last clicked or moved to with the
    /// keyboard.
    pub focused: Option<Focus>,
}

/// A menu screen's focused widget (`ContainerEventHandler.getFocused`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    Button(Action),
    /// The screen's text box: Direct Connection's address, Select World's search, or
    /// Create New World's name or seed, which takes typing.
    Box,
    /// Create New World's tab bar, on its current tab (`TabNavigationBar`).
    Tabs,
    /// Select World's list, on its selected world (`ObjectSelectionList`).
    List,
}

/// A world in Select World's list, as `WorldSelectionList.WorldListEntry` shows it.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct WorldRow {
    pub name: String,
    /// Its folder in `saves/`, the level id.
    pub folder: String,
    /// Its folder and, in brackets, when it was last played.
    pub folder_and_date: String,
    pub mode: GameMode,
    pub commands: bool,
    pub version: String,
}

/// `WorldCreationUiState.SelectedGameMode`, in the order its button cycles through.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum GameMode {
    #[default]
    Survival,
    Hardcore,
    Creative,
}

impl GameMode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Survival => "survival",
            Self::Hardcore => "hardcore",
            Self::Creative => "creative",
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name {
            "hardcore" => Self::Hardcore,
            "creative" => Self::Creative,
            _ => Self::Survival,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Survival => Self::Hardcore,
            Self::Hardcore => Self::Creative,
            Self::Creative => Self::Survival,
        }
    }
}

/// `Difficulty`, by its name.
pub const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];

/// Create New World's choices, `WorldCreationUiState`.
#[derive(Clone, PartialEq, Debug)]
pub struct NewWorld {
    pub name: Field,
    /// Blank for a random seed.
    pub seed: Field,
    /// The tab showing: Game (with the name), World (with the seed) or More.
    pub tab: usize,
    pub mode: GameMode,
    pub structures: bool,
    /// Into `DIFFICULTIES`.
    difficulty: usize,
    /// Chosen with the button; until then, on in Creative only.
    allow_commands: Option<bool>,
}

impl Default for NewWorld {
    fn default() -> Self {
        Self {
            name: Field::new("New World"),
            seed: Field::default(),
            tab: 0,
            mode: GameMode::Survival,
            structures: true,
            difficulty: 2,
            allow_commands: None,
        }
    }
}

impl NewWorld {
    /// `WorldCreationUiState.getDifficulty`: Hardcore is always Hard.
    pub fn difficulty(&self) -> &'static str {
        match self.mode {
            GameMode::Hardcore => "hard",
            _ => DIFFICULTIES[self.difficulty],
        }
    }

    /// `WorldCreationUiState.isAllowCommands`.
    pub fn commands(&self) -> bool {
        match self.mode {
            GameMode::Hardcore => false,
            mode => self.allow_commands.unwrap_or(mode == GameMode::Creative),
        }
    }

    /// The tab's box, if it has one.
    pub fn field(&mut self) -> Option<&mut Field> {
        match self.tab {
            0 => Some(&mut self.name),
            1 => Some(&mut self.seed),
            _ => None,
        }
    }
}

impl Menu {
    /// The screen's text box: Direct Connection's address, Select World's search, or the
    /// Create New World tab's box.
    pub fn field(&mut self) -> Option<&mut Field> {
        match self.screen {
            Screen::DirectConnect => Some(&mut self.address),
            Screen::SelectWorld => Some(&mut self.search),
            Screen::CreateWorld => self.new_world.field(),
            _ => None,
        }
    }

    /// Where the screen's text box is, in a GUI `width` × `height`.
    pub fn field_rect(&self, width: i32, height: i32) -> Option<[i32; 4]> {
        match self.screen {
            Screen::DirectConnect => Some(address_rect(width)),
            Screen::SelectWorld => Some(search_rect(width)),
            Screen::CreateWorld => new_world_box(self.new_world.tab, width, height),
            _ => None,
        }
    }
}

/// What a menu button does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Singleplayer,
    Multiplayer,
    Options,
    Quit,
    Join,
    /// Cancel, Back, and Back to Server List: each screen goes back to the one it came from.
    Back,
    /// Select World's.
    PlaySelected,
    OpenCreateWorld,
    /// Edit, Delete and Re-Create, which don't work yet.
    Unavailable,
    /// Create New World's.
    Create,
    CycleGameMode,
    CycleDifficulty,
    ToggleCommands,
    ToggleStructures,
}

/// `MenuTabBar.arrangeElements`: the tabs share the width less 28 (at most 400), each a
/// width even, centred on an even x, along the top 24.
pub fn tab_rects(width: i32) -> [[i32; 4]; 3] {
    let tabs = width.min(400) - 28;
    let tab = tabs / 3 / 2 * 2;
    let left = (width - tabs) / 2 / 2 * 2;
    std::array::from_fn(|i| [left + i as i32 * tab, 0, tab, 24])
}

/// The top left of tab `tab`'s grid, `w` × `h`: centred across, and a sixth of the way down
/// the space between the tab bar (24 high) and the footer (33), as `GridLayoutTab` places
/// it.
fn tab_grid(width: i32, height: i32, w: i32, h: i32) -> (i32, i32) {
    let area = height - 33 - 24;
    ((width - w) / 2, 24 + ((area - h) as f32 / 6.0) as i32)
}

/// The tab's text box: `GameTab`'s name (208 wide in its 210-wide grid, under its label),
/// or `WorldTab`'s seed (308 in 310, under World Type and its label).
pub fn new_world_box(tab: usize, width: i32, height: i32) -> Option<[i32; 4]> {
    match tab {
        0 => {
            let (x, y) = tab_grid(width, height, 210, GAME_GRID);
            Some([x + 1, y + 13, 208, 20])
        }
        1 => {
            let (x, y) = tab_grid(width, height, 310, WORLD_GRID);
            Some([x + 1, y + 28 + 13, 308, 20])
        }
        _ => None,
    }
}

/// `GameTab`'s grid: the name with its label (9 + 4 + 20), then Game Mode, Difficulty and
/// Allow Commands, 8 apart.
const GAME_GRID: i32 = 33 + 3 * 28;
/// `WorldTab`'s: World Type and Customize, the seed with its label, then the switches for
/// structures and the bonus chest, rows of 24.
const WORLD_GRID: i32 = 20 + 8 + 33 + 8 + 48;
/// `MoreTab`'s three buttons.
const MORE_GRID: i32 = 3 * 28 - 8;

/// Select World's list: below the 49-high header, rows 36 high and 270 wide, above the
/// 60-high footer.
const LIST_TOP: i32 = 49;
const ROW: i32 = 36;

/// The world under GUI point `mouse`, if any, by its index in `menu.worlds`.
pub fn world_at(menu: &Menu, width: i32, height: i32, [x, y]: [f32; 2]) -> Option<usize> {
    let left = (width / 2 - 135) as f32;
    if x < left || x >= left + 270.0 || y < (LIST_TOP + 4) as f32 {
        return None;
    }
    let row = ((y as i32 - LIST_TOP - 4) / ROW) as usize;
    shown_worlds(menu, height).get(row).copied()
}

/// The worlds the list shows, as many as fit, by their indices in `menu.worlds`: those
/// whose name or folder has the search in it (`WorldSelectionList.filterAccepts`).
pub fn shown_worlds(menu: &Menu, height: i32) -> Vec<usize> {
    let search = menu.search.value.to_lowercase();
    let worlds = menu.worlds.iter().enumerate();
    worlds
        .filter(|(_, w)| {
            w.name.to_lowercase().contains(&search) || w.folder.to_lowercase().contains(&search)
        })
        .map(|(i, _)| i)
        .take(visible_rows(height))
        .collect()
}

/// Select World's list, across the screen between the header and the footer.
pub fn list_rect(width: i32, height: i32) -> [i32; 4] {
    [0, LIST_TOP, width, height - LIST_TOP - 60]
}

/// Select World's search box, under the title.
pub fn search_rect(width: i32) -> [i32; 4] {
    [width / 2 - 100, 22, 200, 20]
}

/// How many rows fit between the header and the footer.
fn visible_rows(height: i32) -> usize {
    ((height - LIST_TOP - 60 - 4) / ROW).max(0) as usize
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
                    true,
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
        // `SelectWorldScreen.createFooterButtons`: a grid four 71-wide columns 8 apart, its
        // rows 4 apart, centred in the 60-high footer.
        Screen::SelectWorld => {
            let (x, y) = (width / 2 - 154, height - 52);
            let selected = menu.selected.is_some();
            vec![
                (
                    Action::PlaySelected,
                    button(x, y, 150, "selectWorld.select"),
                    selected,
                ),
                (
                    Action::OpenCreateWorld,
                    button(x + 158, y, 150, "selectWorld.create"),
                    true,
                ),
                (
                    Action::Unavailable,
                    button(x, y + 24, 71, "selectWorld.edit"),
                    false,
                ),
                (
                    Action::Unavailable,
                    button(x + 79, y + 24, 71, "selectWorld.delete"),
                    false,
                ),
                (
                    Action::Unavailable,
                    button(x + 158, y + 24, 71, "selectWorld.recreate"),
                    false,
                ),
                (Action::Back, button(x + 237, y + 24, 71, "gui.back"), true),
            ]
        }
        // The tab's buttons, then the footer's two, 150 wide and 8 apart.
        Screen::CreateWorld => {
            let hardcore = menu.new_world.mode == GameMode::Hardcore;
            let mut buttons = match menu.new_world.tab {
                // `GameTab`: 210 wide below the name.
                0 => {
                    let (x, y) = tab_grid(width, height, 210, GAME_GRID);
                    let y = y + 33 + 8;
                    vec![
                        (
                            Action::CycleGameMode,
                            button(x, y, 210, "selectWorld.gameMode"),
                            true,
                        ),
                        (
                            Action::CycleDifficulty,
                            button(x, y + 28, 210, "options.difficulty"),
                            !hardcore,
                        ),
                        (
                            Action::ToggleCommands,
                            button(x, y + 56, 210, "selectWorld.allowCommands.new"),
                            !hardcore,
                        ),
                    ]
                }
                // `WorldTab`: World Type and Customize, 150 wide and 10 apart, then
                // `SwitchGrid`'s 44-wide switches at the right of its rows.
                1 => {
                    let (x, y) = tab_grid(width, height, 310, WORLD_GRID);
                    let switches = y + 20 + 8 + 33 + 8;
                    vec![
                        (
                            Action::Unavailable,
                            button(x, y, 150, "selectWorld.mapType"),
                            false,
                        ),
                        (
                            Action::Unavailable,
                            button(x + 160, y, 150, "selectWorld.customizeType"),
                            false,
                        ),
                        (
                            Action::ToggleStructures,
                            button(x + 266, switches + 4, 44, "selectWorld.mapFeatures"),
                            true,
                        ),
                        (
                            Action::Unavailable,
                            button(x + 266, switches + 28, 44, "selectWorld.bonusItems"),
                            false,
                        ),
                    ]
                }
                // `MoreTab`: what the game can't change yet.
                _ => {
                    let (x, y) = tab_grid(width, height, 210, MORE_GRID);
                    [
                        "selectWorld.gameRules",
                        "selectWorld.experiments",
                        "selectWorld.dataPacks",
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(i, label)| {
                        let b = button(x, y + i as i32 * 28, 210, label);
                        (Action::Unavailable, b, false)
                    })
                    .collect()
                }
            };
            buttons.extend([
                (
                    Action::Create,
                    button(width / 2 - 154, height - 27, 150, "selectWorld.create"),
                    true,
                ),
                (
                    Action::Back,
                    button(width / 2 + 4, height - 27, 150, "gui.cancel"),
                    true,
                ),
            ]);
            buttons
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
    if m.screen == Screen::CreateWorld {
        // `CreateWorldScreen.extractMenuBackground`: the tab header's background above the
        // menu background.
        mesh.background_from(art, w, h, 24);
        let header = art.named[TAB_HEADER_BACKGROUND];
        mesh.tiled([0, 0, w, 24], [16, 16], header, BACKGROUND + 1.0);
    } else {
        mesh.background(art, w, h);
    }
    let lang = |key: &str| art.lang.get(key).cloned().unwrap_or_else(|| key.to_owned());
    // `EditBox`: highlighted, with its cursor blinking, while it has the focus.
    let boxed = m.focused == Some(Focus::Box);
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
            mesh.edit_box(art, &m.address, address_rect(w), boxed, m.cursor_on);
        }
        Screen::SelectWorld => {
            centred(&mut mesh, &lang("selectWorld.title"), 8, WHITE);
            let search = search_rect(w);
            mesh.edit_box(art, &m.search, search, boxed, m.cursor_on);
            // `EditBox`'s hint, `SEARCH_HINT_STYLE`: grey and italic, in an empty box
            // without the focus.
            if m.search.value.is_empty() && !boxed {
                let span = mc_protocol::chat::Span {
                    text: lang("gui.selectWorld.search"),
                    rgb: [0; 3],
                    bold: false,
                    italic: true,
                };
                let (x, y) = ((search[0] + 4) as f32, (search[1] + 6) as f32);
                mesh.styled(art, &span, x, y, LABEL, [170.0 / 255.0; 3], 1.0);
            }
            let left = w / 2 - 135;
            for (i, index) in shown_worlds(m, h).into_iter().enumerate() {
                let world = &m.worlds[index];
                let top = LIST_TOP + 4 + i as i32 * ROW;
                // `AbstractSelectionList.extractSelection`: outlined in white while the
                // list has the focus, grey otherwise; black inside.
                if m.selected == Some(index) {
                    let outline = if m.focused == Some(Focus::List) {
                        [255; 4]
                    } else {
                        [128, 128, 128, 255]
                    };
                    mesh.rect(art, [left, top, 270, ROW], WIDGET, outline);
                    let inside = [left + 1, top + 1, 268, ROW - 2];
                    mesh.rect(art, inside, WIDGET + 1.0, [0, 0, 0, 255]);
                }
                // The text beside where the world's icon goes.
                let x = (left + 2 + 32 + 3) as f32;
                let y = top + 2;
                let text = |mesh: &mut GuiMesh, t: &str, x: f32, dy: i32, rgb| {
                    mesh.shadowed(art, t, x, (y + dy) as f32, LABEL, 1.0, rgb);
                };
                text(&mut mesh, &world.name, x, 1, WHITE);
                text(&mut mesh, &world.folder_and_date, x, 12, LIST_GREY);
                // `LevelSummary.createInfo`.
                let (mode, rgb) = match world.mode {
                    GameMode::Hardcore => (lang("gameMode.hardcore"), RED),
                    mode => (lang(&format!("gameMode.{}", mode.name())), LIST_GREY),
                };
                text(&mut mesh, &mode, x, 23, rgb);
                let mut rest = String::new();
                if world.commands {
                    rest += &format!(", {}", lang("selectWorld.commands"));
                }
                rest += &format!(", {} {}", lang("selectWorld.version"), world.version);
                let after = x + GuiMesh::text_width(art, &mode);
                text(&mut mesh, &rest, after, 23, LIST_GREY);
            }
        }
        Screen::CreateWorld => {
            let new = &m.new_world;
            mesh.tab_bar(art, w, new.tab, m.mouse, m.focused == Some(Focus::Tabs));
            // `CreateWorldScreen.extractRenderState`'s footer separator.
            let footer = art.named[FOOTER_SEPARATOR];
            mesh.tiled([0, h - 33 - 2, w, 2], [32, 2], footer, BACKGROUND + 3.0);
            if let Some(rect) = new_world_box(new.tab, w, h) {
                let (field, label) = match new.tab {
                    0 => (&new.name, "selectWorld.enterName"),
                    _ => (&new.seed, "selectWorld.enterSeed"),
                };
                let (x, y) = (rect[0] as f32, (rect[1] - 13) as f32);
                mesh.shadowed(art, &lang(label), x, y, LABEL, 1.0, WHITE);
                mesh.edit_box(art, field, rect, boxed, m.cursor_on);
                // `EditBox`'s hint, in an empty box without the focus.
                if field.value.is_empty() && new.tab == 1 && !boxed {
                    let (x, y) = ((rect[0] + 4) as f32, (rect[1] + 6) as f32);
                    let hint = lang("selectWorld.seedInfo");
                    mesh.shadowed(art, &hint, x, y, LABEL, 1.0, HINT);
                }
            }
            // `SwitchGrid`'s labels, left of their switches.
            if new.tab == 1 {
                let (x, y) = tab_grid(w, h, 310, WORLD_GRID);
                let y = y + 20 + 8 + 33 + 8 + 4 + 6;
                for (i, label) in ["selectWorld.mapFeatures", "selectWorld.bonusItems"]
                    .into_iter()
                    .enumerate()
                {
                    let rgb = if i == 0 { WHITE } else { GREY };
                    let y = (y + i as i32 * 24) as f32;
                    mesh.shadowed(art, &lang(label), x as f32, y, LABEL, 1.0, rgb);
                }
            }
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
    for (action, button, active) in buttons(m, art, w, h) {
        let focused = m.focused == Some(Focus::Button(action));
        // `CycleButton`s show their value: "Game Mode: Survival".
        let new = &m.new_world;
        let on_off = |on| lang(if on { "options.on" } else { "options.off" });
        let value = match action {
            Action::CycleGameMode => lang(&format!("selectWorld.gameMode.{}", new.mode.name())),
            Action::CycleDifficulty => lang(&format!("options.difficulty.{}", new.difficulty())),
            Action::ToggleCommands => on_off(new.commands()),
            Action::Unavailable if button.label == "selectWorld.mapType" => {
                lang("generator.minecraft.normal")
            }
            // Switches show only their value.
            Action::ToggleStructures => {
                mesh.labelled_button(
                    art,
                    &button,
                    &on_off(new.structures),
                    active,
                    m.mouse,
                    focused,
                );
                continue;
            }
            Action::Unavailable if button.label == "selectWorld.bonusItems" => {
                mesh.labelled_button(art, &button, &on_off(false), active, m.mouse, false);
                continue;
            }
            _ => {
                mesh.button(art, &button, active, m.mouse, focused);
                continue;
            }
        };
        let label = lang("options.generic_value")
            .replacen("%s", &lang(button.label), 1)
            .replacen("%s", &value, 1);
        mesh.labelled_button(art, &button, &label, active, m.mouse, focused);
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
        self.background_from(art, w, h, 0);
    }

    /// The background with the menu background's tiles starting at `top`.
    fn background_from(&mut self, art: &HudArt, w: i32, h: i32, top: i32) {
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
        self.tiled([0, top, w, h - top], [32, 32], tile, BACKGROUND + 1.0);
    }

    /// A texture `size` GUI pixels big repeated across the rectangle (x, y, w, h) from its
    /// top left, the last ones cut short, as a blit with UVs past the texture's edge repeats.
    fn tiled(&mut self, [x, y, w, h]: [i32; 4], size: [i32; 2], rect: [f32; 4], z: f32) {
        let [u0, v0, u1, v1] = rect;
        for ty in (0..h).step_by(size[1] as usize) {
            for tx in (0..w).step_by(size[0] as usize) {
                let (cw, ch) = ((w - tx).min(size[0]), (h - ty).min(size[1]));
                let u = u0 + (u1 - u0) * cw as f32 / size[0] as f32;
                let v = v0 + (v1 - v0) * ch as f32 / size[1] as f32;
                let (px, py) = ((x + tx) as f32, (y + ty) as f32);
                self.blit(px, py, z, cw as f32, ch as f32, [u0, v0, u, v], [1.0; 4]);
            }
        }
    }

    /// `MenuTabBar.extractWidgetRenderState`: the header separator either side of the tabs,
    /// then each tab: its sprite nine-sliced (130 × 24, 2 at the sides), the menu background
    /// inside the selected one with a line under its title, and the title, 3 lower on the
    /// others. The selected tab is highlighted while the bar has the focus, as any is under
    /// the mouse.
    fn tab_bar(
        &mut self,
        art: &HudArt,
        w: i32,
        selected: usize,
        mouse: Option<[f32; 2]>,
        focused: bool,
    ) {
        let tabs = tab_rects(w);
        let separator = art.named[HEADER_SEPARATOR];
        let first = tabs[0][0];
        let last = tabs[2][0] + tabs[2][2];
        self.tiled([0, 22, first, 2], [32, 2], separator, WIDGET);
        self.tiled([last, 22, w - last, 2], [32, 2], separator, WIDGET);
        for (i, [x, y, tw, th]) in tabs.into_iter().enumerate() {
            let hovered = mouse.is_some_and(|[mx, my]| {
                mx >= x as f32 && my >= y as f32 && mx < (x + tw) as f32 && my < (y + th) as f32
            });
            let is = i == selected;
            let lit = hovered || is && focused;
            let [u0, v0, u1, v1] = art.named[TABS[is as usize * 2 + lit as usize]];
            let u = |px: i32| u0 + (u1 - u0) * px as f32 / 130.0;
            let mut column = |left: i32, from: i32, width: i32| {
                let rect = [u(from), v0, u(from + width), v1];
                let (l, t) = (left as f32, y as f32);
                self.blit(l, t, WIDGET, width as f32, th as f32, rect, [1.0; 4]);
            };
            column(x, 0, 2);
            column(x + 2, 2, tw - 4);
            column(x + tw - 2, 128, 2);
            let title = art
                .lang
                .get(TAB_TITLES[i])
                .map_or(TAB_TITLES[i], String::as_str);
            let width = GuiMesh::text_width(art, title) as i32;
            if is {
                let tile = art.named[MENU_BACKGROUND];
                self.tiled([x + 2, y + 2, tw - 4, th - 2], [32, 32], tile, WIDGET + 1.0);
                let line = width.min(tw - 4);
                self.rect(
                    art,
                    [x + (tw - line) / 2, y + th - 2, line, 1],
                    WIDGET + 2.0,
                    [255; 4],
                );
            }
            let top = if is { y } else { y + 3 };
            let (tx, ty) = (x + tw / 2 - width / 2, (top + y + th - 9) / 2 + 1);
            self.shadowed(art, title, tx as f32, ty as f32, LABEL, 1.0, WHITE);
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
        self.field_text(art, field, rect, focused && cursor, true, FIELD_TEXT);
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

impl Menu {
    /// Create New World's buttons that change a choice.
    pub fn cycle(&mut self, action: Action) {
        let new = &mut self.new_world;
        match action {
            Action::CycleGameMode => new.mode = new.mode.next(),
            Action::CycleDifficulty => new.difficulty = (new.difficulty + 1) % DIFFICULTIES.len(),
            Action::ToggleCommands => new.allow_commands = Some(!new.commands()),
            Action::ToggleStructures => new.structures = !new.structures,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, Field, GameMode, Menu, valid_address};

    /// Commands follow Creative until chosen, and Hardcore is Hard without commands
    /// whatever was chosen, as `WorldCreationUiState` decides.
    #[test]
    fn new_world_choices() {
        let mut m = Menu::default();
        assert_eq!(
            (m.new_world.difficulty(), m.new_world.commands()),
            ("normal", false)
        );
        m.cycle(Action::CycleGameMode);
        assert_eq!(m.new_world.mode, GameMode::Hardcore);
        assert_eq!(
            (m.new_world.difficulty(), m.new_world.commands()),
            ("hard", false)
        );
        m.cycle(Action::CycleGameMode);
        assert_eq!(
            (m.new_world.difficulty(), m.new_world.commands()),
            ("normal", true)
        );
        m.cycle(Action::ToggleCommands);
        m.cycle(Action::CycleDifficulty);
        m.cycle(Action::ToggleStructures);
        assert!(!m.new_world.structures);
        m.cycle(Action::CycleGameMode);
        assert_eq!(m.new_world.mode, GameMode::Survival);
        assert_eq!(
            (m.new_world.difficulty(), m.new_world.commands()),
            ("hard", false)
        );
    }

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
