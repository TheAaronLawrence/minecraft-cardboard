//! The client's own light engine. A server sends light with each chunk, but when blocks
//! change it sends new light only to players at the edge of its view
//! (`ChunkHolder.broadcastChanges`); the vanilla client works out the rest itself
//! (`ClientLevel.tick` runs `LevelLightEngine.runLightUpdates` on the blocks
//! `LevelChunk.setBlockState` queued with `checkBlock`). This ports `LightEngine`,
//! `BlockLightEngine`, `SkyLightEngine` and `ChunkSkyLightSources` over the light arrays
//! the streaming thread's `WorldView` keeps.
//!
//! Vanilla stores light only for sections that have blocks and their neighbours; here every
//! section of a loaded chunk stores it. A section the server sent no sky light for reads
//! as the bottom layer of the nearest one above (`SkyLightSectionStorage.createDataLayer`),
//! and its array is made that way when light is first written to it.

use mc_physics::BlockPhysics;
use mc_registry::Registry;
use mc_render::mesh::WorldView;
use std::collections::{HashMap, HashSet, VecDeque};

/// `Direction` offsets by ordinal: down, up, north, south, west, east. A direction's
/// opposite is its ordinal xor 1.
const DIRECTIONS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];
const DOWN: usize = 0;
const UP: usize = 1;

/// `LightEngine.QueueEntry`: a level in the low 4 bits, a bit per direction to spread in,
/// and two flags.
mod entry {
    pub const ALL_DIRECTIONS: u64 = 1008;
    pub const FROM_EMPTY_SHAPE: u64 = 1024;
    pub const FROM_EMISSION: u64 = 2048;

    const fn bit(direction: usize) -> u64 {
        1 << (direction + 4)
    }
    const fn empty(from_empty_shape: bool) -> u64 {
        if from_empty_shape {
            FROM_EMPTY_SHAPE
        } else {
            0
        }
    }
    pub const fn level(entry: u64) -> i32 {
        (entry & 15) as i32
    }
    pub const fn spreads(entry: u64, direction: usize) -> bool {
        entry & bit(direction) != 0
    }
    pub const fn decrease_all(level: i32) -> u64 {
        ALL_DIRECTIONS | level as u64
    }
    pub const fn decrease_skip(level: i32, skip: usize) -> u64 {
        (ALL_DIRECTIONS & !bit(skip)) | level as u64
    }
    pub const fn increase_from_emission(level: i32, from_empty_shape: bool) -> u64 {
        ALL_DIRECTIONS | FROM_EMISSION | empty(from_empty_shape) | level as u64
    }
    pub const fn increase_skip(level: i32, from_empty_shape: bool, skip: usize) -> u64 {
        (ALL_DIRECTIONS & !bit(skip)) | empty(from_empty_shape) | level as u64
    }
    pub const fn increase_only(level: i32, from_empty_shape: bool, direction: usize) -> u64 {
        bit(direction) | empty(from_empty_shape) | level as u64
    }
}

const PULL_LIGHT_IN: u64 = entry::decrease_all(1);
const REMOVE_TOP_SKY_SOURCE: u64 = entry::decrease_all(15);
const REMOVE_SKY_SOURCE: u64 = entry::decrease_skip(15, UP);
const ADD_SKY_SOURCE: u64 = entry::increase_skip(15, false, UP);

/// What relighting needs to know about blocks.
pub struct Lighting {
    blocks: BlockPhysics,
    air: u32,
}

impl Lighting {
    pub fn new(registry: &Registry) -> anyhow::Result<Self> {
        Ok(Self {
            blocks: BlockPhysics::vanilla(registry)?,
            air: registry
                .block("minecraft:air")
                .map(|b| b.default_state)
                .unwrap_or(0),
        })
    }

    /// `LightEngine.hasDifferentLightProperties`.
    pub fn differs(&self, old: u32, new: u32) -> bool {
        self.blocks.light_differs(old, new)
    }

    /// Relights around these changed blocks, as `runLightUpdates` does after `checkBlock`
    /// for each; the blocks must already be set in `view`. Returns the sections whose light
    /// changed, as chunk x, section y and chunk z.
    pub fn relight(&self, view: &mut WorldView, changed: &[[i32; 3]]) -> HashSet<(i32, i32, i32)> {
        let nodes: HashSet<[i32; 3]> = changed.iter().copied().collect();
        let mut sections = HashSet::new();
        for sky in [true, false] {
            let mut engine = Engine {
                view: &mut *view,
                blocks: &self.blocks,
                air: self.air,
                sky,
                decrease: VecDeque::new(),
                increase: VecDeque::new(),
                sources: HashMap::new(),
                changed: &mut sections,
            };
            engine.run(&nodes);
        }
        sections
    }
}

/// One layer's engine for one run: `SkyLightEngine` when `sky`, else `BlockLightEngine`.
struct Engine<'a> {
    view: &'a mut WorldView,
    blocks: &'a BlockPhysics,
    air: u32,
    sky: bool,
    decrease: VecDeque<([i32; 3], u64)>,
    increase: VecDeque<([i32; 3], u64)>,
    /// `ChunkSkyLightSources.getLowestSourceY` by block column, found as needed. Every
    /// block is set before the run, so they don't change during it.
    sources: HashMap<(i32, i32), i32>,
    changed: &'a mut HashSet<(i32, i32, i32)>,
}

fn offset([x, y, z]: [i32; 3], direction: usize) -> [i32; 3] {
    let [dx, dy, dz] = DIRECTIONS[direction];
    [x + dx, y + dy, z + dz]
}

impl Engine<'_> {
    /// `LightEngine.runLightUpdates`.
    fn run(&mut self, nodes: &HashSet<[i32; 3]>) {
        for &node in nodes {
            if self.sky {
                self.check_sky(node);
            } else {
                self.check_block(node);
            }
        }
        while let Some((node, data)) = self.decrease.pop_front() {
            self.propagate_decrease(node, data);
        }
        // `propagateIncreases`.
        while let Some((node, data)) = self.increase.pop_front() {
            let mut level = self.get(node);
            let target = entry::level(data);
            if data & entry::FROM_EMISSION != 0 && level < target {
                self.set(node, target);
                level = target;
            }
            if level == target {
                self.propagate_increase(node, data, level);
            }
        }
    }

    /// The loaded chunk's lowest and one past its highest section y.
    fn sections(&self, x: i32, z: i32) -> Option<(i32, i32)> {
        let chunk = self.view.column(x.div_euclid(16), z.div_euclid(16))?;
        Some((chunk.sections.first()?.y, chunk.sections.last()?.y + 1))
    }

    /// `storingLightForSection`.
    fn storing(&self, [x, y, z]: [i32; 3]) -> bool {
        self.sections(x, z)
            .is_some_and(|(bottom, top)| (bottom..top).contains(&y.div_euclid(16)))
    }

    fn state(&self, [x, y, z]: [i32; 3]) -> u32 {
        self.view.state(x, y, z)
    }

    /// `getStoredLevel`.
    fn get(&self, [x, y, z]: [i32; 3]) -> i32 {
        let Some(chunk) = self.view.column(x.div_euclid(16), z.div_euclid(16)) else {
            return 0;
        };
        let (lx, lz) = (x.rem_euclid(16) as usize, z.rem_euclid(16) as usize);
        let section_y = y.div_euclid(16);
        let section = chunk.section(section_y);
        let ly = y.rem_euclid(16) as usize;
        i32::from(if self.sky {
            section
                .and_then(|s| s.sky_light(lx, ly, lz))
                .unwrap_or_else(|| {
                    chunk
                        .sections
                        .iter()
                        .filter(|s| s.y > section_y)
                        .find_map(|s| s.sky_light(lx, 0, lz))
                        .unwrap_or(15)
                })
        } else {
            section.and_then(|s| s.block_light(lx, ly, lz)).unwrap_or(0)
        })
    }

    /// `setStoredLevel`.
    fn set(&mut self, [x, y, z]: [i32; 3], level: i32) {
        let (cx, cz) = (x.div_euclid(16), z.div_euclid(16));
        let section_y = y.div_euclid(16);
        let sky = self.sky;
        let Some(chunk) = self.view.column_mut(cx, cz) else {
            return;
        };
        let Some(i) = chunk.sections.iter().position(|s| s.y == section_y) else {
            return;
        };
        let missing = if sky {
            chunk.sections[i].sky_light.is_none()
        } else {
            chunk.sections[i].block_light.is_none()
        };
        if missing {
            let array = if sky {
                // `SkyLightSectionStorage.repeatFirstLayer` of the nearest layer above.
                match chunk.sections[i + 1..]
                    .iter()
                    .find_map(|s| s.sky_light.as_deref())
                {
                    Some(above) => {
                        let mut array = [0; 2048];
                        for layer in array.chunks_mut(128) {
                            layer.copy_from_slice(&above[..128]);
                        }
                        array
                    }
                    None => [0xFF; 2048],
                }
            } else {
                [0; 2048]
            };
            let slot = Some(Box::new(array));
            if sky {
                chunk.sections[i].sky_light = slot;
            } else {
                chunk.sections[i].block_light = slot;
            }
        }
        let section = &mut chunk.sections[i];
        let array = if sky {
            section.sky_light.as_deref_mut()
        } else {
            section.block_light.as_deref_mut()
        }
        .expect("made above");
        let index = (y.rem_euclid(16) as usize * 16 + z.rem_euclid(16) as usize) * 16
            + x.rem_euclid(16) as usize;
        let shift = (index % 2) * 4;
        array[index / 2] = (array[index / 2] & !(0xF << shift)) | ((level as u8 & 0xF) << shift);
        self.changed.insert((cx, section_y, cz));
    }

    /// `LightEngine.getOpacity`.
    fn opacity(&self, state: u32) -> i32 {
        i32::from(self.blocks.light_dampening(state)).max(1)
    }

    /// `BlockLightEngine.getEmission`: what a block in a loaded chunk gives off.
    fn emission(&self, state: u32) -> i32 {
        i32::from(self.blocks.light_emission(state))
    }

    /// `BlockLightEngine.checkNode`.
    fn check_block(&mut self, node: [i32; 3]) {
        if !self.storing(node) {
            return;
        }
        let state = self.state(node);
        let emission = self.emission(state);
        let old = self.get(node);
        if emission < old {
            self.set(node, 0);
            self.decrease.push_back((node, entry::decrease_all(old)));
        } else {
            self.decrease.push_back((node, PULL_LIGHT_IN));
        }
        if emission > 0 {
            let empty = self.blocks.empty_light_shape(state);
            self.increase
                .push_back((node, entry::increase_from_emission(emission, empty)));
        }
    }

    /// `SkyLightEngine.checkNode`.
    fn check_sky(&mut self, node: [i32; 3]) {
        let [x, y, z] = node;
        let lowest = self.lowest_source(x, z, i32::MAX);
        if lowest != i32::MAX {
            self.remove_sources_below(x, z, lowest);
            self.add_sources_above(x, z, lowest);
        }
        if !self.storing(node) {
            return;
        }
        if y >= lowest {
            self.decrease.push_back((node, REMOVE_SKY_SOURCE));
            self.increase.push_back((node, ADD_SKY_SOURCE));
        } else {
            let old = self.get(node);
            if old > 0 {
                self.set(node, 0);
                self.decrease.push_back((node, entry::decrease_all(old)));
            } else {
                self.decrease.push_back((node, PULL_LIGHT_IN));
            }
        }
    }

    /// `ChunkSkyLightSources.getLowestSourceY`: the lowest y in the column that sees the sky,
    /// `i32::MIN` when all of it does, or `default` outside the loaded chunks.
    fn lowest_source(&mut self, x: i32, z: i32, default: i32) -> i32 {
        let Some((bottom, top)) = self.sections(x, z) else {
            return default;
        };
        if let Some(&y) = self.sources.get(&(x, z)) {
            return y;
        }
        // `findLowestSourceY`: down from the top to the first edge light can't cross.
        let mut above = self.air;
        let mut lowest = i32::MIN;
        for y in (bottom * 16..top * 16).rev() {
            let below = self.view.state(x, y, z);
            if self.blocks.light_dampening(below) != 0
                || self.blocks.light_occludes(above, below, DOWN)
            {
                lowest = y + 1;
                break;
            }
            above = below;
        }
        self.sources.insert((x, z), lowest);
        lowest
    }

    /// `SkyLightEngine.removeSourcesBelow`: blocks newly in shadow lose their full light.
    fn remove_sources_below(&mut self, x: i32, z: i32, lowest: i32) {
        let Some((bottom, _)) = self.sections(x, z) else {
            return;
        };
        if lowest <= bottom * 16 {
            return;
        }
        for y in (bottom * 16..lowest).rev() {
            let node = [x, y, z];
            if self.get(node) != 15 {
                return;
            }
            self.set(node, 0);
            let data = if y == lowest - 1 {
                REMOVE_TOP_SKY_SOURCE
            } else {
                REMOVE_SKY_SOURCE
            };
            self.decrease.push_back((node, data));
        }
    }

    /// `SkyLightEngine.addSourcesAbove`: blocks newly open to the sky get full light.
    fn add_sources_above(&mut self, x: i32, z: i32, lowest: i32) {
        let Some((bottom, top)) = self.sections(x, z) else {
            return;
        };
        let neighbours = [(-1, 0), (1, 0), (0, -1), (0, 1)]
            .map(|(dx, dz)| self.lowest_source(x + dx, z + dz, i32::MIN))
            .into_iter()
            .max()
            .unwrap_or(i32::MIN);
        for y in lowest.max(bottom * 16)..top * 16 {
            let node = [x, y, z];
            if self.get(node) == 15 {
                return;
            }
            self.set(node, 15);
            if y < neighbours || y == lowest {
                self.increase.push_back((node, ADD_SKY_SOURCE));
            }
        }
    }

    /// `BlockLightEngine.propagateIncrease` and `SkyLightEngine.propagateIncrease`, which
    /// are the same while every section stores light.
    fn propagate_increase(&mut self, from: [i32; 3], data: u64, from_level: i32) {
        let mut from_state = None;
        for direction in 0..6 {
            if !entry::spreads(data, direction) {
                continue;
            }
            let to = offset(from, direction);
            if !self.storing(to) {
                continue;
            }
            let to_level = self.get(to);
            if from_level - 1 <= to_level {
                continue;
            }
            let to_state = self.state(to);
            let new_level = from_level - self.opacity(to_state);
            if new_level <= to_level {
                continue;
            }
            let from_state = *from_state.get_or_insert_with(|| {
                if data & entry::FROM_EMPTY_SHAPE != 0 {
                    self.air
                } else {
                    self.state(from)
                }
            });
            if !self.blocks.light_occludes(from_state, to_state, direction) {
                self.set(to, new_level);
                if new_level > 1 {
                    let empty = self.blocks.empty_light_shape(to_state);
                    self.increase
                        .push_back((to, entry::increase_skip(new_level, empty, direction ^ 1)));
                }
            }
        }
    }

    /// `BlockLightEngine.propagateDecrease` and `SkyLightEngine.propagateDecrease`: light
    /// that came from the removed level goes, and what's left lights the gap again.
    fn propagate_decrease(&mut self, from: [i32; 3], data: u64) {
        let old_from_level = entry::level(data);
        for direction in 0..6 {
            if !entry::spreads(data, direction) {
                continue;
            }
            let to = offset(from, direction);
            if !self.storing(to) {
                continue;
            }
            let to_level = self.get(to);
            if to_level == 0 {
                continue;
            }
            if to_level < old_from_level {
                let (emission, empty) = if self.sky {
                    (0, false)
                } else {
                    let state = self.state(to);
                    (self.emission(state), self.blocks.empty_light_shape(state))
                };
                self.set(to, 0);
                if emission < to_level {
                    self.decrease
                        .push_back((to, entry::decrease_skip(to_level, direction ^ 1)));
                }
                if emission > 0 {
                    self.increase
                        .push_back((to, entry::increase_from_emission(emission, empty)));
                }
            } else {
                self.increase
                    .push_back((to, entry::increase_only(to_level, false, direction ^ 1)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_world::{Blocks, Chunk, Section};

    /// Building a roof over open sky shades what's under it by its distance from the
    /// roof's edge, taking the roof away lights it fully again, and glowstone lights what's
    /// around it, all as the vanilla engine would.
    #[test]
    fn changed_blocks_relight() {
        let registry = Registry::vanilla().unwrap();
        let state = |name: &str| registry.block(name).unwrap().default_state;
        let (air, stone, glowstone) = (
            state("minecraft:air"),
            state("minecraft:stone"),
            state("minecraft:glowstone"),
        );
        let mut view = WorldView::empty(&registry);
        for x in -1..=1 {
            for z in -1..=1 {
                let sections = (-1..=1)
                    .map(|y| Section {
                        y,
                        blocks: Blocks::Uniform(air),
                        biome_palette: Vec::new(),
                        biomes: [0; 64],
                        block_light: None,
                        sky_light: None,
                    })
                    .collect();
                view.insert(Chunk {
                    x,
                    z,
                    status: "minecraft:full".into(),
                    sections,
                    block_entities: Default::default(),
                });
            }
        }
        let lighting = Lighting::new(&registry).unwrap();
        let set = |view: &mut WorldView, blocks: &[[i32; 3]], state: u32| {
            for &pos in blocks {
                view.set_block(pos, state);
            }
            lighting.relight(view, blocks);
        };
        let roof: Vec<[i32; 3]> = (-3..=3)
            .flat_map(|x| (-3..=3).map(move |z| [x, 10, z]))
            .collect();
        set(&mut view, &roof, stone);
        // The nearest open column is 4 blocks away, at x or z ±4.
        assert_eq!(view.light(0, 9, 0), (11, 0));
        assert_eq!(view.light(0, 2, 0), (11, 0));
        assert_eq!(view.light(3, 9, 1), (14, 0));
        assert_eq!(view.light(0, 11, 0), (15, 0));

        set(&mut view, &[[0, 5, 0]], glowstone);
        assert_eq!(view.light(0, 3, 0), (11, 13));
        assert_eq!(view.light(2, 6, 1), (13, 11));

        set(&mut view, &roof, air);
        assert_eq!(view.light(0, 9, 0), (15, 11));
        // The glowstone above shades it, so its sky light comes from the side.
        assert_eq!(view.light(0, 2, 0), (14, 12));
    }
}
