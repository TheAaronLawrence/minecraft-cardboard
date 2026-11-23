//! How the player moves, as vanilla 26.3's `LocalPlayer`, `LivingEntity` and `Entity` do it:
//! walking, sprinting and sneaking input, jumping, gravity and drag, block friction and
//! speed factors, collision with block shapes and stepping up, backing off edges while
//! sneaking, ladders, creative flight, and wading, swimming and sinking in water and lava,
//! pushed by their currents (`fluid`).
//!
//! `pick` finds the block the player looks at, against block outline shapes, as
//! `Entity.pick` does; `BlockPhysics::destroy_progress` is how far a tick of mining gets.
//!
//! Collision and outline shapes, block friction and destroy times come from `resources/reports/physics.json`, which
//! `cargo xtask gen-reports` dumps from vanilla, so they match it exactly. Not yet covered:
//! bubble columns, depth strider and dolphin's grace, bouncing on slime and beds, cobwebs
//! and powder snow, and entities as obstacles.

mod collide;
pub mod fluid;
mod pick;

use anyhow::{Context, Result};
pub use collide::Aabb;

/// `Entity.move`'s collision for an entity that doesn't step up blocks, such as a dropped
/// item: how much of `motion` a `width` × `height` box with its feet at `feet` can move
/// before blocks stop it.
pub fn collide_box(
    world: &impl World,
    blocks: &BlockPhysics,
    feet: DVec3,
    [width, height]: [f64; 2],
    motion: DVec3,
) -> DVec3 {
    collide::Collider { world, blocks }.collide(
        &Aabb::at_feet(feet, width, height),
        motion,
        0.0,
        false,
    )
}
use glam::DVec3;
use mc_registry::Registry;
pub use pick::{BlockHit, clip_collider, pick, view_vector};
use std::collections::HashSet;

/// What movement needs to know about the world.
pub trait World {
    /// The block state at a block.
    fn state(&self, x: i32, y: i32, z: i32) -> u32;
    /// Whether the chunk holding this block column has arrived.
    fn loaded(&self, x: i32, z: i32) -> bool;
}

/// Each block state's collision boxes, and each block's movement properties.
pub struct BlockPhysics {
    shapes: Vec<Vec<Aabb>>,
    /// Each shape's edges, from one corner to the other, as outlines draw them.
    edges: Vec<Vec<[f64; 6]>>,
    /// Collision and outline shape index per state id.
    state_shapes: Vec<u16>,
    outlines: Vec<u16>,
    /// Per state id: `Block.defaultDestroyTime` (-1 for unbreakable) and
    /// `requiresCorrectToolForDrops`.
    destroy_time: Vec<f32>,
    requires_tool: Vec<bool>,
    /// Per state id: `Block.getFriction`, `getSpeedFactor`, `getJumpFactor`.
    friction: Vec<f32>,
    speed_factor: Vec<f32>,
    jump_factor: Vec<f32>,
    /// States in the `#minecraft:climbable` tag.
    climbable: Vec<bool>,
    /// Per state id: its fluid, packed as `fluid::Fluid::unpack` reads it, and a bit per
    /// direction whose face is sturdy (`BlockState.isFaceSturdy`), plus 64 for ice and 128
    /// for the states the `MOTION_BLOCKING` heightmap counts.
    fluids: Vec<u8>,
    sturdy: Vec<u8>,
    /// States in `#minecraft:blocks_fluid_flow`.
    blocks_fluid_flow: Vec<bool>,
    /// Per state id: `getLightDampening`, `getLightEmission` times 16, and 256 for
    /// `useShapeForLightOcclusion`.
    light: Vec<u16>,
    /// Per state id: 0 for `LightEngine.isEmptyShape`, else 1 + an index into
    /// `occlusion_faces`.
    occlusion: Vec<u16>,
    /// Per state id: 0 unless it `canOcclude`, else 1 + an index into `occlusion_faces` for
    /// its `getFaceOcclusionShape`s, which hide its neighbours' faces.
    render_occlusion: Vec<u16>,
    /// Each occlusion shape's faces, by `Direction` ordinal: a bit per sixteenth of the face
    /// it covers, as `DumpPhysics.faceMask` lays them out.
    occlusion_faces: Vec<[[u64; 4]; 6]>,
    /// Per state id: an index into `sound_types` (`BlockState.getSoundType`).
    sounds: Vec<u16>,
    sound_types: Vec<SoundType>,
    /// States in `#minecraft:inside_step_sound_blocks` and `#combination_step_sound_blocks`,
    /// which change the step sound.
    inside_step: Vec<bool>,
    combination_step: Vec<bool>,
    /// Air, cave air and void air (`BlockState.isAir`).
    air: Vec<bool>,
    /// `#minecraft:fences`, and `#walls` or fence gates, which `Entity.getOnPos` stands on
    /// at their tops.
    fences: Vec<bool>,
    walls_and_gates: Vec<bool>,
    /// States whose block does something used with an empty hand (`useWithoutItem`).
    interactive: Vec<bool>,
}

/// `SoundType`: the sound events a block plays when broken, walked on, placed, hit while
/// being mined, and landed on, with the volume and pitch they're played at.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundType {
    pub break_sound: String,
    pub step: String,
    pub place: String,
    pub hit: String,
    pub fall: String,
    pub volume: f32,
    pub pitch: f32,
}

impl BlockPhysics {
    pub fn vanilla(registry: &Registry) -> Result<Self> {
        let path = mc_assets::locate("resources/reports/physics.json");
        let report: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).with_context(|| {
                format!("reading {}; run `cargo xtask gen-reports`", path.display())
            })?)?;
        let shapes = report["shapes"]
            .as_array()
            .context("physics.json has no shapes")?
            .iter()
            .map(|shape| {
                shape
                    .as_array()
                    .context("shape is not a list")?
                    .iter()
                    .map(|b| {
                        let c: Vec<f64> = b
                            .as_array()
                            .context("box is not a list")?
                            .iter()
                            .map(|v| v.as_f64().context("box coordinate"))
                            .collect::<Result<_>>()?;
                        anyhow::ensure!(c.len() == 6, "box needs 6 coordinates");
                        Ok(Aabb::new(
                            DVec3::new(c[0], c[1], c[2]),
                            DVec3::new(c[3], c[4], c[5]),
                        ))
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;
        let edges = report["edges"]
            .as_array()
            .context("physics.json has no edges")?
            .iter()
            .map(|shape| {
                shape
                    .as_array()
                    .context("edges are not a list")?
                    .iter()
                    .map(|e| {
                        let c: Vec<f64> = e
                            .as_array()
                            .context("edge is not a list")?
                            .iter()
                            .map(|v| v.as_f64().context("edge coordinate"))
                            .collect::<Result<_>>()?;
                        c.try_into().ok().context("edge needs 6 coordinates")
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;
        anyhow::ensure!(
            edges.len() == shapes.len(),
            "physics.json: one edge list per shape"
        );
        let shape_ids = |key: &str| -> Result<Vec<u16>> {
            let ids: Vec<u16> = report[key]
                .as_array()
                .with_context(|| format!("physics.json has no {key}"))?
                .iter()
                .map(|v| v.as_u64().map(|v| v as u16).context("shape index"))
                .collect::<Result<_>>()?;
            anyhow::ensure!(
                ids.len() == registry.state_count() as usize
                    && ids.iter().all(|&i| (i as usize) < shapes.len()),
                "physics.json's {key} don't fit the registry's {} states",
                registry.state_count()
            );
            Ok(ids)
        };
        let state_shapes = shape_ids("states")?;
        let outlines = shape_ids("outlines")?;

        let n = state_shapes.len();
        let (mut friction, mut speed_factor, mut jump_factor) =
            (vec![0.6f32; n], vec![1.0f32; n], vec![1.0f32; n]);
        for (name, props) in report["blocks"].as_object().into_iter().flatten() {
            let block = registry
                .block(name)
                .with_context(|| format!("physics.json names unknown block {name}"))?;
            let states =
                block.first_state as usize..(block.first_state + block.state_count()) as usize;
            for (list, key) in [
                (&mut friction, "friction"),
                (&mut speed_factor, "speed_factor"),
                (&mut jump_factor, "jump_factor"),
            ] {
                if let Some(v) = props[key].as_f64() {
                    list[states.clone()].fill(v as f32);
                }
            }
        }

        let (mut destroy_time, mut requires_tool) = (vec![0.0f32; n], vec![false; n]);
        for (name, mining) in report["mining"]
            .as_object()
            .context("physics.json has no mining")?
        {
            let block = registry
                .block(name)
                .with_context(|| format!("physics.json names unknown block {name}"))?;
            let states =
                block.first_state as usize..(block.first_state + block.state_count()) as usize;
            let time = mining[0].as_f64().context("destroy time")?;
            destroy_time[states.clone()].fill(time as f32);
            requires_tool[states].fill(mining[1].as_bool().context("requires tool")?);
        }

        let tagged = |tag: &str| -> Result<Vec<bool>> {
            let mut states = vec![false; n];
            for name in block_tag(tag)? {
                if let Some(block) = registry.block(&name) {
                    let first = block.first_state as usize;
                    states[first..first + block.state_count() as usize].fill(true);
                }
            }
            Ok(states)
        };
        let per_state = |key: &str| -> Result<Vec<u16>> {
            let values: Vec<u16> = report[key]
                .as_array()
                .with_context(|| {
                    format!("physics.json has no {key}; run `cargo xtask gen-reports`")
                })?
                .iter()
                .map(|v| v.as_u64().map(|v| v as u16).context(key.to_owned()))
                .collect::<Result<_>>()?;
            anyhow::ensure!(
                values.len() == n,
                "physics.json's {key} don't fit the registry"
            );
            Ok(values)
        };
        let bytes = |values: Vec<u16>| values.into_iter().map(|v| v as u8).collect();
        let occlusion_faces = report["occlusion_faces"]
            .as_array()
            .context("physics.json has no occlusion_faces; run `cargo xtask gen-reports`")?
            .iter()
            .map(|faces| {
                let faces: Vec<[u64; 4]> = faces
                    .as_array()
                    .context("occlusion faces are not a list")?
                    .iter()
                    .map(|face| {
                        let hex = face.as_str().filter(|h| h.len() == 64);
                        let hex = hex.context("a face mask needs 64 hex digits")?;
                        let mut words = [0; 4];
                        for (i, word) in words.iter_mut().enumerate() {
                            *word = u64::from_str_radix(&hex[i * 16..i * 16 + 16], 16)?;
                        }
                        Ok(words)
                    })
                    .collect::<Result<_>>()?;
                faces
                    .try_into()
                    .ok()
                    .context("an occlusion shape needs 6 faces")
            })
            .collect::<Result<Vec<_>>>()?;
        let occlusion = per_state("occlusion")?;
        let render_occlusion = per_state("render_occlusion")?;
        anyhow::ensure!(
            occlusion
                .iter()
                .chain(&render_occlusion)
                .all(|&i| i as usize <= occlusion_faces.len()),
            "physics.json's occlusion indices don't fit its occlusion_faces"
        );
        let sound_types = report["sound_types"]
            .as_array()
            .context("physics.json has no sound_types; run `cargo xtask gen-reports`")?
            .iter()
            .map(|t| {
                let name = |i: usize| t[i].as_str().map(str::to_owned).context("sound type");
                Ok(SoundType {
                    break_sound: name(0)?,
                    step: name(1)?,
                    place: name(2)?,
                    hit: name(3)?,
                    fall: name(4)?,
                    volume: t[5].as_f64().context("sound type volume")? as f32,
                    pitch: t[6].as_f64().context("sound type pitch")? as f32,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let sounds = per_state("sounds")?;
        anyhow::ensure!(
            sounds.iter().all(|&i| (i as usize) < sound_types.len()),
            "physics.json's sounds don't fit its sound_types"
        );
        Ok(Self {
            sounds,
            sound_types,
            inside_step: tagged("minecraft:inside_step_sound_blocks")?,
            combination_step: tagged("minecraft:combination_step_sound_blocks")?,
            air: {
                let mut air = vec![false; n];
                for name in ["minecraft:air", "minecraft:cave_air", "minecraft:void_air"] {
                    if let Some(b) = registry.block(name) {
                        let first = b.first_state as usize;
                        air[first..first + b.state_count() as usize].fill(true);
                    }
                }
                air
            },
            interactive: {
                let mut states = vec![false; n];
                for name in report["interactive"]
                    .as_array()
                    .context("physics.json has no interactive; run `cargo xtask gen-reports`")?
                {
                    let name = name.as_str().context("block name")?;
                    if let Some(b) = registry.block(name) {
                        let first = b.first_state as usize;
                        states[first..first + b.state_count() as usize].fill(true);
                    }
                }
                states
            },
            fences: tagged("minecraft:fences")?,
            walls_and_gates: {
                let walls = tagged("minecraft:walls")?;
                let gates = tagged("minecraft:fence_gates")?;
                walls.iter().zip(gates).map(|(w, g)| *w || g).collect()
            },
            shapes,
            edges,
            state_shapes,
            outlines,
            destroy_time,
            requires_tool,
            friction,
            speed_factor,
            jump_factor,
            climbable: tagged("minecraft:climbable")?,
            fluids: bytes(per_state("fluids")?),
            sturdy: bytes(per_state("sturdy")?),
            blocks_fluid_flow: tagged("minecraft:blocks_fluid_flow")?,
            light: per_state("light")?,
            occlusion,
            render_occlusion,
            occlusion_faces,
        })
    }

    /// `BlockState.getSoundType`.
    pub fn sound_type(&self, state: u32) -> &SoundType {
        let i = self.sounds.get(state as usize).copied().unwrap_or(0);
        &self.sound_types[i as usize]
    }

    /// In `#minecraft:inside_step_sound_blocks`: walked through, it's heard instead of the
    /// block below.
    pub fn inside_step_sound(&self, state: u32) -> bool {
        self.inside_step
            .get(state as usize)
            .copied()
            .unwrap_or(false)
    }

    /// In `#minecraft:combination_step_sound_blocks`: heard with the block below, muffled.
    pub fn combination_step_sound(&self, state: u32) -> bool {
        self.combination_step
            .get(state as usize)
            .copied()
            .unwrap_or(false)
    }

    /// `BlockState.isAir`.
    pub fn is_air(&self, state: u32) -> bool {
        self.air.get(state as usize).copied().unwrap_or(true)
    }

    /// Whether `Entity.getOnPos` keeps a supporting block of this state rather than the
    /// block at the offset: walls and gates always, fences when `low` (within half a block).
    fn fence_like(&self, state: u32, low: bool) -> bool {
        let at = |v: &Vec<bool>| v.get(state as usize).copied().unwrap_or(false);
        at(&self.walls_and_gates) || low && at(&self.fences)
    }

    /// The block's collision boxes (`BlockState.getCollisionShape`), in block units.
    pub fn shape(&self, state: u32) -> &[Aabb] {
        self.state_shapes
            .get(state as usize)
            .map_or(&[], |&i| &self.shapes[i as usize])
    }

    /// `BlockState.getLightDampening`: how much light passing into the block loses, 0 to 15.
    pub fn light_dampening(&self, state: u32) -> u8 {
        self.light
            .get(state as usize)
            .map_or(0, |&l| (l & 15) as u8)
    }

    /// `BlockState.getLightEmission`: the block light it gives off, 0 to 15.
    pub fn light_emission(&self, state: u32) -> u8 {
        self.light
            .get(state as usize)
            .map_or(0, |&l| (l >> 4 & 15) as u8)
    }

    /// `LightEngine.hasDifferentLightProperties`: whether replacing `old` with `new` can
    /// change the light around it.
    pub fn light_differs(&self, old: u32, new: u32) -> bool {
        let shaped = |state: u32| {
            self.light
                .get(state as usize)
                .is_some_and(|&l| l & 256 != 0)
        };
        old != new
            && (self.light_dampening(old) != self.light_dampening(new)
                || self.light_emission(old) != self.light_emission(new)
                || shaped(old)
                || shaped(new))
    }

    /// `LightEngine.isEmptyShape`: light ignores the block's shape.
    pub fn empty_light_shape(&self, state: u32) -> bool {
        self.occlusion.get(state as usize).is_none_or(|&i| i == 0)
    }

    /// `LightEngine.shapeOccludes`: whether light going from `from` to `to` in direction
    /// `direction` (a `Direction` ordinal) is blocked by the two faces between them, which
    /// together cover the whole face.
    pub fn light_occludes(&self, from: u32, to: u32, direction: usize) -> bool {
        let face = |state: u32, direction: usize| match self.occlusion.get(state as usize) {
            Some(&i) if i > 0 => self.occlusion_faces[i as usize - 1][direction],
            _ => [0; 4],
        };
        let (a, b) = (face(from, direction), face(to, direction ^ 1));
        (0..4).all(|i| a[i] | b[i] == u64::MAX)
    }

    /// `BlockState.getFaceOcclusionShape` for each `Direction` ordinal, laid out as
    /// `light_occludes` reads them; none when the state can't occlude.
    pub fn face_occlusion(&self, state: u32) -> Option<&[[u64; 4]; 6]> {
        match self.render_occlusion.get(state as usize) {
            Some(&i) if i > 0 => Some(&self.occlusion_faces[i as usize - 1]),
            _ => None,
        }
    }

    /// `BlockState.isSolidRender`: its occlusion shape fills the whole block.
    pub fn solid_render(&self, state: u32) -> bool {
        self.face_occlusion(state)
            .is_some_and(|faces| faces.iter().all(|f| *f == [u64::MAX; 4]))
    }

    /// `Block.getFriction`: how slippery the block is, 0.6 for most.
    /// `Heightmap.Types.MOTION_BLOCKING`'s test: the state blocks motion or holds a fluid,
    /// so rain and snow stop on it.
    pub fn motion_blocking(&self, state: u32) -> bool {
        self.sturdy
            .get(state as usize)
            .is_some_and(|s| s & 128 != 0)
    }

    /// Whether the block does something when used with an empty hand (its class overrides
    /// `useWithoutItem`), so using an item on it may not use the item.
    pub fn interactive(&self, state: u32) -> bool {
        self.interactive
            .get(state as usize)
            .copied()
            .unwrap_or(false)
    }

    pub fn friction(&self, state: u32) -> f32 {
        self.friction.get(state as usize).copied().unwrap_or(0.6)
    }

    /// The boxes the game outlines and aims at (`BlockState.getShape`), in block units.
    pub fn outline(&self, state: u32) -> &[Aabb] {
        self.outlines
            .get(state as usize)
            .map_or(&[], |&i| &self.shapes[i as usize])
    }

    /// The outline's edges, as drawn around the block the player looks at.
    pub fn outline_edges(&self, state: u32) -> &[[f64; 6]] {
        self.outlines
            .get(state as usize)
            .map_or(&[], |&i| &self.edges[i as usize])
    }

    /// How much of the block a tick of mining it with an empty hand breaks, where 1 is all
    /// of it (`BlockBehaviour.getDestroyProgress` with `Player.getDestroySpeed`): a fifth as
    /// much off the ground, and nothing for unbreakable blocks. Not yet counted: the held
    /// tool, effects, and mining underwater.
    pub fn destroy_progress(&self, state: u32, on_ground: bool) -> f32 {
        let time = self.destroy_time[state as usize];
        if time == -1.0 {
            return 0.0;
        }
        let speed = if on_ground { 1.0 } else { 1.0 / 5.0 };
        let tool = if self.requires_tool[state as usize] {
            100.0
        } else {
            30.0
        };
        speed / time / tool
    }
}

/// The blocks in a block tag, following tags it includes.
fn block_tag(tag: &str) -> Result<HashSet<String>> {
    let res = mc_assets::Resources::new(mc_assets::default_root());
    let mut names = HashSet::new();
    let mut pending = vec![tag.to_owned()];
    let mut seen = HashSet::new();
    while let Some(tag) = pending.pop() {
        if !seen.insert(tag.clone()) {
            continue;
        }
        let path = res.data_path("tags/block", &tag, "json");
        let json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&path).with_context(|| format!("block tag {tag}"))?,
        )?;
        for value in json["values"].as_array().into_iter().flatten() {
            // An entry is a name, or `{"id": name, "required": false}`.
            let Some(id) = value.as_str().or(value["id"].as_str()) else {
                continue;
            };
            match id.strip_prefix('#') {
                Some(inner) => pending.push(inner.to_owned()),
                None => {
                    names.insert(id.to_owned());
                }
            }
        }
    }
    Ok(names)
}

/// The movement keys held this tick, as vanilla's `Input` record.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Input {
    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
}

impl Input {
    /// The `player_input` packet's flag byte.
    pub fn bits(self) -> u8 {
        [
            self.forward,
            self.backward,
            self.left,
            self.right,
            self.jump,
            self.sneak,
            self.sprint,
        ]
        .iter()
        .enumerate()
        .fold(0, |bits, (i, &on)| bits | (on as u8) << i)
    }
}

const WIDTH: f64 = 0.6;
const STEP: f32 = 0.6;
/// `Entity.getFluidJumpThreshold` for an eye at least 0.4 high, as all the player's are.
const FLUID_JUMP: f64 = 0.4;
/// `EntityFluidInteraction.applyCurrentTo`'s scales: water's, and lava's outside the
/// nether (`EnvironmentAttributes.FAST_LAVA` there makes it 0.007).
const WATER_CURRENT: f64 = 0.014;
const LAVA_CURRENT: f64 = 0.0023333333333333335;

/// The player's `Pose`, which sets its box and eye height.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pose {
    Standing,
    Crouching,
    /// Swimming, or crawling where nothing taller fits.
    Swimming,
}

impl Pose {
    /// `Avatar.POSES`: the box's height and the eye height.
    fn size(self) -> (f64, f64) {
        match self {
            Pose::Standing => (1.8, 1.62),
            Pose::Crouching => (1.5, 1.27),
            Pose::Swimming => (0.6, 0.4),
        }
    }
}
/// `Attributes.GRAVITY`'s default.
const GRAVITY: f64 = 0.08;
/// `Attributes.JUMP_STRENGTH`'s default, a float widened to double.
const JUMP: f32 = 0.42;
/// The player's `movement_speed` attribute.
const SPEED: f64 = 0.10000000149011612;

/// The local player's movement state.
#[derive(Clone, Debug)]
pub struct Player {
    /// Feet: the bottom centre of the bounding box.
    pub pos: DVec3,
    pub velocity: DVec3,
    /// Degrees: yaw 0 faces south (+z), pitch positive looks down.
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub sprinting: bool,
    /// `LocalPlayer.crouching`: moving slowly and backing off edges, set as each tick starts.
    pub crouching: bool,
    /// The box and eye height, set as each tick ends (`Player.updatePlayerPose`).
    pub pose: Pose,
    /// `Entity.isSwimming`: sprinting under water.
    pub swimming: bool,
    /// `Entity.isInWater`: the box touches water.
    pub in_water: bool,
    /// `LocalPlayer.isUnderWater`: the eye was in water as the tick started.
    pub under_water: bool,
    /// `LocalPlayer.waterVisionTime`: ticks the eye has spent in water, up to 600, which
    /// clears the water's fog; it falls 10 a tick out of it.
    pub water_vision_time: i32,
    touching: fluid::Touching,
    pub flying: bool,
    /// May start flying by double-tapping jump (creative and spectator).
    pub may_fly: bool,
    /// `Abilities.flyingSpeed`, 0.05 by default.
    pub flying_speed: f32,
    /// While an item is in use: its `use_effects`' speed multiplier, and whether it lets
    /// the player sprint (`LocalPlayer.itemUseSpeedMultiplier` and `isSlowDueToUsingItem`).
    pub using_item: Option<(f32, bool)>,
    no_jump_delay: u32,
    jump_trigger_time: u32,
    was_jumping: bool,
    fall_distance: f64,
    /// `Entity.isShiftKeyDown`: sneak was held this tick.
    shift_key_down: bool,
    /// `Entity.mainSupportingBlockPos` and `onGroundNoBlocks`.
    supporting: Option<[i32; 3]>,
    on_ground_no_blocks: bool,
    /// `Entity.moveDist` and `nextStep`: how far it has walked, and where the next step
    /// sounds.
    move_dist: f32,
    next_step: f32,
    /// `Entity.wasTouchingWater` and `firstTick`, for the splash going in.
    was_touching_water: bool,
    first_tick: bool,
    /// The sounds the player made since they were last taken, as `LocalPlayer.playSound`
    /// plays them where the player is.
    pub sounds: Vec<MoveSound>,
}

/// A sound the player's movement makes.
#[derive(Clone, Debug, PartialEq)]
pub enum MoveSound {
    /// A sound event at this volume and pitch, such as a block's step sound.
    Play {
        event: String,
        volume: f32,
        pitch: f32,
    },
    /// `Entity.playSwimSound` at this volume, at a random pitch from 0.6 to 1.4.
    Swim(f32),
    /// `Entity.doWaterSplashEffect`: going into water, at this volume, fast or not.
    Splash { volume: f32, fast: bool },
}

impl Player {
    pub fn new(pos: DVec3, yaw: f32, pitch: f32) -> Self {
        Self {
            pos,
            velocity: DVec3::ZERO,
            yaw,
            pitch,
            on_ground: false,
            horizontal_collision: false,
            sprinting: false,
            crouching: false,
            pose: Pose::Standing,
            swimming: false,
            in_water: false,
            under_water: false,
            water_vision_time: 0,
            touching: fluid::Touching::default(),
            flying: false,
            may_fly: false,
            flying_speed: 0.05,
            using_item: None,
            no_jump_delay: 0,
            jump_trigger_time: 0,
            was_jumping: false,
            fall_distance: 0.0,
            shift_key_down: false,
            supporting: None,
            on_ground_no_blocks: false,
            move_dist: 0.0,
            next_step: 1.0,
            was_touching_water: false,
            first_tick: true,
            sounds: Vec::new(),
        }
    }

    /// `Entity.isShiftKeyDown`, which `isSecondaryUseActive` is.
    pub fn shift_key_down(&self) -> bool {
        self.shift_key_down
    }

    pub fn eye_height(&self) -> f64 {
        self.pose.size().1
    }

    /// Its box's height in its pose.
    pub fn height(&self) -> f64 {
        self.pose.size().0
    }

    /// Where the server put the player; any motion stops.
    pub fn teleport(&mut self, pos: DVec3, yaw: f32, pitch: f32) {
        (self.pos, self.yaw, self.pitch) = (pos, yaw, pitch);
        self.velocity = DVec3::ZERO;
        self.fall_distance = 0.0;
    }

    fn bounding_box(&self) -> Aabb {
        Aabb::at_feet(self.pos, WIDTH, self.pose.size().0)
    }

    /// `Player.canPlayerFitWithinBlocksAndEntitiesWhen`.
    fn fits(&self, pose: Pose, c: &collide::Collider<impl World>) -> bool {
        c.no_collision(&Aabb::at_feet(self.pos, WIDTH, pose.size().0).deflate(1e-7))
    }

    /// `Entity.isEyeInFluid(WATER)`, as the last fluid update left it.
    pub fn eye_in_water(&self) -> bool {
        self.touching.eye_in_water
    }

    /// `LocalPlayer.getWaterVision`: 0 out of water, rising to 0.6 over the first 5
    /// seconds in it and to 1 over the next 25.
    pub fn water_vision(&self) -> f32 {
        if !self.eye_in_water() {
            return 0.0;
        }
        let t = self.water_vision_time as f32;
        if t >= 600.0 {
            return 1.0;
        }
        let a = (t / 100.0).clamp(0.0, 1.0);
        let b = if t < 100.0 {
            0.0
        } else {
            ((t - 100.0) / 500.0).clamp(0.0, 1.0)
        };
        a * 0.6 + b * 0.39999998
    }

    fn in_lava(&self) -> bool {
        self.touching.lava_height > 0.0
    }

    /// `Entity.updateFluidInteraction`: what fluid the box touches, and its current's push.
    fn update_fluid_interaction(&mut self, c: &collide::Collider<impl World>) {
        let b = self.bounding_box();
        let eye = self.pos + DVec3::Y * self.eye_height();
        // `Player.isPushedByFluid`.
        let pushed = !self.flying;
        self.touching = fluid::touching(c.world, c.blocks, &b.deflate(0.001), b.min.y, eye, pushed);
        self.in_water = self.touching.water_height > 0.0;
        if self.in_water {
            self.fall_distance = 0.0;
            if !self.was_touching_water && !self.first_tick {
                // `doWaterSplashEffect`.
                let v = self.velocity;
                let speed =
                    ((v.x * v.x * 0.2 + v.y * v.y + v.z * v.z * 0.2).sqrt() as f32 * 0.2).min(1.0);
                self.sounds.push(MoveSound::Splash {
                    volume: speed,
                    fast: speed >= 0.25,
                });
            }
        }
        self.was_touching_water = self.in_water;
        if pushed {
            if self.in_water {
                self.velocity += self
                    .touching
                    .water_current
                    .push(self.velocity, WATER_CURRENT);
            }
            if self.in_lava() {
                self.velocity += self.touching.lava_current.push(self.velocity, LAVA_CURRENT);
            }
        }
    }

    /// `Player.updatePlayerPose`: swimming, crouching or standing, whichever is wanted and
    /// fits, else the lowest that fits.
    fn update_pose(&mut self, sneak: bool, c: &collide::Collider<impl World>) {
        if !self.fits(Pose::Swimming, c) {
            return;
        }
        let desired = if self.swimming {
            Pose::Swimming
        } else if sneak && !self.flying {
            Pose::Crouching
        } else {
            Pose::Standing
        };
        self.pose = if self.fits(desired, c) {
            desired
        } else if self.fits(Pose::Crouching, c) {
            Pose::Crouching
        } else {
            Pose::Swimming
        };
    }

    /// One game tick (1/20 s) with these keys held.
    pub fn tick(&mut self, input: Input, world: &impl World, blocks: &BlockPhysics) {
        // Vanilla holds the player still until the chunk under it has arrived.
        if !world.loaded(self.pos.x.floor() as i32, self.pos.z.floor() as i32) {
            return;
        }
        let collider = collide::Collider { world, blocks };
        self.shift_key_down = input.sneak;

        // `Player.updateIsUnderwater`, from where the eye was last tick, then
        // `Entity.baseTick`'s fluid update and `Player.updateSwimming`.
        self.under_water = self.touching.eye_in_water;
        self.update_fluid_interaction(&collider);
        if self.in_lava() {
            self.fall_distance *= 0.5;
        }
        // `LocalPlayer.aiStep`'s water vision.
        if self.touching.eye_in_water {
            self.water_vision_time = (self.water_vision_time + 1).min(600);
        } else if self.water_vision_time > 0 {
            self.water_vision_time = (self.water_vision_time - 10).max(0);
        }
        let water_here = fluid::fluid_at(world, blocks, self.block_pos())
            .is_some_and(|f| f.kind == fluid::FluidKind::Water);
        self.swimming = !self.flying
            && self.sprinting
            && if self.swimming {
                self.in_water
            } else {
                self.under_water && water_here
            };

        // `LocalPlayer.aiStep`: crouching while sneak is held, or where only crouching fits.
        self.crouching = !self.flying
            && !self.swimming
            && self.fits(Pose::Crouching, &collider)
            && (input.sneak || !self.fits(Pose::Standing, &collider));
        // `isMovingSlowly`: crouching or crawling.
        let slowly = self.crouching || (self.pose == Pose::Swimming && !self.in_water);
        let shallow = self.in_water && !self.under_water;

        self.jump_trigger_time = self.jump_trigger_time.saturating_sub(1);
        // `KeyboardInput.tick`: x is left minus right, y forward minus back, normalised.
        let impulse = |a: bool, b: bool| (a as i32 - b as i32) as f32;
        let mut mv = glam::Vec2::new(
            impulse(input.left, input.right),
            impulse(input.forward, input.backward),
        );
        mv = if mv.length() < 1.0e-4 {
            glam::Vec2::ZERO
        } else {
            mv.normalize()
        };
        // `LocalPlayer.modifyInput`.
        if mv.length_squared() != 0.0 {
            mv *= 0.98;
            if let Some((speed, _)) = self.using_item {
                mv *= speed;
            }
            if slowly {
                mv *= 0.3; // `sneaking_speed`
            }
            mv = square_movement(mv);
        }

        // Sprinting: started by the sprint key while moving forward, but not in shallow
        // water or while moving slowly unless under water. Running stops by letting go of
        // forward, sneaking, walking into a wall or wading; swimming stops by leaving the
        // water, or by letting go of forward while floating.
        let forward = mv.y > 1.0e-5;
        if !self.sprinting
            && input.sprint
            && forward
            && (self.flying || !shallow)
            && (!slowly || self.under_water)
            && !self.using_item.is_some_and(|(_, can_sprint)| !can_sprint)
        {
            self.sprinting = true;
        }
        if self.sprinting {
            let stop = if self.swimming {
                !self.in_water || (!forward && !self.on_ground && !input.sneak)
            } else {
                !forward || self.crouching || self.horizontal_collision || (!self.flying && shallow)
            };
            if stop {
                self.sprinting = false;
            }
        }

        // Double-tapping jump toggles flight, within 7 ticks, but not while swimming.
        if input.jump && !self.was_jumping && self.may_fly {
            if self.jump_trigger_time == 0 {
                self.jump_trigger_time = 7;
            } else if !self.swimming {
                self.flying = !self.flying;
                self.jump_trigger_time = 0;
            }
        }
        self.was_jumping = input.jump;
        // `goDownInWater`: sneaking sinks.
        if self.in_water && input.sneak && !self.flying {
            self.velocity.y -= 0.04f32 as f64;
        }
        if self.flying {
            let vertical = input.jump as i32 - input.sneak as i32;
            self.velocity.y += (vertical as f32 * self.flying_speed * 3.0) as f64;
        }

        // `LivingEntity.aiStep`: tiny motion stops, then jumping.
        self.no_jump_delay = self.no_jump_delay.saturating_sub(1);
        if self.velocity.x * self.velocity.x + self.velocity.z * self.velocity.z < 9.0e-6 {
            (self.velocity.x, self.velocity.z) = (0.0, 0.0);
        }
        if self.velocity.y.abs() < 0.003 {
            self.velocity.y = 0.0;
        }
        // Jumping: off the ground or out of shallow fluid, else rising through it
        // (`jumpInLiquid`).
        let jumping = input.jump && !self.flying;
        if jumping {
            let height = if self.in_lava() {
                self.touching.lava_height
            } else {
                self.touching.water_height
            };
            let in_water = self.in_water && height > 0.0;
            let rise = if !in_water || self.on_ground && height <= FLUID_JUMP {
                if !self.in_lava() || self.on_ground && self.touching.lava_height <= FLUID_JUMP {
                    if (self.on_ground || in_water && height <= FLUID_JUMP)
                        && self.no_jump_delay == 0
                    {
                        self.jump_from_ground(&collider);
                        self.no_jump_delay = 10;
                    }
                    false
                } else {
                    true
                }
            } else {
                true
            };
            if rise {
                self.velocity.y += 0.04f32 as f64;
            }
        } else {
            self.no_jump_delay = 0;
        }

        // `Player.travel`: a swimmer heads where it looks, and flight keeps 60% of the
        // vertical speed instead of falling.
        if self.swimming {
            let look = view_vector(self.yaw, self.pitch).y;
            let multiplier = if look < -0.2 { 0.085 } else { 0.06 };
            let head = DVec3::new(self.pos.x, self.pos.y + 1.0 - 0.1, self.pos.z).floor();
            let under = fluid::fluid_at(world, blocks, head.as_ivec3().to_array()).is_some();
            if look <= 0.0 || input.jump || under {
                self.velocity.y += (look - self.velocity.y) * multiplier;
            }
        }
        let y_before = self.velocity.y;
        // `LivingEntity.shouldTravelInFluid`.
        if (self.in_water || self.in_lava()) && !self.flying {
            self.travel_in_fluid(mv, &collider);
        } else {
            self.travel_in_air(mv, jumping, &collider);
        }
        if self.flying {
            self.velocity.y = y_before * 0.6;
            if self.on_ground {
                self.flying = false;
            }
        }
        self.update_pose(input.sneak, &collider);
        self.first_tick = false;
    }

    /// `LivingEntity.travelInFluid`: slow strokes, the fluid's drag, and a little gravity.
    fn travel_in_fluid(&mut self, mv: glam::Vec2, c: &collide::Collider<impl World>) {
        let falling = self.velocity.y <= 0.0;
        let old_y = self.pos.y;
        let water = self.in_water;
        let input = DVec3::new(mv.x as f64, 0.0, mv.y as f64);
        self.velocity += input_vector(input, 0.02, self.yaw);
        self.move_by(self.velocity, c);
        if water {
            // `travelInWater`.
            let slow_down = if self.sprinting { 0.9f32 } else { 0.8f32 } as f64;
            if self.horizontal_collision && self.on_climbable(c) {
                self.velocity.y = 0.2;
            }
            self.velocity *= DVec3::new(slow_down, 0.8f32 as f64, slow_down);
            self.velocity.y = self.fluid_falling_adjusted(falling);
        } else {
            // `travelInLava`.
            if self.touching.lava_height <= FLUID_JUMP {
                self.velocity *= DVec3::new(0.5, 0.8f32 as f64, 0.5);
                self.velocity.y = self.fluid_falling_adjusted(falling);
            } else {
                self.velocity *= 0.5;
            }
            self.velocity.y -= GRAVITY / 4.0;
        }
        // `jumpOutOfFluid`: against a wall with room above it, climb out.
        if self.horizontal_collision {
            let v = self.velocity;
            let lifted = self.bounding_box().offset(DVec3::new(
                v.x,
                v.y + 0.6f32 as f64 - self.pos.y + old_y,
                v.z,
            ));
            if c.no_collision(&lifted) && !fluid::contains_any_liquid(c.world, c.blocks, &lifted) {
                self.velocity.y = 0.3f32 as f64;
            }
        }
    }

    /// `LivingEntity.getFluidFallingAdjustedMovement`: sinking slowly unless sprinting.
    fn fluid_falling_adjusted(&self, falling: bool) -> f64 {
        let y = self.velocity.y;
        if self.sprinting {
            y
        } else if falling && (y - 0.005).abs() >= 0.003 && (y - GRAVITY / 16.0).abs() < 0.003 {
            -0.003
        } else {
            y - GRAVITY / 16.0
        }
    }

    fn jump_from_ground(&mut self, c: &collide::Collider<impl World>) {
        let power = JUMP * self.block_jump_factor(c);
        if power <= 1.0e-5 {
            return;
        }
        self.velocity.y = self.velocity.y.max(power as f64);
        if self.sprinting {
            let yaw = self.yaw.to_radians();
            self.velocity.x += (-yaw.sin() * 0.2) as f64;
            self.velocity.z += (yaw.cos() * 0.2) as f64;
        }
    }

    fn block_pos(&self) -> [i32; 3] {
        self.pos.floor().as_ivec3().to_array()
    }

    /// `getBlockPosBelowThatAffectsMyMovement`: half a block below the feet.
    fn block_below(&self) -> [i32; 3] {
        [
            self.pos.x.floor() as i32,
            (self.pos.y - 0.500001).floor() as i32,
            self.pos.z.floor() as i32,
        ]
    }

    /// The block's factor at the feet, or else the one below, as `getBlockSpeedFactor` and
    /// `getBlockJumpFactor` pick them.
    fn block_factor(&self, c: &collide::Collider<impl World>, list: &[f32]) -> f32 {
        let factor = |[x, y, z]: [i32; 3]| list[c.world.state(x, y, z) as usize];
        let here = factor(self.block_pos());
        if here == 1.0 {
            factor(self.block_below())
        } else {
            here
        }
    }

    fn block_jump_factor(&self, c: &collide::Collider<impl World>) -> f32 {
        self.block_factor(c, &c.blocks.jump_factor)
    }

    fn on_climbable(&self, c: &collide::Collider<impl World>) -> bool {
        let [x, y, z] = self.block_pos();
        c.blocks.climbable[c.world.state(x, y, z) as usize]
    }

    /// `LivingEntity.travelInAir` with `handleRelativeFrictionAndCalculateMovement`.
    fn travel_in_air(&mut self, mv: glam::Vec2, jumping: bool, c: &collide::Collider<impl World>) {
        let [bx, by, bz] = self.block_below();
        let friction = if self.on_ground {
            c.blocks.friction[c.world.state(bx, by, bz) as usize]
        } else {
            1.0
        };

        // `getFrictionInfluencedSpeed`.
        let speed = if self.on_ground {
            let walk = (SPEED * if self.sprinting { 1.3 } else { 1.0 }) as f32;
            if friction as f64 > 0.6 {
                walk * (0.21600002 / (friction * friction * friction))
            } else {
                walk
            }
        } else if self.flying {
            self.flying_speed * if self.sprinting { 2.0 } else { 1.0 }
        } else if self.sprinting {
            0.025999999
        } else {
            0.02
        };
        self.velocity += input_vector(DVec3::new(mv.x as f64, 0.0, mv.y as f64), speed, self.yaw);

        let climbing = !self.flying && self.on_climbable(c);
        if climbing {
            self.fall_distance = 0.0;
            self.velocity.x = self.velocity.x.clamp(-0.15, 0.15);
            self.velocity.z = self.velocity.z.clamp(-0.15, 0.15);
            self.velocity.y = self.velocity.y.max(-0.15);
            // Sneaking holds you on a ladder.
            if self.velocity.y < 0.0 && self.crouching {
                self.velocity.y = 0.0;
            }
        }
        self.move_by(self.velocity, c);
        if (self.horizontal_collision || jumping) && climbing {
            self.velocity.y = 0.2;
        }

        let mut y = self.velocity.y;
        if c.world.loaded(bx, bz) {
            if !self.flying {
                y -= GRAVITY;
            }
        } else {
            y = if self.pos.y > -64.0 { -0.1 } else { 0.0 };
        }
        let drag = friction * 0.91;
        self.velocity = DVec3::new(
            self.velocity.x * drag as f64,
            y * 0.98f32 as f64,
            self.velocity.z * drag as f64,
        );
    }

    /// `Entity.move` for the player moving itself.
    fn move_by(&mut self, wanted: DVec3, c: &collide::Collider<impl World>) {
        let wanted = self.back_off_from_edge(wanted, c);
        let moved = c.collide(&self.bounding_box(), wanted, STEP, self.on_ground);
        self.pos += moved;

        let differs = |a: f64, b: f64| (a - b).abs() >= 1.0e-5;
        let (hit_x, hit_z) = (differs(wanted.x, moved.x), differs(wanted.z, moved.z));
        self.horizontal_collision = hit_x || hit_z;
        let vertical = wanted.y != moved.y;
        self.on_ground = vertical && wanted.y < 0.0;
        self.check_supporting_block(moved, c);
        // `LivingEntity.checkFallDamage`: out of water, see whether this move went in.
        if !self.in_water {
            self.update_fluid_interaction(c);
        }
        if !self.in_water && moved.y < 0.0 {
            self.fall_distance -= moved.y;
        }
        if self.on_ground && self.fall_distance > 0.0 && !self.flying {
            self.land(c);
        }
        if self.on_ground || self.flying {
            self.fall_distance = 0.0;
        }

        // `restituteMovementAfterCollisions`: stop along whatever was hit.
        if hit_x {
            self.velocity.x = 0.0;
        }
        if hit_z {
            self.velocity.z = 0.0;
        }
        if vertical {
            self.velocity.y = 0.0;
        }
        // `Player.getMovementEmission`: flying, or sneaking on the ground, makes no sound.
        if !(self.flying || self.on_ground && self.shift_key_down) {
            self.movement_sounds(moved, c);
        }
        let factor = self.block_factor(c, &c.blocks.speed_factor) as f64;
        self.velocity.x *= factor;
        self.velocity.z *= factor;
    }

    /// `Entity.checkSupportingBlock`: the block the player stands on, found under its box,
    /// or under where it was before this move.
    fn check_supporting_block(&mut self, moved: DVec3, c: &collide::Collider<impl World>) {
        if !self.on_ground {
            (self.supporting, self.on_ground_no_blocks) = (None, false);
            return;
        }
        let b = self.bounding_box();
        let area = Aabb::new(
            DVec3::new(b.min.x, b.min.y - 1.0e-6, b.min.z),
            DVec3::new(b.max.x, b.min.y, b.max.z),
        );
        let mut found = c.supporting_block(&area, self.pos);
        if found.is_some() || self.on_ground_no_blocks {
            self.supporting = found;
        } else {
            let back = DVec3::new(-moved.x, 0.0, -moved.z);
            found = c.supporting_block(&area.offset(back), self.pos);
            self.supporting = found;
        }
        self.on_ground_no_blocks = found.is_none();
    }

    /// `Entity.getOnPos(offset)`: the supporting block, moved down to `offset` below the
    /// feet unless it's a fence, wall or gate, whose tops stand higher.
    fn on_pos(&self, offset: f64, c: &collide::Collider<impl World>) -> [i32; 3] {
        let y = (self.pos.y - offset).floor() as i32;
        match self.supporting {
            Some(p) if offset > 1.0e-5 => {
                let state = c.world.state(p[0], p[1], p[2]);
                if c.blocks.fence_like(state, offset <= 0.5) {
                    p
                } else {
                    [p[0], y, p[2]]
                }
            }
            Some(p) => p,
            None => [self.pos.x.floor() as i32, y, self.pos.z.floor() as i32],
        }
    }

    /// `LivingEntity.causeFallDamage` as the client runs it: a fall that hurts thuds, and
    /// the block landed on sounds. A player who may fly takes no fall damage.
    fn land(&mut self, c: &collide::Collider<impl World>) {
        // `calculateFallDamage`, with `safe_fall_distance` 3.
        let damage = (self.fall_distance + 1.0e-6 - 3.0).floor();
        if self.may_fly || damage <= 0.0 {
            return;
        }
        let event = if damage > 4.0 {
            "minecraft:entity.player.big_fall"
        } else {
            "minecraft:entity.player.small_fall"
        };
        self.play(event, 1.0, 1.0);
        // `playBlockFallSound`.
        let (x, y, z) = (
            self.pos.x.floor() as i32,
            (self.pos.y - 0.2f32 as f64).floor() as i32,
            self.pos.z.floor() as i32,
        );
        let state = c.world.state(x, y, z);
        if !c.blocks.is_air(state) {
            let t = c.blocks.sound_type(state);
            let (event, volume, pitch) = (t.fall.clone(), t.volume * 0.5, t.pitch * 0.75);
            self.play(&event, volume, pitch);
        }
    }

    fn play(&mut self, event: &str, volume: f32, pitch: f32) {
        self.sounds.push(MoveSound::Play {
            event: event.to_owned(),
            volume,
            pitch,
        });
    }

    /// `Entity.applyMovementEmissionAndPlaySound`: a step sounds each block walked (or
    /// climbed), from the block stood on, or a stroke in water.
    fn movement_sounds(&mut self, moved: DVec3, c: &collide::Collider<impl World>) {
        let effect_pos = self.on_pos(0.2f32 as f64, c);
        let supporting_pos = self.on_pos(1.0e-5f32 as f64, c);
        let state_at = |[x, y, z]: [i32; 3]| c.world.state(x, y, z);
        let supporting = state_at(supporting_pos);
        let climbing = c.blocks.climbable[supporting as usize];
        let horizontal = (moved.x * moved.x + moved.z * moved.z).sqrt();
        self.move_dist += if climbing {
            (moved.length() * 0.6f32 as f64) as f32
        } else {
            (horizontal * 0.6f32 as f64) as f32
        };
        if self.move_dist <= self.next_step || c.blocks.is_air(supporting) {
            return;
        }
        let only_effect = supporting_pos == effect_pos;
        let mut produced = self.step_from_block(effect_pos, true, moved, c);
        if !only_effect {
            produced |= self.step_from_block(supporting_pos, false, moved, c);
        }
        if produced {
            self.next_step = self.move_dist.trunc() + 1.0;
        } else if self.in_water {
            self.next_step = self.move_dist.trunc() + 1.0;
            self.water_swim_sound();
        }
    }

    /// `Entity.vibrationAndSoundEffectsFromBlock`: whether the block makes a step, with its
    /// sound when `sound`.
    fn step_from_block(
        &mut self,
        pos: [i32; 3],
        sound: bool,
        moved: DVec3,
        c: &collide::Collider<impl World>,
    ) -> bool {
        let state = c.world.state(pos[0], pos[1], pos[2]);
        if c.blocks.is_air(state) {
            return false;
        }
        let climbable = c.blocks.climbable[state as usize];
        let crouching = self.pose == Pose::Crouching;
        if !((self.on_ground || climbable || crouching && moved.y == 0.0) && !self.swimming) {
            return false;
        }
        if sound {
            self.step_sound(pos, state, c);
        }
        true
    }

    /// `Player.playStepSound`: in water a stroke and the block muffled; otherwise a carpet,
    /// snow layer or the like on the block sounds instead of it, or with it.
    fn step_sound(&mut self, pos: [i32; 3], state: u32, c: &collide::Collider<impl World>) {
        let b = c.blocks;
        if self.in_water {
            self.water_swim_sound();
            self.muffled_step(state, b);
            return;
        }
        let above = [pos[0], pos[1] + 1, pos[2]];
        let above_state = c.world.state(above[0], above[1], above[2]);
        if b.inside_step_sound(above_state) || b.combination_step_sound(above_state) {
            let t = b.sound_type(above_state).clone();
            self.play(&t.step, t.volume * 0.15, t.pitch);
            if b.combination_step_sound(above_state) {
                self.muffled_step(state, b);
            }
        } else {
            let t = b.sound_type(state).clone();
            self.play(&t.step, t.volume * 0.15, t.pitch);
        }
    }

    /// `Entity.playMuffledStepSound`.
    fn muffled_step(&mut self, state: u32, b: &BlockPhysics) {
        let t = b.sound_type(state).clone();
        self.play(&t.step, t.volume * 0.05, t.pitch * 0.8);
    }

    /// `Entity.waterSwimSound`: louder the faster it moves.
    fn water_swim_sound(&mut self) {
        let v = self.velocity;
        let speed = ((v.x * v.x * 0.2 + v.y * v.y + v.z * v.z * 0.2).sqrt() as f32 * 0.35).min(1.0);
        self.sounds.push(MoveSound::Swim(speed));
    }

    /// `Player.maybeBackOffFromEdge`: sneaking stops short of an edge you'd fall off.
    fn back_off_from_edge(&self, wanted: DVec3, c: &collide::Collider<impl World>) -> DVec3 {
        let step = STEP as f64;
        if self.flying || wanted.y > 0.0 || !self.crouching || !self.is_above_ground(step, c) {
            return wanted;
        }
        const INC: f64 = 0.05;
        let (mut x, mut z) = (wanted.x, wanted.z);
        let (sx, sz) = (x.signum() * INC, z.signum() * INC);
        let falls = |dx: f64, dz: f64| self.can_fall_at_least(dx, dz, step, c);
        while x != 0.0 && falls(x, 0.0) {
            if x.abs() <= INC {
                x = 0.0;
                break;
            }
            x -= sx;
        }
        while z != 0.0 && falls(0.0, z) {
            if z.abs() <= INC {
                z = 0.0;
                break;
            }
            z -= sz;
        }
        while x != 0.0 && z != 0.0 && falls(x, z) {
            x = if x.abs() <= INC { 0.0 } else { x - sx };
            z = if z.abs() <= INC { 0.0 } else { z - sz };
        }
        DVec3::new(x, wanted.y, z)
    }

    fn is_above_ground(&self, step: f64, c: &collide::Collider<impl World>) -> bool {
        self.on_ground
            || (self.fall_distance < step
                && !self.can_fall_at_least(0.0, 0.0, step - self.fall_distance, c))
    }

    fn can_fall_at_least(
        &self,
        dx: f64,
        dz: f64,
        height: f64,
        c: &collide::Collider<impl World>,
    ) -> bool {
        let b = self.bounding_box();
        let probe = Aabb::new(
            DVec3::new(
                b.min.x + 1.0e-7 + dx,
                b.min.y - height - 1.0e-7,
                b.min.z + 1.0e-7 + dz,
            ),
            DVec3::new(b.max.x - 1.0e-7 + dx, b.min.y, b.max.z - 1.0e-7 + dz),
        );
        c.no_collision(&probe)
    }
}

/// `LocalPlayer.modifyInputSpeedForSquareMovement`: a diagonal reaches the square's corner.
fn square_movement(v: glam::Vec2) -> glam::Vec2 {
    let length = v.length();
    if length <= 0.0 {
        return v;
    }
    let unit = v * (1.0 / length);
    let (ax, ay) = (unit.x.abs(), unit.y.abs());
    let ratio = if ay > ax { ax / ay } else { ay / ax };
    let to_square = (1.0 + ratio * ratio).sqrt();
    unit * (length * to_square).min(1.0)
}

/// `Entity.getInputVector`: the input turned to face `yaw` and scaled to `speed`.
fn input_vector(input: DVec3, speed: f32, yaw: f32) -> DVec3 {
    let len2 = input.length_squared();
    if len2 < 1.0e-7 {
        return DVec3::ZERO;
    }
    let v = if len2 > 1.0 { input.normalize() } else { input } * speed as f64;
    let (sin, cos) = (yaw.to_radians().sin() as f64, yaw.to_radians().cos() as f64);
    DVec3::new(v.x * cos - v.z * sin, v.y, v.z * cos + v.x * sin)
}

#[cfg(test)]
mod tests;
