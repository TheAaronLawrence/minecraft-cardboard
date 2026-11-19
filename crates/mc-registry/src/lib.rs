//! Blocks, block states and items with vanilla's numeric ids, which chunk palettes and the
//! network protocol use. Built from the data generator's reports in `resources/reports/`.

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// The `resources/reports/` folder (see `mc_assets::locate`).
pub fn default_reports_dir() -> PathBuf {
    mc_assets::locate("resources/reports")
}

pub struct Registry {
    /// Indexed by block id.
    blocks: Vec<Block>,
    block_ids: HashMap<String, u32>,
    /// State id to block id.
    state_blocks: Vec<u32>,
    /// Indexed by item id.
    items: Vec<String>,
    item_ids: HashMap<String, u32>,
    /// Indexed by block entity type id.
    block_entity_types: Vec<String>,
}

#[derive(Debug)]
pub struct Block {
    /// Such as `minecraft:oak_log`.
    pub name: String,
    pub id: u32,
    /// Sorted by name: state ids count through them like digits, the last fastest.
    pub properties: Vec<Property>,
    pub first_state: u32,
    pub default_state: u32,
}

#[derive(Debug)]
pub struct Property {
    pub name: String,
    pub values: Vec<String>,
}

impl Block {
    pub fn state_count(&self) -> u32 {
        self.properties
            .iter()
            .map(|p| p.values.len() as u32)
            .product()
    }

    pub fn has_state(&self, state: u32) -> bool {
        (self.first_state..self.first_state + self.state_count()).contains(&state)
    }

    /// The state with these property values. Like vanilla reading a save, a property that
    /// `value` leaves out or gives an unknown value keeps the default state's value.
    pub fn state<'v>(&self, value: impl Fn(&str) -> Option<&'v str>) -> u32 {
        let mut state = self.first_state;
        let mut stride = 1;
        for (i, property) in self.properties.iter().enumerate().rev() {
            let index = value(&property.name)
                .and_then(|v| property.values.iter().position(|known| known == v))
                .unwrap_or_else(|| self.value_index(self.default_state, i));
            state += index as u32 * stride;
            stride *= property.values.len() as u32;
        }
        state
    }

    /// The value of each property in `state`, in property order. Panics when the state
    /// belongs to another block.
    pub fn property_values(&self, state: u32) -> Vec<(&str, &str)> {
        assert!(
            self.has_state(state),
            "state {state} is not a {} state",
            self.name
        );
        self.properties
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    p.name.as_str(),
                    p.values[self.value_index(state, i)].as_str(),
                )
            })
            .collect()
    }

    /// States count through the properties like digits, the last property fastest.
    fn value_index(&self, state: u32, property: usize) -> usize {
        let stride: u32 = self.properties[property + 1..]
            .iter()
            .map(|p| p.values.len() as u32)
            .product();
        ((state - self.first_state) / stride) as usize % self.properties[property].values.len()
    }
}

impl Registry {
    pub fn load(reports_dir: &Path) -> Result<Self> {
        let blocks_report: HashMap<String, BlockReport> =
            read_json(&reports_dir.join("blocks.json"))?;
        let registries: HashMap<String, RegistryReport> =
            read_json(&reports_dir.join("registries.json"))?;

        let block_ids = protocol_ids(&registries, "minecraft:block")?;
        let mut blocks: Vec<Option<Block>> = (0..block_ids.len()).map(|_| None).collect();
        for (name, report) in blocks_report {
            let id = *block_ids
                .get(&name)
                .with_context(|| format!("{name} has no block id"))?;
            let first_state = report
                .states
                .iter()
                .map(|s| s.id)
                .min()
                .with_context(|| format!("{name} has no states"))?;
            let default_state = report
                .states
                .iter()
                .find(|s| s.default)
                .with_context(|| format!("{name} has no default state"))?
                .id;
            let properties = report
                .properties
                .into_iter()
                .map(|(name, values)| Property { name, values })
                .collect();
            let block = Block {
                name,
                id,
                properties,
                first_state,
                default_state,
            };
            ensure!(
                block.state_count() as usize == report.states.len(),
                "{} lists the wrong number of states",
                block.name
            );
            blocks[id as usize] = Some(block);
        }
        let blocks: Vec<Block> = blocks
            .into_iter()
            .enumerate()
            .map(|(id, b)| b.with_context(|| format!("block id {id} has no block")))
            .collect::<Result<_>>()?;

        // Each block owns a contiguous run of state ids, and together they cover 0..N.
        let mut by_first_state: Vec<&Block> = blocks.iter().collect();
        by_first_state.sort_by_key(|b| b.first_state);
        let mut state_blocks = Vec::new();
        for block in by_first_state {
            ensure!(
                block.first_state as usize == state_blocks.len(),
                "state ids of {} don't follow on",
                block.name
            );
            state_blocks.extend(std::iter::repeat_n(block.id, block.state_count() as usize));
        }

        let item_ids = protocol_ids(&registries, "minecraft:item")?;
        let items = names_by_id(&item_ids);

        let block_entity_types =
            names_by_id(&protocol_ids(&registries, "minecraft:block_entity_type")?);

        Ok(Self {
            block_ids,
            blocks,
            state_blocks,
            items,
            item_ids,
            block_entity_types,
        })
    }

    pub fn vanilla() -> Result<Self> {
        Self::load(&default_reports_dir())
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn block(&self, name: &str) -> Option<&Block> {
        self.block_ids
            .get(name)
            .map(|&id| &self.blocks[id as usize])
    }

    pub fn block_by_id(&self, id: u32) -> Option<&Block> {
        self.blocks.get(id as usize)
    }

    pub fn state_count(&self) -> u32 {
        self.state_blocks.len() as u32
    }

    pub fn block_of_state(&self, state: u32) -> Option<&Block> {
        self.state_blocks
            .get(state as usize)
            .map(|&id| &self.blocks[id as usize])
    }

    pub fn item_id(&self, name: &str) -> Option<u32> {
        self.item_ids.get(name).copied()
    }

    pub fn item_name(&self, id: u32) -> Option<&str> {
        self.items.get(id as usize).map(String::as_str)
    }

    /// A block entity type's name, such as `minecraft:chest`, from its protocol id.
    pub fn block_entity_type(&self, id: u32) -> Option<&str> {
        self.block_entity_types.get(id as usize).map(String::as_str)
    }

    pub fn item_count(&self) -> u32 {
        self.items.len() as u32
    }
}

/// Names to ids for one registry, checked to run 0..N with no gaps or repeats.
/// Names indexed by id, from a registry's name-to-id map.
fn names_by_id(ids: &HashMap<String, u32>) -> Vec<String> {
    let mut names = vec![String::new(); ids.len()];
    for (name, &id) in ids {
        names[id as usize].clone_from(name);
    }
    names
}

fn protocol_ids(
    registries: &HashMap<String, RegistryReport>,
    registry: &str,
) -> Result<HashMap<String, u32>> {
    let entries = &registries
        .get(registry)
        .with_context(|| format!("no {registry} registry"))?
        .entries;
    let mut seen = vec![false; entries.len()];
    for (name, entry) in entries {
        let slot = seen
            .get_mut(entry.protocol_id as usize)
            .with_context(|| format!("{name} has id {} out of range", entry.protocol_id))?;
        ensure!(!*slot, "{registry} id {} is used twice", entry.protocol_id);
        *slot = true;
    }
    Ok(entries
        .iter()
        .map(|(name, e)| (name.clone(), e.protocol_id))
        .collect())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading {}; run `cargo xtask gen-reports`", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

#[derive(Deserialize)]
struct BlockReport {
    /// Sorted by name, which is the order state ids count through (the report itself
    /// lists properties in the order the block declares them).
    #[serde(default)]
    properties: BTreeMap<String, Vec<String>>,
    states: Vec<StateReport>,
}

#[derive(Deserialize)]
struct StateReport {
    id: u32,
    #[serde(default)]
    default: bool,
}

#[derive(Deserialize)]
struct RegistryReport {
    entries: HashMap<String, EntryReport>,
}

#[derive(Deserialize)]
struct EntryReport {
    protocol_id: u32,
}
