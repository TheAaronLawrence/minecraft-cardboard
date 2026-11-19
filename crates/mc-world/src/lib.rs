//! Reads Java Edition saves: region files (`region/r.X.Z.mca`) and the chunks inside them,
//! with block states mapped to our registry's ids.

use anyhow::{Context, Result, bail, ensure};
use mc_nbt::{Compound, Tag};
use mc_registry::Registry;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

const SECTOR: usize = 4096;

/// A save's overworld. Region files are read once and kept.
pub struct World {
    region_dir: PathBuf,
    regions: HashMap<(i32, i32), Option<Vec<u8>>>,
}

impl World {
    /// `save` is the world folder (the one holding `level.dat`).
    pub fn open(save: impl AsRef<Path>) -> Self {
        // 26.x saves keep each dimension under `dimensions/`; older ones keep the
        // overworld's regions at the top level.
        let save = save.as_ref();
        let current = save.join("dimensions/minecraft/overworld/region");
        let region_dir = if current.is_dir() {
            current
        } else {
            save.join("region")
        };
        Self {
            region_dir,
            regions: HashMap::new(),
        }
    }

    /// The chunk at chunk coordinates `cx`, `cz`, or `None` if it was never saved.
    pub fn chunk(&mut self, registry: &Registry, cx: i32, cz: i32) -> Result<Option<Chunk>> {
        let (rx, rz) = (cx.div_euclid(32), cz.div_euclid(32));
        let dir = &self.region_dir;
        let region = self
            .regions
            .entry((rx, rz))
            .or_insert_with(|| std::fs::read(dir.join(format!("r.{rx}.{rz}.mca"))).ok());
        let Some(region) = region else {
            return Ok(None);
        };
        let Some(nbt) = chunk_nbt(region, &self.region_dir, cx, cz)? else {
            return Ok(None);
        };
        let (_, root) = mc_nbt::from_bytes(&nbt).with_context(|| format!("chunk {cx},{cz}"))?;
        Chunk::from_nbt(&root, registry)
            .map(Some)
            .with_context(|| format!("chunk {cx},{cz}"))
    }
}

/// The uncompressed NBT of one chunk in a region file.
fn chunk_nbt(region: &[u8], region_dir: &Path, cx: i32, cz: i32) -> Result<Option<Vec<u8>>> {
    ensure!(
        region.len() >= 2 * SECTOR,
        "region file is shorter than its header"
    );
    let slot = (cx.rem_euclid(32) + cz.rem_euclid(32) * 32) as usize * 4;
    let location = u32::from_be_bytes(region[slot..slot + 4].try_into().expect("4 bytes"));
    let (offset, sectors) = (
        (location >> 8) as usize * SECTOR,
        (location & 0xFF) as usize,
    );
    if offset == 0 || sectors == 0 {
        return Ok(None);
    }
    let header = region
        .get(offset..offset + 5)
        .context("chunk offset is past the end of the file")?;
    let length = u32::from_be_bytes(header[..4].try_into().expect("4 bytes")) as usize;
    let compression = header[4];
    // Bit 7 means the chunk was too big for the region and lives in its own file.
    let stored: Vec<u8> = if compression & 0x80 != 0 {
        std::fs::read(region_dir.join(format!("c.{cx}.{cz}.mcc")))?
    } else {
        ensure!(length >= 1, "chunk length is zero");
        region
            .get(offset + 5..offset + 4 + length)
            .context("chunk runs past the end of the file")?
            .to_vec()
    };
    let mut out = Vec::new();
    match compression & 0x7F {
        1 => flate2::read::GzDecoder::new(&stored[..]).read_to_end(&mut out)?,
        2 => flate2::read::ZlibDecoder::new(&stored[..]).read_to_end(&mut out)?,
        3 => return Ok(Some(stored)),
        4 => bail!("LZ4-compressed chunks aren't supported yet"),
        other => bail!("unknown chunk compression {other}"),
    };
    Ok(Some(out))
}

#[derive(Debug)]
pub struct Chunk {
    pub x: i32,
    pub z: i32,
    /// Generation stage, such as `minecraft:full`.
    pub status: String,
    /// Bottom to top, one per 16 blocks of height. The first may be below the world's floor
    /// and the last above its ceiling: vanilla keeps one extra each way for light.
    pub sections: Vec<Section>,
}

#[derive(Debug)]
pub struct Section {
    /// Section y: block y divided by 16.
    pub y: i32,
    pub blocks: Blocks,
    /// Biome names in this section's palette, and each 4×4×4 cell's index into it.
    pub biome_palette: Vec<String>,
    pub biomes: [u8; 64],
    /// 4-bit light levels per block, or `None` when the save left them out.
    pub block_light: Option<Box<[u8; 2048]>>,
    pub sky_light: Option<Box<[u8; 2048]>>,
}

#[derive(Debug)]
pub enum Blocks {
    /// Every block in the section has this state, such as all air.
    Uniform(u32),
    /// State id per block, indexed `(y * 16 + z) * 16 + x`.
    Mixed(Box<[u32; 4096]>),
}

impl Chunk {
    fn from_nbt(root: &Compound, registry: &Registry) -> Result<Self> {
        let int = |name: &str| {
            root.get(name)
                .and_then(Tag::as_i64)
                .with_context(|| format!("no {name}"))
        };
        let (x, z) = (int("xPos")? as i32, int("zPos")? as i32);
        let status = root
            .get("Status")
            .and_then(Tag::as_str)
            .unwrap_or_default()
            .to_owned();
        let air = registry
            .block("minecraft:air")
            .context("no air in the registry")?
            .default_state;
        let mut sections = root
            .get("sections")
            .and_then(Tag::as_list)
            .unwrap_or_default()
            .iter()
            .map(|tag| {
                Section::from_nbt(
                    tag.as_compound().context("section is not a compound")?,
                    registry,
                    air,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        sections.sort_by_key(|s| s.y);
        Ok(Self {
            x,
            z,
            status,
            sections,
        })
    }

    pub fn section(&self, section_y: i32) -> Option<&Section> {
        let first = self.sections.first()?.y;
        self.sections
            .get(usize::try_from(section_y - first).ok()?)
            .filter(|s| s.y == section_y)
    }

    /// The state at world block coordinates, which must be inside this chunk. Outside the
    /// saved sections it's `None`.
    pub fn block(&self, x: i32, y: i32, z: i32) -> Option<u32> {
        debug_assert_eq!((x.div_euclid(16), z.div_euclid(16)), (self.x, self.z));
        let section = self.section(y.div_euclid(16))?;
        Some(section.block(
            x.rem_euclid(16) as usize,
            y.rem_euclid(16) as usize,
            z.rem_euclid(16) as usize,
        ))
    }
}

impl Section {
    fn from_nbt(nbt: &Compound, registry: &Registry, air: u32) -> Result<Self> {
        let y = nbt
            .get("Y")
            .and_then(Tag::as_i64)
            .context("section has no Y")? as i32;
        let blocks = match nbt.get("block_states").and_then(Tag::as_compound) {
            None => Blocks::Uniform(air),
            Some(states) => {
                let palette = states
                    .get("palette")
                    .and_then(Tag::as_list)
                    .context("block_states has no palette")?
                    .iter()
                    .map(|entry| palette_state(entry, registry, air))
                    .collect::<Result<Vec<u32>>>()?;
                match palette.as_slice() {
                    [only] => Blocks::Uniform(*only),
                    _ => {
                        let indices = unpack(states.get("data"), palette.len(), 4, 4096)?;
                        let mut ids = Box::new([0; 4096]);
                        for (id, index) in ids.iter_mut().zip(indices) {
                            *id = *palette
                                .get(index)
                                .context("block index outside the palette")?;
                        }
                        Blocks::Mixed(ids)
                    }
                }
            }
        };

        let (biome_palette, biomes) = match nbt.get("biomes").and_then(Tag::as_compound) {
            None => (Vec::new(), [0; 64]),
            Some(biomes) => {
                let palette: Vec<String> = biomes
                    .get("palette")
                    .and_then(Tag::as_list)
                    .context("biomes has no palette")?
                    .iter()
                    .map(|t| {
                        unwrap_list_item(t)
                            .as_str()
                            .map(str::to_owned)
                            .context("biome name is not a string")
                    })
                    .collect::<Result<_>>()?;
                let mut cells = [0u8; 64];
                if palette.len() > 1 {
                    for (cell, index) in
                        cells
                            .iter_mut()
                            .zip(unpack(biomes.get("data"), palette.len(), 1, 64)?)
                    {
                        ensure!(index < palette.len(), "biome index outside the palette");
                        *cell = index as u8;
                    }
                }
                (palette, cells)
            }
        };

        let light = |name: &str| -> Result<Option<Box<[u8; 2048]>>> {
            match nbt.get(name) {
                None => Ok(None),
                Some(Tag::ByteArray(bytes)) => {
                    let bytes: Vec<u8> = bytes.iter().map(|b| *b as u8).collect();
                    Ok(Some(bytes.into_boxed_slice().try_into().map_err(|_| {
                        anyhow::anyhow!("{name} is not 2048 bytes")
                    })?))
                }
                Some(_) => bail!("{name} is not a byte array"),
            }
        };
        Ok(Self {
            y,
            blocks,
            biome_palette,
            biomes,
            block_light: light("BlockLight")?,
            sky_light: light("SkyLight")?,
        })
    }

    /// Coordinates are 0..16 within the section.
    pub fn block(&self, x: usize, y: usize, z: usize) -> u32 {
        match &self.blocks {
            Blocks::Uniform(id) => *id,
            Blocks::Mixed(ids) => ids[(y * 16 + z) * 16 + x],
        }
    }

    /// The biome name at block coordinates 0..16 within the section.
    pub fn biome(&self, x: usize, y: usize, z: usize) -> Option<&str> {
        let index = self.biomes[(y / 4 * 4 + z / 4) * 4 + x / 4];
        self.biome_palette.get(index as usize).map(String::as_str)
    }

    pub fn sky_light(&self, x: usize, y: usize, z: usize) -> Option<u8> {
        self.sky_light.as_deref().map(|l| nibble(l, x, y, z))
    }

    pub fn block_light(&self, x: usize, y: usize, z: usize) -> Option<u8> {
        self.block_light.as_deref().map(|l| nibble(l, x, y, z))
    }
}

fn nibble(light: &[u8; 2048], x: usize, y: usize, z: usize) -> u8 {
    let index = (y * 16 + z) * 16 + x;
    (light[index / 2] >> ((index % 2) * 4)) & 0xF
}

/// A palette entry as a state id. 26.x writes a default state as just its name and others
/// as `{id, properties}`; a list mixing the two wraps each name as `{"": name}`. Older saves
/// write `{Name, Properties}`. Like vanilla, an unknown block becomes air and unknown
/// properties keep their defaults.
fn palette_state(entry: &Tag, registry: &Registry, air: u32) -> Result<u32> {
    let entry = unwrap_list_item(entry);
    let (name, properties) = match entry {
        Tag::String(name) => (name.as_str(), None),
        Tag::Compound(c) => {
            let name = c
                .get("id")
                .or_else(|| c.get("Name"))
                .and_then(Tag::as_str)
                .context("palette entry has no id")?;
            (
                name,
                c.get("properties")
                    .or_else(|| c.get("Properties"))
                    .and_then(Tag::as_compound),
            )
        }
        _ => bail!("palette entry is neither a name nor a compound"),
    };
    let Some(block) = registry.block(name) else {
        return Ok(air);
    };
    Ok(block.state(|p| {
        properties
            .and_then(|props| props.get(p))
            .and_then(Tag::as_str)
    }))
}

/// Vanilla stores a list of mixed tag types by wrapping each item that isn't a compound in
/// a compound under the empty key.
fn unwrap_list_item(tag: &Tag) -> &Tag {
    match tag {
        Tag::Compound(c) if c.len() == 1 => c.get("").unwrap_or(tag),
        _ => tag,
    }
}

/// Unpacks `count` palette indices from a long array. Since 1.16 an index never spans two
/// longs: each long holds `64 / bits` indices from its lowest bits up, and the rest is unused.
fn unpack(
    data: Option<&Tag>,
    palette_len: usize,
    min_bits: u32,
    count: usize,
) -> Result<Vec<usize>> {
    let bits = (usize::BITS - (palette_len - 1).leading_zeros()).max(min_bits) as usize;
    let Some(Tag::LongArray(longs)) = data else {
        bail!("paletted data is missing")
    };
    let per_long = 64 / bits;
    ensure!(
        longs.len() == count.div_ceil(per_long),
        "paletted data has {} longs, expected {}",
        longs.len(),
        count.div_ceil(per_long)
    );
    let mask = (1u64 << bits) - 1;
    Ok((0..count)
        .map(|i| ((longs[i / per_long] as u64 >> ((i % per_long) * bits)) & mask) as usize)
        .collect())
}
