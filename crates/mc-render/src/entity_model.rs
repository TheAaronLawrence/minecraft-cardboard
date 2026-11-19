//! Blocks vanilla draws in code with entity models (`ModelPart` cubes) rather than block
//! models: banners, shulker boxes, and mob heads. The boxes, texture offsets, poses and
//! transforms are 26.3's `BannerModel`, `BannerFlagModel`, `ShulkerModel`, `SkullModel`,
//! `PiglinHeadModel` and `DragonHeadModel`, and their block entity renderers.

use crate::atlas::{Atlas, Sprite};
use crate::bake::{self, BakedModel, Dir, Quad};
use glam::{Mat3, Mat4, Quat, Vec3};
use std::f32::consts::{FRAC_PI_2, PI};

const COLORS: [&str; 16] = [
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

/// `DyeColor`'s texture diffuse colours, in `COLORS` order, which banners are tinted with.
const DYE_RGB: [u32; 16] = [
    0xF9FFFE, 0xF9801D, 0xC74EBD, 0x3AB3DA, 0xFED83D, 0x80C71F, 0xF38BAA, 0x474F52, 0x9D9D97,
    0x169C9C, 0x8932B8, 0x3C44AA, 0x835432, 0x5E7C16, 0xB02E26, 0x1D1D21,
];

const BANNER_BASE: &str = "minecraft:entity/banner/banner_base";
const BANNER_FLAG: &str = "minecraft:entity/banner/base";
const SKULLS: [(&str, &str); 7] = [
    ("skeleton", "minecraft:entity/skeleton/skeleton"),
    (
        "wither_skeleton",
        "minecraft:entity/skeleton/wither_skeleton",
    ),
    ("zombie", "minecraft:entity/zombie/zombie"),
    ("creeper", "minecraft:entity/creeper/creeper"),
    ("dragon", "minecraft:entity/enderdragon/dragon"),
    ("piglin", "minecraft:entity/piglin/piglin"),
    // A head without a profile wears the default skin.
    ("player", "minecraft:entity/player/wide/steve"),
];

/// Every texture these models use, so the atlas loads them.
pub fn sprites() -> Vec<String> {
    let mut out = vec![BANNER_BASE.to_owned(), BANNER_FLAG.to_owned()];
    out.push("minecraft:entity/shulker/shulker".to_owned());
    out.extend(
        COLORS
            .iter()
            .map(|c| format!("minecraft:entity/shulker/shulker_{c}")),
    );
    out.extend(SKULLS.iter().map(|(_, s)| s.to_string()));
    out
}

/// The dye colour a banner's flag is tinted with, from its block name.
pub fn banner_color(name: &str) -> Option<[f32; 3]> {
    let color = name
        .strip_suffix("_wall_banner")
        .or_else(|| name.strip_suffix("_banner"))?;
    let i = COLORS.iter().position(|c| *c == color)?;
    let c = DYE_RGB[i];
    Some([16, 8, 0].map(|s| (c >> s & 0xFF) as f32 / 255.0))
}

/// One `addBox`, in model pixels, with its texture offset.
struct Cube {
    from: [f32; 3],
    size: [f32; 3],
    tex: [f32; 2],
    /// `CubeDeformation`: grows the box on every side without moving its texture.
    grow: f32,
    mirror: bool,
}

const fn cube(from: [f32; 3], size: [f32; 3], tex: [f32; 2]) -> Cube {
    Cube {
        from,
        size,
        tex,
        grow: 0.0,
        mirror: false,
    }
}

/// `ModelPart.translateAndRotate`: move by the offset in pixels, then turn Z, Y, X.
fn pose(offset: [f32; 3], rot: [f32; 3]) -> Mat4 {
    Mat4::from_translation(Vec3::from(offset) / 16.0)
        * Mat4::from_quat(
            Quat::from_rotation_z(rot[2])
                * Quat::from_rotation_y(rot[1])
                * Quat::from_rotation_x(rot[0]),
        )
}

/// Collects one texture's cubes as block-space quads.
struct Builder {
    quads: Vec<Quad>,
    sprite: Sprite,
    tex_size: [f32; 2],
    tint: i32,
}

impl Builder {
    /// `ModelPart.Cube`'s vertices and UVs, placed by `m` (model units of 1/16 into block
    /// space). Each face keeps vanilla's corner-to-UV pairing; the corners are then put in
    /// the winding the mesher expects, since negative scales and mirroring flip it.
    fn cube(&mut self, m: Mat4, c: &Cube) {
        let min = Vec3::from(c.from) - c.grow;
        let max = Vec3::from(c.from) + Vec3::from(c.size) + c.grow;
        let (mut x0, x1) = (min.x, max.x);
        let mut x1 = x1;
        if c.mirror {
            std::mem::swap(&mut x0, &mut x1);
        }
        let (y0, y1, z0, z1) = (min.y, max.y, min.z, max.z);
        let v = |x, y, z| Vec3::new(x, y, z);
        let p = [
            v(x0, y0, z0),
            v(x1, y0, z0),
            v(x1, y1, z0),
            v(x0, y1, z0),
            v(x0, y0, z1),
            v(x1, y0, z1),
            v(x1, y1, z1),
            v(x0, y1, z1),
        ];
        let [w, h, d] = c.size;
        let [u, t] = c.tex;
        let (u0, u1, u2, u22, u3, u4) = (u, u + d, u + d + w, u + d + w + w, u + d + w + d, {
            u + d + w + d + w
        });
        let (t0, t1, t2) = (t, t + d, t + d + h);
        let faces = [
            ([5, 4, 0, 1], [u1, t0, u2, t1]),
            ([2, 3, 7, 6], [u2, t1, u22, t0]),
            ([0, 4, 7, 3], [u0, t1, u1, t2]),
            ([1, 0, 3, 2], [u1, t1, u2, t2]),
            ([5, 1, 2, 6], [u2, t1, u3, t2]),
            ([4, 5, 6, 7], [u3, t1, u4, t2]),
        ];
        let normal_m = Mat3::from_mat4(m).inverse().transpose();
        let center = (min + max) / 2.0;
        let [su0, sv0, su1, sv1] = self.sprite.rect;
        let [tw, th] = self.tex_size;
        let to_atlas = |u: f32, v: f32| [su0 + (su1 - su0) * u / tw, sv0 + (sv1 - sv0) * v / th];
        for (idx, [a, b, c2, d2]) in faces {
            // `Polygon`: corners take (u2,v1), (u1,v1), (u1,v2), (u2,v2).
            let uv = [
                to_atlas(c2, b),
                to_atlas(a, b),
                to_atlas(a, d2),
                to_atlas(c2, d2),
            ];
            let local = idx.map(|i| p[i]);
            let face_center = local.iter().copied().sum::<Vec3>() / 4.0;
            let out = face_center - center;
            if out.length_squared() < 1e-8 {
                continue;
            }
            let outward = (normal_m * out).normalize();
            let mut pos = local.map(|q| bake::snap(m.transform_point3(q / 16.0)));
            let mut uv = uv;
            let geometric = (pos[1] - pos[0]).cross(pos[2] - pos[0]);
            if geometric.dot(outward) < 0.0 {
                pos.swap(1, 3);
                uv.swap(1, 3);
            }
            let dir = Dir::nearest(outward);
            self.quads.push(Quad {
                pos,
                uv,
                dir,
                cull: None,
                tint: self.tint,
                shade: true,
                layer: self.sprite.layer,
                on_face: bake::lies_on_face(&pos, dir),
            });
        }
    }
}

/// The model for a block state, or `None` if vanilla doesn't draw the block this way.
pub fn model(block: &str, values: &[(&str, &str)], atlas: &Atlas) -> Option<BakedModel> {
    let name = block.strip_prefix("minecraft:").unwrap_or(block);
    let value = |p: &str| values.iter().find(|(n, _)| *n == p).map(|(_, v)| *v);
    // `RotationSegment.convertToDegrees` for ground blocks, `Direction.toYRot` for walls.
    let segment = || {
        value("rotation")
            .and_then(|r| r.parse::<f32>().ok())
            .unwrap_or(0.0)
            * 22.5
    };
    let quads = if banner_color(name).is_some() {
        let wall = name.ends_with("_wall_banner");
        let deg = if wall {
            y_rot(value("facing")?)
        } else {
            segment()
        };
        banner(atlas, wall, deg)
    } else if let Some(color) = name.strip_suffix("shulker_box") {
        let color = color.strip_suffix('_');
        let sprite = match color {
            Some(c) => format!("minecraft:entity/shulker/shulker_{c}"),
            None => "minecraft:entity/shulker/shulker".to_owned(),
        };
        shulker(atlas.sprite(&sprite), value("facing").unwrap_or("up"))
    } else {
        let (kind, wall) = skull_kind(name)?;
        let sprite = atlas.sprite(SKULLS.iter().find(|(k, _)| *k == kind)?.1);
        let root = if wall {
            let facing = value("facing")?;
            let [sx, _, sz] = dir_step(facing);
            Mat4::from_translation(Vec3::new(
                0.5 - 0.25 * sx as f32,
                0.25,
                0.5 - 0.25 * sz as f32,
            )) * Mat4::from_rotation_y(-y_rot(opposite(facing)).to_radians())
        } else {
            Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5))
                * Mat4::from_rotation_y(-segment().to_radians())
        } * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0));
        skull(sprite, kind, root)
    };
    // Block entities are lit flat, without ambient occlusion.
    Some(BakedModel {
        quads,
        ambient_occlusion: false,
    })
}

fn banner(atlas: &Atlas, wall: bool, deg: f32) -> Vec<Quad> {
    let root = Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5))
        * Mat4::from_rotation_y(-deg.to_radians())
        * Mat4::from_scale(Vec3::new(2.0 / 3.0, -2.0 / 3.0, -2.0 / 3.0));
    let mut b = Builder {
        quads: Vec::new(),
        sprite: atlas.sprite(BANNER_BASE),
        tex_size: [64.0, 64.0],
        tint: -1,
    };
    if !wall {
        b.cube(
            root,
            &cube([-1.0, -42.0, -1.0], [2.0, 42.0, 2.0], [44.0, 0.0]),
        );
    }
    let (bar_y, bar_z) = if wall { (-20.5, 9.5) } else { (-44.0, -1.0) };
    b.cube(
        root,
        &cube([-10.0, bar_y, bar_z], [20.0, 2.0, 2.0], [0.0, 42.0]),
    );
    // The flag is the base pattern dyed the banner's colour; its pixels match `banner_base`.
    // Vanilla sways it slightly over time; it hangs straight here.
    let mut flag = Builder {
        quads: b.quads,
        sprite: atlas.sprite(BANNER_FLAG),
        tex_size: [64.0, 64.0],
        tint: 0,
    };
    let offset = if wall {
        [0.0, -20.5, 10.5]
    } else {
        [0.0, -44.0, 0.0]
    };
    flag.cube(
        root * pose(offset, [0.0; 3]),
        &cube([-10.0, 0.0, -2.0], [20.0, 40.0, 1.0], [0.0, 0.0]),
    );
    flag.quads
}

fn shulker(sprite: Sprite, facing: &str) -> Vec<Quad> {
    // `Direction.getRotation`.
    let rx = Quat::from_rotation_x;
    let facing_rot = match facing {
        "down" => rx(PI),
        "north" => rx(FRAC_PI_2) * Quat::from_rotation_z(PI),
        "south" => rx(FRAC_PI_2),
        "west" => rx(FRAC_PI_2) * Quat::from_rotation_z(FRAC_PI_2),
        "east" => rx(FRAC_PI_2) * Quat::from_rotation_z(-FRAC_PI_2),
        _ => Quat::IDENTITY,
    };
    let root = Mat4::from_translation(Vec3::splat(0.5))
        * Mat4::from_scale(Vec3::splat(0.9995))
        * Mat4::from_quat(facing_rot)
        * Mat4::from_scale(Vec3::new(1.0, -1.0, -1.0))
        * Mat4::from_translation(Vec3::new(0.0, -1.0, 0.0));
    let mut b = Builder {
        quads: Vec::new(),
        sprite,
        tex_size: [64.0, 64.0],
        tint: -1,
    };
    let part = root * pose([0.0, 24.0, 0.0], [0.0; 3]);
    b.cube(
        part,
        &cube([-8.0, -16.0, -8.0], [16.0, 12.0, 16.0], [0.0, 0.0]),
    );
    b.cube(
        part,
        &cube([-8.0, -8.0, -8.0], [16.0, 8.0, 16.0], [0.0, 28.0]),
    );
    b.quads
}

/// Which head a skull block is, and whether it hangs on a wall.
fn skull_kind(name: &str) -> Option<(&'static str, bool)> {
    let (base, wall) = match name
        .strip_suffix("_wall_skull")
        .or_else(|| name.strip_suffix("_wall_head"))
    {
        Some(b) => (b, true),
        None => (
            name.strip_suffix("_skull")
                .or_else(|| name.strip_suffix("_head"))?,
            false,
        ),
    };
    let kind = SKULLS.iter().find(|(k, _)| *k == base)?.0;
    Some((kind, wall))
}

fn skull(sprite: Sprite, kind: &str, root: Mat4) -> Vec<Quad> {
    let tex_size = match kind {
        "skeleton" | "wither_skeleton" | "creeper" => [64.0, 32.0],
        "dragon" => [256.0, 256.0],
        _ => [64.0, 64.0],
    };
    let mut b = Builder {
        quads: Vec::new(),
        sprite,
        tex_size,
        tint: -1,
    };
    match kind {
        "dragon" => {
            let head = root
                * Mat4::from_translation(Vec3::new(0.0, -7.986666 / 16.0, 0.0))
                * Mat4::from_scale(Vec3::splat(0.75));
            let mirrored = |c: Cube| Cube { mirror: true, ..c };
            for c in [
                cube([-6.0, -1.0, -24.0], [12.0, 5.0, 16.0], [176.0, 44.0]),
                cube([-8.0, -8.0, -10.0], [16.0, 16.0, 16.0], [112.0, 30.0]),
                mirrored(cube([-5.0, -12.0, -4.0], [2.0, 4.0, 6.0], [0.0, 0.0])),
                mirrored(cube([-5.0, -3.0, -22.0], [2.0, 2.0, 4.0], [112.0, 0.0])),
                cube([3.0, -12.0, -4.0], [2.0, 4.0, 6.0], [0.0, 0.0]),
                cube([3.0, -3.0, -22.0], [2.0, 2.0, 4.0], [112.0, 0.0]),
            ] {
                b.cube(head, &c);
            }
            // `setupAnim` at rest opens the jaw by 0.2 radians.
            b.cube(
                head * pose([0.0, 4.0, -8.0], [0.2, 0.0, 0.0]),
                &cube([-6.0, 0.0, -16.0], [12.0, 4.0, 16.0], [176.0, 65.0]),
            );
        }
        "piglin" => {
            for c in [
                cube([-5.0, -8.0, -4.0], [10.0, 8.0, 8.0], [0.0, 0.0]),
                cube([-2.0, -4.0, -5.0], [4.0, 4.0, 1.0], [31.0, 1.0]),
                cube([2.0, -2.0, -5.0], [1.0, 2.0, 1.0], [2.0, 4.0]),
                cube([-3.0, -2.0, -5.0], [1.0, 2.0, 1.0], [2.0, 0.0]),
            ] {
                b.cube(root, &c);
            }
            let ear = std::f32::consts::FRAC_PI_6;
            b.cube(
                root * pose([4.5, -6.0, 0.0], [0.0, 0.0, -ear]),
                &cube([0.0, 0.0, -2.0], [1.0, 5.0, 4.0], [51.0, 6.0]),
            );
            b.cube(
                root * pose([-4.5, -6.0, 0.0], [0.0, 0.0, ear]),
                &cube([-1.0, 0.0, -2.0], [1.0, 5.0, 4.0], [39.0, 6.0]),
            );
        }
        _ => {
            b.cube(root, &cube([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], [0.0, 0.0]));
            // Humanoid heads add the skin's outer "hat" layer.
            if matches!(kind, "zombie" | "player") {
                b.cube(
                    root,
                    &Cube {
                        grow: 0.25,
                        ..cube([-4.0, -8.0, -4.0], [8.0, 8.0, 8.0], [32.0, 0.0])
                    },
                );
            }
        }
    }
    b.quads
}

fn y_rot(facing: &str) -> f32 {
    match facing {
        "south" => 0.0,
        "west" => 90.0,
        "north" => 180.0,
        _ => 270.0,
    }
}

fn opposite(facing: &str) -> &'static str {
    match facing {
        "south" => "north",
        "west" => "east",
        "north" => "south",
        _ => "west",
    }
}

fn dir_step(facing: &str) -> [i32; 3] {
    match facing {
        "south" => Dir::South,
        "west" => Dir::West,
        "north" => Dir::North,
        _ => Dir::East,
    }
    .offset()
}
