//! Particles: `SingleQuadParticle`s, sprites on quads that face the camera. Only the poof
//! of smoke a mob leaves when it dies (`ExplodeParticle`) is made yet.

use crate::atlas::Atlas;
use crate::mesh::Vertex;
use glam::{EulerRot, Quat, Vec3};
use std::f32::consts::PI;

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

/// A particle as drawn this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    /// Its middle, in world space.
    pub pos: Vec3,
    /// Half its side (`quadSize`), in blocks.
    pub size: f32,
    /// Its sprite's id.
    pub sprite: &'static str,
    /// Its grey (`rCol`, `gCol` and `bCol`, all the same for a poof).
    pub grey: f32,
    /// The sky and block light where it is.
    pub light: [u8; 2],
}

/// `QuadParticleRenderState.renderRotatedQuad`: each particle as a square facing the
/// camera (`FacingCameraMode.LOOKAT_XYZ`, the camera's own rotation), tinted and lit, to
/// draw with the cutout entities (`RenderPipelines.OPAQUE_PARTICLE`).
pub fn vertices(atlas: &Atlas, camera: &crate::gpu::Camera, particles: &[Particle]) -> Vec<Vertex> {
    // `Camera.setRotation`.
    let rotation = Quat::from_euler(
        EulerRot::YXZ,
        PI - camera.yaw.to_radians(),
        -camera.pitch.to_radians(),
        0.0,
    );
    let mut out = Vec::with_capacity(particles.len() * 4);
    for p in particles {
        let [u0, v0, u1, v1] = atlas.sprite(p.sprite).rect;
        let grey = (p.grey * 255.0).round() as u8;
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
                color: [grey, grey, grey, 255],
                light: [sky.min(15) * 16, block.min(15) * 16, 0, 0],
            });
        }
    }
    out
}
