//! The particles in the world, moved each tick: `Particle.tick` and `move`, for the poofs
//! of smoke dying mobs leave (`ExplodeParticle`).

use glam::DVec3;

/// A poof: where it is now and last tick, its motion, and how far through its life.
struct Poof {
    old: DVec3,
    pos: DVec3,
    velocity: DVec3,
    on_ground: bool,
    /// `stoppedByCollision`: once a fall is stopped dead, it never moves again.
    stopped: bool,
    age: u32,
    lifetime: u32,
    size: f32,
    grey: f32,
}

/// Every particle, and the random numbers they're made with.
pub struct Particles {
    poofs: Vec<Poof>,
    seed: u64,
}

impl Default for Particles {
    fn default() -> Self {
        Self {
            poofs: Vec::new(),
            seed: 0x2545_F491_4F6C_DD1D,
        }
    }
}

/// A particle's box: 0.2 wide and high (`Particle`'s `setSize(0.2, 0.2)`).
const SIZE: f64 = 0.2;

impl Particles {
    /// A float in 0..1, from xorshift.
    fn random(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        (self.seed >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A standard normal value (Box-Muller), as `RandomSource.nextGaussian`.
    fn gaussian(&mut self) -> f64 {
        let (a, b) = (self.random().max(1e-7) as f64, self.random() as f64);
        (-2.0 * a.ln()).sqrt() * (std::f64::consts::TAU * b).cos()
    }

    /// `LivingEntity.makePoofParticles`: 20 puffs of smoke about a `width` × `height`
    /// box with its feet at `feet`, each set off a little the way it drifts.
    pub fn poof(&mut self, feet: DVec3, [width, height]: [f32; 2]) {
        for _ in 0..20 {
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
            self.poofs.push(Poof {
                old: pos,
                pos,
                velocity: drift + jitter,
                on_ground: false,
                stopped: false,
                age: 0,
                lifetime,
                size,
                grey,
            });
        }
    }

    /// `Particle.tick` for each: smoke rises (gravity -0.1), slows by a tenth a tick, and
    /// goes once it's lived its life.
    pub fn tick(&mut self, world: &impl mc_physics::World, blocks: &mc_physics::BlockPhysics) {
        self.poofs.retain_mut(|p| {
            p.old = p.pos;
            if p.age >= p.lifetime {
                return false;
            }
            p.age += 1;
            p.velocity.y += 0.04 * 0.1;
            // `Particle.move`: its position is its box's bottom middle.
            if !p.stopped {
                let motion = p.velocity;
                let moved = mc_physics::collide_box(world, blocks, p.pos, [SIZE, SIZE], motion);
                p.pos += moved;
                if motion.y.abs() >= 1e-5 && moved.y.abs() < 1e-5 {
                    p.stopped = true;
                }
                p.on_ground = moved.y != motion.y && motion.y < 0.0;
                if moved.x != motion.x {
                    p.velocity.x = 0.0;
                }
                if moved.z != motion.z {
                    p.velocity.z = 0.0;
                }
            }
            p.velocity *= 0.9;
            if p.on_ground {
                p.velocity.x *= 0.7;
                p.velocity.z *= 0.7;
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
        let sprites = mc_render::particle::POOF;
        self.poofs
            .iter()
            .map(|p| mc_render::particle::Particle {
                pos: p.old.lerp(p.pos, pt as f64).as_vec3(),
                size: p.size,
                // `SpriteSet.get(age, lifetime)`.
                sprite: sprites[(p.age as usize * (sprites.len() - 1)) / p.lifetime as usize],
                grey: p.grey,
                light: light(p.pos),
            })
            .collect()
    }
}
