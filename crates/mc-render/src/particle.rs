//! Particles: `SingleQuadParticle`s, sprites on quads that face the camera: the poof of
//! smoke a mob leaves when it dies (`ExplodeParticle`), and the sparks and dark hearts of
//! a hit (`CritParticle`), and crumbs of an item (`BreakingItemParticle`).

use crate::atlas::Atlas;
use crate::mesh::Vertex;
use glam::Vec3;

/// `particles/poof.json`'s sprites, in order: a poof starts as `generic_7` and shrinks to
/// `generic_0`.
pub const POOF: [&str; 8] = [
    "minecraft:particle/generic_7",
    "minecraft:particle/generic_6",
    "minecraft:particle/generic_5",
    "minecraft:particle/generic_4",
    "minecraft:particle/generic_3",
    "minecraft:particle/generic_2",
    "minecraft:particle/generic_1",
    "minecraft:particle/generic_0",
];

/// `particles/entity_effect.json`'s sprites, in order: a potion swirl starts as
/// `effect_7` and ends as `effect_0`.
pub const EFFECT: [&str; 8] = [
    "minecraft:particle/effect_7",
    "minecraft:particle/effect_6",
    "minecraft:particle/effect_5",
    "minecraft:particle/effect_4",
    "minecraft:particle/effect_3",
    "minecraft:particle/effect_2",
    "minecraft:particle/effect_1",
    "minecraft:particle/effect_0",
];

/// `particles/crit.json`, `enchanted_hit.json` and `damage_indicator.json`'s sprites.
pub const CRIT: &str = "minecraft:particle/critical_hit";
pub const ENCHANTED_HIT: &str = "minecraft:particle/enchanted_hit";
pub const DAMAGE: &str = "minecraft:particle/damage";

/// A particle as drawn this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    /// Its middle, in world space.
    pub pos: Vec3,
    /// Half its side (`quadSize`), in blocks.
    pub size: f32,
    /// Its sprite's id.
    pub sprite: &'static str,
    /// Or the part of a sprite it shows, in atlas coordinates, as an item's crumbs do.
    pub rect: Option<[f32; 4]>,
    /// Its tint (`rCol`, `gCol` and `bCol`) and `alpha`.
    pub color: [f32; 3],
    pub alpha: f32,
    /// Whether it blends over what's behind it (`Layer.TRANSLUCENT`) rather than being
    /// cut out (`Layer.OPAQUE`).
    pub translucent: bool,
    /// The sky and block light where it is.
    pub light: [u8; 2],
}

/// `QuadParticleRenderState.renderRotatedQuad`: each particle as a square facing the
/// camera (`FacingCameraMode.LOOKAT_XYZ`, the camera's own rotation), tinted and lit, to
/// draw with the cutout entities (`RenderPipelines.OPAQUE_PARTICLE`), or blended
/// (`TRANSLUCENT_PARTICLE`) as the caller sorts them.
pub fn vertices(atlas: &Atlas, camera: &crate::gpu::Camera, particles: &[Particle]) -> Vec<Vertex> {
    let rotation = camera.rotation();
    let mut out = Vec::with_capacity(particles.len() * 4);
    for p in particles {
        let [u0, v0, u1, v1] = p.rect.unwrap_or_else(|| atlas.sprite(p.sprite).rect);
        let [r, g, b, a] = [p.color[0], p.color[1], p.color[2], p.alpha]
            .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        let [sky, block] = p.light;
        for ([x, y], uv) in [
            ([1.0, -1.0], [u1, v1]),
            ([1.0, 1.0], [u1, v0]),
            ([-1.0, 1.0], [u0, v0]),
            ([-1.0, -1.0], [u0, v1]),
        ] {
            out.push(Vertex {
                pos: (p.pos + rotation * Vec3::new(x, y, 0.0) * p.size).to_array(),
                uv,
                color: [r, g, b, a],
                light: [sky.min(15) * 16, block.min(15) * 16, 0, 0],
            });
        }
    }
    out
}
