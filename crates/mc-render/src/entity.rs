//! Entities in the world: players and mobs as vanilla 26.3's `ModelPart` models, posed by
//! their models' `setupAnim` and placed as `LivingEntityRenderer.submit` places them, and
//! dropped items as `ItemEntityRenderer` bobs and spins them. Each frame's entities become
//! world-space quads, lit by the light where the entity is and shaded by vanilla's two
//! entity lights (`Lighting`'s level setup, as the `minecraft_mix_light` shader mixes them).
//!
//! Not drawn yet: armour trims and enchantment glint, babies' own models (a baby is the adult model at
//! half size), cold and warm variants' own models (they get their textures on the
//! temperate model), and hurt and death animations.

use crate::atlas::Atlas;
use crate::bake::Quad;
use crate::entity_model::{Builder, Cube, DYE_RGB, Node, cube};
use crate::item::ItemIcon;
use crate::mesh::Vertex;
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::f32::consts::PI;

/// The textures these models use, so the atlas loads them. Players' default skins are
/// loaded with the player heads' (`entity_model::sprites`).
pub const SPRITES: [&str; 17] = [
    ZOMBIE,
    SPIDER_EYES,
    SHEEP_UNDERCOAT,
    SKELETON,
    CREEPER,
    SPIDER,
    SHEEP,
    SHEEP_WOOL,
    "minecraft:entity/cow/cow_temperate",
    "minecraft:entity/cow/cow_warm",
    "minecraft:entity/cow/cow_cold",
    "minecraft:entity/pig/pig_temperate",
    "minecraft:entity/pig/pig_warm",
    "minecraft:entity/pig/pig_cold",
    "minecraft:entity/chicken/chicken_temperate",
    "minecraft:entity/chicken/chicken_warm",
    "minecraft:entity/chicken/chicken_cold",
];
/// How many players' downloaded skins can be drawn at once: each has a blank 64×64 slot in
/// the atlas that `Renderer::write_sprite` fills once the skin arrives.
pub const SKIN_SLOTS: usize = 32;

/// The atlas id of a skin slot.
pub fn skin_slot(i: usize) -> String {
    format!("minecraft:cardboard/player_skin_{i}")
}

/// The blank skin slots, for `Atlas::build`.
pub(crate) fn skin_slots() -> impl Iterator<Item = (String, image::RgbaImage)> {
    (0..SKIN_SLOTS).map(|i| (skin_slot(i), image::RgbaImage::new(64, 64)))
}

pub const ZOMBIE: &str = "minecraft:entity/zombie/zombie";
pub const SKELETON: &str = "minecraft:entity/skeleton/skeleton";
pub const CREEPER: &str = "minecraft:entity/creeper/creeper";
pub const SPIDER: &str = "minecraft:entity/spider/spider";
pub const SHEEP: &str = "minecraft:entity/sheep/sheep";
const SHEEP_WOOL: &str = "minecraft:entity/sheep/sheep_wool";
const SHEEP_UNDERCOAT: &str = "minecraft:entity/sheep/sheep_wool_undercoat";
const SPIDER_EYES: &str = "minecraft:entity/spider/spider_eyes";

/// Which model an entity is drawn with, and what about it changes the model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// `PlayerModel`, with slim (three-pixel) arms or wide ones, crouching while it's in
    /// the crouching pose (`HumanoidRenderState.isCrouching`).
    Player {
        slim: bool,
        crouching: bool,
    },
    /// `ZombieModel`: arms held out, higher when it's aggressive (`Mob.isAggressive`).
    Zombie {
        aggressive: bool,
    },
    Skeleton,
    Creeper,
    Spider,
    Cow,
    Pig,
    /// The wool's `DyeColor` id, and whether it's been sheared.
    Sheep {
        color: u8,
        sheared: bool,
    },
    Chicken,
}

/// A living entity's render state (`LivingEntityRenderState`), already interpolated to
/// the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Living {
    pub kind: Kind,
    /// The texture's sprite id.
    pub texture: String,
    /// Feet, in world space.
    pub pos: Vec3,
    /// `bodyRot`, the head's yaw relative to it, and the head's pitch, in degrees.
    pub body_yaw: f32,
    pub head_yaw: f32,
    pub pitch: f32,
    /// `WalkAnimationState`'s position and speed.
    pub walk_pos: f32,
    pub walk_speed: f32,
    /// Ticks since the entity was added, for arms' idle bob.
    pub age: f32,
    /// A chicken's wing flap angle and speed.
    pub flap: f32,
    pub flap_speed: f32,
    pub baby: bool,
    /// The name over its head, if it shows one.
    pub name_tag: Option<NameTag>,
    /// The armour it wears and the items it holds; only humanoids draw them.
    pub equipment: Equipment,
    /// The swing its arm is in, if any.
    pub swing: Option<Swing>,
    /// Its bounding box's width and height while it's on fire, for the flames around it.
    pub burning: Option<[f32; 2]>,
    /// Flashing red, as it does while hurt or dying (`hasRedOverlay`).
    pub hurt: bool,
    /// Ticks since it died, to the frame (`deathTime`), or 0 while it lives.
    pub death_time: f32,
}

/// What a humanoid wears and holds (`HumanoidRenderState`'s equipment and hand items).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Equipment {
    /// Head, chest, legs and feet.
    pub armor: [Option<Armor>; 4],
    /// The items in its right and left hands, by name, such as `minecraft:iron_sword`.
    pub hands: [Option<String>; 2],
    /// Whether its main hand is its left.
    pub left_handed: bool,
}

/// An arm swinging (`ArmedEntityRenderState.currentSwing` and `swingAnimation`): which arm,
/// its `SwingAnimationType` (none 0, whack 1, stab 2), and how far through it is, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swing {
    pub left: bool,
    pub kind: u8,
    pub progress: f32,
}

/// A piece of armour: its equipment asset (`equippable`'s `asset_id`, such as
/// `minecraft:diamond`) and its `dyed_color`, if any.
#[derive(Clone, Debug, PartialEq)]
pub struct Armor {
    pub asset: String,
    pub dye: Option<u32>,
}

/// An equipment asset's layers (`EquipmentClientInfo`) for humanoids and for leggings:
/// each layer's atlas sprite, and for dyeable layers the colour when undyed (0 for none,
/// which hides the layer until it's dyed).
#[derive(Debug, Default)]
pub struct EquipmentLayers {
    pub humanoid: Vec<(String, Option<u32>)>,
    pub leggings: Vec<(String, Option<u32>)>,
}

/// Every equipment asset in `assets/*/equipment`, by id, and the sprites their humanoid
/// layers need.
pub(crate) fn equipment_assets(
    res: &mc_assets::Resources,
) -> anyhow::Result<(HashMap<String, EquipmentLayers>, Vec<String>)> {
    let mut assets = HashMap::new();
    let mut sprites = Vec::new();
    let dir = res.asset_path("equipment", "minecraft:x", "json");
    let dir = dir.parent().expect("a file in a folder");
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = path.file_stem().and_then(|n| n.to_str()) else {
            continue;
        };
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let mut layers = EquipmentLayers::default();
        for (kind, out) in [
            ("humanoid", &mut layers.humanoid),
            ("humanoid_leggings", &mut layers.leggings),
        ] {
            for layer in json["layers"][kind].as_array().into_iter().flatten() {
                let Some(texture) = layer["texture"].as_str() else {
                    continue;
                };
                let (ns, path) = mc_assets::split_id(texture);
                let sprite = format!("{ns}:entity/equipment/{kind}/{path}");
                let dyeable = layer.get("dyeable").map(|d| {
                    d["color_when_undyed"]
                        .as_i64()
                        .map_or(0, |c| c as u32 & 0xff_ffff | 0xff00_0000)
                });
                sprites.push(sprite.clone());
                out.push((sprite, dyeable));
            }
        }
        assets.insert(format!("minecraft:{name}"), layers);
    }
    Ok((assets, sprites))
}

/// A name shown over an entity (`EntityRenderState.nameTag`).
#[derive(Clone, Debug, PartialEq)]
pub struct NameTag {
    pub text: String,
    /// The `NAME_TAG` attachment's height above the feet: the entity's height in its pose.
    pub height: f32,
    /// `isDiscrete` (sneaking): the tag is fainter, hidden by walls, and shown only within
    /// 32 blocks.
    pub discrete: bool,
}

/// An item lying on the ground (`ItemEntityRenderState`).
#[derive(Clone, Debug, PartialEq)]
pub struct Dropped {
    /// The item's name, such as `minecraft:stone`.
    pub item: String,
    pub pos: Vec3,
    pub age: f32,
    /// `ItemEntity.bobOffs`: a random phase for the bob and spin, in radians.
    pub bob_offset: f32,
    /// Its bounding box's width and height while it's on fire, for the flames around it.
    pub burning: Option<[f32; 2]>,
}

#[derive(Clone, Debug, PartialEq)]
// A frame's few dozen entities, built and dropped each frame: boxing would cost more.
#[allow(clippy::large_enum_variant)]
pub enum Entity {
    Living(Living),
    Item(Dropped),
}

/// An entity and the sky and block light (0 to 15) where it is.
pub type Lit = (Entity, [u8; 2]);

/// Entity quads, four vertices each.
#[derive(Default)]
pub struct Meshes {
    /// Bodies, to draw with the cutout pipeline.
    pub cutout: Vec<Vertex>,
    /// Glowing layers such as spiders' eyes (`RenderTypes.eyes`), to blend over them at
    /// full brightness without writing depth.
    pub eyes: Vec<Vertex>,
    /// Name tags' backgrounds and faint text (`DisplayMode.SEE_THROUGH`), to blend over
    /// everything drawn so far, walls included, without testing depth.
    pub see_through: Vec<Vertex>,
    /// Name tags' text, to draw opaque over `see_through` where nothing hides it.
    pub tags: Vec<Vertex>,
}

/// Quads for these entities as seen from `camera`, with their name tags unless the HUD is
/// hidden.
pub fn vertices(
    models: &crate::bake::BlockModels,
    camera: &crate::gpu::Camera,
    name_tags: bool,
    entities: &[Lit],
) -> Meshes {
    let (atlas, art) = (&models.atlas, &models.hud);
    let fire = ["minecraft:block/fire_0", "minecraft:block/fire_1"].map(|s| atlas.sprite(s).rect);
    let mut out = Meshes::default();
    for (entity, light) in entities {
        match entity {
            Entity::Living(l) => {
                // `AvatarRenderer.getRenderOffset`: a crouching player, and its name, are
                // drawn two pixels lower.
                let lowered;
                let l = if matches!(
                    l.kind,
                    Kind::Player {
                        crouching: true,
                        ..
                    }
                ) {
                    lowered = Living {
                        pos: l.pos - Vec3::Y * 2.0 / 16.0,
                        ..l.clone()
                    };
                    &lowered
                } else {
                    l
                };
                living(atlas, l, *light, &mut out);
                equipment(models, l, *light, &mut out.cutout);
                if let Some(tag) = l.name_tag.as_ref().filter(|_| name_tags) {
                    crate::hud::name_tag(art, tag, l.pos, *light, camera, &mut out);
                }
                if let Some(size) = l.burning {
                    flames(fire, l.pos, size, camera.yaw, *light, &mut out.cutout);
                }
            }
            Entity::Item(d) => {
                if let Some(icon) = art.items.get(&d.item) {
                    dropped(atlas, icon, d, *light, &mut out.cutout);
                }
                if let Some(size) = d.burning {
                    flames(fire, d.pos, size, camera.yaw, *light, &mut out.cutout);
                }
            }
        }
    }
    out
}

/// `FlameFeatureRenderer`: fire around a burning entity whose bounding box is `width` by
/// `height`, as cards of `fire_0` and `fire_1` (their atlas rects) stacked up it, each a little narrower and
/// nearer the camera than the last, turned to face a camera looking along `yaw`. Lit at
/// full block light (its light already is, for a burning entity), and shaded as faces
/// pointing up are, which is not at all.
fn flames(
    fire: [[f32; 4]; 2],
    pos: Vec3,
    [width, height]: [f32; 2],
    yaw: f32,
    [sky, block]: [u8; 2],
    out: &mut Vec<Vertex>,
) {
    let s = width * 1.4;
    let mut h = height / s;
    // `Mth.rotationAroundAxis(Y_AXIS, camera.orientation)`: the camera's turn about y.
    let m = Mat4::from_translation(pos)
        * Mat4::from_scale(Vec3::splat(s))
        * Mat4::from_rotation_y(PI - yaw.to_radians())
        * Mat4::from_translation(Vec3::new(0.0, 0.0, 0.3 - h.trunc() * 0.02));
    let (mut r, mut y, mut z) = (0.5, 0.0, 0.0);
    let mut i = 0;
    while h > 0.0 {
        let [mut u0, v0, mut u1, v1] = fire[i % 2];
        if i / 2 % 2 == 0 {
            std::mem::swap(&mut u0, &mut u1);
        }
        let corners = [
            ([-r, y, z], [u1, v1]),
            ([r, y, z], [u0, v1]),
            ([r, y + 1.4, z], [u0, v0]),
            ([-r, y + 1.4, z], [u1, v0]),
        ];
        for (p, uv) in corners {
            out.push(Vertex {
                pos: m.transform_point3(Vec3::from(p)).to_array(),
                uv,
                color: [255; 4],
                light: [sky.min(15) * 16, block.min(15) * 16, 0, 0],
            });
        }
        h -= 0.45;
        y += 0.45;
        r *= 0.9;
        z -= 0.03;
        i += 1;
    }
}

/// `LivingEntityRenderer.submit`'s transform, without the move to the entity's position:
/// turned to face `bodyRot`, fallen over as far as its death has got (`setupRotations`),
/// flipped into model space (y down, 16 units to a block), and lowered so the model's
/// feet, 24 pixels down, stand at the origin.
fn living_root(l: &Living, model_scale: f32) -> Mat4 {
    let scale = if l.baby { 0.5 } else { 1.0 };
    // Over onto its side in the first 12.5 ticks; a spider rolls onto its back
    // (`SpiderRenderer.getFlipDegrees`).
    let fall = if l.death_time > 0.0 {
        let flip = if l.kind == Kind::Spider { 180.0 } else { 90.0 };
        ((l.death_time - 1.0) / 20.0 * 1.6).sqrt().min(1.0) * flip
    } else {
        0.0
    };
    Mat4::from_scale(Vec3::splat(scale))
        * Mat4::from_rotation_y((180.0 - l.body_yaw).to_radians())
        * Mat4::from_rotation_z(fall.to_radians())
        * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
        * Mat4::from_scale(Vec3::splat(model_scale))
        * Mat4::from_translation(Vec3::new(0.0, -1.501, 0.0))
}

fn living(atlas: &Atlas, l: &Living, light: [u8; 2], out: &mut Meshes) {
    let start = out.cutout.len();
    let (nodes, tex_size, model_scale) = match l.kind {
        // `AvatarRenderer.scale`: players are drawn at 15/16 size.
        Kind::Player { slim, .. } => (player(l, slim), [64.0, 64.0], 0.9375),
        Kind::Zombie { aggressive } => (humanoid(l, HUMANOID, Some(aggressive)), [64.0; 2], 1.0),
        Kind::Skeleton => (humanoid(l, SKELETON_LIMBS, None), [64.0, 32.0], 1.0),
        Kind::Creeper => (creeper(l), [64.0, 32.0], 1.0),
        Kind::Spider => (spider(l), [64.0, 32.0], 1.0),
        Kind::Cow => (quadruped(l, &COW), [64.0, 64.0], 1.0),
        Kind::Pig => (quadruped(l, &PIG), [64.0, 64.0], 1.0),
        Kind::Sheep { .. } => (quadruped(l, &SHEEP_BODY), [64.0, 32.0], 1.0),
        Kind::Chicken => (chicken(l), [64.0, 32.0], 1.0),
    };
    let root = living_root(l, model_scale);
    let mut b = Builder {
        quads: Vec::new(),
        sprite: atlas.sprite(&l.texture),
        tex_size,
        tint: -1,
    };
    b.nodes(root, &nodes);
    emit(&b.quads, l.pos, [1.0; 3], light, &mut out.cutout);
    // A layer: the same or another model's parts, with another texture.
    let layer = |sprite: &str, nodes: &[Node]| {
        let mut b = Builder {
            quads: Vec::new(),
            sprite: atlas.sprite(sprite),
            tex_size: [64.0, 32.0],
            tint: -1,
        };
        b.nodes(root, nodes);
        b.quads
    };
    match l.kind {
        // `SpiderEyesLayer`: the eyes glow over the head, whatever the light.
        Kind::Spider => {
            for q in layer(SPIDER_EYES, &nodes) {
                for (p, uv) in q.pos.iter().zip(q.uv) {
                    out.eyes.push(Vertex {
                        pos: (l.pos + *p).to_array(),
                        uv,
                        color: [255; 4],
                        light: [240, 240, 0, 0],
                    });
                }
            }
        }
        Kind::Sheep { color, sheared } => {
            // `SheepWoolUndercoatLayer`: a coloured sheep's coat under its wool, on the
            // body model, which still shows once it's sheared. Babies have none.
            if color != 0 && !l.baby {
                let quads = layer(SHEEP_UNDERCOAT, &nodes);
                emit(&quads, l.pos, wool_color(color), light, &mut out.cutout);
            }
            // `SheepWoolLayer`: the fur model over the body, tinted with the wool's colour.
            if !sheared {
                let quads = layer(SHEEP_WOOL, &quadruped(l, &SHEEP_FUR));
                emit(&quads, l.pos, wool_color(color), light, &mut out.cutout);
            }
        }
        _ => {}
    }
    // `getOverlayCoords`: the body and the layers drawn as it is (the sheep's coats) flash;
    // glowing eyes don't.
    if l.hurt {
        for v in &mut out.cutout[start..] {
            v.light[3] |= crate::mesh::RED;
        }
    }
}

/// `ColorLerper.Type.SHEEP.getColor`: white wool is a light grey; other colours are their
/// dye's texture colour at three quarters brightness.
fn wool_color(dye: u8) -> [f32; 3] {
    if dye == 0 {
        return [0xE6 as f32 / 255.0; 3];
    }
    let c = DYE_RGB[dye as usize & 15];
    [16, 8, 0].map(|s| ((c >> s & 0xFF) as f32 * 0.75).floor() / 255.0)
}

/// Vertices for model quads placed relative to `origin`, tinted by `color`.
fn emit(quads: &[Quad], origin: Vec3, color: [f32; 3], light: [u8; 2], out: &mut Vec<Vertex>) {
    for q in quads {
        emit_quad(q.pos, q.uv, origin, color, light, out);
    }
}

/// One quad's vertices, wound so its front faces out, shaded by which way that is.
fn emit_quad(
    pos: [Vec3; 4],
    uv: [[f32; 2]; 4],
    origin: Vec3,
    color: [f32; 3],
    [sky, block]: [u8; 2],
    out: &mut Vec<Vertex>,
) {
    let normal = (pos[1] - pos[0]).cross(pos[2] - pos[0]).normalize_or_zero();
    let shade = entity_shade(normal);
    let rgb = color.map(|c| (c * shade * 255.0).round().clamp(0.0, 255.0) as u8);
    for (p, uv) in pos.iter().zip(uv) {
        out.push(Vertex {
            pos: (origin + *p).to_array(),
            uv,
            color: [rgb[0], rgb[1], rgb[2], 255],
            light: [sky.min(15) * 16, block.min(15) * 16, 0, 0],
        });
    }
}

/// `minecraft_mix_light` with `Lighting`'s two level lights: 40% ambient, plus 60% of each
/// light the face turns towards.
fn entity_shade(normal: Vec3) -> f32 {
    let l0 = Vec3::new(0.2, 1.0, -0.7).normalize();
    let l1 = Vec3::new(-0.2, 1.0, 0.7).normalize();
    (0.4 + 0.6 * (normal.dot(l0).max(0.0) + normal.dot(l1).max(0.0))).min(1.0)
}

const fn mirrored(c: Cube) -> Cube {
    Cube { mirror: true, ..c }
}

const fn grown(c: Cube, grow: f32) -> Cube {
    Cube { grow, ..c }
}

/// A part's pose: offset in pixels, then rotation in radians.
fn part(
    parent: Option<usize>,
    [x, y, z]: [f32; 3],
    [rx, ry, rz]: [f32; 3],
    cubes: &'static [Cube],
) -> Node {
    Node {
        parent,
        pose: [x, y, z, rx, ry, rz],
        cubes,
    }
}

/// `HumanoidModel.createMesh`'s head and hat.
const HEAD: &[Cube] = &[cube([-4.0, -8.0, -4.0], [8.0; 3], [0.0, 0.0])];
const HAT: &[Cube] = &[grown(cube([-4.0, -8.0, -4.0], [8.0; 3], [32.0, 0.0]), 0.5)];
const BODY: &[Cube] = &[cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 16.0])];

/// A humanoid's arms and legs: right arm, left arm, right leg, left leg, and the legs'
/// offset from the middle.
#[derive(Clone, Copy)]
struct Limbs {
    cubes: [&'static [Cube]; 4],
    leg_x: f32,
}

/// `HumanoidModel.createMesh`'s, used by zombies.
const HUMANOID: Limbs = Limbs {
    cubes: [
        &[cube([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 16.0])],
        &[mirrored(cube(
            [-1.0, -2.0, -2.0],
            [4.0, 12.0, 4.0],
            [40.0, 16.0],
        ))],
        &[cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0])],
        &[mirrored(cube(
            [-2.0, 0.0, -2.0],
            [4.0, 12.0, 4.0],
            [0.0, 16.0],
        ))],
    ],
    leg_x: 1.9,
};

/// `SkeletonModel.createDefaultSkeletonMesh`: two pixels thick.
const SKELETON_LIMBS: Limbs = Limbs {
    cubes: [
        &[cube([-1.0, -2.0, -1.0], [2.0, 12.0, 2.0], [40.0, 16.0])],
        &[mirrored(cube(
            [-1.0, -2.0, -1.0],
            [2.0, 12.0, 2.0],
            [40.0, 16.0],
        ))],
        &[cube([-1.0, 0.0, -1.0], [2.0, 12.0, 2.0], [0.0, 16.0])],
        &[mirrored(cube(
            [-1.0, 0.0, -1.0],
            [2.0, 12.0, 2.0],
            [0.0, 16.0],
        ))],
    ],
    leg_x: 2.0,
};

/// `HumanoidModel.setupAnim` with empty hands, standing: the head looks, arms and legs
/// swing with the walk, and the arms bob (`AnimationUtils.bobModelPart`). A zombie's arms
/// are held out instead (`AnimationUtils.animateZombieArms`, with no attack swinging).
/// Returns the head, hat, body, right arm, left arm, right leg and left leg.
fn humanoid(l: &Living, limbs: Limbs, zombie: Option<bool>) -> Vec<Node> {
    humanoid_parts(l, [HEAD, HAT, BODY], limbs, zombie)
}

/// [`humanoid`] with these head, hat and body cubes, as armour models are posed.
fn humanoid_parts(
    l: &Living,
    [head, hat, body]: [&'static [Cube]; 3],
    limbs: Limbs,
    zombie: Option<bool>,
) -> Vec<Node> {
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let mut right_arm = [(p + PI).cos() * 2.0 * s * 0.5, 0.0, 0.0];
    let mut left_arm = [p.cos() * 2.0 * s * 0.5, 0.0, 0.0];
    // `HumanoidModel.poseRightArm` and `poseLeftArm`: a player's arm holding an item
    // (`ArmPose.ITEM`) swings half as far, raised a little. Mobs' arms stay `EMPTY`.
    if matches!(l.kind, Kind::Player { .. }) {
        for (arm, held) in [&mut right_arm, &mut left_arm]
            .into_iter()
            .zip(&l.equipment.hands)
        {
            if held.is_some() {
                arm[0] = arm[0] * 0.5 - PI / 10.0;
            }
        }
    }
    // `HumanoidModel.setupAttackAnimation`: the body twists with the swing, carrying the
    // shoulders round, and a whack swings the arm down and across.
    let (mut body_twist, mut right_pivot, mut left_pivot) = (0.0, [-5.0, 0.0], [5.0, 0.0]);
    let swing = l.swing.filter(|s| s.progress > 0.0);
    if let Some(sw) = swing {
        let t = sw.progress;
        body_twist = (t.sqrt() * PI * 2.0).sin() * 0.2 * if sw.left { -1.0 } else { 1.0 };
        let (sin, cos) = body_twist.sin_cos();
        right_pivot = [-cos * 5.0, sin * 5.0];
        left_pivot = [cos * 5.0, -sin * 5.0];
        right_arm[1] += body_twist;
        left_arm[1] += body_twist;
        left_arm[0] += body_twist;
        // `SwingAnimationType.WHACK`; stabbing (spears) isn't posed yet.
        if sw.kind == 1 {
            let arm = if sw.left {
                &mut left_arm
            } else {
                &mut right_arm
            };
            let out_quart = 1.0 - (1.0 - t).powi(4);
            let a = (out_quart * PI).sin();
            let b = (t * PI).sin() * -(l.pitch.to_radians() - 0.7) * 0.75;
            arm[0] -= a * 1.2 + b;
            arm[1] += body_twist * 2.0;
            arm[2] += (t * PI).sin() * -0.4;
        }
    }
    // `AnimationUtils.animateZombieArms`: both arms held out, higher when aggressive (but
    // down for a baby holding something), swinging together.
    if let Some(aggressive) = zombie {
        let t = swing.map_or(0.0, |s| s.progress);
        let raise = !l.baby || l.equipment.hands[l.equipment.left_handed as usize].is_none();
        let drop = if raise {
            -PI / if aggressive { 1.5 } else { 2.25 }
        } else {
            0.0
        };
        let ay = if raise { 1.0 } else { -1.0 } * (t * PI).sin();
        let ax = ((1.0 - (1.0 - t) * (1.0 - t)) * PI).sin();
        let (x, y) = (drop + ay * 1.2 - ax * 0.4, 0.1 - ay * 0.6);
        let sign = if raise { 1.0 } else { -1.0 };
        right_arm = [x, -y * sign, 0.0];
        left_arm = [x, y * sign, 0.0];
    }
    // `HumanoidModel.setupAnim`'s crouch: the body leans forward and drops with the arms,
    // the head drops further, and the legs move back.
    let crouching = matches!(
        l.kind,
        Kind::Player {
            crouching: true,
            ..
        }
    );
    let (head_y, body_y, leg_z, body_x) = if crouching {
        right_arm[0] += 0.4;
        left_arm[0] += 0.4;
        (4.2, 3.2, 4.0, 0.5)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    // `AnimationUtils.bobModelPart`, last.
    for (arm, sign) in [(&mut right_arm, 1.0), (&mut left_arm, -1.0)] {
        arm[2] += sign * ((l.age * 0.09).cos() * 0.05 + 0.05);
        arm[0] += sign * (l.age * 0.067).sin() * 0.05;
    }
    let [ra, la, rl, ll] = limbs.cubes;
    vec![
        part(
            None,
            [0.0, head_y, 0.0],
            [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0],
            head,
        ),
        part(Some(0), [0.0; 3], [0.0; 3], hat),
        part(None, [0.0, body_y, 0.0], [body_x, body_twist, 0.0], body),
        part(
            None,
            [right_pivot[0], 2.0 + body_y, right_pivot[1]],
            right_arm,
            ra,
        ),
        part(
            None,
            [left_pivot[0], 2.0 + body_y, left_pivot[1]],
            left_arm,
            la,
        ),
        part(
            None,
            [-limbs.leg_x, 12.0, leg_z],
            [p.cos() * 1.4 * s, 0.005, 0.005],
            rl,
        ),
        part(
            None,
            [limbs.leg_x, 12.0, leg_z],
            [(p + PI).cos() * 1.4 * s, -0.005, -0.005],
            ll,
        ),
    ]
}

/// `HumanoidModel.createArmorMeshSet`, as `LayerDefinitions` builds it for humanoids and
/// players (64 × 32 textures): helmets and boots are 1 pixel out from the body
/// (`OUTER_ARMOR_DEFORMATION`), chestplates too, and leggings half a pixel
/// (`INNER_ARMOR_DEFORMATION`); legs are a tenth of a pixel thinner. Head, chest, legs and
/// feet: each the head, hat and body cubes and the limbs.
const ARMOR: [([&[Cube]; 3], Limbs); 4] = {
    const fn box_(from: [f32; 3], size: [f32; 3], tex: [f32; 2], grow: f32) -> Cube {
        grown(cube(from, size, tex), grow)
    }
    const RIGHT_ARM: Cube = cube([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 16.0]);
    const LEFT_ARM: Cube = mirrored(cube([-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 16.0]));
    const RIGHT_LEG: Cube = cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0]);
    const LEFT_LEG: Cube = mirrored(RIGHT_LEG);
    const NONE: &[Cube] = &[];
    const HEAD: [&[Cube]; 3] = [
        &[box_([-4.0, -8.0, -4.0], [8.0; 3], [0.0, 0.0], 1.0)],
        &[box_([-4.0, -8.0, -4.0], [8.0; 3], [32.0, 0.0], 1.5)],
        NONE,
    ];
    const OUTER_BODY: &[Cube] = &[box_([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 16.0], 1.0)];
    const INNER_BODY: &[Cube] = &[box_([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 16.0], 0.5)];
    const CHEST_ARMS: [&[Cube]; 2] = [&[grown(RIGHT_ARM, 1.0)], &[grown(LEFT_ARM, 1.0)]];
    const LEGGINGS: [&[Cube]; 2] = [&[grown(RIGHT_LEG, 0.4)], &[grown(LEFT_LEG, 0.4)]];
    const BOOTS: [&[Cube]; 2] = [&[grown(RIGHT_LEG, 0.9)], &[grown(LEFT_LEG, 0.9)]];
    const fn limbs(cubes: [&'static [Cube]; 4]) -> Limbs {
        Limbs { cubes, leg_x: 1.9 }
    }
    [
        (HEAD, limbs([NONE; 4])),
        (
            [NONE, NONE, OUTER_BODY],
            limbs([CHEST_ARMS[0], CHEST_ARMS[1], NONE, NONE]),
        ),
        (
            [NONE, NONE, INNER_BODY],
            limbs([NONE, NONE, LEGGINGS[0], LEGGINGS[1]]),
        ),
        ([NONE; 3], limbs([NONE, NONE, BOOTS[0], BOOTS[1]])),
    ]
};

/// For a humanoid that can wear armour and hold items: whether it's a zombie (aggressive
/// or not) as `humanoid` poses it, its model scale, and how far its hands sit out from its
/// arms' pivots (`translateToHand`: slim players' half a pixel, skeletons' a pixel). None
/// for other mobs.
fn humanoid_kind(l: &Living) -> Option<(Option<bool>, f32, f32)> {
    match l.kind {
        Kind::Player { slim, .. } => Some((None, 0.9375, if slim { 0.5 } else { 0.0 })),
        Kind::Zombie { aggressive } => Some((Some(aggressive), 1.0, 0.0)),
        Kind::Skeleton => Some((None, 1.0, 1.0)),
        _ => None,
    }
}

/// `HumanoidArmorLayer` and `ItemInHandLayer`: a humanoid's armour, chest, legs, feet then
/// head, each of its equipment asset's layers tinted by its dye, on armour models posed as
/// the body is; then the items in its hands.
fn equipment(models: &crate::bake::BlockModels, l: &Living, light: [u8; 2], out: &mut Vec<Vertex>) {
    let Some((zombie, model_scale, hand_x)) = humanoid_kind(l) else {
        return;
    };
    let root = living_root(l, model_scale);
    for slot in [1, 2, 3, 0] {
        let Some(armor) = &l.equipment.armor[slot] else {
            continue;
        };
        let Some(asset) = models.equipment.get(&armor.asset) else {
            continue;
        };
        // `usesInnerModel`: leggings have their own layer textures.
        let layers = if slot == 2 {
            &asset.leggings
        } else {
            &asset.humanoid
        };
        let (parts, armor_limbs) = &ARMOR[slot];
        let nodes = humanoid_parts(l, *parts, *armor_limbs, zombie);
        for (sprite, dyeable) in layers {
            // `getColorForLayer`: a dyeable layer takes the dye, or its undyed colour;
            // with neither it's not drawn.
            let color = match dyeable {
                Some(undyed) => armor.dye.map_or(*undyed, |d| d | 0xff00_0000),
                None => 0xffff_ffff,
            };
            if color == 0 {
                continue;
            }
            let mut b = Builder {
                quads: Vec::new(),
                sprite: models.atlas.sprite(sprite),
                tex_size: [64.0, 32.0],
                tint: -1,
            };
            b.nodes(root, &nodes);
            let rgb = [16, 8, 0].map(|s| (color >> s & 0xff) as f32 / 255.0);
            emit(&b.quads, l.pos, rgb, light, out);
        }
    }
    // `ItemInHandLayer.submitArmWithItem`: from each hand, turned to point the item
    // forward, then placed by its model's hand transform.
    let nodes = humanoid_parts(l, [&[]; 3], ARMOR[3].1, zombie);
    for (i, held) in l.equipment.hands.iter().enumerate() {
        let Some(icon) = held.as_ref().and_then(|item| models.hud.items.get(item)) else {
            continue;
        };
        let left = i == 1;
        let sign = if left { -1.0 } else { 1.0 };
        let [x, y, z, arm_x, arm_y, arm_z] = nodes[3 + i].pose;
        let hand = root
            * crate::entity_model::pose([x + sign * hand_x, y, z], [arm_x, arm_y, arm_z])
            * Mat4::from_rotation_x(-PI / 2.0)
            * Mat4::from_rotation_y(PI)
            * Mat4::from_translation(Vec3::new(sign, 2.0, -10.0) / 16.0);
        let m = hand * item_transform(icon.hand(left), left);
        for (corners, uv, tint) in item_quads(&models.atlas, icon) {
            emit_quad(
                corners.map(|c| m.transform_point3(c)),
                uv,
                l.pos,
                tint,
                light,
                out,
            );
        }
    }
}

/// `ItemTransform.apply`, mirrored for the left hand, then the model centred, as
/// `ItemStackRenderState.submit` places an item.
fn item_transform(t: mc_assets::Transform, left: bool) -> Mat4 {
    let [rx, ry, rz] = t.rotation.map(f32::to_radians);
    let [tx, ty, tz] = t.translation;
    let (tx, ry, rz) = if left { (-tx, -ry, -rz) } else { (tx, ry, rz) };
    Mat4::from_translation(Vec3::new(tx, ty, tz) / 16.0)
        * Mat4::from_quat(Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz))
        * Mat4::from_scale(Vec3::from(t.scale))
        * Mat4::from_translation(Vec3::splat(-0.5))
}

/// What the player sees of their own hands (`FirstPersonHandsAndItemsRenderState`),
/// already interpolated to the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FirstPerson {
    /// The player's skin sprite, and whether it has slim arms.
    pub skin: String,
    pub slim: bool,
    /// Whether their main hand is their left.
    pub left_handed: bool,
    /// The items shown in the main and off hand, by name; they lag the real ones while
    /// the hand dips to swap them.
    pub items: [Option<String>; 2],
    /// How far each hand is lowered, 0 (up) to 1 (out of sight): `1 - handHeight`.
    pub lowered: [f32; 2],
    /// The swing in progress, if any: which hand (0 main, 1 off), its `SwingAnimationType`
    /// (whack 1), and how far through it is, 0 to 1.
    pub swing: Option<(usize, u8, f32)>,
    /// How far the view has turned past where the hands have caught up to, in degrees
    /// (`viewXRot - xBob` and `viewYRot - yBob`); the hands lag a tenth of it.
    pub sway: [f32; 2],
}

/// `GameRenderer.renderItemInHand` and `FirstPersonHandsAndItemsRenderer.submitHandsWithItems`:
/// the player's arm and the items they hold, as world-space quads around `camera.eye`,
/// lit with `light`, for [`crate::gpu::Renderer::set_hand`] to draw over the world.
pub fn first_person(
    models: &crate::bake::BlockModels,
    camera: &crate::gpu::Camera,
    hands: &FirstPerson,
    light: [u8; 2],
) -> Vec<Vertex> {
    let mut out = Vec::new();
    // The pose stack starts as the inverse view rotation (`Camera.rotation`), so the
    // quads end up in world space and the world's entity lights shade them.
    let view = Mat4::from_quat(
        Quat::from_rotation_y(PI - camera.yaw.to_radians())
            * Quat::from_rotation_x(-camera.pitch.to_radians()),
    );
    let root = view
        * Mat4::from_rotation_x((hands.sway[0] * 0.1).to_radians())
        * Mat4::from_rotation_y((hands.sway[1] * 0.1).to_radians());
    let deg = |d: f32| d.to_radians();
    for (hand, item) in hands.items.iter().enumerate() {
        // `submitArmWithItem`: which arm holds this hand, and its swing.
        let left = (hand == 1) != hands.left_handed;
        let invert = if left { -1.0 } else { 1.0 };
        let swing = hands.swing.filter(|&(h, ..)| h == hand);
        // `attackHand` is the main hand when nothing swings.
        let attack = match hands.swing {
            Some((h, _, progress)) if h == hand => progress,
            _ => 0.0,
        };
        let lowered = hands.lowered[hand];
        let Some(item) = item else {
            // `renderPlayerArm`: only an empty main hand shows the arm.
            if hand != 0 {
                continue;
            }
            let sqrt = attack.sqrt();
            let m =
                root * Mat4::from_translation(Vec3::new(
                    invert * (-0.3 * (sqrt * PI).sin() + 0.64),
                    0.4 * (sqrt * PI * 2.0).sin() - 0.6 + lowered * -0.6,
                    -0.4 * (attack * PI).sin() - 0.72,
                )) * Mat4::from_rotation_y(deg(invert * 45.0))
                    * Mat4::from_rotation_y(deg(invert * (sqrt * PI).sin() * 70.0))
                    * Mat4::from_rotation_z(deg(invert * (attack * attack * PI).sin() * -20.0))
                    * Mat4::from_translation(Vec3::new(-invert, 3.6, 3.5))
                    * Mat4::from_rotation_z(deg(invert * 120.0))
                    * Mat4::from_rotation_x(deg(200.0))
                    * Mat4::from_rotation_y(deg(invert * -135.0))
                    * Mat4::from_translation(Vec3::new(invert * 5.6, 0.0, 0.0));
            // `AvatarRenderer.renderHand`: the arm at its resting offset, tilted a little
            // out, with its sleeve.
            let limbs = if hands.slim { PLAYER_SLIM } else { PLAYER_WIDE };
            let arm = part(
                None,
                [invert * -5.0, 2.0, 0.0],
                [0.0, 0.0, invert * 0.1],
                limbs.cubes[left as usize],
            );
            let mut b = Builder {
                quads: Vec::new(),
                sprite: models.atlas.sprite(&hands.skin),
                tex_size: [64.0, 64.0],
                tint: -1,
            };
            b.nodes(m, &[arm]);
            emit(&b.quads, camera.eye, [1.0; 3], light, &mut out);
            continue;
        };
        let Some(icon) = models.hud.items.get(item) else {
            continue;
        };
        // `applyItemArmTransform`, then `swingArm` for a whack in this hand.
        let mut m =
            root * Mat4::from_translation(Vec3::new(invert * 0.56, -0.52 + lowered * -0.6, -0.72));
        if swing.is_some_and(|(_, kind, _)| kind == 1) {
            let sqrt = attack.sqrt();
            let xz = (sqrt * PI).sin();
            m = m
                * Mat4::from_translation(Vec3::new(
                    invert * -0.4 * xz,
                    0.2 * (sqrt * PI * 2.0).sin(),
                    -0.2 * (attack * PI).sin(),
                ))
                // `applyItemArmAttackTransform`.
                * Mat4::from_rotation_y(deg(invert * (45.0 + (attack * attack * PI).sin() * -20.0)))
                * Mat4::from_rotation_z(deg(invert * xz * -20.0))
                * Mat4::from_rotation_x(deg(xz * -80.0))
                * Mat4::from_rotation_y(deg(invert * -45.0));
        }
        let m = m * item_transform(icon.first_person_hand(left), left);
        for (corners, uv, tint) in item_quads(&models.atlas, icon) {
            emit_quad(
                corners.map(|c| m.transform_point3(c)),
                uv,
                camera.eye,
                tint,
                light,
                &mut out,
            );
        }
    }
    out
}

/// `ScreenEffectRenderer.submitFire`: what a burning player sees, two cards of `fire_1`
/// half a block ahead, mirrored, low in the view and turned 10° in from each side, drawn
/// translucent and unlit over everything, the hand too. They're placed where `camera`
/// sees them as the hand's 70° view would, for [`crate::gpu::Renderer::set_fire_overlay`].
pub fn fire_overlay(atlas: &Atlas, camera: &crate::gpu::Camera) -> Vec<Vertex> {
    let [u0, v0, u1, v1] = atlas.sprite("minecraft:block/fire_1").rect;
    // The hand pass's view is `camera.view()` (the screen effects don't bob), so undo it.
    let to_world = camera.view().inverse();
    let mut out = Vec::new();
    for side in [1.0, -1.0] {
        let m = to_world
            * Mat4::from_translation(Vec3::new(side * 0.24, -0.3, 0.0))
            * Mat4::from_rotation_y(side * -PI / 18.0);
        // `buildSpriteQuad`: the sprite's right edge at the left.
        let corners = [
            ([-0.5, -0.5], [u1, v1]),
            ([0.5, -0.5], [u0, v1]),
            ([0.5, 0.5], [u0, v0]),
            ([-0.5, 0.5], [u1, v0]),
        ];
        for ([x, y], uv) in corners {
            out.push(Vertex {
                pos: m.transform_point3(Vec3::new(x, y, -0.5)).to_array(),
                uv,
                // `-436207617`: white at 0xE5 alpha.
                color: [255, 255, 255, 0xE5],
                light: [0, 0, 0, crate::mesh::UNLIT],
            });
        }
    }
    out
}

/// How far a player's outer layer (jacket, sleeves, trousers) sits out, in pixels.
const PLAYER_LAYER: f32 = 0.25;
/// `PlayerModel.createMesh`'s limbs: wide or slim arms, each with its sleeve.
const PLAYER_WIDE: Limbs = Limbs {
    cubes: [
        &[
            cube([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 16.0]),
            grown(
                cube([-3.0, -2.0, -2.0], [4.0, 12.0, 4.0], [40.0, 32.0]),
                PLAYER_LAYER,
            ),
        ],
        &[
            cube([-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], [32.0, 48.0]),
            grown(
                cube([-1.0, -2.0, -2.0], [4.0, 12.0, 4.0], [48.0, 48.0]),
                PLAYER_LAYER,
            ),
        ],
        PLAYER_LEGS[0],
        PLAYER_LEGS[1],
    ],
    leg_x: 1.9,
};
const PLAYER_SLIM: Limbs = Limbs {
    cubes: [
        &[
            cube([-2.0, -2.0, -2.0], [3.0, 12.0, 4.0], [40.0, 16.0]),
            grown(
                cube([-2.0, -2.0, -2.0], [3.0, 12.0, 4.0], [40.0, 32.0]),
                PLAYER_LAYER,
            ),
        ],
        &[
            cube([-1.0, -2.0, -2.0], [3.0, 12.0, 4.0], [32.0, 48.0]),
            grown(
                cube([-1.0, -2.0, -2.0], [3.0, 12.0, 4.0], [48.0, 48.0]),
                PLAYER_LAYER,
            ),
        ],
        PLAYER_LEGS[0],
        PLAYER_LEGS[1],
    ],
    leg_x: 1.9,
};
const PLAYER_LEGS: [&[Cube]; 2] = [
    &[
        cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0]),
        grown(
            cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 32.0]),
            PLAYER_LAYER,
        ),
    ],
    &[
        cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [16.0, 48.0]),
        grown(
            cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 48.0]),
            PLAYER_LAYER,
        ),
    ],
];

/// `PlayerModel.createMesh`: a humanoid with its own left limbs' texture, wide or slim
/// arms, and the outer layer (jacket, sleeves, trousers) a quarter pixel out.
fn player(l: &Living, slim: bool) -> Vec<Node> {
    const JACKET: &[Cube] = &[grown(
        cube([-4.0, 0.0, -2.0], [8.0, 12.0, 4.0], [16.0, 32.0]),
        PLAYER_LAYER,
    )];
    let mut nodes = humanoid(l, if slim { PLAYER_SLIM } else { PLAYER_WIDE }, None);
    nodes.push(part(Some(2), [0.0; 3], [0.0; 3], JACKET));
    nodes
}

/// `CreeperModel`: a head and body on four short legs. Its fields swap the legs' names,
/// so the part called `right_hind_leg` swings as `leftHindLeg`, and so on.
fn creeper(l: &Living) -> Vec<Node> {
    const LEG: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 6.0, 4.0], [0.0, 16.0])];
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let a = p.cos() * 1.4 * s;
    let b = (p + PI).cos() * 1.4 * s;
    vec![
        part(
            None,
            [0.0, 6.0, 0.0],
            [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0],
            HEAD,
        ),
        part(None, [0.0, 6.0, 0.0], [0.0; 3], BODY),
        part(None, [-2.0, 18.0, 4.0], [b, 0.0, 0.0], LEG),
        part(None, [2.0, 18.0, 4.0], [a, 0.0, 0.0], LEG),
        part(None, [-2.0, 18.0, -4.0], [a, 0.0, 0.0], LEG),
        part(None, [2.0, 18.0, -4.0], [b, 0.0, 0.0], LEG),
    ]
}

/// `SpiderModel`: head, neck and abdomen, and eight legs splayed out, each pair swinging
/// and lifting a quarter cycle apart.
fn spider(l: &Living) -> Vec<Node> {
    const RIGHT: &[Cube] = &[cube([-15.0, -1.0, -1.0], [16.0, 2.0, 2.0], [18.0, 0.0])];
    const LEFT: &[Cube] = &[mirrored(cube(
        [-1.0, -1.0, -1.0],
        [16.0, 2.0, 2.0],
        [18.0, 0.0],
    ))];
    let p = l.walk_pos * 0.6662;
    let s = l.walk_speed;
    // Hind, middle hind, middle front, front.
    let phase = [0.0, PI, PI / 2.0, PI * 3.0 / 2.0];
    let swing = phase.map(|o| -((p * 2.0 + o).cos() * 0.4) * s);
    let step = phase.map(|o| ((p + o).sin() * 0.4).abs() * s);
    let (q, e) = (PI / 4.0, PI / 8.0);
    let z = 0.58119464;
    // Each pair: z offset, its rest yaw and roll for the right leg (the left mirrors them).
    let legs = [(2.0, q, -q), (1.0, e, -z), (0.0, -e, -z), (-1.0, -q, -q)];
    const SPIDER_HEAD: &[Cube] = &[cube([-4.0, -4.0, -8.0], [8.0; 3], [32.0, 4.0])];
    const NECK: &[Cube] = &[cube([-3.0; 3], [6.0; 3], [0.0, 0.0])];
    const ABDOMEN: &[Cube] = &[cube([-5.0, -4.0, -6.0], [10.0, 8.0, 12.0], [0.0, 12.0])];
    let look = [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0];
    let mut nodes = vec![
        part(None, [0.0, 15.0, -3.0], look, SPIDER_HEAD),
        part(None, [0.0, 15.0, 0.0], [0.0; 3], NECK),
        part(None, [0.0, 15.0, 9.0], [0.0; 3], ABDOMEN),
    ];
    for (i, (z_offset, yaw, roll)) in legs.into_iter().enumerate() {
        nodes.push(part(
            None,
            [-4.0, 15.0, z_offset],
            [0.0, yaw + swing[i], roll + step[i]],
            RIGHT,
        ));
        nodes.push(part(
            None,
            [4.0, 15.0, z_offset],
            [0.0, -yaw - swing[i], -roll - step[i]],
            LEFT,
        ));
    }
    nodes
}

/// A `QuadrupedModel`: head, body (lying along z), and legs at their offsets: right hind,
/// left hind, right front, left front.
struct Quadruped {
    head: (&'static [Cube], [f32; 3]),
    body: (&'static [Cube], [f32; 3]),
    legs: [(&'static [Cube], [f32; 3]); 4],
}

/// `CowModel.createBaseCowModel`.
const COW: Quadruped = {
    const RIGHT: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0])];
    const LEFT: &[Cube] = &[mirrored(cube(
        [-2.0, 0.0, -2.0],
        [4.0, 12.0, 4.0],
        [0.0, 16.0],
    ))];
    Quadruped {
        head: (
            &[
                cube([-4.0, -4.0, -6.0], [8.0, 8.0, 6.0], [0.0, 0.0]),
                cube([-3.0, 1.0, -7.0], [6.0, 3.0, 1.0], [1.0, 33.0]),
                cube([-5.0, -5.0, -5.0], [1.0, 3.0, 1.0], [22.0, 0.0]),
                cube([4.0, -5.0, -5.0], [1.0, 3.0, 1.0], [22.0, 0.0]),
            ],
            [0.0, 4.0, -8.0],
        ),
        body: (
            &[
                cube([-6.0, -10.0, -7.0], [12.0, 18.0, 10.0], [18.0, 4.0]),
                cube([-2.0, 2.0, -8.0], [4.0, 6.0, 1.0], [52.0, 0.0]),
            ],
            [0.0, 5.0, 2.0],
        ),
        legs: [
            (RIGHT, [-4.0, 12.0, 7.0]),
            (LEFT, [4.0, 12.0, 7.0]),
            (RIGHT, [-4.0, 12.0, -5.0]),
            (LEFT, [4.0, 12.0, -5.0]),
        ],
    }
};

/// `PigModel.createBasePigModel`: `QuadrupedModel.createBodyMesh(6)` with a snout, and its
/// left legs mirrored.
const PIG: Quadruped = {
    const RIGHT: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 6.0, 4.0], [0.0, 16.0])];
    const LEFT: &[Cube] = &[mirrored(cube(
        [-2.0, 0.0, -2.0],
        [4.0, 6.0, 4.0],
        [0.0, 16.0],
    ))];
    Quadruped {
        head: (
            &[
                cube([-4.0, -4.0, -8.0], [8.0; 3], [0.0, 0.0]),
                cube([-2.0, 0.0, -9.0], [4.0, 3.0, 1.0], [16.0, 16.0]),
            ],
            [0.0, 12.0, -6.0],
        ),
        body: (
            &[cube([-5.0, -10.0, -7.0], [10.0, 16.0, 8.0], [28.0, 8.0])],
            [0.0, 11.0, 2.0],
        ),
        legs: [
            (RIGHT, [-3.0, 18.0, 7.0]),
            (LEFT, [3.0, 18.0, 7.0]),
            (RIGHT, [-3.0, 18.0, -5.0]),
            (LEFT, [3.0, 18.0, -5.0]),
        ],
    }
};

/// `SheepModel.createBodyLayer`: `createBodyMesh(12)` with its right legs mirrored.
const SHEEP_BODY: Quadruped = {
    const RIGHT: &[Cube] = &[mirrored(cube(
        [-2.0, 0.0, -2.0],
        [4.0, 12.0, 4.0],
        [0.0, 16.0],
    ))];
    const LEFT: &[Cube] = &[cube([-2.0, 0.0, -2.0], [4.0, 12.0, 4.0], [0.0, 16.0])];
    Quadruped {
        head: (
            &[cube([-3.0, -4.0, -6.0], [6.0, 6.0, 8.0], [0.0, 0.0])],
            [0.0, 6.0, -8.0],
        ),
        body: (
            &[cube([-4.0, -10.0, -7.0], [8.0, 16.0, 6.0], [28.0, 8.0])],
            [0.0, 5.0, 2.0],
        ),
        legs: [
            (RIGHT, [-3.0, 12.0, 7.0]),
            (LEFT, [3.0, 12.0, 7.0]),
            (RIGHT, [-3.0, 12.0, -5.0]),
            (LEFT, [3.0, 12.0, -5.0]),
        ],
    }
};

/// `SheepFurModel.createFurLayer`: the wool, grown out from the body.
const SHEEP_FUR: Quadruped = {
    const LEG: &[Cube] = &[grown(
        cube([-2.0, 0.0, -2.0], [4.0, 6.0, 4.0], [0.0, 16.0]),
        0.5,
    )];
    Quadruped {
        head: (
            &[grown(cube([-3.0, -4.0, -4.0], [6.0; 3], [0.0, 0.0]), 0.6)],
            [0.0, 6.0, -8.0],
        ),
        body: (
            &[grown(
                cube([-4.0, -10.0, -7.0], [8.0, 16.0, 6.0], [28.0, 8.0]),
                1.75,
            )],
            [0.0, 5.0, 2.0],
        ),
        legs: [
            (LEG, [-3.0, 12.0, 7.0]),
            (LEG, [3.0, 12.0, 7.0]),
            (LEG, [-3.0, 12.0, -5.0]),
            (LEG, [3.0, 12.0, -5.0]),
        ],
    }
};

/// `QuadrupedModel.setupAnim`: the head looks, and diagonal legs swing together.
fn quadruped(l: &Living, q: &Quadruped) -> Vec<Node> {
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let a = p.cos() * 1.4 * s;
    let b = (p + PI).cos() * 1.4 * s;
    let mut nodes = vec![
        part(
            None,
            q.head.1,
            [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0],
            q.head.0,
        ),
        part(None, q.body.1, [PI / 2.0, 0.0, 0.0], q.body.0),
    ];
    for ((cubes, at), swing) in q.legs.into_iter().zip([a, b, b, a]) {
        nodes.push(part(None, at, [swing, 0.0, 0.0], cubes));
    }
    nodes
}

/// `AdultChickenModel`: head with beak and wattle, body, legs that walk and wings that
/// flap (`ChickenModel.setupAnim`).
fn chicken(l: &Living) -> Vec<Node> {
    const CHICKEN_HEAD: &[Cube] = &[cube([-2.0, -6.0, -2.0], [4.0, 6.0, 3.0], [0.0, 0.0])];
    const BEAK: &[Cube] = &[cube([-2.0, -4.0, -4.0], [4.0, 2.0, 2.0], [14.0, 0.0])];
    const WATTLE: &[Cube] = &[cube([-1.0, -2.0, -3.0], [2.0; 3], [14.0, 4.0])];
    const CHICKEN_BODY: &[Cube] = &[cube([-3.0, -4.0, -3.0], [6.0, 8.0, 6.0], [0.0, 9.0])];
    const LEG: &[Cube] = &[cube([-1.0, 0.0, -3.0], [3.0, 5.0, 3.0], [26.0, 0.0])];
    const RIGHT_WING: &[Cube] = &[cube([0.0, 0.0, -3.0], [1.0, 4.0, 6.0], [24.0, 13.0])];
    const LEFT_WING: &[Cube] = &[cube([-1.0, 0.0, -3.0], [1.0, 4.0, 6.0], [24.0, 13.0])];
    let (p, s) = (l.walk_pos * 0.6662, l.walk_speed);
    let flap = (l.flap.sin() + 1.0) * l.flap_speed;
    let look = [l.pitch.to_radians(), l.head_yaw.to_radians(), 0.0];
    vec![
        part(None, [0.0, 15.0, -4.0], look, CHICKEN_HEAD),
        part(Some(0), [0.0; 3], [0.0; 3], BEAK),
        part(Some(0), [0.0; 3], [0.0; 3], WATTLE),
        part(None, [0.0, 16.0, 0.0], [PI / 2.0, 0.0, 0.0], CHICKEN_BODY),
        part(None, [-2.0, 19.0, 1.0], [p.cos() * 1.4 * s, 0.0, 0.0], LEG),
        part(
            None,
            [1.0, 19.0, 1.0],
            [(p + PI).cos() * 1.4 * s, 0.0, 0.0],
            LEG,
        ),
        part(None, [-4.0, 13.0, 0.0], [0.0, 0.0, flap], RIGHT_WING),
        part(None, [4.0, 13.0, 0.0], [0.0, 0.0, -flap], LEFT_WING),
    ]
}

/// `ItemEntityRenderer.submit` for one item: bobbing above the ground, its lowest point a
/// pixel up, spinning, in the model's `ground` transform. Flat items are their sprite's
/// front and back, a pixel apart, as `item/generated` (whose `ground` they all share)
/// places them; vanilla also fills in the pixel-thick edges.
/// Corners, uv and colour of one of an item's quads, in the item's own 0..1 space.
type ItemQuad = ([Vec3; 4], [[f32; 2]; 4], [f32; 3]);

/// `ItemModelGenerator`: a sprite as a card from z 7.5 to 8.5 pixels, its front and back
/// the whole sprite, and a side face along each edge between an opaque pixel and a clear
/// one (or the sprite's edge), textured with that pixel. Sides facing left and right are
/// shaded as east and west.
fn extrude(atlas: &Atlas, rect: [f32; 4], tint: [f32; 3], lift: f32, out: &mut Vec<ItemQuad>) {
    let [u0, v0, u1, v1] = rect;
    let (front, back) = (8.5 / 16.0 + lift, 7.5 / 16.0 - lift);
    let v = Vec3::new;
    out.push((
        [
            v(0.0, 0.0, front),
            v(1.0, 0.0, front),
            v(1.0, 1.0, front),
            v(0.0, 1.0, front),
        ],
        [[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
        tint,
    ));
    out.push((
        [
            v(1.0, 0.0, back),
            v(0.0, 0.0, back),
            v(0.0, 1.0, back),
            v(1.0, 1.0, back),
        ],
        [[u1, v1], [u0, v1], [u0, v0], [u1, v0]],
        tint,
    ));
    let sprite = crate::atlas::Sprite {
        rect,
        layer: crate::atlas::Layer::Cutout,
    };
    let [w, h] = atlas.sprite_size(sprite);
    let clear = |x: i32, y: i32| {
        x < 0
            || y < 0
            || x >= w as i32
            || y >= h as i32
            || atlas.alpha(sprite, x as u32, y as u32) == 0
    };
    let (sx, sy) = (1.0 / w as f32, 1.0 / h as f32);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if clear(x, y) {
                continue;
            }
            // The pixel's middle, shrunk by a tenth of a pixel (`UV_SHRINK`).
            let pu = |f: f32| u0 + (u1 - u0) * (x as f32 + f) * sx;
            let pv = |f: f32| v0 + (v1 - v0) * (y as f32 + f) * sy;
            let uv = [
                [pu(0.1), pv(0.1)],
                [pu(0.9), pv(0.1)],
                [pu(0.9), pv(0.9)],
                [pu(0.1), pv(0.9)],
            ];
            // Left, right, top and bottom of the pixel in item space, y up.
            let (l, r) = (x as f32 * sx, (x + 1) as f32 * sx);
            let (t, b) = (1.0 - y as f32 * sy, 1.0 - (y + 1) as f32 * sy);
            // (neighbour that must be clear, the edge's two ends, the face's normal)
            let sides = [
                ((x, y - 1), [v(l, t, 0.0), v(r, t, 0.0)], Vec3::Y),
                ((x, y + 1), [v(l, b, 0.0), v(r, b, 0.0)], Vec3::NEG_Y),
                ((x - 1, y), [v(l, t, 0.0), v(l, b, 0.0)], Vec3::X),
                ((x + 1, y), [v(r, t, 0.0), v(r, b, 0.0)], Vec3::NEG_X),
            ];
            for ((nx, ny), [a, c], normal) in sides {
                if !clear(nx, ny) {
                    continue;
                }
                let (z0, z1) = (Vec3::Z * (7.5 / 16.0), Vec3::Z * (8.5 / 16.0));
                let mut pos = [a + z0, a + z1, c + z1, c + z0];
                if (pos[1] - pos[0]).cross(pos[2] - pos[0]).dot(normal) < 0.0 {
                    pos.swap(1, 3);
                }
                out.push((pos, uv, tint));
            }
        }
    }
}

/// An item's quads: a flat item's sprite layers extruded a pixel thick, as
/// `ItemModelGenerator.bakeExtrudedSprite` does, or its model's.
fn item_quads(atlas: &Atlas, icon: &ItemIcon) -> Vec<ItemQuad> {
    let mut quads = Vec::new();
    match icon {
        ItemIcon::Flat { layers, .. } => {
            for (i, &(rect, tint)) in layers.iter().enumerate() {
                // Later layers sit a hair out, as the GUI stacks them, so they don't fight.
                let lift = i as f32 * 0.001;
                extrude(atlas, rect, tint, lift, &mut quads);
            }
        }
        ItemIcon::Model { model, tints, .. } => {
            for q in &model.quads {
                let tint = usize::try_from(q.tint)
                    .ok()
                    .and_then(|i| tints.get(i).copied())
                    .unwrap_or([1.0; 3]);
                quads.push((q.pos, q.uv, tint));
            }
        }
    }
    quads
}

fn dropped(atlas: &Atlas, icon: &ItemIcon, d: &Dropped, light: [u8; 2], out: &mut Vec<Vertex>) {
    let quads = item_quads(atlas, icon);
    let (translation, rotation, scale) = match icon {
        ItemIcon::Flat { .. } => (Vec3::new(0.0, 2.0, 0.0), Quat::IDENTITY, Vec3::splat(0.5)),
        ItemIcon::Model { ground, .. } => {
            let [rx, ry, rz] = ground.rotation.map(f32::to_radians);
            (
                Vec3::from(ground.translation),
                Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz),
                Vec3::from(ground.scale),
            )
        }
    };
    // `ItemTransform.apply`, then the model centred.
    let transform = Mat4::from_translation(translation / 16.0)
        * Mat4::from_quat(rotation)
        * Mat4::from_scale(scale)
        * Mat4::from_translation(Vec3::splat(-0.5));
    let min_y = quads
        .iter()
        .flat_map(|(corners, _, _)| corners.iter().map(|&c| transform.transform_point3(c).y))
        .fold(f32::MAX, f32::min);
    if min_y == f32::MAX {
        return;
    }
    let bob = (d.age / 10.0 + d.bob_offset).sin() * 0.1 + 0.1;
    let spin = d.age / 20.0 + d.bob_offset;
    let root = Mat4::from_translation(Vec3::new(0.0, bob - min_y + 1.0 / 16.0, 0.0))
        * Mat4::from_rotation_y(spin)
        * transform;
    for (corners, uv, tint) in quads {
        let pos = corners.map(|c| root.transform_point3(c));
        emit_quad(pos, uv, d.pos, tint, light, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing(kind: Kind) -> Living {
        Living {
            kind,
            texture: String::new(),
            pos: Vec3::ZERO,
            body_yaw: 0.0,
            head_yaw: 0.0,
            pitch: 0.0,
            walk_pos: 0.0,
            walk_speed: 0.0,
            age: 0.0,
            flap: 0.0,
            flap_speed: 0.0,
            baby: false,
            name_tag: None,
            equipment: Equipment::default(),
            swing: None,
            burning: None,
            hurt: false,
            death_time: 0.0,
        }
    }

    /// The corners of a model's quads, placed as the renderer places them.
    fn corners(l: &Living, nodes: &[Node], tex_size: [f32; 2], model_scale: f32) -> Vec<Vec3> {
        let atlas_sprite = crate::atlas::Sprite {
            rect: [0.0, 0.0, 1.0, 1.0],
            layer: crate::atlas::Layer::Cutout,
        };
        let mut b = Builder {
            quads: Vec::new(),
            sprite: atlas_sprite,
            tex_size,
            tint: -1,
        };
        b.nodes(living_root(l, model_scale), nodes);
        b.quads.iter().flat_map(|q| q.pos).collect()
    }

    /// Standing models stand where vanilla's do: `LivingEntityRenderer` lowers the model
    /// 1.501 blocks after flipping it, so the 24-pixel-high feet sit 0.001 above the ground
    /// and a part 8.5 pixels above the origin (a hat's top) is at 1.501 + 8.5/16, scaled by
    /// 15/16 for players. A mob turned to yaw 0 faces south, +z.
    #[test]
    fn models_stand_where_vanillas_do() {
        let extent = |points: &[Vec3]| {
            let min = points.iter().fold(Vec3::MAX, |a, &b| a.min(b));
            let max = points.iter().fold(Vec3::MIN, |a, &b| a.max(b));
            (min, max)
        };
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        // Humanoid legs splay by 0.005 radians, which lowers their corners a little.
        let feet = |a: f32| (a - 0.001).abs() < 1e-3;
        let zombie = standing(Kind::Zombie { aggressive: false });
        let nodes = humanoid(&zombie, HUMANOID, Some(false));
        let (min, max) = extent(&corners(&zombie, &nodes, [64.0; 2], 1.0));
        assert!(
            feet(min.y) && close(max.y, 1.501 + 8.5 / 16.0),
            "{min} {max}"
        );

        let steve = standing(Kind::Player {
            slim: false,
            crouching: false,
        });
        let (min, max) = extent(&corners(&steve, &player(&steve, false), [64.0; 2], 0.9375));
        // The trousers layer reaches a quarter pixel below the feet.
        assert!(
            (min.y - (1.501 - 24.25 / 16.0) * 0.9375).abs() < 1e-3,
            "{min}"
        );
        assert!(close(max.y, (1.501 + 8.5 / 16.0) * 0.9375), "{max}");
        // Crouching drops the head 4.2 pixels (`HumanoidModel.setupAnim`).
        let crouching = standing(Kind::Player {
            slim: false,
            crouching: true,
        });
        let (_, max) = extent(&corners(
            &crouching,
            &player(&crouching, false),
            [64.0; 2],
            0.9375,
        ));
        assert!(close(max.y, (1.501 + 4.3 / 16.0) * 0.9375), "{max}");

        // A cow's horns top it, a pixel above its head at y 4 - 5 pixels.
        let cow = standing(Kind::Cow);
        let nodes = quadruped(&cow, &COW);
        let (min, max) = extent(&corners(&cow, &nodes, [64.0; 2], 1.0));
        assert!(
            close(min.y, 0.001) && close(max.y, 1.501 + 1.0 / 16.0),
            "{min} {max}"
        );
        // Its head, 8 to 15 pixels in front of the origin, is towards +z.
        let (head_min, head_max) = extent(&corners(&cow, &nodes[..1], [64.0; 2], 1.0));
        assert!(
            close(head_min.z, 0.5) && close(head_max.z, 15.0 / 16.0),
            "{head_min} {head_max}"
        );
    }

    /// A zombie's flames (0.6 by 1.95) are six cards, 0.84 blocks to a side, a 0.45-block
    /// step (scaled) apart, each 0.9 the width of the one below; seen from a camera looking
    /// south, they face it, front out, and the first stands 0.3 (less 0.02 per whole card
    /// height) towards it.
    #[test]
    fn flames_stack_up_facing_the_camera() {
        let fire = [[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 1.0, 1.0]];
        let mut out = Vec::new();
        flames(fire, Vec3::ZERO, [0.6, 1.95], 0.0, [15, 15], &mut out);
        let quads: Vec<&[Vertex]> = out.chunks(4).collect();
        assert_eq!(quads.len(), 6);
        let s = 0.6 * 1.4;
        for (i, q) in quads.iter().enumerate() {
            let p = q.iter().map(|v| Vec3::from(v.pos)).collect::<Vec<_>>();
            // The camera looks along +z, so a card facing it faces -z.
            let normal = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
            assert!((normal - Vec3::NEG_Z).length() < 1e-5, "{i}: {normal}");
            let width = (p[1] - p[0]).length();
            assert!(
                (width - s * 0.9f32.powi(i as i32)).abs() < 1e-5,
                "{i}: {width}"
            );
            assert!(
                (p[0].y - s * 0.45 * i as f32).abs() < 1e-5,
                "{i}: {}",
                p[0].y
            );
            assert!((p[2].y - p[0].y - s * 1.4).abs() < 1e-5);
            let z = -(0.3 - 2.0 * 0.02 - 0.03 * i as f32) * s;
            assert!((p[0].z - z).abs() < 1e-5, "{i}: {} vs {z}", p[0].z);
            // `fire_0` then `fire_1` mirrored, then the pair unmirrored, and so on.
            let u = [q[0].uv[0], q[1].uv[0]];
            let expected = [[0.0, 0.5], [0.5, 1.0], [0.5, 0.0], [1.0, 0.5]][i % 4];
            assert_eq!(u, expected, "{i}");
        }
        assert!(quads.iter().all(|q| q.iter().all(|v| v.color == [255; 4])));
    }

    /// A dying mob tips over sideways, √((t − 1) / 20 × 1.6) of the way at t ticks, so it's
    /// 56.9° over at 6 and flat from 13.5; a spider rolls on to its back instead.
    #[test]
    fn dying_mobs_fall_over() {
        let up = |kind, death_time| {
            let l = Living {
                death_time,
                ..standing(kind)
            };
            // Model space is y down: its up is -y.
            let up = living_root(&l, 1.0)
                .transform_vector3(Vec3::NEG_Y)
                .normalize();
            (up.angle_between(Vec3::Y).to_degrees(), up)
        };
        let pig = Kind::Pig;
        assert!(up(pig, 0.0).0 < 1e-3);
        let expected = (5.0f32 / 20.0 * 1.6).sqrt() * 90.0;
        assert!(
            (up(pig, 6.0).0 - expected).abs() < 1e-3,
            "{:?}",
            up(pig, 6.0)
        );
        let (angle, flat) = up(pig, 13.5);
        assert!((angle - 90.0).abs() < 1e-3 && flat.x > 0.99, "{flat}");
        assert!((up(pig, 20.0).0 - 90.0).abs() < 1e-3);
        assert!((up(Kind::Spider, 20.0).0 - 180.0).abs() < 1e-3);
    }

    /// Wool colours as `ColorLerper` makes them: white is E6E6E6, others are their dye's
    /// colour at three quarters.
    #[test]
    fn wool_colours_match_vanilla() {
        assert_eq!(wool_color(0), [230.0 / 255.0; 3]);
        // Red, B02E26: 0xB0 * 0.75 = 132, 0x2E * 0.75 = 34.5 floored, 0x26 * 0.75 = 28.5.
        assert_eq!(wool_color(14), [132.0 / 255.0, 34.0 / 255.0, 28.0 / 255.0]);
    }
}
