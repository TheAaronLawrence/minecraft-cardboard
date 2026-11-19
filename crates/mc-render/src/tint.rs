//! Biome colours for grass, leaves, leaf litter and water.

use anyhow::{Context, Result};
use mc_assets::Resources;
use std::collections::HashMap;

/// Which colour a block's tinted faces take. Vanilla assigns these in code
/// (`BlockColors.createDefault`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tint {
    Grass,
    Foliage,
    DryFoliage,
    Water,
    Fixed([f32; 3]),
}

pub fn block_tint(name: &str, properties: &[(&str, &str)]) -> Option<Tint> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let int = |p: &str| {
        properties
            .iter()
            .find(|(n, _)| *n == p)
            .and_then(|(_, v)| v.parse::<u32>().ok())
    };
    Some(match name {
        "grass_block" | "short_grass" | "tall_grass" | "fern" | "large_fern" | "potted_fern"
        | "sugar_cane" | "bush" | "pink_petals" | "wildflowers" => Tint::Grass,
        "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves"
        | "mangrove_leaves" | "vine" => Tint::Foliage,
        "leaf_litter" => Tint::DryFoliage,
        "attached_melon_stem" | "attached_pumpkin_stem" => Tint::Fixed(rgb(0xE0C71C)),
        // `BlockTintSources.stem`: green turning yellow as it grows.
        "melon_stem" | "pumpkin_stem" => {
            let age = int("age")?;
            Tint::Fixed(rgb((age * 32) << 16 | (255 - age * 8) << 8 | (age * 4)))
        }
        "redstone_wire" => Tint::Fixed(redstone(int("power")?)),
        "birch_leaves" => Tint::Fixed(rgb(0x80A755)),
        "spruce_leaves" => Tint::Fixed(rgb(0x619961)),
        "lily_pad" => Tint::Fixed(rgb(0x208030)),
        "water" | "bubble_column" | "water_cauldron" => Tint::Water,
        _ => return crate::entity_model::banner_color(name).map(Tint::Fixed),
    })
}

/// `RedstoneWireBlock.COLORS`: dark red when unpowered, brightening to orange-red at 15.
fn redstone(power: u32) -> [f32; 3] {
    let f = power as f32 / 15.0;
    let r = f * 0.6 + if f > 0.0 { 0.4 } else { 0.3 };
    let g = (f * f * 0.7 - 0.5).clamp(0.0, 1.0);
    let b = (f * f * 0.6 - 0.7).clamp(0.0, 1.0);
    // `ARGB.colorFromFloat` truncates each channel to a byte.
    [r, g, b].map(|c| (c * 255.0).floor() / 255.0)
}

/// A colour map, such as `grass`, from `textures/colormap`.
pub(crate) fn colormap(res: &Resources, name: &str) -> Result<image::RgbaImage> {
    Ok(
        image::open(res.asset_path("textures", &format!("colormap/{name}"), "png"))
            .with_context(|| format!("colormap {name}"))?
            .to_rgba8(),
    )
}

/// `GrassColor.get` / `FoliageColor.get`: the map's colour for a climate, where humidity is
/// scaled by temperature.
pub(crate) fn lookup(map: &image::RgbaImage, temperature: f32, downfall: f32) -> [f32; 3] {
    let (t, d) = (temperature.clamp(0.0, 1.0), downfall.clamp(0.0, 1.0));
    let (x, y) = (((1.0 - t) * 255.0) as u32, ((1.0 - d * t) * 255.0) as u32);
    let p = map.get_pixel(x.min(255), y.min(255));
    [
        p[0] as f32 / 255.0,
        p[1] as f32 / 255.0,
        p[2] as f32 / 255.0,
    ]
}

pub(crate) fn rgb(c: u32) -> [f32; 3] {
    [
        (c >> 16 & 0xFF) as f32 / 255.0,
        (c >> 8 & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
    ]
}

fn parse_hex(v: &serde_json::Value) -> Option<u32> {
    v.as_str()
        .and_then(|s| u32::from_str_radix(s.trim_start_matches('#'), 16).ok())
        .or_else(|| v.as_u64().map(|n| n as u32))
}

struct BiomeColors {
    grass: [f32; 3],
    foliage: [f32; 3],
    dry_foliage: [f32; 3],
    water: [f32; 3],
}

pub struct Biomes {
    colors: HashMap<String, BiomeColors>,
    fallback: BiomeColors,
}

impl Biomes {
    /// Loads the named biomes' colours from their data files.
    pub fn load(res: &Resources, names: impl IntoIterator<Item = String>) -> Result<Self> {
        let colormap = |name: &str| colormap(res, name);
        let (grass_map, foliage_map) = (colormap("grass")?, colormap("foliage")?);
        let dry_foliage_map = colormap("dry_foliage")?;

        let mut colors = HashMap::new();
        for name in names {
            let Ok(bytes) = std::fs::read(res.data_path("worldgen/biome", &name, "json")) else {
                continue;
            };
            let json: serde_json::Value =
                serde_json::from_slice(&bytes).with_context(|| format!("biome {name}"))?;
            let temperature = json["temperature"].as_f64().unwrap_or(0.5) as f32;
            let downfall = json["downfall"].as_f64().unwrap_or(0.5) as f32;
            let effects = &json["effects"];
            let mut grass = parse_hex(&effects["grass_color"])
                .map(rgb)
                .unwrap_or_else(|| lookup(&grass_map, temperature, downfall));
            let foliage = parse_hex(&effects["foliage_color"])
                .map(rgb)
                .unwrap_or_else(|| lookup(&foliage_map, temperature, downfall));
            let dry_foliage = parse_hex(&effects["dry_foliage_color"])
                .map(rgb)
                .unwrap_or_else(|| lookup(&dry_foliage_map, temperature, downfall));
            let water = parse_hex(&effects["water_color"])
                .map(rgb)
                .unwrap_or(rgb(0x3F76E4));
            match effects["grass_color_modifier"].as_str() {
                // Vanilla picks between two swamp colours with noise; this is the common one.
                Some("swamp") => grass = rgb(0x6A7039),
                Some("dark_forest") => {
                    let c = to_u32(grass);
                    grass = rgb(((c & 0xFEFEFE) + 0x28340A) >> 1);
                }
                _ => {}
            }
            colors.insert(
                name,
                BiomeColors {
                    grass,
                    foliage,
                    dry_foliage,
                    water,
                },
            );
        }
        let fallback = BiomeColors {
            grass: lookup(&grass_map, 0.8, 0.4),
            foliage: lookup(&foliage_map, 0.8, 0.4),
            dry_foliage: lookup(&dry_foliage_map, 0.8, 0.4),
            water: rgb(0x3F76E4),
        };
        Ok(Self { colors, fallback })
    }

    pub fn color(&self, tint: Tint, biome: Option<&str>) -> [f32; 3] {
        let colors = biome
            .and_then(|b| self.colors.get(b))
            .unwrap_or(&self.fallback);
        match tint {
            Tint::Grass => colors.grass,
            Tint::Foliage => colors.foliage,
            Tint::DryFoliage => colors.dry_foliage,
            Tint::Water => colors.water,
            Tint::Fixed(c) => c,
        }
    }
}

fn to_u32(c: [f32; 3]) -> u32 {
    let [r, g, b] = c.map(|v| (v * 255.0).round() as u32);
    r << 16 | g << 8 | b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(name: &str, properties: &[(&str, &str)]) -> u32 {
        match block_tint(name, properties) {
            Some(Tint::Fixed(c)) => to_u32(c),
            other => panic!("{name}: {other:?}"),
        }
    }

    /// Wire colours as 26.3's `RedstoneWireBlock.getColorForPower` returns them (float
    /// rounding makes full power 0xFF3200), and a grown stem matching the attached stem's
    /// constant.
    #[test]
    fn state_dependent_tints() {
        assert_eq!(fixed("redstone_wire", &[("power", "0")]), 0x4C0000);
        assert_eq!(fixed("redstone_wire", &[("power", "7")]), 0xAD0000);
        assert_eq!(fixed("redstone_wire", &[("power", "15")]), 0xFF3200);
        assert_eq!(fixed("pumpkin_stem", &[("age", "0")]), 0x00FF00);
        assert_eq!(fixed("melon_stem", &[("age", "7")]), 0xE0C71C);
    }
}
