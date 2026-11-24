//! Biome colours for grass, leaves, leaf litter and water, the water fog biomes set, and
//! where they rain or snow.

use crate::noise::SimplexNoise;
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

/// `BlockTintSource.color`: a tinted block's colour outside a level (in an enderman's
/// hands), where grass takes `GrassColor.getDefaultColor`, `grass`.
pub fn color_in_hand(name: &str, tint: Tint, grass: [f32; 3]) -> [f32; 3] {
    match tint {
        // `BlockTintSources.sugarCane`.
        Tint::Grass if name == "minecraft:sugar_cane" => [1.0; 3],
        Tint::Grass => grass,
        Tint::Foliage => rgb(0x48B518),
        Tint::DryFoliage => rgb(0x5C3C32),
        Tint::Water => [1.0; 3],
        Tint::Fixed(c) => c,
    }
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
    /// `GrassColorModifier.SWAMP`: the grass is one of two colours, by noise.
    swamp: bool,
    foliage: [f32; 3],
    dry_foliage: [f32; 3],
    water: [f32; 3],
    /// The `minecraft:visual/water_fog_color` and `water_fog_end_distance` attributes.
    water_fog_color: Option<[f32; 3]>,
    water_fog_end: Option<FloatModifier>,
    /// `Biome.ClimateSettings`: whether it rains or snows at all, its temperature, and
    /// whether its temperature has the `frozen` modifier (frozen oceans' ice patches).
    has_precipitation: bool,
    temperature: f32,
    frozen: bool,
}

/// `Biome.Precipitation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precipitation {
    None,
    Rain,
    Snow,
}

/// An environment attribute's float value: replacing the value below it, or one of
/// `FloatModifier`'s operations on it.
#[derive(Clone, Copy, Debug)]
enum FloatModifier {
    Override(f32),
    Add(f32),
    Subtract(f32),
    Multiply(f32),
    Minimum(f32),
    Maximum(f32),
}

impl FloatModifier {
    /// A number, or `{"argument": n, "modifier": "multiply"}` (`AttributeModifier.FLOAT_LIBRARY`
    /// without `alpha_blend`).
    fn parse(v: &serde_json::Value) -> Option<Self> {
        if let Some(n) = v.as_f64() {
            return Some(Self::Override(n as f32));
        }
        let n = v["argument"].as_f64()? as f32;
        Some(match v["modifier"].as_str()? {
            "override" => Self::Override(n),
            "add" => Self::Add(n),
            "subtract" => Self::Subtract(n),
            "multiply" => Self::Multiply(n),
            "minimum" => Self::Minimum(n),
            "maximum" => Self::Maximum(n),
            _ => return None,
        })
    }

    fn apply(self, value: f32) -> f32 {
        match self {
            Self::Override(n) => n,
            Self::Add(n) => value + n,
            Self::Subtract(n) => value - n,
            Self::Multiply(n) => value * n,
            Self::Minimum(n) => value.min(n),
            Self::Maximum(n) => value.max(n),
        }
    }
}

pub struct Biomes {
    colors: HashMap<String, BiomeColors>,
    fallback: BiomeColors,
    /// `Biome.TEMPERATURE_NOISE`, `BIOME_INFO_NOISE`, and `FROZEN_TEMPERATURE_NOISE`'s three
    /// layers.
    temperature_noise: SimplexNoise,
    info_noise: SimplexNoise,
    frozen_noise: [SimplexNoise; 3],
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
            let attributes = &json["attributes"];
            let water_fog_color =
                parse_hex(&attributes["minecraft:visual/water_fog_color"]).map(rgb);
            let water_fog_end =
                FloatModifier::parse(&attributes["minecraft:visual/water_fog_end_distance"]);
            let modifier = effects["grass_color_modifier"].as_str();
            // `GrassColorModifier.DARK_FOREST`; `SWAMP` depends on the position (`color`).
            if modifier == Some("dark_forest") {
                let c = to_u32(grass);
                grass = rgb(((c & 0xFEFEFE) + 0x28340A) >> 1);
            }
            colors.insert(
                name,
                BiomeColors {
                    grass,
                    swamp: modifier == Some("swamp"),
                    foliage,
                    dry_foliage,
                    water,
                    water_fog_color,
                    water_fog_end,
                    has_precipitation: json["has_precipitation"].as_bool().unwrap_or(true),
                    temperature,
                    frozen: json["temperature_modifier"].as_str() == Some("frozen"),
                },
            );
        }
        let fallback = BiomeColors {
            grass: lookup(&grass_map, 0.8, 0.4),
            swamp: false,
            foliage: lookup(&foliage_map, 0.8, 0.4),
            dry_foliage: lookup(&dry_foliage_map, 0.8, 0.4),
            water: rgb(0x3F76E4),
            water_fog_color: None,
            water_fog_end: None,
            has_precipitation: true,
            temperature: 0.8,
            frozen: false,
        };
        let mut frozen = crate::bake::JavaRandom::new(3456);
        Ok(Self {
            colors,
            fallback,
            temperature_noise: SimplexNoise::new(1234),
            info_noise: SimplexNoise::new(2345),
            frozen_noise: std::array::from_fn(|_| SimplexNoise::from_random(&mut frozen)),
        })
    }

    /// `Biome.getPrecipitationAt`: none where it doesn't rain, snow where it's colder than
    /// 0.15 (`coldEnoughToSnow`), rain elsewhere.
    pub fn precipitation(&self, biome: &str, [x, y, z]: [i32; 3], sea_level: i32) -> Precipitation {
        let b = self.colors.get(biome).unwrap_or(&self.fallback);
        if !b.has_precipitation {
            return Precipitation::None;
        }
        // `TemperatureModifier.FROZEN`: patches of a frozen ocean are warm enough to rain.
        let mut temperature = b.temperature;
        if b.frozen {
            let large = [(1.0, 0.142_857_15), (0.5, 0.285_714_3), (0.25, 0.571_428_6)]
                .iter()
                .zip(&self.frozen_noise)
                .map(|(&(frequency, amplitude), n): (&(f64, f32), _)| {
                    amplitude * n.get(x as f64 * 0.05 * frequency, z as f64 * 0.05 * frequency)
                })
                .sum::<f32>() as f64
                * 7.0;
            let edge = f64::from(self.info_noise.get(x as f64 * 0.2, z as f64 * 0.2));
            if large + edge < 0.3 && self.info_noise.get(x as f64 * 0.09, z as f64 * 0.09) < 0.8 {
                temperature = 0.2;
            }
        }
        // `getHeightAdjustedTemperature`: colder above 17 blocks over the sea.
        let snow_level = sea_level + 17;
        if y > snow_level {
            let v = self
                .temperature_noise
                .get(f64::from(x as f32 / 8.0), f64::from(z as f32 / 8.0))
                * 8.0;
            temperature -= (v + (y - snow_level) as f32) * 0.05 / 40.0;
        }
        if temperature < 0.15 {
            Precipitation::Snow
        } else {
            Precipitation::Rain
        }
    }

    /// Whether the biome has rain or snow at all (`Biome.hasPrecipitation`).
    pub fn has_precipitation(&self, biome: &str) -> bool {
        self.colors
            .get(biome)
            .unwrap_or(&self.fallback)
            .has_precipitation
    }

    /// The biome's water fog colour and end distance, from the dimension's `color` and `end`
    /// (`Biome.getAttributes().applyModifier`).
    pub fn water_fog(&self, biome: &str, color: [f32; 3], end: f32) -> ([f32; 3], f32) {
        let b = self.colors.get(biome).unwrap_or(&self.fallback);
        (
            b.water_fog_color.unwrap_or(color),
            b.water_fog_end.map_or(end, |m| m.apply(end)),
        )
    }

    /// The tint's colour in the biome at block `x`, `z` (`ColorResolver.getColor`).
    pub fn color(&self, tint: Tint, biome: Option<&str>, [x, z]: [i32; 2]) -> [f32; 3] {
        let colors = biome
            .and_then(|b| self.colors.get(b))
            .unwrap_or(&self.fallback);
        match tint {
            // `GrassColorModifier.SWAMP.modifyColor`.
            Tint::Grass if colors.swamp => {
                let ground = self.info_noise.get(x as f64 * 0.0225, z as f64 * 0.0225);
                rgb(if ground < -0.1 { 0x4C763C } else { 0x6A7039 })
            }
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

    /// Swamps set their own water fog colour and multiply its distance by 0.85; warm oceans
    /// only set the colour; plains keep the dimension's.
    #[test]
    fn biome_water_fog() {
        let res = Resources::new(mc_assets::default_root());
        let names = [
            "minecraft:swamp",
            "minecraft:warm_ocean",
            "minecraft:plains",
        ];
        let biomes = Biomes::load(&res, names.map(str::to_owned)).unwrap();
        let fog = |biome| {
            let (color, end) = biomes.water_fog(biome, rgb(0x050533), 96.0);
            (to_u32(color), end)
        };
        assert_eq!(fog("minecraft:swamp"), (0x232317, 96.0 * 0.85));
        assert_eq!(fog("minecraft:warm_ocean"), (0x041F33, 96.0));
        assert_eq!(fog("minecraft:plains"), (0x050533, 96.0));
    }
}
