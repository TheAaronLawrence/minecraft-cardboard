//! Collision of the player's box with block shapes: `Entity.collide`, `collideWithShapes`
//! and `Shapes.collide`, with vanilla's 1e-7 tolerance and its step-up search.

use crate::{BlockPhysics, World};
use glam::DVec3;

const EPS: f64 = 1.0e-7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    pub fn new(min: DVec3, max: DVec3) -> Self {
        Self { min, max }
    }

    /// A box `width` wide and `height` tall standing on `feet`.
    pub fn at_feet(feet: DVec3, width: f64, height: f64) -> Self {
        let h = width / 2.0;
        Self::new(
            feet - DVec3::new(h, 0.0, h),
            feet + DVec3::new(h, height, h),
        )
    }

    pub fn offset(&self, by: DVec3) -> Self {
        Self::new(self.min + by, self.max + by)
    }

    pub fn deflate(&self, by: f64) -> Self {
        Self::new(self.min + DVec3::splat(by), self.max - DVec3::splat(by))
    }

    /// Grown to cover moving by `by`.
    pub fn expand_towards(&self, by: DVec3) -> Self {
        Self::new(
            self.min + by.min(DVec3::ZERO),
            self.max + by.max(DVec3::ZERO),
        )
    }

    /// `AABB.contains`.
    pub fn contains(&self, p: DVec3) -> bool {
        (0..3).all(|i| self.min[i] <= p[i] && p[i] < self.max[i])
    }

    /// `AABB.clip`'s search: how far along the ray from `from` to `to` (0 to 1) it first
    /// crosses into the box, and the face it crosses, as `Direction.get3DDataValue`.
    pub fn clip(&self, from: DVec3, to: DVec3) -> Option<(f64, u8)> {
        let ray = to - from;
        let mut nearest = 1.0;
        let mut face = None;
        // Per axis: the face the ray enters through, and the two other axes.
        for (axis, a, c, faces) in [(0, 1, 2, [4, 5]), (1, 0, 2, [0, 1]), (2, 0, 1, [2, 3])] {
            let (plane, hit_face) = if ray[axis] > EPS {
                (self.min[axis], faces[0])
            } else if ray[axis] < -EPS {
                (self.max[axis], faces[1])
            } else {
                continue;
            };
            let t = (plane - from[axis]) / ray[axis];
            let (pa, pc) = (from[a] + t * ray[a], from[c] + t * ray[c]);
            if 0.0 < t
                && t < nearest
                && self.min[a] - EPS < pa
                && pa < self.max[a] + EPS
                && self.min[c] - EPS < pc
                && pc < self.max[c] + EPS
            {
                nearest = t;
                face = Some(hit_face);
            }
        }
        face.map(|face| (nearest, face))
    }

    fn intersects(&self, o: &Aabb) -> bool {
        (0..3).all(|i| self.min[i] < o.max[i] && self.max[i] > o.min[i])
    }
}

pub struct Collider<'a, W: ?Sized> {
    pub world: &'a W,
    pub blocks: &'a BlockPhysics,
}

impl<W: World + ?Sized> Collider<'_, W> {
    /// The block boxes overlapping `area`. Columns whose chunk hasn't arrived have none.
    pub fn boxes(&self, area: &Aabb) -> Vec<Aabb> {
        let lo = (area.min - DVec3::splat(EPS)).floor().as_ivec3() - 1;
        let hi = (area.max + DVec3::splat(EPS)).floor().as_ivec3() + 1;
        let mut out = Vec::new();
        for x in lo.x..=hi.x {
            for z in lo.z..=hi.z {
                if !self.world.loaded(x, z) {
                    continue;
                }
                for y in lo.y..=hi.y {
                    let origin = DVec3::new(x as f64, y as f64, z as f64);
                    for b in self.blocks.shape(self.world.state(x, y, z)) {
                        let b = b.offset(origin);
                        if b.intersects(area) {
                            out.push(b);
                        }
                    }
                }
            }
        }
        out
    }

    /// `CollisionGetter.findSupportingBlock`: of the blocks whose collision overlaps `area`,
    /// the one whose centre is nearest `feet`; on a tie, the greater by `Vec3i.compareTo`
    /// (y, then z, then x).
    pub fn supporting_block(&self, area: &Aabb, feet: DVec3) -> Option<[i32; 3]> {
        let lo = (area.min - DVec3::splat(EPS)).floor().as_ivec3() - 1;
        let hi = (area.max + DVec3::splat(EPS)).floor().as_ivec3() + 1;
        let mut best: Option<([i32; 3], f64)> = None;
        for x in lo.x..=hi.x {
            for z in lo.z..=hi.z {
                if !self.world.loaded(x, z) {
                    continue;
                }
                for y in lo.y..=hi.y {
                    let origin = DVec3::new(x as f64, y as f64, z as f64);
                    let shape = self.blocks.shape(self.world.state(x, y, z));
                    if !shape.iter().any(|b| b.offset(origin).intersects(area)) {
                        continue;
                    }
                    let d = (origin + 0.5).distance_squared(feet);
                    let key = |[x, y, z]: [i32; 3]| (y, z, x);
                    let better =
                        best.is_none_or(|(p, bd)| d < bd || d == bd && key(p) < key([x, y, z]));
                    if better {
                        best = Some(([x, y, z], d));
                    }
                }
            }
        }
        best.map(|(p, _)| p)
    }

    pub fn no_collision(&self, area: &Aabb) -> bool {
        self.boxes(area).is_empty()
    }

    /// How far a box can move of `motion`, stepping up to `step` blocks onto what it walks
    /// into when on the ground (`Entity.collide`).
    pub fn collide(&self, b: &Aabb, motion: DVec3, step: f32, on_ground: bool) -> DVec3 {
        let boxes = self.boxes(&b.expand_towards(motion));
        let moved = if motion == DVec3::ZERO {
            motion
        } else {
            collide_with_shapes(motion, b, &boxes)
        };
        let (hit_x, hit_y, hit_z) = (
            motion.x != moved.x,
            motion.y != moved.y,
            motion.z != moved.z,
        );
        let landing = hit_y && motion.y < 0.0;
        if step > 0.0 && (landing || on_ground) && (hit_x || hit_z) {
            let base = if landing {
                b.offset(DVec3::new(0.0, moved.y, 0.0))
            } else {
                *b
            };
            let mut area = base.expand_towards(DVec3::new(motion.x, step as f64, motion.z));
            if !landing {
                area = area.expand_towards(DVec3::new(0.0, -1.0e-5, 0.0));
            }
            let boxes = self.boxes(&area);
            // `collectCandidateStepUpHeights`: the tops and bottoms of what's in the way.
            let skip = moved.y as f32;
            let mut heights: Vec<f32> = boxes
                .iter()
                .flat_map(|c| [c.min.y, c.max.y])
                .map(|y| (y - base.min.y) as f32)
                .filter(|&h| h >= 0.0 && h != skip && h <= step)
                .collect();
            heights.sort_by(f32::total_cmp);
            heights.dedup();
            let horizontal = |v: DVec3| v.x * v.x + v.z * v.z;
            for h in heights {
                let stepped =
                    collide_with_shapes(DVec3::new(motion.x, h as f64, motion.z), &base, &boxes);
                if horizontal(stepped) > horizontal(moved) {
                    return stepped - DVec3::new(0.0, b.min.y - base.min.y, 0.0);
                }
            }
        }
        moved
    }
}

/// Moves along y, then the larger horizontal axis, then the other, clipping each against
/// the boxes (`Direction.axisStepOrder`).
fn collide_with_shapes(motion: DVec3, b: &Aabb, boxes: &[Aabb]) -> DVec3 {
    if boxes.is_empty() {
        return motion;
    }
    let order = if motion.x.abs() < motion.z.abs() {
        [1, 2, 0]
    } else {
        [1, 0, 2]
    };
    let (mut moved, mut b) = (DVec3::ZERO, *b);
    for axis in order {
        if motion[axis] == 0.0 {
            continue;
        }
        let d = clip(boxes, &b, axis, motion[axis]);
        if d != 0.0 {
            moved[axis] = d;
            let mut by = DVec3::ZERO;
            by[axis] = d;
            b = b.offset(by);
        }
    }
    moved
}

/// `Shapes.collide` on one axis: the farthest the box moves before touching a box it
/// overlaps on the other two axes.
fn clip(boxes: &[Aabb], mover: &Aabb, axis: usize, mut d: f64) -> f64 {
    if d.abs() < EPS {
        return 0.0;
    }
    let others = [(axis + 1) % 3, (axis + 2) % 3];
    for c in boxes {
        if !others
            .iter()
            .all(|&a| c.max[a] > mover.min[a] + EPS && c.min[a] < mover.max[a] - EPS)
        {
            continue;
        }
        if d > 0.0 && c.min[axis] >= mover.max[axis] - EPS {
            d = d.min(c.min[axis] - mover.max[axis]);
        } else if d < 0.0 && c.max[axis] <= mover.min[axis] + EPS {
            d = d.max(c.max[axis] - mover.min[axis]);
        }
    }
    if d.abs() < EPS { 0.0 } else { d }
}
