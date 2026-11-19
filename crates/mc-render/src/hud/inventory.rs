//! The survival inventory, as vanilla's `InventoryScreen` and `AbstractContainerScreen` draw
//! it over the HUD: the dimmed world, the inventory texture, "Crafting", each slot's item (or
//! the empty armour and offhand icons), the highlight on the slot under the mouse, the item
//! on the cursor, and the hovered item's name in a tooltip. The player model isn't drawn yet.

use super::{GuiMesh, HudArt, SCREEN};

const TEXTURE: &str = "minecraft:gui/container/inventory";

/// The texture drawn whole, by its full id.
pub(crate) const TEXTURES: [&str; 1] = [TEXTURE];

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
pub(crate) const SPRITES: [&str; 9] = [
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
    /// `InventoryMenu`'s 46 slots, as they show (with a drag's spread already in them).
    pub slots: Vec<Stack>,
    /// The item on the cursor.
    pub carried: Stack,
    /// Slots a drag is spreading the carried items over, drawn with a white overlay.
    pub dragged: Vec<u8>,
    /// Where the mouse is, in GUI pixels.
    pub mouse: Option<[f32; 2]>,
}

/// The screen's top left (`AbstractContainerScreen.init`'s `leftPos` and `topPos`).
pub fn origin(width: i32, height: i32) -> (i32, i32) {
    ((width - IMAGE.0) / 2, (height - IMAGE.1) / 2)
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
pub fn slot_at(width: i32, height: i32, [mx, my]: [f32; 2]) -> Option<usize> {
    let (left, top) = origin(width, height);
    let (mx, my) = (mx.floor() as i32 - left, my.floor() as i32 - top);
    (0..46).find(|&i| {
        let (x, y) = slot_position(i);
        mx >= x - 1 && mx < x + 17 && my >= y - 1 && my < y + 17
    })
}

/// `hasClickedOutside`: the mouse is off the screen's texture.
pub fn outside(width: i32, height: i32, [mx, my]: [f32; 2]) -> bool {
    let (left, top) = origin(width, height);
    let (mx, my) = (mx.floor() as i32, my.floor() as i32);
    mx < left || my < top || mx >= left + IMAGE.0 || my >= top + IMAGE.1
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
        let (left, top) = origin(width, height);
        let [u0, v0, u1, v1] = art.named[TEXTURE];
        let part = [
            u0,
            v0,
            u0 + (u1 - u0) * IMAGE.0 as f32 / 256.0,
            v0 + (v1 - v0) * IMAGE.1 as f32 / 256.0,
        ];
        let (w, h) = (IMAGE.0 as f32, IMAGE.1 as f32);
        self.blit(left as f32, top as f32, DIM + 1.0, w, h, part, [1.0; 4]);
        // `extractLabels`: the title, 0x404040 and unshadowed.
        let title = art
            .lang
            .get("container.crafting")
            .map_or("container.crafting", String::as_str);
        let grey = [64.0 / 255.0; 3];
        let (tx, ty) = ((left + 97) as f32, (top + 6) as f32);
        self.glyphs(art, title, tx, ty, DIM + 2.0, 1.0, grey);

        let hovered = inv.mouse.and_then(|m| slot_at(width, height, m));
        let highlight = |mesh: &mut Self, sprite: &str, z: f32| {
            if let Some(i) = hovered {
                let (x, y) = slot_position(i);
                let (x, y) = ((left + x - 4) as f32, (top + y - 4) as f32);
                mesh.blit(x, y, z, 24.0, 24.0, art.named[sprite], [1.0; 4]);
            }
        };
        highlight(self, HIGHLIGHT_BACK, DIM + 3.0);
        for (i, stack) in inv.slots.iter().enumerate() {
            let (x, y) = slot_position(i);
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
        if let (Some(i), None, Some([mx, my])) = (hovered, &inv.carried, inv.mouse)
            && let Some((name, _)) = &inv.slots[i]
            && let Some((key, rarity)) = art.item_names.get(name)
        {
            let text = art.lang.get(key).map_or(key.as_str(), String::as_str);
            self.tooltip(
                art,
                text,
                rarity_rgb(rarity),
                mx as i32,
                my as i32,
                width,
                height,
            );
        }
    }

    /// `GuiGraphicsExtractor.tooltip` for one line of text, placed by
    /// `DefaultTooltipPositioner` and framed by `TooltipRenderUtil`.
    #[allow(clippy::too_many_arguments)]
    fn tooltip(
        &mut self,
        art: &HudArt,
        text: &str,
        rgb: [f32; 3],
        mx: i32,
        my: i32,
        width: i32,
        height: i32,
    ) {
        // One line is 10 high, less the 2 below the last.
        let (w, h) = (Self::text_width(art, text) as i32, 8);
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
        self.shadowed(art, text, x as f32, y as f32, TOOLTIP + 2.0, 1.0, rgb);
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
        assert_eq!(origin(480, 270), (152, 52));
        let at = |x: f32, y: f32| slot_at(480, 270, [x, y]);
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
        assert!(outside(480, 270, [151.0, 100.0]));
        assert!(!outside(480, 270, [152.0, 52.0]));
    }
}
