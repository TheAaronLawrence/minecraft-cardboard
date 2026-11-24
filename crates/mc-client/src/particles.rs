//! The particles in the world, moved each tick: `Particle.tick` and `move`, for the poofs
//! of smoke dying mobs leave (`ExplodeParticle`), the sparks and dark hearts of hits
//! (`CritParticle`), the crumbs of food being eaten (`BreakingItemParticle`), potion
//! swirls (`SpellParticle`), a glow squid's glow (`GlowParticle`), squids' ink
//! (`SquidInkParticle`), bits of blocks broken, mined and run on (`TerrainParticle`), and
//! the purple specks about endermen and teleports (`PortalParticle`).

use glam::DVec3;

/// What a particle is, which decides how it looks and moves.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Poof,
    /// `ParticleTypes.CRIT`.
    Crit,
    /// `ParticleTypes.ENCHANTED_HIT`: a crit tinted blue.
    EnchantedHit,
    /// `ParticleTypes.DAMAGE_INDICATOR`: a dark heart, flung up.
    Damage,
    /// `ParticleTypes.ITEM`: a quarter of an item's sprite, the part in `rect`.
    Item,
    /// `ParticleTypes.BLOCK` (`TerrainParticle`): a quarter of a block's particle sprite,
    /// the part in `rect`.
    Terrain,
    /// `ParticleTypes.ENTITY_EFFECT` (`SpellParticle`): a potion's swirl, tinted its
    /// colour, rising.
    Effect,
    /// `ParticleTypes.GLOW` (`GlowParticle.GlowSquidProvider`): a speck drifting off a glow
    /// squid, light or dark teal, glowing brighter until it's gone.
    Glow,
    /// `ParticleTypes.SQUID_INK` and `GLOW_SQUID_INK` (`SquidInkParticle`): a puff of
    /// black or teal ink that shrinks as it fades, sinking through air, always fully lit.
    Ink,
    /// `ParticleTypes.CLOUD` (`PlayerCloudParticle`): a big pale puff that swells into
    /// view, blended.
    Cloud,
    /// `ParticleTypes.PORTAL` (`PortalParticle`): a purple speck that sweeps back in to
    /// `start` from its offset, dropping a block as it goes, glowing brighter until it's
    /// gone; it keeps the one `generic_<sprite>` it's given.
    Portal {
        start: DVec3,
        sprite: u8,
    },
}

/// A particle: where it is now and last tick, its motion, and how far through its life.
struct Particle {
    kind: Kind,
    old: DVec3,
    pos: DVec3,
    velocity: DVec3,
    on_ground: bool,
    /// `stoppedByCollision`: once a fall is stopped dead, it never moves again.
    stopped: bool,
    age: u32,
    lifetime: u32,
    size: f32,
    color: [f32; 3],
    alpha: f32,
    /// An item particle's part of its sprite, in atlas coordinates.
    rect: Option<[f32; 4]>,
    /// `Particle.gravity`, `friction` and `hasPhysics`.
    gravity: f64,
    friction: f64,
    physics: bool,
}

/// Every particle, and the random numbers they're made with.
pub struct Particles {
    particles: Vec<Particle>,
    seed: u64,
    /// The Particles option.
    pub status: mc_render::hud::options::Particles,
}

impl Default for Particles {
    fn default() -> Self {
        Self {
            particles: Vec::new(),
            seed: 0x2545_F491_4F6C_DD1D,
            status: Default::default(),
        }
    }
}

/// `ItemStackRenderState.pickParticleMaterial`: the sprite an item's particles are cut
/// from, a generated model's first layer.
pub fn item_sprite(
    icons: &std::collections::HashMap<String, mc_render::item::ItemIcon>,
    item: &str,
) -> Option<[f32; 4]> {
    match icons.get(item)? {
        mc_render::item::ItemIcon::Flat { layers, .. } => layers.first().map(|l| l.0),
        _ => None,
    }
}

/// A particle's box: 0.2 wide and high (`Particle`'s `setSize(0.2, 0.2)`).
const SIZE: f64 = 0.2;

impl Particles {
    /// A float in 0..1, from xorshift.
    pub fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    /// `ClientLevel.addParticle`'s check, before each particle: with `override_limiter`
    /// every one shows; otherwise none on Minimal (bar one in ten with `always_show`, which
    /// then counts as Decreased) and two in three on Decreased
    /// (`calculateParticleLevel`).
    pub fn allowed(&mut self, override_limiter: bool, always_show: bool) -> bool {
        use mc_render::hud::options::Particles as Status;
        if override_limiter {
            return true;
        }
        let mut status = self.status;
        if always_show && status == Status::Minimal && self.random() < 0.1 {
            status = Status::Decreased;
        }
        if status == Status::Decreased && self.random() < 1.0 / 3.0 {
            status = Status::Minimal;
        }
        status != Status::Minimal
    }

    /// A standard normal value (Box-Muller), as `RandomSource.nextGaussian`.
    pub fn gaussian(&mut self) -> f64 {
        let (a, b) = (self.random().max(1e-7) as f64, self.random() as f64);
        (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
    }

    /// `LivingEntity.makePoofParticles`: 20 puffs of smoke about a `width` × `height`
    /// box with its feet at `feet`, each set off a little the way it drifts.
    pub fn poof(&mut self, feet: DVec3, [width, height]: [f32; 2]) {
        for _ in 0..20 {
            if !self.allowed(false, false) {
                continue;
            }
            let drift = DVec3::new(self.gaussian(), self.gaussian(), self.gaussian()) * 0.02;
            // `getRandomX(1)`, `getRandomY` and `getRandomZ(1)`.
            let spot = DVec3::new(
                (2.0 * self.random() as f64 - 1.0) * width as f64,
                self.random() as f64 * height as f64,
                (2.0 * self.random() as f64 - 1.0) * width as f64,
            );
            self.explode(feet + spot - drift * 10.0, drift);
        }
    }

    /// `ExplodeParticle.Provider` (`ParticleTypes.POOF`): a puff of smoke at `pos`, moving
    /// at `velocity` give or take a little, rising as it slows.
    pub fn explode(&mut self, pos: DVec3, velocity: DVec3) {
        let jitter = DVec3::new(
            (self.random() * 2.0 - 1.0) as f64,
            (self.random() * 2.0 - 1.0) as f64,
            (self.random() * 2.0 - 1.0) as f64,
        ) * 0.05;
        let grey = self.random() * 0.3 + 0.7;
        let size = 0.1 * (self.random() * self.random() * 6.0 + 1.0);
        let lifetime = (16.0 / (self.random() as f64 * 0.8 + 0.2)) as u32 + 2;
        self.particles.push(Particle {
            kind: Kind::Poof,
            old: pos,
            pos,
            velocity: velocity + jitter,
            on_ground: false,
            stopped: false,
            age: 0,
            lifetime,
            size,
            color: [grey; 3],
            rect: None,
            alpha: 1.0,
            // Smoke rises.
            gravity: -0.1,
            friction: 0.9,
            physics: true,
        });
    }

    /// `CritParticle`'s providers: a spark (or dark heart) at `pos`, thrown by `aux`.
    pub fn crit(&mut self, kind: Kind, pos: DVec3, aux: DVec3) {
        // `DamageIndicatorProvider` throws its hearts upward.
        let aux = if kind == Kind::Damage {
            aux + DVec3::Y
        } else {
            aux
        };
        // `Particle`'s constructor, with no motion of its own: a random direction at a
        // random speed, a little upward.
        // Its lifetime is drawn, then replaced below.
        self.random();
        let mut d = DVec3::new(
            ((self.random() * 2.0 - 1.0) * 0.4) as f64,
            ((self.random() * 2.0 - 1.0) * 0.4) as f64,
            ((self.random() * 2.0 - 1.0) * 0.4) as f64,
        );
        let speed = ((self.random() + self.random() + 1.0) * 0.15) as f64;
        d = d / d.length() * speed * f64::from(0.4f32);
        d.y += f64::from(0.1f32);
        // `SingleQuadParticle`, then `CritParticle`: slowed, then thrown.
        let size = 0.1 * (self.random() * 0.5 + 0.5) * 2.0 * 0.75;
        let velocity = d * f64::from(0.1f32) + aux * 0.4;
        let grey = self.random() * 0.3 + 0.6;
        let lifetime = ((6.0 / (self.random() as f64 * 0.8 + 0.6)) as u32).max(1);
        let mut p = Particle {
            kind,
            old: pos,
            pos,
            velocity,
            on_ground: false,
            stopped: false,
            age: 0,
            lifetime,
            size,
            color: [grey; 3],
            rect: None,
            alpha: 1.0,
            gravity: 0.5,
            friction: 0.7,
            physics: false,
        };
        // The constructor ends with a tick; the providers then tint and time it.
        p.tick(None);
        match kind {
            Kind::EnchantedHit => {
                p.color[0] *= 0.3;
                p.color[1] *= 0.8;
            }
            Kind::Damage => p.lifetime = 20,
            _ => {}
        }
        self.particles.push(p);
    }

    /// `LivingEntity.spawnItemParticles`: `count` crumbs of the item whose sprite is
    /// `sprite` (atlas coordinates), from in front of the mouth of an entity with its eye
    /// at `eye` looking at `yaw` and `pitch`, tossed forward and up.
    pub fn item_crumbs(&mut self, sprite: [f32; 4], eye: DVec3, yaw: f32, pitch: f32, count: u32) {
        // `Vec3.xRot(-xRot)` then `yRot(-yRot)`.
        let (x_rot, y_rot) = (-pitch.to_radians() as f64, -yaw.to_radians() as f64);
        let turn = |v: DVec3| {
            let (s, c) = x_rot.sin_cos();
            let v = DVec3::new(v.x, v.y * c + v.z * s, v.z * c - v.y * s);
            let (s, c) = y_rot.sin_cos();
            DVec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s)
        };
        for _ in 0..count {
            let d = DVec3::new(
                (self.random() as f64 - 0.5) * 0.1,
                self.random() as f64 * 0.1 + 0.1,
                0.0,
            );
            let d = turn(d);
            let y = -self.random() as f64 * 0.6 - 0.3;
            let p = DVec3::new((self.random() as f64 - 0.5) * 0.3, y, 0.6);
            let pos = eye + turn(p);
            // `ClientLevel.addParticle`, which the Particles option limits.
            if self.allowed(false, false) {
                self.item(sprite, pos, d + DVec3::Y * 0.05);
            }
        }
    }

    /// `BreakingItemParticle`: a random quarter of the item sprite `sprite` (atlas
    /// coordinates), flipped across, half the usual size, at `pos` moving at `velocity`
    /// (its own motion is none), falling.
    pub fn item(&mut self, sprite: [f32; 4], pos: DVec3, velocity: DVec3) {
        let lifetime = (4.0 / (self.random() * 0.9 + 0.1)) as u32;
        let size = 0.1 * (self.random() * 0.5 + 0.5) * 2.0 / 2.0;
        let (uo, vo) = (self.random() * 3.0, self.random() * 3.0);
        let [u0, v0, u1, v1] = sprite;
        let u = |f: f32| u0 + (u1 - u0) * f;
        let v = |f: f32| v0 + (v1 - v0) * f;
        self.particles.push(Particle {
            kind: Kind::Item,
            old: pos,
            pos,
            velocity,
            on_ground: false,
            stopped: false,
            age: 0,
            lifetime,
            size,
            color: [1.0; 3],
            rect: Some([
                u((uo + 1.0) / 4.0),
                v(vo / 4.0),
                u(uo / 4.0),
                v((vo + 1.0) / 4.0),
            ]),
            alpha: 1.0,
            gravity: 1.0,
            friction: 0.98,
            physics: true,
        });
    }

    /// `TerrainParticle`: a random quarter of a block's particle sprite `sprite` (atlas
    /// coordinates), flipped across, 0.6 grey times the block's `tint`, half the usual size,
    /// at `pos`, set off about `aux` as `Particle`'s constructor does, falling. A block
    /// being cracked sheds slower, smaller ones (`setPower(0.2)` and `scale(0.6)`).
    pub fn terrain(
        &mut self,
        sprite: [f32; 4],
        tint: [f32; 3],
        pos: DVec3,
        aux: DVec3,
        cracking: bool,
    ) {
        let lifetime = (4.0 / (self.random() * 0.9 + 0.1)) as u32;
        let mut d = DVec3::new(
            aux.x + ((self.random() * 2.0 - 1.0) * 0.4) as f64,
            aux.y + ((self.random() * 2.0 - 1.0) * 0.4) as f64,
            aux.z + ((self.random() * 2.0 - 1.0) * 0.4) as f64,
        );
        let speed = ((self.random() + self.random() + 1.0) * 0.15) as f64;
        d = d / d.length() * speed * f64::from(0.4f32);
        d.y += f64::from(0.1f32);
        let mut size = 0.1 * (self.random() * 0.5 + 0.5) * 2.0 / 2.0;
        let (uo, vo) = (self.random() * 3.0, self.random() * 3.0);
        if cracking {
            d.x *= f64::from(0.2f32);
            d.y = (d.y - f64::from(0.1f32)) * f64::from(0.2f32) + f64::from(0.1f32);
            d.z *= f64::from(0.2f32);
            size *= 0.6;
        }
        let [u0, v0, u1, v1] = sprite;
        let u = |f: f32| u0 + (u1 - u0) * f;
        let v = |f: f32| v0 + (v1 - v0) * f;
        self.particles.push(Particle {
            kind: Kind::Terrain,
            old: pos,
            pos,
            velocity: d,
            on_ground: false,
            stopped: false,
            age: 0,
            lifetime,
            size,
            color: tint.map(|c| 0.6 * c),
            alpha: 1.0,
            rect: Some([
                u((uo + 1.0) / 4.0),
                v(vo / 4.0),
                u(uo / 4.0),
                v((vo + 1.0) / 4.0),
            ]),
            gravity: 1.0,
            friction: 0.98,
            physics: true,
        });
    }

    /// `getRandomX(0.5)`, `getRandomY` and `getRandomZ(0.5)`: somewhere in a `width` ×
    /// `height` box with its bottom middle at `feet`.
    pub fn in_box(&mut self, feet: DVec3, [width, height]: [f32; 2]) -> DVec3 {
        let (x, y, z) = (self.random(), self.random(), self.random());
        let [width, height] = [width, height].map(f64::from);
        feet + DVec3::new(
            (2.0 * x as f64 - 1.0) * width * 0.5,
            y as f64 * height,
            (2.0 * z as f64 - 1.0) * width * 0.5,
        )
    }

    /// `LivingEntity.tickEffects` on the client: one time in 4 (in 15 if it's invisible,
    /// five times as rarely if its effects are all ambient, as a beacon's are) a swirl of
    /// one of its effects' `colors`, somewhere in its box, rising.
    pub fn mob_effects(
        &mut self,
        colors: &[u32],
        invisible: bool,
        ambient: bool,
        feet: DVec3,
        size: [f32; 2],
    ) {
        if colors.is_empty() {
            return;
        }
        let bound = if invisible { 15 } else { 4 } * if ambient { 5 } else { 1 };
        if (self.random() * bound as f32) as u32 != 0 {
            return;
        }
        let color = colors[(self.random() * colors.len() as f32) as usize % colors.len()];
        let at = self.in_box(feet, size);
        if self.allowed(false, false) {
            self.effect(at, DVec3::ONE, color);
        }
    }

    /// `SpellParticle.MobEffectProvider`: a swirl at `pos` tinted `argb`, its motion a
    /// random sideways drift and `aux`'s height; with no sideways `aux` it barely drifts.
    pub fn effect(&mut self, pos: DVec3, aux: DVec3, argb: u32) {
        let channel = |shift: u32| ((argb >> shift) & 0xff) as f32 / 255.0;
        let tint = ([channel(16), channel(8), channel(0)], channel(24));
        self.drift(Kind::Effect, pos, aux, |_| tint);
    }

    /// `PlayerCloudParticle.Provider`: a cloud at `pos`, drifting a little and moving at
    /// `aux`. (It isn't pulled down onto players within two blocks yet.)
    pub fn cloud(&mut self, pos: DVec3, aux: DVec3) {
        // `Particle`'s constructor: its lifetime (replaced below), then a random direction
        // at a random speed, a little upward, slowed to a tenth.
        self.random();
        let mut d = DVec3::new(
            ((self.random() * 2.0 - 1.0) * 0.4) as f64,
            ((self.random() * 2.0 - 1.0) * 0.4) as f64,
            ((self.random() * 2.0 - 1.0) * 0.4) as f64,
        );
        let speed = ((self.random() + self.random() + 1.0) * 0.15) as f64;
        d = d / d.length() * speed * f64::from(0.4f32);
        d.y += f64::from(0.1f32);
        let size = 0.1 * (self.random() * 0.5 + 0.5) * 2.0 * 1.875;
        let grey = 1.0 - self.random() * 0.3;
        let base = (8.0 / (self.random() as f64 * 0.8 + 0.3)) as i32;
        let lifetime = (base as f32 * 2.5).max(1.0) as u32;
        self.particles.push(Particle {
            kind: Kind::Cloud,
            old: pos,
            pos,
            velocity: d * f64::from(0.1f32) + aux,
            on_ground: false,
            stopped: false,
            age: 0,
            lifetime,
            size,
            color: [grey; 3],
            alpha: 1.0,
            rect: None,
            gravity: 0.0,
            friction: 0.96,
            physics: false,
        });
    }

    /// `PortalParticle.Provider`: a speck at `pos` that sweeps through `offset` back to
    /// `pos` over its life.
    pub fn portal(&mut self, pos: DVec3, offset: DVec3) {
        // `Particle`'s and `SingleQuadParticle`'s lifetime and size, both replaced, and
        // `SpriteSet.get(random)`.
        self.random();
        self.random();
        let sprite = (self.random() * 8.0).min(7.0) as u8;
        let size = 0.1 * (self.random() * 0.2 + 0.5);
        let br = self.random() * 0.6 + 0.4;
        let lifetime = (self.random() * 10.0) as u32 + 40;
        self.particles.push(Particle {
            kind: Kind::Portal { start: pos, sprite },
            old: pos,
            pos,
            velocity: offset,
            on_ground: false,
            stopped: false,
            age: 0,
            lifetime,
            size,
            color: [br * 0.9, br * 0.3, br],
            alpha: 1.0,
            rect: None,
            gravity: 0.0,
            friction: 1.0,
            physics: false,
        });
    }

    /// `ClientLevel.addParticle(PORTAL)` 128 times along a teleport, from `from` to `to`, about a
    /// box `width` × `height` (`LivingEntity.handleEntityEvent(46)` and
    /// `LevelEventHandler`'s 2018).
    pub fn teleport(&mut self, from: DVec3, to: DVec3, [width, height]: [f64; 2], spread: bool) {
        for i in 0..128 {
            // An entity's spreads evenly along the way; level event 2018's at random.
            let d = if spread {
                f64::from(i) / 127.0
            } else {
                f64::from(self.random())
            };
            let mut r = || f64::from((self.random() - 0.5) * 0.2);
            let offset = DVec3::new(r(), r(), r());
            let at = from.lerp(to, d)
                + DVec3::new(
                    (f64::from(self.random()) - 0.5) * width * 2.0,
                    f64::from(self.random()) * height,
                    (f64::from(self.random()) - 0.5) * width * 2.0,
                );
            if self.allowed(false, false) {
                self.portal(at, offset);
            }
        }
    }

    /// `SquidInkParticle.Provider` (`glow` false, black) and `GlowInkProvider` (teal): a
    /// puff at `pos` moving at `velocity`, half a block across.
    pub fn ink(&mut self, pos: DVec3, velocity: DVec3, glow: bool) {
        // `Particle`'s constructor's lifetime and `SingleQuadParticle`'s size, both
        // replaced.
        self.random();
        self.random();
        let size = 0.5f32;
        let lifetime = (size * 12.0 / (self.random() * 0.8 + 0.2)) as u32;
        let color = if glow { [0.2, 0.8, 0.6] } else { [0.0; 3] };
        self.particles.push(Particle {
            kind: Kind::Ink,
            old: pos,
            pos,
            velocity,
            on_ground: false,
            stopped: false,
            age: 0,
            lifetime,
            size,
            color,
            alpha: 1.0,
            rect: None,
            gravity: 0.0,
            friction: 0.92,
            physics: false,
        });
    }

    /// `GlowParticle.GlowSquidProvider`: a glow at `pos`, drifting as a swirl does, light
    /// or dark teal at random, weightless.
    pub fn glow(&mut self, pos: DVec3) {
        self.drift(Kind::Glow, pos, DVec3::ZERO, |p| {
            let color = if p.random() < 0.5 {
                [0.6, 1.0, 0.8]
            } else {
                [0.08, 0.4, 0.4]
            };
            (color, 1.0)
        });
    }

    /// What `SpellParticle.MobEffectProvider` and `GlowSquidProvider` share: a particle at
    /// `pos`, its motion a random sideways drift and `aux`'s height, slowed (with no
    /// sideways `aux`, it barely drifts); `tint` picks its colour and alpha after it's
    /// made, before its lifetime.
    fn drift(
        &mut self,
        kind: Kind,
        pos: DVec3,
        aux: DVec3,
        tint: impl FnOnce(&mut Self) -> ([f32; 3], f32),
    ) {
        let xa = 0.5 - self.random() as f64;
        let za = 0.5 - self.random() as f64;
        // `Particle`'s constructors: its lifetime (replaced below), then a direction
        // about the one given at a random speed, a little upward.
        self.random();
        let mut d = DVec3::new(
            xa + ((self.random() * 2.0 - 1.0) * 0.4) as f64,
            aux.y + ((self.random() * 2.0 - 1.0) * 0.4) as f64,
            za + ((self.random() * 2.0 - 1.0) * 0.4) as f64,
        );
        let speed = ((self.random() + self.random() + 1.0) * 0.15) as f64;
        d = d / d.length() * speed * f64::from(0.4f32);
        d.y += f64::from(0.1f32);
        // `SingleQuadParticle`, then `SpellParticle` or `GlowParticle`: three quarters
        // the size, rising slowly.
        let size = 0.1 * (self.random() * 0.5 + 0.5) * 2.0 * 0.75;
        d.y *= f64::from(0.2f32);
        if aux.x == 0.0 && aux.z == 0.0 {
            d.x *= f64::from(0.1f32);
            d.z *= f64::from(0.1f32);
        }
        let (color, alpha) = tint(self);
        let lifetime = (8.0 / (self.random() as f64 * 0.8 + 0.2)) as u32;
        self.particles.push(Particle {
            kind,
            old: pos,
            pos,
            velocity: d,
            on_ground: false,
            stopped: false,
            age: 0,
            lifetime,
            size,
            color,
            alpha,
            rect: None,
            // `SpellParticle` floats up; a glow has no weight.
            gravity: if kind == Kind::Effect { -0.1 } else { 0.0 },
            friction: 0.96,
            physics: false,
        });
    }

    /// `Particle.tick` for each, and gone once it's lived its life.
    pub fn tick(&mut self, world: &impl mc_physics::World, blocks: &mc_physics::BlockPhysics) {
        let collide =
            |pos, motion| mc_physics::collide_box(world, blocks, pos, [SIZE, SIZE], motion);
        self.particles.retain_mut(|p| {
            if !p.tick(Some(&collide)) {
                return false;
            }
            // `SquidInkParticle.tick`: it fades over the second half of its life, and sinks
            // while it's in air.
            if p.kind == Kind::Ink {
                let half = p.lifetime / 2;
                if p.age > half {
                    p.alpha = 1.0 - (p.age - half) as f32 / p.lifetime as f32;
                }
                let b = p.pos.floor().as_ivec3();
                if blocks.is_air(world.state(b.x, b.y, b.z)) {
                    p.velocity.y -= f64::from(0.0074f32);
                }
            }
            true
        });
    }

    /// What to draw, `pt` of the way to the next tick, lit by `light` where each is.
    pub fn render(
        &self,
        pt: f32,
        light: impl Fn(DVec3) -> [u8; 2],
    ) -> Vec<mc_render::particle::Particle> {
        use mc_render::particle::{CRIT, DAMAGE, EFFECT, ENCHANTED_HIT, GLOW, POOF};
        self.particles
            .iter()
            .map(|p| mc_render::particle::Particle {
                pos: p.old.lerp(p.pos, pt as f64).as_vec3(),
                // `PlayerCloudParticle.getQuadSize`: it swells to full size over the first
                // thirty-second of its life.
                size: match p.kind {
                    Kind::Cloud => {
                        p.size * ((p.age as f32 + pt) / p.lifetime as f32 * 32.0).clamp(0.0, 1.0)
                    }
                    // `PortalParticle.getQuadSize`: it grows in, quickly at first.
                    Kind::Portal { .. } => {
                        let s = 1.0 - (p.age as f32 + pt) / p.lifetime as f32;
                        p.size * (1.0 - s * s)
                    }
                    _ => p.size,
                },
                sprite: match p.kind {
                    // `SpriteSet.get(age, lifetime)`.
                    Kind::Poof | Kind::Ink | Kind::Cloud => {
                        POOF[(p.age as usize * (POOF.len() - 1)) / p.lifetime.max(1) as usize]
                    }
                    Kind::Crit => CRIT,
                    Kind::EnchantedHit => ENCHANTED_HIT,
                    Kind::Damage => DAMAGE,
                    Kind::Item | Kind::Terrain => "",
                    Kind::Effect => {
                        EFFECT[(p.age as usize * (EFFECT.len() - 1)) / p.lifetime as usize]
                    }
                    Kind::Glow => GLOW,
                    Kind::Portal { sprite, .. } => POOF[POOF.len() - 1 - sprite as usize],
                },
                rect: p.rect,
                color: p.color,
                alpha: p.alpha,
                // `SimpleAnimatedParticle`: blended, and always fully lit.
                translucent: matches!(p.kind, Kind::Effect | Kind::Ink | Kind::Cloud),
                light: if p.kind == Kind::Ink {
                    [15, 15]
                } else {
                    light(p.pos)
                },
                // `GlowParticle.getLightCoords`: block light grows to full over its life.
                emission: match p.kind {
                    Kind::Glow => (p.age as f32 + pt) / p.lifetime as f32,
                    // `PortalParticle.getLightCoords`: brighter by its age to the fourth.
                    Kind::Portal { .. } => (p.age as f32 / p.lifetime as f32).powi(4),
                    _ => 0.0,
                },
            })
            .collect()
    }
}

impl Particle {
    /// `Particle.tick` (and `CritParticle.tick`'s fade to red): returns whether it lives
    /// on. Without the world it moves as if nothing were in the way.
    fn tick(&mut self, collide: Option<&dyn Fn(DVec3, DVec3) -> DVec3>) -> bool {
        self.old = self.pos;
        if self.age >= self.lifetime {
            return false;
        }
        self.age += 1;
        // `PortalParticle.tick`: not moved by its motion but placed along its curve.
        if let Kind::Portal { start, .. } = self.kind {
            let t = self.age as f32 / self.lifetime as f32;
            let back = 1.0 - (-t + t * t * 2.0);
            self.pos = start + self.velocity * f64::from(back) + DVec3::Y * f64::from(1.0 - t);
            return true;
        }
        self.velocity.y -= 0.04 * self.gravity;
        // `Particle.move`: its position is its box's bottom middle.
        match collide.filter(|_| self.physics) {
            Some(collide) if !self.stopped => {
                let motion = self.velocity;
                let moved = collide(self.pos, motion);
                self.pos += moved;
                if motion.y.abs() >= 1e-5 && moved.y.abs() < 1e-5 {
                    self.stopped = true;
                }
                self.on_ground = moved.y != motion.y && motion.y < 0.0;
                if moved.x != motion.x {
                    self.velocity.x = 0.0;
                }
                if moved.z != motion.z {
                    self.velocity.z = 0.0;
                }
            }
            Some(_) => {}
            None => self.pos += self.velocity,
        }
        // `speedUpWhenYMotionIsBlocked`.
        if matches!(self.kind, Kind::Effect | Kind::Glow) && self.pos.y == self.old.y {
            self.velocity.x *= 1.1;
            self.velocity.z *= 1.1;
        }
        self.velocity *= self.friction;
        if self.on_ground {
            self.velocity.x *= 0.7;
            self.velocity.z *= 0.7;
        }
        if matches!(self.kind, Kind::Crit | Kind::EnchantedHit | Kind::Damage) {
            self.color[1] *= 0.96;
            self.color[2] *= 0.9;
        }
        true
    }
}
