//! Walking on a server: the player moves with vanilla's physics (`mc_physics`) at 20 ticks a
//! second against the blocks the server sent, and tells the server where it is the way
//! `LocalPlayer.sendChanges` does, so the server accepts the movement. It also mines and
//! places blocks as `Minecraft.handleKeybinds` and `MultiPlayerGameMode` do: the server
//! changes the blocks and sends them back, so nothing here changes the world itself.

use glam::{DVec3, IVec3, Vec3};
use mc_physics::{BlockHit, BlockPhysics, Input, Player};
use mc_protocol::codec::Put;
use mc_world::Blocks;
use std::collections::HashMap;

const TICK: f32 = 0.05;

/// The server's chunks' blocks, by chunk, then section y.
struct Collision {
    chunks: HashMap<(i32, i32), Vec<(i32, Blocks)>>,
    air: u32,
}

impl Collision {
    fn set(&mut self, [x, y, z]: [i32; 3], state: u32) {
        let Some(sections) = self.chunks.get_mut(&(x.div_euclid(16), z.div_euclid(16))) else {
            return;
        };
        if let Some((_, blocks)) = sections.iter_mut().find(|(s, _)| *s == y.div_euclid(16)) {
            if let Blocks::Uniform(id) = *blocks {
                *blocks = Blocks::Mixed(Box::new([id; 4096]));
            }
            if let Blocks::Mixed(ids) = blocks {
                let (x, y, z) = (x.rem_euclid(16), y.rem_euclid(16), z.rem_euclid(16));
                ids[((y * 16 + z) * 16 + x) as usize] = state;
            }
        }
    }
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

/// The mouse buttons held: attack (left) and use (right).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Buttons {
    pub attack: bool,
    pub use_item: bool,
}

/// A mouse button press, counted until the next tick handles it, as vanilla's
/// `KeyMapping.consumeClick` does, so a click shorter than a tick still counts.
#[derive(Clone, Copy)]
pub enum Click {
    Attack,
    Use,
    Pick,
}

/// `ServerboundPlayerActionPacket.Action`.
const START_DESTROY_BLOCK: i32 = 0;
const ABORT_DESTROY_BLOCK: i32 = 2;
const STOP_DESTROY_BLOCK: i32 = 3;

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
    /// Creative: blocks break at once and reach is longer.
    instabuild: bool,
    /// The hotbar slot selected, and the one the server last heard of.
    selected: u8,
    sent_selected: u8,
    /// The block looked at, from the last frame.
    hit: Option<BlockHit>,
    /// The block being mined, its face, and how much of it is broken (0 to 1).
    destroying: Option<(IVec3, u8, f32)>,
    destroy_delay: u32,
    right_click_delay: u32,
    miss_time: u32,
    /// Numbers each action the server acknowledges.
    sequence: i32,
    /// What the hotbar holds.
    hotbar: [Option<(String, i32)>; 9],
    /// Other players mining blocks, by entity id: the block and crack stage.
    others_mining: HashMap<i32, (IVec3, u8)>,
    /// Presses not yet handled, by `Click`.
    clicks: [u32; 3],
    /// Air, cave air and void air.
    airs: [u32; 3],
}

impl Walker {
    pub fn new(
        outbox: mc_protocol::Outbox,
        registry: &mc_registry::Registry,
    ) -> anyhow::Result<Self> {
        let state = |name: &str| registry.block(name).map(|b| b.default_state);
        let air = state("minecraft:air").expect("air");
        let airs = [
            air,
            state("minecraft:cave_air").unwrap_or(air),
            state("minecraft:void_air").unwrap_or(air),
        ];
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
            instabuild: false,
            selected: 0,
            sent_selected: 0,
            hit: None,
            destroying: None,
            destroy_delay: 0,
            right_click_delay: 0,
            miss_time: 0,
            sequence: 0,
            hotbar: Default::default(),
            others_mining: HashMap::new(),
            clicks: [0; 3],
            airs,
        })
    }

    pub fn chunk(&mut self, key: (i32, i32), blocks: Option<Vec<(i32, Blocks)>>) {
        match blocks {
            Some(b) => self.collision.chunks.insert(key, b),
            None => self.collision.chunks.remove(&key),
        };
    }

    /// The server changed these blocks.
    pub fn blocks(&mut self, changes: &[([i32; 3], u32)]) {
        for &(pos, state) in changes {
            self.collision.set(pos, state);
        }
    }

    pub fn server_event(&mut self, event: mc_protocol::Event) {
        match event {
            mc_protocol::Event::Login { entity_id } => self.entity_id = entity_id,
            mc_protocol::Event::Abilities {
                flying,
                may_fly,
                instabuild,
                flying_speed,
            } => {
                self.instabuild = instabuild;
                self.player.may_fly = may_fly;
                self.player.flying = flying;
                self.player.flying_speed = flying_speed;
                self.sent_flying = flying;
            }
            mc_protocol::Event::HeldSlot(slot) => {
                (self.selected, self.sent_selected) = (slot, slot)
            }
            mc_protocol::Event::Hotbar(slots) => {
                for (slot, item) in slots {
                    self.hotbar[slot as usize] = item;
                }
            }
            mc_protocol::Event::Destruction { id, pos, stage } => match stage {
                Some(stage) => {
                    self.others_mining.insert(id, (IVec3::from(pos), stage));
                }
                None => {
                    self.others_mining.remove(&id);
                }
            },
            _ => {}
        }
    }

    pub fn click(&mut self, click: Click) {
        self.clicks[click as usize] += 1;
    }

    /// Selects a hotbar slot, 0 to 8.
    pub fn select(&mut self, slot: u8) {
        self.selected = slot.min(8);
    }

    /// Moves the hotbar selection by `steps` slots, wrapping, as the scroll wheel does.
    pub fn scroll(&mut self, steps: i32) {
        self.selected = (self.selected as i32 + steps).rem_euclid(9) as u8;
    }

    /// What the hotbar shows.
    pub fn hotbar(&self) -> mc_render::hud::Hotbar {
        mc_render::hud::Hotbar {
            slots: self.hotbar.clone(),
            selected: self.selected,
        }
    }

    /// The blocks being mined, by us or others, with their states and crack stages
    /// (`MultiPlayerGameMode.getDestroyStage`: a tenth of the progress, rounded down).
    pub fn cracks(&self) -> Vec<([i32; 3], u32, u8)> {
        let own = self
            .destroying
            .filter(|&(.., progress)| progress > 0.0)
            .map(|(pos, _, progress)| (pos, ((progress * 10.0) as u8).min(9)));
        own.into_iter()
            .chain(self.others_mining.values().copied())
            .map(|(pos, stage)| {
                let state = mc_physics::World::state(&self.collision, pos.x, pos.y, pos.z);
                (pos.to_array(), state, stage)
            })
            .collect()
    }

    /// The block looked at and its state, to outline.
    pub fn target(&self) -> Option<(IVec3, u32)> {
        let pos = self.hit?.pos;
        Some((
            pos,
            mc_physics::World::state(&self.collision, pos.x, pos.y, pos.z),
        ))
    }

    /// The edges of the outline around the block looked at, in world coordinates.
    pub fn outline_edges(&self) -> Vec<[f32; 6]> {
        let Some((pos, state)) = self.target() else {
            return Vec::new();
        };
        let o = pos.as_dvec3();
        self.blocks
            .outline_edges(state)
            .iter()
            .map(|e| std::array::from_fn(|i| (e[i] + o[i % 3]) as f32))
            .collect()
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
    pub fn frame(
        &mut self,
        dt: f32,
        input: Input,
        buttons: Buttons,
        yaw: f32,
        pitch: f32,
    ) -> Option<Vec3> {
        if !self.placed {
            return None;
        }
        self.since_tick += dt;
        while self.since_tick >= TICK {
            self.since_tick -= TICK;
            (self.player.yaw, self.player.pitch) = (yaw, pitch);
            if let Err(e) = self.tick_hands(buttons) {
                eprintln!("sending block actions: {e:#}");
            }
            self.previous = self.player.pos;
            self.player.tick(input, &self.collision, &self.blocks);
            if let Err(e) = self.send_changes(input) {
                eprintln!("sending movement: {e:#}");
            }
        }
        let feet = self
            .previous
            .lerp(self.player.pos, (self.since_tick / TICK) as f64);
        let eye = feet + DVec3::Y * self.player.eye_height();
        // `Player.blockInteractionRange`: 4.5, and half a block more in creative.
        let reach = if self.instabuild { 5.0 } else { 4.5 };
        let look = mc_physics::view_vector(yaw, pitch);
        self.hit = mc_physics::pick(&self.collision, &self.blocks, eye, eye + look * reach);
        Some(eye.as_vec3())
    }

    /// One tick of `Minecraft.handleKeybinds` for the mouse: clicks start mining, using and
    /// picking; holding keeps mining, and keeps using every 4 ticks.
    fn tick_hands(&mut self, b: Buttons) -> anyhow::Result<()> {
        self.right_click_delay = self.right_click_delay.saturating_sub(1);
        self.miss_time = self.miss_time.saturating_sub(1);
        self.send_selected_slot()?;
        let hit = self.hit;
        let [attacks, uses, picks] = std::mem::take(&mut self.clicks);
        for _ in 0..attacks {
            self.start_attack(hit)?;
        }
        for _ in 0..uses {
            self.start_use(hit)?;
        }
        if let Some(h) = hit.filter(|_| picks > 0) {
            let mut body = Vec::new();
            body.put_block_pos(h.pos.to_array()).put_bool(false);
            self.outbox.send("pick_item_from_block", &body)?;
        }
        if b.use_item && self.right_click_delay == 0 {
            self.start_use(hit)?;
        }
        // A click shorter than a tick mines for that tick too.
        self.continue_attack(b.attack || attacks > 0, hit)?;
        Ok(())
    }

    /// `MultiPlayerGameMode.ensureHasSentCarriedItem`.
    fn send_selected_slot(&mut self) -> anyhow::Result<()> {
        if self.selected != self.sent_selected {
            let mut body = Vec::new();
            body.put_u16(self.selected as u16);
            self.outbox.send("set_carried_item", &body)?;
            self.sent_selected = self.selected;
        }
        Ok(())
    }

    fn is_air(&self, pos: IVec3) -> bool {
        let state = mc_physics::World::state(&self.collision, pos.x, pos.y, pos.z);
        self.airs.contains(&state)
    }

    fn player_action(
        &mut self,
        action: i32,
        pos: IVec3,
        face: u8,
        predicted: bool,
    ) -> anyhow::Result<()> {
        let sequence = if predicted {
            self.sequence += 1;
            self.sequence
        } else {
            0
        };
        let mut body = Vec::new();
        body.put_varint(action)
            .put_block_pos(pos.to_array())
            .put_u8(face)
            .put_varint(sequence);
        self.outbox.send("player_action", &body)
    }

    /// `Minecraft.startAttack`: a click on a block starts mining it; on nothing, a punch.
    fn start_attack(&mut self, hit: Option<BlockHit>) -> anyhow::Result<()> {
        if self.miss_time > 0 {
            return Ok(());
        }
        match hit {
            Some(h) if !self.is_air(h.pos) => self.start_destroy(h.pos, h.face),
            Some(_) => Ok(()),
            None => {
                if !self.instabuild {
                    self.miss_time = 10;
                }
                self.outbox.send("punch", &[])
            }
        }
    }

    /// `Minecraft.continueAttack`.
    fn continue_attack(&mut self, held: bool, hit: Option<BlockHit>) -> anyhow::Result<()> {
        if !held {
            self.miss_time = 0;
        }
        if self.miss_time > 0 {
            return Ok(());
        }
        match hit {
            Some(h) if held => {
                if !self.is_air(h.pos) {
                    self.continue_destroy(h.pos, h.face)?;
                }
                Ok(())
            }
            _ => self.stop_destroy(),
        }
    }

    /// `MultiPlayerGameMode.startDestroyBlock`.
    fn start_destroy(&mut self, pos: IVec3, face: u8) -> anyhow::Result<()> {
        if self.instabuild {
            self.destroy_delay = 5;
            return self.player_action(START_DESTROY_BLOCK, pos, face, true);
        }
        if self.destroying.is_some_and(|(p, ..)| p == pos) {
            return Ok(());
        }
        if let Some((old, ..)) = self.destroying {
            self.player_action(ABORT_DESTROY_BLOCK, old, face, false)?;
        }
        let state = mc_physics::World::state(&self.collision, pos.x, pos.y, pos.z);
        let progress = self.blocks.destroy_progress(state, self.player.on_ground);
        // A block that breaks in one tick breaks on the click; the server agrees.
        self.destroying = (progress < 1.0).then_some((pos, face, 0.0));
        self.player_action(START_DESTROY_BLOCK, pos, face, true)
    }

    /// `MultiPlayerGameMode.continueDestroyBlock`: a tick of mining.
    fn continue_destroy(&mut self, pos: IVec3, face: u8) -> anyhow::Result<()> {
        if self.destroy_delay > 0 {
            self.destroy_delay -= 1;
            return Ok(());
        }
        if self.instabuild {
            self.destroy_delay = 5;
            return self.player_action(START_DESTROY_BLOCK, pos, face, true);
        }
        match self.destroying {
            Some((p, f, progress)) if p == pos => {
                let state = mc_physics::World::state(&self.collision, pos.x, pos.y, pos.z);
                let progress =
                    progress + self.blocks.destroy_progress(state, self.player.on_ground);
                if progress >= 1.0 {
                    self.destroying = None;
                    self.destroy_delay = 5;
                    self.player_action(STOP_DESTROY_BLOCK, pos, f, true)
                } else {
                    self.destroying = Some((p, f, progress));
                    Ok(())
                }
            }
            _ => self.start_destroy(pos, face),
        }
    }

    /// `MultiPlayerGameMode.stopDestroyBlock`: letting go stops mining.
    fn stop_destroy(&mut self) -> anyhow::Result<()> {
        match self.destroying.take() {
            // Vanilla says down here, whatever face was mined.
            Some((pos, ..)) => self.player_action(ABORT_DESTROY_BLOCK, pos, 0, false),
            None => Ok(()),
        }
    }

    /// `Minecraft.startUseItem` with the main hand: use the held item on the block looked
    /// at (placing it, if it's a block), or on nothing.
    fn start_use(&mut self, hit: Option<BlockHit>) -> anyhow::Result<()> {
        self.right_click_delay = 4;
        if self.destroying.is_some() {
            return Ok(());
        }
        self.sequence += 1;
        let mut body = Vec::new();
        body.put_varint(0); // main hand
        match hit {
            Some(h) => {
                let cursor = h.location - h.pos.as_dvec3();
                body.put_block_pos(h.pos.to_array())
                    .put_varint(h.face as i32)
                    .put_f32(cursor.x as f32)
                    .put_f32(cursor.y as f32)
                    .put_f32(cursor.z as f32)
                    .put_bool(h.inside)
                    .put_bool(false) // world border hit
                    .put_varint(self.sequence);
                self.outbox.send("use_item_on", &body)
            }
            None => {
                body.put_varint(self.sequence)
                    .put_f32(self.player.yaw)
                    .put_f32(self.player.pitch);
                self.outbox.send("use_item", &body)
            }
        }
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
