//! Other players, mobs and dropped items, as the server reports them, moved the way the
//! vanilla client moves them: living entities glide to each reported position over a few
//! ticks (`SteppedInterpolationHandler`), turn their bodies after their heads
//! (`LivingEntity.tickHeadTurn`, `BodyRotationControl`) and swing their limbs with how far
//! they went (`WalkAnimationState`); dropped items fall, slide and bounce on their own
//! (`ItemEntity.tick`) and snap to where the server says.

use glam::DVec3;
use mc_protocol::{DataValue, Event, Path};
use mc_render::entity::{Armor, Dropped, Entity, Equipment, Kind, Lit, Living, NameTag, Swing};
use std::collections::{HashMap, HashSet, VecDeque};

/// `Mth.wrapDegrees`.
fn wrap(a: f32) -> f32 {
    let a = a % 360.0;
    if a >= 180.0 {
        a - 360.0
    } else if a < -180.0 {
        a + 360.0
    } else {
        a
    }
}

/// `Mth.rotLerp`.
fn rot_lerp(t: f32, from: f32, to: f32) -> f32 {
    from + t * wrap(to - from)
}

/// `Mth.rotateIfNecessary`: `base` turned no further than `max` from `target`.
fn rotate_if_necessary(base: f32, target: f32, max: f32) -> f32 {
    target - wrap(target - base).clamp(-max, max)
}

/// A position and rotation (`PositionAndRotation`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Place {
    pos: DVec3,
    yaw: f32,
    pitch: f32,
}

impl Place {
    fn lerp(&self, to: &Place, t: f32) -> Place {
        Place {
            pos: self.pos.lerp(to.pos, t as f64),
            yaw: rot_lerp(t, self.yaw, to.yaw),
            pitch: self.pitch + (to.pitch - self.pitch) * t,
        }
    }
}

/// `SteppedInterpolationHandler`: the positions still to pass through, each reached so
/// many ticks after the one before, speeding up when they pile up.
#[derive(Debug)]
struct Interpolation {
    /// The type's `updateInterval`: ticks a move takes.
    steps: i32,
    remaining: VecDeque<(Place, i32)>,
    last_step: Place,
    current_step_ticks: f32,
    remaining_ticks: f32,
    speed: f32,
    /// Where it all ends.
    target: Place,
}

impl Interpolation {
    fn new(steps: i32) -> Self {
        let zero = Place {
            pos: DVec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
        };
        Self {
            steps,
            remaining: VecDeque::new(),
            last_step: zero,
            current_step_ticks: 0.0,
            remaining_ticks: 0.0,
            speed: 1.0,
            target: zero,
        }
    }

    fn active(&self) -> bool {
        !self.remaining.is_empty()
    }

    fn cancel(&mut self) {
        self.remaining.clear();
        self.remaining_ticks = 0.0;
        self.speed = 1.0;
    }

    fn add_step(&mut self, place: Place, ticks: i32) {
        self.remaining.push_back((place, ticks));
        self.remaining_ticks += ticks as f32;
    }

    /// `AbstractInterpolationHandler.interpolateTo`, from the entity at `now`: a path or
    /// none (to stay), and a rotation or none (to keep).
    fn interpolate_to(&mut self, now: Place, path: Option<&Path>, rotation: Option<(f32, f32)>) {
        let current = if self.active() { self.target } else { now };
        let path = path
            .cloned()
            .unwrap_or(Path::Linear(current.pos.to_array()));
        let (yaw, pitch) = rotation.unwrap_or((current.yaw, current.pitch));
        let end = DVec3::from(path.end());
        let place = Place {
            pos: end,
            yaw,
            pitch,
        };
        if self.active() && self.target == place {
            return;
        }
        // `startInterpolating`.
        if !self.active() {
            self.last_step = now;
            self.current_step_ticks = 1.0;
        }
        match &path {
            Path::Stepped(steps) if end != self.target.pos => {
                if yaw == self.target.yaw && pitch == self.target.pitch {
                    for &(p, ticks) in steps {
                        let place = Place {
                            pos: DVec3::from(p),
                            yaw,
                            pitch,
                        };
                        self.add_step(place, ticks);
                    }
                } else {
                    // The turn spreads over the steps by when each is reached.
                    let total: i32 = steps.iter().map(|s| s.1).sum();
                    let (from_yaw, from_pitch) = (self.target.yaw, self.target.pitch);
                    let mut offset = 0;
                    for &(p, ticks) in steps {
                        offset += ticks;
                        let a = offset as f32 / total as f32;
                        let place = Place {
                            pos: DVec3::from(p),
                            yaw: rot_lerp(a, from_yaw, yaw),
                            pitch: from_pitch + (pitch - from_pitch) * a,
                        };
                        self.add_step(place, ticks);
                    }
                }
            }
            _ => self.add_step(place, self.steps),
        }
        self.target = place;
    }

    /// `doInterpolate`: where the entity is this tick; then time moves on a tick.
    fn advance(&mut self) -> Place {
        let place = loop {
            let Some(&(step, offset)) = self.remaining.front() else {
                break self.target;
            };
            if self.current_step_ticks < offset as f32 {
                break self
                    .last_step
                    .lerp(&step, self.current_step_ticks / offset as f32);
            }
            self.current_step_ticks -= offset as f32;
            self.last_step = step;
            self.remaining.pop_front();
        };
        let n = self.steps as f32;
        let target_speed = (self.remaining_ticks / n).max(1.0);
        self.speed += (target_speed - self.speed) / n;
        let mut ticks = 1.0;
        if ticks * self.speed < self.remaining_ticks {
            ticks *= self.speed;
        } else {
            ticks = self.remaining_ticks;
            self.speed = 1.0;
        }
        self.current_step_ticks += ticks;
        self.remaining_ticks -= ticks;
        place
    }
}

/// What kind of thing an entity is, and how it moves on the client.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sort {
    /// A mob: its body follows `BodyRotationControl`.
    Mob(MobType),
    /// Another player: its body follows `LivingEntity.tickHeadTurn`.
    Player,
    Item,
    /// An arrow (`AbstractArrow`), spectral or not.
    Arrow {
        spectral: bool,
    },
    /// A thrown item (`ThrowableItemProjectile`): the item it shows when the server names
    /// none (`getDefaultItem`), and how fast it falls (`getDefaultGravity`).
    Thrown {
        item: &'static str,
        gravity: f64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MobType {
    Zombie,
    Skeleton,
    Creeper,
    Spider,
    Cow,
    Pig,
    Sheep,
    Chicken,
    Enderman,
    Slime,
    Villager,
    Witch,
    Wolf,
    Husk,
    Drowned,
    Stray,
    ZombieVillager,
    CaveSpider,
    IronGolem,
    Squid,
    GlowSquid,
    Phantom,
    Cat,
    Ocelot,
    Fox,
    Bogged,
    SnowGolem,
    MagmaCube,
    Blaze,
    Ghast,
    Silverfish,
    Endermite,
    PolarBear,
    Goat,
    Piglin,
    PiglinBrute,
    ZombifiedPiglin,
    Hoglin,
    Zoglin,
    Strider,
    WitherSkeleton,
    Bat,
    Rabbit,
    Frog,
    Camel,
    Armadillo,
}

struct Tracked {
    /// Its entity id, which sets how fast a witch's nose twitches.
    id: i32,
    sort: Sort,
    uuid: u128,
    /// `VecDeltaCodec`'s base: where the server last said the entity was.
    base: [f64; 3],
    now: Place,
    old: Place,
    interpolation: Interpolation,
    velocity: DVec3,
    on_ground: bool,
    head_yaw: f32,
    old_head_yaw: f32,
    body_yaw: f32,
    old_body_yaw: f32,
    /// `LivingEntity.lerpYHeadRot` and `lerpHeadSteps`.
    head_target: f32,
    head_steps: u32,
    /// `BodyRotationControl`'s state.
    head_stable_time: u32,
    last_stable_head_yaw: f32,
    /// `WalkAnimationState`.
    walk_speed_old: f32,
    walk_speed: f32,
    walk_pos: f32,
    /// `tickCount`.
    age: u32,
    /// `ItemEntity.bobOffs`.
    bob_offset: f32,
    /// A chicken's `flap`, `flapSpeed` and `flapping`, and the first two last tick.
    flap: [f32; 2],
    flap_speed: [f32; 2],
    flapping: f32,
    /// A slime's `oSquish`, `squish`, `targetSquish` and `wasOnGround`.
    squish: [f32; 2],
    target_squish: f32,
    was_on_ground: bool,
    data: HashMap<u8, DataValue>,
    /// What it holds and wears, by `EquipmentSlot` id.
    equipment: HashMap<u8, mc_protocol::Equipped>,
    swing: SwingState,
    /// `LivingEntity.hurtTime`: ticks left of the red flash after a hit, from 10.
    hurt_time: i32,
    /// `LivingEntity.deathTime`: ticks since its health ran out.
    death_time: u32,
    /// `LivingEntity.getMaxHealth`, once the server has sent its attributes.
    max_health: Option<f32>,
    /// A mob's own animation state, ticked as vanilla ticks it on the client.
    anim: MobAnim,
    /// The item it's using (`useItem`): its hand (0 main, 1 off), the item, and the ticks
    /// left (`useItemRemaining`).
    using: Option<(u8, String, i32)>,
    /// An arrow's `shakeTime` and whether it's stuck in a block (`IN_GROUND`).
    shake: i32,
    in_ground: bool,
    /// `AbstractArrow.inGroundTime`: ticks it's been stuck.
    in_ground_time: i32,
}

/// Animation state some mobs keep on the client, this tick's and last tick's.
#[derive(Default)]
struct MobAnim {
    /// `IronGolem.attackAnimationTick`, and a hoglin's `attackAnimationRemainingTicks`.
    golem_attack: i32,
    /// `Squid`'s `tentacleMovement`, `tentacleSpeed`, `rotateSpeed`, and last and this
    /// tick's `tentacleAngle`, `xBodyRot` and `zBodyRot`.
    tentacle_movement: f32,
    tentacle_speed: f32,
    rotate_speed: f32,
    tentacle_angle: [f32; 2],
    x_body_rot: [f32; 2],
    z_body_rot: [f32; 2],
    /// `Cat`'s `lieDownAmount`, `lieDownAmountTail` and `relaxStateOneAmount`.
    lie: [f32; 2],
    lie_tail: [f32; 2],
    relax: [f32; 2],
    /// Ticks a piglin has been charging its crossbow.
    crossbow: i32,
    /// `PolarBear.clientSideStandAnimation`, 0 to 6.
    stand: [f32; 2],
    /// `Fox`'s `crouchAmount` and `interestedAngle`.
    crouch: [f32; 2],
    interested: [f32; 2],
    /// The ticks its `AnimationState`s started, while they run: `Bat`'s fly and rest; a
    /// rabbit's hop and idle head tilt; a frog's jump, croak, tongue and swim idle; a
    /// camel's sit, sit pose, sit up, idle and dash; an armadillo's roll out, roll up and
    /// peek.
    started: [Option<i64>; 5],
    /// Whether a frog is in water (`isInWater`), where it swims.
    in_water: bool,
    /// `Armadillo.inStateTicks`, and whether it peeked since (`peekReceivedClient`).
    in_state: i64,
    peeked: bool,
    /// `Rabbit`'s (and a camel's) `idleAnimationTimeout`; a rabbit's `jumpTicks`,
    /// `jumpDuration`, and whether it
    /// kicked up dust as it hopped (`handleEntityEvent(1)`) since that was last taken.
    jump_ticks: i32,
    jump_duration: i32,
    idle_timeout: i32,
    hop_dust: bool,
    /// The mob's own `random`, as an xorshift.
    random: u64,
}

impl MobAnim {
    /// `random.nextInt(bound)`.
    fn next_int(&mut self, bound: u32) -> i32 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        ((self.random >> 32) % u64::from(bound)) as i32
    }
}

/// `Projectile.lerpRotation`: a fifth of the way from `old` to `new`, the short way round.
fn lerp_rotation(mut old: f32, new: f32) -> f32 {
    while new - old < -180.0 {
        old -= 360.0;
    }
    while new - old >= 180.0 {
        old += 360.0;
    }
    old + (new - old) * 0.2
}

/// `PlayerInfo.getSkin`: a player's downloaded skin once it's arrived, else the default
/// skin for the UUID; and whether it has slim arms.
fn skin(skins: &HashMap<u128, (String, bool)>, uuid: u128) -> (String, bool) {
    match skins.get(&uuid) {
        Some(skin) => skin.clone(),
        None => {
            let skin = mc_render::entity_model::default_skin(uuid);
            (skin.to_owned(), skin.contains("/slim/"))
        }
    }
}

/// `LivingEntity.SwingState`: the swing an arm is in, as its hand, `SwingAnimationType` and
/// length in ticks; ticks since it started; and its progress, 0 to 1, last tick and now.
#[derive(Clone, Default)]
pub(crate) struct SwingState {
    pub(crate) current: Option<(u8, u8, i32)>,
    ticks: i32,
    old: f32,
    now: f32,
}

impl SwingState {
    /// `startIfAble`: a swing starts unless one is under half way through; the same swing
    /// again restarts without resetting its progress, so it runs on smoothly.
    pub(crate) fn start_if_able(&mut self, swing: (u8, u8, i32)) {
        if let Some((.., duration)) = self.current
            && self.ticks > 0
            && self.ticks <= duration / 2
        {
            return;
        }
        if self.current != Some(swing) {
            (self.old, self.now) = (0.0, 0.0);
        }
        self.current = Some(swing);
        self.ticks = 0;
    }

    pub(crate) fn tick(&mut self) {
        self.old = self.now;
        match self.current {
            Some((.., duration)) => {
                if duration > 0 {
                    self.now = (self.ticks as f32 / duration as f32).min(1.0);
                }
                // `ticks++ > durationTicks`.
                self.ticks += 1;
                if self.ticks - 1 > duration {
                    self.current = None;
                    self.now = 0.0;
                }
            }
            None => self.now = 0.0,
        }
    }

    /// `getAnimation`: the progress `pt` of the way to the next tick; a swing that
    /// restarted runs on past 1 rather than back.
    pub(crate) fn animation(&self, pt: f32) -> f32 {
        let now = if self.now >= self.old {
            self.now
        } else {
            self.now + 1.0
        };
        self.old + (now - self.old) * pt
    }
}

/// `SynchedEntityData` indices: `Entity` has 8, `LivingEntity` 7, `Mob` 1 (its flags),
/// `AgeableMob` and `Zombie` start with the baby flag, then each type's own.
const SHARED_FLAGS: u8 = 0;
/// `Entity.DATA_CUSTOM_NAME`.
const CUSTOM_NAME: u8 = 2;
/// `Pose.SITTING`'s id.
const SITTING: i32 = 10;
/// `Camel.DASH` and `LAST_POSE_CHANGE_TICK`, after `AbstractHorse.DATA_ID_FLAGS`.
const CAMEL_DASH: u8 = 19;
const CAMEL_POSE_TICK: u8 = 20;
/// `Armadillo.ARMADILLO_STATE` (0 idle, 1 rolling, 2 scared, 3 unrolling), after
/// `AgeableMob`'s two.
const MONSTER_STATE: u8 = 18;
/// `Entity.DATA_NO_GRAVITY`.
const NO_GRAVITY: u8 = 5;
const POSE: u8 = 6;
const HEALTH: u8 = 9;
/// `LivingEntity.DATA_EFFECT_PARTICLES` and `DATA_EFFECT_AMBIENCE_ID`.
const EFFECT_PARTICLES: u8 = 10;
const EFFECT_AMBIENCE: u8 = 11;
/// `Avatar.DATA_PLAYER_MAIN_HAND`: `HumanoidArm`'s id, left 0 or right 1.
const MAIN_HAND: u8 = 15;
/// `Pose.CROUCHING`'s id.
const CROUCHING: i32 = 5;
const ITEM: u8 = 8;
/// `AbstractArrow`'s flags (1 a critical hit, 2 no physics), `IN_GROUND`, and
/// `Arrow.ID_EFFECT_COLOR` (-1 for none).
const ARROW_FLAGS: u8 = 8;
const IN_GROUND: u8 = 10;
const ARROW_COLOR: u8 = 11;
/// `LivingEntity.DATA_LIVING_ENTITY_FLAGS`: 1 using an item, 2 in the off hand.
const LIVING_FLAGS: u8 = 8;
const MOB_FLAGS: u8 = 15;
const BABY: u8 = 16;
const SHEEP_WOOL: u8 = 18;
/// Cows' and chickens' variants come straight after `AgeableMob`'s two; a pig's after its
/// boost time.
const VARIANT: u8 = 18;
const PIG_VARIANT: u8 = 19;
/// `AbstractCubeMob.ID_SIZE`, after `AgeableMob`'s two, as slimes are ageable.
const SLIME_SIZE: u8 = 18;
/// `Enderman.DATA_CARRY_STATE` (a state id, 0 for none) and `DATA_CREEPY`.
const CARRY_STATE: u8 = 16;
const CREEPY: u8 = 17;
/// `AbstractVillager.DATA_UNHAPPY_COUNTER` and `Villager.DATA_VILLAGER_DATA`, after
/// `AgeableMob`'s two.
const UNHAPPY: u8 = 18;
const VILLAGER_DATA: u8 = 19;
/// `TamableAnimal.DATA_FLAGS_ID` (1 sitting, 4 tame), after `AgeableMob`'s two, and
/// `Wolf`'s collar, anger end time and variant.
const TAME_FLAGS: u8 = 18;
const COLLAR: u8 = 21;
const ANGER_END: u8 = 22;
const WOLF_VARIANT: u8 = 23;
/// `ZombieVillager.DATA_VILLAGER_DATA`, after `Zombie`'s three and its converting flag.
const ZOMBIE_VILLAGER_DATA: u8 = 20;
/// `Phantom.ID_SIZE`, straight after `Mob`'s.
const PHANTOM_SIZE: u8 = 16;
/// `Cat`'s variant, `IS_LYING`, `RELAX_STATE_ONE` and collar, after `TamableAnimal`'s two.
const CAT_VARIANT: u8 = 20;
const CAT_LYING: u8 = 21;
const CAT_RELAX: u8 = 22;
const CAT_COLLAR: u8 = 23;
/// `Fox.DATA_TYPE_ID` (0 red, 1 snow) and `DATA_FLAGS_ID`, after `AgeableMob`'s two.
const FOX_TYPE: u8 = 18;
const FOX_FLAGS: u8 = 19;
/// `GlowSquid.DATA_DARK_TICKS_REMAINING`, after `AgeableMob`'s two.
const DARK_TICKS: u8 = 18;
/// The first field a `Monster`, golem or `Mob` of its own adds, straight after `Mob`'s:
/// `Bogged.DATA_SHEARED`, `SnowGolem.DATA_PUMPKIN_ID` (16 has a pumpkin),
/// `Blaze.DATA_FLAGS_ID` (1 charged, which shows it on fire) and
/// `Ghast.DATA_IS_CHARGING`; and `Bat.DATA_ID_FLAGS` (1 resting), as an
/// `AmbientCreature` adds none.
const MONSTER_DATA: u8 = 16;
/// `PolarBear.DATA_STANDING_ID`, and `Goat`'s horns, after `AgeableMob`'s two and its
/// screaming flag.
const BEAR_STANDING: u8 = 18;
const GOAT_LEFT_HORN: u8 = 19;
const GOAT_RIGHT_HORN: u8 = 20;
/// `Piglin.DATA_BABY_ID`, `DATA_IS_CHARGING_CROSSBOW` and `DATA_IS_DANCING`, after
/// `AbstractPiglin`'s immunity to zombification.
const PIGLIN_BABY: u8 = 17;
const PIGLIN_CHARGING: u8 = 18;
const PIGLIN_DANCING: u8 = 19;
/// `Strider.DATA_SUFFOCATING`, after `AgeableMob`'s two and its boost time.
const STRIDER_COLD: u8 = 19;

pub struct Entities {
    /// Entity type names by protocol id.
    types: Vec<String>,
    /// Players' downloaded skins, by UUID: the atlas sprite and whether it's slim.
    skins: HashMap<u128, (String, bool)>,
    /// Players' names, by UUID, from the player list.
    names: HashMap<u128, String>,
    /// Each wearable item's equipment asset (`equippable`'s `asset_id`), by item.
    armor: HashMap<String, String>,
    /// Item icons, whose sprites thrown items break into.
    icons: std::sync::Arc<HashMap<String, mc_render::item::ItemIcon>>,
    /// What using each item looks like and how long it lasts, by item.
    uses: HashMap<String, (mc_render::entity::UseAnim, i32)>,
    tracked: HashMap<i32, Tracked>,
    /// The player itself, which the server doesn't track for it, as the third-person
    /// views draw it.
    own: Option<Tracked>,
    /// Seeds items' bob offsets, as `Entity.random` would.
    seed: u64,
    /// The poofs of smoke dying mobs leave.
    pub(crate) particles: crate::particles::Particles,
    /// `TrackingEmitter`s: sparks burst from an entity hit critically, for three ticks.
    /// The entity, the particle, and the bursts left.
    emitters: Vec<(i32, crate::particles::Kind, u8)>,
    /// Lightning bolts, which `LightningBolt.tick` runs on the client too.
    bolts: HashMap<i32, Bolt>,
    random: mc_render::bake::JavaRandom,
    /// Where bolts struck since this was last taken, with their thunder's and impact's
    /// pitches (`LightningBolt.tick`).
    pub thunder: Vec<(DVec3, f32, f32)>,
    /// The level's game time, which a wolf's anger lasts until (`NeutralMob.isAngry`).
    pub game_time: i64,
    /// A carved pumpkin's default state, which a snow golem wears.
    pumpkin: u32,
    /// Items with the `tool` component (`AbstractPiglin.isHoldingMeleeWeapon`): the
    /// `swords`, `axes`, `pickaxes`, `shovels` and `hoes` tags, shears, the mace and the
    /// trident; and the `piglin_loved` tag, which a piglin admires.
    tools: HashSet<String>,
    piglin_loved: HashSet<String>,
}

/// A lightning bolt: where it struck, the ticks it has left to show, the times it'll
/// flash again, and the seed its shape grows from.
struct Bolt {
    pos: DVec3,
    life: i32,
    flashes: i32,
    seed: i64,
}

impl Entities {
    pub fn new(
        registry: &mc_registry::Registry,
        icons: std::sync::Arc<HashMap<String, mc_render::item::ItemIcon>>,
    ) -> Self {
        Self {
            pumpkin: registry
                .block("minecraft:carved_pumpkin")
                .map_or(0, |b| b.default_state),
            tools: {
                let res = mc_assets::Resources::new(mc_assets::default_root());
                let tags = ["swords", "axes", "pickaxes", "shovels", "hoes"];
                let mut tools: HashSet<String> = tags
                    .iter()
                    .flat_map(|t| res.tag("item", t).unwrap_or_default())
                    .collect();
                tools.extend(
                    ["minecraft:shears", "minecraft:mace", "minecraft:trident"].map(String::from),
                );
                tools
            },
            piglin_loved: mc_assets::Resources::new(mc_assets::default_root())
                .tag("item", "piglin_loved")
                .unwrap_or_default(),
            types: (0..)
                .map_while(|i| registry.entity_type(i).map(str::to_owned))
                .collect(),
            skins: HashMap::new(),
            names: HashMap::new(),
            armor: (0..registry.item_count())
                .filter_map(|id| {
                    let item = registry.item_name(id)?;
                    let asset = registry.item_info(item)?.equipment.clone()?;
                    Some((item.to_owned(), asset))
                })
                .collect(),
            uses: crate::hands::use_animations(registry),
            icons,
            tracked: HashMap::new(),
            own: None,
            seed: 0x5DEECE66D,
            particles: Default::default(),
            emitters: Vec::new(),
            bolts: HashMap::new(),
            thunder: Vec::new(),
            game_time: 0,
            random: mc_render::bake::JavaRandom::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as i64),
            ),
        }
    }

    /// Draws this player with a downloaded skin, or with its default one again for `None`.
    pub fn set_skin(&mut self, uuid: u128, skin: Option<(String, bool)>) {
        match skin {
            Some(skin) => self.skins.insert(uuid, skin),
            None => self.skins.remove(&uuid),
        };
    }

    /// Names this player over its head, or forgets its name for `None`.
    /// A player's skin sprite, and whether it has slim arms.
    pub fn skin(&self, uuid: u128) -> (String, bool) {
        skin(&self.skins, uuid)
    }

    pub fn set_name(&mut self, uuid: u128, name: Option<String>) {
        match name {
            Some(name) => self.names.insert(uuid, name),
            None => self.names.remove(&uuid),
        };
    }

    /// Applies an entity event; returns the event back if it isn't one.
    pub fn event(&mut self, event: Event) -> Option<Event> {
        match event {
            Event::AddEntity {
                id,
                uuid,
                kind,
                pos,
                velocity,
                yaw,
                pitch,
                head_yaw,
                data: _,
            } => {
                let name = self.types.get(kind as usize).map(String::as_str);
                // `new LightningBolt`: two ticks to show, one to three flashes.
                if name == Some("minecraft:lightning_bolt") {
                    let seed = self.random.next_long();
                    let flashes = self.random.next_int(3) + 1;
                    let pos = DVec3::from(pos);
                    let bolt = Bolt {
                        pos,
                        life: 2,
                        flashes,
                        seed,
                    };
                    self.bolts.insert(id, bolt);
                    return None;
                }
                let sort = match name.and_then(|n| n.strip_prefix("minecraft:")) {
                    Some("player") => Sort::Player,
                    Some("item") => Sort::Item,
                    Some("zombie") => Sort::Mob(MobType::Zombie),
                    Some("skeleton") => Sort::Mob(MobType::Skeleton),
                    Some("creeper") => Sort::Mob(MobType::Creeper),
                    Some("spider") => Sort::Mob(MobType::Spider),
                    Some("cow") => Sort::Mob(MobType::Cow),
                    Some("pig") => Sort::Mob(MobType::Pig),
                    Some("sheep") => Sort::Mob(MobType::Sheep),
                    Some("chicken") => Sort::Mob(MobType::Chicken),
                    Some("enderman") => Sort::Mob(MobType::Enderman),
                    Some("slime") => Sort::Mob(MobType::Slime),
                    Some("villager") => Sort::Mob(MobType::Villager),
                    Some("witch") => Sort::Mob(MobType::Witch),
                    Some("wolf") => Sort::Mob(MobType::Wolf),
                    Some("husk") => Sort::Mob(MobType::Husk),
                    Some("drowned") => Sort::Mob(MobType::Drowned),
                    Some("stray") => Sort::Mob(MobType::Stray),
                    Some("zombie_villager") => Sort::Mob(MobType::ZombieVillager),
                    Some("cave_spider") => Sort::Mob(MobType::CaveSpider),
                    Some("iron_golem") => Sort::Mob(MobType::IronGolem),
                    Some("squid") => Sort::Mob(MobType::Squid),
                    Some("glow_squid") => Sort::Mob(MobType::GlowSquid),
                    Some("phantom") => Sort::Mob(MobType::Phantom),
                    Some("cat") => Sort::Mob(MobType::Cat),
                    Some("ocelot") => Sort::Mob(MobType::Ocelot),
                    Some("fox") => Sort::Mob(MobType::Fox),
                    Some("bogged") => Sort::Mob(MobType::Bogged),
                    Some("snow_golem") => Sort::Mob(MobType::SnowGolem),
                    Some("magma_cube") => Sort::Mob(MobType::MagmaCube),
                    Some("blaze") => Sort::Mob(MobType::Blaze),
                    Some("ghast") => Sort::Mob(MobType::Ghast),
                    Some("silverfish") => Sort::Mob(MobType::Silverfish),
                    Some("endermite") => Sort::Mob(MobType::Endermite),
                    Some("polar_bear") => Sort::Mob(MobType::PolarBear),
                    Some("goat") => Sort::Mob(MobType::Goat),
                    Some("piglin") => Sort::Mob(MobType::Piglin),
                    Some("piglin_brute") => Sort::Mob(MobType::PiglinBrute),
                    Some("zombified_piglin") => Sort::Mob(MobType::ZombifiedPiglin),
                    Some("hoglin") => Sort::Mob(MobType::Hoglin),
                    Some("zoglin") => Sort::Mob(MobType::Zoglin),
                    Some("strider") => Sort::Mob(MobType::Strider),
                    Some("wither_skeleton") => Sort::Mob(MobType::WitherSkeleton),
                    Some("bat") => Sort::Mob(MobType::Bat),
                    Some("rabbit") => Sort::Mob(MobType::Rabbit),
                    Some("frog") => Sort::Mob(MobType::Frog),
                    Some("camel") => Sort::Mob(MobType::Camel),
                    Some("armadillo") => Sort::Mob(MobType::Armadillo),
                    Some("arrow") => Sort::Arrow { spectral: false },
                    Some("spectral_arrow") => Sort::Arrow { spectral: true },
                    Some("snowball") => Sort::Thrown {
                        item: "minecraft:snowball",
                        gravity: 0.03,
                    },
                    Some("egg") => Sort::Thrown {
                        item: "minecraft:egg",
                        gravity: 0.03,
                    },
                    Some("ender_pearl") => Sort::Thrown {
                        item: "minecraft:ender_pearl",
                        gravity: 0.03,
                    },
                    Some("experience_bottle") => Sort::Thrown {
                        item: "minecraft:experience_bottle",
                        gravity: 0.07,
                    },
                    Some("splash_potion") => Sort::Thrown {
                        item: "minecraft:splash_potion",
                        gravity: 0.05,
                    },
                    Some("lingering_potion") => Sort::Thrown {
                        item: "minecraft:lingering_potion",
                        gravity: 0.05,
                    },
                    // Not drawn yet; tracking it would do nothing.
                    _ => return None,
                };
                // `EntityType.updateInterval`: players 2, the rest of these 3. Items don't
                // interpolate (`InterpolationHandler.NO_OP`).
                let steps = if sort == Sort::Player { 2 } else { 3 };
                let place = Place {
                    pos: DVec3::from(pos),
                    yaw,
                    pitch,
                };
                // xorshift for a bob phase in 0..2π.
                self.seed ^= self.seed << 13;
                self.seed ^= self.seed >> 7;
                self.seed ^= self.seed << 17;
                let bob_offset =
                    (self.seed >> 40) as f32 / (1u64 << 24) as f32 * std::f32::consts::TAU;
                let mut tracked = Tracked::new(sort, uuid, place, head_yaw, steps);
                tracked.velocity = DVec3::from(velocity);
                tracked.bob_offset = bob_offset;
                // `new Squid`: `1 / (nextFloat() + 1) * 0.2`, from the same xorshift.
                tracked.anim.tentacle_speed =
                    1.0 / ((self.seed >> 20 & 0xFFFFFF) as f32 / (1u32 << 24) as f32 + 1.0) * 0.2;
                // `Rabbit.idleAnimationTimeout`'s first `random.nextInt(40) + 180`.
                tracked.anim.random = self.seed | 1;
                // A camel's starts at 0, so it idles at once.
                tracked.anim.idle_timeout = tracked.anim.next_int(40) + 180;
                if sort == Sort::Mob(MobType::Camel) {
                    tracked.anim.idle_timeout = 0;
                }
                tracked.id = id;
                self.tracked.insert(id, tracked);
            }
            Event::RemoveEntities(ids) => {
                for id in ids {
                    self.tracked.remove(&id);
                    self.bolts.remove(&id);
                }
            }
            // `ClientPacketListener.handleMoveEntity`.
            Event::MoveEntity {
                id,
                delta,
                rotation,
                on_ground,
            } => {
                if let Some(e) = self.tracked.get_mut(&id) {
                    let path = delta.map(|d| d.decode(e.base));
                    if let Some(path) = &path {
                        e.base = path.end();
                    }
                    e.move_to(path.as_ref(), rotation);
                    e.on_ground = on_ground;
                }
            }
            // `handleEntityPositionSync`: far moves snap.
            Event::SyncEntity {
                id,
                path,
                yaw,
                pitch,
                on_ground,
            } => {
                if let Some(e) = self.tracked.get_mut(&id) {
                    let end = path.end();
                    e.base = end;
                    if e.now.pos.distance_squared(DVec3::from(end)) > 4096.0 {
                        e.snap(DVec3::from(end), yaw, pitch);
                    } else {
                        e.move_to(Some(&path), Some((yaw, pitch)));
                    }
                    e.on_ground = on_ground;
                }
            }
            // `setValuesFromPositionPacket`, with `PositionMoveRotation.calculateAbsolute`'s
            // relative parts.
            Event::TeleportEntity {
                id,
                pos,
                velocity,
                yaw,
                pitch,
                relative,
                on_ground,
            } => {
                if let Some(e) = self.tracked.get_mut(&id) {
                    let bit = |b: i32| relative & 1 << b != 0;
                    let current = e.now.pos.to_array();
                    let pos: [f64; 3] = std::array::from_fn(|i| {
                        pos[i] + if bit(i as i32) { current[i] } else { 0.0 }
                    });
                    let yaw = yaw + if bit(3) { e.now.yaw } else { 0.0 };
                    let pitch = pitch + if bit(4) { e.now.pitch } else { 0.0 };
                    let old_velocity = e.velocity.to_array();
                    e.velocity = DVec3::from(std::array::from_fn::<f64, 3, _>(|i| {
                        velocity[i]
                            + if bit(5 + i as i32) {
                                old_velocity[i]
                            } else {
                                0.0
                            }
                    }));
                    if e.now.pos.distance_squared(DVec3::from(pos)) > 4096.0 {
                        e.snap(DVec3::from(pos), yaw, pitch);
                    } else {
                        e.move_to(Some(&Path::Linear(pos)), Some((yaw, pitch)));
                    }
                    e.on_ground = on_ground;
                }
            }
            // `handleRotateMob`: `LivingEntity.lerpHeadTo(yaw, 3)`.
            Event::HeadRotation { id, yaw } => {
                if let Some(e) = self.tracked.get_mut(&id) {
                    (e.head_target, e.head_steps) = (yaw, 3);
                }
            }
            Event::EntityMotion { id, velocity } => {
                if let Some(e) = self.tracked.get_mut(&id) {
                    e.velocity = DVec3::from(velocity);
                    // `AbstractArrow.lerpMotion`: set moving, it's no longer stuck.
                    if matches!(e.sort, Sort::Arrow { .. }) && e.velocity != DVec3::ZERO {
                        e.in_ground = false;
                    }
                }
            }
            // `handleAnimate`: `createTrackingEmitter`, which bursts once at once.
            Event::Crit { id, magic } => {
                use crate::particles::Kind;
                let kind = if magic {
                    Kind::EnchantedHit
                } else {
                    Kind::Crit
                };
                self.emitters.push((id, kind, 3));
                self.emit();
            }
            // `handleParticleEvent`.
            Event::Particles {
                name,
                override_limiter,
                always_show,
                pos,
                dist,
                speed,
                count,
                alternative,
            } => {
                use crate::particles::Kind;
                let kind = match name.as_str() {
                    "minecraft:crit" => Kind::Crit,
                    "minecraft:enchanted_hit" => Kind::EnchantedHit,
                    "minecraft:squid_ink" | "minecraft:glow_squid_ink" => Kind::Ink,
                    _ => Kind::Damage,
                };
                let glow = name == "minecraft:glow_squid_ink";
                // `ParticleEngine.createParticle` by kind.
                let spawn = |p: &mut crate::particles::Particles, at: DVec3, aux: DVec3| {
                    if kind == Kind::Ink {
                        p.ink(at, aux, glow);
                    } else {
                        p.crit(kind, at, aux);
                    }
                };
                let (pos, dist, speed) = (
                    DVec3::from(pos),
                    DVec3::from(dist.map(f64::from)),
                    DVec3::from(speed.map(f64::from)),
                );
                let p = &mut self.particles;
                if count == 0 && p.allowed(override_limiter, always_show) {
                    spawn(p, pos, speed * dist);
                }
                for _ in 0..count {
                    if !p.allowed(override_limiter, always_show) {
                        continue;
                    }
                    if alternative != 0 {
                        let mut r = || f64::from(p.random());
                        let offset = DVec3::new(r(), r(), r()) * dist;
                        let mut aux = speed;
                        if alternative == 2 {
                            aux *= DVec3::new(r(), r(), r());
                        }
                        spawn(p, pos + offset, aux);
                    } else {
                        let offset = DVec3::new(p.gaussian(), p.gaussian(), p.gaussian()) * dist;
                        let aux = DVec3::new(p.gaussian(), p.gaussian(), p.gaussian()) * speed;
                        spawn(p, pos + offset, aux);
                    }
                }
            }
            // `handleSwingAnimation`.
            Event::Swing {
                id,
                hand,
                kind,
                duration,
            } => {
                if let Some(e) = self.tracked.get_mut(&id) {
                    e.swing.start_if_able((hand, kind, duration));
                }
            }
            Event::Equipment { id, slots } => {
                if let Some(e) = self.tracked.get_mut(&id) {
                    for (slot, item) in slots {
                        match item {
                            Some(item) => e.equipment.insert(slot, item),
                            None => e.equipment.remove(&slot),
                        };
                    }
                }
            }
            Event::EntityData { id, values } => {
                if let Some(e) = self.tracked.get_mut(&id) {
                    // `AbstractArrow.onSyncedDataUpdated`: stuck after its first tick, it
                    // quivers.
                    for (index, value) in &values {
                        if let (IN_GROUND, DataValue::Bool(stuck), Sort::Arrow { .. }) =
                            (*index, value, e.sort)
                        {
                            if *stuck && e.age > 0 && e.shake <= 0 {
                                e.shake = 7;
                            }
                            e.in_ground = *stuck;
                        }
                    }
                    e.data_changed(&values);
                    e.data.extend(values);
                }
            }
            // `Snowball` and `ThrownEgg.handleEntityEvent(3)`: it breaks into 8 bits of its
            // item, a snowball's dropping and an egg's scattering a little.
            Event::EntityEvent { id, event: 3 }
                if self
                    .tracked
                    .get(&id)
                    .is_some_and(|e| matches!(e.sort, Sort::Thrown { .. })) =>
            {
                let e = &self.tracked[&id];
                let item = e.item_shown();
                let egg = item == "minecraft:egg";
                if !egg && item != "minecraft:snowball" {
                    return None;
                }
                let rect = crate::particles::item_sprite(&self.icons, &item)?;
                let p = &mut self.particles;
                for _ in 0..8 {
                    let mut r = || f64::from(p.random() - 0.5) * 0.08;
                    let velocity = if egg {
                        DVec3::new(r(), r(), r())
                    } else {
                        DVec3::ZERO
                    };
                    if p.allowed(false, false) {
                        p.item(rect, e.now.pos, velocity);
                    }
                }
            }
            // `LivingEntity.handleDamageEvent` and `animateHurt`: it flashes red. The
            // player's own pass on.
            Event::Damaged { id, .. } | Event::HurtAnimation { id, .. }
                if self.tracked.contains_key(&id) =>
            {
                self.tracked.get_mut(&id).expect("checked").hurt_time = 10;
            }
            // `ClientPacketListener.handleUpdateAttributes`: only max health matters here,
            // clamped to `Attributes.MAX_HEALTH`'s range.
            Event::Attributes { id, values } if self.tracked.contains_key(&id) => {
                let e = self.tracked.get_mut(&id).expect("checked");
                for (name, _, value) in values {
                    if name == "minecraft:max_health" {
                        e.max_health = Some(value.clamp(1.0, 1024.0) as f32);
                    }
                }
            }
            // `LivingEntity.handleEntityEvent(60)`: the mob's gone in a poof, just before
            // the server removes it.
            // `Arrow.handleEntityEvent(0)`: a tipped arrow's potion has worn off, in a
            // burst of 20 swirls.
            Event::EntityEvent { id, event: 0 }
                if self
                    .tracked
                    .get(&id)
                    .is_some_and(|e| e.sort == Sort::Arrow { spectral: false }) =>
            {
                let e = &self.tracked[&id];
                if let Some(color) = e.arrow_color() {
                    for _ in 0..20 {
                        let at = e.random_point(&mut self.particles);
                        if self.particles.allowed(false, false) {
                            self.particles.effect(at, DVec3::ZERO, color | 0xff00_0000);
                        }
                    }
                }
            }
            // `IronGolem.handleEntityEvent(4)`: its arms come down in an attack.
            Event::EntityEvent { id, event: 4 }
                if self.tracked.get(&id).is_some_and(|e| {
                    matches!(
                        e.sort,
                        Sort::Mob(MobType::IronGolem | MobType::Hoglin | MobType::Zoglin)
                    )
                }) =>
            {
                self.tracked
                    .get_mut(&id)
                    .expect("checked")
                    .anim
                    .golem_attack = 10;
            }
            // `Rabbit.handleEntityEvent(1)`: a hop starts, kicking up dust.
            Event::EntityEvent { id, event: 1 }
                if self
                    .tracked
                    .get(&id)
                    .is_some_and(|e| e.sort == Sort::Mob(MobType::Rabbit)) =>
            {
                let a = &mut self.tracked.get_mut(&id).expect("checked").anim;
                (a.jump_duration, a.jump_ticks, a.hop_dust) = (15, 0, true);
            }
            // `Armadillo.handleEntityEvent(64)`: it peeks out of its shell again.
            Event::EntityEvent { id, event: 64 }
                if self
                    .tracked
                    .get(&id)
                    .is_some_and(|e| e.sort == Sort::Mob(MobType::Armadillo)) =>
            {
                self.tracked.get_mut(&id).expect("checked").anim.peeked = true;
            }
            // `Squid.handleEntityEvent(19)`: the next stroke starts.
            Event::EntityEvent { id, event: 19 }
                if self.tracked.get(&id).is_some_and(|e| {
                    matches!(e.sort, Sort::Mob(MobType::Squid | MobType::GlowSquid))
                }) =>
            {
                self.tracked
                    .get_mut(&id)
                    .expect("checked")
                    .anim
                    .tentacle_movement = 0.0;
            }
            Event::EntityEvent { id, event: 60 } => {
                if let Some(e) = self.tracked.get(&id) {
                    let baby = e.baby();
                    self.particles.poof(e.now.pos, e.size(baby));
                }
            }
            other => return Some(other),
        }
        None
    }

    /// `TrackingEmitter.tick` for each emitter: 16 tries at a spark from inside a sphere
    /// around the entity's middle, thrown outward and up; gone after three bursts or when
    /// its entity is.
    fn emit(&mut self) {
        let (tracked, particles) = (&self.tracked, &mut self.particles);
        self.emitters.retain_mut(|(id, kind, left)| {
            let Some(e) = tracked.get(id) else {
                return false;
            };
            let baby = e.baby();
            let [width, height] = e.size(baby).map(f64::from);
            for _ in 0..16 {
                let mut r = || f64::from(particles.random() * 2.0 - 1.0);
                let (xa, ya, za) = (r(), r(), r());
                if xa * xa + ya * ya + za * za > 1.0 {
                    continue;
                }
                // `Entity.getX(scale)` and `getY(scale)`: scaled by the box.
                let at = e.now.pos
                    + DVec3::new(
                        width * xa / 4.0,
                        height * (0.5 + ya / 4.0),
                        width * za / 4.0,
                    );
                if particles.allowed(false, false) {
                    particles.crit(*kind, at, DVec3::new(xa, ya + 0.2, za));
                }
            }
            *left -= 1;
            *left > 0
        });
    }

    /// One game tick for every entity, against the world's blocks. Returns whether a
    /// lightning bolt lights the sky (`setSkyFlashTime(2)`).
    pub fn tick(
        &mut self,
        world: &impl mc_physics::World,
        blocks: &mc_physics::BlockPhysics,
    ) -> bool {
        for e in self.tracked.values_mut() {
            e.tick(
                world,
                blocks,
                &self.uses,
                &mut self.particles,
                self.game_time,
            );
        }
        self.particles.tick(world, blocks);
        self.emit();
        // `LightningBolt.tick`: a bolt shows for its life, then may flash again after up
        // to ten ticks with a new shape, and goes once its flashes run out.
        let random = &mut self.random;
        let mut flash = false;
        let thunder = &mut self.thunder;
        self.bolts.retain(|_, bolt| {
            // A new bolt thunders as it first ticks.
            if bolt.life == 2 {
                let thunder_pitch = 0.8 + random.next_float() * 0.2;
                let impact_pitch = 0.5 + random.next_float() * 0.2;
                thunder.push((bolt.pos, thunder_pitch, impact_pitch));
            }
            bolt.life -= 1;
            if bolt.life < 0 {
                if bolt.flashes == 0 {
                    return false;
                } else if bolt.life < -random.next_int(10) {
                    bolt.flashes -= 1;
                    bolt.life = 1;
                    bolt.seed = random.next_long();
                }
            }
            flash |= bolt.life >= 0;
            true
        });
        flash
    }

    /// `ProjectileUtil.getEntityHitResult` for `EntitySelector.CAN_BE_PICKED`: the living
    /// entity whose box the ray from `from` to `to` enters first (or that `from` is inside),
    /// with where it's hit, if that's nearer than `max_sq` squared blocks; and whether it's
    /// alive.
    pub fn pick(&self, from: DVec3, to: DVec3, max_sq: f64) -> Option<(i32, DVec3, bool)> {
        let mut nearest = max_sq;
        let mut found = None;
        for (&id, e) in &self.tracked {
            if !e.living() {
                continue;
            }
            let baby = e.baby();
            let [width, height] = e.size(baby);
            let bb = mc_physics::Aabb::at_feet(e.now.pos, f64::from(width), f64::from(height));
            let clip = bb.clip(from, to).map(|(t, _)| from + (to - from) * t);
            let alive = e.death_time == 0;
            if bb.contains(from) {
                if nearest >= 0.0 {
                    found = Some((id, clip.unwrap_or(from), alive));
                    nearest = 0.0;
                }
            } else if let Some(p) = clip {
                let distance = from.distance_squared(p);
                if distance < nearest || nearest == 0.0 {
                    found = Some((id, p, alive));
                    nearest = distance;
                }
            }
        }
        found
    }

    /// The lightning bolts to draw (`LightningBoltRenderer`), while each shows.
    /// Where an entity is now, if it's in view.
    pub fn position(&self, id: i32) -> Option<DVec3> {
        self.tracked.get(&id).map(|t| t.now.pos)
    }

    pub fn lightning(&self) -> Vec<mc_render::mesh::Vertex> {
        self.bolts
            .values()
            .flat_map(|bolt| mc_render::weather::lightning(bolt.pos, bolt.seed))
            .collect()
    }

    /// The Particles option.
    pub fn set_particle_status(&mut self, status: mc_render::hud::options::Particles) {
        self.particles.status = status;
    }

    /// `Entity.canSpawnSprintParticle` for each other entity: where those sprinting (shared
    /// flag 3), not crouching (flag 1), alive and out of water and lava stand, how they
    /// move, and how wide they are, for `spawnSprintParticle`; and rabbits that hopped
    /// since this was last called (`Rabbit.handleEntityEvent(1)`).
    pub fn sprinters(
        &mut self,
        world: &impl mc_physics::World,
        blocks: &mc_physics::BlockPhysics,
    ) -> Vec<(DVec3, DVec3, f64)> {
        let mut hops = Vec::new();
        for e in self.tracked.values_mut() {
            if std::mem::take(&mut e.anim.hop_dust) {
                hops.push((e.now.pos, e.velocity, f64::from(e.size(e.baby())[0])));
            }
        }
        hops.extend(self.sprinting(world, blocks));
        hops
    }

    fn sprinting<'a>(
        &'a self,
        world: &'a impl mc_physics::World,
        blocks: &'a mc_physics::BlockPhysics,
    ) -> impl Iterator<Item = (DVec3, DVec3, f64)> + 'a {
        self.tracked
            .values()
            .filter(|e| e.living())
            .filter(|e| matches!(e.data.get(&SHARED_FLAGS), Some(DataValue::Byte(f)) if f & 0x08 != 0 && f & 0x02 == 0))
            .filter(|e| !matches!(e.data.get(&HEALTH), Some(DataValue::Float(h)) if *h <= 0.0))
            .filter(|e| {
                let at = e.now.pos.floor().as_ivec3().to_array();
                mc_physics::fluid::fluid_at(world, blocks, at).is_none()
            })
            .map(|e| {
                let baby = e.baby();
                (e.now.pos, e.velocity, f64::from(e.size(baby)[0]))
            })
    }

    /// `LivingEntity.tickEffects` for the player itself, with the effect colours, ambience
    /// and invisibility the server gave it, its feet at `feet` and its box `size`.
    pub fn own_effects(
        &mut self,
        colors: &[u32],
        invisible: bool,
        ambient: bool,
        feet: DVec3,
        size: [f32; 2],
    ) {
        self.particles
            .mob_effects(colors, invisible, ambient, feet, size);
    }

    /// The particles to draw, `partial` of the way to the next tick, lit by `light`.
    pub fn particles(
        &self,
        partial: f32,
        light: impl Fn(DVec3) -> [u8; 2],
    ) -> Vec<mc_render::particle::Particle> {
        self.particles.render(partial, light)
    }

    /// What to draw, `partial` of the way from the last tick to the next, with the sky and
    /// block `light` at each entity's eyes (`EntityRenderer.getPackedLightCoords`); one on
    /// fire has full block light.
    pub fn render(&self, partial: f32, light: impl Fn(DVec3) -> [u8; 2]) -> Vec<Lit> {
        self.tracked
            .values()
            .filter_map(|e| self.lit(e, partial, &light))
            .collect()
    }

    fn lit(&self, e: &Tracked, partial: f32, light: impl Fn(DVec3) -> [u8; 2]) -> Option<Lit> {
        let entity = e.render(partial, self)?;
        let (feet, baby) = match &entity {
            Entity::Living(l) => (l.pos, l.baby),
            Entity::Item(d) => (d.pos, false),
            Entity::Arrow(a) => (a.pos, false),
            Entity::Thrown(t) => (t.pos, false),
        };
        let eye = e.eye_height() * if baby { 0.5 } else { 1.0 };
        let [sky, mut block] = light(feet.as_dvec3() + DVec3::Y * eye);
        // `Entity.isOnFire`: bit 0 of the shared flags.
        if matches!(e.data.get(&SHARED_FLAGS), Some(DataValue::Byte(f)) if f & 1 != 0) {
            block = 15;
        }
        // `BlazeRenderer` and `MagmaCubeRenderer.getBlockLightLevel`.
        if matches!(e.sort, Sort::Mob(MobType::Blaze | MobType::MagmaCube)) {
            block = 15;
        }
        // `GlowSquidRenderer.getBlockLightLevel`: it glows at 15, dimming for ten ticks
        // after it's hurt.
        if e.sort == Sort::Mob(MobType::GlowSquid) {
            let dark = match e.data.get(&DARK_TICKS) {
                Some(DataValue::Int(d)) => *d as f32,
                _ => 0.0,
            };
            let glow = (15.0 * (1.0 - dark / 10.0)).clamp(0.0, 15.0) as u8;
            block = if glow == 15 { 15 } else { glow.max(block) };
        }
        Some((entity, [sky, block]))
    }

    /// `ClientLevel.tickEntities` for the player itself, after it moved from `own.old` to
    /// `own.pos`: `LivingEntity.tick` turns its body and swings its legs as it does for
    /// other players, and `Player.serverAiStep` turns its head with its view.
    pub(crate) fn tick_own(&mut self, uuid: u128, own: Own) {
        let place = Place {
            pos: own.pos,
            yaw: own.yaw,
            pitch: own.pitch,
        };
        let e = self
            .own
            .get_or_insert_with(|| Tracked::new(Sort::Player, uuid, place, own.yaw, 0));
        e.uuid = uuid;
        e.old = Place {
            pos: own.old,
            ..e.now
        };
        e.now = place;
        e.old_head_yaw = e.head_yaw;
        e.old_body_yaw = e.body_yaw;
        e.head_yaw = own.yaw;
        e.on_ground = own.on_ground;
        e.age += 1;
        e.data.insert(SHARED_FLAGS, DataValue::Byte(own.flags));
        e.data.insert(POSE, DataValue::Int(own.pose));
        e.data.insert(HEALTH, DataValue::Float(own.health));
        if own.health > 0.0 {
            e.death_time = 0;
        }
        e.tick_living();
        e.swing = own.swing;
        e.hurt_time = own.hurt_time;
        e.using = own.using;
        e.equipment = own
            .equipment
            .into_iter()
            .enumerate()
            .filter_map(|(slot, item)| {
                Some((
                    slot as u8,
                    mc_protocol::Equipped {
                        item: item?,
                        dye: None,
                        charged: false,
                    },
                ))
            })
            .collect();
    }

    /// The player itself, `partial` of the way to the next tick and looking down at
    /// `pitch`, lit as other entities are; without the name tag the camera's own entity
    /// never shows (`EntityRenderer.shouldShowName`).
    pub fn render_own(
        &self,
        partial: f32,
        pitch: f32,
        light: impl Fn(DVec3) -> [u8; 2],
    ) -> Option<Lit> {
        let (mut entity, light) = self.lit(self.own.as_ref()?, partial, light)?;
        if let Entity::Living(l) = &mut entity {
            l.name_tag = None;
            l.pitch = pitch;
        }
        Some((entity, light))
    }
}

/// The player itself as one tick left it, for `Entities::tick_own`.
pub(crate) struct Own {
    /// Its feet the tick before and now.
    pub old: DVec3,
    pub pos: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    /// As `Entity.DATA_SHARED_FLAGS_ID`: on fire 1, crouching 2, sprinting 8, invisible
    /// 32.
    pub flags: u8,
    /// `Pose`'s id.
    pub pose: i32,
    pub health: f32,
    /// What it holds and wears, by `EquipmentSlot` id: main hand, off hand, feet, legs,
    /// chest and head.
    pub equipment: [Option<String>; 6],
    pub swing: SwingState,
    pub hurt_time: i32,
    /// The item it's using: its hand, the item and the ticks left.
    pub using: Option<(u8, String, i32)>,
}

impl Tracked {
    /// `LivingEntity.recreateFromPacket`: body and head both face the head's way.
    fn new(sort: Sort, uuid: u128, place: Place, head_yaw: f32, steps: i32) -> Self {
        Tracked {
            id: 0,
            sort,
            uuid,
            base: place.pos.to_array(),
            now: place,
            old: place,
            interpolation: Interpolation::new(steps),
            velocity: DVec3::ZERO,
            on_ground: false,
            head_yaw,
            old_head_yaw: head_yaw,
            body_yaw: head_yaw,
            old_body_yaw: head_yaw,
            head_target: head_yaw,
            head_steps: 0,
            head_stable_time: 0,
            last_stable_head_yaw: head_yaw,
            walk_speed_old: 0.0,
            walk_speed: 0.0,
            walk_pos: 0.0,
            age: 0,
            bob_offset: 0.0,
            flap: [0.0; 2],
            flap_speed: [0.0; 2],
            flapping: 1.0,
            squish: [0.0; 2],
            target_squish: 0.0,
            was_on_ground: false,
            data: HashMap::new(),
            equipment: HashMap::new(),
            swing: SwingState::default(),
            hurt_time: 0,
            death_time: 0,
            max_health: None,
            anim: MobAnim::default(),
            using: None,
            shake: 0,
            in_ground: false,
            in_ground_time: 0,
        }
    }

    /// `EntityType`'s eye height, where light is read; types that don't set one have 85%
    /// of their height.
    fn eye_height(&self) -> f64 {
        match self.sort {
            // `Avatar.POSES`: lower while crouching, swimming, fall flying, spin attacking
            // or sleeping.
            Sort::Player => match self.pose() {
                CROUCHING => 1.27,
                1 | 3 | 4 => 0.4,
                2 => 0.2,
                _ => 1.62,
            },
            Sort::Item | Sort::Thrown { .. } => 0.2125,
            Sort::Arrow { .. } => 0.13,
            Sort::Mob(mob) => match mob {
                MobType::Zombie
                | MobType::Skeleton
                | MobType::Husk
                | MobType::Drowned
                | MobType::Stray
                | MobType::ZombieVillager => 1.74,
                MobType::CaveSpider => 0.45,
                MobType::IronGolem => 2.7 * 0.85,
                MobType::Squid | MobType::GlowSquid => 0.4,
                MobType::Phantom => 0.175 * (1.0 + 0.15 * f64::from(self.phantom_size())),
                MobType::Cat => 0.35,
                MobType::Ocelot => 0.7 * 0.85,
                MobType::Fox => 0.4,
                MobType::Bogged => 1.74,
                MobType::SnowGolem => 1.7,
                MobType::MagmaCube => 0.325 * f64::from(self.slime_size()),
                MobType::Blaze => 1.8 * 0.85,
                MobType::Ghast => 2.6,
                MobType::Silverfish | MobType::Endermite => 0.13,
                MobType::PolarBear => 1.4 * 0.85,
                MobType::Goat => 1.3 * 0.85,
                MobType::Piglin | MobType::PiglinBrute | MobType::ZombifiedPiglin => 1.79,
                MobType::Hoglin | MobType::Zoglin => 1.4 * 0.85,
                MobType::Strider => 1.7 * 0.85,
                MobType::WitherSkeleton => 2.1,
                MobType::Bat => 0.45,
                MobType::Rabbit => 0.59,
                MobType::Frog => 0.5 * 0.85,
                // `Camel.ADULT_SITTING_DIMENSIONS` while it sits.
                MobType::Camel if self.pose() == SITTING => 0.845,
                MobType::Camel => 2.275,
                MobType::Armadillo => 0.26,
                MobType::Enderman => 2.55,
                MobType::Slime => 0.325 * f64::from(self.slime_size()),
                MobType::Villager | MobType::Witch => 1.62,
                MobType::Wolf => 0.68,
                MobType::Creeper => 1.7 * 0.85,
                MobType::Spider => 0.65,
                MobType::Cow => 1.3,
                MobType::Pig => 0.9 * 0.85,
                MobType::Sheep => 1.235,
                MobType::Chicken => 0.644,
            },
        }
    }

    /// `Entity.displayFireAnimation`: the bounding box's size while it's on fire
    /// (`isOnFire`): shared flag 0, unless it's immune to fire. A blaze is on fire while
    /// it's charged.
    fn burning(&self, baby: bool) -> Option<[f32; 2]> {
        let byte = |i: u8| match self.data.get(&i) {
            Some(DataValue::Byte(f)) => *f,
            _ => 0,
        };
        let on_fire = match self.sort {
            Sort::Mob(MobType::Blaze) => byte(MONSTER_DATA) & 1 != 0,
            // `EntityType.fireImmune`.
            Sort::Mob(
                MobType::MagmaCube
                | MobType::Ghast
                | MobType::ZombifiedPiglin
                | MobType::Zoglin
                | MobType::Strider
                | MobType::WitherSkeleton,
            ) => false,
            _ => byte(SHARED_FLAGS) & 1 != 0,
        };
        on_fire.then(|| self.size(baby))
    }

    /// The bounding box's width and height (`getBbWidth` and `getBbHeight`).
    fn size(&self, baby: bool) -> [f32; 2] {
        match self.sort {
            // `Avatar.POSES`, and `LivingEntity.SLEEPING_DIMENSIONS`.
            Sort::Player => match self.pose() {
                CROUCHING => [0.6, 1.5],
                1 | 3 | 4 => [0.6, 0.6],
                2 | 7 => [0.2, 0.2],
                _ => [0.6, 1.8],
            },
            // `EntityTypes`' sizes, and each type's `BABY_DIMENSIONS`.
            Sort::Item | Sort::Thrown { .. } => [0.25, 0.25],
            Sort::Arrow { .. } => [0.5, 0.5],
            Sort::Mob(mob) => match (mob, baby) {
                (
                    MobType::Zombie | MobType::Husk | MobType::Drowned | MobType::ZombieVillager,
                    false,
                ) => [0.6, 1.95],
                (
                    MobType::Zombie | MobType::Husk | MobType::Drowned | MobType::ZombieVillager,
                    true,
                ) => [0.49, 0.98],
                (MobType::CaveSpider, _) => [0.7, 0.5],
                (MobType::IronGolem, _) => [1.4, 2.7],
                (MobType::Squid | MobType::GlowSquid, false) => [0.8, 0.8],
                (MobType::Squid | MobType::GlowSquid, true) => [0.5, 0.5],
                (MobType::Phantom, _) => [0.9, 0.5].map(|d| d * (1.0 + 0.15 * self.phantom_size())),
                (MobType::Cat | MobType::Ocelot | MobType::Fox, false) => [0.6, 0.7],
                (MobType::Cat | MobType::Ocelot, true) => [0.3, 0.35],
                (MobType::Fox, true) => [0.36, 0.42],
                (MobType::Bogged, _) => [0.6, 1.99],
                (MobType::SnowGolem, _) => [0.7, 1.9],
                (MobType::MagmaCube, _) => [0.52 * self.slime_size(); 2],
                (MobType::Blaze, _) => [0.6, 1.8],
                (MobType::Ghast, _) => [4.0, 4.0],
                (MobType::Silverfish | MobType::Endermite, _) => [0.4, 0.3],
                (MobType::PolarBear, false) => [1.4, 1.4],
                (MobType::PolarBear, true) => [0.7, 0.7],
                (MobType::Goat, false) => [0.9, 1.3],
                (MobType::Goat, true) => [0.45, 0.65],
                (MobType::Piglin | MobType::PiglinBrute | MobType::ZombifiedPiglin, false) => {
                    [0.6, 1.95]
                }
                (MobType::Piglin | MobType::PiglinBrute | MobType::ZombifiedPiglin, true) => {
                    [0.49, 0.98]
                }
                (MobType::Hoglin | MobType::Zoglin, false) => [1.3964844, 1.4],
                (MobType::Hoglin | MobType::Zoglin, true) => [0.75, 0.85],
                (MobType::Strider, false) => [0.9, 1.7],
                (MobType::Strider, true) => [0.45, 0.85],
                (MobType::WitherSkeleton, _) => [0.7, 2.4],
                (MobType::Bat, _) => [0.5, 0.9],
                (MobType::Rabbit, false) => [0.49, 0.6],
                (MobType::Rabbit, true) => [0.24, 0.4],
                (MobType::Frog, _) => [0.5, 0.5],
                (MobType::Camel, false) if self.pose() == SITTING => [1.7, 2.375 - 1.43],
                (MobType::Camel, true) if self.pose() == SITTING => [0.95, 0.425],
                (MobType::Camel, false) => [1.7, 2.375],
                (MobType::Camel, true) => [0.95, 1.4],
                (MobType::Armadillo, false) => [0.7, 0.65],
                (MobType::Armadillo, true) => [0.7 * 0.6, 0.65 * 0.6],
                (MobType::Skeleton | MobType::Stray, _) => [0.6, 1.99],
                (MobType::Enderman, _) => [0.6, 2.9],
                (MobType::Slime, _) => [0.52 * self.slime_size(); 2],
                (MobType::Villager, false) | (MobType::Witch, _) => [0.6, 1.95],
                (MobType::Villager, true) => [0.3, 0.975],
                (MobType::Wolf, false) => [0.6, 0.85],
                (MobType::Wolf, true) => [0.3, 0.425],
                (MobType::Creeper, _) => [0.6, 1.7],
                (MobType::Spider, _) => [1.4, 0.9],
                (MobType::Cow, false) => [0.9, 1.4],
                (MobType::Cow, true) => [0.45, 0.7],
                (MobType::Pig, false) => [0.9, 0.9],
                (MobType::Pig, true) => [0.45, 0.45],
                (MobType::Sheep, false) => [0.9, 1.3],
                (MobType::Sheep, true) => [0.45, 0.65],
                (MobType::Chicken, false) => [0.4, 0.7],
                (MobType::Chicken, true) => [0.3, 0.4],
            },
        }
    }

    /// `isBaby`: `AgeableMob`'s and `Zombie`'s `DATA_BABY_ID` are both 16; a piglin's
    /// comes after its zombification immunity, and a brute is never one.
    fn baby(&self) -> bool {
        let index = match self.sort {
            Sort::Mob(MobType::Piglin) => PIGLIN_BABY,
            Sort::Mob(MobType::PiglinBrute) => return false,
            _ => BABY,
        };
        matches!(self.data.get(&index), Some(DataValue::Bool(true)))
    }

    /// `Phantom.getPhantomSize`.
    fn phantom_size(&self) -> f32 {
        match self.data.get(&PHANTOM_SIZE) {
            Some(DataValue::Int(size)) => *size as f32,
            _ => 0.0,
        }
    }

    /// `AbstractCubeMob.getSize`.
    fn slime_size(&self) -> f32 {
        match self.data.get(&SLIME_SIZE) {
            Some(DataValue::Int(size)) => *size as f32,
            _ => 1.0,
        }
    }

    fn living(&self) -> bool {
        matches!(self.sort, Sort::Player | Sort::Mob(_))
    }

    /// `Entity.moveOrInterpolateTo`: living entities glide there, others jump.
    fn move_to(&mut self, path: Option<&Path>, rotation: Option<(f32, f32)>) {
        if self.living() {
            self.interpolation.interpolate_to(self.now, path, rotation);
        } else {
            if let Some(path) = path {
                self.now.pos = DVec3::from(path.end());
            }
            if let Some((yaw, pitch)) = rotation {
                (self.now.yaw, self.now.pitch) = (yaw, pitch);
            }
        }
    }

    /// `Entity.snapTo`: there at once, with nothing to draw it moving from.
    fn snap(&mut self, pos: DVec3, yaw: f32, pitch: f32) {
        self.now = Place { pos, yaw, pitch };
        self.old = self.now;
        self.interpolation.cancel();
    }

    fn tick(
        &mut self,
        world: &impl mc_physics::World,
        blocks: &mc_physics::BlockPhysics,
        uses: &HashMap<String, (mc_render::entity::UseAnim, i32)>,
        particles: &mut crate::particles::Particles,
        game_time: i64,
    ) {
        // `Entity.commonTick`.
        self.old = self.now;
        self.old_head_yaw = self.head_yaw;
        self.old_body_yaw = self.body_yaw;
        if self.interpolation.active() {
            self.now = self.interpolation.advance();
        } else {
            self.interpolation.cancel();
        }
        self.age += 1;
        match self.sort {
            Sort::Item => self.tick_item(world, blocks),
            Sort::Arrow { .. } => self.tick_arrow(world, blocks, particles),
            Sort::Thrown { gravity, .. } => self.tick_thrown(world, blocks, gravity),
            Sort::Player | Sort::Mob(_) => {
                self.tick_living();
                self.tick_mob(world, blocks, game_time);
                self.tick_effects(particles);
                self.tick_using(uses);
            }
        }
        // `GlowSquid.aiStep`: a glow every tick somewhere about it (`getRandomX(0.6)`).
        if self.sort == Sort::Mob(MobType::GlowSquid) {
            let [width, height] = self.size(self.baby());
            let at = particles.in_box(self.now.pos, [width * 1.2, height]);
            if particles.allowed(false, false) {
                particles.glow(at);
            }
        }
        // `Arrow.tick` on the client: a tipped arrow trails two swirls a tick in flight, and
        // one every five ticks once stuck.
        if self.sort == (Sort::Arrow { spectral: false })
            && let Some(color) = self.arrow_color()
        {
            let count = match self.in_ground {
                true if self.in_ground_time % 5 == 0 => 1,
                true => 0,
                false => 2,
            };
            for _ in 0..count {
                let at = self.random_point(particles);
                if particles.allowed(false, false) {
                    particles.effect(at, DVec3::ZERO, color);
                }
            }
        }
    }

    /// `LivingEntity.tickEffects` on the client, from its effects' particles (data 10),
    /// whether they're all ambient (data 11) and whether it's invisible (shared flag 5).
    fn tick_effects(&self, particles: &mut crate::particles::Particles) {
        let Some(DataValue::Effects(colors)) = self.data.get(&EFFECT_PARTICLES) else {
            return;
        };
        let invisible =
            matches!(self.data.get(&SHARED_FLAGS), Some(DataValue::Byte(f)) if f & 0x20 != 0);
        let ambient = matches!(self.data.get(&EFFECT_AMBIENCE), Some(DataValue::Bool(true)));
        let baby = self.baby();
        particles.mob_effects(colors, invisible, ambient, self.now.pos, self.size(baby));
    }

    /// Somewhere in its box (`getRandomX(0.5)`, `getRandomY` and `getRandomZ(0.5)`).
    fn random_point(&self, particles: &mut crate::particles::Particles) -> DVec3 {
        particles.in_box(self.now.pos, self.size(false))
    }

    /// `Arrow.getColor`: a tipped arrow's potion colour (`ID_EFFECT_COLOR`), or none.
    fn arrow_color(&self) -> Option<u32> {
        match self.data.get(&ARROW_COLOR) {
            Some(DataValue::Int(c)) if *c != -1 => Some(*c as u32),
            _ => None,
        }
    }

    /// `LivingEntity.onSyncedDataUpdated` and `updatingUsingItem` for another entity: its
    /// living flags (data 8) start using the item in the hand they name, for its use
    /// duration, and stop it; it counts down while that item stays in the hand.
    fn tick_using(&mut self, uses: &HashMap<String, (mc_render::entity::UseAnim, i32)>) {
        let flags = match self.data.get(&LIVING_FLAGS) {
            Some(DataValue::Byte(f)) => *f,
            _ => 0,
        };
        if flags & 1 == 0 {
            self.using = None;
            return;
        }
        let hand = u8::from(flags & 2 != 0);
        let held = self.equipment.get(&hand).map(|e| e.item.clone());
        match (&mut self.using, held) {
            (Some((_, item, remaining)), Some(held)) if *item == held => *remaining -= 1,
            (Some(_), _) => self.using = None,
            (None, Some(held)) => {
                let duration = uses.get(&held).map_or(0, |u| u.1);
                self.using = Some((hand, held, duration - 1));
            }
            (None, None) => {}
        }
    }

    /// `LivingEntity.tick` and `aiStep` as they run for another client's entity.
    fn tick_living(&mut self) {
        // `LivingEntity.baseTick`: the hurt countdown, and `tickDeath` once its health
        // (data 9) is gone.
        self.hurt_time = (self.hurt_time - 1).max(0);
        if matches!(self.data.get(&HEALTH), Some(DataValue::Float(h)) if *h <= 0.0) {
            self.death_time += 1;
        }
        self.swing.tick();
        if self.head_steps > 0 {
            self.head_yaw = rot_lerp(
                1.0 / self.head_steps as f32,
                self.head_yaw,
                self.head_target,
            );
            self.head_steps -= 1;
        }
        if matches!(self.sort, Sort::Mob(MobType::Slime | MobType::MagmaCube)) {
            // `AbstractCubeMob.tick`: squashed as it lands, stretched as it leaves the
            // ground, easing back.
            self.squish[0] = self.squish[1];
            self.squish[1] += (self.target_squish - self.squish[1]) * 0.5;
            if self.on_ground && !self.was_on_ground {
                self.target_squish = -0.5;
            } else if !self.on_ground && self.was_on_ground {
                self.target_squish = 1.0;
            }
            self.was_on_ground = self.on_ground;
            // `decreaseSquish`: a magma cube's eases back slower.
            self.target_squish *= if self.sort == Sort::Mob(MobType::Slime) {
                0.6
            } else {
                0.9
            };
        }
        if self.sort == Sort::Mob(MobType::Chicken) {
            // `Chicken.aiStep`.
            self.flap[0] = self.flap[1];
            self.flap_speed[0] = self.flap_speed[1];
            let change = if self.on_ground { -1.0 } else { 4.0 };
            self.flap_speed[1] = (self.flap_speed[1] + change * 0.3).clamp(0.0, 1.0);
            if !self.on_ground && self.flapping < 1.0 {
                self.flapping = 1.0;
            }
            self.flapping *= 0.9;
            self.flap[1] += self.flapping * 2.0;
        }
        // `calculateEntityAnimation(false)`: how far it went across the ground.
        let moved = self.now.pos - self.old.pos;
        let distance = (moved.x * moved.x + moved.z * moved.z).sqrt() as f32;
        self.walk_speed_old = self.walk_speed;
        // `updateWalkAnimation`: a frog's walk stops while it jumps, and a camel's while it
        // sits or dashes.
        let (target, factor) = match self.sort {
            Sort::Mob(MobType::Frog) if self.anim.started[0].is_some() => (0.0, 0.4),
            Sort::Mob(MobType::Frog) => ((distance * 25.0).min(1.0), 0.4),
            Sort::Mob(MobType::Camel) if self.pose() == 0 && self.anim.started[4].is_none() => {
                ((distance * 6.0).min(1.0), 0.2)
            }
            Sort::Mob(MobType::Camel) => (0.0, 0.2),
            _ => ((distance * 4.0).min(1.0), 0.4),
        };
        self.walk_speed += (target - self.walk_speed) * factor;
        self.walk_pos += self.walk_speed;

        let side = (moved.x * moved.x + moved.z * moved.z) as f32;
        match self.sort {
            // `LivingEntity.tick`'s body turn towards where it walks, then `tickHeadTurn`.
            Sort::Player => {
                let mut target = self.body_yaw;
                if side > 0.0025000002 {
                    let walking = (moved.z.atan2(moved.x) as f32).to_degrees() - 90.0;
                    let off = (wrap(self.now.yaw) - walking).abs();
                    target = if 95.0 < off && off < 265.0 {
                        walking - 180.0
                    } else {
                        walking
                    };
                }
                self.body_yaw += wrap(target - self.body_yaw) * 0.3;
                let head = wrap(self.now.yaw - self.body_yaw);
                if head.abs() > 50.0 {
                    self.body_yaw += head - head.signum() * 50.0;
                }
            }
            // `BodyRotationControl.clientTick`, with `Mob.getMaxHeadYRot` 75.
            _ => {
                if side > 2.5000003e-7 {
                    self.body_yaw = self.now.yaw;
                    self.head_yaw = rotate_if_necessary(self.head_yaw, self.body_yaw, 75.0);
                    self.last_stable_head_yaw = self.head_yaw;
                    self.head_stable_time = 0;
                } else if (self.head_yaw - self.last_stable_head_yaw).abs() > 15.0 {
                    self.head_stable_time = 0;
                    self.last_stable_head_yaw = self.head_yaw;
                    self.body_yaw = rotate_if_necessary(self.body_yaw, self.head_yaw, 75.0);
                } else {
                    self.head_stable_time += 1;
                    if self.head_stable_time > 10 {
                        let t = ((self.head_stable_time - 10) as f32 / 10.0).clamp(0.0, 1.0);
                        self.body_yaw =
                            rotate_if_necessary(self.body_yaw, self.head_yaw, 75.0 * (1.0 - t));
                    }
                }
            }
        }
    }

    /// The animation state some mobs tick on the client.
    fn tick_mob(
        &mut self,
        world: &impl mc_physics::World,
        blocks: &mc_physics::BlockPhysics,
        game_time: i64,
    ) {
        let Sort::Mob(mob) = self.sort else {
            return;
        };
        let a = &mut self.anim;
        let shift = |v: &mut [f32; 2]| v[0] = v[1];
        match mob {
            // `IronGolem.aiStep`.
            // `IronGolem.aiStep`, and `Hoglin` and `Zoglin`'s.
            MobType::IronGolem | MobType::Hoglin | MobType::Zoglin => {
                a.golem_attack = (a.golem_attack - 1).max(0)
            }
            // How long it's been charging its crossbow, for `getTicksUsingItem`.
            MobType::Piglin => {
                let charging =
                    matches!(self.data.get(&PIGLIN_CHARGING), Some(DataValue::Bool(true)));
                a.crossbow = if charging { a.crossbow + 1 } else { 0 };
            }
            // `Squid.aiStep` on the client: its tentacles stroke once per cycle and rest
            // at the end until the server starts the next (event 19); in water it spins
            // and tilts along how it's moving, and out of it it flops over.
            MobType::Squid | MobType::GlowSquid => {
                shift(&mut a.x_body_rot);
                shift(&mut a.z_body_rot);
                shift(&mut a.tentacle_angle);
                a.tentacle_movement =
                    (a.tentacle_movement + a.tentacle_speed).min(std::f32::consts::TAU);
                let block = self.now.pos.floor().as_ivec3();
                let in_water = mc_physics::fluid::fluid_at(world, blocks, block.to_array())
                    .is_some_and(|f| f.kind == mc_physics::fluid::FluidKind::Water);
                let pi = std::f32::consts::PI;
                if in_water {
                    if a.tentacle_movement < pi {
                        let t = a.tentacle_movement / pi;
                        a.tentacle_angle[1] = (t * t * pi).sin() * pi * 0.25;
                        a.rotate_speed = if t > 0.75 { 1.0 } else { a.rotate_speed * 0.8 };
                    } else {
                        a.tentacle_angle[1] = 0.0;
                        a.rotate_speed *= 0.99;
                    }
                    let v = self.velocity;
                    let horizontal = (v.x * v.x + v.z * v.z).sqrt();
                    a.z_body_rot[1] += pi * a.rotate_speed * 1.5;
                    let target = -(horizontal.atan2(v.y) as f32).to_degrees();
                    a.x_body_rot[1] += (target - a.x_body_rot[1]) * 0.1;
                } else {
                    a.tentacle_angle[1] = a.tentacle_movement.sin().abs() * pi * 0.25;
                    a.x_body_rot[1] += (-90.0 - a.x_body_rot[1]) * 0.02;
                }
            }
            // `PolarBear.tick` on the client.
            MobType::PolarBear => {
                shift(&mut a.stand);
                let up = matches!(self.data.get(&BEAR_STANDING), Some(DataValue::Bool(true)));
                a.stand[1] = (a.stand[1] + if up { 1.0 } else { -1.0 }).clamp(0.0, 6.0);
            }
            // `Cat.updateLieDownAmount` and `updateRelaxStateOneAmount`.
            MobType::Cat => {
                shift(&mut a.lie);
                shift(&mut a.lie_tail);
                shift(&mut a.relax);
                let flag = |i: u8| matches!(self.data.get(&i), Some(DataValue::Bool(true)));
                if flag(CAT_LYING) {
                    a.lie[1] = (a.lie[1] + 0.15).min(1.0);
                    a.lie_tail[1] = (a.lie_tail[1] + 0.08).min(1.0);
                } else {
                    a.lie[1] = (a.lie[1] - 0.22).max(0.0);
                    a.lie_tail[1] = (a.lie_tail[1] - 0.13).max(0.0);
                }
                a.relax[1] = if flag(CAT_RELAX) {
                    (a.relax[1] + 0.1).min(1.0)
                } else {
                    (a.relax[1] - 0.13).max(0.0)
                };
            }
            // `Fox.tick`: its interested head tilt eases in and out, and its crouch deepens.
            MobType::Fox => {
                shift(&mut a.interested);
                shift(&mut a.crouch);
                let flags = match self.data.get(&FOX_FLAGS) {
                    Some(DataValue::Byte(f)) => *f,
                    _ => 0,
                };
                let target = if flags & 8 != 0 { 1.0 } else { 0.0 };
                a.interested[1] += (target - a.interested[1]) * 0.4;
                a.crouch[1] = if flags & 4 != 0 {
                    (a.crouch[1] + 0.2).min(5.0)
                } else {
                    0.0
                };
            }
            // `Bat.setupAnimationStates`, after its tick: resting (flag 1) or flying.
            MobType::Bat => {
                let resting =
                    matches!(self.data.get(&MONSTER_DATA), Some(DataValue::Byte(f)) if f & 1 != 0);
                let (run, stop) = if resting { (1, 0) } else { (0, 1) };
                a.started[stop] = None;
                a.started[run].get_or_insert(i64::from(self.age));
            }
            // `Rabbit.setupAnimationStates` in `baseTick`, then `aiStep`'s hop count. The
            // idle head tilt waits for its timeout, and never comes while it has no AI
            // (`Mob`'s flag 1); leashes aren't tracked yet.
            MobType::Rabbit => {
                let no_ai =
                    matches!(self.data.get(&MOB_FLAGS), Some(DataValue::Byte(f)) if f & 1 != 0);
                if a.idle_timeout <= 0 && !no_ai {
                    a.idle_timeout = a.next_int(40) + 180;
                    a.started[1] = Some(i64::from(self.age));
                } else if a.jump_ticks > 0 {
                    a.started[0].get_or_insert(i64::from(self.age));
                    a.started[1] = None;
                } else {
                    a.idle_timeout -= 1;
                    a.started[0] = None;
                }
                if a.jump_ticks != a.jump_duration {
                    a.jump_ticks += 1;
                } else if a.jump_duration != 0 {
                    (a.jump_ticks, a.jump_duration) = (0, 0);
                }
            }
            // `Frog.tick`, before its walk updates: it treads water while it's in water
            // and not moving.
            MobType::Frog => {
                let block = self.now.pos.floor().as_ivec3();
                a.in_water = mc_physics::fluid::fluid_at(world, blocks, block.to_array())
                    .is_some_and(|f| f.kind == mc_physics::fluid::FluidKind::Water);
                if a.in_water && self.walk_speed_old > 1.0e-5 {
                    a.started[3] = None;
                } else if a.in_water {
                    a.started[3].get_or_insert(i64::from(self.age));
                } else {
                    a.started[3] = None;
                }
            }
            // `Camel.setupAnimationStates`: it idles every 4 to 6 seconds; sitting down,
            // sitting and standing up follow how long ago its pose changed (`getPoseTime`,
            // from `LAST_POSE_CHANGE_TICK`, negative while it sits).
            MobType::Camel => {
                let age = i64::from(self.age);
                if a.idle_timeout <= 0 {
                    a.idle_timeout = a.next_int(40) + 80;
                    a.started[3] = Some(age);
                } else {
                    a.idle_timeout -= 1;
                }
                let tick = match self.data.get(&CAMEL_POSE_TICK) {
                    Some(DataValue::Int(t)) => i64::from(*t),
                    _ => 0,
                };
                let sitting = tick < 0;
                let pose_time = game_time - tick.abs();
                if (pose_time < 0) != sitting {
                    a.started[2] = None;
                    a.started[4] = None;
                    if sitting && (0..40).contains(&pose_time) {
                        a.started[0].get_or_insert(age);
                        a.started[1] = None;
                    } else {
                        a.started[0] = None;
                        a.started[1].get_or_insert(age);
                    }
                } else {
                    a.started[0] = None;
                    a.started[1] = None;
                    let dashing = matches!(self.data.get(&CAMEL_DASH), Some(DataValue::Bool(true)));
                    // `isInPoseTransition() && getPoseTime() >= 0`.
                    let in_transition = pose_time < if sitting { 40 } else { 52 };
                    let standing_up = in_transition && pose_time >= 0;
                    for (i, on) in [(4, dashing), (2, standing_up)] {
                        if on {
                            a.started[i].get_or_insert(age);
                        } else {
                            a.started[i] = None;
                        }
                    }
                }
            }
            // `Armadillo.setupAnimationStates`, then `inStateTicks++`.
            MobType::Armadillo => {
                let age = i64::from(self.age);
                let state = match self.data.get(&MONSTER_STATE) {
                    Some(DataValue::Int(s)) => *s,
                    _ => 0,
                };
                match state {
                    // Rolling up.
                    1 => {
                        a.started[0] = None;
                        a.started[1].get_or_insert(age);
                        a.started[2] = None;
                    }
                    // Scared: peeking out every so often (event 64); one that's scared as
                    // it's first seen is already a peek's length in.
                    2 => {
                        a.started[0] = None;
                        a.started[1] = None;
                        if std::mem::take(&mut a.peeked) {
                            a.started[2] = None;
                        }
                        if a.in_state == 0 {
                            a.started[2] = Some(age - 50);
                        } else {
                            a.started[2].get_or_insert(age);
                        }
                    }
                    // Unrolling.
                    3 => {
                        a.started[0].get_or_insert(age);
                        a.started[1] = None;
                        a.started[2] = None;
                    }
                    _ => a.started[..3].fill(None),
                }
                a.in_state += 1;
            }
            _ => {}
        }
    }

    /// `AbstractArrow.tick` on the client: stuck once it's inside a block's collision
    /// shape; otherwise it turns a fifth of the way to where it's heading, sheds sparks if
    /// it's a critical hit, flies until it meets a block, where it sticks a little short
    /// with a quiver (`onHitBlock`), and slows and falls. The server hits entities.
    fn tick_arrow(
        &mut self,
        world: &impl mc_physics::World,
        blocks: &mc_physics::BlockPhysics,
        particles: &mut crate::particles::Particles,
    ) {
        let flags = match self.data.get(&ARROW_FLAGS) {
            Some(DataValue::Byte(f)) => *f,
            _ => 0,
        };
        let physics = flags & 2 == 0;
        let pos = self.now.pos;
        let block = pos.floor().as_ivec3();
        let state = world.state(block.x, block.y, block.z);
        if physics
            && blocks
                .shape(state)
                .iter()
                .any(|b| b.offset(block.as_dvec3()).contains(pos))
        {
            self.velocity = DVec3::ZERO;
            self.in_ground = true;
        }
        self.shake = (self.shake - 1).max(0);
        if self.in_ground && physics {
            self.in_ground_time += 1;
            return;
        }
        self.in_ground_time = 0;
        // `isInWater`, as the block it's in.
        let in_water = mc_physics::fluid::fluid_at(world, blocks, block.to_array())
            .is_some_and(|f| f.kind == mc_physics::fluid::FluidKind::Water);
        if in_water {
            // `getWaterInertia`.
            self.velocity *= f64::from(0.6f32);
        }
        let movement = self.velocity;
        if flags & 1 != 0 {
            for i in 0..4 {
                let at = pos + movement * (i as f64 / 4.0);
                if particles.allowed(false, false) {
                    let aux = DVec3::new(-movement.x, -movement.y + 0.2, -movement.z);
                    particles.crit(crate::particles::Kind::Crit, at, aux);
                }
            }
        }
        let sign = if physics { 1.0 } else { -1.0 };
        let yaw = (sign * movement.x).atan2(sign * movement.z).to_degrees() as f32;
        let horizontal = movement.x.hypot(movement.z);
        let pitch = movement.y.atan2(horizontal).to_degrees() as f32;
        self.now.pitch = lerp_rotation(self.now.pitch, pitch);
        self.now.yaw = lerp_rotation(self.now.yaw, yaw);
        if physics {
            match mc_physics::clip_collider(world, blocks, pos, pos + movement) {
                Some(hit) => {
                    let back = DVec3::new(
                        movement.x.signum(),
                        movement.y.signum(),
                        movement.z.signum(),
                    ) * f64::from(0.05f32);
                    self.now.pos = hit.location - back;
                    self.velocity = DVec3::ZERO;
                    self.in_ground = true;
                    self.shake = 7;
                }
                None => self.now.pos = pos + movement,
            }
        } else {
            self.now.pos = pos + movement;
        }
        if !in_water {
            self.velocity *= f64::from(0.99f32);
        }
        if physics && !self.in_ground {
            self.velocity.y -= self.gravity(0.05);
        }
    }

    /// `ThrowableProjectile.tick` on the client: it falls, slows (more in water), and flies
    /// on until it meets a block. The server hits entities and breaks it.
    fn tick_thrown(
        &mut self,
        world: &impl mc_physics::World,
        blocks: &mc_physics::BlockPhysics,
        gravity: f64,
    ) {
        self.velocity.y -= self.gravity(gravity);
        let block = self.now.pos.floor().as_ivec3();
        let in_water = mc_physics::fluid::fluid_at(world, blocks, block.to_array())
            .is_some_and(|f| f.kind == mc_physics::fluid::FluidKind::Water);
        self.velocity *= f64::from(if in_water { 0.8f32 } else { 0.99 });
        let (from, to) = (self.now.pos, self.now.pos + self.velocity);
        self.now.pos =
            mc_physics::clip_collider(world, blocks, from, to).map_or(to, |h| h.location);
    }

    /// `Entity.getGravity`: `default`, or none if the server says so.
    fn gravity(&self, default: f64) -> f64 {
        match self.data.get(&NO_GRAVITY) {
            Some(DataValue::Bool(true)) => 0.0,
            _ => default,
        }
    }

    /// The item a thrown item shows: the server's, or its type's own.
    fn item_shown(&self) -> String {
        match (self.data.get(&ITEM), self.sort) {
            (Some(DataValue::Item(Some((item, _)))), _) => item.clone(),
            (_, Sort::Thrown { item, .. }) => item.to_owned(),
            _ => String::new(),
        }
    }

    /// `ItemEntity.tick` on the client: gravity, sliding with block friction, and bouncing.
    fn tick_item(&mut self, world: &impl mc_physics::World, blocks: &mc_physics::BlockPhysics) {
        self.velocity.y -= 0.04;
        let still = self.on_ground && self.velocity.x.powi(2) + self.velocity.z.powi(2) <= 1e-5;
        if still && !self.age.is_multiple_of(4) {
            return;
        }
        let motion = self.velocity;
        let moved = mc_physics::collide_box(world, blocks, self.now.pos, [0.25, 0.25], motion);
        self.now.pos += moved;
        self.on_ground = moved.y != motion.y && motion.y < 0.0;
        if moved.x != motion.x {
            self.velocity.x = 0.0;
        }
        if moved.z != motion.z {
            self.velocity.z = 0.0;
        }
        if moved.y != motion.y {
            self.velocity.y = 0.0;
        }
        let mut friction = 0.98;
        if self.on_ground {
            let below = (self.now.pos - DVec3::new(0.0, 0.500001, 0.0))
                .floor()
                .as_ivec3();
            friction *= blocks.friction(world.state(below.x, below.y, below.z));
        }
        self.velocity *= DVec3::new(friction as f64, 0.98, friction as f64);
        if self.on_ground && self.velocity.y < 0.0 {
            self.velocity.y *= -0.5;
        }
    }

    /// Its render state, with what `all` knows about skins, names, items and the level.
    fn render(&self, pt: f32, all: &Entities) -> Option<Entity> {
        let (skins, names, armor, uses) = (&all.skins, &all.names, &all.armor, &all.uses);
        let (game_time, pumpkin) = (all.game_time, all.pumpkin);
        let pos = self.old.pos.lerp(self.now.pos, pt as f64).as_vec3();
        let age = self.age as f32 + pt;
        match self.sort {
            // `ArrowRenderer.extractRenderState`: its rotation lerped plainly.
            Sort::Arrow { spectral } => {
                let tipped =
                    matches!(self.data.get(&ARROW_COLOR), Some(DataValue::Int(c)) if *c > 0);
                let texture =
                    mc_render::entity::ARROW[if spectral { 2 } else { usize::from(tipped) }];
                let lerp = |a: f32, b: f32| a + (b - a) * pt;
                return Some(Entity::Arrow(mc_render::entity::Arrow {
                    pos,
                    yaw: lerp(self.old.yaw, self.now.yaw),
                    pitch: lerp(self.old.pitch, self.now.pitch),
                    shake: self.shake as f32 - pt,
                    texture,
                }));
            }
            Sort::Thrown { .. } => {
                return Some(Entity::Thrown(mc_render::entity::Thrown {
                    item: self.item_shown(),
                    pos,
                    age: self.age,
                }));
            }
            _ => {}
        }
        if self.sort == Sort::Item {
            let Some(DataValue::Item(Some((item, _)))) = self.data.get(&ITEM) else {
                return None;
            };
            return Some(Entity::Item(Dropped {
                item: item.clone(),
                pos,
                age,
                bob_offset: self.bob_offset,
                burning: self.burning(false),
            }));
        }
        let flag = |i: u8| matches!(self.data.get(&i), Some(DataValue::Bool(true)));
        let variant = |index: u8, animal: &str| {
            let name = match self.data.get(&index) {
                Some(DataValue::Variant(v)) => v.strip_prefix("minecraft:").unwrap_or(v),
                _ => "temperate",
            };
            format!("minecraft:entity/{animal}/{animal}_{name}")
        };
        let int = |i: u8| match self.data.get(&i) {
            Some(DataValue::Int(v)) => *v,
            _ => 0,
        };
        // `Mob.isAggressive`: bit 4 of its flags.
        let aggressive =
            matches!(self.data.get(&MOB_FLAGS), Some(DataValue::Byte(f)) if f & 4 != 0);
        let mut layers = Vec::new();
        // `VillagerProfessionLayer`'s textures: its type's clothes, then (for an adult with
        // a profession) the profession's and, but for a nitwit, its level's badge, from
        // stone at 1 to diamond at 5; under `entity/villager` or `entity/zombie_villager`.
        let villager_layers = |index: u8, base: &str, layers: &mut Vec<String>| {
            if let Some(DataValue::Villager {
                kind,
                profession,
                level,
            }) = self.data.get(&index)
            {
                let name = |n: &str| n.strip_prefix("minecraft:").unwrap_or(n).to_owned();
                // `VillagerProfessionLayer`: a baby zombie villager's clothes are its own.
                let dir = if self.baby() && base == "zombie_villager" {
                    "baby"
                } else {
                    "type"
                };
                let base = format!("minecraft:entity/{base}");
                layers.push(format!("{base}/{dir}/{}", name(kind)));
                let profession = name(profession);
                if profession != "none" && !self.baby() {
                    layers.push(format!("{base}/profession/{profession}"));
                    if profession != "nitwit" {
                        let levels = mc_render::entity::VILLAGER_LEVELS;
                        let level = levels[(*level).clamp(1, 5) as usize - 1];
                        layers.push(format!("{base}/profession_level/{level}"));
                    }
                }
            }
        };
        let (kind, texture) = match self.sort {
            Sort::Player => {
                let crouching = self.pose() == CROUCHING;
                let (skin, slim) = skin(skins, self.uuid);
                (Kind::Player { slim, crouching }, skin)
            }
            Sort::Mob(mob) => match mob {
                MobType::Zombie => (
                    Kind::Zombie { aggressive },
                    mc_render::entity::ZOMBIE.into(),
                ),
                MobType::Skeleton => (Kind::Skeleton, mc_render::entity::SKELETON.into()),
                MobType::Husk => (Kind::Husk { aggressive }, mc_render::entity::HUSK.into()),
                MobType::Drowned => (
                    Kind::Drowned { aggressive },
                    mc_render::entity::DROWNED.into(),
                ),
                MobType::Stray => (Kind::Stray, mc_render::entity::STRAY.into()),
                MobType::Enderman => {
                    let carried = u32::try_from(int(CARRY_STATE)).ok().filter(|&s| s != 0);
                    let kind = Kind::Enderman {
                        creepy: flag(CREEPY),
                        carried,
                    };
                    (kind, mc_render::entity::ENDERMAN.into())
                }
                MobType::Slime => {
                    let squish = self.squish[0] + (self.squish[1] - self.squish[0]) * pt;
                    let kind = Kind::Slime {
                        size: self.slime_size(),
                        squish,
                    };
                    (kind, mc_render::entity::SLIME.into())
                }
                MobType::Villager => {
                    villager_layers(VILLAGER_DATA, "villager", &mut layers);
                    let kind = Kind::Villager {
                        unhappy: int(UNHAPPY) > 0,
                    };
                    (kind, mc_render::entity::VILLAGER.into())
                }
                MobType::Witch => {
                    let kind = Kind::Witch {
                        id: self.id,
                        holding: self.equipment.contains_key(&0),
                    };
                    (kind, mc_render::entity::WITCH.into())
                }
                // `Wolf.getTexture` and `getTailAngle`: angry, tame or wild.
                MobType::Wolf => {
                    let flags = match self.data.get(&TAME_FLAGS) {
                        Some(DataValue::Byte(f)) => *f,
                        _ => 0,
                    };
                    let tame = flags & 4 != 0;
                    let end = i64::from(int(ANGER_END));
                    let angry = end > 0 && end - game_time > 0;
                    let tail = if angry {
                        1.5393804
                    } else if tame {
                        // `Wolf.createAttributes`' 8 until the server says otherwise.
                        let max = self.max_health.unwrap_or(8.0);
                        let health = match self.data.get(&HEALTH) {
                            Some(DataValue::Float(h)) => *h,
                            _ => max,
                        };
                        (0.55 - (max - health) / max * 0.4) * std::f32::consts::PI
                    } else {
                        std::f32::consts::PI / 5.0
                    };
                    let variant = match self.data.get(&WOLF_VARIANT) {
                        Some(DataValue::Variant(v)) => v.as_str(),
                        _ => "pale",
                    };
                    let mood = match (angry, tame) {
                        (true, _) => "_angry",
                        (_, true) => "_tame",
                        _ => "",
                    };
                    let kind = Kind::Wolf {
                        angry,
                        sitting: flags & 1 != 0,
                        tail,
                        // `Wolf.defineSynchedData`: red until dyed, which the server
                        // doesn't send.
                        collar: tame.then(|| match self.data.get(&COLLAR) {
                            Some(DataValue::Int(c)) => *c as u8,
                            _ => 14,
                        }),
                    };
                    (kind, mc_render::entity::wolf_texture(variant, mood))
                }
                MobType::ZombieVillager => {
                    villager_layers(ZOMBIE_VILLAGER_DATA, "zombie_villager", &mut layers);
                    let kind = Kind::ZombieVillager { aggressive };
                    let texture = if self.baby() {
                        mc_render::entity::ZOMBIE_VILLAGER_BABY
                    } else {
                        mc_render::entity::ZOMBIE_VILLAGER
                    };
                    (kind, texture.into())
                }
                MobType::CaveSpider => (Kind::CaveSpider, mc_render::entity::CAVE_SPIDER.into()),
                // `IronGolemRenderer.extractRenderState`, and `IronGolem.getCrackiness`
                // (`Crackiness.GOLEM`) from its health over its max health, 100 by default.
                MobType::IronGolem => {
                    let max = self.max_health.unwrap_or(100.0);
                    let health = match self.data.get(&HEALTH) {
                        Some(DataValue::Float(h)) => *h,
                        _ => max,
                    };
                    let fraction = health / max;
                    let cracks = match fraction {
                        f if f < 0.25 => 3,
                        f if f < 0.5 => 2,
                        f if f < 0.75 => 1,
                        _ => 0,
                    };
                    let a = self.anim.golem_attack;
                    let attack = if a > 0 { a as f32 - pt } else { 0.0 };
                    let kind = Kind::IronGolem { attack, cracks };
                    (kind, mc_render::entity::IRON_GOLEM.into())
                }
                MobType::Squid | MobType::GlowSquid => {
                    let a = &self.anim;
                    let lerp = |v: [f32; 2]| v[0] + (v[1] - v[0]) * pt;
                    let kind = Kind::Squid {
                        tentacles: lerp(a.tentacle_angle),
                        tilt: lerp(a.x_body_rot),
                        spin: lerp(a.z_body_rot),
                    };
                    // `SquidRenderer` and `GlowSquidRenderer.getTextureLocation`.
                    let texture = match (mob == MobType::Squid, self.baby()) {
                        (true, false) => mc_render::entity::SQUID,
                        (true, true) => mc_render::entity::SQUID_BABY,
                        (false, false) => mc_render::entity::GLOW_SQUID,
                        (false, true) => mc_render::entity::GLOW_SQUID_BABY,
                    };
                    (kind, texture.into())
                }
                // `PhantomRenderer.extractRenderState`: `getUniqueFlapTickOffset` is three
                // ticks per entity id.
                MobType::Phantom => {
                    let kind = Kind::Phantom {
                        flap: self.id as f32 * 3.0 + age,
                        size: self.phantom_size(),
                    };
                    (kind, mc_render::entity::PHANTOM.into())
                }
                // `CatRenderer.extractRenderState`: crouching (shared flag 1), sprinting
                // (flag 3), or sitting (`TamableAnimal`'s flag 1).
                MobType::Cat => {
                    let shared = match self.data.get(&SHARED_FLAGS) {
                        Some(DataValue::Byte(f)) => *f,
                        _ => 0,
                    };
                    let tame = match self.data.get(&TAME_FLAGS) {
                        Some(DataValue::Byte(f)) => *f,
                        _ => 0,
                    };
                    let pose = if tame & 1 != 0 {
                        mc_render::entity::FelinePose::Sitting
                    } else if shared & 2 != 0 {
                        mc_render::entity::FelinePose::Crouching
                    } else if shared & 8 != 0 {
                        mc_render::entity::FelinePose::Sprinting
                    } else {
                        mc_render::entity::FelinePose::Standing
                    };
                    let a = &self.anim;
                    let lerp = |v: [f32; 2]| v[0] + (v[1] - v[0]) * pt;
                    let name = match self.data.get(&CAT_VARIANT) {
                        Some(DataValue::Variant(v)) => v.strip_prefix("minecraft:").unwrap_or(v),
                        _ => "black",
                    };
                    let kind = Kind::Cat {
                        pose,
                        lie: lerp(a.lie),
                        lie_tail: lerp(a.lie_tail),
                        relax: lerp(a.relax),
                        // `Cat.defineSynchedData`: red until dyed.
                        collar: (tame & 4 != 0).then(|| match self.data.get(&CAT_COLLAR) {
                            Some(DataValue::Int(c)) => *c as u8,
                            _ => 14,
                        }),
                    };
                    // `CatVariant.assetInfo`: babies have their own.
                    let baby = if self.baby() { "_baby" } else { "" };
                    (kind, format!("minecraft:entity/cat/cat_{name}{baby}"))
                }
                // `OcelotRenderer.extractRenderState`: crouching (shared flag 1) or
                // sprinting (flag 3).
                MobType::Ocelot => {
                    let shared = match self.data.get(&SHARED_FLAGS) {
                        Some(DataValue::Byte(f)) => *f,
                        _ => 0,
                    };
                    let pose = if shared & 2 != 0 {
                        mc_render::entity::FelinePose::Crouching
                    } else if shared & 8 != 0 {
                        mc_render::entity::FelinePose::Sprinting
                    } else {
                        mc_render::entity::FelinePose::Standing
                    };
                    let texture = if self.baby() {
                        mc_render::entity::OCELOT_BABY
                    } else {
                        mc_render::entity::OCELOT
                    };
                    (Kind::Ocelot { pose }, texture.into())
                }
                // `FoxRenderer.getTextureLocation`: red or snow, asleep or not.
                MobType::Fox => {
                    let flags = match self.data.get(&FOX_FLAGS) {
                        Some(DataValue::Byte(f)) => *f,
                        _ => 0,
                    };
                    let a = &self.anim;
                    let lerp = |v: [f32; 2]| v[0] + (v[1] - v[0]) * pt;
                    let kind = Kind::Fox {
                        flags,
                        crouch: lerp(a.crouch),
                        roll: lerp(a.interested) * 0.11 * std::f32::consts::PI,
                    };
                    let snow = if int(FOX_TYPE) == 1 { "_snow" } else { "" };
                    let sleep = if flags & 32 != 0 { "_sleep" } else { "" };
                    let baby = if self.baby() { "_baby" } else { "" };
                    (kind, format!("minecraft:entity/fox/fox{snow}{sleep}{baby}"))
                }
                MobType::Bogged => {
                    let kind = Kind::Bogged {
                        sheared: flag(MONSTER_DATA),
                    };
                    (kind, mc_render::entity::BOGGED.into())
                }
                MobType::SnowGolem => {
                    let byte = match self.data.get(&MONSTER_DATA) {
                        Some(DataValue::Byte(b)) => *b,
                        _ => 16,
                    };
                    let kind = Kind::SnowGolem {
                        pumpkin: (byte & 16 != 0).then_some(pumpkin),
                    };
                    (kind, mc_render::entity::SNOW_GOLEM.into())
                }
                MobType::MagmaCube => {
                    let kind = Kind::MagmaCube {
                        size: self.slime_size(),
                        squish: self.squish[0] + (self.squish[1] - self.squish[0]) * pt,
                    };
                    (kind, mc_render::entity::MAGMA_CUBE.into())
                }
                MobType::Blaze => (Kind::Blaze, mc_render::entity::BLAZE.into()),
                // `GhastRenderer.getTextureLocation`: its mouth opens as it charges.
                MobType::Ghast => {
                    let texture = if flag(MONSTER_DATA) {
                        mc_render::entity::GHAST_SHOOTING
                    } else {
                        mc_render::entity::GHAST
                    };
                    (Kind::Ghast, texture.into())
                }
                MobType::Silverfish => (Kind::Silverfish, mc_render::entity::SILVERFISH.into()),
                MobType::Endermite => (Kind::Endermite, mc_render::entity::ENDERMITE.into()),
                // `PolarBear.getStandingAnimationScale`.
                MobType::PolarBear => {
                    let a = &self.anim;
                    let stand = (a.stand[0] + (a.stand[1] - a.stand[0]) * pt) / 6.0;
                    let texture = if self.baby() {
                        mc_render::entity::POLAR_BEAR_BABY
                    } else {
                        mc_render::entity::POLAR_BEAR
                    };
                    (Kind::PolarBear { stand }, texture.into())
                }
                // `Goat.defineSynchedData`: both horns until it loses one.
                MobType::Goat => {
                    let horn = |i: u8| !matches!(self.data.get(&i), Some(DataValue::Bool(false)));
                    let kind = Kind::Goat {
                        horns: [horn(GOAT_LEFT_HORN), horn(GOAT_RIGHT_HORN)],
                    };
                    let texture = if self.baby() {
                        mc_render::entity::GOAT_BABY
                    } else {
                        mc_render::entity::GOAT
                    };
                    (kind, texture.into())
                }
                // `Piglin.getArmPose` (a brute only ever holds its weapon high).
                MobType::Piglin | MobType::PiglinBrute => {
                    use mc_render::entity::PiglinPose;
                    let main = self.equipment.get(&0).map(|e| e.item.as_str());
                    let off = self.equipment.get(&1).map(|e| e.item.as_str());
                    let piglin = mob == MobType::Piglin;
                    let melee = aggressive && main.is_some_and(|i| all.tools.contains(i));
                    let pose = if piglin && flag(PIGLIN_DANCING) {
                        PiglinPose::Dancing
                    } else if piglin && off.is_some_and(|i| all.piglin_loved.contains(i)) {
                        PiglinPose::Admiring
                    } else if melee {
                        PiglinPose::Melee
                    } else if piglin && flag(PIGLIN_CHARGING) {
                        // `CrossbowItem.getChargeDuration`: 1.25 seconds without Quick
                        // Charge.
                        let ticks = self.anim.crossbow as f32 + pt;
                        PiglinPose::CrossbowCharge(ticks.clamp(0.0, 25.0) / 25.0)
                    } else if piglin
                        && self
                            .equipment
                            .get(&0)
                            .is_some_and(|e| e.item == "minecraft:crossbow" && e.charged)
                    {
                        PiglinPose::CrossbowHold
                    } else {
                        PiglinPose::Default
                    };
                    // `PiglinRenderer.getTextureLocation`.
                    let texture = if piglin && self.baby() {
                        mc_render::entity::PIGLIN_BABY
                    } else if piglin {
                        mc_render::entity::PIGLIN
                    } else {
                        mc_render::entity::PIGLIN_BRUTE
                    };
                    (Kind::Piglin { pose }, texture.into())
                }
                // `ZombifiedPiglinRenderer.getTextureLocation`.
                MobType::ZombifiedPiglin => {
                    let texture = if self.baby() {
                        mc_render::entity::ZOMBIFIED_PIGLIN_BABY
                    } else {
                        mc_render::entity::ZOMBIFIED_PIGLIN
                    };
                    (Kind::ZombifiedPiglin { aggressive }, texture.into())
                }
                MobType::Hoglin | MobType::Zoglin => {
                    // `HoglinRenderer` and `ZoglinRenderer.getTextureLocation`.
                    let texture = match (mob == MobType::Hoglin, self.baby()) {
                        (true, false) => mc_render::entity::HOGLIN,
                        (true, true) => mc_render::entity::HOGLIN_BABY,
                        (false, false) => mc_render::entity::ZOGLIN,
                        (false, true) => mc_render::entity::ZOGLIN_BABY,
                    };
                    let kind = Kind::Hoglin {
                        attack: self.anim.golem_attack,
                    };
                    (kind, texture.into())
                }
                // `StriderRenderer.getTextureLocation`: cold and grey out of lava.
                MobType::Strider => {
                    let texture = match (flag(STRIDER_COLD), self.baby()) {
                        (true, false) => mc_render::entity::STRIDER_COLD,
                        (true, true) => mc_render::entity::STRIDER_COLD_BABY,
                        (false, false) => mc_render::entity::STRIDER,
                        (false, true) => mc_render::entity::STRIDER_BABY,
                    };
                    (Kind::Strider, texture.into())
                }
                MobType::WitherSkeleton => (
                    Kind::WitherSkeleton,
                    mc_render::entity::WITHER_SKELETON.into(),
                ),
                // `BatRenderer.extractRenderState`, with the animations' ticks since they
                // started (`AnimationState.getTimeInMillis`).
                MobType::Bat => {
                    let since = self.anim.started.map(|s| s.map(|s| age - s as f32));
                    let kind = Kind::Bat {
                        resting: matches!(self.data.get(&MONSTER_DATA), Some(DataValue::Byte(f)) if f & 1 != 0),
                        fly: since[0],
                        rest: since[1],
                    };
                    (kind, mc_render::entity::BAT.into())
                }
                // `RabbitRenderer.getTextureLocation`: its variant's texture, or Toast's if
                // that's its name (`checkMagicName`).
                MobType::Rabbit => {
                    let since = self.anim.started.map(|s| s.map(|s| age - s as f32));
                    let kind = Kind::Rabbit {
                        hop: since[0],
                        tilt: since[1],
                    };
                    let toast = matches!(self.data.get(&CUSTOM_NAME), Some(DataValue::Text(t)) if t == "Toast");
                    let name = if toast {
                        "toast"
                    } else {
                        match self.data.get(&VARIANT) {
                            Some(DataValue::Int(v @ 0..=5)) => {
                                mc_render::entity::RABBIT_VARIANTS[*v as usize]
                            }
                            Some(DataValue::Int(99)) => "caerbannog",
                            _ => "brown",
                        }
                    };
                    let baby = if self.baby() { "_baby" } else { "" };
                    (kind, format!("minecraft:entity/rabbit/rabbit_{name}{baby}"))
                }
                // `FrogRenderer`: its variant's texture.
                MobType::Frog => {
                    let since = self.anim.started.map(|s| s.map(|s| age - s as f32));
                    let kind = Kind::Frog {
                        anims: [since[0], since[1], since[2], since[3]],
                        swimming: self.anim.in_water,
                    };
                    (kind, variant(VARIANT, "frog"))
                }
                MobType::Camel => {
                    let since = self.anim.started.map(|s| s.map(|s| age - s as f32));
                    let texture = match self.baby() {
                        true => mc_render::entity::CAMEL_BABY,
                        false => mc_render::entity::CAMEL,
                    };
                    (Kind::Camel { anims: since }, texture.into())
                }
                // `ArmadilloRenderer`, with `Armadillo.ArmadilloState.shouldHideInShell`.
                MobType::Armadillo => {
                    let since = self.anim.started.map(|s| s.map(|s| age - s as f32));
                    let hiding = match self.data.get(&MONSTER_STATE) {
                        Some(DataValue::Int(1)) => self.anim.in_state > 5,
                        Some(DataValue::Int(2)) => true,
                        Some(DataValue::Int(3)) => self.anim.in_state < 26,
                        _ => false,
                    };
                    let kind = Kind::Armadillo {
                        hiding,
                        anims: [since[0], since[1], since[2]],
                    };
                    let texture = match self.baby() {
                        true => mc_render::entity::ARMADILLO_BABY,
                        false => mc_render::entity::ARMADILLO,
                    };
                    (kind, texture.into())
                }
                MobType::Creeper => (Kind::Creeper, mc_render::entity::CREEPER.into()),
                MobType::Spider => (Kind::Spider, mc_render::entity::SPIDER.into()),
                MobType::Cow => (Kind::Cow, variant(VARIANT, "cow")),
                MobType::Pig => (Kind::Pig, variant(PIG_VARIANT, "pig")),
                MobType::Chicken => (Kind::Chicken, variant(VARIANT, "chicken")),
                MobType::Sheep => {
                    // The wool's colour in the low four bits, and 16 once sheared.
                    let byte = match self.data.get(&SHEEP_WOOL) {
                        Some(DataValue::Byte(b)) => *b,
                        _ => 0,
                    };
                    let kind = Kind::Sheep {
                        color: byte & 15,
                        sheared: byte & 16 != 0,
                    };
                    (kind, mc_render::entity::SHEEP.into())
                }
            },
            Sort::Item | Sort::Arrow { .. } | Sort::Thrown { .. } => {
                unreachable!("handled above")
            }
        };
        // `LivingEntityRenderer.extractRenderState`.
        let body_yaw = rot_lerp(pt, self.old_body_yaw, self.body_yaw);
        let head = rot_lerp(pt, self.old_head_yaw, self.head_yaw);
        let pitch = self.old.pitch + (self.now.pitch - self.old.pitch) * pt;
        Some(Entity::Living(Living {
            kind,
            texture,
            pos,
            body_yaw,
            head_yaw: wrap(head - body_yaw),
            pitch,
            // `WalkAnimationState.position(partialTicks)`: a baby's walk runs 3 times as far
            // (`updateWalkAnimation`'s `positionScale`).
            walk_pos: (self.walk_pos - self.walk_speed * (1.0 - pt))
                * if self.baby() { 3.0 } else { 1.0 },
            walk_speed: (self.walk_speed_old + (self.walk_speed - self.walk_speed_old) * pt)
                .min(1.0),
            age,
            flap: self.flap[0] + (self.flap[1] - self.flap[0]) * pt,
            flap_speed: self.flap_speed[0] + (self.flap_speed[1] - self.flap_speed[0]) * pt,
            baby: self.baby(),
            name_tag: self.name_tag(names),
            equipment: self.equipment(armor, uses, pt),
            swing: self.swing(pt),
            burning: self.burning(self.baby()),
            // `hasRedOverlay`.
            hurt: self.hurt_time > 0 || self.death_time > 0,
            death_time: if self.death_time > 0 {
                self.death_time as f32 + pt
            } else {
                0.0
            },
            layers,
        }))
    }

    /// `HumanoidMobRenderer.extractRenderState` and `ArmedEntityRenderState`'s hand items:
    /// the armour in the head, chest, legs and feet slots (5 to 2) that has an equipment
    /// asset, and the main hand's (slot 0) and off hand's (slot 1) items in whichever hands
    /// they're in. Players say which is their main arm; mobs are left-handed when their flags
    /// have `MOB_FLAG_LEFTHANDED` (2) set.
    /// The item in use is drawn in the arm holding it, while it has ticks left.
    fn equipment(
        &self,
        armor: &HashMap<String, String>,
        uses: &HashMap<String, (mc_render::entity::UseAnim, i32)>,
        pt: f32,
    ) -> Equipment {
        let left_handed = self.left_handed();
        let using = self.using.as_ref().and_then(|(hand, item, remaining)| {
            let &(anim, duration) = uses.get(item)?;
            (*remaining > 0).then_some(mc_render::entity::Using {
                hand: usize::from((*hand == 1) != left_handed),
                anim,
                duration,
                remaining: *remaining as f32 - pt + 1.0,
                ticks: duration - remaining,
            })
        });
        let item = |slot| self.equipment.get(&slot).map(|e| e.item.clone());
        let (main, off) = (item(0), item(1));
        Equipment {
            armor: [5, 4, 3, 2].map(|slot| {
                let worn = self.equipment.get(&slot)?;
                Some(Armor {
                    asset: armor.get(&worn.item)?.clone(),
                    dye: worn.dye,
                })
            }),
            hands: if left_handed {
                [off, main]
            } else {
                [main, off]
            },
            left_handed,
            using,
        }
    }

    /// Whether its main arm is its left.
    fn left_handed(&self) -> bool {
        match self.sort {
            Sort::Player => matches!(self.data.get(&MAIN_HAND), Some(DataValue::Int(0))),
            _ => matches!(self.data.get(&MOB_FLAGS), Some(DataValue::Byte(f)) if f & 2 != 0),
        }
    }

    /// `ArmedEntityRenderState.currentSwing` and `swingAnimation`: the swing under way, and
    /// which arm it's in (`InteractionHand.asArm`).
    fn swing(&self, pt: f32) -> Option<Swing> {
        let (hand, kind, _) = self.swing.current?;
        Some(Swing {
            left: (hand == 1) != self.left_handed(),
            kind,
            progress: self.swing.animation(pt),
        })
    }

    /// `Entity.getPose`, as its `Pose` id.
    /// `onSyncedDataUpdated` for the values that change: a frog's pose starts or stops
    /// its jump, croak and tongue; an armadillo's state restarts `inStateTicks`.
    fn data_changed(&mut self, values: &[(u8, DataValue)]) {
        for (index, value) in values {
            let changed = self.data.get(index) != Some(value);
            match (self.sort, *index, value) {
                (Sort::Mob(MobType::Frog), POSE, DataValue::Int(pose)) if changed => {
                    let age = i64::from(self.age);
                    for (slot, wanted) in [(0, 6), (1, 8), (2, 9)] {
                        self.anim.started[slot] = (*pose == wanted).then_some(age);
                    }
                }
                (Sort::Mob(MobType::Armadillo), MONSTER_STATE, _) if changed => {
                    self.anim.in_state = 0
                }
                _ => {}
            }
        }
    }

    fn pose(&self) -> i32 {
        match self.data.get(&POSE) {
            Some(DataValue::Int(pose)) => *pose,
            _ => 0,
        }
    }

    /// `AvatarRenderer.shouldShowName`: players show their names unless they're invisible
    /// (shared flag 5), at their height in their pose (`Avatar.POSES`), faintly while
    /// sneaking (shared flag 1, `isDiscrete`). Other mobs show only custom names, which
    /// aren't read yet.
    fn name_tag(&self, names: &HashMap<u128, String>) -> Option<NameTag> {
        if self.sort != Sort::Player {
            return None;
        }
        let flags = match self.data.get(&SHARED_FLAGS) {
            Some(DataValue::Byte(f)) => *f,
            _ => 0,
        };
        if flags & 1 << 5 != 0 {
            return None;
        }
        let height = match self.pose() {
            // Fall flying, swimming and spin attacking; sleeping and dying.
            1 | 3 | 4 => 0.6,
            2 | 7 => 0.2,
            CROUCHING => 1.5,
            _ => 1.8,
        };
        Some(NameTag {
            text: names.get(&self.uuid)?.clone(),
            height,
            discrete: flags & 1 << 1 != 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64) -> Place {
        Place {
            pos: DVec3::new(x, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
        }
    }

    /// A move glides over the type's interval: a mob (3 ticks) told to go 3 blocks starts
    /// a third of the way on its first tick (`setStartingPoint`'s one tick already gone),
    /// and arrives on the third.
    #[test]
    fn moves_glide_over_the_update_interval() {
        let mut i = Interpolation::new(3);
        i.interpolate_to(at(0.0), Some(&Path::Linear([3.0, 0.0, 0.0])), None);
        let xs: Vec<f64> = (0..4).map(|_| i.advance().pos.x).collect();
        for (x, want) in xs.iter().zip([1.0, 2.0, 3.0, 3.0]) {
            assert!((x - want).abs() < 1e-6, "{xs:?}");
        }
        assert!(!i.active());
    }

    /// Moves arriving faster than they play out speed it up, so it never falls far behind:
    /// with a new 1-block move every tick, it keeps up within a few ticks' distance.
    #[test]
    fn piled_up_moves_catch_up() {
        let mut i = Interpolation::new(3);
        let mut now = at(0.0);
        for t in 1..=40 {
            i.interpolate_to(now, Some(&Path::Linear([t as f64, 0.0, 0.0])), None);
            now = i.advance();
        }
        assert!(now.pos.x > 36.0, "{}", now.pos.x);
    }

    /// `ProjectileUtil.getEntityHitResult`: the nearer of two cows in line is hit on its
    /// near face, a cow behind the eye isn't, and nothing is past the range.
    #[test]
    fn picks_the_nearest_entity_in_reach() {
        let mut e = Entities {
            types: vec!["minecraft:cow".into()],
            tracked: HashMap::new(),
            own: None,
            pumpkin: 0,
            tools: HashSet::new(),
            piglin_loved: HashSet::new(),
            skins: HashMap::new(),
            names: HashMap::new(),
            armor: HashMap::new(),
            uses: HashMap::new(),
            icons: Default::default(),
            seed: 1,
            particles: Default::default(),
            emitters: Vec::new(),
            bolts: HashMap::new(),
            random: mc_render::bake::JavaRandom::new(1),
            thunder: Vec::new(),
            game_time: 0,
        };
        for (id, x) in [(1, 5.0), (2, 2.5), (3, -2.0)] {
            e.event(Event::AddEntity {
                id,
                uuid: 0,
                kind: 0,
                pos: [x, 0.0, 0.0],
                velocity: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                head_yaw: 0.0,
                data: 0,
            });
        }
        // Along +x at y 1: cow 2's box (0.9 wide) starts at x 2.05.
        let (from, to) = (DVec3::new(0.0, 1.0, 0.0), DVec3::new(6.0, 1.0, 0.0));
        let (id, at, alive) = e.pick(from, to, 36.0).unwrap();
        assert_eq!((id, alive), (2, true));
        assert!((at.x - 2.05).abs() < 1e-6, "{at}");
        assert_eq!(e.pick(from, to, 2.0 * 2.0), None);
    }

    /// `BodyRotationControl`: a standing mob's body follows its head once it turns past
    /// 75 degrees, and turns to face where the head looks after the head has been still
    /// for 20 ticks.
    #[test]
    fn mob_bodies_follow_their_heads() {
        let mut e = Entities {
            types: vec!["minecraft:cow".into()],
            tracked: HashMap::new(),
            own: None,
            pumpkin: 0,
            tools: HashSet::new(),
            piglin_loved: HashSet::new(),
            skins: HashMap::new(),
            names: HashMap::new(),
            armor: HashMap::new(),
            uses: HashMap::new(),
            icons: Default::default(),
            seed: 1,
            particles: Default::default(),
            emitters: Vec::new(),
            bolts: HashMap::new(),
            random: mc_render::bake::JavaRandom::new(1),
            thunder: Vec::new(),
            game_time: 0,
        };
        e.event(Event::AddEntity {
            id: 1,
            uuid: 0,
            kind: 0,
            pos: [0.0; 3],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            data: 0,
        });
        e.event(Event::HeadRotation { id: 1, yaw: 120.0 });
        let cow = |e: &mut Entities| {
            let c = e.tracked.get_mut(&1).unwrap();
            c.tick_living();
            (c.head_yaw, c.body_yaw)
        };
        cow(&mut e);
        cow(&mut e);
        let (head, body) = cow(&mut e);
        assert!(
            (head - 120.0).abs() < 1e-3 && (body - 45.0).abs() < 1e-3,
            "{head} {body}"
        );
        for _ in 0..19 {
            cow(&mut e);
        }
        let (_, body) = cow(&mut e);
        assert!((body - 120.0).abs() < 1e-3, "{body}");
    }

    /// `SwingState`: a 6-tick swing runs from 0 to 1 a sixth at a time, then ends; a new
    /// swing in its first half is ignored, and a later one restarts it, running on past 1
    /// as the progress goes round again.
    #[test]
    fn swings_follow_vanilla_timing() {
        let mut s = SwingState::default();
        let whack = (0, 1, 6);
        s.start_if_able(whack);
        let progress: Vec<f32> = (0..8)
            .map(|_| {
                s.tick();
                s.animation(1.0)
            })
            .collect();
        let sixth = |n: f32| n / 6.0;
        let expected = [
            0.0,
            sixth(1.0),
            sixth(2.0),
            sixth(3.0),
            sixth(4.0),
            sixth(5.0),
            1.0,
            1.0,
        ];
        assert_eq!(progress[..7], expected[..7]);
        s.tick();
        assert_eq!((s.current, s.animation(1.0)), (None, 0.0));

        let mut s = SwingState::default();
        s.start_if_able(whack);
        for _ in 0..3 {
            s.tick();
        }
        // Three ticks in (ticks 3 of 6): not past half way, so ignored.
        s.start_if_able(whack);
        s.tick();
        assert_eq!(s.animation(1.0), sixth(3.0));
        // Four ticks in: restarts, and the progress runs on from 1/2 towards 1 + 0.
        s.start_if_able(whack);
        s.tick();
        assert_eq!(s.animation(0.5), sixth(3.0) + (1.0 - sixth(3.0)) * 0.5);
    }

    /// A player's tag shows its name from the player list at its height, drops and fades
    /// while it sneaks, and disappears while it's invisible or once it leaves the list.
    #[test]
    fn players_show_name_tags() {
        let mut e = Entities {
            types: vec!["minecraft:player".into()],
            tracked: HashMap::new(),
            own: None,
            pumpkin: 0,
            tools: HashSet::new(),
            piglin_loved: HashSet::new(),
            skins: HashMap::new(),
            names: HashMap::new(),
            armor: HashMap::new(),
            uses: HashMap::new(),
            icons: Default::default(),
            seed: 1,
            particles: Default::default(),
            emitters: Vec::new(),
            bolts: HashMap::new(),
            random: mc_render::bake::JavaRandom::new(1),
            thunder: Vec::new(),
            game_time: 0,
        };
        e.event(Event::AddEntity {
            id: 1,
            uuid: 7,
            kind: 0,
            pos: [0.0; 3],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            data: 0,
        });
        e.set_name(7, Some("Steve".into()));
        let tag = |e: &mut Entities, flags: u8, pose: i32| {
            let values = vec![
                (SHARED_FLAGS, DataValue::Byte(flags)),
                (POSE, DataValue::Int(pose)),
            ];
            e.event(Event::EntityData { id: 1, values });
            match e.render(0.0, |_| [15, 0]).pop() {
                Some((Entity::Living(l), _)) => l.name_tag.map(|t| (t.text, t.height, t.discrete)),
                _ => panic!("the player isn't drawn"),
            }
        };
        assert_eq!(tag(&mut e, 0, 0), Some(("Steve".into(), 1.8, false)));
        // Sneaking: shared flag 1, crouching pose.
        assert_eq!(tag(&mut e, 2, 5), Some(("Steve".into(), 1.5, true)));
        // Invisible: shared flag 5.
        assert_eq!(tag(&mut e, 1 << 5, 0), None);
        e.set_name(7, None);
        assert_eq!(tag(&mut e, 0, 0), None);
    }

    /// A hit turns an entity red for the 9 ticks after it (`hurtTime` counts down from 10
    /// and shows while above 0); a hit on an untracked id (the player's own) passes on; a
    /// dead one stays red as it dies.
    #[test]
    fn hits_and_deaths_flash_red() {
        let mut e = Entities {
            types: vec!["minecraft:player".into()],
            tracked: HashMap::new(),
            own: None,
            pumpkin: 0,
            tools: HashSet::new(),
            piglin_loved: HashSet::new(),
            skins: HashMap::new(),
            names: HashMap::new(),
            armor: HashMap::new(),
            uses: HashMap::new(),
            icons: Default::default(),
            seed: 1,
            particles: Default::default(),
            emitters: Vec::new(),
            bolts: HashMap::new(),
            random: mc_render::bake::JavaRandom::new(1),
            thunder: Vec::new(),
            game_time: 0,
        };
        e.event(Event::AddEntity {
            id: 1,
            uuid: 7,
            kind: 0,
            pos: [0.0; 3],
            velocity: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            data: 0,
        });
        let hurt = |e: &Entities| match e.render(0.0, |_| [15, 0]).pop() {
            Some((Entity::Living(l), _)) => l.hurt,
            _ => panic!("the player isn't drawn"),
        };
        let tick = |e: &mut Entities| e.tracked.get_mut(&1).expect("added").tick_living();
        assert!(!hurt(&e));
        assert!(
            e.event(Event::Damaged {
                id: 1,
                kind: String::new()
            })
            .is_none()
        );
        let flashing: Vec<bool> = (0..11)
            .map(|_| {
                let now = hurt(&e);
                tick(&mut e);
                now
            })
            .collect();
        assert_eq!(flashing, [[true; 10].as_slice(), &[false]].concat());
        assert!(matches!(
            e.event(Event::Damaged {
                id: 2,
                kind: String::new()
            }),
            Some(Event::Damaged { id: 2, .. })
        ));
        let values = vec![(HEALTH, DataValue::Float(0.0))];
        e.event(Event::EntityData { id: 1, values });
        assert!(!hurt(&e));
        tick(&mut e);
        assert!(hurt(&e));
    }
}
