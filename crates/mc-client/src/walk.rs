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

/// A section's sky and block light, each 4 bits a block, or `None` when the server hasn't
/// sent that layer.
pub type SectionLight = [Option<Box<[u8; 2048]>>; 2];

/// The server's light, by chunk, then section y: a copy of what the streaming thread keeps
/// for meshing, which entities are lit by.
#[derive(Default)]
struct Light {
    chunks: HashMap<(i32, i32), HashMap<i32, SectionLight>>,
}

impl Light {
    /// Sets these sections' light; a layer left out keeps what it had, unless `whole`
    /// replaces the chunk's light.
    fn set(&mut self, key: (i32, i32), sections: Vec<(i32, SectionLight)>, whole: bool) {
        let chunk = self.chunks.entry(key).or_default();
        if whole {
            chunk.clear();
        }
        for (y, layers) in sections {
            let section = chunk.entry(y).or_default();
            for (old, new) in section.iter_mut().zip(layers) {
                if new.is_some() {
                    *old = new;
                }
            }
        }
    }

    /// Sky and block light at a block, read as `WorldView.light` reads it: a section
    /// without sky light takes it from the nearest section above that has some, and above
    /// them all it's 15. Outside the loaded chunks it's full sky light.
    fn at(&self, [x, y, z]: [i32; 3]) -> [u8; 2] {
        let Some(sections) = self.chunks.get(&(x.div_euclid(16), z.div_euclid(16))) else {
            return [15, 0];
        };
        let (lx, lz) = (x.rem_euclid(16) as usize, z.rem_euclid(16) as usize);
        let section_y = y.div_euclid(16);
        let read = |section: i32, layer: usize, ly: usize| {
            let light = sections.get(&section)?[layer].as_deref()?;
            Some(mc_world::nibble(light, lx, ly, lz))
        };
        let ly = y.rem_euclid(16) as usize;
        let sky = read(section_y, 0, ly).unwrap_or_else(|| {
            let mut above: Vec<i32> = sections
                .keys()
                .copied()
                .filter(|&s| s > section_y)
                .collect();
            above.sort_unstable();
            above.into_iter().find_map(|s| read(s, 0, 0)).unwrap_or(15)
        });
        [sky, read(section_y, 1, ly).unwrap_or(0)]
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
    light: Light,
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
    /// The inventory, whose last row is the hotbar.
    inventory: crate::inventory::Inventory,
    /// Other players mining blocks, by entity id: the block and crack stage.
    others_mining: HashMap<i32, (IVec3, u8)>,
    /// Other players, mobs and dropped items.
    entities: crate::entities::Entities,
    /// Downloads players' skins as they join the tab list.
    skins: crate::skins::Loader,
    /// Which player's skin each of the atlas's skin slots holds.
    skin_slots: [Option<u128>; mc_render::entity::SKIN_SLOTS],
    /// Presses not yet handled, by `Click`.
    clicks: [u32; 3],
    /// Air, cave air and void air.
    airs: [u32; 3],
    /// `GameType`'s id; survival and adventure (0 and 2) show the hearts, food and XP.
    game_mode: u8,
    hardcore: bool,
    health: Health,
    food: i32,
    saturation: f32,
    xp_progress: f32,
    xp_level: i32,
    /// `LocalPlayer.showDeathScreen`: false when the `immediate_respawn` game rule is on.
    show_death_screen: bool,
    /// `Player.getScore`, which the death screen shows.
    score: i32,
    /// The death screen, while it's open.
    death: Option<Death>,
    /// A respawn was asked for and the server hasn't respawned the player yet.
    respawning: bool,
}

/// The open death screen: its message, ticks since it (or its confirmation) opened, and
/// whether it's asking to confirm leaving.
struct Death {
    message: Option<String>,
    ticks: u32,
    confirm: bool,
}

/// What a click on the death screen asks the game to do.
#[derive(PartialEq, Eq, Debug)]
pub enum ScreenAction {
    None,
    /// Title Screen: leaves the game, which takes the player back to the launcher when the
    /// launcher started it.
    Quit,
}

/// The player's health as `LocalPlayer` keeps it, and what `Hud` keeps to blink the hearts
/// when it changes.
struct Health {
    value: f32,
    /// `LocalPlayer.flashOnSetHealth`: the first health the server sends doesn't flash.
    flash: bool,
    /// `LivingEntity.damageCooldownTime`, counted down each tick.
    damage_cooldown: u32,
    /// `Hud.tickCount`.
    tick: i32,
    last: i32,
    display: i32,
    last_time: std::time::Instant,
    blink_until: i32,
}

impl Health {
    /// `LocalPlayer.hurtTo`.
    fn hurt_to(&mut self, health: f32) {
        if self.flash {
            let damage = self.value - health;
            if damage < 0.0 {
                self.damage_cooldown = 10;
            } else if damage > 0.0 {
                self.damage_cooldown = 20;
            }
        }
        self.flash = true;
        self.value = health;
    }

    /// The part of `Hud.extractPlayerHealth` that runs each frame: when the health changes
    /// while hurt or healing, the hearts blink, and the old health shows for a second.
    /// Returns the health, the old health and whether the hearts blink now.
    fn frame(&mut self) -> (i32, i32, bool) {
        let current = self.value.ceil() as i32;
        let blink = self.blink_until > self.tick && (self.blink_until - self.tick) / 3 % 2 == 1;
        let now = std::time::Instant::now();
        if current < self.last && self.damage_cooldown > 0 {
            self.last_time = now;
            self.blink_until = self.tick + 20;
        } else if current > self.last && self.damage_cooldown > 0 {
            self.last_time = now;
            self.blink_until = self.tick + 10;
        }
        if now.duration_since(self.last_time).as_millis() > 1000 {
            self.display = current;
            self.last_time = now;
        }
        self.last = current;
        (current, self.display, blink)
    }
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
            light: Light::default(),
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
            inventory: crate::inventory::Inventory::new(registry),
            others_mining: HashMap::new(),
            entities: crate::entities::Entities::new(registry),
            skins: crate::skins::Loader::new(),
            skin_slots: [None; mc_render::entity::SKIN_SLOTS],
            clicks: [0; 3],
            airs,
            game_mode: 0,
            hardcore: false,
            // A new `LocalPlayer`'s: full health and food, saturation 5.
            health: Health {
                value: 20.0,
                flash: false,
                damage_cooldown: 0,
                tick: 0,
                last: 0,
                display: 0,
                last_time: std::time::Instant::now(),
                blink_until: 0,
            },
            food: 20,
            saturation: 5.0,
            xp_progress: 0.0,
            xp_level: 0,
            show_death_screen: true,
            score: 0,
            death: None,
            respawning: false,
        })
    }

    pub fn chunk(&mut self, key: (i32, i32), blocks: Option<Vec<(i32, Blocks)>>) {
        match blocks {
            Some(b) => {
                self.collision.chunks.insert(key, b);
            }
            None => {
                self.collision.chunks.remove(&key);
                self.light.chunks.remove(&key);
            }
        }
    }

    /// The server's light for these sections of a chunk; `whole` replaces all it had.
    pub fn light(&mut self, key: (i32, i32), sections: Vec<(i32, SectionLight)>, whole: bool) {
        self.light.set(key, sections, whole);
    }

    /// The server changed these blocks.
    pub fn blocks(&mut self, changes: &[([i32; 3], u32)]) {
        for &(pos, state) in changes {
            self.collision.set(pos, state);
        }
    }

    pub fn server_event(&mut self, event: mc_protocol::Event) {
        let Some(event) = self.entities.event(event) else {
            return;
        };
        match event {
            mc_protocol::Event::AddPlayers(profiles) => {
                for profile in profiles {
                    self.entities.set_name(profile.uuid, Some(profile.name));
                    if let Some(textures) = profile.textures {
                        self.skins.request(profile.uuid, textures);
                    }
                }
            }
            mc_protocol::Event::RemovePlayers(ids) => {
                for uuid in ids {
                    for slot in &mut self.skin_slots {
                        if *slot == Some(uuid) {
                            *slot = None;
                        }
                    }
                    self.entities.set_skin(uuid, None);
                    self.entities.set_name(uuid, None);
                }
            }
            mc_protocol::Event::Login {
                entity_id,
                hardcore,
                game_mode,
                show_death_screen,
            } => {
                (self.entity_id, self.hardcore, self.game_mode) = (entity_id, hardcore, game_mode);
                self.show_death_screen = show_death_screen;
            }
            mc_protocol::Event::ShowDeathScreen(show) => self.show_death_screen = show,
            mc_protocol::Event::Score(score) => self.score = score,
            // `ClientPacketListener.handlePlayerCombatKill`.
            mc_protocol::Event::Died { entity_id, message } if entity_id == self.entity_id => {
                if self.show_death_screen {
                    self.open_death_screen(Some(message));
                } else {
                    self.respawn();
                }
            }
            // `handleRespawn`: a new player, whose first health doesn't flash, and the death
            // screen closes.
            mc_protocol::Event::Respawn { game_mode } => {
                self.game_mode = game_mode;
                self.death = None;
                self.respawning = false;
                self.health.value = 20.0;
                self.health.flash = false;
            }
            mc_protocol::Event::GameMode(mode) => self.game_mode = mode,
            // `ClientPacketListener.handleSetHealth`.
            mc_protocol::Event::Health {
                health,
                food,
                saturation,
            } => {
                self.health.hurt_to(health);
                (self.food, self.saturation) = (food, saturation);
            }
            mc_protocol::Event::Experience { progress, level } => {
                (self.xp_progress, self.xp_level) = (progress, level);
            }
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
            mc_protocol::Event::Inventory { state_id, slots } => {
                self.inventory.set(state_id, slots)
            }
            mc_protocol::Event::Carried(stack) => self.inventory.carried = stack,
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

    /// The entities to draw this frame, between the last tick and the next.
    pub fn entities(&self) -> Vec<mc_render::entity::Lit> {
        self.entities.render(self.since_tick / TICK, |p| {
            self.light.at(p.floor().as_ivec3().to_array())
        })
    }

    /// Skins that have arrived since the last call, each with the atlas sprite it now has,
    /// for the renderer to write there. A skin that finds every slot taken isn't drawn.
    pub fn new_skins(&mut self) -> Vec<(String, image::RgbaImage)> {
        let mut out = Vec::new();
        let arrived: Vec<_> = self.skins.arrived().collect();
        for (uuid, img, slim) in arrived {
            let slot = match self.skin_slots.iter().position(|s| *s == Some(uuid)) {
                Some(i) => i,
                None => match self.skin_slots.iter().position(Option::is_none) {
                    Some(i) => i,
                    None => continue,
                },
            };
            self.skin_slots[slot] = Some(uuid);
            let sprite = mc_render::entity::skin_slot(slot);
            self.entities.set_skin(uuid, Some((sprite.clone(), slim)));
            out.push((sprite, img));
        }
        out
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

    fn open_death_screen(&mut self, message: Option<String>) {
        self.death = Some(Death {
            message,
            ticks: 0,
            confirm: false,
        });
    }

    /// `LocalPlayer.respawn`: asks the server to respawn the player.
    fn respawn(&mut self) {
        // `ServerboundClientCommandPacket.Action.PERFORM_RESPAWN`.
        if let Err(e) = self.outbox.send("client_command", &[0]) {
            eprintln!("asking to respawn: {e:#}");
        }
        self.respawning = true;
    }

    fn dead(&self) -> bool {
        self.health.value <= 0.0
    }

    /// Whether the death screen is open, so the mouse is free to click its buttons.
    pub fn screen_open(&self) -> bool {
        self.death.is_some()
    }

    /// A click at `mouse` (GUI pixels) on the death screen, in a GUI `width` × `height`.
    /// `DeathScreen.init`'s and `TitleConfirmScreen`'s buttons.
    pub fn click_screen(&mut self, mouse: [f32; 2], width: i32, height: i32) -> ScreenAction {
        let Some(screen) = self.death_screen() else {
            return ScreenAction::None;
        };
        if !screen.active {
            return ScreenAction::None;
        }
        let buttons = mc_render::hud::death_buttons(&screen, width, height);
        let Some(pressed) = buttons.iter().position(|b| b.contains(mouse)) else {
            return ScreenAction::None;
        };
        match (screen.confirm, pressed) {
            // Respawn (or Spectate World): the button stops working until the server
            // respawns the player.
            (false, 0) => {
                self.respawn();
                if let Some(death) = &mut self.death {
                    death.ticks = 0;
                }
                ScreenAction::None
            }
            // Title Screen: hardcore leaves at once; otherwise ask first.
            (false, _) if self.hardcore => ScreenAction::Quit,
            (false, _) => {
                if let Some(death) = &mut self.death {
                    (death.confirm, death.ticks) = (true, 0);
                }
                ScreenAction::None
            }
            (true, 0) => ScreenAction::Quit,
            (true, _) => {
                self.cancel_quit();
                ScreenAction::None
            }
        }
    }

    pub fn inventory_open(&self) -> bool {
        self.inventory.is_open()
    }

    /// E: `InventoryScreen` (a creative player gets it too, as the creative inventory
    /// isn't built yet).
    pub fn open_inventory(&mut self) {
        if !self.dead() {
            self.inventory.open();
        }
    }

    /// E or Esc, or dying (`AbstractContainerScreen.tick`).
    pub fn close_inventory(&mut self) {
        if let Err(e) = self.inventory.close(&self.outbox) {
            eprintln!("closing the inventory: {e:#}");
        }
    }

    /// A mouse button (0 left, 1 right, 2 middle) pressed or released at `mouse` in a GUI
    /// `width` × `height`.
    pub fn inventory_mouse(
        &mut self,
        button: u8,
        down: bool,
        shift: bool,
        mouse: [f32; 2],
        width: i32,
        height: i32,
    ) {
        use mc_render::hud::inventory::{outside, slot_at};
        let (hovered, outside) = (slot_at(width, height, mouse), outside(width, height, mouse));
        let creative = self.game_mode == 1;
        let result = if down {
            self.inventory
                .mouse_down(&self.outbox, button, shift, hovered, outside, creative)
        } else {
            self.inventory
                .mouse_up(&self.outbox, button, shift, hovered, outside, creative)
        };
        if let Err(e) = result {
            eprintln!("inventory click: {e:#}");
        }
    }

    pub fn inventory_drag(&mut self, mouse: [f32; 2], width: i32, height: i32) {
        self.inventory
            .mouse_moved(mc_render::hud::inventory::slot_at(width, height, mouse));
    }

    pub fn inventory_key(
        &mut self,
        key: crate::inventory::InventoryKey,
        mouse: Option<[f32; 2]>,
        width: i32,
        height: i32,
    ) {
        let hovered = mouse.and_then(|m| mc_render::hud::inventory::slot_at(width, height, m));
        if let Err(e) = self
            .inventory
            .key(&self.outbox, key, hovered, self.game_mode == 1)
        {
            eprintln!("inventory key: {e:#}");
        }
    }

    pub fn inventory_view(&self, mouse: Option<[f32; 2]>) -> mc_render::hud::inventory::Inventory {
        self.inventory.view(mouse)
    }

    /// `ChatScreen.handleChatInput`: a `/` command, or else a chat message.
    pub fn send_chat(&self, message: &str) -> anyhow::Result<()> {
        match message.strip_prefix('/') {
            Some(command) => self.outbox.send_command(command),
            None => self.outbox.send_chat(message),
        }
    }

    /// Leaves the server, closing the connection.
    pub fn leave(&self) {
        self.outbox.close();
    }

    /// Esc: closes the confirmation once its buttons work, choosing Respawn, as
    /// `ConfirmScreen.keyPressed` does; the death screen itself stays open.
    pub fn escape(&mut self) {
        if self.death_screen().is_some_and(|s| s.confirm && s.active) {
            self.cancel_quit();
        }
    }

    /// `TitleConfirmScreen`'s Respawn: respawns, and since the player is still dead until
    /// the server says otherwise, `Gui.setScreen(null)` opens a new death screen without a
    /// message.
    fn cancel_quit(&mut self) {
        self.respawn();
        self.open_death_screen(None);
    }

    fn death_screen(&self) -> Option<mc_render::hud::DeathScreen> {
        let death = self.death.as_ref()?;
        Some(mc_render::hud::DeathScreen {
            hardcore: self.hardcore,
            message: death.message.clone(),
            score: self.score,
            // `DeathScreen.tick` turns them on after 20 ticks, as `ConfirmScreen.setDelay(20)`
            // does; a pressed Respawn stays off.
            active: death.ticks >= 20 && !(self.respawning && !death.confirm),
            confirm: death.confirm,
            mouse: None,
        })
    }

    /// What the hotbar shows, and in survival and adventure (`GameType.isSurvival`) the
    /// hearts, food and experience above it. Call it once a frame: the hearts blink from
    /// the frame the health changes. Max health is always 20, as attributes aren't read yet.
    pub fn hotbar(&mut self) -> mc_render::hud::Hotbar {
        let (health, old_health, blink) = self.health.frame();
        let vitals = matches!(self.game_mode, 0 | 2).then(|| mc_render::hud::Vitals {
            tick: self.health.tick,
            health,
            old_health,
            blink,
            max_health: 20.0,
            hardcore: self.hardcore,
            food: self.food,
            saturation: self.saturation,
            xp_progress: self.xp_progress,
            xp_level: self.xp_level,
        });
        mc_render::hud::Hotbar {
            slots: std::array::from_fn(|i| self.inventory.slots[36 + i].clone()),
            selected: self.selected,
            vitals,
            death: self.death_screen(),
            chat: Default::default(),
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
            // `Hud.tick` and `LivingEntity.baseTick`'s countdown.
            self.health.tick = self.health.tick.wrapping_add(1);
            self.health.damage_cooldown = self.health.damage_cooldown.saturating_sub(1);
            if let Some(death) = &mut self.death {
                death.ticks += 1;
            }
            self.previous = self.player.pos;
            self.entities.tick(&self.collision, &self.blocks);
            if self.dead() && self.inventory.is_open() {
                self.close_inventory();
            }
            if self.dead() {
                // `Minecraft.tick`: a dead player with no screen gets the death screen
                // (`Gui.setScreen(null)`), or respawns at once when it isn't shown.
                if self.death.is_none() && !self.respawning {
                    if self.show_death_screen {
                        self.open_death_screen(None);
                    } else {
                        self.respawn();
                    }
                }
                // A dead player doesn't move or act; the server ignores it until it
                // respawns.
                continue;
            }
            (self.player.yaw, self.player.pitch) = (yaw, pitch);
            if let Err(e) = self.tick_hands(buttons) {
                eprintln!("sending block actions: {e:#}");
            }
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

    /// `Minecraft.startAttack`: a click on a block starts mining it; on nothing (or air), a
    /// miss. Either way the arm swings, which the server shows others when it gets a
    /// `punch`.
    fn start_attack(&mut self, hit: Option<BlockHit>) -> anyhow::Result<()> {
        if self.miss_time > 0 {
            return Ok(());
        }
        match hit {
            Some(h) if !self.is_air(h.pos) => self.start_destroy(h.pos, h.face)?,
            _ => {
                if !self.instabuild {
                    self.miss_time = 10;
                }
            }
        }
        self.outbox.send("punch", &[])
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
            // Each tick of mining swings the arm again.
            Some(h) if held => {
                if !self.is_air(h.pos) {
                    self.continue_destroy(h.pos, h.face)?;
                    self.outbox.send("punch", &[])?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Light is read at the block's own nibble, in chunks at negative coordinates too; a
    /// section without sky light takes the bottom of the nearest one above that has it, and
    /// above every section it's full.
    #[test]
    fn entity_light_reads_like_world_view() {
        let mut light = Light::default();
        // In chunk (-1, 0), section 0: sky 9 and block 4 at x 3, y 5, z 7 (world -13, 5, 7).
        let mut sky = Box::new([0u8; 2048]);
        let mut block = Box::new([0u8; 2048]);
        let index = (5 * 16 + 7) * 16 + 3;
        sky[index / 2] |= 9 << (index % 2 * 4);
        block[index / 2] |= 4 << (index % 2 * 4);
        // Section 2's bottom layer has sky light 12 at x 3, z 7.
        let mut upper = Box::new([0u8; 2048]);
        let index = 7 * 16 + 3;
        upper[index / 2] |= 12 << (index % 2 * 4);
        light.set(
            (-1, 0),
            vec![
                (0, [Some(sky), Some(block)]),
                (1, [None, None]),
                (2, [Some(upper), None]),
            ],
            true,
        );
        assert_eq!(light.at([-13, 5, 7]), [9, 4]);
        assert_eq!(light.at([-12, 5, 7]), [0, 0]);
        assert_eq!(light.at([-13, 20, 7]), [12, 0]);
        assert_eq!(light.at([-13, 60, 7]), [15, 0]);
        // An update of one layer keeps the other.
        let mut lit = Box::new([0u8; 2048]);
        lit[(5 * 16 + 7) * 16 / 2 + 1] = 0x70;
        light.set((-1, 0), vec![(0, [None, Some(lit)])], false);
        assert_eq!(light.at([-13, 5, 7]), [9, 7]);
        assert_eq!(light.at([3, 5, 7]), [15, 0], "an unloaded chunk");
    }
}
