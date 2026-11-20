//! Item icons: how each item looks in an inventory slot. An item's definition in
//! `assets/minecraft/items` names a model; a model generated from sprites is drawn flat, and
//! one with elements, as block items have, is drawn in 3D with its `display.gui` transform.
//! Items whose model is drawn by code (chests, banners, heads) reuse the block's model.

use crate::atlas::Atlas;
use crate::bake::{BakedModel, StateModel};
use anyhow::{Context, Result};
use mc_assets::{Model, Resources, Transform, Variant};
use mc_registry::Registry;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub enum ItemIcon {
    /// Sprite layers drawn one over another, each as its atlas rect and tint.
    Flat {
        layers: Vec<([f32; 4], [f32; 3])>,
        /// How it sits in a mob's right and left hand, then in the player's own as they
        /// see it.
        hands: [Transform; 4],
    },
    /// A model drawn in 3D, with a colour per tint index.
    Model {
        model: Arc<BakedModel>,
        tints: Vec<[f32; 3]>,
        gui: Transform,
        /// How it sits on the ground when dropped.
        ground: Transform,
        /// Lit from the front (`gui_light: front`) rather than the side.
        front_lit: bool,
        hands: [Transform; 4],
    },
}

impl ItemIcon {
    /// `ItemTransforms`' third person transform for a right (`false`) or left hand.
    pub fn hand(&self, left: bool) -> Transform {
        let (ItemIcon::Flat { hands, .. } | ItemIcon::Model { hands, .. }) = self;
        hands[left as usize]
    }

    /// Its first person transform for a right (`false`) or left hand.
    pub fn first_person_hand(&self, left: bool) -> Transform {
        let (ItemIcon::Flat { hands, .. } | ItemIcon::Model { hands, .. }) = self;
        hands[2 + left as usize]
    }
}

/// `ItemTransform.NO_TRANSFORM`.
const IDENTITY: Transform = Transform {
    rotation: [0.0; 3],
    translation: [0.0; 3],
    scale: [1.0; 3],
};

/// A model's hand transforms, third person then first: a left hand's is the right hand's
/// when it has none (`ItemTransforms.Deserializer`).
fn hands(model: &Model) -> [Transform; 4] {
    let [right, first_right] = [0, 2].map(|i| model.hands[i].unwrap_or(IDENTITY));
    [
        right,
        model.hands[1].unwrap_or(right),
        first_right,
        model.hands[3].unwrap_or(first_right),
    ]
}

/// An item's definition, resolved to one model, before the atlas exists.
pub(crate) struct Loaded {
    name: String,
    model: String,
    tints: Vec<[f32; 3]>,
    /// Drawn by code: the block's own model stands in, with this model's transform.
    special: bool,
}

/// Every item's definition, the models they name, and the sprites those need in the atlas.
pub(crate) struct Items {
    items: Vec<Loaded>,
    models: HashMap<String, Model>,
    pub sprites: Vec<String>,
}

/// Which model an item shows when nothing about the stack picks another: `select`'s
/// fallback, `condition`'s false case, a range's fallback, a composite's first part.
/// Bundles' selected item and empty models show nothing.
fn resolve(v: &Value) -> Option<(&str, &[Value], bool)> {
    let kind = v["type"].as_str()?;
    match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "model" => Some((
            v["model"].as_str()?,
            v["tints"].as_array().map_or(&[], Vec::as_slice),
            false,
        )),
        "special" => Some((v["base"].as_str()?, &[], true)),
        "select" => resolve(&v["fallback"]).or_else(|| resolve(&v["cases"][0]["model"])),
        "condition" => resolve(&v["on_false"]),
        "range_dispatch" => resolve(&v["fallback"]).or_else(|| resolve(&v["entries"][0]["model"])),
        "composite" => resolve(&v["models"][0]),
        _ => None,
    }
}

/// An item tint's colour with nothing about the stack known: a constant's value, grass at
/// the given climate, or the source's `default`.
fn tint(v: &Value, grass: &image::RgbaImage) -> [f32; 3] {
    let kind = v["type"].as_str().unwrap_or_default();
    let color = |c: &Value| match c {
        Value::Array(rgb) => std::array::from_fn(|i| rgb[i].as_f64().unwrap_or(1.0) as f32),
        Value::Number(n) => crate::tint::rgb(n.as_i64().unwrap_or(-1) as u32),
        _ => [1.0; 3],
    };
    match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "constant" => color(&v["value"]),
        "grass" => crate::tint::lookup(
            grass,
            v["temperature"].as_f64().unwrap_or(0.5) as f32,
            v["downfall"].as_f64().unwrap_or(1.0) as f32,
        ),
        _ => color(&v["default"]),
    }
}

pub(crate) fn load(res: &Resources, registry: &Registry) -> Result<Items> {
    let grass = crate::tint::colormap(res, "grass")?;
    let mut items = Vec::new();
    let mut sprites = Vec::new();
    let mut models: HashMap<String, Model> = HashMap::new();
    for id in 0..registry.item_count() {
        let Some(name) = registry.item_name(id) else {
            continue;
        };
        let path = res.asset_path("items", name, "json");
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let json: Value =
            serde_json::from_slice(&bytes).with_context(|| format!("{}", path.display()))?;
        let Some((model_id, tints, special)) = resolve(&json["model"]) else {
            continue;
        };
        if !models.contains_key(model_id) {
            let model = res.model(model_id)?;
            if model.generated {
                sprites.extend(
                    (0..).map_while(|i| model.sprite(&format!("layer{i}")).map(str::to_owned)),
                );
            }
            for element in model.elements.iter().flatten() {
                for face in element.faces.values() {
                    sprites.extend(model.sprite(&face.texture).map(str::to_owned));
                }
            }
            models.insert(model_id.to_owned(), model);
        }
        items.push(Loaded {
            name: name.to_owned(),
            model: model_id.to_owned(),
            tints: tints.iter().map(|t| tint(t, &grass)).collect(),
            special,
        });
    }
    Ok(Items {
        items,
        models,
        sprites,
    })
}

/// Each item's icon, by name such as `minecraft:stone`.
pub(crate) fn bake(
    items: Items,
    atlas: &Atlas,
    states: &[StateModel],
    registry: &Registry,
) -> Result<HashMap<String, ItemIcon>> {
    let mut icons = HashMap::new();
    let mut baked: HashMap<String, Arc<BakedModel>> = HashMap::new();
    let identity = IDENTITY;
    for item in items.items {
        let model = &items.models[&item.model];
        let icon = if item.special {
            // The block's model, which draws the block entity, sits where the base says.
            let Some(block) = registry.block(&item.name) else {
                continue;
            };
            // Special renderers draw the block entity's model unturned, which is the block
            // facing south, or at rotation 0; 26.3's banners default to rotation 8.
            let values = block.property_values(block.default_state);
            let facing = values
                .iter()
                .find(|(name, _)| *name == "facing")
                .map(|&(_, value)| value);
            let horizontal = matches!(facing, Some("north" | "south" | "east" | "west"));
            let state = block.state(|p| match p {
                "facing" if horizontal => Some("south"),
                "rotation" => Some("0"),
                _ => None,
            });
            let Some((_, block_model)) =
                states[state as usize].parts.first().and_then(|p| p.first())
            else {
                continue;
            };
            ItemIcon::Model {
                model: block_model.clone(),
                tints: item.tints,
                gui: model.gui.unwrap_or(identity),
                ground: model.ground.unwrap_or(identity),
                front_lit: model.gui_light_front.unwrap_or(false),
                hands: hands(model),
            }
        } else if model.generated {
            let layers = (0..)
                .map_while(|i| model.sprite(&format!("layer{i}")))
                .enumerate()
                .map(|(i, sprite)| {
                    (
                        atlas.sprite(sprite).rect,
                        item.tints.get(i).copied().unwrap_or([1.0; 3]),
                    )
                })
                .collect();
            ItemIcon::Flat {
                layers,
                hands: hands(model),
            }
        } else if model.elements.is_some() {
            let model_baked = baked
                .entry(item.model.clone())
                .or_insert_with(|| {
                    Arc::new(crate::bake::bake_variant(model, &default_variant(), atlas))
                })
                .clone();
            ItemIcon::Model {
                model: model_baked,
                tints: item.tints,
                gui: model.gui.unwrap_or(identity),
                ground: model.ground.unwrap_or(identity),
                front_lit: model.gui_light_front.unwrap_or(false),
                hands: hands(model),
            }
        } else {
            continue;
        };
        icons.insert(item.name, icon);
    }
    Ok(icons)
}

fn default_variant() -> Variant {
    Variant {
        model: String::new(),
        x: 0,
        y: 0,
        uvlock: false,
        weight: 1,
    }
}
