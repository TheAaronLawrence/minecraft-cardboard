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

/// Each `Direction`'s step: down, up, north, south, west, east.
const FACE_OFFSETS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];

/// A damage type's `effects` (such as `burning`), from its data file.
fn damage_effects(kind: &str) -> Option<String> {
    let res = mc_assets::Resources::new(mc_assets::default_root());
    let path = res.data_path("damage_type", kind, "json");
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    json["effects"].as_str().map(str::to_owned)
}

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

/// Where rain and snow fall, for `weather::columns`.
impl mc_render::weather::Weather for Walker {
    /// `Heightmap.Types.MOTION_BLOCKING`: above the highest block that blocks motion or
    /// holds a fluid; a chunk that hasn't arrived has nothing in it.
    fn height(&self, x: i32, z: i32) -> i32 {
        let Some(sections) = self
            .collision
            .chunks
            .get(&(x.div_euclid(16), z.div_euclid(16)))
        else {
            return i32::MIN;
        };
        let mut sorted: Vec<_> = sections.iter().collect();
        sorted.sort_unstable_by_key(|(y, _)| std::cmp::Reverse(*y));
        let (lx, lz) = (x.rem_euclid(16), z.rem_euclid(16));
        for (section_y, blocks) in sorted {
            for ly in (0..16).rev() {
                let state = match blocks {
                    Blocks::Uniform(id) => *id,
                    Blocks::Mixed(ids) => ids[((ly * 16 + lz) * 16 + lx) as usize],
                };
                if self.blocks.motion_blocking(state) {
                    return section_y * 16 + ly + 1;
                }
                if matches!(blocks, Blocks::Uniform(_)) {
                    break;
                }
            }
        }
        self.sky.environment.as_ref().map_or(0, |e| e.min_y)
    }

    /// `ClientLevel.getPrecipitationAt`: none where the chunk hasn't arrived, otherwise as
    /// the biome there falls.
    fn precipitation(&self, [x, y, z]: [i32; 3]) -> mc_render::tint::Precipitation {
        if !self
            .collision
            .chunks
            .contains_key(&(x.div_euclid(16), z.div_euclid(16)))
        {
            return mc_render::tint::Precipitation::None;
        }
        let biome = self.biome_at_quart([x >> 2, y >> 2, z >> 2]);
        self.biome_data
            .precipitation(biome, [x, y, z], self.sky.sea_level)
    }

    fn light(&self, pos: [i32; 3]) -> [u8; 2] {
        let [sky, block] = self.light.at(pos);
        [
            sky,
            block.max(self.blocks.light_emission(mc_physics::World::state(
                &self.collision,
                pos[0],
                pos[1],
                pos[2],
            ))),
        ]
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
const RELEASE_USE_ITEM: i32 = 6;

pub struct Walker {
    /// The Attack Indicator option.
    pub attack_indicator_mode: mc_render::hud::options::AttackIndicator,
    player: Player,
    /// Feet at the previous tick, to draw between ticks.
    previous: DVec3,
    /// `Camera.eyeHeightOld` and `eyeHeight`: the eye height the view eases towards the
    /// pose's, half the way each tick.
    camera_eye: [f32; 2],
    placed: bool,
    since_tick: f32,
    collision: Collision,
    light: Light,
    /// Each chunk's biomes by section, for the water fog, and the biomes' attributes.
    biomes: HashMap<(i32, i32), Vec<crate::stream::SectionBiomes>>,
    biome_data: std::sync::Arc<mc_render::tint::Biomes>,
    /// The mob effects the player has, such as `minecraft:fire_resistance`.
    effects: std::collections::HashSet<String>,
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
    /// The living entity looked at instead, from the last frame: its id, and whether it's
    /// alive (`Minecraft.crosshairPickEntity`).
    hit_entity: Option<(i32, bool)>,
    /// The block being mined, its face, and how much of it is broken (0 to 1).
    destroying: Option<(IVec3, u8, f32)>,
    destroy_delay: u32,
    right_click_delay: u32,
    /// The item in use (`LivingEntity.useItem` and `useItemRemaining`, and
    /// `LocalPlayer.usingItemHand`): eating, drinking, drawing a bow or blocking. It starts
    /// when this client predicts `Item.use` starts it (`startedUsingItem`), or when the
    /// server's living flags (data 8) say so.
    using: Option<Use>,
    /// Each item's components that using it depends on, by name.
    item_uses: HashMap<String, mc_registry::ItemUse>,
    /// What using each item looks like and how long it lasts, by name.
    use_animations: HashMap<String, (mc_render::entity::UseAnim, i32)>,
    /// Item icons, whose sprites food's crumbs are cut from.
    item_icons: std::sync::Arc<HashMap<String, mc_render::item::ItemIcon>>,
    /// `Camera.oldFovModifier` and `fovModifier`.
    fov_modifier: [f32; 2],
    miss_time: u32,
    /// Numbers each action the server acknowledges.
    sequence: i32,
    /// The inventory, whose last row is the hotbar.
    inventory: crate::inventory::Inventory,
    /// Other players mining blocks, by entity id: the block and crack stage.
    others_mining: HashMap<i32, (IVec3, u8)>,
    /// Chests' lids that have opened, by where the chest is.
    chest_lids: HashMap<IVec3, ChestLid>,
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
    /// The player's own UUID, for its skin.
    uuid: u128,
    /// The player's hands as they see them.
    hands: crate::hands::Hands,
    /// The view's walking bob and hurt tilt.
    view_bob: crate::hands::ViewBob,
    /// The server says the player is on fire.
    on_fire: bool,
    /// The player's air supply in ticks (data 1), which the server counts down under water.
    air: i32,
    /// Items that place a block (`BlockItem`s, named as their block is), and the block's id.
    block_items: HashMap<String, u32>,
    /// Each state's block id.
    state_blocks: Vec<u32>,
    /// Sounds the game plays, waiting for `take_sounds`.
    sounds: Vec<crate::sound::Instance>,
    /// `ClientLevel.random` for the sounds the client picks itself.
    sound_random: mc_render::bake::JavaRandom,
    /// `ClientLevel.rainSoundTime`.
    rain_sound_time: i32,
    /// A block being placed that the server hasn't sent back yet: where it may appear, its
    /// block id, and ticks to wait for it. Its sound plays once it does, as vanilla plays it
    /// once its client predicts the placement.
    placing: Option<([IVec3; 2], u32, u32)>,
    /// The dimension's `background_music`: default, creative and underwater.
    music: [Option<crate::sound::Track>; 3],
    /// The time of day and what the sky looks like.
    sky: Sky,
}

/// The world's time and the sky it gives, as `ClientLevel` keeps its game time and
/// `ClientClockManager` its world clocks, ticked with the world, and as the camera's
/// `EnvironmentAttributeProbe` samples the environment each tick to blend between frames.
struct Sky {
    /// The dimension's environment, from the login; none until then.
    environment: Option<mc_render::environment::Environment>,
    /// A superflat world, whose horizon is its floor.
    flat: bool,
    game_time: i64,
    /// Each world clock's total ticks, part of a tick, and rate.
    clocks: HashMap<String, (i64, f32, f32)>,
    /// The attributes at the last tick and this one.
    visuals: Option<[mc_render::environment::Visuals; 2]>,
    /// `LightmapRenderStateExtractor.blockLightFlicker`.
    flicker: f32,
    random: mc_render::bake::JavaRandom,
    /// `Level.rainLevel` and `thunderLevel`, which the server sets.
    rain: f32,
    thunder: f32,
    /// `ClientLevel.skyFlashTime`: ticks left of a lightning bolt's flash.
    flash: u32,
    /// `AtmosphericFogEnvironment.rainFogMultiplier` at the last tick and this one.
    rain_fog: [f32; 2],
    /// The level's sea level, where snow starts to fall higher up.
    sea_level: i32,
    /// `Hud.vignetteBrightness`: how dark the vignette is, easing toward the darkness at
    /// the eye.
    vignette: f32,
}

impl Sky {
    /// `Level.getThunderLevel`: thunder only rumbles as hard as it rains.
    fn thunder_level(&self) -> f32 {
        self.thunder * self.rain
    }
}

impl Sky {
    /// `ClientClockManager.tick`: each clock runs at its rate.
    fn tick_clocks(&mut self, game_time: i64) {
        let delta = game_time - self.game_time;
        self.game_time = game_time;
        for (total, partial, rate) in self.clocks.values_mut() {
            let ticks = *partial as f64 + delta as f64 * *rate as f64;
            let full = ticks.floor();
            *partial = (ticks - full) as f32;
            *total += full as i64;
        }
    }
}

/// `ChestLidController`: a chest's lid, which opens or shuts a tenth of the way a tick,
/// and the chest's block, whose lid it is.
struct ChestLid {
    block: u32,
    should_be_open: bool,
    openness: f32,
    o_openness: f32,
    /// Ticks since it shut, after which it's forgotten: by then its chunk has the lid
    /// again (`stream::LID_SHUT`).
    shut_for: u32,
}

impl ChestLid {
    /// `ChestLidController.tickLid`.
    fn tick(&mut self) {
        self.o_openness = self.openness;
        self.openness = if self.should_be_open {
            (self.openness + 0.1).min(1.0)
        } else {
            (self.openness - 0.1).max(0.0)
        };
        self.shut_for = if self.openness == 0.0 && !self.should_be_open {
            self.shut_for + 1
        } else {
            0
        };
    }
}

/// An item in use: its hand (0 main, 1 off), the item, and the ticks left.
struct Use {
    hand: u8,
    item: String,
    remaining: i32,
}

/// The open death screen: its message, ticks since it (or its confirmation) opened,
/// whether it's asking to confirm leaving, and which button has the focus.
struct Death {
    message: Option<String>,
    ticks: u32,
    confirm: bool,
    focused: Option<usize>,
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
    /// `LocalPlayer.hurtTo`; returns whether it hurt the player.
    fn hurt_to(&mut self, health: f32) -> bool {
        let damage = self.value - health;
        let hurt = self.flash && damage > 0.0;
        if self.flash {
            if damage < 0.0 {
                self.damage_cooldown = 10;
            } else if hurt {
                self.damage_cooldown = 20;
            }
        }
        self.flash = true;
        self.value = health;
        hurt
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
    /// A player with this UUID, who has joined with `outbox`.
    pub fn new(
        outbox: mc_protocol::Outbox,
        registry: &mc_registry::Registry,
        biomes: std::sync::Arc<mc_render::tint::Biomes>,
        uuid: u128,
        vignette: f32,
        item_icons: std::sync::Arc<HashMap<String, mc_render::item::ItemIcon>>,
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
            camera_eye: [1.62; 2],
            placed: false,
            since_tick: 0.0,
            collision: Collision {
                chunks: HashMap::new(),
                air,
            },
            light: Light::default(),
            biomes: HashMap::new(),
            biome_data: biomes,
            effects: Default::default(),
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
            using: None,
            item_uses: (0..registry.item_count())
                .filter_map(|id| {
                    let item = registry.item_name(id)?;
                    Some((item.to_owned(), registry.item_info(item)?.using.clone()))
                })
                .collect(),
            use_animations: crate::hands::use_animations(registry),
            item_icons,
            fov_modifier: [1.0; 2],
            miss_time: 0,
            hit_entity: None,
            sequence: 0,
            inventory: crate::inventory::Inventory::new(registry),
            others_mining: HashMap::new(),
            chest_lids: HashMap::new(),
            entities: crate::entities::Entities::new(registry),
            attack_indicator_mode: Default::default(),
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
            uuid,
            sky: Sky {
                environment: None,
                flat: false,
                game_time: 0,
                clocks: HashMap::new(),
                visuals: None,
                flicker: 0.0,
                random: mc_render::bake::JavaRandom::new(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_nanos() as i64),
                ),
                rain: 0.0,
                thunder: 0.0,
                flash: 0,
                rain_fog: [0.0; 2],
                sea_level: 63,
                vignette,
            },
            hands: Default::default(),
            view_bob: Default::default(),
            on_fire: false,
            air: 300,
            block_items: (0..registry.item_count())
                .filter_map(|id| registry.item_name(id))
                .filter(|name| {
                    registry
                        .item_info(name)
                        .is_some_and(|i| i.name.starts_with("block."))
                })
                .filter_map(|name| Some((name.to_owned(), registry.block(name)?.id)))
                .collect(),
            state_blocks: (0..registry.state_count())
                .map(|s| registry.block_of_state(s).map_or(0, |b| b.id))
                .collect(),
            sounds: Vec::new(),
            sound_random: mc_render::bake::JavaRandom::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(1, |d| d.as_nanos() as i64 ^ 0x2545_F491),
            ),
            rain_sound_time: 0,
            placing: None,
            music: Default::default(),
        })
    }

    pub fn chunk(&mut self, key: (i32, i32), data: Option<crate::stream::ChunkData>) {
        match data {
            Some((blocks, biomes)) => {
                self.collision.chunks.insert(key, blocks);
                self.biomes.insert(key, biomes);
            }
            None => {
                self.collision.chunks.remove(&key);
                self.light.chunks.remove(&key);
                self.biomes.remove(&key);
            }
        }
    }

    /// The biome at a quart position (a 4×4×4 cell), as `ChunkAccess.getNoiseBiome` gives
    /// it, with the height clamped into the chunk's sections; where nothing has arrived,
    /// plains (`ClientLevel.getUncachedNoiseBiome`).
    fn biome_at_quart(&self, [x, y, z]: [i32; 3]) -> &str {
        let found = self
            .biomes
            .get(&(x.div_euclid(4), z.div_euclid(4)))
            .and_then(|sections| {
                // The sections with biomes, leaving out the light's padding above and below.
                let with = || sections.iter().filter(|s| !s.1.is_empty());
                let (low, high) = (with().map(|s| s.0).min()?, with().map(|s| s.0).max()?);
                let y = y.clamp(low * 4, high * 4 + 3);
                with().find(|s| s.0 == y.div_euclid(4)).map(|s| (y, s))
            })
            .and_then(|(y, (_, palette, cells))| {
                let (lx, ly, lz) = (x.rem_euclid(4), y.rem_euclid(4), z.rem_euclid(4));
                let index = cells[((ly * 4 + lz) * 4 + lx) as usize];
                palette.get(index as usize)
            });
        found.map_or("minecraft:plains", String::as_str)
    }

    /// The water fog's colour and end distance at `eye`: `EnvironmentAttributeProbe.tick`'s
    /// Gaussian sample of the biomes around it, blended as `SpatialAttributeInterpolator`
    /// does, each biome applied to the defaults.
    fn water_fog(&self, eye: DVec3, color: [f32; 3], end: f32) -> ([f32; 3], f32) {
        let mut result: Option<([f32; 3], f32)> = None;
        let mut accumulated = 0.0;
        for (biome, w) in self.biome_weights(eye) {
            let (c, e) = self.biome_data.water_fog(biome, color, end);
            accumulated += w;
            result = Some(match result {
                None => (c, e),
                Some((rc, re)) => {
                    let t = (w / accumulated) as f32;
                    (
                        std::array::from_fn(|i| rc[i] + (c[i] - rc[i]) * t),
                        re + (e - re) * t,
                    )
                }
            });
        }
        result.unwrap_or((color, end))
    }

    /// `GaussianSampler.sample` of the biomes around `eye`, at quart resolution: each
    /// biome's weight, in the order they're first met.
    fn biome_weights(&self, eye: DVec3) -> Vec<(&str, f64)> {
        const KERNEL: [f64; 7] = [0.0, 1.0, 4.0, 6.0, 4.0, 1.0, 0.0];
        let p = eye * 0.25 - DVec3::splat(0.5);
        let base = p.floor();
        let rel = p - base;
        let base = base.as_ivec3();
        let weight = |i: usize, r: f64| KERNEL[i + 1] + (KERNEL[i] - KERNEL[i + 1]) * r;
        // Weights summed per biome, in the order biomes are first met.
        let mut weights: Vec<(&str, f64)> = Vec::new();
        for z in 0..6 {
            for x in 0..6 {
                for y in 0..6 {
                    let w = weight(x, rel.x) * weight(y, rel.y) * weight(z, rel.z);
                    let biome = self.biome_at_quart([
                        base.x - 2 + x as i32,
                        base.y - 2 + y as i32,
                        base.z - 2 + z as i32,
                    ]);
                    match weights.iter_mut().find(|(b, _)| *b == biome) {
                        Some((_, total)) => *total += w,
                        None => weights.push((biome, w)),
                    }
                }
            }
        }
        weights
    }

    /// `EnvironmentAttributeProbe.tick`: the attributes at the camera now, for the world
    /// clocks' times.
    fn sample_sky(&self) -> Option<mc_render::environment::Visuals> {
        let environment = self.sky.environment.as_ref()?;
        let eye = self.player.pos + DVec3::Y * self.camera_eye[1] as f64;
        let weights = self.biome_weights(eye);
        let clocks = &self.sky.clocks;
        let visuals = environment
            .sample(&weights, |clock| clocks.get(clock).map_or(0, |c| c.0))
            .weather(self.sky.rain, self.sky.thunder_level());
        Some(if self.sky.flash > 0 {
            visuals.flash()
        } else {
            visuals
        })
    }

    /// `Hud.vignetteBrightness` now, for the next world to start from.
    pub fn vignette(&self) -> f32 {
        self.sky.vignette
    }

    /// The sky this frame, with the eye at `eye`; none before the server says which
    /// dimension the player is in.
    pub fn sky(&self, eye: Vec3) -> Option<mc_render::sky::SkyState> {
        let environment = self.sky.environment.as_ref()?;
        let [old, new] = self.sky.visuals?;
        let partial = self.since_tick / TICK;
        // `ClientLevelData.getHorizonHeight`.
        let horizon = if self.sky.flat {
            environment.min_y as f32
        } else {
            63.0
        };
        Some(mc_render::sky::SkyState {
            visuals: old.lerp(&new, partial),
            skybox: environment.skybox,
            dark_disc: eye.y < horizon,
            // `ClientLevelData.voidDarknessOnsetRange`: over 32 blocks above the floor, or
            // 1 in a superflat world.
            void_darkness: {
                let range = if self.sky.flat { 1.0 } else { 32.0 };
                ((range + environment.min_y as f32 - eye.y) / range).clamp(0.0, 1.0)
            },
            block_factor: self.sky.flicker + 1.4,
            game_time: self.sky.game_time,
            partial_tick: partial,
            rain: self.sky.rain,
            thunder: self.sky.thunder_level(),
            vignette: self.sky.vignette.clamp(0.0, 1.0),
            rain_fog: {
                let [old, new] = self.sky.rain_fog;
                old + (new - old) * partial
            },
        })
    }

    /// The rain and snow around the eye this frame (`WeatherEffectRenderer`).
    pub fn weather(&self, eye: Vec3) -> mc_render::weather::Columns {
        let partial = self.since_tick / TICK;
        mc_render::weather::columns(
            self,
            eye.as_dvec3(),
            self.sky.game_time,
            partial,
            self.sky.rain,
            mc_render::weather::RADIUS,
        )
    }

    /// The server's light for these sections of a chunk; `whole` replaces all it had.
    pub fn light(&mut self, key: (i32, i32), sections: Vec<(i32, SectionLight)>, whole: bool) {
        self.light.set(key, sections, whole);
    }

    /// The server changed these blocks.
    pub fn blocks(&mut self, changes: &[([i32; 3], u32)]) {
        for &(pos, state) in changes {
            self.collision.set(pos, state);
            // A chest broken or replaced takes its lid with it.
            let block = self.state_blocks.get(state as usize).copied();
            self.chest_lids
                .retain(|p, lid| *p != IVec3::from(pos) || Some(lid.block) == block);
            // `BlockItem.place`: the block's place sound.
            if let Some((at, block, _)) = self.placing
                && at.contains(&IVec3::from(pos))
                && self.state_blocks.get(state as usize) == Some(&block)
            {
                self.placing = None;
                let t = self.blocks.sound_type(state);
                let (event, volume, pitch) =
                    (t.place.clone(), (t.volume + 1.0) / 2.0, t.pitch * 0.8);
                self.block_sound(&event, volume, pitch, pos);
            }
        }
    }

    /// `LevelEventHandler.levelEvent`'s sounds: a block broken (2001), hit while someone
    /// mines it (2020), and the simple ones `sound::level_event` lists.
    fn level_event(&mut self, kind: i32, pos: [i32; 3], data: i32) {
        match kind {
            2001 => self.break_sound(pos, data as u32),
            2020 => self.hit_sound(IVec3::from(pos)),
            _ => {
                if let Some((event, category, volume, pitch)) =
                    crate::sound::level_event(kind, data, &mut self.sound_random)
                {
                    let seed = self.sound_seed();
                    self.sounds.push(crate::sound::Instance::at_block(
                        event, category, volume, pitch, pos, seed,
                    ));
                }
            }
        }
    }

    /// The sounds to play since the last call.
    pub fn take_sounds(&mut self) -> Vec<crate::sound::Instance> {
        std::mem::take(&mut self.sounds)
    }

    /// Where a sound bound to an entity is: the player's feet, or another entity's.
    pub fn entity_position(&self, id: i32) -> Option<DVec3> {
        if id == self.entity_id {
            Some(self.player.pos)
        } else {
            self.entities.position(id)
        }
    }

    /// `Minecraft.getSituationalMusic` in a world: the dimension's music, as
    /// `BackgroundMusic.select` picks it for a creative player or one under water.
    pub fn music(&self) -> Option<crate::sound::Track> {
        let [default, creative, underwater] = &self.music;
        if self.player.under_water && underwater.is_some() {
            return underwater.clone();
        }
        let creative_player = self.instabuild && self.player.may_fly;
        if creative_player && creative.is_some() {
            creative.clone()
        } else {
            default.clone()
        }
    }

    fn sound_seed(&mut self) -> i64 {
        self.sound_random.next_long()
    }

    /// `ClientLevel.playLocalSound` at a block's centre.
    fn block_sound(&mut self, event: &str, volume: f32, pitch: f32, pos: [i32; 3]) {
        let seed = self.sound_seed();
        self.sounds.push(crate::sound::Instance::at_block(
            event,
            crate::sound::Category::Blocks,
            volume,
            pitch,
            pos,
            seed,
        ));
    }

    /// `LocalPlayer.playSound`: where the player is, as a player's sound.
    fn player_sound(&mut self, event: &str, volume: f32, pitch: f32) {
        let seed = self.sound_seed();
        self.sounds.push(crate::sound::Instance::at(
            event,
            crate::sound::Category::Players,
            volume,
            pitch,
            self.player.pos,
            seed,
        ));
    }

    /// `(random.nextFloat() - random.nextFloat()) * scale + 1`, as many sounds' pitches go.
    fn wobble(&mut self, scale: f32) -> f32 {
        let r = &mut self.sound_random;
        (r.next_float() - r.next_float()) * scale + 1.0
    }

    /// The sounds the player's movement made this tick.
    fn movement_sounds(&mut self) {
        for sound in std::mem::take(&mut self.player.sounds) {
            match sound {
                mc_physics::MoveSound::Play {
                    event,
                    volume,
                    pitch,
                } => self.player_sound(&event, volume, pitch),
                mc_physics::MoveSound::Swim(volume) => {
                    let pitch = self.wobble(0.4);
                    self.player_sound("minecraft:entity.player.swim", volume, pitch);
                }
                mc_physics::MoveSound::Splash { volume, fast } => {
                    let pitch = self.wobble(0.4);
                    let event = if fast {
                        "minecraft:entity.player.splash.high_speed"
                    } else {
                        "minecraft:entity.player.splash"
                    };
                    self.player_sound(event, volume, pitch);
                }
            }
        }
    }

    /// `ClientLevel.addBreakingBlockEffects`'s sound: a hit on the block being mined.
    fn hit_sound(&mut self, pos: IVec3) {
        let state = mc_physics::World::state(&self.collision, pos.x, pos.y, pos.z);
        if self.blocks.is_air(state) {
            return;
        }
        let t = self.blocks.sound_type(state);
        let (event, volume, pitch) = (t.hit.clone(), (t.volume + 1.0) / 8.0, t.pitch * 0.5);
        self.block_sound(&event, volume, pitch, pos.to_array());
    }

    /// `LevelEventHandler`'s 2001 as the player's own `Block.playerWillDestroy` plays it: the
    /// block breaking.
    fn break_sound(&mut self, pos: [i32; 3], state: u32) {
        if self.blocks.is_air(state) {
            return;
        }
        let t = self.blocks.sound_type(state);
        let (event, volume, pitch) = (t.break_sound.clone(), (t.volume + 1.0) / 2.0, t.pitch * 0.8);
        self.block_sound(&event, volume, pitch, pos);
    }

    /// `ClientLevel.tickWeatherEffects`'s sound: rain on the ground near the camera, now and
    /// then, quieter and lower when it falls on a roof above.
    fn tick_rain_sound(&mut self) {
        let rain = self.sky.rain;
        if rain <= 0.0 {
            return;
        }
        let Some(min_y) = self.sky.environment.as_ref().map(|e| e.min_y) else {
            return;
        };
        use mc_render::weather::Weather;
        let mut random =
            mc_render::bake::JavaRandom::new(self.sky.game_time.wrapping_mul(312_987_231));
        let eye = self.player.pos + DVec3::Y * f64::from(self.camera_eye[1]);
        let camera = eye.floor().as_ivec3();
        let radius = mc_render::weather::RADIUS;
        let diameter = 2 * radius + 1;
        let count = (0.225 * (diameter * diameter) as f32 * rain * rain) as i32;
        let mut at = None;
        for _ in 0..count {
            let x = camera.x + random.next_int(diameter) - radius;
            let z = camera.z + random.next_int(diameter) - radius;
            let y = self.height(x, z);
            if y > min_y
                && y <= camera.y + 10
                && y >= camera.y - 10
                && self.precipitation([x, y, z]) == mc_render::tint::Precipitation::Rain
            {
                at = Some([x, y - 1, z]);
                // The splash's place in the block, which isn't drawn.
                random.next_double();
                random.next_double();
            }
        }
        let Some(at) = at else { return };
        if random.next_int(3) < self.rain_sound_time {
            self.rain_sound_time = 0;
            let roofed = self.height(camera.x, camera.z) > camera.y;
            if at[1] > camera.y + 1 && roofed {
                self.weather_sound("minecraft:weather.rain.above", 0.1, 0.5, at);
            } else {
                self.weather_sound("minecraft:weather.rain", 0.2, 1.0, at);
            }
        } else {
            self.rain_sound_time += 1;
        }
    }

    fn weather_sound(&mut self, event: &str, volume: f32, pitch: f32, pos: [i32; 3]) {
        let seed = self.sound_seed();
        self.sounds.push(crate::sound::Instance::at_block(
            event,
            crate::sound::Category::Weather,
            volume,
            pitch,
            pos,
            seed,
        ));
    }

    pub fn server_event(&mut self, event: mc_protocol::Event) {
        // `handleSetEntityMotion`'s `lerpMotion` on the player: the server's knockback.
        if let mc_protocol::Event::EntityMotion { id, velocity } = event
            && id == self.entity_id
        {
            self.player.velocity = DVec3::from(velocity);
            return;
        }
        // The player's own shared flags (data 0), where bit 0 is `isOnFire`; air supply
        // (data 1); and living flags (data 8), where bit 0 is `isUsingItem`.
        if let mc_protocol::Event::EntityData { id, values } = &event
            && *id == self.entity_id
        {
            for (index, value) in values {
                match (index, value) {
                    (0, mc_protocol::DataValue::Byte(flags)) => self.on_fire = flags & 1 != 0,
                    (1, mc_protocol::DataValue::Int(air)) => self.air = *air,
                    // `LocalPlayer.onSyncedDataUpdated`: the server starts or stops it.
                    (8, mc_protocol::DataValue::Byte(flags)) => {
                        if flags & 1 == 0 {
                            self.using = None;
                        } else if self.using.is_none() {
                            self.start_using(u8::from(flags & 2 != 0));
                        }
                    }
                    _ => {}
                }
            }
            return;
        }
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
                dimension,
                flat,
                sea_level,
            } => {
                (self.entity_id, self.hardcore, self.game_mode) = (entity_id, hardcore, game_mode);
                self.show_death_screen = show_death_screen;
                let res = mc_assets::Resources::new(mc_assets::default_root());
                self.sky.environment =
                    match mc_render::environment::Environment::load(&res, &dimension) {
                        Ok(environment) => Some(environment),
                        Err(e) => {
                            eprintln!("loading {dimension}'s environment: {e:#}");
                            None
                        }
                    };
                (self.sky.flat, self.sky.visuals) = (flat, None);
                self.music = crate::sound::background_music(&res, &dimension);
                self.sky.sea_level = sea_level;
                // A new `ClientLevel`, clear until the server says otherwise.
                (self.sky.rain, self.sky.thunder, self.sky.flash) = (0.0, 0.0, 0);
            }
            // `ClientPacketListener.handleGameEvent`'s weather, which `Level.setRainLevel`
            // and `setThunderLevel` clamp.
            mc_protocol::Event::RainLevel(level) => self.sky.rain = level.clamp(0.0, 1.0),
            mc_protocol::Event::ThunderLevel(level) => self.sky.thunder = level.clamp(0.0, 1.0),
            // `ClientPacketListener.handleSetTime`.
            mc_protocol::Event::Time { game_time, clocks } => {
                self.sky.tick_clocks(game_time);
                for (name, clock) in clocks {
                    let state = (clock.total_ticks, clock.partial_tick, clock.rate);
                    self.sky.clocks.insert(name, state);
                }
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
                self.view_bob.reset();
                self.on_fire = false;
                self.air = 300;
                self.effects.clear();
                self.using = None;
            }
            mc_protocol::Event::GameMode(mode) => self.game_mode = mode,
            mc_protocol::Event::Effect { effect, active } => {
                if active {
                    self.effects.insert(effect);
                } else {
                    self.effects.remove(&effect);
                }
            }
            // `LivingEntity.handleDamageEvent`: the player's own hurt sound, as the damage
            // type's `effects` say (`DamageEffects`).
            mc_protocol::Event::Damaged { id, kind } if id == self.entity_id => {
                self.view_bob.hurt();
                let event = match damage_effects(&kind).as_deref() {
                    Some("drowning") => "minecraft:entity.player.hurt_drown",
                    Some("burning") => "minecraft:entity.player.hurt_on_fire",
                    Some("poking") => "minecraft:entity.player.hurt_sweet_berry_bush",
                    Some("freezing") => "minecraft:entity.player.hurt_freeze",
                    _ => "minecraft:entity.player.hurt",
                };
                let pitch = self.wobble(0.2);
                self.player_sound(event, 1.0, pitch);
            }
            // `Player.handleEntityEvent(9)`: the server finished the item's use.
            mc_protocol::Event::EntityEvent { id, event: 9 } if id == self.entity_id => {
                self.complete_using();
            }
            // `LivingEntity.handleEntityEvent(3)`: the player's own death sound.
            mc_protocol::Event::EntityEvent { id, event: 3 } if id == self.entity_id => {
                let pitch = self.wobble(0.2);
                self.player_sound("minecraft:entity.player.death", 1.0, pitch);
            }
            // `ClientPacketListener.handleSoundEvent` and `handleSoundEntityEvent`.
            mc_protocol::Event::Sound {
                event,
                category,
                pos,
                entity,
                volume,
                pitch,
                seed,
            } => {
                let category = crate::sound::Category::from_id(category);
                let instance = match entity {
                    Some(id) => match self.entity_position(id) {
                        Some(at) => crate::sound::Instance::following(
                            &event,
                            category,
                            volume,
                            pitch,
                            (id, at),
                            seed,
                        ),
                        None => return,
                    },
                    None => crate::sound::Instance::at(
                        &event,
                        category,
                        volume,
                        pitch,
                        DVec3::from(pos),
                        seed,
                    ),
                };
                self.sounds.push(instance);
            }
            mc_protocol::Event::LevelEvent {
                kind,
                pos,
                data,
                global: false,
            } => self.level_event(kind, pos, data),
            // `Entity.pushFromExplosion`.
            mc_protocol::Event::ExplosionPush(push) => self.player.velocity += DVec3::from(push),
            mc_protocol::Event::HurtAnimation { id, yaw } if id == self.entity_id => {
                self.view_bob.hurt_from(yaw)
            }
            // `ClientPacketListener.handleSetHealth`.
            mc_protocol::Event::Health {
                health,
                food,
                saturation,
            } => {
                if self.health.hurt_to(health) {
                    self.view_bob.hurt();
                }
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
            mc_protocol::Event::Inventory {
                container,
                state_id,
                slots,
            } => self.inventory.set(container, state_id, slots),
            mc_protocol::Event::OpenScreen {
                container,
                menu,
                title,
            } => {
                use mc_render::hud::inventory::Menu;
                let menu = match menu {
                    "minecraft:crafting" => Some(Menu::Crafting),
                    "minecraft:furnace" => Some(Menu::Furnace),
                    name => name
                        .strip_prefix("minecraft:generic_9x")
                        .and_then(|rows| rows.parse().ok())
                        .map(Menu::Chest),
                };
                match menu {
                    Some(menu) if !self.dead() => {
                        self.inventory.open_container(container, menu, title)
                    }
                    // A screen this game can't show yet: closed again at once.
                    _ => {
                        if let Err(e) = crate::inventory::Inventory::refuse(&self.outbox, container)
                        {
                            eprintln!("closing a screen: {e:#}");
                        }
                    }
                }
            }
            mc_protocol::Event::ContainerData {
                container,
                id,
                value,
            } => self.inventory.set_data(container, id, value),
            // `ChestBlockEntity.triggerEvent(1, viewers)`.
            mc_protocol::Event::BlockEvent {
                pos,
                kind: 1,
                value,
                block,
            } if block.ends_with("chest") => {
                let state = mc_physics::World::state(&self.collision, pos[0], pos[1], pos[2]);
                let block = self.state_blocks.get(state as usize).copied().unwrap_or(0);
                let lid = self.chest_lids.entry(IVec3::from(pos)).or_insert(ChestLid {
                    block,
                    should_be_open: false,
                    openness: 0.0,
                    o_openness: 0.0,
                    shut_for: 0,
                });
                (lid.block, lid.should_be_open) = (block, value > 0);
            }
            mc_protocol::Event::ContainerClosed(container) => {
                self.inventory.server_close(container)
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

    /// The chests whose lids are open or moving: where each is, its state, how far open
    /// to draw it this frame (`ChestRenderer.submit` eases it, `1 - (1 - open)³`), and the
    /// light there.
    pub fn chest_lids(&self) -> Vec<([i32; 3], u32, f32, [u8; 2])> {
        let partial = self.since_tick / TICK;
        self.chest_lids
            .iter()
            .map(|(pos, lid)| {
                let [x, y, z] = pos.to_array();
                let state = mc_physics::World::state(&self.collision, x, y, z);
                let open = lid.o_openness + (lid.openness - lid.o_openness) * partial;
                let eased = 1.0 - (1.0 - open).powi(3);
                (pos.to_array(), state, eased, self.light.at(pos.to_array()))
            })
            .collect()
    }

    /// The lightning bolts to draw this frame.
    pub fn lightning(&self) -> Vec<mc_render::mesh::Vertex> {
        self.entities.lightning()
    }

    /// `Options.broadcastOptions`, once the options have changed: the view distance.
    pub fn send_view_distance(&self, chunks: i32) {
        // One chunk more than drawn, as on joining.
        let distance = (chunks + 1).clamp(2, 32) as u8;
        if let Err(e) = self.outbox.send_client_information(distance) {
            eprintln!("sending the view distance: {e:#}");
        }
    }

    /// The Particles option.
    pub fn set_particle_status(&mut self, status: mc_render::hud::options::Particles) {
        self.entities.set_particle_status(status);
    }

    /// The particles to draw this frame, between the last tick and the next.
    pub fn particles(&self) -> Vec<mc_render::particle::Particle> {
        self.entities.particles(self.since_tick / TICK, |p| {
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
            focused: None,
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
        self.press_screen(pressed)
    }

    /// The death screen's button `pressed` (by its index in `death_buttons`), clicked or
    /// chosen with Enter or Space: it takes the focus, as `ContainerEventHandler` gives it.
    pub fn press_screen(&mut self, pressed: usize) -> ScreenAction {
        let Some(screen) = self.death_screen() else {
            return ScreenAction::None;
        };
        if !screen.active {
            return ScreenAction::None;
        }
        self.focus_screen(Some(pressed));
        let seed = self.sound_seed();
        self.sounds.push(crate::sound::Instance::ui(
            "minecraft:ui.button.click",
            1.0,
            seed,
        ));
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
            // `TitleConfirmScreen` opens with nothing focused.
            (false, _) => {
                if let Some(death) = &mut self.death {
                    (death.confirm, death.ticks, death.focused) = (true, 0, None);
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

    /// Whether the server closed the open screen since this was last asked, so the game
    /// takes the mouse back.
    pub fn take_closed_screen(&mut self) -> bool {
        self.inventory.take_closed_by_server()
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
        let menu = self.inventory.menu();
        let hovered = slot_at(menu, width, height, mouse);
        let outside = outside(menu, width, height, mouse);
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
        let menu = self.inventory.menu();
        self.inventory
            .mouse_moved(mc_render::hud::inventory::slot_at(
                menu, width, height, mouse,
            ));
    }

    pub fn inventory_key(
        &mut self,
        key: crate::inventory::InventoryKey,
        mouse: Option<[f32; 2]>,
        width: i32,
        height: i32,
    ) {
        let menu = self.inventory.menu();
        let hovered =
            mouse.and_then(|m| mc_render::hud::inventory::slot_at(menu, width, height, m));
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

    /// Moves the death screen's focus to button `focused`, by its index in `death_buttons`.
    pub fn focus_screen(&mut self, focused: Option<usize>) {
        if let Some(death) = &mut self.death {
            death.focused = focused;
        }
    }

    pub fn death_screen(&self) -> Option<mc_render::hud::DeathScreen> {
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
            focused: death.focused,
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
            air: self.air,
            eye_in_water: self.player.eye_in_water(),
        });
        mc_render::hud::Hotbar {
            slots: std::array::from_fn(|i| self.inventory.slots[36 + i].clone()),
            offhand: self.inventory.slots[45].clone(),
            selected: self.selected,
            vitals,
            death: self.death_screen(),
            chat: Default::default(),
            attack_indicator: self.attack_indicator(),
        }
    }

    /// `Hud.extractCrosshair`'s attack indicator: full once recharged while aiming at a
    /// living entity with a weapon slower than 4 attacks a second, a bar while recharging;
    /// or with the option at Hotbar, `Hud.extractItemHotbar`'s, only while recharging.
    fn attack_indicator(&self) -> Option<mc_render::hud::AttackIndicator> {
        use mc_render::hud::AttackIndicator;
        use mc_render::hud::options::AttackIndicator as Mode;
        if self.game_mode == 3 {
            return None;
        }
        let strength = self.hands.attack_strength();
        match self.attack_indicator_mode {
            Mode::Off => return None,
            Mode::Hotbar => return (strength < 1.0).then_some(AttackIndicator::Hotbar(strength)),
            Mode::Crosshair => {}
        }
        if strength >= 1.0
            && self.hit_entity.is_some_and(|(_, alive)| alive)
            && self.hands.attack_delay() > 5.0
        {
            Some(AttackIndicator::Full)
        } else if strength < 1.0 {
            Some(AttackIndicator::Charging(strength))
        } else {
            None
        }
    }

    /// `GameRenderer`'s view bob and hurt tilt this frame, for [`mc_render::gpu::Camera::bob`].
    pub fn view_bob(&self) -> glam::Mat4 {
        self.view_bob.matrix(self.since_tick / TICK, self.dead())
    }

    /// What the camera at `eye` sees from inside water or lava: `FogRenderer.setupFog` with
    /// `WaterFogEnvironment` or `LavaFogEnvironment`, and `LevelExtractor`'s underwater
    /// overlay while the player's eye is in water. The darkening near the bottom of the
    /// world isn't applied yet.
    /// `Camera.extractRenderState`'s `smartCull`: off for a spectator whose eye is inside a
    /// block that's `isSolidRender`, who would otherwise see nothing past it.
    pub fn smart_cull(&self, eye: Vec3) -> bool {
        use mc_physics::World;
        let [x, y, z] = eye.floor().as_ivec3().to_array();
        !(self.game_mode == 3 && self.blocks.solid_render(self.collision.state(x, y, z)))
    }

    pub fn in_fluid(&self, eye: Vec3, yaw: f32, pitch: f32) -> Option<mc_render::gpu::InFluid> {
        use mc_physics::fluid::FluidKind;
        let kind = mc_physics::fluid::camera_fluid(&self.collision, &self.blocks, eye.as_dvec3())?;
        let rgb = |c: u32| [c >> 16, c >> 8 & 0xFF, c & 0xFF].map(|v| v as f32 / 255.0);
        Some(match kind {
            FluidKind::Water => {
                // `WATER_FOG_COLOR` and `WATER_FOG_END_DISTANCE`, from their defaults and
                // the biomes around, brightened toward the brightest channel as the eye
                // gets used to the water (`computeFogColor`).
                let vision = self.player.water_vision();
                let (base, end) = self.water_fog(eye.as_dvec3(), rgb(0x050533), 96.0);
                let scale = 1.0 / base[0].max(base[1]).max(base[2]);
                let overlay = self.player.eye_in_water().then(|| {
                    // `Lightmap.getBrightness` of the eye's light at noon.
                    let level = self.light.at(eye.floor().as_ivec3().to_array());
                    let v = level[0].max(level[1]) as f32 / 15.0;
                    [-yaw / 64.0, pitch / 64.0, v / (4.0 - 3.0 * v)]
                });
                mc_render::gpu::InFluid {
                    fog_color: base.map(|c| c + (c * scale - c) * vision),
                    // `WATER_FOG_START_DISTANCE`'s default; no biome changes it.
                    fog_start: -8.0,
                    fog_end: end * vision.max(0.25),
                    overlay,
                }
            }
            // Fire resistance sees 5 blocks into lava instead of 1.
            FluidKind::Lava => {
                let resistant = self.effects.contains("minecraft:fire_resistance");
                mc_render::gpu::InFluid {
                    fog_color: rgb(0x991A00),
                    fog_start: if resistant { 0.0 } else { 0.25 },
                    fog_end: if resistant { 5.0 } else { 1.0 },
                    overlay: None,
                }
            }
        })
    }

    /// `ScreenEffectRenderer`'s fire: whether the player sees themselves burning, which
    /// spectators don't (`Entity.displayFireAnimation`).
    pub fn sees_fire(&self) -> bool {
        self.on_fire && self.game_mode != 3
    }

    /// The player's own hands as they see them, `pt` of the way to the next tick, looking
    /// at `yaw` and `pitch`, and the light at `eye`; none when they don't see them (dead,
    /// or a spectator).
    pub fn first_person(
        &self,
        yaw: f32,
        pitch: f32,
        eye: Vec3,
    ) -> Option<(mc_render::entity::FirstPerson, [u8; 2])> {
        if self.dead() || self.game_mode == 3 {
            return None;
        }
        let pt = self.since_tick / TICK;
        let using = self.using.as_ref().and_then(|u| {
            let &(anim, duration) = self.use_animations.get(&u.item)?;
            (u.remaining > 0).then_some(mc_render::entity::Using {
                hand: u.hand as usize,
                anim,
                duration,
                remaining: u.remaining as f32 - pt + 1.0,
                ticks: duration - u.remaining,
            })
        });
        let hands = self
            .hands
            .view(pt, yaw, pitch, self.entities.skin(self.uuid), using);
        Some((hands, self.light.at(eye.floor().as_ivec3().to_array())))
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

    /// Accepts the server's teleport, now that the player has moved there.
    pub fn accept_teleport(&self, teleport: &mc_protocol::Teleport) {
        if let Err(e) = self.outbox.accept_teleport(teleport) {
            eprintln!("accepting a teleport: {e:#}");
        }
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
        // `DeltaTracker.Timer.advanceGameTime`: every whole tick the time holds is due and
        // only the fraction carries over, but `Minecraft.runTick` runs at most 10 of them a
        // frame, so after a long stall the game falls behind instead of racing to catch up.
        self.since_tick += dt;
        let due = (self.since_tick / TICK).floor();
        self.since_tick -= due * TICK;
        for _ in 0..(due as u32).min(10) {
            // `ChestBlockEntity.lidAnimateTick`.
            for lid in self.chest_lids.values_mut() {
                lid.tick();
            }
            self.chest_lids.retain(|_, l| l.shut_for < 20);
            // `Hud.tick` and `LivingEntity.baseTick`'s countdown.
            self.health.tick = self.health.tick.wrapping_add(1);
            self.health.damage_cooldown = self.health.damage_cooldown.saturating_sub(1);
            if let Some(death) = &mut self.death {
                death.ticks += 1;
            }
            // `ClientLevel.tickTime`, the lightmap's flicker, and the attributes the sky
            // blends between.
            let game_time = self.sky.game_time + 1;
            self.sky.tick_clocks(game_time);
            let r = &mut self.sky.random;
            let (a, b, c, d) = (
                r.next_float(),
                r.next_float(),
                r.next_float(),
                r.next_float(),
            );
            self.sky.flicker = (self.sky.flicker + (a - b) * c * d * 0.1) * 0.9;
            self.sky.flash = self.sky.flash.saturating_sub(1);
            // `Hud.updateVignetteBrightness`: the dark where the eye is, as the level's raw
            // brightness (the sky light less the night's `skyDarken`, or the block light)
            // seems after the dimension's ambient light.
            if let (Some(environment), Some([_, now])) = (&self.sky.environment, self.sky.visuals) {
                let eye = self.player.pos + DVec3::Y * self.camera_eye[1] as f64;
                let [sky, block] = self.light.at(eye.floor().as_ivec3().to_array());
                let darken = (15.0 - now.sky_light_level) as i32;
                let raw = (i32::from(sky) - darken).max(i32::from(block)) as f32 / 15.0;
                let curved = raw / (4.0 - 3.0 * raw);
                let level = curved + (1.0 - curved) * environment.ambient_light;
                let target = (1.0 - level).clamp(0.0, 1.0);
                self.sky.vignette += (target - self.sky.vignette) * 0.01;
            }
            // `AtmosphericFogEnvironment.updateRainFogState`, a tick at a time: rain pulls
            // the fog in where the sky shows, half as far where it doesn't rain.
            {
                let eye = self.player.pos + DVec3::Y * self.camera_eye[1] as f64;
                let block = eye.floor().as_ivec3();
                let sky_light = f32::from(self.light.at(block.to_array())[0]);
                let biome = self.biome_at_quart((block >> 2i32).to_array());
                let rains = if self.biome_data.has_precipitation(biome) {
                    1.0
                } else {
                    0.5
                };
                let target = self.sky.rain * ((sky_light - 8.0) / 7.0).clamp(0.0, 1.0) * rains;
                let [_, fog] = self.sky.rain_fog;
                self.sky.rain_fog = [fog, fog + (target - fog) * 0.2];
            }
            if let Some(now) = self.sample_sky() {
                let old = self.sky.visuals.map_or(now, |[_, new]| new);
                self.sky.visuals = Some([old, now]);
            }
            // `Camera.tickFov`: halfway to `getFieldOfViewModifier` each tick. Only a drawn
            // bow narrows it here; flying and speed don't widen it yet.
            let target = match &self.using {
                Some(u) if u.item == "minecraft:bow" => {
                    let ticks = self.use_animations.get(&u.item).map_or(0, |a| a.1) - u.remaining;
                    1.0 - (ticks as f32 / 20.0).min(1.0).powi(2) * 0.15
                }
                _ => 1.0,
            };
            let [_, now] = self.fov_modifier;
            self.fov_modifier = [now, (now + (target - now) * 0.5).clamp(0.1, 1.5)];
            // `Camera.tick`, before the entities move.
            let eye = self.player.eye_height() as f32;
            self.camera_eye = [
                self.camera_eye[1],
                self.camera_eye[1] + (eye - self.camera_eye[1]) * 0.5,
            ];
            self.previous = self.player.pos;
            if self.entities.tick(&self.collision, &self.blocks) {
                self.sky.flash = 2;
            }
            // `LightningBolt.tick`: heard from anywhere, the impact only nearby.
            for (pos, thunder, impact) in std::mem::take(&mut self.entities.thunder) {
                for (event, volume, pitch) in [
                    ("minecraft:entity.lightning_bolt.thunder", 10000.0, thunder),
                    ("minecraft:entity.lightning_bolt.impact", 2.0, impact),
                ] {
                    let seed = self.sound_seed();
                    self.sounds.push(crate::sound::Instance::at(
                        event,
                        crate::sound::Category::Weather,
                        volume,
                        pitch,
                        pos,
                        seed,
                    ));
                }
            }
            self.tick_rain_sound();
            if let Some((_, _, ticks)) = &mut self.placing {
                *ticks = ticks.saturating_sub(1);
                if *ticks == 0 {
                    self.placing = None;
                }
            }
            let dead = self.dead();
            self.view_bob.tick(
                self.player.velocity,
                self.player.on_ground && !self.player.swimming,
                dead,
            );
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
            self.tick_using();
            let before = self.player.pos;
            self.player.tick(input, &self.collision, &self.blocks);
            self.movement_sounds();
            let moved = self.player.pos - before;
            self.view_bob.walked(moved.x.hypot(moved.z) as f32);
            let held = [36 + self.selected as usize, 45].map(|i| {
                self.inventory.slots[i]
                    .as_ref()
                    .map(|(item, _)| item.as_str())
            });
            self.hands.tick(held, yaw, pitch);
            if let Err(e) = self.send_changes(input) {
                eprintln!("sending movement: {e:#}");
            }
        }
        let feet = self
            .previous
            .lerp(self.player.pos, (self.since_tick / TICK) as f64);
        let pt = self.since_tick / TICK;
        let camera_eye = self.camera_eye[0] + (self.camera_eye[1] - self.camera_eye[0]) * pt;
        // Aiming uses the pose's own eye height, as `Entity.pick` does.
        let eye = feet + DVec3::Y * self.player.eye_height();
        // `LocalPlayer.pick`: `blockInteractionRange` is 4.5 and `entityInteractionRange` 3,
        // each with creative's longer reach. The block in the way limits how far an
        // entity can be hit, and each is dropped beyond its own range (`filterHitResult`).
        let (block_reach, entity_reach) = if self.instabuild {
            (5.0, 5.0)
        } else {
            (4.5, 3.0)
        };
        let reach: f64 = f64::max(block_reach, entity_reach);
        let look = mc_physics::view_vector(yaw, pitch);
        let block = mc_physics::pick(&self.collision, &self.blocks, eye, eye + look * reach);
        let block_sq = block.map_or(reach * reach, |h| h.location.distance_squared(eye));
        let entity = self
            .entities
            .pick(eye, eye + look * block_sq.sqrt(), block_sq)
            .filter(|(_, p, _)| p.distance_squared(eye) < block_sq);
        (self.hit, self.hit_entity) = match entity {
            Some((id, p, alive)) if p.distance_squared(eye) < entity_reach * entity_reach => {
                (None, Some((id, alive)))
            }
            Some(_) => (None, None),
            None => (
                block.filter(|h| h.location.distance_squared(eye) < block_reach * block_reach),
                None,
            ),
        };
        Some((feet + DVec3::Y * camera_eye as f64).as_vec3())
    }

    /// One tick of `Minecraft.handleKeybinds` for the mouse: clicks start mining, using and
    /// picking; Attack down keeps mining, and Use down keeps using every 4 ticks. While an
    /// item is in use, the clicks are dropped, and Use let go stops using it
    /// (`MultiPlayerGameMode.releaseUsingItem`).
    fn tick_hands(&mut self, b: Buttons) -> anyhow::Result<()> {
        self.right_click_delay = self.right_click_delay.saturating_sub(1);
        self.miss_time = self.miss_time.saturating_sub(1);
        self.send_selected_slot()?;
        let hit = self.hit;
        let [attacks, uses, picks] = std::mem::take(&mut self.clicks);
        // Clicks while an item is in use are dropped.
        let [attacks, uses, picks] = match self.using.is_some() {
            true => [0; 3],
            false => [attacks, uses, picks],
        };
        if self.using.is_some() && !b.use_item {
            self.player_action(RELEASE_USE_ITEM, IVec3::ZERO, 0, false)?;
            self.using = None;
        }
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
        if b.use_item && self.right_click_delay == 0 && self.using.is_none() {
            self.start_use(hit)?;
        }
        // A click shorter than a tick mines for that tick too; using an item doesn't mine
        // (`Minecraft.continueAttack`).
        let held = b.attack || attacks > 0;
        if self.using.is_some() {
            if !held {
                self.miss_time = 0;
            }
            return Ok(());
        }
        self.continue_attack(held, hit)?;
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

    /// `Minecraft.startAttack`: a click on an entity attacks it; on a block starts mining
    /// it; on nothing (or air), a miss. Either way the arm swings, which the server shows
    /// others when it gets a `punch`.
    fn start_attack(&mut self, hit: Option<BlockHit>) -> anyhow::Result<()> {
        if self.miss_time > 0 {
            return Ok(());
        }
        match (self.hit_entity, hit) {
            // `MultiPlayerGameMode.attack`: the server deals the damage and knockback;
            // the client's `Player.attack` only restarts the attack's recharge.
            (Some((id, _)), _) if self.game_mode != 3 => {
                self.send_selected_slot()?;
                let mut body = Vec::new();
                body.put_varint(id);
                self.outbox.send("attack", &body)?;
                self.hands.reset_swap();
                if self.instabuild {
                    self.destroy_delay = 5;
                }
            }
            (_, Some(h)) if !self.is_air(h.pos) => self.start_destroy(h.pos, h.face)?,
            _ => {
                if !self.instabuild {
                    self.miss_time = 10;
                }
                self.hands.reset_swap();
            }
        }
        self.hands.swing(0);
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
                    self.hands.swing(0);
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
            self.creative_break(pos);
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
        if progress < 1.0 {
            self.hit_sound(pos);
        } else {
            self.break_sound(pos.to_array(), state);
        }
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
            self.creative_break(pos);
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
                    self.break_sound(pos.to_array(), state);
                    self.player_action(STOP_DESTROY_BLOCK, pos, f, true)
                } else {
                    self.destroying = Some((p, f, progress));
                    Ok(())
                }
            }
            _ => self.start_destroy(pos, face),
        }
    }

    /// `MultiPlayerGameMode.destroyBlock` in creative: the block breaks, with its sound.
    fn creative_break(&mut self, pos: IVec3) {
        let state = mc_physics::World::state(&self.collision, pos.x, pos.y, pos.z);
        self.break_sound(pos.to_array(), state);
    }

    /// `MultiPlayerGameMode.stopDestroyBlock`: letting go stops mining.
    fn stop_destroy(&mut self) -> anyhow::Result<()> {
        match self.destroying.take() {
            // Vanilla says down here, whatever face was mined.
            Some((pos, ..)) => {
                self.hands.reset_swap();
                self.player_action(ABORT_DESTROY_BLOCK, pos, 0, false)
            }
            None => Ok(()),
        }
    }

    /// `Minecraft.startUseItem`: for the main hand then the off hand, use the held item on
    /// the block looked at, placing it if it's a block; unless the block takes the click
    /// itself (`useWithoutItem`, skipped while sneaking with something in hand), use the
    /// item on nothing. The first hand that places a block or starts using its item ends
    /// it, as does a block taking the click. Placing swings the arm, and placing or
    /// using drops the hand (`itemUsed`); other successes aren't predicted.
    fn start_use(&mut self, hit: Option<BlockHit>) -> anyhow::Result<()> {
        self.right_click_delay = 4;
        if self.destroying.is_some() {
            return Ok(());
        }
        let secondary =
            self.player.shift_key_down() && (self.held(0).is_some() || self.held(1).is_some());
        for hand in 0..2u8 {
            let held = self.held(hand).map(str::to_owned);
            if let Some(h) = hit {
                self.sequence += 1;
                let cursor = h.location - h.pos.as_dvec3();
                let mut body = Vec::new();
                body.put_varint(i32::from(hand))
                    .put_block_pos(h.pos.to_array())
                    .put_varint(h.face as i32)
                    .put_f32(cursor.x as f32)
                    .put_f32(cursor.y as f32)
                    .put_f32(cursor.z as f32)
                    .put_bool(h.inside)
                    .put_bool(false) // world border hit
                    .put_varint(self.sequence);
                self.outbox.send("use_item_on", &body)?;
                if let Some(&block) = held.as_ref().and_then(|item| self.block_items.get(item)) {
                    self.hands.swing(hand);
                    self.hands.item_used(hand as usize);
                    // The block lands on the block clicked, if that's replaceable, or
                    // against its face.
                    let [dx, dy, dz] = FACE_OFFSETS[h.face as usize];
                    let next = h.pos + IVec3::new(dx, dy, dz);
                    self.placing = Some(([h.pos, next], block, 20));
                    return Ok(());
                }
                let state = mc_physics::World::state(&self.collision, h.pos.x, h.pos.y, h.pos.z);
                if !secondary && self.blocks.interactive(state) {
                    return Ok(());
                }
            }
            let Some(item) = held else {
                continue;
            };
            // `MultiPlayerGameMode.useItem`.
            self.sequence += 1;
            let mut body = Vec::new();
            body.put_varint(i32::from(hand))
                .put_varint(self.sequence)
                .put_f32(self.player.yaw)
                .put_f32(self.player.pitch);
            self.outbox.send("use_item", &body)?;
            if self.starts_using(&item) {
                self.start_using(hand);
                self.hands.item_used(hand as usize);
                return Ok(());
            }
        }
        Ok(())
    }

    /// The item in `hand` (0 main, 1 off).
    fn held(&self, hand: u8) -> Option<&str> {
        let slot = if hand == 0 {
            36 + self.selected as usize
        } else {
            45
        };
        self.inventory.slots[slot]
            .as_ref()
            .map(|(item, _)| item.as_str())
    }

    /// Whether `Item.use` starts using `item`: food the player can eat (when hungry,
    /// unless it's always edible or they're in creative) and drinks that take time
    /// (`Consumable.startConsuming`), a shield (`blocks_attacks`), and a bow with arrows to
    /// shoot, or in creative (`BowItem.use`).
    fn starts_using(&self, item: &str) -> bool {
        let creative = self.game_mode == 1;
        if let Some(using) = self.item_uses.get(item) {
            if let Some(c) = &using.consumable {
                let edible = using
                    .food
                    .is_none_or(|always| always || creative || self.food < 20);
                return edible && c.ticks() > 0;
            }
            if using.blocks_attacks {
                return true;
            }
        }
        // `Player.getProjectile`: arrows anywhere in the inventory or hands.
        let arrows = self.inventory.slots[9..=45].iter().flatten().any(|(i, _)| {
            matches!(
                i.as_str(),
                "minecraft:arrow" | "minecraft:tipped_arrow" | "minecraft:spectral_arrow"
            )
        });
        item == "minecraft:bow" && (creative || arrows)
    }

    /// `LivingEntity.startUsingItem`: the item in `hand`, for its use duration.
    fn start_using(&mut self, hand: u8) {
        let Some(item) = self.held(hand).map(str::to_owned) else {
            return;
        };
        let remaining = self.use_animations.get(&item).map_or(0, |u| u.1);
        self.using = Some(Use {
            hand,
            item,
            remaining,
        });
    }

    /// `LivingEntity.updatingUsingItem`, each tick: while the item stays in the hand, it
    /// ticks (`Consumable.shouldEmitParticlesAndSounds`: crumbs and a crunch every 4
    /// ticks, once past the first 7/32 of the time) and counts down, which the server
    /// finishes; another item stops it. Using it slows the player as its `use_effects` say.
    fn tick_using(&mut self) {
        if let Some(u) = &self.using {
            if self.held(u.hand) == Some(u.item.as_str()) {
                let (item, remaining) = (u.item.clone(), u.remaining);
                let consumable = self
                    .item_uses
                    .get(&item)
                    .and_then(|i| i.consumable.as_ref());
                if let Some(ticks) = consumable.map(mc_registry::Consumable::ticks) {
                    let wait = (ticks as f32 * 0.21875) as i32;
                    if ticks - remaining > wait && remaining % 4 == 0 {
                        self.consume_effects(&item, 5);
                    }
                }
                if let Some(u) = &mut self.using {
                    u.remaining -= 1;
                }
            } else {
                self.using = None;
            }
        }
        self.player.using_item = self.using.as_ref().map(|u| {
            let effects = self
                .item_uses
                .get(&u.item)
                .map(|i| i.use_effects.clone())
                .unwrap_or_default();
            (effects.speed_multiplier, effects.can_sprint)
        });
    }

    /// `LivingEntity.completeUsingItem` on the client: the item, if still held, is
    /// consumed with a burst of crumbs and its sound (`Consumable.onConsume`), and its use
    /// stops.
    fn complete_using(&mut self) {
        let Some(u) = self.using.take() else {
            return;
        };
        if self.held(u.hand) == Some(u.item.as_str()) {
            self.consume_effects(&u.item, 16);
        }
    }

    /// `Consumable.emitParticlesAndSounds`: `count` crumbs of the item from the mouth, and
    /// its sound, which the player hears themselves (`Player.playSound`); eating is loud
    /// or quiet at random, drinking always half as loud.
    fn consume_effects(&mut self, item: &str, count: u32) {
        let Some(c) = self.item_uses.get(item).and_then(|i| i.consumable.clone()) else {
            return;
        };
        // `ItemStackRenderState.pickParticleMaterial`: a generated model's particle is its
        // first layer.
        let sprite = match self.item_icons.get(item) {
            Some(mc_render::item::ItemIcon::Flat { layers, .. }) => layers.first().map(|l| l.0),
            _ => None,
        };
        if let Some(rect) = sprite.filter(|_| c.has_consume_particles) {
            let eye = self.player.pos + DVec3::Y * self.player.eye_height();
            let (yaw, pitch) = (self.player.yaw, self.player.pitch);
            self.entities
                .particles
                .item_crumbs(rect, eye, yaw, pitch, count);
        }
        let r = &mut self.sound_random;
        let (volume, pitch) = if c.animation == "drink" {
            (0.5, 0.9 + r.next_float() * 0.1)
        } else {
            let volume = if r.next_float() < 0.5 { 0.5 } else { 1.0 };
            (volume, 1.0 + 0.2 * (r.next_float() - r.next_float()))
        };
        self.player_sound(&c.sound, volume, pitch);
    }

    /// `Camera.getFov`'s modifier, `pt` of the way to the next tick: how much a drawn bow
    /// narrows the view.
    pub fn fov_modifier(&self) -> f32 {
        let [old, now] = self.fov_modifier;
        old + (now - old) * (self.since_tick / TICK)
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
