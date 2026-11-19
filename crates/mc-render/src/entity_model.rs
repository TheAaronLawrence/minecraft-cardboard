//! Blocks vanilla draws in code with entity models (`ModelPart` cubes) rather than block
//! models: banners, shulker boxes, mob heads, decorated pots, conduits and copper golem
//! statues, plus the end portal and gateway surfaces. The boxes, texture offsets, poses and
//! transforms are 26.3's (`BannerModel`, `ShulkerModel`, `SkullModel`, `PiglinHeadModel`,
//! `DragonHeadModel`, `DecoratedPotRenderer`, `ConduitRenderer`, `CopperGolemModel`'s pose
//! layers) and their block entity renderers.

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

const POT_BASE: &str = "minecraft:entity/decorated_pot/decorated_pot_base";
const POT_SIDE: &str = "minecraft:entity/decorated_pot/decorated_pot_side";
const CONDUIT: &str = "minecraft:entity/conduit/base";
/// Vanilla draws portals with a parallax star shader; until we write one, they show the
/// end sky texture, tinted dark teal by `PORTAL_TINT`.
const PORTAL: &str = "minecraft:environment/end_sky";
pub const PORTAL_TINT: [f32; 3] = [0.15, 0.3, 0.32];
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
    out.extend([POT_BASE, POT_SIDE, CONDUIT, PORTAL].map(str::to_owned));
    out.extend(
        ["", "_exposed", "_weathered", "_oxidized"]
            .map(|age| format!("minecraft:entity/copper_golem/copper_golem{age}")),
    );
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
    /// Only this face (an index into `FACES`), as `addBox` with a set of visible faces.
    only: Option<usize>,
}

const fn cube(from: [f32; 3], size: [f32; 3], tex: [f32; 2]) -> Cube {
    Cube {
        from,
        size,
        tex,
        grow: 0.0,
        mirror: false,
        only: None,
    }
}

/// `ModelPart.Cube`'s faces in order: down, up, west, north, east, south (in model space),
/// as corner indices and outward normals.
const FACES: [([usize; 4], [f32; 3]); 6] = [
    ([5, 4, 0, 1], [0.0, -1.0, 0.0]),
    ([2, 3, 7, 6], [0.0, 1.0, 0.0]),
    ([0, 4, 7, 3], [-1.0, 0.0, 0.0]),
    ([1, 0, 3, 2], [0.0, 0.0, -1.0]),
    ([5, 1, 2, 6], [1.0, 0.0, 0.0]),
    ([4, 5, 6, 7], [0.0, 0.0, 1.0]),
];
const NORTH: usize = 3;

/// One part of a model: its parent (an earlier index), `PartPose` offset in pixels and
/// rotation in radians, and its cubes.
struct Node {
    parent: Option<usize>,
    pose: [f32; 6],
    cubes: &'static [Cube],
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
    /// Every part's cubes, each placed by its parents' poses under `root`.
    fn nodes(&mut self, root: Mat4, nodes: &[Node]) {
        let mut placed: Vec<Mat4> = Vec::with_capacity(nodes.len());
        for n in nodes {
            let [x, y, z, rx, ry, rz] = n.pose;
            let parent = n.parent.map_or(root, |i| placed[i]);
            let m = parent * pose([x, y, z], [rx, ry, rz]);
            for c in n.cubes {
                self.cube(m, c);
            }
            placed.push(m);
        }
    }

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
        let uvs = [
            [u1, t0, u2, t1],
            [u2, t1, u22, t0],
            [u0, t1, u1, t2],
            [u1, t1, u2, t2],
            [u2, t1, u3, t2],
            [u3, t1, u4, t2],
        ];
        let normal_m = Mat3::from_mat4(m).inverse().transpose();
        let [su0, sv0, su1, sv1] = self.sprite.rect;
        let [tw, th] = self.tex_size;
        let to_atlas = |u: f32, v: f32| [su0 + (su1 - su0) * u / tw, sv0 + (sv1 - sv0) * v / th];
        for (face, ((idx, normal), [a, b, c2, d2])) in FACES.iter().zip(uvs).enumerate() {
            if c.only.is_some_and(|o| o != face) {
                continue;
            }
            // `Polygon`: corners take (u2,v1), (u1,v1), (u1,v2), (u2,v2).
            let uv = [
                to_atlas(c2, b),
                to_atlas(a, b),
                to_atlas(a, d2),
                to_atlas(c2, d2),
            ];
            let local = idx.map(|i| p[i]);
            // Mirroring moves the west face to the east and back.
            let mut out = Vec3::from(*normal);
            if c.mirror {
                out.x = -out.x;
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
    let quads = if name == "decorated_pot" {
        decorated_pot(atlas, value("facing")?)
    } else if name == "conduit" {
        // Inactive: just the shell, as without its block entity it can't be active.
        let mut b = Builder {
            quads: Vec::new(),
            sprite: atlas.sprite(CONDUIT),
            tex_size: [32.0, 16.0],
            tint: -1,
        };
        b.cube(
            Mat4::from_translation(Vec3::splat(0.5)),
            &cube([-3.0, -3.0, -3.0], [6.0, 6.0, 6.0], [0.0, 0.0]),
        );
        b.quads
    } else if name == "end_portal" || name == "end_gateway" {
        portal(atlas, name == "end_portal")
    } else if let Some(age) = name
        .strip_prefix("waxed_")
        .unwrap_or(name)
        .strip_suffix("copper_golem_statue")
    {
        let sprite = match age.strip_suffix('_') {
            Some(age) => format!("minecraft:entity/copper_golem/copper_golem_{age}"),
            None => "minecraft:entity/copper_golem/copper_golem".to_owned(),
        };
        let nodes = match value("copper_golem_pose")? {
            "sitting" => GOLEM_SITTING,
            "running" => GOLEM_RUNNING,
            "star" => GOLEM_STAR,
            _ => GOLEM_STANDING,
        };
        // `CopperGolemStatueBlockRenderer`, then the statue model's root: y 0, half a turn
        // about Z, which stands the model upright on the block's floor.
        let root = Mat4::from_translation(Vec3::new(0.5, 0.0, 0.5))
            * Mat4::from_rotation_y(-y_rot(opposite(value("facing")?)).to_radians())
            * Mat4::from_rotation_z(PI);
        let mut b = Builder {
            quads: Vec::new(),
            sprite: atlas.sprite(&sprite),
            tex_size: [64.0, 64.0],
            tint: -1,
        };
        b.nodes(root, nodes);
        b.quads
    } else if banner_color(name).is_some() {
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

fn decorated_pot(atlas: &Atlas, facing: &str) -> Vec<Quad> {
    let center = Vec3::new(0.5, 0.5, 0.5);
    let root = Mat4::from_translation(center)
        * Mat4::from_rotation_y((180.0 - y_rot(facing)).to_radians())
        * Mat4::from_translation(-center);
    let mut b = Builder {
        quads: Vec::new(),
        sprite: atlas.sprite(POT_BASE),
        tex_size: [32.0, 32.0],
        tint: -1,
    };
    b.nodes(root, POT);
    // Each side is one face, which a pottery sherd would retexture; plain sides here.
    let mut sides = Builder {
        quads: b.quads,
        sprite: atlas.sprite(POT_SIDE),
        tex_size: [16.0, 16.0],
        tint: -1,
    };
    sides.nodes(root, POT_SIDES);
    sides.quads
}

const POT_LID: Cube = cube([0.0, 0.0, 0.0], [14.0, 0.0, 14.0], [-14.0, 13.0]);
const POT: &[Node] = &[
    Node {
        parent: None,
        pose: [0.0, 37.0, 16.0, PI, 0.0, 0.0],
        cubes: &[
            Cube {
                grow: -0.1,
                ..cube([4.0, 17.0, 4.0], [8.0, 3.0, 8.0], [0.0, 0.0])
            },
            Cube {
                grow: 0.2,
                ..cube([5.0, 20.0, 5.0], [6.0, 1.0, 6.0], [0.0, 5.0])
            },
        ],
    },
    Node {
        parent: None,
        pose: [1.0, 16.0, 1.0, 0.0, 0.0, 0.0],
        cubes: &[POT_LID],
    },
    Node {
        parent: None,
        pose: [1.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        cubes: &[POT_LID],
    },
];
const POT_SIDE_FACE: Cube = Cube {
    only: Some(NORTH),
    ..cube([0.0, 0.0, 0.0], [14.0, 16.0, 0.0], [1.0, 0.0])
};
const POT_SIDES: &[Node] = &[
    // back, left, right, front
    Node {
        parent: None,
        pose: [15.0, 16.0, 1.0, 0.0, 0.0, PI],
        cubes: &[POT_SIDE_FACE],
    },
    Node {
        parent: None,
        pose: [1.0, 16.0, 1.0, 0.0, -FRAC_PI_2, PI],
        cubes: &[POT_SIDE_FACE],
    },
    Node {
        parent: None,
        pose: [15.0, 16.0, 15.0, 0.0, FRAC_PI_2, PI],
        cubes: &[POT_SIDE_FACE],
    },
    Node {
        parent: None,
        pose: [1.0, 16.0, 15.0, PI, 0.0, 0.0],
        cubes: &[POT_SIDE_FACE],
    },
];

/// The portal surface: the end portal's top and bottom faces at 0.75 and 0.375 (its
/// `TRANSFORMATION` squeezes the unit cube to that slab), and the gateway's whole cube,
/// culled by solid neighbours.
fn portal(atlas: &Atlas, end_portal: bool) -> Vec<Quad> {
    let sprite = atlas.sprite(PORTAL);
    let (min, max) = if end_portal {
        (Vec3::new(0.0, 0.375, 0.0), Vec3::new(1.0, 0.75, 1.0))
    } else {
        (Vec3::ZERO, Vec3::ONE)
    };
    let [u0, v0, u1, v1] = sprite.rect;
    Dir::ALL
        .into_iter()
        .filter(|d| !end_portal || matches!(d, Dir::Up | Dir::Down))
        .map(|dir| {
            let pos = bake::corners(dir, min, max);
            Quad {
                pos,
                uv: [[u0, v0], [u0, v1], [u1, v1], [u1, v0]],
                dir,
                cull: (!end_portal).then_some(dir),
                tint: 0,
                shade: false,
                layer: sprite.layer,
                on_face: bake::lies_on_face(&pos, dir),
            }
        })
        .collect()
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

// `CopperGolemModel`'s body layers for each statue pose, as decoded from 26.3. Mojang writes
// some angles rounded, such as -1.5708; they're kept as written.
#[allow(clippy::approx_constant)]
const GOLEM_STANDING: &[Node] = &[
    Node {
        parent: None,
        pose: [0.0, -5.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[Cube {
            from: [-4.0, -6.0, -3.0],
            size: [8.0, 6.0, 6.0],
            tex: [0.0, 15.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // body
    Node {
        parent: Some(0),
        pose: [0.0, -6.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[
            Cube {
                from: [-4.0, -5.0, -5.0],
                size: [8.0, 5.0, 10.0],
                tex: [0.0, 0.0],
                grow: 0.015,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-1.0, -2.0, -6.0],
                size: [2.0, 3.0, 2.0],
                tex: [56.0, 0.0],
                grow: 0.0,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-1.0, -9.0, -1.0],
                size: [2.0, 4.0, 2.0],
                tex: [37.0, 8.0],
                grow: -0.015,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-2.0, -13.0, -2.0],
                size: [4.0, 4.0, 4.0],
                tex: [37.0, 0.0],
                grow: -0.015,
                mirror: false,
                only: None,
            },
        ],
    }, // head
    Node {
        parent: Some(0),
        pose: [-4.0, -6.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[Cube {
            from: [-3.0, -1.0, -2.0],
            size: [3.0, 10.0, 4.0],
            tex: [36.0, 16.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // right_arm
    Node {
        parent: Some(0),
        pose: [4.0, -6.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[Cube {
            from: [0.0, -1.0, -2.0],
            size: [3.0, 10.0, 4.0],
            tex: [50.0, 16.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // left_arm
    Node {
        parent: None,
        pose: [0.0, -5.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[Cube {
            from: [-4.0, 0.0, -2.0],
            size: [4.0, 5.0, 4.0],
            tex: [0.0, 27.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // right_leg
    Node {
        parent: None,
        pose: [0.0, -5.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[Cube {
            from: [0.0, 0.0, -2.0],
            size: [4.0, 5.0, 4.0],
            tex: [16.0, 27.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // left_leg
];
#[allow(clippy::approx_constant)]
const GOLEM_RUNNING: &[Node] = &[
    Node {
        parent: None,
        pose: [-1.064, -5.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // body
    Node {
        parent: Some(0),
        pose: [1.1, 0.1, 0.7, 0.1204, -0.0064, -0.0779],
        cubes: &[Cube {
            from: [-4.02, -6.116, -3.5],
            size: [8.0, 6.0, 6.0],
            tex: [0.0, 15.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // body_r1
    Node {
        parent: Some(0),
        pose: [0.7, -5.6, -1.8, 0.0, 0.0, 0.0],
        cubes: &[
            Cube {
                from: [-4.0, -5.1, -5.0],
                size: [8.0, 5.0, 10.0],
                tex: [0.0, 0.0],
                grow: 0.0,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-1.02, -2.1, -6.0],
                size: [2.0, 3.0, 2.0],
                tex: [56.0, 0.0],
                grow: 0.0,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-1.02, -9.1, -1.0],
                size: [2.0, 4.0, 2.0],
                tex: [37.0, 8.0],
                grow: -0.015,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-2.0, -13.1, -2.0],
                size: [4.0, 4.0, 4.0],
                tex: [37.0, 0.0],
                grow: -0.015,
                mirror: false,
                only: None,
            },
        ],
    }, // head
    Node {
        parent: Some(0),
        pose: [-4.0, -6.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // right_arm
    Node {
        parent: Some(3),
        pose: [0.7, -0.248, -1.62, 1.0036, 0.0, 0.0],
        cubes: &[Cube {
            from: [-3.052, -1.11, -2.036],
            size: [3.0, 10.0, 4.0],
            tex: [36.0, 16.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // right_arm_r1
    Node {
        parent: Some(0),
        pose: [4.0, -6.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // left_arm
    Node {
        parent: Some(5),
        pose: [0.732, 0.0, 0.0, -0.8715, -0.0535, -0.0449],
        cubes: &[Cube {
            from: [0.032, -1.1, -2.0],
            size: [3.0, 10.0, 4.0],
            tex: [50.0, 16.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // left_arm_r1
    Node {
        parent: None,
        pose: [-3.064, -5.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // right_leg
    Node {
        parent: Some(7),
        pose: [1.048, 0.0, -0.9, -0.8727, 0.0, 0.0],
        cubes: &[Cube {
            from: [-1.856, -0.1, -1.09],
            size: [4.0, 5.0, 4.0],
            tex: [0.0, 27.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // right_leg_r1
    Node {
        parent: None,
        pose: [0.936, -5.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // left_leg
    Node {
        parent: Some(9),
        pose: [1.0, 0.0, 0.0, 0.7854, 0.0, 0.0],
        cubes: &[Cube {
            from: [-2.088, -0.1, -2.0],
            size: [4.0, 5.0, 4.0],
            tex: [16.0, 27.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // left_leg_r1
];
#[allow(clippy::approx_constant)]
const GOLEM_SITTING: &[Node] = &[
    Node {
        parent: None,
        pose: [0.0, -3.0, 2.325, 0.0, 0.0, 0.0],
        cubes: &[
            Cube {
                from: [-3.0, -4.0, -4.525],
                size: [6.0, 1.0, 6.0],
                tex: [3.0, 19.0],
                grow: 0.0,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-4.0, -3.0, -3.525],
                size: [8.0, 6.0, 6.0],
                tex: [0.0, 15.0],
                grow: 0.0,
                mirror: false,
                only: None,
            },
        ],
    }, // body
    Node {
        parent: Some(0),
        pose: [0.0, -1.0, -4.325, 0.0, 0.0, -3.1416],
        cubes: &[Cube {
            from: [-4.0, -3.0, -2.2],
            size: [8.0, 6.0, 3.0],
            tex: [3.0, 18.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // body_r1
    Node {
        parent: Some(0),
        pose: [0.0, -6.0, -0.2, 0.0, 0.0, 0.0],
        cubes: &[
            Cube {
                from: [-1.0, -7.0, -3.3],
                size: [2.0, 4.0, 2.0],
                tex: [37.0, 8.0],
                grow: -0.015,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-2.0, -11.0, -4.3],
                size: [4.0, 4.0, 4.0],
                tex: [37.0, 0.0],
                grow: -0.015,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-4.0, -3.0, -7.325],
                size: [8.0, 5.0, 10.0],
                tex: [0.0, 0.0],
                grow: 0.0,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-1.0, 0.0, -8.325],
                size: [2.0, 3.0, 2.0],
                tex: [56.0, 0.0],
                grow: 0.0,
                mirror: false,
                only: None,
            },
        ],
    }, // head
    Node {
        parent: Some(0),
        pose: [-4.0, -5.6, -1.8, 0.4363, 0.0, 0.0],
        cubes: &[],
    }, // right_arm
    Node {
        parent: Some(3),
        pose: [0.0, 0.0893, 0.1198, -1.0472, 0.0, 0.0],
        cubes: &[Cube {
            from: [-3.075, -0.9733, -1.9966],
            size: [3.0, 10.0, 4.0],
            tex: [36.0, 16.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // right_arm_r1
    Node {
        parent: Some(0),
        pose: [4.0, -5.6, -1.7, 0.4363, 0.0, 0.0],
        cubes: &[],
    }, // left_arm
    Node {
        parent: Some(5),
        pose: [0.0, -0.0015, -0.0808, -1.0472, 0.0, 0.0],
        cubes: &[Cube {
            from: [0.075, -1.0443, -1.8997],
            size: [3.0, 10.0, 4.0],
            tex: [50.0, 16.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // left_arm_r1
    Node {
        parent: None,
        pose: [-2.1, -2.1, -2.075, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // right_leg
    Node {
        parent: Some(7),
        pose: [0.05, -1.9, 1.075, -1.5708, 0.0, 0.0],
        cubes: &[Cube {
            from: [-2.0, 0.975, 0.0],
            size: [4.0, 5.0, 4.0],
            tex: [0.0, 27.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // right_leg_r1
    Node {
        parent: None,
        pose: [2.0, -2.0, -2.075, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // left_leg
    Node {
        parent: Some(9),
        pose: [0.05, -2.0, 1.075, -1.5708, 0.0, 0.0],
        cubes: &[Cube {
            from: [-2.0, 0.975, 0.0],
            size: [4.0, 5.0, 4.0],
            tex: [16.0, 27.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // left_leg_r1
];
#[allow(clippy::approx_constant)]
const GOLEM_STAR: &[Node] = &[
    Node {
        parent: None,
        pose: [0.0, -5.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[Cube {
            from: [-4.0, -6.0, -3.0],
            size: [8.0, 6.0, 6.0],
            tex: [0.0, 15.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // body
    Node {
        parent: Some(0),
        pose: [0.0, -6.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[
            Cube {
                from: [-4.0, -5.0, -5.0],
                size: [8.0, 5.0, 10.0],
                tex: [0.0, 0.0],
                grow: 0.0,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-1.0, -2.0, -6.0],
                size: [2.0, 3.0, 2.0],
                tex: [56.0, 0.0],
                grow: 0.0,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-1.0, -9.0, -1.0],
                size: [2.0, 4.0, 2.0],
                tex: [37.0, 8.0],
                grow: -0.015,
                mirror: false,
                only: None,
            },
            Cube {
                from: [-2.0, -13.0, -2.0],
                size: [4.0, 4.0, 4.0],
                tex: [37.0, 0.0],
                grow: -0.015,
                mirror: false,
                only: None,
            },
        ],
    }, // head
    Node {
        parent: Some(0),
        pose: [-4.0, -6.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // right_arm
    Node {
        parent: Some(2),
        pose: [1.0, 1.0, 0.0, 0.0, 0.0, 1.9199],
        cubes: &[Cube {
            from: [-1.5, -5.0, -2.0],
            size: [3.0, 10.0, 4.0],
            tex: [36.0, 16.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // right_arm_r1
    Node {
        parent: Some(2),
        pose: [-1.0, 7.4, -1.0, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // rightItem
    Node {
        parent: Some(0),
        pose: [4.0, -6.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // left_arm
    Node {
        parent: Some(5),
        pose: [-1.0, 1.0, 0.0, 0.0, 0.0, -1.9199],
        cubes: &[Cube {
            from: [-1.5, -5.0, -2.0],
            size: [3.0, 10.0, 4.0],
            tex: [50.0, 16.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // left_arm_r1
    Node {
        parent: None,
        pose: [-3.0, -5.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // right_leg
    Node {
        parent: Some(7),
        pose: [0.35, 2.0, 0.01, 0.0, 0.0, 0.2618],
        cubes: &[Cube {
            from: [-2.0, -2.5, -2.0],
            size: [4.0, 5.0, 4.0],
            tex: [0.0, 27.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // right_leg_r1
    Node {
        parent: None,
        pose: [1.0, -5.0, 0.0, 0.0, 0.0, 0.0],
        cubes: &[],
    }, // left_leg
    Node {
        parent: Some(9),
        pose: [1.65, 2.0, 0.0, 0.0, 0.0, -0.2618],
        cubes: &[Cube {
            from: [-2.0, -2.5, -2.0],
            size: [4.0, 5.0, 4.0],
            tex: [16.0, 27.0],
            grow: 0.0,
            mirror: false,
            only: None,
        }],
    }, // left_leg_r1
];
