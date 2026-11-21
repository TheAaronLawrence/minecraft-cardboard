//! Water and lava as movement sees them: each state's `FluidState`, its height
//! (`FlowingFluid.getHeight`, `FluidState.getHeightForCamera`), the way it flows
//! (`FlowingFluid.getFlow`), and what an entity's box touches (`EntityFluidInteraction`).

use crate::collide::Aabb;
use crate::{BlockPhysics, World};
use glam::DVec3;

/// `Direction` ordinals, the bits of `BlockPhysics::sturdy`.
const DOWN: u8 = 0;
/// `Direction.Plane.HORIZONTAL`, in order: north, east, south, west, with their ordinals.
const HORIZONTAL: [([i32; 2], u8); 4] = [([0, -1], 2), ([1, 0], 5), ([0, 1], 3), ([-1, 0], 4)];
/// The `sturdy` bit for `IceBlock`, whose faces never stop a fall of fluid.
const ICE: u8 = 64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FluidKind {
    Water,
    Lava,
}

/// A block's `FluidState`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fluid {
    pub kind: FluidKind,
    /// A source block (`Fluids.WATER`) rather than flowing (`Fluids.FLOWING_WATER`); the
    /// two are separate fluid types.
    pub source: bool,
    pub falling: bool,
    /// 1 to 8; 8 is full.
    pub amount: u8,
}

impl Fluid {
    /// Decodes `physics.json`'s `fluids` entry, as `DumpPhysics` packs it.
    pub(crate) fn unpack(bits: u8) -> Option<Self> {
        let kind = match bits & 3 {
            1 => FluidKind::Water,
            2 => FluidKind::Lava,
            _ => return None,
        };
        Some(Self {
            kind,
            source: bits & 4 != 0,
            falling: bits & 8 != 0,
            amount: bits >> 4,
        })
    }

    /// `FlowingFluid.getOwnHeight`.
    pub fn own_height(self) -> f32 {
        self.amount as f32 / 9.0
    }
}

impl BlockPhysics {
    /// The block state's fluid: water and lava blocks, waterlogged blocks, and the blocks
    /// that are always under water, such as kelp and seagrass.
    pub fn fluid(&self, state: u32) -> Option<Fluid> {
        Fluid::unpack(self.fluids.get(state as usize).copied().unwrap_or(0))
    }

    fn sturdy(&self, state: u32) -> u8 {
        self.sturdy.get(state as usize).copied().unwrap_or(0)
    }
}

/// The fluid at a block.
pub fn fluid_at(world: &impl World, blocks: &BlockPhysics, [x, y, z]: [i32; 3]) -> Option<Fluid> {
    blocks.fluid(world.state(x, y, z))
}

/// `FlowingFluid.getHeight`: a fluid with the same fluid above is a full block.
pub fn height(world: &impl World, blocks: &BlockPhysics, fluid: Fluid, [x, y, z]: [i32; 3]) -> f32 {
    let above = fluid_at(world, blocks, [x, y + 1, z]);
    if above.is_some_and(|a| a.kind == fluid.kind) {
        1.0
    } else {
        fluid.own_height()
    }
}

/// `FluidState.getHeightForCamera`: a source under a block with a sturdy bottom fills it.
fn height_for_camera(
    world: &impl World,
    blocks: &BlockPhysics,
    fluid: Fluid,
    [x, y, z]: [i32; 3],
) -> f32 {
    if fluid.source && blocks.sturdy(world.state(x, y + 1, z)) & 1 << DOWN != 0 {
        return 1.0;
    }
    height(world, blocks, fluid, [x, y, z])
}

/// `Vec3.normalize`: too short a vector becomes zero.
fn normalize(v: DVec3) -> DVec3 {
    let length = v.length();
    if length < 1.0e-5 {
        DVec3::ZERO
    } else {
        v / length
    }
}

/// `FlowingFluid.getFlow`: which way the fluid at a block pushes, from the heights around
/// it, straight down the sides of a fall.
pub fn flow(world: &impl World, blocks: &BlockPhysics, fluid: Fluid, [x, y, z]: [i32; 3]) -> DVec3 {
    // `affectsFlow`: empty or the same fluid.
    let affects = |f: Option<Fluid>| f.is_none_or(|f| f.kind == fluid.kind);
    let (mut flow_x, mut flow_z) = (0.0f64, 0.0f64);
    for ([dx, dz], _) in HORIZONTAL {
        let (nx, nz) = (x + dx, z + dz);
        let neighbour = fluid_at(world, blocks, [nx, y, nz]);
        if !affects(neighbour) {
            continue;
        }
        let mut neighbour_height = neighbour.map_or(0.0, Fluid::own_height);
        let mut distance = 0.0f32;
        if neighbour_height == 0.0 {
            if !blocks.blocks_fluid_flow[world.state(nx, y, nz) as usize] {
                let below = fluid_at(world, blocks, [nx, y - 1, nz]);
                if affects(below) {
                    neighbour_height = below.map_or(0.0, Fluid::own_height);
                    if neighbour_height > 0.0 {
                        distance = fluid.own_height() - (neighbour_height - 0.8888889);
                    }
                }
            }
        } else if neighbour_height > 0.0 {
            distance = fluid.own_height() - neighbour_height;
        }
        if distance != 0.0 {
            flow_x += (dx as f32 * distance) as f64;
            flow_z += (dz as f32 * distance) as f64;
        }
    }
    let mut flow = DVec3::new(flow_x, 0.0, flow_z);
    if fluid.falling {
        // `isSolidFace` for the blocks beside it and above those.
        let solid = |[x, y, z]: [i32; 3], face: u8| {
            if fluid_at(world, blocks, [x, y, z]).is_some_and(|f| f.kind == fluid.kind) {
                return false;
            }
            let sturdy = blocks.sturdy(world.state(x, y, z));
            sturdy & ICE == 0 && sturdy & 1 << face != 0
        };
        if HORIZONTAL.iter().any(|&([dx, dz], face)| {
            solid([x + dx, y, z + dz], face) || solid([x + dx, y + 1, z + dz], face)
        }) {
            flow = normalize(flow) + DVec3::new(0.0, -6.0, 0.0);
        }
    }
    normalize(flow)
}

/// The fluid the camera at `eye` is in, which fogs the view (`Camera.getFluidInCamera`):
/// water below its surface, or lava at or below it. Vanilla looks for lava at the corners
/// of the near plane too, 0.05 in front of the eye; this looks at the eye alone.
pub fn camera_fluid(world: &impl World, blocks: &BlockPhysics, eye: DVec3) -> Option<FluidKind> {
    let pos = eye.floor().as_ivec3().to_array();
    let fluid = fluid_at(world, blocks, pos)?;
    let top = pos[1] as f64 + height_for_camera(world, blocks, fluid, pos) as f64;
    let inside = match fluid.kind {
        FluidKind::Water => eye.y < top,
        FluidKind::Lava => eye.y <= top,
    };
    inside.then_some(fluid.kind)
}

/// Whether any block the box reaches into holds a fluid (`CollisionGetter.containsAnyLiquid`).
pub fn contains_any_liquid(world: &impl World, blocks: &BlockPhysics, b: &Aabb) -> bool {
    let (x0, y0, z0) = (
        b.min.x.floor() as i32,
        b.min.y.floor() as i32,
        b.min.z.floor() as i32,
    );
    let (x1, y1, z1) = (
        b.max.x.ceil() as i32,
        b.max.y.ceil() as i32,
        b.max.z.ceil() as i32,
    );
    (x0..x1)
        .any(|x| (y0..y1).any(|y| (z0..z1).any(|z| fluid_at(world, blocks, [x, y, z]).is_some())))
}

/// What an entity's box touched this tick: `EntityFluidInteraction.update`'s trackers and
/// current accumulators, for water and lava.
#[derive(Clone, Copy, Default, Debug)]
pub struct Touching {
    /// How far the fluid reaches above the bottom of the box (`getFluidHeight`).
    pub water_height: f64,
    pub lava_height: f64,
    /// Whether the eye is inside water (`isEyeInFluid`).
    pub eye_in_water: bool,
    pub water_current: Current,
    pub lava_current: Current,
}

/// `EntityFluidInteraction.CurrentAccumulator`.
#[derive(Clone, Copy, Default, Debug)]
pub struct Current {
    height: f64,
    sum: DVec3,
    count: u32,
}

impl Current {
    /// `CurrentAccumulator.applyTo` for a player: the average flow, scaled, and at least a
    /// nudge when standing still.
    pub fn push(&self, velocity: DVec3, scale: f64) -> DVec3 {
        if self.count == 0 || self.sum.length_squared() < 1.0e-5f32 as f64 {
            return DVec3::ZERO;
        }
        let mut impulse = self.sum * (1.0 / self.count as f64) * scale;
        if velocity.x.abs() < 0.003
            && velocity.z.abs() < 0.003
            && impulse.length() < 0.0045000000000000005
        {
            impulse = normalize(impulse) * 0.0045000000000000005;
        }
        impulse
    }
}

/// `EntityFluidInteraction.update` for a box with its bottom at `bottom` (the undeflated
/// box's `minY`) and its eye at `eye`, over the block column `eye_block`. `box_` is
/// `getFluidInteractionBox`, the bounding box deflated by 0.001.
pub fn touching(
    world: &impl World,
    blocks: &BlockPhysics,
    box_: &Aabb,
    bottom: f64,
    eye: DVec3,
    with_current: bool,
) -> Touching {
    let mut t = Touching::default();
    let (x0, y0, z0) = (
        box_.min.x.floor() as i32,
        box_.min.y.floor() as i32,
        box_.min.z.floor() as i32,
    );
    let (x1, y1, z1) = (
        box_.max.x.ceil() as i32 - 1,
        box_.max.y.ceil() as i32 - 1,
        box_.max.z.ceil() as i32 - 1,
    );
    // `hasFluidAndLoaded`: every chunk a block around the box has arrived.
    for cx in (x0 - 1).div_euclid(16)..=(x1 + 1).div_euclid(16) {
        for cz in (z0 - 1).div_euclid(16)..=(z1 + 1).div_euclid(16) {
            if !world.loaded(cx * 16, cz * 16) {
                return t;
            }
        }
    }
    let (eye_x, eye_z) = (eye.x.floor() as i32, eye.z.floor() as i32);
    for x in x0..=x1 {
        for y in y0..=y1 {
            for z in z0..=z1 {
                let Some(fluid) = fluid_at(world, blocks, [x, y, z]) else {
                    continue;
                };
                let fluid_bottom = y as f64;
                let fluid_top = fluid_bottom + height(world, blocks, fluid, [x, y, z]) as f64;
                if fluid_top < box_.min.y {
                    continue;
                }
                let water = fluid.kind == FluidKind::Water;
                if water
                    && x == eye_x
                    && z == eye_z
                    && eye.y >= fluid_bottom
                    && eye.y
                        <= fluid_bottom + height_for_camera(world, blocks, fluid, [x, y, z]) as f64
                {
                    t.eye_in_water = true;
                }
                // Source and flowing water are separate fluid types with their own trackers,
                // but `getFluidHeight` and the shared current take the highest of them.
                let (height, current) = if water {
                    (&mut t.water_height, &mut t.water_current)
                } else {
                    (&mut t.lava_height, &mut t.lava_current)
                };
                *height = height.max(fluid_top - bottom);
                if with_current {
                    let mut flow = flow(world, blocks, fluid, [x, y, z]);
                    current.height = current.height.max(*height);
                    if current.height < 0.4 {
                        flow *= current.height;
                    }
                    current.sum += flow;
                    current.count += 1;
                }
            }
        }
    }
    t
}
