//! How the player moves, as vanilla 26.3's `LocalPlayer`, `LivingEntity` and `Entity` do it:
//! walking, sprinting and sneaking input, jumping, gravity and drag, block friction and
//! speed factors, collision with block shapes and stepping up, backing off edges while
//! sneaking, ladders, and creative flight.
//!
//! `pick` finds the block the player looks at, against block outline shapes, as
//! `Entity.pick` does; `BlockPhysics::destroy_progress` is how far a tick of mining gets.
//!
//! Collision and outline shapes, block friction and destroy times come from `resources/reports/physics.json`, which
//! `cargo xtask gen-reports` dumps from vanilla, so they match it exactly. Not yet covered:
//! swimming and other fluid movement, bouncing on slime and beds, cobwebs and powder snow,
//! and entities as obstacles.

mod collide;
mod pick;

use anyhow::{Context, Result};
pub use collide::Aabb;
use glam::DVec3;
use mc_registry::Registry;
pub use pick::{BlockHit, pick, view_vector};
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

        let mut climbable = vec![false; n];
        for name in block_tag("minecraft:climbable")? {
            if let Some(block) = registry.block(&name) {
                let first = block.first_state as usize;
                climbable[first..first + block.state_count() as usize].fill(true);
            }
        }
        Ok(Self {
            shapes,
            edges,
            state_shapes,
            outlines,
            destroy_time,
            requires_tool,
            friction,
            speed_factor,
            jump_factor,
            climbable,
        })
    }

    fn shape(&self, state: u32) -> &[Aabb] {
        self.state_shapes
            .get(state as usize)
            .map_or(&[], |&i| &self.shapes[i as usize])
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
const HEIGHT: f64 = 1.8;
const CROUCH_HEIGHT: f64 = 1.5;
const EYE: f64 = 1.62;
const CROUCH_EYE: f64 = 1.27;
const STEP: f32 = 0.6;
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
    pub crouching: bool,
    pub flying: bool,
    /// May start flying by double-tapping jump (creative and spectator).
    pub may_fly: bool,
    /// `Abilities.flyingSpeed`, 0.05 by default.
    pub flying_speed: f32,
    no_jump_delay: u32,
    jump_trigger_time: u32,
    was_jumping: bool,
    fall_distance: f64,
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
            flying: false,
            may_fly: false,
            flying_speed: 0.05,
            no_jump_delay: 0,
            jump_trigger_time: 0,
            was_jumping: false,
            fall_distance: 0.0,
        }
    }

    pub fn eye_height(&self) -> f64 {
        if self.crouching { CROUCH_EYE } else { EYE }
    }

    /// Where the server put the player; any motion stops.
    pub fn teleport(&mut self, pos: DVec3, yaw: f32, pitch: f32) {
        (self.pos, self.yaw, self.pitch) = (pos, yaw, pitch);
        self.velocity = DVec3::ZERO;
        self.fall_distance = 0.0;
    }

    fn bounding_box(&self) -> Aabb {
        let height = if self.crouching {
            CROUCH_HEIGHT
        } else {
            HEIGHT
        };
        Aabb::at_feet(self.pos, WIDTH, height)
    }

    /// One game tick (1/20 s) with these keys held.
    pub fn tick(&mut self, input: Input, world: &impl World, blocks: &BlockPhysics) {
        // Vanilla holds the player still until the chunk under it has arrived.
        if !world.loaded(self.pos.x.floor() as i32, self.pos.z.floor() as i32) {
            return;
        }
        let collider = collide::Collider { world, blocks };

        // Pose: crouching while sneak is held and not flying, standing only where it fits.
        let wants_crouch = input.sneak && !self.flying;
        if wants_crouch {
            self.crouching = true;
        } else if self.crouching {
            let standing = Aabb::at_feet(self.pos, WIDTH, HEIGHT).deflate(1e-7);
            self.crouching = !collider.no_collision(&standing);
        }

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
            if self.crouching {
                mv *= 0.3; // `sneaking_speed`
            }
            mv = square_movement(mv);
        }

        // Sprinting: started by the sprint key while moving forward, stopped by letting go
        // of forward, sneaking, or walking into a wall.
        let forward = mv.y > 1.0e-5;
        if self.sprinting && (!forward || self.crouching || self.horizontal_collision) {
            self.sprinting = false;
        } else if !self.sprinting && input.sprint && forward && !self.crouching {
            self.sprinting = true;
        }

        // Double-tapping jump toggles flight, within 7 ticks.
        if input.jump && !self.was_jumping && self.may_fly {
            if self.jump_trigger_time == 0 {
                self.jump_trigger_time = 7;
            } else {
                self.flying = !self.flying;
                self.jump_trigger_time = 0;
            }
        }
        self.was_jumping = input.jump;
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
        let jumping = input.jump && !self.flying;
        if jumping {
            if self.on_ground && self.no_jump_delay == 0 {
                self.jump_from_ground(&collider);
                self.no_jump_delay = 10;
            }
        } else {
            self.no_jump_delay = 0;
        }

        // `Player.travel`: flight keeps 60% of the vertical speed instead of falling.
        let y_before = self.velocity.y;
        self.travel_in_air(mv, jumping, &collider);
        if self.flying {
            self.velocity.y = y_before * 0.6;
            if self.on_ground {
                self.flying = false;
            }
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
        if self.on_ground || self.flying {
            self.fall_distance = 0.0;
        } else if moved.y < 0.0 {
            self.fall_distance -= moved.y;
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
        let factor = self.block_factor(c, &c.blocks.speed_factor) as f64;
        self.velocity.x *= factor;
        self.velocity.z *= factor;
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
