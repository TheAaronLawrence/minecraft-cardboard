//! The survival inventory and the screens of chests, crafting tables and furnaces, as
//! vanilla's `InventoryScreen`, `ContainerScreen`, `CraftingScreen` and `FurnaceScreen` draw
//! them over the HUD with `AbstractContainerScreen`: the dimmed world, the screen's texture,
//! its labels, each slot's item (or the empty armour and offhand icons), a furnace's flame
//! and arrow, the highlight on the slot under the mouse, the item on the cursor, and the
//! hovered item's name in a tooltip. The player model isn't drawn yet.

use super::{GuiMesh, HudArt, SCREEN};

const TEXTURE: &str = "minecraft:gui/container/inventory";
const CHEST: &str = "minecraft:gui/container/generic_54";
const CRAFTING: &str = "minecraft:gui/container/crafting_table";
const FURNACE: &str = "minecraft:gui/container/furnace";

/// The textures drawn whole, by their full ids.
pub(crate) const TEXTURES: [&str; 4] = [TEXTURE, CHEST, CRAFTING, FURNACE];

/// `FurnaceScreen`'s flame (14 × 14) and arrow (24 × 16).
const LIT_PROGRESS: &str = "container/furnace/lit_progress";
const BURN_PROGRESS: &str = "container/furnace/burn_progress";

const HIGHLIGHT_BACK: &str = "container/slot_highlight_back";
const HIGHLIGHT_FRONT: &str = "container/slot_highlight_front";
const TOOLTIP_BACKGROUND: &str = "tooltip/background";
const TOOLTIP_FRAME: &str = "tooltip/frame";
/// `InventoryMenu`'s empty-slot icons: armour from the head down, then the offhand.
const EMPTY_ICONS: [&str; 5] = [
    "container/slot/helmet",
    "container/slot/chestplate",
    "container/slot/leggings",
    "container/slot/boots",
    "container/slot/shield",
];

/// The GUI sprites this screen draws by name.
/// `EffectsInInventory.EFFECT_BACKGROUND_SPRITE` and `EFFECT_BACKGROUND_AMBIENT_SPRITE`:
/// 32 × 32, sliced with a 4-pixel border.
const EFFECT_BACKGROUND: [&str; 2] = [
    "container/inventory/effect_background",
    "container/inventory/effect_background_ambient",
];

pub(crate) const SPRITES: [&str; 13] = [
    EFFECT_BACKGROUND[0],
    EFFECT_BACKGROUND[1],
    LIT_PROGRESS,
    BURN_PROGRESS,
    HIGHLIGHT_BACK,
    HIGHLIGHT_FRONT,
    TOOLTIP_BACKGROUND,
    TOOLTIP_FRAME,
    EMPTY_ICONS[0],
    EMPTY_ICONS[1],
    EMPTY_ICONS[2],
    EMPTY_ICONS[3],
    EMPTY_ICONS[4],
];

/// `AbstractContainerScreen.DEFAULT_IMAGE_WIDTH` and `_HEIGHT`.
const IMAGE: (i32, i32) = (176, 166);

/// Which menu a screen shows (`MenuType`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Menu {
    /// `InventoryMenu`, the player's own.
    #[default]
    Player,
    /// `ChestMenu` with this many rows: chests, barrels, ender chests.
    Chest(u8),
    /// `CraftingMenu`: 0 the result, 1 to 9 the grid.
    Crafting,
    /// `FurnaceMenu`: 0 the ingredient, 1 the fuel, 2 the result.
    Furnace,
}

impl Menu {
    /// The menu's own slots, before the player's inventory and hotbar (36 slots), which
    /// every menu but the player's has after them.
    pub fn own(self) -> usize {
        match self {
            Menu::Player => 9,
            Menu::Chest(rows) => rows as usize * 9,
            Menu::Crafting => 10,
            Menu::Furnace => 3,
        }
    }

    /// How many slots the menu has.
    pub fn slot_count(self) -> usize {
        match self {
            Menu::Player => 46,
            _ => self.own() + 36,
        }
    }

    /// The screen's size (`imageWidth`, `imageHeight`): a chest's is 114 plus 18 a row.
    pub fn image(self) -> (i32, i32) {
        match self {
            Menu::Chest(rows) => (176, 114 + rows as i32 * 18),
            _ => IMAGE,
        }
    }

    /// Where the menu puts slot `i`, from the screen's top left (`Slot.x`, `Slot.y`).
    pub fn slot_position(self, i: usize) -> (i32, i32) {
        let own = self.own();
        let (inventory_top, own_slot) = match self {
            Menu::Player => return slot_position(i),
            // `ChestMenu`: rows of nine from (8, 18); the inventory 13 below them.
            Menu::Chest(rows) => (
                18 + rows as i32 * 18 + 13,
                (8 + i as i32 % 9 * 18, 18 + i as i32 / 9 * 18),
            ),
            // `CraftingMenu`: the result at (124, 35), the grid from (30, 17).
            Menu::Crafting => {
                let g = i as i32 - 1;
                let slot = match i {
                    0 => (124, 35),
                    _ => (30 + g % 3 * 18, 17 + g / 3 * 18),
                };
                (84, slot)
            }
            // `AbstractFurnaceMenu`.
            Menu::Furnace => (84, [(56, 17), (56, 53), (116, 35)][i.min(2)]),
        };
        if i < own {
            return own_slot;
        }
        // `addStandardInventorySlots`: three rows of nine, then the hotbar 58 below.
        let j = (i - own) as i32;
        match j {
            0..27 => (8 + j % 9 * 18, inventory_top + j / 9 * 18),
            _ => (8 + (j - 27) * 18, inventory_top + 58),
        }
    }
}

/// Depths: the dimming, the texture, the slot highlight behind the items, the items and
/// their counts, the highlight in front, the carried item, then the tooltip.
const DIM: f32 = SCREEN - 160.0;
const ITEMS: f32 = SCREEN - 100.0;
const COUNTS: f32 = SCREEN - 60.0;
const FRONT: f32 = SCREEN - 50.0;
const CARRIED: f32 = SCREEN;
const TOOLTIP: f32 = SCREEN + 60.0;

pub type Stack = Option<(String, i32)>;

/// What the inventory screen shows.
#[derive(Clone, PartialEq, Debug)]
pub struct Inventory {
    pub menu: Menu,
    /// The screen's title, from the server; the player's inventory has none.
    pub title: String,
    /// The menu's slots, as they show (with a drag's spread already in them).
    pub slots: Vec<Stack>,
    /// The item on the cursor.
    pub carried: Stack,
    /// Slots a drag is spreading the carried items over, drawn with a white overlay.
    pub dragged: Vec<u8>,
    /// Where the mouse is, in GUI pixels.
    pub mouse: Option<[f32; 2]>,
    /// A furnace's fuel left (0 to 1) while it burns, and how far its cooking has got.
    pub lit: Option<f32>,
    pub burn: f32,
    /// The player's effects, which the player's own inventory lists beside it.
    pub effects: Vec<super::ActiveEffect>,
}

/// `EffectsInInventory.canSeeEffects`: whether the player's inventory, in a GUI `width`
/// wide, has room beside it to list the effects, so the HUD leaves them out
/// (`InventoryScreen.showsActiveEffects`).
pub fn shows_effects(width: i32) -> bool {
    let (left, _) = origin(Menu::Player, width, 0);
    width - (left + IMAGE.0 + 2) >= 32
}

/// The screen's top left (`AbstractContainerScreen.init`'s `leftPos` and `topPos`).
pub fn origin(menu: Menu, width: i32, height: i32) -> (i32, i32) {
    let (w, h) = menu.image();
    ((width - w) / 2, (height - h) / 2)
}

/// Where `InventoryMenu` puts slot `i`, from the screen's top left: the crafting result,
/// the 2 × 2 grid, the armour down the left, three rows of inventory, the hotbar below
/// them, and the offhand beside the player.
pub fn slot_position(i: usize) -> (i32, i32) {
    let i = i as i32;
    match i {
        0 => (154, 28),
        1..5 => (98 + (i - 1) % 2 * 18, 18 + (i - 1) / 2 * 18),
        5..9 => (8, 8 + (i - 5) * 18),
        9..36 => (8 + (i - 9) % 9 * 18, 84 + (i - 9) / 9 * 18),
        36..45 => (8 + (i - 36) * 18, 142),
        _ => (77, 62),
    }
}

/// `AbstractContainerScreen.getHoveredSlot`: the slot under `mouse`, a pixel of margin
/// around its 16 × 16 (`isHovering`).
pub fn slot_at(menu: Menu, width: i32, height: i32, [mx, my]: [f32; 2]) -> Option<usize> {
    let (left, top) = origin(menu, width, height);
    let (mx, my) = (mx.floor() as i32 - left, my.floor() as i32 - top);
    (0..menu.slot_count()).find(|&i| {
        let (x, y) = menu.slot_position(i);
        mx >= x - 1 && mx < x + 17 && my >= y - 1 && my < y + 17
    })
}

/// `hasClickedOutside`: the mouse is off the screen's texture.
pub fn outside(menu: Menu, width: i32, height: i32, [mx, my]: [f32; 2]) -> bool {
    let (left, top) = origin(menu, width, height);
    let (w, h) = menu.image();
    let (mx, my) = (mx.floor() as i32, my.floor() as i32);
    mx < left || my < top || mx >= left + w || my >= top + h
}

/// `Rarity`'s colours for an item's name.
fn rarity_rgb(rarity: &str) -> [f32; 3] {
    let rgb = match rarity {
        "uncommon" => 0xFFFF55,
        "rare" => 0x55FFFF,
        "epic" => 0xFF55FF,
        _ => 0xFFFFFF,
    };
    [16, 8, 0].map(|s| ((rgb >> s) & 0xFF) as f32 / 255.0)
}

impl GuiMesh {
    /// `InventoryScreen.extractBackground` and `AbstractContainerScreen.extractRenderState`
    /// in a GUI `width` × `height`.
    pub(crate) fn inventory(&mut self, art: &HudArt, inv: &Inventory, width: i32, height: i32) {
        // `Screen.extractTransparentBackground`: 0xC0101010 at the top to 0xD0101010.
        let corners = [[0, 0], [0, height], [width, height], [width, 0]];
        let colors = [0xC0, 0xD0, 0xD0, 0xC0].map(|a| [0x10, 0x10, 0x10, a]);
        for ([x, y], color) in corners.into_iter().zip(colors) {
            self.vertices.push(crate::mesh::Vertex {
                pos: [x as f32, y as f32, DIM],
                uv: art.white,
                color,
                light: [0; 4],
            });
        }
        let menu = inv.menu;
        let (left, top) = origin(menu, width, height);
        let (image_w, image_h) = menu.image();
        // Part of a 256 × 256 texture: `[u, v, w, h]` in its pixels, drawn at `[x, y]` from
        // the screen's top left.
        let part = |mesh: &mut Self, texture: &str, [u, v, w, h]: [i32; 4], [x, y]: [i32; 2]| {
            let [u0, v0, u1, v1] = art.named[texture];
            let at = |a: f32, b: f32, px: i32| a + (b - a) * px as f32 / 256.0;
            let uv = [
                at(u0, u1, u),
                at(v0, v1, v),
                at(u0, u1, u + w),
                at(v0, v1, v + h),
            ];
            let (x, y) = ((left + x) as f32, (top + y) as f32);
            mesh.blit(x, y, DIM + 1.0, w as f32, h as f32, uv, [1.0; 4]);
        };
        let whole = [0, 0, image_w, image_h];
        match menu {
            Menu::Player => part(self, TEXTURE, whole, [0, 0]),
            // `ContainerScreen.extractBackground`: the rows, then the inventory below.
            Menu::Chest(rows) => {
                let rows_h = rows as i32 * 18 + 17;
                part(self, CHEST, [0, 0, 176, rows_h], [0, 0]);
                part(self, CHEST, [0, 126, 176, 96], [0, rows_h]);
            }
            Menu::Crafting => part(self, CRAFTING, whole, [0, 0]),
            // `AbstractFurnaceScreen.extractBackground`: the flame shrinks from the top as
            // the fuel burns, and the arrow grows as the item cooks.
            Menu::Furnace => {
                part(self, FURNACE, whole, [0, 0]);
                let sprite = |name: &str, [u, v, w, h]: [i32; 4], sw: i32, sh: i32| {
                    let [u0, v0, u1, v1] = art.named[name];
                    let at = |a: f32, b: f32, px: i32, of: i32| a + (b - a) * px as f32 / of as f32;
                    [
                        at(u0, u1, u, sw),
                        at(v0, v1, v, sh),
                        at(u0, u1, u + w, sw),
                        at(v0, v1, v + h, sh),
                    ]
                };
                if let Some(lit) = inv.lit {
                    let h = (lit * 13.0).ceil() as i32 + 1;
                    let uv = sprite(LIT_PROGRESS, [0, 14 - h, 14, h], 14, 14);
                    let (x, y) = ((left + 56) as f32, (top + 36 + 14 - h) as f32);
                    self.blit(x, y, DIM + 2.0, 14.0, h as f32, uv, [1.0; 4]);
                }
                let w = (inv.burn * 24.0).ceil() as i32;
                if w > 0 {
                    let uv = sprite(BURN_PROGRESS, [0, 0, w, 16], 24, 16);
                    let (x, y) = ((left + 79) as f32, (top + 34) as f32);
                    self.blit(x, y, DIM + 2.0, w as f32, 16.0, uv, [1.0; 4]);
                }
            }
        }
        // `extractLabels`: the title, then (but in the player's inventory) "Inventory"
        // above the player's slots, 0x404040 and unshadowed.
        let lang = |key: &str| art.lang.get(key).map_or(key, String::as_str).to_owned();
        let grey = [64.0 / 255.0; 3];
        let label = |mesh: &mut Self, text: &str, x: i32, y: i32| {
            let (x, y) = ((left + x) as f32, (top + y) as f32);
            mesh.glyphs(art, text, x, y, DIM + 2.0, 1.0, grey);
        };
        match menu {
            Menu::Player => label(self, &lang("container.crafting"), 97, 6),
            _ => {
                let title_x = match menu {
                    Menu::Crafting => 29,
                    Menu::Furnace => (image_w - Self::text_width(art, &inv.title) as i32) / 2,
                    _ => 8,
                };
                label(self, &inv.title, title_x, 6);
                label(self, &lang("container.inventory"), 8, image_h - 94);
            }
        }

        let hovered = inv.mouse.and_then(|m| slot_at(menu, width, height, m));
        let highlight = |mesh: &mut Self, sprite: &str, z: f32| {
            if let Some(i) = hovered {
                let (x, y) = menu.slot_position(i);
                let (x, y) = ((left + x - 4) as f32, (top + y - 4) as f32);
                mesh.blit(x, y, z, 24.0, 24.0, art.named[sprite], [1.0; 4]);
            }
        };
        highlight(self, HIGHLIGHT_BACK, DIM + 3.0);
        for (i, stack) in inv.slots.iter().enumerate() {
            let (x, y) = menu.slot_position(i);
            let (x, y) = ((left + x) as f32, (top + y) as f32);
            match stack {
                Some((name, count)) => {
                    if inv.dragged.contains(&(i as u8)) {
                        // `extractSlot`: 0x80FFFFFF over a slot the drag fills.
                        let white = [255, 255, 255, 128];
                        self.rect(art, [x as i32, y as i32, 16, 16], ITEMS - 20.0, white);
                    }
                    self.slot_item(art, name, *count, x, y, ITEMS, COUNTS);
                }
                None => {
                    let icon = match i {
                        _ if menu != Menu::Player => None,
                        5..9 => Some(EMPTY_ICONS[i - 5]),
                        45 => Some(EMPTY_ICONS[4]),
                        _ => None,
                    };
                    if let Some(icon) = icon {
                        self.blit(x, y, DIM + 4.0, 16.0, 16.0, art.named[icon], [1.0; 4]);
                    }
                }
            }
        }
        highlight(self, HIGHLIGHT_FRONT, FRONT);
        if let (Some((name, count)), Some([mx, my])) = (&inv.carried, inv.mouse) {
            let (x, y) = (mx.floor() - 8.0, my.floor() - 8.0);
            self.slot_item(art, name, *count, x, y, CARRIED, CARRIED + 20.0);
        }
        // `extractTooltip`: the hovered item's name, with nothing on the cursor.
        let mut tooltip = None;
        if let (Some(i), None) = (hovered, &inv.carried)
            && let Some((name, _)) = &inv.slots[i]
            && let Some((key, rarity)) = art.item_names.get(name)
        {
            let text = art.lang.get(key).map_or(key.as_str(), String::as_str);
            tooltip = Some(vec![(text.to_owned(), rarity_rgb(rarity))]);
        }
        if menu == Menu::Player
            && let Some(lines) = self.effect_list(art, inv, width, height)
        {
            tooltip = Some(lines);
        }
        if let (Some(lines), Some([mx, my])) = (tooltip, inv.mouse) {
            self.tooltip(art, &lines, mx as i32, my as i32, width, height);
        }
    }

    /// `EffectsInInventory.extractRenderState`: the player's effects in a column right of
    /// the inventory, if there's room, each on a background as wide as its name and time
    /// need (narrow, just the icon, with too little room, or the name cut short), soonest
    /// to end at the top. Returns the tooltip of the one under the mouse whose name doesn't
    /// fit.
    fn effect_list(
        &mut self,
        art: &HudArt,
        inv: &Inventory,
        width: i32,
        height: i32,
    ) -> Option<Vec<(String, [f32; 3])>> {
        let (left, top) = origin(Menu::Player, width, height);
        let x0 = left + IMAGE.0 + 2;
        let available = width - x0;
        if inv.effects.is_empty() || available < 32 {
            return None;
        }
        let max_width = if available >= 120 { available - 7 } else { 32 };
        let n = inv.effects.len() as i32;
        let step = if n > 5 { 132 / (n - 1) } else { 33 };
        let mut sorted: Vec<&super::ActiveEffect> = inv.effects.iter().collect();
        sorted.sort_by(|a, b| a.order(b));
        let lang = |key: &str| art.lang.get(key).map_or(key, String::as_str).to_owned();
        let width_of = |text: &str| Self::text_width(art, text) as i32;
        let mut tooltip = None;
        for (i, e) in sorted.into_iter().enumerate() {
            let y0 = top + i as i32 * step;
            // Each a step in front of the one above, which its background covers.
            let z = DIM + 1.0 + i as f32;
            // `getEffectName`: levels II to X are spelled out.
            let mut name = lang(&format!("effect.{}", e.effect.replacen(':', ".", 1)));
            if (1..=9).contains(&e.amplifier) {
                name = format!(
                    "{name} {}",
                    lang(&format!("enchantment.level.{}", e.amplifier + 1))
                );
            }
            // `MobEffectUtil.formatDuration` and `StringUtil.formatTickDuration`.
            let duration = if e.duration == -1 {
                lang("effect.duration.infinite")
            } else {
                let s = e.duration / 20;
                let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
                if h > 0 {
                    format!("{h:02}:{m:02}:{s:02}")
                } else {
                    format!("{m:02}:{s:02}")
                }
            };
            let w = (32 + width_of(&name).max(width_of(&duration)) + 7).min(max_width);
            let background = EFFECT_BACKGROUND[usize::from(e.ambient)];
            self.nine_slice_tiled(art, background, [32, 32], 4, [x0, y0, w, 32], z);
            let (text_x, text_y) = ((x0 + 32) as f32, (y0 + 7) as f32);
            let room = w - 32 - 7;
            let compact = if room > 0 {
                let clip = width_of(&name) > room;
                let shown = if clip {
                    // `ComponentRenderUtils.clipText`: as much as fits with "..." after.
                    let limit = room - width_of("...");
                    let mut cut = String::new();
                    for c in name.chars() {
                        cut.push(c);
                        if width_of(&cut) > limit {
                            cut.pop();
                            break;
                        }
                    }
                    cut + "..."
                } else {
                    name.clone()
                };
                self.shadowed(art, &shown, text_x, text_y, z + 0.1, 1.0, [1.0; 3]);
                let grey = [128.0 / 255.0; 3];
                self.shadowed(art, &duration, text_x, text_y + 9.0, z + 0.1, 1.0, grey);
                clip
            } else {
                true
            };
            if compact
                && let Some([mx, my]) = inv.mouse
                && (x0..=x0 + w).contains(&(mx as i32))
                && (y0..=y0 + step).contains(&(my as i32))
            {
                tooltip = Some(vec![(name, [1.0; 3]), (duration, [1.0; 3])]);
            }
            if let Some(&rect) = art.named.get(&e.sprite()) {
                let (x, y) = ((x0 + 7) as f32, (y0 + 7) as f32);
                self.blit(x, y, z + 0.8, 18.0, 18.0, rect, [1.0; 4]);
            }
        }
        tooltip
    }

    /// `GuiGraphicsExtractor.tooltip` for lines of text, each in its colour, placed by
    /// `DefaultTooltipPositioner` and framed by `TooltipRenderUtil`.
    fn tooltip(
        &mut self,
        art: &HudArt,
        lines: &[(String, [f32; 3])],
        mx: i32,
        my: i32,
        width: i32,
        height: i32,
    ) {
        // Each line is 10 high, less the 2 below the last.
        let w = lines
            .iter()
            .map(|(text, _)| Self::text_width(art, text) as i32)
            .max()
            .unwrap_or(0);
        let h = lines.len() as i32 * 10 - 2;
        let (mut x, mut y) = (mx + 12, my - 12);
        if x + w > width {
            x = (x - 24 - w).max(4);
        }
        if y + h + 3 > height {
            y = height - h - 3;
        }
        let rect = [x - 12, y - 12, w + 24, h + 24];
        self.nine_slice(art.named[TOOLTIP_BACKGROUND], rect, 9, TOOLTIP);
        self.nine_slice(art.named[TOOLTIP_FRAME], rect, 10, TOOLTIP + 1.0);
        for (i, (text, rgb)) in lines.iter().enumerate() {
            let y = (y + i as i32 * 10) as f32;
            self.shadowed(art, text, x as f32, y, TOOLTIP + 2.0, 1.0, *rgb);
        }
    }

    /// `blitNineSlicedSprite` for a 100 × 100 sprite with a `border`: corners as they are,
    /// edges and middle stretched (the tooltip sprites' edges are even, so stretching draws
    /// what tiling would).
    fn nine_slice(&mut self, sprite: [f32; 4], [x, y, w, h]: [i32; 4], border: i32, z: f32) {
        let [u0, v0, u1, v1] = sprite;
        let u = |px: i32| u0 + (u1 - u0) * px as f32 / 100.0;
        let v = |px: i32| v0 + (v1 - v0) * px as f32 / 100.0;
        let b = border;
        // Each axis: (screen start, length, texture start, texture end).
        let cols = [
            (x, b, 0, b),
            (x + b, w - 2 * b, b, 100 - b),
            (x + w - b, b, 100 - b, 100),
        ];
        let rows = [
            (y, b, 0, b),
            (y + b, h - 2 * b, b, 100 - b),
            (y + h - b, b, 100 - b, 100),
        ];
        for (sx, sw, ta, tb) in cols {
            for (sy, sh, va, vb) in rows {
                if sw > 0 && sh > 0 {
                    let rect = [u(ta), v(va), u(tb), v(vb)];
                    self.blit(
                        sx as f32, sy as f32, z, sw as f32, sh as f32, rect, [1.0; 4],
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slots sit where `InventoryMenu` puts them, and the mouse finds them with a pixel of
    /// margin; between slots and off the texture there's none.
    #[test]
    fn finds_slots_like_vanilla() {
        // At 480 × 270 the screen's top left is (152, 52).
        assert_eq!(origin(Menu::Player, 480, 270), (152, 52));
        let at = |x: f32, y: f32| slot_at(Menu::Player, 480, 270, [x, y]);
        assert_eq!(at(152.0 + 154.0, 52.0 + 28.0), Some(0));
        assert_eq!(at(152.0 + 116.0, 52.0 + 36.0), Some(4));
        assert_eq!(at(152.0 + 7.0, 52.0 + 61.0), Some(8));
        assert_eq!(
            at(152.0 + 8.0 + 18.0 * 8.0 + 16.0, 52.0 + 84.0 + 36.0),
            Some(35)
        );
        assert_eq!(at(152.0 + 8.0, 52.0 + 142.0), Some(36));
        assert_eq!(at(152.0 + 77.0, 52.0 + 62.0), Some(45));
        // Between the inventory's last row and the hotbar.
        assert_eq!(at(152.0 + 8.0, 52.0 + 139.0), None);
        assert!(outside(Menu::Player, 480, 270, [151.0, 100.0]));
        assert!(!outside(Menu::Player, 480, 270, [152.0, 52.0]));
    }

    /// A container's player slots sit below its own: a three-row chest's inventory starts
    /// 13 under its rows (y 85) and its hotbar 58 lower; the crafting grid's last slot and
    /// a furnace's fuel are where their menus put them.
    #[test]
    fn places_container_slots_like_vanilla() {
        let chest = Menu::Chest(3);
        assert_eq!(chest.image(), (176, 168));
        assert_eq!(chest.slot_position(26), (8 + 8 * 18, 18 + 2 * 18));
        assert_eq!(chest.slot_position(27), (8, 85));
        assert_eq!(chest.slot_position(62), (8 + 8 * 18, 143));
        assert_eq!(Menu::Chest(6).slot_position(54), (8, 139));
        assert_eq!(Menu::Crafting.slot_position(9), (66, 53));
        assert_eq!(Menu::Crafting.slot_position(10), (8, 84));
        assert_eq!(Menu::Furnace.slot_position(1), (56, 53));
        assert_eq!(Menu::Furnace.slot_position(38), (8 + 8 * 18, 142));
        assert_eq!(Menu::Furnace.slot_count(), 39);
    }
}
