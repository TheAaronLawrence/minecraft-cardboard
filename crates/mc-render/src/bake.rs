//! Turns every block state's models into quads in block space (0..1), ready to mesh.

use crate::atlas::{Atlas, Layer};
use anyhow::{Context, Result};
use glam::{Mat3, Quat, Vec3};
use mc_assets::{Axis, BlockState, Direction, ElementRotation, Model, Resources, Variant};
use mc_registry::Registry;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The six faces of a block, as vanilla orders them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Dir {
    pub const ALL: [Dir; 6] = [
        Dir::Down,
        Dir::Up,
        Dir::North,
        Dir::South,
        Dir::West,
        Dir::East,
    ];

    pub fn offset(self) -> [i32; 3] {
        match self {
            Dir::Down => [0, -1, 0],
            Dir::Up => [0, 1, 0],
            Dir::North => [0, 0, -1],
            Dir::South => [0, 0, 1],
            Dir::West => [-1, 0, 0],
            Dir::East => [1, 0, 0],
        }
    }

    pub(crate) fn normal(self) -> Vec3 {
        let [x, y, z] = self.offset();
        Vec3::new(x as f32, y as f32, z as f32)
    }

    /// The face a direction points at most closely.
    pub(crate) fn nearest(v: Vec3) -> Dir {
        *Dir::ALL
            .iter()
            .max_by(|a, b| a.normal().dot(v).total_cmp(&b.normal().dot(v)))
            .expect("six directions")
    }

    /// Vanilla's directional shading, so faces read as 3D even in flat light.
    pub fn shade(self) -> f32 {
        match self {
            Dir::Down => 0.5,
            Dir::Up => 1.0,
            Dir::North | Dir::South => 0.8,
            Dir::West | Dir::East => 0.6,
        }
    }
}

impl From<Direction> for Dir {
    fn from(d: Direction) -> Self {
        match d {
            Direction::Down => Dir::Down,
            Direction::Up => Dir::Up,
            Direction::North => Dir::North,
            Direction::South => Dir::South,
            Direction::West => Dir::West,
            Direction::East => Dir::East,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Quad {
    /// Corners in block space, in vanilla's vertex order for the face.
    pub pos: [Vec3; 4],
    /// Atlas coordinates per corner.
    pub uv: [[f32; 2]; 4],
    /// Which way the quad faces, for shading and light.
    pub dir: Dir,
    /// Skip the quad when the neighbor this way hides it.
    pub cull: Option<Dir>,
    /// Which tint the block applies (grass, foliage, water), if any.
    pub tint: i32,
    /// The direction whose shading the quad takes instead of its own
    /// (`shade_direction_override`).
    pub shade: Option<Dir>,
    pub layer: Layer,
    /// Lies on the block's outer face in `dir`, so its light comes from the neighbor.
    pub on_face: bool,
}

#[derive(Debug, Default)]
pub struct BakedModel {
    pub quads: Vec<Quad>,
    pub ambient_occlusion: bool,
}

/// One state's models: each part is a weighted list the game picks one model from.
#[derive(Debug, Default)]
pub struct StateModel {
    pub parts: Vec<Vec<(u32, Arc<BakedModel>)>>,
    /// A full opaque cube, which hides its neighbors' faces and darkens their corners.
    pub occludes: bool,
}

pub struct BlockModels {
    /// Indexed by state id.
    pub states: Vec<StateModel>,
    pub atlas: Atlas,
    /// Each item's icon, by name such as `minecraft:stone`, and the HUD's sprites.
    pub hud: crate::hud::HudArt,
    /// The end portal shader's own textures, which it tiles, so they aren't in the atlas.
    pub portal_textures: PortalTextures,
    /// Armour's layers, by equipment asset (`entity::equipment_assets`).
    pub equipment: HashMap<String, crate::entity::EquipmentLayers>,
}

/// `TheEndPortalRenderer`'s two textures: the sky behind, and the stars on every layer.
pub struct PortalTextures {
    pub sky: image::RgbaImage,
    pub stars: image::RgbaImage,
}

impl PortalTextures {
    fn load(res: &Resources) -> Result<Self> {
        let load = |id: &str| -> Result<image::RgbaImage> {
            let path = res.asset_path("textures", id, "png");
            Ok(image::open(&path)
                .with_context(|| format!("{}", path.display()))?
                .to_rgba8())
        };
        Ok(Self {
            sky: load("minecraft:environment/end_sky")?,
            stars: load("minecraft:entity/end_portal/end_portal")?,
        })
    }
}

/// Sprites the renderer draws itself, not through block models.
pub const WATER_SPRITE: &str = "minecraft:block/water_still";
pub const LAVA_SPRITE: &str = "minecraft:block/lava_still";
pub const WATER_FLOW_SPRITE: &str = "minecraft:block/water_flow";
pub const LAVA_FLOW_SPRITE: &str = "minecraft:block/lava_flow";

impl BlockModels {
    /// `extra_sprites` are images to add to the atlas under their ids, such as the skins of
    /// the world's player heads (see `entity_model::skin_sprite`).
    pub fn bake(
        res: &Resources,
        registry: &Registry,
        extra_sprites: &[(String, image::RgbaImage)],
    ) -> Result<Self> {
        // Load each block's blockstate file and every model it can pick.
        let mut files = Vec::new();
        let mut models: HashMap<String, Model> = HashMap::new();
        let mut sprites: HashSet<String> = [
            WATER_SPRITE,
            LAVA_SPRITE,
            WATER_FLOW_SPRITE,
            LAVA_FLOW_SPRITE,
        ]
        .map(str::to_owned)
        .into();
        for block in registry.blocks() {
            let file = res
                .blockstate(&block.name)
                .with_context(|| format!("blockstate of {}", block.name))?;
            for variant in file.all_variants() {
                if !models.contains_key(&variant.model) {
                    let model = res.model(&variant.model)?;
                    for element in model.elements.iter().flatten() {
                        for face in element.faces.values() {
                            sprites.extend(model.sprite(&face.texture).map(str::to_owned));
                        }
                    }
                    models.insert(variant.model.clone(), model);
                }
            }
            files.push(file);
        }
        sprites.extend(crate::chest::SPRITES.iter().map(|s| s.to_string()));
        let items = crate::item::load(res, registry)?;
        sprites.extend(items.sprites.iter().cloned());
        sprites.extend(crate::hud::sprites());
        sprites.extend(crate::entity_model::sprites(res));
        let (equipment, equipment_sprites) = crate::entity::equipment_assets(res)?;
        sprites.extend(equipment_sprites);
        let mut extra = extra_sprites.to_vec();
        extra.extend(crate::entity::skin_slots());
        let atlas = Atlas::build(res, sprites, &extra)?;

        let mut baked: HashMap<(String, i32, i32, bool), Arc<BakedModel>> = HashMap::new();
        let mut states = Vec::with_capacity(registry.state_count() as usize);
        states.resize_with(registry.state_count() as usize, StateModel::default);
        for (block, file) in registry.blocks().iter().zip(&files) {
            for state in block.first_state..block.first_state + block.state_count() {
                let values = block.property_values(state);
                let selected =
                    file.select(&|p| values.iter().find(|(n, _)| *n == p).map(|(_, v)| *v));
                let parts: Vec<Vec<(u32, Arc<BakedModel>)>> = selected
                    .iter()
                    .map(|variants| {
                        variants
                            .as_slice()
                            .iter()
                            .map(|v| {
                                let key = (v.model.clone(), v.x, v.y, v.uvlock);
                                let model = baked
                                    .entry(key)
                                    .or_insert_with(|| {
                                        Arc::new(bake_variant(&models[&v.model], v, &atlas))
                                    })
                                    .clone();
                                (v.weight, model)
                            })
                            .collect()
                    })
                    .collect();
                let occludes = matches!(file, BlockState::Variants(_))
                    && parts.len() == 1
                    && parts[0].iter().all(|(_, m)| is_full_opaque_cube(m));
                let entity = crate::chest::model(&block.name, &values, &atlas)
                    .or_else(|| crate::entity_model::model(&block.name, &values, &atlas));
                states[state as usize] = match entity {
                    Some(model) => StateModel {
                        parts: vec![vec![(1, Arc::new(model))]],
                        occludes: false,
                    },
                    None => StateModel { parts, occludes },
                };
            }
        }
        let icons = crate::item::bake(items, &atlas, &states, registry)?;
        let hud = crate::hud::HudArt::new(res, &atlas, icons, crate::hud::item_names()?)?;
        Ok(Self {
            states,
            atlas,
            hud,
            portal_textures: PortalTextures::load(res)?,
            equipment,
        })
    }
}

impl BlockModels {
    /// Just the GUI's sprites, with no blocks or items, for drawing menus from a few files,
    /// such as the launcher's before the game is installed. Sprites without a file in `res`
    /// are the missing texture.
    pub fn gui(res: &Resources) -> Result<Self> {
        let atlas = Atlas::build(res, crate::hud::sprites(), &[])?;
        let hud = crate::hud::HudArt::new(res, &atlas, HashMap::new(), HashMap::new())?;
        let blank = || image::RgbaImage::new(16, 16);
        Ok(Self {
            states: Vec::new(),
            atlas,
            hud,
            portal_textures: PortalTextures {
                sky: blank(),
                stars: blank(),
            },
            equipment: HashMap::new(),
        })
    }
}

/// Six opaque faces covering the whole block, each culled by the neighbor it faces.
fn is_full_opaque_cube(model: &BakedModel) -> bool {
    Dir::ALL.iter().all(|&d| {
        model
            .quads
            .iter()
            .any(|q| q.cull == Some(d) && q.on_face && q.layer == Layer::Opaque && covers_face(q))
    }) && model.quads.iter().all(|q| q.layer == Layer::Opaque)
}

fn covers_face(q: &Quad) -> bool {
    let min = q.pos.iter().copied().reduce(Vec3::min).unwrap();
    let max = q.pos.iter().copied().reduce(Vec3::max).unwrap();
    let span = max - min;
    let full = |v: f32| (v - 1.0).abs() < 1e-4;
    match q.dir {
        Dir::Down | Dir::Up => full(span.x) && full(span.z),
        Dir::North | Dir::South => full(span.x) && full(span.y),
        Dir::West | Dir::East => full(span.z) && full(span.y),
    }
}

pub(crate) fn bake_variant(model: &Model, variant: &Variant, atlas: &Atlas) -> BakedModel {
    // Vanilla rotates by -x degrees about X, then -y degrees about Y, around the block centre.
    let rotation = Quat::from_rotation_y((-variant.y as f32).to_radians())
        * Quat::from_rotation_x((-variant.x as f32).to_radians());
    let rotated = variant.x != 0 || variant.y != 0;
    let center = Vec3::splat(0.5);
    let mut quads = Vec::new();
    for element in model.elements.iter().flatten() {
        let from = Vec3::from(element.from) / 16.0;
        let to = Vec3::from(element.to) / 16.0;
        for (&direction, face) in &element.faces {
            let face_dir = Dir::from(direction);
            let Some(sprite_id) = model.sprite(&face.texture) else {
                continue;
            };
            let sprite = atlas.sprite(sprite_id);
            let forced = model
                .textures
                .get(face.texture.trim_start_matches('#'))
                .is_some_and(|t| {
                    matches!(
                        t,
                        mc_assets::TextureRef::Detailed {
                            force_translucent: true,
                            ..
                        }
                    )
                });

            let mut pos = corners(face_dir, from, to);
            if let Some(r) = element.rotation {
                for p in &mut pos {
                    *p = rotate_element(*p, r);
                }
            }
            for p in &mut pos {
                *p = snap(rotation * (*p - center) + center);
            }
            let dir = Dir::nearest(rotation * element_normal(face_dir, element.rotation));

            // UV lock keeps the texture lined up with the world instead of the model, as
            // vanilla does for stairs and walls; recompute the default UVs on the rotated face.
            let uv_rect = match face.uv {
                Some(uv) if !(variant.uvlock && rotated) => uv,
                _ => {
                    let (min, max) = (
                        pos.iter().copied().reduce(Vec3::min).unwrap(),
                        pos.iter().copied().reduce(Vec3::max).unwrap(),
                    );
                    let rect_dir = if variant.uvlock && rotated {
                        dir
                    } else {
                        face_dir
                    };
                    let (min, max) = if variant.uvlock && rotated {
                        (min, max)
                    } else {
                        (from, to)
                    };
                    default_uv(rect_dir, min * 16.0, max * 16.0)
                }
            };
            let [u0, v0, u1, v1] = uv_rect.map(|c| c / 16.0);
            let [su0, sv0, su1, sv1] = sprite.rect;
            let corner_uv = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]];
            let shift = (face.rotation.rem_euclid(360) / 90) as usize;
            let uv = std::array::from_fn(|i| {
                let [u, v] = corner_uv[(i + shift) % 4];
                [su0 + (su1 - su0) * u, sv0 + (sv1 - sv0) * v]
            });

            let cull = face
                .cullface
                .map(|c| Dir::nearest(rotation * Dir::from(c).normal()));
            let on_face = lies_on_face(&pos, dir);
            let layer = if forced {
                Layer::Translucent
            } else {
                sprite.layer
            };
            quads.push(Quad {
                pos,
                uv,
                dir,
                cull,
                tint: face.tintindex,
                shade: element.shade_direction_override.map(Dir::from),
                layer,
                on_face,
            });
        }
    }
    BakedModel {
        quads,
        ambient_occlusion: model.ambient_occlusion.unwrap_or(true),
    }
}

/// Whether a quad lies in the block's outer face on the `dir` side.
pub(crate) fn lies_on_face(pos: &[Vec3; 4], dir: Dir) -> bool {
    let edge = if dir.normal().max_element() > 0.0 {
        1.0
    } else {
        0.0
    };
    pos.iter()
        .all(|p| (p.dot(dir.normal()).abs() - edge).abs() < 1e-4)
}

/// Rotating by quarter turns leaves float error, such as -0.0000001 for 0; put coordinates
/// that close to a 1/16 step back on it, so neighbouring blocks' faces meet without cracks.
pub(crate) fn snap(p: Vec3) -> Vec3 {
    p.map(|c| {
        let steps = (c * 16.0).round();
        if (c * 16.0 - steps).abs() < 1e-3 {
            steps / 16.0
        } else {
            c
        }
    })
}

/// The four corners of an element's face, in vanilla's order (which the UV corners follow).
pub fn corners(dir: Dir, min: Vec3, max: Vec3) -> [Vec3; 4] {
    let v = Vec3::new;
    match dir {
        Dir::Down => [
            v(min.x, min.y, max.z),
            v(min.x, min.y, min.z),
            v(max.x, min.y, min.z),
            v(max.x, min.y, max.z),
        ],
        Dir::Up => [
            v(min.x, max.y, min.z),
            v(min.x, max.y, max.z),
            v(max.x, max.y, max.z),
            v(max.x, max.y, min.z),
        ],
        Dir::North => [
            v(max.x, max.y, min.z),
            v(max.x, min.y, min.z),
            v(min.x, min.y, min.z),
            v(min.x, max.y, min.z),
        ],
        Dir::South => [
            v(min.x, max.y, max.z),
            v(min.x, min.y, max.z),
            v(max.x, min.y, max.z),
            v(max.x, max.y, max.z),
        ],
        Dir::West => [
            v(min.x, max.y, min.z),
            v(min.x, min.y, min.z),
            v(min.x, min.y, max.z),
            v(min.x, max.y, max.z),
        ],
        Dir::East => [
            v(max.x, max.y, max.z),
            v(max.x, min.y, max.z),
            v(max.x, min.y, min.z),
            v(max.x, max.y, min.z),
        ],
    }
}

/// The UVs vanilla gives a face that doesn't list its own, in model units (0..16).
fn default_uv(dir: Dir, from: Vec3, to: Vec3) -> [f32; 4] {
    match dir {
        Dir::Down => [from.x, 16.0 - to.z, to.x, 16.0 - from.z],
        Dir::Up => [from.x, from.z, to.x, to.z],
        Dir::North => [16.0 - to.x, 16.0 - to.y, 16.0 - from.x, 16.0 - from.y],
        Dir::South => [from.x, 16.0 - to.y, to.x, 16.0 - from.y],
        Dir::West => [from.z, 16.0 - to.y, to.z, 16.0 - from.y],
        Dir::East => [16.0 - to.z, 16.0 - to.y, 16.0 - from.z, 16.0 - from.y],
    }
}

fn element_rotation(r: ElementRotation) -> (Vec3, Mat3, Vec3) {
    match r {
        ElementRotation::Axis {
            origin,
            axis,
            angle,
            rescale,
        } => {
            let (rot, scale_axes) = match axis {
                Axis::X => (
                    Mat3::from_rotation_x(angle.to_radians()),
                    Vec3::new(0.0, 1.0, 1.0),
                ),
                Axis::Y => (
                    Mat3::from_rotation_y(angle.to_radians()),
                    Vec3::new(1.0, 0.0, 1.0),
                ),
                Axis::Z => (
                    Mat3::from_rotation_z(angle.to_radians()),
                    Vec3::new(1.0, 1.0, 0.0),
                ),
            };
            // Stretch across the two axes the rotation tilts, by 1/cos(angle).
            let scale = if rescale {
                Vec3::ONE + scale_axes * (1.0 / angle.to_radians().cos() - 1.0)
            } else {
                Vec3::ONE
            };
            (Vec3::from(origin) / 16.0, rot, scale)
        }
        ElementRotation::Euler { origin, x, y, z } => {
            let rot = Mat3::from_quat(Quat::from_euler(
                glam::EulerRot::XYZ,
                x.to_radians(),
                y.to_radians(),
                z.to_radians(),
            ));
            (Vec3::from(origin) / 16.0, rot, Vec3::ONE)
        }
    }
}

fn rotate_element(p: Vec3, r: ElementRotation) -> Vec3 {
    let (origin, rot, scale) = element_rotation(r);
    rot * ((p - origin) * scale) + origin
}

fn element_normal(dir: Dir, r: Option<ElementRotation>) -> Vec3 {
    r.map_or(dir.normal(), |r| element_rotation(r).1 * dir.normal())
}

/// Vanilla's per-position random choice among weighted models, so the same block gets the
/// same rotation of grass or stone as in the game. `seed` is [`position_seed`].
pub fn pick<'a>(part: &'a [(u32, Arc<BakedModel>)], random: &mut JavaRandom) -> &'a BakedModel {
    if part.len() == 1 {
        return &part[0].1;
    }
    let total: u32 = part.iter().map(|(w, _)| w).sum();
    let mut roll = random.next_int(total as i32) as u32;
    for (weight, model) in part {
        if roll < *weight {
            return model;
        }
        roll -= weight;
    }
    &part[part.len() - 1].1
}

/// `Mth.getSeed`: the seed vanilla derives from a block position. Java multiplies `x` as an
/// `int`, so it wraps before widening.
pub fn position_seed(x: i32, y: i32, z: i32) -> i64 {
    let mut l = i64::from(x.wrapping_mul(3_129_871))
        ^ i64::from(z).wrapping_mul(116_129_781)
        ^ i64::from(y);
    l = l
        .wrapping_mul(l)
        .wrapping_mul(42_317_861)
        .wrapping_add(l.wrapping_mul(11));
    l >> 16
}

/// `java.util.Random`, which vanilla's legacy random source matches bit for bit.
pub struct JavaRandom(i64);

impl JavaRandom {
    const MULTIPLIER: i64 = 0x5_DEEC_E66D;
    const MASK: i64 = (1 << 48) - 1;

    pub fn new(seed: i64) -> Self {
        Self((seed ^ Self::MULTIPLIER) & Self::MASK)
    }

    fn next(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(Self::MULTIPLIER).wrapping_add(0xB)) & Self::MASK;
        (self.0 >> (48 - bits)) as i32
    }

    pub fn next_int(&mut self, bound: i32) -> i32 {
        if (bound as u32).is_power_of_two() {
            return ((i64::from(bound) * i64::from(self.next(31))) >> 31) as i32;
        }
        loop {
            let bits = self.next(31);
            let value = bits % bound;
            if bits - value + (bound - 1) >= 0 {
                return value;
            }
        }
    }

    pub fn next_long(&mut self) -> i64 {
        (i64::from(self.next(32)) << 32).wrapping_add(i64::from(self.next(32)))
    }
}

#[cfg(test)]
mod tests {
    use super::{JavaRandom, position_seed};

    /// Expected values printed by Java 25 running vanilla's `Mth.getSeed` and `java.util.Random`.
    #[test]
    fn variant_choice_matches_java() {
        let mut r = JavaRandom::new(42);
        assert_eq!([r.next_int(10), r.next_int(10), r.next_int(10)], [0, 3, 8]);
        assert_eq!(JavaRandom::new(42).next_long(), -5_025_562_857_975_149_833);

        // Far out, x * 3129871 overflows an int, which vanilla relies on.
        assert_eq!(position_seed(1, 64, -3), -62_687_207_132_611);
        assert_eq!(
            position_seed(30_000_000, 70, -29_999_999),
            -67_469_557_267_894
        );
        // A three-way pick takes Java's rejection loop, a four-way one its power-of-two path.
        let seed = position_seed(1, 64, -3);
        assert_eq!(
            [
                JavaRandom::new(seed).next_int(3),
                JavaRandom::new(seed).next_int(4)
            ],
            [1, 2]
        );
    }
}
