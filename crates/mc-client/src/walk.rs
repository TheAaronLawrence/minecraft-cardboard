//! Walking on a server: the player moves with vanilla's physics (`mc_physics`) at 20 ticks a
//! second against the blocks the server sent, and tells the server where it is the way
//! `LocalPlayer.sendChanges` does, so the server accepts the movement.

use glam::{DVec3, Vec3};
use mc_physics::{BlockPhysics, Input, Player};
use mc_protocol::codec::Put;
use mc_world::Blocks;
use std::collections::HashMap;

const TICK: f32 = 0.05;

/// The server's chunks' blocks, by chunk, then section y.
struct Collision {
    chunks: HashMap<(i32, i32), Vec<(i32, Blocks)>>,
    air: u32,
}

impl mc_physics::World for Collision {
    fn state(&self, x: i32, y: i32, z: i32) -> u32 {
        let Some(sections) = self.chunks.get(&(x.div_euclid(16), z.div_euclid(16))) else {
            return self.air;
        };
        let section_y = y.div_euclid(16);
        match sections.iter().find(|(s, _)| *s == section_y) {
            None => self.air,
            Some((_, Blocks::Uniform(id))) => *id,
            Some((_, Blocks::Mixed(ids))) => {
                let (x, y, z) = (x.rem_euclid(16), y.rem_euclid(16), z.rem_euclid(16));
                ids[((y * 16 + z) * 16 + x) as usize]
            }
        }
    }

    fn loaded(&self, x: i32, z: i32) -> bool {
        self.chunks
            .contains_key(&(x.div_euclid(16), z.div_euclid(16)))
    }
}

pub struct Walker {
    player: Player,
    /// Feet at the previous tick, to draw between ticks.
    previous: DVec3,
    placed: bool,
    since_tick: f32,
    collision: Collision,
    blocks: BlockPhysics,
    outbox: mc_protocol::Outbox,
    entity_id: i32,
    /// What the server last heard: position, rotation, input, sprinting, flying.
    sent_pos: DVec3,
    sent_rot: (f32, f32),
    sent_status: (bool, bool),
    sent_input: Input,
    sent_sprinting: bool,
    sent_flying: bool,
    position_reminder: u32,
}

impl Walker {
    pub fn new(
        outbox: mc_protocol::Outbox,
        registry: &mc_registry::Registry,
    ) -> anyhow::Result<Self> {
        let air = registry.block("minecraft:air").expect("air").default_state;
        Ok(Self {
            player: Player::new(DVec3::ZERO, 0.0, 0.0),
            previous: DVec3::ZERO,
            placed: false,
            since_tick: 0.0,
            collision: Collision {
                chunks: HashMap::new(),
                air,
            },
            blocks: BlockPhysics::vanilla(registry)?,
            outbox,
            entity_id: 0,
            sent_pos: DVec3::ZERO,
            sent_rot: (0.0, 0.0),
            sent_status: (false, false),
            sent_input: Input::default(),
            sent_sprinting: false,
            sent_flying: false,
            position_reminder: 0,
        })
    }

    pub fn chunk(&mut self, key: (i32, i32), blocks: Option<Vec<(i32, Blocks)>>) {
        match blocks {
            Some(b) => self.collision.chunks.insert(key, b),
            None => self.collision.chunks.remove(&key),
        };
    }

    pub fn server_event(&mut self, event: mc_protocol::Event) {
        match event {
            mc_protocol::Event::Login { entity_id } => self.entity_id = entity_id,
            mc_protocol::Event::Abilities {
                flying,
                may_fly,
                flying_speed,
            } => {
                self.player.may_fly = may_fly;
                self.player.flying = flying;
                self.player.flying_speed = flying_speed;
                self.sent_flying = flying;
            }
            _ => {}
        }
    }

    /// The server put the player here (feet).
    pub fn teleport(&mut self, feet: Vec3, yaw: f32, pitch: f32) {
        let feet = feet.as_dvec3();
        self.player.teleport(feet, yaw, pitch);
        (self.previous, self.sent_pos, self.sent_rot) = (feet, feet, (yaw, pitch));
        self.placed = true;
    }

    /// Runs the ticks `dt` seconds hold, looking where the camera looks, and returns the
    /// eye position to draw from, between the last two ticks.
    pub fn frame(&mut self, dt: f32, input: Input, yaw: f32, pitch: f32) -> Option<Vec3> {
        if !self.placed {
            return None;
        }
        self.since_tick += dt;
        while self.since_tick >= TICK {
            self.since_tick -= TICK;
            (self.player.yaw, self.player.pitch) = (yaw, pitch);
            self.previous = self.player.pos;
            self.player.tick(input, &self.collision, &self.blocks);
            if let Err(e) = self.send_changes(input) {
                eprintln!("sending movement: {e:#}");
            }
        }
        let feet = self
            .previous
            .lerp(self.player.pos, (self.since_tick / TICK) as f64);
        Some((feet + DVec3::Y * self.player.eye_height()).as_vec3())
    }

    fn send_changes(&mut self, input: Input) -> anyhow::Result<()> {
        let p = &self.player;
        let out = &self.outbox;
        if input != self.sent_input {
            out.send("player_input", &[input.bits()])?;
            self.sent_input = input;
        }
        if p.sprinting != self.sent_sprinting {
            // `ServerboundPlayerCommandPacket.Action`: START_SPRINTING 1, STOP_SPRINTING 2.
            let mut body = Vec::new();
            body.put_varint(self.entity_id)
                .put_varint(if p.sprinting { 1 } else { 2 })
                .put_varint(0);
            out.send("player_command", &body)?;
            self.sent_sprinting = p.sprinting;
        }
        if p.flying != self.sent_flying {
            out.send("player_abilities", &[if p.flying { 2 } else { 0 }])?;
            self.sent_flying = p.flying;
        }

        // `LocalPlayer.sendPosition`: position when it moved over 0.0002 blocks or every
        // 20 ticks, rotation when it changed, otherwise the ground flags when they changed.
        self.position_reminder += 1;
        let moved = (p.pos - self.sent_pos).length_squared() > 2.0e-4f64 * 2.0e-4
            || self.position_reminder >= 20;
        let turned = (p.yaw, p.pitch) != self.sent_rot;
        let status = (p.on_ground, p.horizontal_collision);
        let flags = status.0 as u8 | (status.1 as u8) << 1;
        let mut body = Vec::new();
        if moved {
            body.put_f64(p.pos.x).put_f64(p.pos.y).put_f64(p.pos.z);
        }
        if turned {
            body.put_f32(p.yaw).put_f32(p.pitch);
        }
        body.put_u8(flags);
        let name = match (moved, turned) {
            (true, true) => Some("move_player_pos_rot"),
            (true, false) => Some("move_player_pos"),
            (false, true) => Some("move_player_rot"),
            (false, false) => (status != self.sent_status).then_some("move_player_status_only"),
        };
        if let Some(name) = name {
            out.send(name, &body)?;
        }
        if moved {
            self.sent_pos = p.pos;
            self.position_reminder = 0;
        }
        if turned {
            self.sent_rot = (p.yaw, p.pitch);
        }
        self.sent_status = status;
        out.send("client_tick_end", &[])
    }
}

/// The keys held, as vanilla's default bindings read them.
pub fn input(keys: &std::collections::HashSet<winit::keyboard::KeyCode>) -> Input {
    use winit::keyboard::KeyCode as K;
    Input {
        forward: keys.contains(&K::KeyW),
        backward: keys.contains(&K::KeyS),
        left: keys.contains(&K::KeyA),
        right: keys.contains(&K::KeyD),
        jump: keys.contains(&K::Space),
        sneak: keys.contains(&K::ShiftLeft),
        sprint: keys.contains(&K::ControlLeft),
    }
}
