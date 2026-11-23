//! The block the player looks at: `BlockGetter.clip` with outline shapes and no fluids, as
//! `Entity.pick` runs it, walking the blocks along the ray (`traverseBlocks`) and clipping
//! the ray against each one's boxes (`VoxelShape.clip`, `AABB.clip`).

use crate::{Aabb, BlockPhysics, World};
use glam::{DVec3, IVec3};

/// Where a ray met a block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockHit {
    pub pos: IVec3,
    /// The face hit, as `Direction.get3DDataValue`: down 0, up 1, north 2, south 3, west 4,
    /// east 5.
    pub face: u8,
    /// The point hit, in world coordinates.
    pub location: DVec3,
    /// The ray started inside the block's shape.
    pub inside: bool,
}

/// The direction the player looks for these angles in degrees
/// (`Entity.calculateViewVector`).
pub fn view_vector(yaw: f32, pitch: f32) -> DVec3 {
    let pitch = pitch * (std::f32::consts::PI / 180.0);
    let yaw = -yaw * (std::f32::consts::PI / 180.0);
    let (cos_yaw, sin_yaw) = (yaw.cos(), yaw.sin());
    let (cos_pitch, sin_pitch) = (pitch.cos(), pitch.sin());
    DVec3::new(
        (sin_yaw * cos_pitch) as f64,
        -sin_pitch as f64,
        (cos_yaw * cos_pitch) as f64,
    )
}

/// The first block whose outline the segment from `from` to `to` passes through.
pub fn pick(
    world: &(impl World + ?Sized),
    blocks: &BlockPhysics,
    from: DVec3,
    to: DVec3,
) -> Option<BlockHit> {
    clip(from, to, |pos| {
        blocks.outline(world.state(pos.x, pos.y, pos.z))
    })
}

/// `Level.clip` with `ClipContext.Block.COLLIDER` and no fluids, as projectiles move: the
/// first block whose collision shape the segment from `from` to `to` passes through.
pub fn clip_collider(
    world: &(impl World + ?Sized),
    blocks: &BlockPhysics,
    from: DVec3,
    to: DVec3,
) -> Option<BlockHit> {
    clip(from, to, |pos| {
        blocks.shape(world.state(pos.x, pos.y, pos.z))
    })
}

/// `BlockGetter.clip`: walks the blocks along the segment, clipping it against each one's
/// boxes as `shape` gives them.
fn clip<'a>(from: DVec3, to: DVec3, shape: impl Fn(IVec3) -> &'a [Aabb]) -> Option<BlockHit> {
    if from == to {
        return None;
    }
    let clip_at = |pos: IVec3| clip_shape(shape(pos), from, to, pos);
    // Both ends pulled a hair inward, so a ray ending exactly on a block edge doesn't visit
    // the block beyond it.
    let end = to.lerp(from, -1.0e-7);
    let start = from.lerp(to, -1.0e-7);
    let mut pos = start.floor().as_ivec3();
    if let Some(hit) = clip_at(pos) {
        return Some(hit);
    }
    let d = end - start;
    let step = IVec3::new(sign(d.x), sign(d.y), sign(d.z));
    let delta = DVec3::new(
        if step.x == 0 {
            f64::MAX
        } else {
            step.x as f64 / d.x
        },
        if step.y == 0 {
            f64::MAX
        } else {
            step.y as f64 / d.y
        },
        if step.z == 0 {
            f64::MAX
        } else {
            step.z as f64 / d.z
        },
    );
    let frac = start - start.floor();
    let mut t = DVec3::new(
        delta.x * if step.x > 0 { 1.0 - frac.x } else { frac.x },
        delta.y * if step.y > 0 { 1.0 - frac.y } else { frac.y },
        delta.z * if step.z > 0 { 1.0 - frac.z } else { frac.z },
    );
    while t.x <= 1.0 || t.y <= 1.0 || t.z <= 1.0 {
        if t.x < t.y {
            if t.x < t.z {
                pos.x += step.x;
                t.x += delta.x;
            } else {
                pos.z += step.z;
                t.z += delta.z;
            }
        } else if t.y < t.z {
            pos.y += step.y;
            t.y += delta.y;
        } else {
            pos.z += step.z;
            t.z += delta.z;
        }
        if let Some(hit) = clip_at(pos) {
            return Some(hit);
        }
    }
    None
}

fn sign(v: f64) -> i32 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

/// `VoxelShape.clip`: a ray starting inside the shape hits it where it starts, on the face
/// facing back along the ray; otherwise the nearest box face it crosses.
fn clip_shape(shape: &[Aabb], from: DVec3, to: DVec3, pos: IVec3) -> Option<BlockHit> {
    if shape.is_empty() {
        return None;
    }
    let ray = to - from;
    if ray.length_squared() < 1.0e-7 {
        return None;
    }
    let origin = pos.as_dvec3();
    let nudged = from + ray * 0.001;
    let local = nudged - origin;
    if shape
        .iter()
        .any(|b| (0..3).all(|i| b.min[i] <= local[i] && local[i] < b.max[i]))
    {
        return Some(BlockHit {
            pos,
            face: nearest_direction(-ray),
            location: nudged,
            inside: true,
        });
    }
    // `AABB.clip`: the nearest face crossing over all the boxes.
    let mut nearest: Option<(f64, u8)> = None;
    for b in shape {
        if let Some((t, face)) = b.offset(origin).clip(from, to)
            && nearest.is_none_or(|(n, _)| t < n)
        {
            nearest = Some((t, face));
        }
    }
    nearest.map(|(t, face)| BlockHit {
        pos,
        face,
        location: from + ray * t,
        inside: false,
    })
}

/// `Direction.getApproximateNearest`: the direction closest to `v`.
fn nearest_direction(v: DVec3) -> u8 {
    // In `Direction.values()` order: down, up, north, south, west, east.
    let normals = [
        DVec3::NEG_Y,
        DVec3::Y,
        DVec3::NEG_Z,
        DVec3::Z,
        DVec3::NEG_X,
        DVec3::X,
    ];
    let mut best = (2u8, f64::MIN);
    for (i, n) in normals.iter().enumerate() {
        let dot = n.dot(v);
        if dot > best.1 {
            best = (i as u8, dot);
        }
    }
    best.0
}
