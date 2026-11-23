//! Chests, which vanilla draws in code (`ChestRenderer`) from an entity texture rather than
//! a block model. The boxes and texture offsets are those of 26.3's `ChestModel`. A closed
//! chest is baked into its chunk whole; while its lid is open the chunk has only its body
//! (`body`), and the lid is drawn each frame at its openness (`Lid::vertices`).

use crate::atlas::Atlas;
use crate::bake::{self, BakedModel, Dir, Quad};
use crate::mesh::Vertex;
use glam::{Quat, Vec3};

/// Every chest texture, so the atlas loads them.
pub const SPRITES: &[&str] = &[
    "minecraft:entity/chest/normal",
    "minecraft:entity/chest/normal_left",
    "minecraft:entity/chest/normal_right",
    "minecraft:entity/chest/trapped",
    "minecraft:entity/chest/trapped_left",
    "minecraft:entity/chest/trapped_right",
    "minecraft:entity/chest/ender",
    "minecraft:entity/chest/copper",
    "minecraft:entity/chest/copper_left",
    "minecraft:entity/chest/copper_right",
    "minecraft:entity/chest/copper_exposed",
    "minecraft:entity/chest/copper_exposed_left",
    "minecraft:entity/chest/copper_exposed_right",
    "minecraft:entity/chest/copper_weathered",
    "minecraft:entity/chest/copper_weathered_left",
    "minecraft:entity/chest/copper_weathered_right",
    "minecraft:entity/chest/copper_oxidized",
    "minecraft:entity/chest/copper_oxidized_left",
    "minecraft:entity/chest/copper_oxidized_right",
];

/// One `addBox`: position and size in pixels, its texture offset, and the face left off
/// where two halves of a double chest meet.
struct Box3 {
    from: [f32; 3],
    size: [f32; 3],
    tex: [f32; 2],
    open_side: Option<Dir>,
}

/// The closed chest model for a block state, or `None` if the block isn't a chest.
pub fn model(block: &str, values: &[(&str, &str)], atlas: &Atlas) -> Option<BakedModel> {
    let (quads, rotation) = model_quads(block, values, atlas)?;
    Some(baked(quads.into_iter().map(|(_, q)| q), rotation))
}

/// The chest without its lid and lock, for a chunk while the lid is open.
pub fn body(block: &str, values: &[(&str, &str)], atlas: &Atlas) -> Option<BakedModel> {
    let (quads, rotation) = model_quads(block, values, atlas)?;
    let body = quads.into_iter().filter(|(b, _)| *b == 0).map(|(_, q)| q);
    Some(baked(body, rotation))
}

/// The chest's lid and lock, to draw at any openness.
pub fn lid(block: &str, values: &[(&str, &str)], atlas: &Atlas) -> Option<Lid> {
    let (quads, rotation) = model_quads(block, values, atlas)?;
    let quads = quads.into_iter().filter(|(b, _)| *b != 0).map(|(_, q)| q);
    Some(Lid {
        quads: quads.collect(),
        rotation,
    })
}

/// A chest's lid and lock in the model's own space (blocks, facing south), and the turn
/// to the chest's facing.
#[derive(Debug)]
pub struct Lid {
    quads: Vec<Quad>,
    rotation: Quat,
}

impl Lid {
    /// The lid's vertices for the chest at `pos`, `open` of the way open (`ChestModel
    /// .setupAnim`: turned `open` × 90° up about its hinge, 9 pixels up at the back), lit
    /// by `light` (sky and block, 0 to 15) and shaded as the chunk shades its faces.
    pub fn vertices(&self, pos: [i32; 3], open: f32, [sky, block]: [u8; 2]) -> Vec<Vertex> {
        let hinge = Vec3::new(0.0, 9.0, 1.0) / 16.0;
        let turn = Quat::from_rotation_x(-open * std::f32::consts::FRAC_PI_2);
        let center = Vec3::splat(0.5);
        let origin = Vec3::from(pos.map(|c| c as f32));
        let mut out = Vec::with_capacity(self.quads.len() * 4);
        for q in &self.quads {
            let p = q.pos.map(|p| {
                let p = turn * (p - hinge) + hinge;
                origin + self.rotation * (p - center) + center
            });
            let normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            let shade = (normal * normal).dot(Vec3::new(
                Dir::East.shade(),
                if normal.y > 0.0 {
                    Dir::Up.shade()
                } else {
                    Dir::Down.shade()
                },
                Dir::South.shade(),
            ));
            let c = (shade * 255.0).round() as u8;
            for (p, uv) in p.iter().zip(q.uv) {
                out.push(Vertex {
                    pos: p.to_array(),
                    uv,
                    color: [c, c, c, 255],
                    light: [sky.min(15) * 16, block.min(15) * 16, 0, 0],
                });
            }
        }
        out
    }
}

/// Quads turned to the chest's facing, as a block entity's model: lit flat, without
/// ambient occlusion.
fn baked(quads: impl Iterator<Item = Quad>, rotation: Quat) -> BakedModel {
    let center = Vec3::splat(0.5);
    let quads = quads
        .map(|q| {
            let pos = q.pos.map(|p| bake::snap(rotation * (p - center) + center));
            let dir = Dir::nearest(rotation * q.dir.normal());
            Quad {
                pos,
                dir,
                on_face: bake::lies_on_face(&pos, dir),
                ..q
            }
        })
        .collect();
    BakedModel {
        quads,
        ambient_occlusion: false,
    }
}

/// The chest's quads in model space (blocks, facing south), each with its box: 0 the
/// bottom, 1 the lid and 2 the lock; and the turn to its facing.
fn model_quads(
    block: &str,
    values: &[(&str, &str)],
    atlas: &Atlas,
) -> Option<(Vec<(usize, Quad)>, Quat)> {
    let name = block.strip_prefix("minecraft:").unwrap_or(block);
    let name = name.strip_prefix("waxed_").unwrap_or(name);
    let material = match name {
        "chest" => "normal",
        "trapped_chest" => "trapped",
        "ender_chest" => "ender",
        "copper_chest" => "copper",
        "exposed_copper_chest" => "copper_exposed",
        "weathered_copper_chest" => "copper_weathered",
        "oxidized_copper_chest" => "copper_oxidized",
        _ => return None,
    };
    let value = |p: &str| values.iter().find(|(n, _)| *n == p).map(|(_, v)| *v);
    let kind = value("type").unwrap_or("single");
    let sprite = match kind {
        "single" => format!("minecraft:entity/chest/{material}"),
        side => format!("minecraft:entity/chest/{material}_{side}"),
    };
    let sprite = atlas.sprite(&sprite);
    let bottom_tex = [0.0, 19.0];
    let lid_tex = [0.0, 0.0];
    // The lid and lock are posed 9 pixels up and 1 south.
    let posed = |b: [f32; 3]| [b[0], b[1] + 9.0, b[2] + 1.0];
    let boxes = match kind {
        // A left half joins its partner on its west side, a right half on its east.
        "left" => [
            Box3 {
                from: [0.0, 0.0, 1.0],
                size: [15.0, 10.0, 14.0],
                tex: bottom_tex,
                open_side: Some(Dir::West),
            },
            Box3 {
                from: posed([0.0, 0.0, 0.0]),
                size: [15.0, 5.0, 14.0],
                tex: lid_tex,
                open_side: Some(Dir::West),
            },
            Box3 {
                from: posed([0.0, -2.0, 14.0]),
                size: [1.0, 4.0, 1.0],
                tex: lid_tex,
                open_side: None,
            },
        ],
        "right" => [
            Box3 {
                from: [1.0, 0.0, 1.0],
                size: [15.0, 10.0, 14.0],
                tex: bottom_tex,
                open_side: Some(Dir::East),
            },
            Box3 {
                from: posed([1.0, 0.0, 0.0]),
                size: [15.0, 5.0, 14.0],
                tex: lid_tex,
                open_side: Some(Dir::East),
            },
            Box3 {
                from: posed([15.0, -2.0, 14.0]),
                size: [1.0, 4.0, 1.0],
                tex: lid_tex,
                open_side: None,
            },
        ],
        _ => [
            Box3 {
                from: [1.0, 0.0, 1.0],
                size: [14.0, 10.0, 14.0],
                tex: bottom_tex,
                open_side: None,
            },
            Box3 {
                from: posed([1.0, 0.0, 0.0]),
                size: [14.0, 5.0, 14.0],
                tex: lid_tex,
                open_side: None,
            },
            Box3 {
                from: posed([7.0, -2.0, 14.0]),
                size: [2.0, 4.0, 1.0],
                tex: lid_tex,
                open_side: None,
            },
        ],
    };
    // The model faces south; vanilla turns it by -yRot of `facing` (south 0, west 90, ...).
    let y_rot: f32 = match value("facing").unwrap_or("north") {
        "south" => 0.0,
        "west" => 90.0,
        "north" => 180.0,
        _ => 270.0,
    };
    let rotation = Quat::from_rotation_y((-y_rot).to_radians());
    let [su0, sv0, su1, sv1] = sprite.rect;
    let to_atlas = |u: f32, v: f32| [su0 + (su1 - su0) * u / 64.0, sv0 + (sv1 - sv0) * v / 64.0];

    let mut quads = Vec::new();
    for (index, b) in boxes.iter().enumerate() {
        let min = Vec3::from(b.from);
        let max = min + Vec3::from(b.size);
        let [w, h, d] = b.size;
        let [u, v] = b.tex;
        // ModelPart's box unwrap: depth, width, depth, width across; depth then height down.
        let (u0, u1, u2, u4) = (u, u + d, u + d + w, u + d + w + d);
        let (v0, v1) = (v, v + d);
        for face in Dir::ALL {
            if b.open_side == Some(face) {
                continue;
            }
            let corners = bake::corners(face, min, max);
            // Chests aren't flipped like mobs, so side textures run upward and leftward as
            // seen from outside: the texture's latch notch ends up beside the lock.
            let uv = corners.map(|p| {
                let (tu, tv) = match face {
                    Dir::North => (u1 + (p.x - min.x), v1 + (p.y - min.y)),
                    Dir::South => (u4 + (max.x - p.x), v1 + (p.y - min.y)),
                    Dir::West => (u0 + (max.z - p.z), v1 + (p.y - min.y)),
                    Dir::East => (u2 + (p.z - min.z), v1 + (p.y - min.y)),
                    Dir::Down => (u1 + (p.x - min.x), v0 + (p.z - min.z)),
                    Dir::Up => (u2 + (p.x - min.x), v0 + (p.z - min.z)),
                };
                debug_assert!(tv <= v1 + h + 1e-3);
                to_atlas(tu, tv)
            });
            quads.push((
                index,
                Quad {
                    pos: corners.map(|p| p / 16.0),
                    uv,
                    dir: face,
                    cull: None,
                    tint: -1,
                    shade: None,
                    layer: sprite.layer,
                    on_face: false,
                },
            ));
        }
    }
    Some((quads, rotation))
}
