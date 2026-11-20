//! The HUD drawn over the world: the hotbar with its items and their counts, as vanilla's
//! `Gui.renderItemHotbar` lays it out, and in survival the hearts, food and experience bar
//! above it, as `Hud.extractPlayerHealth` and `ExperienceBar` do, in GUI pixels (screen
//! pixels divided by the GUI scale). Also the cracks over a block being mined, as vanilla's crumbling overlay draws
//! them. Everything samples the block atlas, which holds the HUD's sprites and the font.
//! Over it all, the inventory (`hud/inventory.rs`) and the death screen, as vanilla's `DeathScreen` and its `TitleConfirmScreen`
//! lay them out.

pub mod chat;
pub mod inventory;
pub mod launcher;
pub mod menu;
pub mod pause;

use crate::atlas::Atlas;
use crate::bake::{BlockModels, Dir};
use crate::item::ItemIcon;
use crate::mesh::Vertex;
use anyhow::{Context, Result};
use glam::{Mat3, Quat, Vec3};
use std::collections::HashMap;
use std::sync::Arc;

const HOTBAR: &str = "minecraft:gui/sprites/hud/hotbar";
const SELECTION: &str = "minecraft:gui/sprites/hud/hotbar_selection";
const FONT: &str = "minecraft:font/ascii";
const XP_BACKGROUND: &str = "hud/experience_bar_background";
const XP_PROGRESS: &str = "hud/experience_bar_progress";
const FOOD: [&str; 3] = ["hud/food_empty", "hud/food_half", "hud/food_full"];

/// `Hud.HeartType`'s sprites for a container or a normal heart, which come in hardcore,
/// half and blinking forms; a container has no half form.
fn heart(container: bool, hardcore: bool, half: bool, blink: bool) -> String {
    let blink = if blink { "_blinking" } else { "" };
    if container {
        let hardcore = if hardcore { "_hardcore" } else { "" };
        format!("hud/heart/container{hardcore}{blink}")
    } else {
        let hardcore = if hardcore { "hardcore_" } else { "" };
        let size = if half { "half" } else { "full" };
        format!("hud/heart/{hardcore}{size}{blink}")
    }
}

/// `AbstractButton.SPRITES`: enabled, disabled and highlighted.
const BUTTON: [&str; 3] = [
    "widget/button",
    "widget/button_disabled",
    "widget/button_highlighted",
];

/// Depths in GUI pixels of a screen over the HUD: its background, buttons and text.
const SCREEN: f32 = 400.0;

/// The GUI sprites drawn by name, named as vanilla's `Identifier`s for them are.
fn named_sprites() -> Vec<String> {
    let mut names: Vec<String> = [XP_BACKGROUND, XP_PROGRESS]
        .into_iter()
        .chain(FOOD)
        .chain(BUTTON)
        .chain(menu::TEXT_FIELD)
        .chain(pause::ICONS)
        .chain(inventory::SPRITES)
        .map(str::to_owned)
        .collect();
    for i in 0..16 {
        let bit = |b: u32| i & 1 << b != 0;
        names.push(heart(bit(0), bit(1), bit(2), bit(3)));
    }
    names.sort();
    names.dedup();
    names
}

fn gui_sprite(name: &str) -> String {
    format!("minecraft:gui/sprites/{name}")
}

/// Depths in GUI pixels toward the viewer: sprites behind the items, which sit around 0,
/// and text in front of them.
const BACK: f32 = -200.0;
const TEXT: f32 = 200.0;

fn crack_sprite(stage: u8) -> String {
    format!("minecraft:block/destroy_stage_{stage}")
}

/// The HUD's sprites, for the atlas.
pub(crate) fn sprites() -> Vec<String> {
    [HOTBAR, SELECTION, FONT]
        .map(str::to_owned)
        .into_iter()
        .chain((0..10).map(crack_sprite))
        .chain(named_sprites().iter().map(|n| gui_sprite(n)))
        .chain(menu::TEXTURES.map(str::to_owned))
        .chain(pause::TEXTURES.map(str::to_owned))
        .chain(inventory::TEXTURES.map(str::to_owned))
        .collect()
}

/// What the HUD draws with: item icons and where its sprites are in the atlas.
#[derive(Clone)]
pub struct HudArt {
    pub items: Arc<HashMap<String, ItemIcon>>,
    hotbar: [f32; 4],
    selection: [f32; 4],
    font: [f32; 4],
    /// Each character's cell in `font/ascii.png` (16 × 16 cells of 8 × 8) and advance.
    glyphs: HashMap<char, (u8, u8)>,
    cracks: [[f32; 4]; 10],
    /// The GUI sprites drawn by name, by vanilla's name for them, and the menus' textures
    /// by their full ids.
    named: HashMap<String, [f32; 4]>,
    /// The centre of a plain white sprite, for filling with a colour.
    white: [f32; 2],
    /// The death screen's English text, by translation key.
    lang: HashMap<String, String>,
    /// Each item's name key and rarity, by item, for tooltips.
    item_names: HashMap<String, (String, String)>,
}

impl HudArt {
    /// A font cell's rect in the atlas.
    fn glyph_rect(&self, cell: u8) -> [f32; 4] {
        let [u0, v0, u1, v1] = self.font;
        let (du, dv) = ((u1 - u0) / 16.0, (v1 - v0) / 16.0);
        let (col, row) = ((cell % 16) as f32, (cell / 16) as f32);
        [
            u0 + du * col,
            v0 + dv * row,
            u0 + du * (col + 1.0),
            v0 + dv * (row + 1.0),
        ]
    }

    /// `StringSplitter.splitLines`: `text` broken at spaces into lines at most `max` GUI
    /// pixels wide, and inside a word too wide for a line of its own, as vanilla breaks one.
    pub(crate) fn wrap(&self, text: &str, max: i32) -> Vec<String> {
        let width = |s: &str| GuiMesh::text_width(self, s) as i32;
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for word in paragraph.split(' ') {
                let candidate = if line.is_empty() {
                    word.to_owned()
                } else {
                    format!("{line} {word}")
                };
                if width(&candidate) <= max {
                    line = candidate;
                    continue;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                for c in word.chars() {
                    line.push(c);
                    if width(&line) > max && line.chars().count() > 1 {
                        line.pop();
                        lines.push(std::mem::replace(&mut line, c.to_string()));
                    }
                }
            }
            lines.push(line);
        }
        lines
    }

    pub(crate) fn new(
        res: &mc_assets::Resources,
        atlas: &Atlas,
        items: HashMap<String, ItemIcon>,
        item_names: HashMap<String, (String, String)>,
    ) -> Result<Self> {
        Ok(Self {
            items: Arc::new(items),
            hotbar: atlas.sprite(HOTBAR).rect,
            selection: atlas.sprite(SELECTION).rect,
            font: atlas.sprite(FONT).rect,
            glyphs: glyphs(res)?,
            cracks: std::array::from_fn(|i| atlas.sprite(&crack_sprite(i as u8)).rect),
            named: named_sprites()
                .into_iter()
                .map(|n| {
                    let rect = atlas.sprite(&gui_sprite(&n)).rect;
                    (n, rect)
                })
                .chain(
                    menu::TEXTURES
                        .into_iter()
                        .chain(pause::TEXTURES)
                        .chain(inventory::TEXTURES)
                        .map(|id| (id.to_owned(), atlas.sprite(id).rect)),
                )
                .collect(),
            white: {
                let [u0, v0, u1, v1] = atlas.sprite(crate::atlas::WHITE_SPRITE).rect;
                [(u0 + u1) / 2.0, (v0 + v1) / 2.0]
            },
            lang: gui_lang(res)?,
            item_names,
        })
    }
}

/// Each item's name key and rarity, from the `items.json` report (`[key, max stack, rarity,
/// slot, equipment asset]`).
pub(crate) fn item_names() -> Result<HashMap<String, (String, String)>> {
    let path = mc_registry::default_reports_dir().join("items.json");
    type Report = (String, i32, String, Option<String>, Option<String>);
    let items: HashMap<String, Report> = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("{}", path.display()))?,
    )?;
    Ok(items
        .into_iter()
        .map(|(item, (key, _, rarity, ..))| (item, (key, rarity)))
        .collect())
}

/// The English translations of the keys the death screen and the menus use.
fn gui_lang(res: &mc_assets::Resources) -> Result<HashMap<String, String>> {
    let path = res.asset_path("lang", "minecraft:en_us", "json");
    let lang: HashMap<String, serde_json::Value> = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("{}", path.display()))?,
    )?;
    Ok(lang
        .into_iter()
        .filter(|(k, _)| {
            [
                "deathScreen.",
                "addServer.",
                "selectServer.",
                "menu.",
                "connect.",
                "disconnect.",
                "gui.",
                "options.",
                "container.",
                "item.",
                "block.",
            ]
            .iter()
            .any(|p| k.starts_with(p))
        })
        .filter_map(|(k, v)| Some((k, v.as_str()?.to_owned())))
        .collect())
}

/// The default font's `ascii.png` glyphs: which character each cell holds, from the font's
/// provider list, and its advance, `BitmapProvider`'s width of its rightmost non-clear column
/// plus one.
fn glyphs(res: &mc_assets::Resources) -> Result<HashMap<char, (u8, u8)>> {
    let path = res.asset_path("font", "minecraft:include/default", "json");
    let json: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("{}", path.display()))?,
    )?;
    let provider = json["providers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["file"].as_str() == Some("minecraft:font/ascii.png"))
        .context("the default font has no ascii.png")?;
    let image = image::open(res.asset_path("textures", FONT, "png"))?.to_rgba8();
    let mut glyphs = HashMap::new();
    for (row, chars) in provider["chars"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        for (col, c) in chars.as_str().unwrap_or_default().chars().enumerate() {
            if c == '\0' {
                continue;
            }
            let (x0, y0) = (col as u32 * 8, row as u32 * 8);
            let width = (0..8)
                .rev()
                .find(|&x| (0..8).any(|y| image.get_pixel(x0 + x, y0 + y)[3] != 0))
                .map_or(0, |x| x + 1);
            glyphs.insert(c, ((row * 16 + col) as u8, width as u8 + 1));
        }
    }
    // The space provider's.
    glyphs.insert(' ', (0, 4));
    Ok(glyphs)
}

/// What the hotbar shows: each slot's item name and count, and the slot selected; and in
/// survival and adventure, what's above it.
#[derive(Clone, Default, PartialEq)]
pub struct Hotbar {
    pub slots: [Option<(String, i32)>; 9],
    pub selected: u8,
    pub vitals: Option<Vitals>,
    /// The death screen, when the player has died.
    pub death: Option<DeathScreen>,
    pub chat: chat::Chat,
}

/// A screen drawn instead of the HUD.
#[derive(Clone, PartialEq, Debug)]
pub enum Screen {
    Menu(menu::Menu),
    /// Drawn over the HUD rather than instead of it.
    Pause(pause::Pause),
    /// Drawn over the HUD too.
    Inventory(inventory::Inventory),
    Launcher(launcher::Launcher),
}

/// What the death screen shows.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct DeathScreen {
    pub hardcore: bool,
    /// The death message, or none when the screen opened without one (such as on joining
    /// while dead).
    pub message: Option<String>,
    pub score: i32,
    /// The buttons work: a second after the screen (or the confirmation) opened.
    pub active: bool,
    /// Showing `TitleConfirmScreen`, which asks before leaving for the title screen.
    pub confirm: bool,
    /// Where the mouse is, in GUI pixels, to highlight the button under it.
    pub mouse: Option<[f32; 2]>,
}

/// A button on the death screen: x, y, width and height in GUI pixels, and its label's
/// translation key.
pub struct Button {
    pub rect: [i32; 4],
    pub label: &'static str,
}

impl Button {
    pub fn contains(&self, [x, y]: [f32; 2]) -> bool {
        let [bx, by, w, h] = self.rect.map(|c| c as f32);
        x >= bx && y >= by && x < bx + w && y < by + h
    }
}

/// The death screen's two buttons in a GUI `width` × `height` GUI pixels.
/// `DeathScreen.init`: Respawn (or Spectate World in hardcore) above Title Screen, 200 wide.
/// `TitleConfirmScreen`, from `ConfirmScreen.init`: the title, an empty message, then Title
/// Screen and Respawn side by side, 150 wide and 4 apart, the whole centred on the screen.
pub fn death_buttons(d: &DeathScreen, width: i32, height: i32) -> [Button; 2] {
    if d.confirm {
        let (x, y) = confirm_origin(width, height);
        [
            Button {
                rect: [x, y + CONFIRM_BUTTONS_Y, 150, 20],
                label: "deathScreen.titleScreen",
            },
            Button {
                rect: [x + 154, y + CONFIRM_BUTTONS_Y, 150, 20],
                label: "deathScreen.respawn",
            },
        ]
    } else {
        let respawn = if d.hardcore {
            "deathScreen.spectate"
        } else {
            "deathScreen.respawn"
        };
        [
            Button {
                rect: [width / 2 - 100, height / 4 + 72, 200, 20],
                label: respawn,
            },
            Button {
                rect: [width / 2 - 100, height / 4 + 96, 200, 20],
                label: "deathScreen.titleScreen",
            },
        ]
    }
}

/// The confirmation's layout, a column 8 apart: the title (9 high), the empty message (0),
/// and the buttons' row, padded 16 above (36). Its width is the buttons' 304.
const CONFIRM_HEIGHT: i32 = 9 + 8 + 8 + 16 + 20;
const CONFIRM_BUTTONS_Y: i32 = 9 + 8 + 8 + 16;

/// `FrameLayout.centerInRectangle`: the confirmation's top left.
fn confirm_origin(width: i32, height: i32) -> (i32, i32) {
    ((width - 304) / 2, (height - CONFIRM_HEIGHT) / 2)
}

/// What the hearts, food and experience bar show, as `Hud.extractPlayerHealth` works them
/// out each frame.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Vitals {
    /// `Hud.tickCount`, which seeds the hearts' and food's shaking.
    pub tick: i32,
    /// Health in half hearts, rounded up.
    pub health: i32,
    /// The health shown a moment ago (`displayHealth`), which blinking hearts show.
    pub old_health: i32,
    /// Whether the hearts are in the bright half of a blink, after the health changed.
    pub blink: bool,
    pub max_health: f32,
    pub hardcore: bool,
    pub food: i32,
    pub saturation: f32,
    /// Progress through the level, 0 to 1, and the level.
    pub xp_progress: f32,
    pub xp_level: i32,
}

/// Quads in GUI pixels. Vertices are `Vertex`es whose x and y are in GUI pixels, whose z is
/// how far toward the viewer they are (the depth test keeps the nearest), and whose colour
/// is the tint.
#[derive(Default)]
pub struct GuiMesh {
    pub vertices: Vec<Vertex>,
    /// Runs of quads drawn only inside a rectangle (x, y, width, height in GUI pixels):
    /// each item stays inside its 16 × 16 slot, as vanilla draws items into a cell of their
    /// own first. Quads outside every run aren't clipped.
    pub clips: Vec<(std::ops::Range<u32>, [f32; 4])>,
}

impl GuiMesh {
    fn quad(&mut self, corners: [[f32; 3]; 4], uv: [[f32; 2]; 4], color: [f32; 4]) {
        let color = color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        for (p, uv) in corners.into_iter().zip(uv) {
            self.vertices.push(Vertex {
                pos: p,
                uv,
                color,
                light: [0; 4],
            });
        }
    }

    /// A sprite (or part of one, `rect` being its atlas rect) as an axis-aligned rectangle
    /// at depth `z`.
    #[allow(clippy::too_many_arguments)]
    fn blit(&mut self, x: f32, y: f32, z: f32, w: f32, h: f32, rect: [f32; 4], color: [f32; 4]) {
        let [u0, v0, u1, v1] = rect;
        self.quad(
            [[x, y, z], [x, y + h, z], [x + w, y + h, z], [x + w, y, z]],
            [[u0, v0], [u0, v1], [u1, v1], [u1, v0]],
            color,
        );
    }

    /// `Font.drawString` with its shadow (one GUI pixel down and right, a quarter as bright),
    /// at depth `z`, `scale` times its size.
    #[allow(clippy::too_many_arguments)]
    fn shadowed(
        &mut self,
        art: &HudArt,
        text: &str,
        x: f32,
        y: f32,
        z: f32,
        scale: f32,
        rgb: [f32; 3],
    ) {
        let [r, g, b] = rgb;
        let shadow = [r * 0.25, g * 0.25, b * 0.25];
        self.glyphs(art, text, x + scale, y + scale, z, scale, shadow);
        self.glyphs(art, text, x, y, z + 0.5, scale, rgb);
    }

    /// `Font.drawString` without a shadow, `scale` times its size.
    #[allow(clippy::too_many_arguments)]
    fn glyphs(
        &mut self,
        art: &HudArt,
        text: &str,
        x: f32,
        y: f32,
        z: f32,
        scale: f32,
        rgb: [f32; 3],
    ) {
        let mut pen = x;
        for c in text.chars() {
            let Some(&(cell, advance)) = art.glyphs.get(&c) else {
                continue;
            };
            if c != ' ' {
                let rect = art.glyph_rect(cell);
                let size = 8.0 * scale;
                self.blit(pen, y, z, size, size, rect, [rgb[0], rgb[1], rgb[2], 1.0]);
            }
            pen += advance as f32 * scale;
        }
    }

    fn text_width(art: &HudArt, text: &str) -> f32 {
        text.chars()
            .filter_map(|c| art.glyphs.get(&c))
            .map(|&(_, advance)| advance as f32)
            .sum()
    }

    /// An item's icon in the 16 × 16 slot whose top left is `x`, `y`, around depth `z`.
    fn item(&mut self, icon: &ItemIcon, x: f32, y: f32, z: f32) {
        match icon {
            ItemIcon::Flat { layers, .. } => {
                for (i, &(rect, [r, g, b])) in layers.iter().enumerate() {
                    self.blit(x, y, z + i as f32 * 0.01, 16.0, 16.0, rect, [r, g, b, 1.0]);
                }
            }
            ItemIcon::Model {
                model,
                tints,
                gui,
                front_lit,
                ..
            } => {
                // `ItemTransform.apply`, then the model centred on the slot, 16 GUI pixels
                // to a block, with y pointing down the screen.
                let r = Quat::from_euler(
                    glam::EulerRot::XYZ,
                    gui.rotation[0].to_radians(),
                    gui.rotation[1].to_radians(),
                    gui.rotation[2].to_radians(),
                );
                let transform = |p: Vec3| {
                    let t = Vec3::from(gui.translation) / 16.0;
                    t + r * (Vec3::from(gui.scale) * (p - Vec3::splat(0.5)))
                };
                let place =
                    |v: Vec3| Vec3::new(x + 8.0 + v.x * 16.0, y + 8.0 - v.y * 16.0, z + v.z * 16.0);
                let lights = item_lights(*front_lit);
                let mut quads: Vec<PlacedQuad> = model
                    .quads
                    .iter()
                    .filter_map(|q| {
                        let moved = q.pos.map(transform);
                        // The facing from the corners' winding, which models built with a
                        // mirroring scale (banners, heads) keep right where `dir` doesn't.
                        let n = (moved[1] - moved[0])
                            .cross(moved[2] - moved[0])
                            .normalize_or_zero();
                        // Faces turned away from the screen aren't drawn.
                        if n.z <= 1e-4 {
                            return None;
                        }
                        let light = (lights[0].dot(n).max(0.0) + lights[1].dot(n).max(0.0))
                            .mul_add(0.6, 0.4)
                            .min(1.0);
                        let tint = usize::try_from(q.tint)
                            .ok()
                            .and_then(|i| tints.get(i))
                            .copied()
                            .unwrap_or([1.0; 3]);
                        let placed = moved.map(place);
                        let depth = placed.iter().map(|p| p.z).sum::<f32>();
                        Some((
                            depth,
                            placed.map(|p| p.to_array()),
                            q.uv,
                            [tint[0] * light, tint[1] * light, tint[2] * light, 1.0],
                        ))
                    })
                    .collect();
                // Far faces first, so translucent ones blend over what's behind them.
                quads.sort_by(|a, b| a.0.total_cmp(&b.0));
                for (_, corners, uv, color) in quads {
                    self.quad(corners, uv, color);
                }
            }
        }
    }
}

impl GuiMesh {
    /// `GuiGraphics.renderItem` and `renderItemDecorations`: an item kept inside its 16 × 16
    /// slot at `x`, `y` around depth `z`, then its count, right-aligned, at `text_z`.
    #[allow(clippy::too_many_arguments)]
    fn slot_item(
        &mut self,
        art: &HudArt,
        name: &str,
        count: i32,
        x: f32,
        y: f32,
        z: f32,
        text_z: f32,
    ) {
        if let Some(icon) = art.items.get(name) {
            let start = (self.vertices.len() / 4) as u32;
            self.item(icon, x, y, z);
            let end = (self.vertices.len() / 4) as u32;
            self.clips.push((start..end, [x, y, 16.0, 16.0]));
        }
        if count != 1 {
            let text = count.to_string();
            let w = GuiMesh::text_width(art, &text);
            let (tx, ty) = (x + 19.0 - 2.0 - w, y + 6.0 + 3.0);
            self.shadowed(art, &text, tx, ty, text_z, 1.0, [1.0; 3]);
        }
    }
}

/// A quad of an item icon: its depth, corners in GUI pixels, UVs and colour.
type PlacedQuad = (f32, [[f32; 3]; 4], [[f32; 2]; 4], [f32; 4]);

/// `Lighting`'s two GUI light directions, turned into the space of an item's rotated
/// normals: the `ITEMS_3D` or `ITEMS_FLAT` rotations of `DIFFUSE_LIGHT_0` and `_1`. The
/// item pose flips y, and so does `ITEMS_3D`'s setup, which cancels out; `ITEMS_FLAT`'s
/// doesn't, so its lights get the flip instead.
fn item_lights(front_lit: bool) -> [Vec3; 2] {
    let rotate = |m: Mat3| {
        [
            m * Vec3::new(0.2, 1.0, -0.7).normalize(),
            m * Vec3::new(-0.2, 1.0, 0.7).normalize(),
        ]
    };
    let base = Mat3::from_rotation_y(-std::f32::consts::FRAC_PI_8)
        * Mat3::from_rotation_x(3.0 * std::f32::consts::FRAC_PI_4);
    if front_lit {
        let flip = Mat3::from_diagonal(Vec3::new(1.0, -1.0, 1.0));
        rotate(base).map(|l| flip * l)
    } else {
        rotate(Mat3::from_rotation_y(1.0821041) * Mat3::from_rotation_x(3.2375858) * base)
    }
}

/// The hotbar along the bottom of a GUI `width` × `height` GUI pixels.
pub fn hotbar(art: &HudArt, hotbar: &Hotbar, width: u32, height: u32) -> GuiMesh {
    let mut mesh = GuiMesh::default();
    let center = (width / 2) as f32;
    let bottom = height as f32;
    let white = [1.0; 4];
    mesh.blit(
        center - 91.0,
        bottom - 22.0,
        BACK,
        182.0,
        22.0,
        art.hotbar,
        white,
    );
    let selected = hotbar.selected.min(8) as f32;
    mesh.blit(
        center - 91.0 - 1.0 + selected * 20.0,
        bottom - 22.0 - 1.0,
        BACK + 1.0,
        24.0,
        23.0,
        art.selection,
        white,
    );
    for (slot, item) in hotbar.slots.iter().enumerate() {
        let Some((name, count)) = item else {
            continue;
        };
        let (x, y) = (
            center - 90.0 + slot as f32 * 20.0 + 2.0,
            bottom - 16.0 - 3.0,
        );
        mesh.slot_item(art, name, *count, x, y, 0.0, TEXT);
    }
    if let Some(vitals) = &hotbar.vitals {
        mesh.vitals(art, vitals, width as i32, height as i32);
    }
    mesh.chat(art, &hotbar.chat, width as i32, height as i32);
    if let Some(death) = &hotbar.death {
        mesh.death_screen(art, death, width as i32, height as i32);
    }
    mesh
}

impl GuiMesh {
    /// `Hud.extractHearts` and `extractFood` (without absorption, armour, air or effects),
    /// then `ExperienceBar.extractBackground` and `ContextualBar.extractExperienceLevel`, for
    /// a GUI `width` × `height` GUI pixels. Each sprite goes a little in front of the one
    /// before, so they overlap as vanilla draws them, in order.
    fn vitals(&mut self, art: &HudArt, v: &Vitals, width: i32, height: i32) {
        let mut z = BACK + 2.0;
        let mut sprite = |mesh: &mut Self, name: &str, x: i32, y: i32, w: i32, h: i32| {
            let rect = art.named[name];
            mesh.blit(x as f32, y as f32, z, w as f32, h as f32, rect, [1.0; 4]);
            z += 0.01;
        };
        // `extractPlayerHealth`.
        let mut random = crate::bake::JavaRandom::new(i64::from(v.tick.wrapping_mul(312_871)));
        let (x_left, x_right, y_base) = (width / 2 - 91, width / 2 + 91, height - 39);
        let max_health = v.max_health.max(v.old_health.max(v.health) as f32);
        let rows = (max_health / 2.0 / 10.0).ceil() as i32;
        let row_height = (10 - (rows - 2)).max(3);

        // `extractHearts`.
        let containers = (max_health / 2.0).ceil() as i32;
        for i in (0..containers).rev() {
            let (x, mut y) = (x_left + i % 10 * 8, y_base - i / 10 * row_height);
            if v.health <= 4 {
                y += random.next_int(2);
            }
            sprite(self, &heart(true, v.hardcore, false, v.blink), x, y, 9, 9);
            let halves = i * 2;
            if v.blink && halves < v.old_health {
                let half = halves + 1 == v.old_health;
                sprite(self, &heart(false, v.hardcore, half, true), x, y, 9, 9);
            }
            if halves < v.health {
                let half = halves + 1 == v.health;
                sprite(self, &heart(false, v.hardcore, half, false), x, y, 9, 9);
            }
        }

        // `extractFood`.
        let [empty, half, full] = FOOD;
        for i in 0..10 {
            let mut y = y_base;
            if v.saturation <= 0.0 && v.tick % (v.food * 3 + 1) == 0 {
                y += random.next_int(3) - 1;
            }
            let x = x_right - i * 8 - 9;
            sprite(self, empty, x, y, 9, 9);
            if i * 2 + 1 < v.food {
                sprite(self, full, x, y, 9, 9);
            }
            if i * 2 + 1 == v.food {
                sprite(self, half, x, y, 9, 9);
            }
        }

        // `ExperienceBar.extractBackground`; `getXpNeededForNextLevel` is never 0.
        let (left, top) = ((width - 182) / 2, height - 24 - 5);
        let progress = (v.xp_progress * 183.0) as i32;
        sprite(self, XP_BACKGROUND, left, top, 182, 5);
        if progress > 0 {
            // The sprite's left `progress` pixels, at their own size.
            let [u0, v0, u1, v1] = art.named[XP_PROGRESS];
            let u1 = u0 + (u1 - u0) * progress.min(182) as f32 / 182.0;
            self.blit(
                left as f32,
                top as f32,
                z,
                progress as f32,
                5.0,
                [u0, v0, u1, v1],
                [1.0; 4],
            );
        }

        // `ContextualBar.extractExperienceLevel`: the level in green, outlined in black.
        if v.xp_level > 0 {
            let text = v.xp_level.to_string();
            let x = (width - Self::text_width(art, &text) as i32) / 2;
            let y = height - 24 - 9 - 2;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (x, y) = ((x + dx) as f32, (y + dy) as f32);
                self.glyphs(art, &text, x, y, TEXT, 1.0, [0.0; 3]);
            }
            // 0xFF80FF20.
            let green = [128.0 / 255.0, 1.0, 32.0 / 255.0];
            self.glyphs(art, &text, x as f32, y as f32, TEXT + 1.0, 1.0, green);
        }
    }
}

impl GuiMesh {
    /// `DeathScreen.extractRenderState`, or `TitleConfirmScreen`'s over the same background.
    fn death_screen(&mut self, art: &HudArt, d: &DeathScreen, width: i32, height: i32) {
        let lang = |key: &str| art.lang.get(key).map_or(key, String::as_str).to_owned();
        let width_of = |text: &str| Self::text_width(art, text) as i32;
        let white = [1.0; 3];
        // `extractDeathBackground`: 0x60500000 at the top to 0xA0803030 at the bottom.
        self.gradient(
            art,
            width as f32,
            height as f32,
            [0x50, 0, 0, 0x60],
            [0x80, 0x30, 0x30, 0xa0],
        );
        let z = SCREEN + 2.0;
        if d.confirm {
            let (x, y) = confirm_origin(width, height);
            let title = lang("deathScreen.quit.confirm");
            // A `StringWidget` centred in the column, as wide as the buttons (the title is
            // narrower).
            let x = x + (304 - width_of(&title)) / 2;
            self.shadowed(art, &title, x as f32, y as f32, z, 1.0, white);
        } else {
            // The title at twice the size, centred on a quarter of the width in its own
            // scaled coordinates, at y 30 in them.
            let key = if d.hardcore {
                "deathScreen.title.hardcore"
            } else {
                "deathScreen.title"
            };
            let title = lang(key);
            let left = width / 2 / 2 - width_of(&title) / 2;
            self.shadowed(art, &title, (left * 2) as f32, 60.0, z, 2.0, white);
            if let Some(message) = &d.message {
                let x = width / 2 - width_of(message) / 2;
                self.shadowed(art, message, x as f32, 85.0, z, 1.0, white);
            }
            // "Score: %s", the score in yellow (`ChatFormatting.YELLOW`).
            let format = lang("deathScreen.score.value");
            let (before, after) = format.split_once("%s").unwrap_or((&format, ""));
            let score = d.score.to_string();
            let parts = [
                (before, white),
                (&score, [1.0, 1.0, 85.0 / 255.0]),
                (after, white),
            ];
            let total: i32 = parts.iter().map(|(t, _)| width_of(t)).sum();
            let mut x = width / 2 - total / 2;
            for (text, rgb) in parts {
                self.shadowed(art, text, x as f32, 100.0, z, 1.0, rgb);
                x += width_of(text);
            }
        }
        for button in death_buttons(d, width, height) {
            self.button(art, &button, d.active, d.mouse);
        }
    }

    /// `Button.extractContents`: the nine-sliced sprite, then the label centred on it, grey
    /// when inactive (`AbstractWidget.WithInactiveMessage`).
    fn button(&mut self, art: &HudArt, b: &Button, active: bool, mouse: Option<[f32; 2]>) {
        let hovered = mouse.is_some_and(|m| b.contains(m));
        let sprite = match (active, hovered) {
            (true, false) => BUTTON[0],
            (false, _) => BUTTON[1],
            (true, true) => BUTTON[2],
        };
        let [x, y, w, h] = b.rect;
        let [u0, v0, u1, v1] = art.named[sprite];
        // `GuiGraphicsExtractor.blitNineSlicedSprite` for the 200 × 20 sprites with a 3-pixel
        // border at the buttons' height of 20: the left border, the middle 194 tiled across
        // (wider buttons, such as the pause screen's 204, repeat its start), then the right
        // border.
        let u = |px: i32| u0 + (u1 - u0) * px as f32 / 200.0;
        let z = SCREEN + 1.0;
        let white = [1.0; 4];
        let mut column = |left: i32, from: i32, width: i32| {
            let rect = [u(from), v0, u(from + width), v1];
            self.blit(
                left as f32,
                y as f32,
                z,
                width as f32,
                h as f32,
                rect,
                white,
            );
        };
        column(x, 0, 3);
        let mut left = x + 3;
        while left < x + w - 3 {
            let width = (x + w - 3 - left).min(194);
            column(left, 3, width);
            left += width;
        }
        column(x + w - 3, 197, 3);
        // `ActiveTextCollector.defaultScrollingHelper` for a label that fits.
        let label = art.lang.get(b.label).map_or(b.label, String::as_str);
        let text_width = Self::text_width(art, label) as i32;
        let left = x + w / 2 - text_width / 2;
        let top = (y + y + h - 9) / 2 + 1;
        // 0xA0A0A0 when inactive.
        let rgb = if active { [1.0; 3] } else { [160.0 / 255.0; 3] };
        self.shadowed(art, label, left as f32, top as f32, SCREEN + 2.0, 1.0, rgb);
    }

    /// `GuiGraphicsExtractor.fill`: a rectangle (x, y, width, height) in one colour.
    fn rect(&mut self, art: &HudArt, [x, y, w, h]: [i32; 4], z: f32, color: [u8; 4]) {
        let (x0, y0, x1, y1) = (x as f32, y as f32, (x + w) as f32, (y + h) as f32);
        for [px, py] in [[x0, y0], [x0, y1], [x1, y1], [x1, y0]] {
            self.vertices.push(Vertex {
                pos: [px, py, z],
                uv: art.white,
                color,
                light: [0; 4],
            });
        }
    }

    /// `GuiGraphicsExtractor.fillGradient` over the whole screen, from `top` to `bottom`
    /// (RGBA bytes).
    fn gradient(&mut self, art: &HudArt, width: f32, height: f32, top: [u8; 4], bottom: [u8; 4]) {
        let corners = [[0.0, 0.0], [0.0, height], [width, height], [width, 0.0]];
        let colors = [top, bottom, bottom, top];
        for ([x, y], color) in corners.into_iter().zip(colors) {
            self.vertices.push(Vertex {
                pos: [x, y, SCREEN],
                uv: art.white,
                color,
                light: [0; 4],
            });
        }
    }
}

/// `SubmitNodeCollection.submitNameTag`: the entity's name half a block over its
/// `NAME_TAG` attachment, facing the camera, a text pixel being 0.025 blocks, centred on a
/// translucent black background (`getBackgroundOpacity(0.25)`) one pixel wider than the
/// text on every side but the right. Shown within 64 blocks (`extractNameTags`), or 32
/// when discrete (`LivingEntityRenderer.shouldShowName`).
///
/// A tag that isn't discrete is drawn twice: the text opaque where nothing hides it, lit
/// by the light at the entity or a light of 2, whichever is brighter
/// (`lightCoordsWithEmission`), then the background with half-transparent text over
/// everything, so unhidden text ends up three quarters bright plus half. A
/// discrete one is drawn once, background and half-transparent text, where nothing hides
/// it.
pub(crate) fn name_tag(
    art: &HudArt,
    tag: &crate::entity::NameTag,
    feet: Vec3,
    [sky, block]: [u8; 2],
    camera: &crate::gpu::Camera,
    out: &mut crate::entity::Meshes,
) {
    let range = if tag.discrete { 32.0 } else { 64.0 };
    if feet.distance_squared(camera.eye) >= range * range {
        return;
    }
    // The camera's orientation: text runs to its right and down its up, whatever its pitch.
    let yaw = camera.yaw.to_radians();
    let right = Vec3::new(-yaw.cos(), 0.0, -yaw.sin());
    let up = right.cross(camera.forward());
    let anchor = feet + Vec3::Y * (tag.height + 0.5);
    let place = |x: f32, y: f32| (anchor + (right * x - up * y) * 0.025).to_array();
    let quad = |out: &mut Vec<Vertex>, [x0, y0, x1, y1]: [f32; 4], uv: [f32; 4], color, light| {
        let [u0, v0, u1, v1] = uv;
        for (x, y, u, v) in [
            (x0, y0, u0, v0),
            (x0, y1, u0, v1),
            (x1, y1, u1, v1),
            (x1, y0, u1, v0),
        ] {
            out.push(Vertex {
                pos: place(x, y),
                uv: [u, v],
                color,
                light,
            });
        }
    };
    let text = |out: &mut Vec<Vertex>, color: [u8; 4], light: [u8; 4]| {
        let mut pen = -GuiMesh::text_width(art, &tag.text) / 2.0;
        for c in tag.text.chars() {
            let Some(&(cell, advance)) = art.glyphs.get(&c) else {
                continue;
            };
            if c != ' ' {
                quad(
                    out,
                    [pen, 0.0, pen + 8.0, 8.0],
                    art.glyph_rect(cell),
                    color,
                    light,
                );
            }
            pen += advance as f32;
        }
    };
    let width = GuiMesh::text_width(art, &tag.text);
    let x = -width / 2.0;
    let [u, v] = art.white;
    let background = |out: &mut Vec<Vertex>| {
        quad(
            out,
            [x - 1.0, -1.0, x + width, 9.0],
            [u, v, u, v],
            [0, 0, 0, 64],
            [0; 4],
        );
    };
    let lit = [sky.min(15) * 16, block.min(15) * 16, 0, 0];
    // `(backgroundAlpha + 0.75) * 0.5`, white.
    let faint = [255, 255, 255, 128];
    if tag.discrete {
        background(&mut out.eyes);
        text(&mut out.eyes, faint, lit);
    } else {
        // The see-through shader doesn't use the lightmap: full light leaves colours as they
        // are, near enough.
        background(&mut out.see_through);
        text(&mut out.see_through, faint, [240, 240, 0, 0]);
        let glowing = [sky.clamp(2, 15) * 16, block.clamp(2, 15) * 16, 0, 0];
        text(&mut out.tags, [255; 4], glowing);
    }
}

/// The crack overlay on a block being mined at `pos` with this state, at `stage` 0 to 9:
/// the block's model's quads, textured by `SheetedDecalTextureGenerator`, which projects the
/// crack texture onto each face from the block's own coordinates.
pub fn cracks(models: &BlockModels, state: u32, pos: [i32; 3], stage: u8) -> Vec<Vertex> {
    let Some(state_model) = models.states.get(state as usize) else {
        return Vec::new();
    };
    let [x, y, z] = pos;
    let mut random = crate::bake::JavaRandom::new(crate::bake::position_seed(x, y, z));
    let [su0, sv0, su1, sv1] = models.hud.cracks[stage.min(9) as usize];
    let origin = Vec3::new(x as f32, y as f32, z as f32);
    let mut out = Vec::new();
    for part in &state_model.parts {
        if part.is_empty() {
            continue;
        }
        let model = crate::bake::pick(part, &mut random);
        for q in &model.quads {
            let uv = q.pos.map(|p| decal_uv(p, q.dir));
            // Each face spans at most one repeat of the texture; move it into the one the
            // atlas holds.
            let shift = |i: usize| {
                let min = uv.iter().map(|c| c[i]).fold(f32::MAX, f32::min);
                (min + 1e-4).floor()
            };
            let (du, dv) = (shift(0), shift(1));
            for (p, [u, v]) in q.pos.iter().zip(uv) {
                out.push(Vertex {
                    pos: (origin + *p).to_array(),
                    uv: [
                        su0 + (su1 - su0) * (u - du).clamp(0.0, 1.0),
                        sv0 + (sv1 - sv0) * (v - dv).clamp(0.0, 1.0),
                    ],
                    color: [255; 4],
                    light: [0; 4],
                });
            }
        }
    }
    out
}

/// `SheetedDecalTextureGenerator`'s texture coordinates for a point of a block (0..1) on a
/// face pointing `dir`: turned half round y, a quarter back round x, then by the face's
/// `Direction.getRotation`, and the negated x and y taken.
fn decal_uv(p: Vec3, dir: Dir) -> [f32; 2] {
    use std::f32::consts::{FRAC_PI_2, PI};
    let xyz = |x: f32, y: f32, z: f32| Quat::from_euler(glam::EulerRot::XYZ, x, y, z);
    let face = match dir {
        Dir::Down => Quat::from_rotation_x(PI),
        Dir::Up => Quat::IDENTITY,
        Dir::North => xyz(FRAC_PI_2, 0.0, PI),
        Dir::South => Quat::from_rotation_x(FRAC_PI_2),
        Dir::West => xyz(FRAC_PI_2, 0.0, FRAC_PI_2),
        Dir::East => xyz(FRAC_PI_2, 0.0, -FRAC_PI_2),
    };
    let v = face * (Quat::from_rotation_x(-FRAC_PI_2) * (Quat::from_rotation_y(PI) * p));
    [-v.x, -v.y]
}

#[cfg(test)]
mod tests {
    use super::GuiMesh;

    /// A word wider than a line, such as a URL, breaks inside itself so every line fits,
    /// and the words around it still break at spaces.
    #[test]
    fn wrap_breaks_words_wider_than_a_line() {
        let res = mc_assets::Resources::new(mc_assets::default_root());
        let art = crate::bake::BlockModels::gui(&res).unwrap().hud;
        let url = "https://api.github.com/repos/TheAaronLawrence/minecraft-cardboard/releases";
        let text = format!("GitHub answered 404 to {url} today");
        let max = 120;
        let lines = art.wrap(&text, max);
        assert!(lines.len() > 2, "{lines:?}");
        for line in &lines {
            assert!(GuiMesh::text_width(&art, line) as i32 <= max, "{line:?}");
        }
        assert_eq!(lines.first().unwrap(), "GitHub answered 404 to");
        assert_eq!(lines.concat().replace(' ', ""), text.replace(' ', ""));
        // Words that fit still go whole onto the next line.
        assert_eq!(
            art.wrap("aaa bbb", GuiMesh::text_width(&art, "aaa ") as i32 + 2),
            ["aaa", "bbb"]
        );
    }
}
