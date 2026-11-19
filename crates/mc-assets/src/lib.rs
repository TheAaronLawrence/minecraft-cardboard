//! Reads the vanilla resources committed under `resources/`: blockstates, block models
//! (with their parent chains flattened) and texture files.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The purple and black sprite the game draws in code; it has no texture file.
pub const MISSING_SPRITE: &str = "minecraft:missingno";

/// The `resources/` folder: beside the executable in a portable build, otherwise at the
/// repository root.
pub fn default_root() -> PathBuf {
    locate("resources")
}

/// A path relative to the repository root, such as `resources/reports`. A portable build
/// ships these folders beside its executable, so that copy wins when it exists; otherwise
/// it's the repository this was built from.
pub fn locate(relative: &str) -> PathBuf {
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(relative)));
    match beside_exe {
        Some(path) if path.exists() => path,
        _ => Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative),
    }
}

pub struct Resources {
    root: PathBuf,
}

impl Resources {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `kind` is the folder under `assets/<namespace>/`, such as `models` or `textures`.
    pub fn asset_path(&self, kind: &str, id: &str, extension: &str) -> PathBuf {
        let (namespace, path) = split_id(id);
        self.root
            .join("assets")
            .join(namespace)
            .join(kind)
            .join(format!("{path}.{extension}"))
    }

    /// `kind` is the folder under `data/<namespace>/`, such as `worldgen/biome`.
    pub fn data_path(&self, kind: &str, id: &str, extension: &str) -> PathBuf {
        let (namespace, path) = split_id(id);
        self.root
            .join("data")
            .join(namespace)
            .join(kind)
            .join(format!("{path}.{extension}"))
    }

    /// Names of every block with a blockstate file, such as `oak_log`.
    pub fn block_names(&self) -> Result<Vec<String>> {
        let dir = self.root.join("assets/minecraft/blockstates");
        let mut names = std::fs::read_dir(&dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .map(|e| {
                Ok(e?
                    .path()
                    .file_stem()
                    .context("no file name")?
                    .to_string_lossy()
                    .into_owned())
            })
            .collect::<Result<Vec<_>>>()?;
        names.sort();
        Ok(names)
    }

    pub fn blockstate(&self, id: &str) -> Result<BlockState> {
        read_json(&self.asset_path("blockstates", id, "json"))
    }

    /// Loads a model and merges its parents into it: the child's textures override the
    /// parent's, and the elements come from the nearest model that defines any.
    pub fn model(&self, id: &str) -> Result<Model> {
        let mut model = Model {
            textures: HashMap::new(),
            elements: None,
            ambient_occlusion: None,
            generated: false,
        };
        let mut next = Some(id.to_owned());
        let mut depth = 0;
        while let Some(id) = next.take() {
            if id == "builtin/generated" {
                model.generated = true;
                break;
            }
            depth += 1;
            anyhow::ensure!(
                depth <= 32,
                "model parent chain of {id} is too deep or circular"
            );
            let raw: RawModel = read_json(&self.asset_path("models", &id, "json"))
                .with_context(|| format!("loading model {id}"))?;
            for (key, value) in raw.textures {
                model.textures.entry(key).or_insert(value);
            }
            if model.elements.is_none() {
                model.elements = raw.elements;
            }
            model.ambient_occlusion = model.ambient_occlusion.or(raw.ambientocclusion);
            next = raw.parent;
        }
        Ok(model)
    }
}

/// `minecraft:block/stone` and `block/stone` both mean namespace `minecraft`, path `block/stone`.
pub fn split_id(id: &str) -> (&str, &str) {
    id.split_once(':').unwrap_or(("minecraft", id))
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockState {
    /// Property string such as `axis=x,lit=true` (empty for blocks with one state) to its variants.
    Variants(HashMap<String, Variants>),
    Multipart(Vec<MultipartCase>),
}

impl BlockState {
    pub fn all_variants(&self) -> Vec<&Variant> {
        match self {
            BlockState::Variants(map) => map.values().flat_map(Variants::as_slice).collect(),
            BlockState::Multipart(cases) => cases.iter().flat_map(|c| c.apply.as_slice()).collect(),
        }
    }

    /// The models a block state draws. `property` gives the state's value for a property
    /// name. A `variants` file should match exactly one entry; a `multipart` file draws
    /// every case whose condition holds.
    pub fn select<'a, 'p>(
        &'a self,
        property: &dyn Fn(&str) -> Option<&'p str>,
    ) -> Vec<&'a Variants> {
        match self {
            BlockState::Variants(map) => map
                .iter()
                .filter(|(key, _)| {
                    key.split(',').filter(|pair| !pair.is_empty()).all(|pair| {
                        pair.split_once('=')
                            .is_some_and(|(k, v)| property(k) == Some(v))
                    })
                })
                .map(|(_, variants)| variants)
                .collect(),
            BlockState::Multipart(cases) => cases
                .iter()
                .filter(|case| {
                    case.when
                        .as_ref()
                        .is_none_or(|when| condition_holds(when, property))
                })
                .map(|case| &case.apply)
                .collect(),
        }
    }
}

/// A multipart `when`: `{"OR": [...]}`, `{"AND": [...]}`, or property values that must
/// all match, where `"a|b"` allows either value and a leading `!` negates the list.
fn condition_holds<'p>(
    when: &serde_json::Value,
    property: &dyn Fn(&str) -> Option<&'p str>,
) -> bool {
    let Some(map) = when.as_object() else {
        return false;
    };
    let all = |list: &serde_json::Value| {
        list.as_array()
            .is_some_and(|l| l.iter().all(|c| condition_holds(c, property)))
    };
    let any = |list: &serde_json::Value| {
        list.as_array()
            .is_some_and(|l| l.iter().any(|c| condition_holds(c, property)))
    };
    if let Some(list) = map.get("OR") {
        return any(list);
    }
    if let Some(list) = map.get("AND") {
        return all(list);
    }
    map.iter().all(|(name, expected)| {
        let Some(expected) = expected.as_str() else {
            return false;
        };
        let (negate, values) = expected
            .strip_prefix('!')
            .map_or((false, expected), |rest| (true, rest));
        let Some(actual) = property(name) else {
            return false;
        };
        values.split('|').any(|v| v == actual) != negate
    })
}

#[derive(Debug, Deserialize)]
pub struct MultipartCase {
    /// Kept raw for now: an object of property conditions, possibly nested in `OR`/`AND`.
    #[serde(default)]
    pub when: Option<serde_json::Value>,
    pub apply: Variants,
}

/// One variant, or a weighted list the game picks from at random by position.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Variants {
    One(Variant),
    Weighted(Vec<Variant>),
}

impl Variants {
    pub fn as_slice(&self) -> &[Variant] {
        match self {
            Variants::One(v) => std::slice::from_ref(v),
            Variants::Weighted(list) => list,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Variant {
    pub model: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub uvlock: bool,
    #[serde(default = "one")]
    pub weight: u32,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize)]
struct RawModel {
    parent: Option<String>,
    #[serde(default)]
    textures: HashMap<String, TextureRef>,
    elements: Option<Vec<Element>>,
    ambientocclusion: Option<bool>,
}

/// A model with its parent chain merged in.
#[derive(Debug)]
pub struct Model {
    /// Texture variables, such as `side`, to a sprite or to another variable (`#end`).
    pub textures: HashMap<String, TextureRef>,
    /// `None` for models drawn by code (chests, signs) or generated from a sprite (items).
    pub elements: Option<Vec<Element>>,
    pub ambient_occlusion: Option<bool>,
    /// Extruded from its `layer0`, `layer1`… sprites, like most item models.
    pub generated: bool,
}

impl Model {
    /// Resolves a texture slot, such as a face's `texture`, to a sprite id such as
    /// `minecraft:block/oak_log`. Like vanilla, the slot's leading `#` is optional
    /// (`heavy_core` writes `"all"`), and slot values starting with `#` point at other slots.
    pub fn sprite(&self, slot: &str) -> Option<&str> {
        let mut slot = slot.strip_prefix('#').unwrap_or(slot);
        for _ in 0..=self.textures.len() {
            let value = self.textures.get(slot)?.sprite();
            match value.strip_prefix('#') {
                Some(next) => slot = next,
                None => return Some(value),
            }
        }
        None
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum TextureRef {
    Plain(String),
    Detailed {
        sprite: String,
        #[serde(default)]
        force_translucent: bool,
    },
}

impl TextureRef {
    pub fn sprite(&self) -> &str {
        match self {
            TextureRef::Plain(s) | TextureRef::Detailed { sprite: s, .. } => s,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct Element {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub rotation: Option<ElementRotation>,
    #[serde(default = "yes")]
    pub shade: bool,
    pub faces: HashMap<Direction, Face>,
}

fn yes() -> bool {
    true
}

/// Angles are in degrees, around `origin` in model units (0..16).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
pub enum ElementRotation {
    /// The classic form. `rescale` stretches the element so a 22.5° or 45° face still spans
    /// the block, as crosses (grass, flowers) do.
    Axis {
        origin: [f32; 3],
        axis: Axis,
        angle: f32,
        #[serde(default)]
        rescale: bool,
    },
    /// Newer models: rotations about x, then y, then z.
    Euler {
        origin: [f32; 3],
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default)]
        z: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

#[derive(Debug, Deserialize)]
pub struct Face {
    pub texture: String,
    pub uv: Option<[f32; 4]>,
    pub cullface: Option<Direction>,
    #[serde(default)]
    pub rotation: i32,
    #[serde(default = "no_tint")]
    pub tintindex: i32,
}

fn no_tint() -> i32 {
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negated_conditions_exclude_every_listed_value() {
        // Resource packs may write `!`; vanilla's own files don't, so test it here.
        let state: BlockState = serde_json::from_str(
            r#"{"multipart": [
            {"when": {"facing": "!north|south"}, "apply": {"model": "minecraft:block/side"}}
        ]}"#,
        )
        .unwrap();
        let picks = |facing: &'static str| {
            state
                .select(&move |p| (p == "facing").then_some(facing))
                .len()
        };
        assert_eq!([picks("north"), picks("south"), picks("east")], [0, 0, 1]);
    }
}
