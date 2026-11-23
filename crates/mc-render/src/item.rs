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
    model: Picked,
    tints: Vec<[f32; 3]>,
    /// What it shows while it's being used (`minecraft:using_item`), if that differs.
    using: Option<UsingDef>,
}

/// A model an item's definition picks: the model's id, and for one drawn by code
/// (`minecraft:special`), the special model's type. Most special models are the block's
/// own model, which stands in with this model's transform.
#[derive(Clone)]
struct Picked {
    model: String,
    special: Option<String>,
}

/// `minecraft:using_item`'s true case: a model, or a `range_dispatch` on
/// `minecraft:use_duration` (ticks used, times `scale`) between models by threshold.
struct UsingDef {
    scale: f32,
    entries: Vec<(f32, Picked)>,
    fallback: Picked,
}

/// What an item shows while it's being used: [`UsingDef`] baked.
pub struct UsingIcons {
    scale: f32,
    entries: Vec<(f32, ItemIcon)>,
    fallback: ItemIcon,
}

impl UsingIcons {
    /// `RangeSelectItemModel`: the entry with the highest threshold at or under the
    /// value, `ticks` used times the scale, or the fallback under them all.
    pub fn pick(&self, ticks: i32) -> &ItemIcon {
        let value = ticks as f32 * self.scale;
        self.entries
            .iter()
            .rev()
            .find(|(threshold, _)| *threshold <= value)
            .map_or(&self.fallback, |(_, icon)| icon)
    }
}

/// The shield's texture without patterns (`Sheets.SHIELD_BASE_NO_PATTERN`).
pub const SHIELD: &str = "minecraft:entity/shield/shield_base_nopattern";

/// Every item's definition, the models they name, and the sprites those need in the atlas.
pub(crate) struct Items {
    items: Vec<Loaded>,
    models: HashMap<String, Model>,
    pub sprites: Vec<String>,
}

/// Which model an item shows when nothing about the stack picks another: `select`'s
/// fallback, `condition`'s false case, a range's fallback, a composite's first part.
/// Bundles' selected item and empty models show nothing.
fn resolve(v: &Value) -> Option<(Picked, &[Value])> {
    let kind = v["type"].as_str()?;
    match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "model" => Some((
            Picked {
                model: v["model"].as_str()?.to_owned(),
                special: None,
            },
            v["tints"].as_array().map_or(&[], Vec::as_slice),
        )),
        "special" => {
            let special = v["model"]["type"].as_str().unwrap_or_default();
            let special = special.strip_prefix("minecraft:").unwrap_or(special);
            Some((
                Picked {
                    model: v["base"].as_str()?.to_owned(),
                    special: Some(special.to_owned()),
                },
                &[],
            ))
        }
        "select" => resolve(&v["fallback"]).or_else(|| resolve(&v["cases"][0]["model"])),
        "condition" => resolve(&v["on_false"]),
        "range_dispatch" => resolve(&v["fallback"]).or_else(|| resolve(&v["entries"][0]["model"])),
        "composite" => resolve(&v["models"][0]),
        _ => None,
    }
}

/// A `condition` on `minecraft:using_item`'s true case, as [`UsingDef`].
fn resolve_using(v: &Value) -> Option<UsingDef> {
    let is = |v: &Value, key: &str, want: &str| {
        let s = v[key].as_str().unwrap_or_default();
        s.strip_prefix("minecraft:").unwrap_or(s) == want
    };
    if !is(v, "type", "condition") || !is(v, "property", "using_item") {
        return None;
    }
    let on_true = &v["on_true"];
    if is(on_true, "type", "range_dispatch") && is(on_true, "property", "use_duration") {
        let entries = on_true["entries"]
            .as_array()?
            .iter()
            .filter_map(|e| Some((e["threshold"].as_f64()? as f32, resolve(&e["model"])?.0)))
            .collect::<Vec<_>>();
        let fallback = resolve(&on_true["fallback"])?.0;
        let mut entries = entries;
        entries.sort_by(|a, b| a.0.total_cmp(&b.0));
        return Some(UsingDef {
            scale: on_true["scale"].as_f64().unwrap_or(1.0) as f32,
            entries,
            fallback,
        });
    }
    Some(UsingDef {
        scale: 1.0,
        entries: Vec::new(),
        fallback: resolve(on_true)?.0,
    })
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
        let Some((picked, tints)) = resolve(&json["model"]) else {
            continue;
        };
        let using = resolve_using(&json["model"]);
        let picks = std::iter::once(&picked).chain(
            using
                .iter()
                .flat_map(|u| u.entries.iter().map(|(_, p)| p).chain([&u.fallback])),
        );
        for pick in picks {
            if pick.special.as_deref() == Some("shield") && !sprites.iter().any(|s| s == SHIELD) {
                sprites.push(SHIELD.to_owned());
            }
            if models.contains_key(&pick.model) {
                continue;
            }
            let model = res.model(&pick.model)?;
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
            models.insert(pick.model.clone(), model);
        }
        items.push(Loaded {
            name: name.to_owned(),
            model: picked,
            tints: tints.iter().map(|t| tint(t, &grass)).collect(),
            using,
        });
    }
    Ok(Items {
        items,
        models,
        sprites,
    })
}

/// Each item's icon, by name such as `minecraft:stone`, and what those that change while
/// used show then.
pub(crate) type Icons = (HashMap<String, ItemIcon>, HashMap<String, UsingIcons>);

pub(crate) fn bake(
    items: Items,
    atlas: &Atlas,
    states: &[StateModel],
    registry: &Registry,
) -> Result<Icons> {
    let mut icons = HashMap::new();
    let mut using = HashMap::new();
    let mut baked: HashMap<String, Arc<BakedModel>> = HashMap::new();
    let shield = Arc::new(shield_model(atlas));
    for item in items.items {
        let mut icon = |pick: &Picked| {
            let model = &items.models[&pick.model];
            let found = match pick.special.as_deref() {
                Some("shield") => Some(shield.clone()),
                Some(_) => special_block(&item.name, states, registry),
                None if model.generated => {
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
                    return Some(ItemIcon::Flat {
                        layers,
                        hands: hands(model),
                    });
                }
                None if model.elements.is_some() => Some(
                    baked
                        .entry(pick.model.clone())
                        .or_insert_with(|| {
                            Arc::new(crate::bake::bake_variant(model, &default_variant(), atlas))
                        })
                        .clone(),
                ),
                None => None,
            };
            Some(ItemIcon::Model {
                model: found?,
                tints: item.tints.clone(),
                gui: model.gui.unwrap_or(IDENTITY),
                ground: model.ground.unwrap_or(IDENTITY),
                front_lit: model.gui_light_front.unwrap_or(false),
                hands: hands(model),
            })
        };
        let Some(main) = icon(&item.model) else {
            continue;
        };
        if let Some(u) = &item.using {
            let entries = u
                .entries
                .iter()
                .filter_map(|(threshold, pick)| Some((*threshold, icon(pick)?)))
                .collect();
            if let Some(fallback) = icon(&u.fallback) {
                let icons = UsingIcons {
                    scale: u.scale,
                    entries,
                    fallback,
                };
                using.insert(item.name.clone(), icons);
            }
        }
        icons.insert(item.name, main);
    }
    Ok((icons, using))
}

/// The block's model standing in for a special item model, which draws the block entity:
/// special renderers draw it unturned, which is the block facing south, or at rotation 0;
/// 26.3's banners default to rotation 8.
fn special_block(
    item: &str,
    states: &[StateModel],
    registry: &Registry,
) -> Option<Arc<BakedModel>> {
    let block = registry.block(item)?;
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
    let (_, model) = states[state as usize].parts.first()?.first()?;
    Some(model.clone())
}

/// `ShieldModel` with `ShieldSpecialRenderer`'s plain texture, in item space: its plate
/// and handle, flipped by the item definition's `transformation` (scale 1, -1, -1).
fn shield_model(atlas: &Atlas) -> BakedModel {
    use crate::entity_model::{Builder, Node, cube};
    const PLATE: &[crate::entity_model::Cube] =
        &[cube([-6.0, -11.0, -2.0], [12.0, 22.0, 1.0], [0.0, 0.0])];
    const HANDLE: &[crate::entity_model::Cube] =
        &[cube([-1.0, -3.0, -1.0], [2.0, 6.0, 6.0], [26.0, 0.0])];
    let mut b = Builder {
        quads: Vec::new(),
        sprite: atlas.sprite(SHIELD),
        tex_size: [64.0, 64.0],
        tint: -1,
    };
    let nodes = [PLATE, HANDLE].map(|cubes| Node {
        parent: None,
        pose: [0.0; 6],
        cubes,
    });
    b.nodes(
        glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, -1.0)),
        &nodes,
    );
    BakedModel {
        quads: b.quads,
        ambient_occlusion: false,
    }
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
