//! Other players, mobs and dropped items, as the server reports them, moved the way the
//! vanilla client moves them: living entities glide to each reported position over a few
//! ticks (`SteppedInterpolationHandler`), turn their bodies after their heads
//! (`LivingEntity.tickHeadTurn`, `BodyRotationControl`) and swing their limbs with how far
//! they went (`WalkAnimationState`); dropped items fall, slide and bounce on their own
//! (`ItemEntity.tick`) and snap to where the server says.

use glam::DVec3;
use mc_protocol::{DataValue, Event, Path};
use mc_render::entity::{Armor, Dropped, Entity, Equipment, Kind, Lit, Living, NameTag, Swing};
use std::collections::{HashMap, VecDeque};

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
}

struct Tracked {
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
    data: HashMap<u8, DataValue>,
    /// What it holds and wears, by `EquipmentSlot` id.
    equipment: HashMap<u8, mc_protocol::Equipped>,
    swing: SwingState,
    /// `LivingEntity.hurtTime`: ticks left of the red flash after a hit, from 10.
    hurt_time: i32,
    /// `LivingEntity.deathTime`: ticks since its health ran out.
    death_time: u32,
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
#[derive(Default)]
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
const POSE: u8 = 6;
const HEALTH: u8 = 9;
/// `Avatar.DATA_PLAYER_MAIN_HAND`: `HumanoidArm`'s id, left 0 or right 1.
const MAIN_HAND: u8 = 15;
/// `Pose.CROUCHING`'s id.
const CROUCHING: i32 = 5;
const ITEM: u8 = 8;
const MOB_FLAGS: u8 = 15;
const BABY: u8 = 16;
const SHEEP_WOOL: u8 = 18;
/// Cows' and chickens' variants come straight after `AgeableMob`'s two; a pig's after its
/// boost time.
const VARIANT: u8 = 18;
const PIG_VARIANT: u8 = 19;

pub struct Entities {
    /// Entity type names by protocol id.
    types: Vec<String>,
    /// Players' downloaded skins, by UUID: the atlas sprite and whether it's slim.
    skins: HashMap<u128, (String, bool)>,
    /// Players' names, by UUID, from the player list.
    names: HashMap<u128, String>,
    /// Each wearable item's equipment asset (`equippable`'s `asset_id`), by item.
    armor: HashMap<String, String>,
    tracked: HashMap<i32, Tracked>,
    /// Seeds items' bob offsets, as `Entity.random` would.
    seed: u64,
    /// The poofs of smoke dying mobs leave.
    particles: crate::particles::Particles,
    /// `TrackingEmitter`s: sparks burst from an entity hit critically, for three ticks.
    /// The entity, the particle, and the bursts left.
    emitters: Vec<(i32, crate::particles::Kind, u8)>,
    /// Lightning bolts, which `LightningBolt.tick` runs on the client too.
    bolts: HashMap<i32, Bolt>,
    random: mc_render::bake::JavaRandom,
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
    pub fn new(registry: &mc_registry::Registry) -> Self {
        Self {
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
            tracked: HashMap::new(),
            seed: 0x5DEECE66D,
            particles: Default::default(),
            emitters: Vec::new(),
            bolts: HashMap::new(),
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
                // `LivingEntity.recreateFromPacket`: body and head both face the head's way.
                self.tracked.insert(
                    id,
                    Tracked {
                        sort,
                        uuid,
                        base: pos,
                        now: place,
                        old: place,
                        interpolation: Interpolation::new(steps),
                        velocity: DVec3::from(velocity),
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
                        bob_offset,
                        flap: [0.0; 2],
                        flap_speed: [0.0; 2],
                        flapping: 1.0,
                        data: HashMap::new(),
                        equipment: HashMap::new(),
                        swing: SwingState::default(),
                        hurt_time: 0,
                        death_time: 0,
                    },
                );
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
                    _ => Kind::Damage,
                };
                let (pos, dist, speed) = (
                    DVec3::from(pos),
                    DVec3::from(dist.map(f64::from)),
                    DVec3::from(speed.map(f64::from)),
                );
                let p = &mut self.particles;
                if count == 0 {
                    p.crit(kind, pos, speed * dist);
                }
                for _ in 0..count {
                    if alternative != 0 {
                        let mut r = || f64::from(p.random());
                        let offset = DVec3::new(r(), r(), r()) * dist;
                        let mut aux = speed;
                        if alternative == 2 {
                            aux *= DVec3::new(r(), r(), r());
                        }
                        p.crit(kind, pos + offset, aux);
                    } else {
                        let offset = DVec3::new(p.gaussian(), p.gaussian(), p.gaussian()) * dist;
                        let aux = DVec3::new(p.gaussian(), p.gaussian(), p.gaussian()) * speed;
                        p.crit(kind, pos + offset, aux);
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
                    e.data.extend(values);
                }
            }
            // `LivingEntity.handleDamageEvent` and `animateHurt`: it flashes red. The
            // player's own pass on.
            Event::Damaged(id) | Event::HurtAnimation { id, .. }
                if self.tracked.contains_key(&id) =>
            {
                self.tracked.get_mut(&id).expect("checked").hurt_time = 10;
            }
            // `LivingEntity.handleEntityEvent(60)`: the mob's gone in a poof, just before
            // the server removes it.
            Event::EntityEvent { id, event: 60 } => {
                if let Some(e) = self.tracked.get(&id) {
                    let baby = matches!(e.data.get(&BABY), Some(DataValue::Bool(true)));
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
            let baby = matches!(e.data.get(&BABY), Some(DataValue::Bool(true)));
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
                particles.crit(*kind, at, DVec3::new(xa, ya + 0.2, za));
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
            e.tick(world, blocks);
        }
        self.particles.tick(world, blocks);
        self.emit();
        // `LightningBolt.tick`: a bolt shows for its life, then may flash again after up
        // to ten ticks with a new shape, and goes once its flashes run out.
        let random = &mut self.random;
        let mut flash = false;
        self.bolts.retain(|_, bolt| {
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
            let baby = matches!(e.data.get(&BABY), Some(DataValue::Bool(true)));
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
    pub fn lightning(&self) -> Vec<mc_render::mesh::Vertex> {
        self.bolts
            .values()
            .flat_map(|bolt| mc_render::weather::lightning(bolt.pos, bolt.seed))
            .collect()
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
            .filter_map(|e| {
                let entity = e.render(partial, &self.skins, &self.names, &self.armor)?;
                let (feet, baby) = match &entity {
                    Entity::Living(l) => (l.pos, l.baby),
                    Entity::Item(d) => (d.pos, false),
                };
                let eye = e.eye_height() * if baby { 0.5 } else { 1.0 };
                let [sky, mut block] = light(feet.as_dvec3() + DVec3::Y * eye);
                // `Entity.isOnFire`: bit 0 of the shared flags.
                if matches!(e.data.get(&SHARED_FLAGS), Some(DataValue::Byte(f)) if f & 1 != 0) {
                    block = 15;
                }
                Some((entity, [sky, block]))
            })
            .collect()
    }
}

impl Tracked {
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
            Sort::Item => 0.2125,
            Sort::Mob(mob) => match mob {
                MobType::Zombie | MobType::Skeleton => 1.74,
                MobType::Creeper => 1.7 * 0.85,
                MobType::Spider => 0.65,
                MobType::Cow => 1.3,
                MobType::Pig => 0.9 * 0.85,
                MobType::Sheep => 1.235,
                MobType::Chicken => 0.644,
            },
        }
    }

    /// `Entity.displayFireAnimation`: the bounding box's size while shared flag 0 says
    /// it's on fire. None of these types is immune to fire.
    fn burning(&self, baby: bool) -> Option<[f32; 2]> {
        matches!(self.data.get(&SHARED_FLAGS), Some(DataValue::Byte(f)) if f & 1 != 0)
            .then(|| self.size(baby))
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
            Sort::Item => [0.25, 0.25],
            Sort::Mob(mob) => match (mob, baby) {
                (MobType::Zombie, false) => [0.6, 1.95],
                (MobType::Zombie, true) => [0.49, 0.98],
                (MobType::Skeleton, _) => [0.6, 1.99],
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

    fn living(&self) -> bool {
        self.sort != Sort::Item
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

    fn tick(&mut self, world: &impl mc_physics::World, blocks: &mc_physics::BlockPhysics) {
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
            Sort::Player | Sort::Mob(_) => self.tick_living(),
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
        self.walk_speed += ((distance * 4.0).min(1.0) - self.walk_speed) * 0.4;
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

    fn render(
        &self,
        pt: f32,
        skins: &HashMap<u128, (String, bool)>,
        names: &HashMap<u128, String>,
        armor: &HashMap<String, String>,
    ) -> Option<Entity> {
        let pos = self.old.pos.lerp(self.now.pos, pt as f64).as_vec3();
        let age = self.age as f32 + pt;
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
        let (kind, texture) = match self.sort {
            Sort::Player => {
                let crouching = self.pose() == CROUCHING;
                let (skin, slim) = skin(skins, self.uuid);
                (Kind::Player { slim, crouching }, skin)
            }
            Sort::Mob(mob) => match mob {
                MobType::Zombie => {
                    // `Mob.isAggressive`: bit 4 of its flags.
                    let aggressive =
                        matches!(self.data.get(&MOB_FLAGS), Some(DataValue::Byte(f)) if f & 4 != 0);
                    (
                        Kind::Zombie { aggressive },
                        mc_render::entity::ZOMBIE.into(),
                    )
                }
                MobType::Skeleton => (Kind::Skeleton, mc_render::entity::SKELETON.into()),
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
            Sort::Item => unreachable!("handled above"),
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
            walk_pos: self.walk_pos - self.walk_speed * (1.0 - pt),
            walk_speed: (self.walk_speed_old + (self.walk_speed - self.walk_speed_old) * pt)
                .min(1.0),
            age,
            flap: self.flap[0] + (self.flap[1] - self.flap[0]) * pt,
            flap_speed: self.flap_speed[0] + (self.flap_speed[1] - self.flap_speed[0]) * pt,
            baby: flag(BABY),
            name_tag: self.name_tag(names),
            equipment: self.equipment(armor),
            swing: self.swing(pt),
            burning: self.burning(flag(BABY)),
            // `hasRedOverlay`.
            hurt: self.hurt_time > 0 || self.death_time > 0,
            death_time: if self.death_time > 0 {
                self.death_time as f32 + pt
            } else {
                0.0
            },
        }))
    }

    /// `HumanoidMobRenderer.extractRenderState` and `ArmedEntityRenderState`'s hand items:
    /// the armour in the head, chest, legs and feet slots (5 to 2) that has an equipment
    /// asset, and the main hand's (slot 0) and off hand's (slot 1) items in whichever hands
    /// they're in. Players say which is their main arm; mobs are left-handed when their flags
    /// have `MOB_FLAG_LEFTHANDED` (2) set.
    fn equipment(&self, armor: &HashMap<String, String>) -> Equipment {
        let left_handed = self.left_handed();
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
            skins: HashMap::new(),
            names: HashMap::new(),
            armor: HashMap::new(),
            seed: 1,
            particles: Default::default(),
            emitters: Vec::new(),
            bolts: HashMap::new(),
            random: mc_render::bake::JavaRandom::new(1),
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
            skins: HashMap::new(),
            names: HashMap::new(),
            armor: HashMap::new(),
            seed: 1,
            particles: Default::default(),
            emitters: Vec::new(),
            bolts: HashMap::new(),
            random: mc_render::bake::JavaRandom::new(1),
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
            skins: HashMap::new(),
            names: HashMap::new(),
            armor: HashMap::new(),
            seed: 1,
            particles: Default::default(),
            emitters: Vec::new(),
            bolts: HashMap::new(),
            random: mc_render::bake::JavaRandom::new(1),
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
            skins: HashMap::new(),
            names: HashMap::new(),
            armor: HashMap::new(),
            seed: 1,
            particles: Default::default(),
            emitters: Vec::new(),
            bolts: HashMap::new(),
            random: mc_render::bake::JavaRandom::new(1),
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
        assert!(e.event(Event::Damaged(1)).is_none());
        let flashing: Vec<bool> = (0..11)
            .map(|_| {
                let now = hurt(&e);
                tick(&mut e);
                now
            })
            .collect();
        assert_eq!(flashing, [[true; 10].as_slice(), &[false]].concat());
        assert!(matches!(
            e.event(Event::Damaged(2)),
            Some(Event::Damaged(2))
        ));
        let values = vec![(HEALTH, DataValue::Float(0.0))];
        e.event(Event::EntityData { id: 1, values });
        assert!(!hurt(&e));
        tick(&mut e);
        assert!(hurt(&e));
    }
}
