//! The particles in the world, moved each tick: `Particle.tick` and `move`, for the poofs
//! of smoke dying mobs leave (`ExplodeParticle`) and the sparks and dark hearts of hits
//! (`CritParticle`).

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
            let pos = feet + spot - drift * 10.0;
            // `ExplodeParticle`'s constructor.
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
                velocity: drift + jitter,
                on_ground: false,
                stopped: false,
                age: 0,
                lifetime,
                size,
                color: [grey; 3],
                // Smoke rises.
                gravity: -0.1,
                friction: 0.9,
                physics: true,
            });
        }
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

    /// `Particle.tick` for each, and gone once it's lived its life.
    pub fn tick(&mut self, world: &impl mc_physics::World, blocks: &mc_physics::BlockPhysics) {
        let collide =
            |pos, motion| mc_physics::collide_box(world, blocks, pos, [SIZE, SIZE], motion);
        self.particles.retain_mut(|p| p.tick(Some(&collide)));
    }

    /// What to draw, `pt` of the way to the next tick, lit by `light` where each is.
    pub fn render(
        &self,
        pt: f32,
        light: impl Fn(DVec3) -> [u8; 2],
    ) -> Vec<mc_render::particle::Particle> {
        use mc_render::particle::{CRIT, DAMAGE, ENCHANTED_HIT, POOF};
        self.particles
            .iter()
            .map(|p| mc_render::particle::Particle {
                pos: p.old.lerp(p.pos, pt as f64).as_vec3(),
                size: p.size,
                sprite: match p.kind {
                    // `SpriteSet.get(age, lifetime)`.
                    Kind::Poof => POOF[(p.age as usize * (POOF.len() - 1)) / p.lifetime as usize],
                    Kind::Crit => CRIT,
                    Kind::EnchantedHit => ENCHANTED_HIT,
                    Kind::Damage => DAMAGE,
                },
                color: p.color,
                light: light(p.pos),
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
        self.velocity *= self.friction;
        if self.on_ground {
            self.velocity.x *= 0.7;
            self.velocity.z *= 0.7;
        }
        if self.kind != Kind::Poof {
            self.color[1] *= 0.96;
            self.color[2] *= 0.9;
        }
        true
    }
}
