//! Chests, which vanilla draws in code (`ChestRenderer`) from an entity texture rather than
//! a block model. The boxes and texture offsets are those of 26.3's `ChestModel`, closed.

use crate::atlas::Atlas;
use crate::bake::{self, BakedModel, Dir, Quad};
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

/// The chest model for a block state, or `None` if the block isn't a chest.
pub fn model(block: &str, values: &[(&str, &str)], atlas: &Atlas) -> Option<BakedModel> {
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
    for b in &boxes {
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
            let center = Vec3::splat(0.5);
            let pos = corners.map(|p| bake::snap(rotation * (p / 16.0 - center) + center));
            let dir = Dir::nearest(rotation * face.normal());
            quads.push(Quad {
                pos,
                uv,
                dir,
                cull: None,
                tint: -1,
                shade: None,
                layer: sprite.layer,
                on_face: bake::lies_on_face(&pos, dir),
            });
        }
    }
    // Block entities are lit flat, without ambient occlusion.
    Some(BakedModel {
        quads,
        ambient_occlusion: false,
    })
}
