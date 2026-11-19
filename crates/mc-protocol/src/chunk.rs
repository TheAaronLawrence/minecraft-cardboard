//! `level_chunk_with_light`: a chunk's sections, block entities and light as the server
//! sends them, decoded into the same `mc_world::Chunk` a save gives.

use crate::codec::Reader;
use anyhow::{Context, Result, ensure};
use mc_nbt::Tag;
use mc_registry::Registry;
use mc_world::{Blocks, Chunk, Section};
use std::collections::HashMap;

/// The height range of the dimension the player is in.
pub struct Dimension {
    /// The lowest section's y (block y / 16).
    min_section: i32,
    sections: usize,
}

impl Dimension {
    /// The section y of index `i` in light data, which starts a section below the world.
    pub fn light_section_y(&self, i: usize) -> i32 {
        self.min_section - 1 + i as i32
    }

    /// How many sections light data covers.
    pub fn light_sections(&self) -> usize {
        self.sections + 2
    }

    /// From the registry data's inline entry, or else vanilla's own file for `name`.
    pub fn new(name: &str, data: Option<&Tag>) -> Result<Self> {
        let (min_y, height) = match data.and_then(Tag::as_compound) {
            Some(c) => {
                let int = |k: &str| c.get(k).and_then(Tag::as_i64).with_context(|| k.to_owned());
                (int("min_y")?, int("height")?)
            }
            None => {
                let res = mc_assets::Resources::new(mc_assets::default_root());
                let path = res.data_path("dimension_type", name, "json");
                let json: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(&path).with_context(|| format!("dimension type {name}"))?,
                )?;
                let int = |k: &str| json[k].as_i64().with_context(|| format!("{name}: {k}"));
                (int("min_y")?, int("height")?)
            }
        };
        Ok(Self {
            min_section: (min_y as i32).div_euclid(16),
            sections: (height / 16) as usize,
        })
    }
}

pub fn decode(
    r: &mut Reader,
    dimension: &Dimension,
    registry: &Registry,
    biome_names: &[String],
    block_entity_name: &dyn Fn(u32) -> Option<String>,
) -> Result<Chunk> {
    let (cx, cz) = (r.i32()?, r.i32()?);

    // Heightmaps: a count, then each one's type id and longs. The renderer doesn't use them.
    for _ in 0..r.count()? {
        r.varint()?;
        r.long_array()?;
    }

    let data = r.byte_array()?;
    let mut sr = Reader::new(data);
    let air = registry
        .block("minecraft:air")
        .context("no air in the registry")?
        .default_state;
    // One extra section below and above, as saves keep, so the light below and above fits.
    let mut sections: Vec<Section> = (0..dimension.sections + 2)
        .map(|i| Section {
            y: dimension.min_section - 1 + i as i32,
            blocks: Blocks::Uniform(air),
            biome_palette: Vec::new(),
            biomes: [0; 64],
            block_light: None,
            sky_light: None,
        })
        .collect();
    for section in &mut sections[1..=dimension.sections] {
        sr.i16()?; // non-air blocks
        sr.i16()?; // fluids
        let states = paletted(&mut sr, 4096, 4, 8)?;
        section.blocks = match states {
            Paletted::Single(id) => Blocks::Uniform(id),
            Paletted::Mixed(ids) => {
                ensure!(
                    ids.iter().all(|&id| id < registry.state_count()),
                    "unknown block state"
                );
                Blocks::Mixed(ids.into_boxed_slice().try_into().expect("4096 entries"))
            }
        };
        let name = |id: u32| {
            biome_names
                .get(id as usize)
                .cloned()
                .with_context(|| format!("unknown biome id {id}"))
        };
        match paletted(&mut sr, 64, 1, 3)? {
            Paletted::Single(id) => section.biome_palette = vec![name(id)?],
            Paletted::Mixed(ids) => {
                // Our sections keep a palette of names and an index per cell.
                for (cell, id) in section.biomes.iter_mut().zip(ids) {
                    let name = name(id)?;
                    let index = match section.biome_palette.iter().position(|n| *n == name) {
                        Some(i) => i,
                        None => {
                            section.biome_palette.push(name);
                            section.biome_palette.len() - 1
                        }
                    };
                    *cell = index as u8;
                }
            }
        }
    }

    // Block entities: packed x and z within the chunk, y, type, and their data.
    let mut block_entities = HashMap::new();
    for _ in 0..r.count()? {
        let xz = r.u8()?;
        let y = r.i16()? as i32;
        let kind = r.count()? as u32;
        let mut data = match r.nbt()? {
            Some(Tag::Compound(c)) => c,
            _ => Default::default(),
        };
        let (x, z) = (cx * 16 + (xz >> 4) as i32, cz * 16 + (xz & 15) as i32);
        // Saves carry the id and position inside the data; match them.
        if let Some(name) = block_entity_name(kind) {
            data.insert("id".into(), Tag::String(name));
        }
        for (key, v) in [("x", x), ("y", y), ("z", z)] {
            data.insert(key.into(), Tag::Int(v));
        }
        block_entities.insert([x, y, z], data);
    }

    // Sections the light data leaves out have no array, as in a save: full sky light above
    // the terrain, no block light.
    let light = light(r, sections.len())?;
    for (i, array) in light.sky {
        sections[i].sky_light = Some(array);
    }
    for (i, array) in light.block {
        sections[i].block_light = Some(array);
    }

    Ok(Chunk {
        x: cx,
        z: cz,
        status: "minecraft:full".into(),
        sections,
        block_entities,
    })
}

/// `ClientboundLightUpdatePacketData`'s arrays, by section index (bit i of its masks is
/// section `min_section - 1 + i`). Sections marked all dark come as zeros; sections it
/// leaves out keep what they had.
pub struct Light {
    pub sky: Vec<(usize, Box<[u8; 2048]>)>,
    pub block: Vec<(usize, Box<[u8; 2048]>)>,
}

/// Reads light data for a column of `sections` sections, counting the one below and above.
pub fn light(r: &mut Reader, sections: usize) -> Result<Light> {
    let sky_mask = r.bit_set()?;
    let block_mask = r.bit_set()?;
    let empty_sky = r.bit_set()?;
    let empty_block = r.bit_set()?;
    let bit = |mask: &[u8], i: usize| mask.get(i / 8).is_some_and(|b| b >> (i % 8) & 1 != 0);
    let mut arrays = |mask: &[u8], empty: &[u8]| -> Result<Vec<(usize, Box<[u8; 2048]>)>> {
        let count = r.count()?;
        let indices: Vec<usize> = (0..sections).filter(|&i| bit(mask, i)).collect();
        ensure!(
            count == indices.len(),
            "light arrays don't match their mask"
        );
        let mut out = indices
            .into_iter()
            .map(|i| {
                let bytes = r.byte_array()?;
                let array: Box<[u8; 2048]> = bytes
                    .to_vec()
                    .into_boxed_slice()
                    .try_into()
                    .ok()
                    .context("light array isn't 2048 bytes")?;
                Ok((i, array))
            })
            .collect::<Result<Vec<_>>>()?;
        out.extend(
            (0..sections)
                .filter(|&i| bit(empty, i) && !bit(mask, i))
                .map(|i| (i, Box::new([0; 2048]))),
        );
        Ok(out)
    };
    let sky = arrays(sky_mask, empty_sky)?;
    let block = arrays(block_mask, empty_block)?;
    Ok(Light { sky, block })
}

enum Paletted {
    Single(u32),
    Mixed(Vec<u32>),
}

/// A `PalettedContainer`: the bits per entry, the palette (none when the entries are global
/// ids), and the entries packed into longs, which no entry spans. `min_bits` and
/// `max_indirect` are the strategy's: blocks pack palettes into at least 4 bits and use one
/// up to 8; biomes 1 and 3.
fn paletted(r: &mut Reader, size: usize, min_bits: u8, max_indirect: u8) -> Result<Paletted> {
    let bits = r.u8()?;
    if bits == 0 {
        return Ok(Paletted::Single(r.count()? as u32));
    }
    let palette: Option<Vec<u32>> = if bits <= max_indirect {
        Some(
            (0..r.count()?)
                .map(|_| Ok(r.count()? as u32))
                .collect::<Result<_>>()?,
        )
    } else {
        None
    };
    let bits = bits.max(min_bits) as usize;
    let per_long = 64 / bits;
    let longs = r.longs(size.div_ceil(per_long))?;
    let mask = (1u64 << bits) - 1;
    let ids = (0..size)
        .map(|i| {
            let raw = (longs[i / per_long] >> (i % per_long * bits) & mask) as u32;
            match &palette {
                Some(p) => p
                    .get(raw as usize)
                    .copied()
                    .context("index outside the palette"),
                None => Ok(raw),
            }
        })
        .collect::<Result<_>>()?;
    Ok(Paletted::Mixed(ids))
}
