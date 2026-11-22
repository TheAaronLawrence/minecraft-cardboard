//! Which chunk sections to draw: the faces of each section that open onto each other
//! (`VisGraph`), the sections the camera can see through those openings
//! (`SectionOcclusionGraph`), and the ones inside the view (`Frustum`).

use crate::bake::Dir;
use glam::{IVec3, Mat4, Vec3, Vec4, Vec4Swizzles};
use std::collections::{HashMap, HashSet, VecDeque};

/// `VisibilitySet`: for each pair of a section's faces, whether something can see from one
/// through the section and out of the other. Bit `a + b * 6` for `Dir` ordinals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility(pub u64);

impl Visibility {
    /// Every face sees every other, as through an empty section.
    pub const ALL: Visibility = Visibility((1 << 36) - 1);

    /// `visibilityBetween`.
    pub fn between(self, a: Dir, b: Dir) -> bool {
        self.0 & 1 << (a as usize + b as usize * 6) != 0
    }

    /// `VisibilitySet.add`: each of these faces sees all the others.
    fn add(&mut self, faces: u8) {
        for a in 0..6 {
            for b in 0..6 {
                if faces & 1 << a != 0 && faces & 1 << b != 0 {
                    self.0 |= 1 << (a + b * 6);
                }
            }
        }
    }
}

/// `VisGraph`: a section's blocks that light can't pass (`isSolidRender`), by index
/// `x | z << 4 | y << 8`, resolved to the faces that open onto each other.
pub struct VisGraph {
    opaque: [u64; 64],
    count: u32,
}

impl Default for VisGraph {
    fn default() -> Self {
        Self {
            opaque: [0; 64],
            count: 0,
        }
    }
}

impl VisGraph {
    pub fn set_opaque(&mut self, x: usize, y: usize, z: usize) {
        let i = x | z << 4 | y << 8;
        if self.opaque[i / 64] & 1 << (i % 64) == 0 {
            self.opaque[i / 64] |= 1 << (i % 64);
            self.count += 1;
        }
    }

    /// `VisGraph.resolve`: flood the open blocks from every open one on the section's
    /// edge, and each flood's faces see each other.
    pub fn resolve(mut self) -> Visibility {
        if self.count < 256 {
            return Visibility::ALL;
        }
        let mut visibility = Visibility(0);
        if self.count == 4096 {
            return visibility;
        }
        let edge = |c: usize| c == 0 || c == 15;
        for x in 0..16 {
            for y in 0..16 {
                for z in 0..16 {
                    if edge(x) || edge(y) || edge(z) {
                        let i = x | z << 4 | y << 8;
                        if !self.get(i) {
                            let faces = self.flood_fill(i);
                            visibility.add(faces);
                        }
                    }
                }
            }
        }
        visibility
    }

    fn get(&self, i: usize) -> bool {
        self.opaque[i / 64] & 1 << (i % 64) != 0
    }

    /// `floodFill`: the faces the open blocks joined to `start` touch, as `Dir` bits.
    fn flood_fill(&mut self, start: usize) -> u8 {
        let mut faces = 0u8;
        let mut queue = VecDeque::from([start]);
        self.opaque[start / 64] |= 1 << (start % 64);
        while let Some(i) = queue.pop_front() {
            let (x, z, y) = (i & 15, i >> 4 & 15, i >> 8 & 15);
            for (c, low, high) in [(x, Dir::West, Dir::East), (y, Dir::Down, Dir::Up)] {
                faces |= match c {
                    0 => 1 << low as u8,
                    15 => 1 << high as u8,
                    _ => 0,
                };
            }
            faces |= match z {
                0 => 1 << Dir::North as u8,
                15 => 1 << Dir::South as u8,
                _ => 0,
            };
            let neighbours = [
                (y > 0).then(|| i - 256),
                (y < 15).then(|| i + 256),
                (z > 0).then(|| i - 16),
                (z < 15).then(|| i + 16),
                (x > 0).then(|| i - 1),
                (x < 15).then(|| i + 1),
            ];
            for n in neighbours.into_iter().flatten() {
                if !self.get(n) {
                    self.opaque[n / 64] |= 1 << (n % 64);
                    queue.push_back(n);
                }
            }
        }
        faces
    }
}

/// `Frustum` as the occlusion graph uses it: the view's side and near planes, around a
/// camera moved back until the 8-block cube it's in is wholly inside
/// (`offsetToFullyIncludeCameraCube(8)`), so sections right around the eye aren't cut.
pub struct Frustum {
    /// Planes relative to `cam`: a point `p` is on the inside when `n · (p - cam) + d >= 0`.
    planes: [Vec4; 5],
    cam: Vec3,
}

impl Frustum {
    /// `rotation_proj` is the projection times the view's rotation, without its
    /// translation; `forward` is where the camera looks.
    pub fn new(rotation_proj: Mat4, eye: Vec3, forward: Vec3) -> Self {
        let [r0, r1, r2, r3] = [0, 1, 2, 3].map(|i| rotation_proj.row(i));
        // wgpu's clip space keeps 0 <= z <= w; vanilla's culling ignores its far plane,
        // which is beyond the render distance.
        let planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2];
        let mut frustum = Self { planes, cam: eye };
        let (low, high) = ((eye / 8.0).floor() * 8.0, (eye / 8.0).ceil() * 8.0);
        // Each step moves the camera back 4 blocks; 64 is far more than any view needs.
        for _ in 0..64 {
            if frustum.contains(low - frustum.cam, high - frustum.cam) {
                break;
            }
            frustum.cam -= forward * 4.0;
        }
        frustum
    }

    /// The box lies wholly inside every plane.
    fn contains(&self, min: Vec3, max: Vec3) -> bool {
        self.planes.iter().all(|p| {
            let n = p.xyz();
            n.dot(Vec3::select(n.cmpge(Vec3::ZERO), min, max)) + p.w >= 0.0
        })
    }

    /// `isVisible`: some of the box may be inside.
    pub fn intersects(&self, min: Vec3, max: Vec3) -> bool {
        let (min, max) = (min - self.cam, max - self.cam);
        self.planes.iter().all(|p| {
            let n = p.xyz();
            n.dot(Vec3::select(n.cmpge(Vec3::ZERO), max, min)) + p.w >= 0.0
        })
    }
}

/// `SectionOcclusionGraph.runUpdates` with smart culling: the sections reachable from the
/// camera's section by stepping only away from the camera, and only between faces that
/// see each other. `visibility` gives each section's, or none where nothing is loaded.
/// `ys` is the lowest and highest section y; `distance` is the render distance in chunks,
/// which also limits how far above or below the camera sections draw (`getRelativeFrom`).
/// Returns none when the camera's column isn't loaded, where vanilla would draw nothing
/// until it is.
pub fn visible_sections(
    eye: Vec3,
    ys: (i32, i32),
    distance: i32,
    smart: bool,
    visibility: &impl Fn(IVec3) -> Option<Visibility>,
) -> Option<HashSet<IVec3>> {
    #[derive(Clone, Copy)]
    struct Node {
        sources: u8,
        directions: u8,
    }
    let section_of = |p: Vec3| (p / 16.0).floor().as_ivec3();
    let camera = section_of(eye);
    let in_world = |s: IVec3| (ys.0..=ys.1).contains(&s.y);
    let loaded = |s: IVec3| in_world(s) && visibility(s).is_some();
    let in_range = |s: IVec3| (s.y - camera.y).abs() <= distance;
    let mut nodes: HashMap<IVec3, Node> = HashMap::new();
    let mut queue = VecDeque::new();
    // `initializeQueueForFullUpdate`: the camera's section, or from above or below the
    // world, every section in its top or bottom layer, nearest first.
    if in_world(camera) {
        visibility(camera)?;
        nodes.insert(
            camera,
            Node {
                sources: 0,
                directions: 0,
            },
        );
        queue.push_back(camera);
    } else {
        let below = camera.y < ys.0;
        let y = if below { ys.0 } else { ys.1 };
        let source = if below { Dir::Up } else { Dir::Down };
        visibility(IVec3::new(camera.x, y, camera.z))?;
        let mut starts = Vec::new();
        for dx in -distance..=distance {
            for dz in -distance..=distance {
                let s = IVec3::new(camera.x + dx, y, camera.z + dz);
                if !loaded(s) {
                    continue;
                }
                let mut directions = 1 << source as u8;
                directions |= match dx.signum() {
                    1 => 1 << Dir::East as u8,
                    -1 => 1 << Dir::West as u8,
                    _ => 0,
                };
                directions |= match dz.signum() {
                    1 => 1 << Dir::South as u8,
                    -1 => 1 << Dir::North as u8,
                    _ => 0,
                };
                starts.push((s, directions));
            }
        }
        let block = eye.floor();
        let distance = |s: IVec3| (s.as_vec3() * 16.0 + 8.0 - block).length_squared();
        starts.sort_by(|a, b| distance(a.0).total_cmp(&distance(b.0)));
        for (s, directions) in starts {
            nodes.insert(
                s,
                Node {
                    sources: 1 << source as u8,
                    directions,
                },
            );
            queue.push_back(s);
        }
    }
    let camera_center = camera.as_vec3() * 16.0 + 8.0;
    // `ViewArea.minY` and `maxY`: the bottom and top blocks.
    let (min_y, max_y) = ((ys.0 * 16) as f32, (ys.1 * 16 + 15) as f32);
    while let Some(at) = queue.pop_front() {
        let node = nodes[&at];
        let seen = visibility(at).expect("only loaded sections are queued");
        // `MINIMUM_ADVANCED_CULLING_SECTION_DISTANCE`: past 60 blocks, also check the
        // way back to the camera.
        let distant = (at - camera).abs().max_element() > 3;
        for direction in Dir::ALL {
            let next = at + IVec3::from(direction.offset());
            if !loaded(next) || !in_range(next) {
                continue;
            }
            if smart && node.directions & 1 << direction.opposite() as u8 != 0 {
                continue;
            }
            if smart
                && node.sources != 0
                && !Dir::ALL.iter().any(|&source| {
                    node.sources & 1 << source as u8 != 0
                        && seen.between(source.opposite(), direction)
                })
            {
                continue;
            }
            if smart && distant {
                // From the section's corner nearest the camera, step toward the camera a
                // section's diagonal at a time; every section on the way must be reached.
                let origin = at.as_vec3() * 16.0;
                let axis = IVec3::from(direction.offset()).abs().as_vec3();
                let toward = |c: f32, o: f32, along: bool| if along { c > o } else { c < o };
                let corner = Vec3::new(
                    toward(camera_center.x, origin.x, axis.x != 0.0) as u8 as f32,
                    toward(camera_center.y, origin.y, axis.y != 0.0) as u8 as f32,
                    toward(camera_center.z, origin.z, axis.z != 0.0) as u8 as f32,
                );
                let mut check = origin + corner * 16.0;
                let step = (eye - check).normalize() * 28.0;
                let mut seen_from_camera = true;
                while check.distance_squared(eye) > 3600.0 {
                    check += step;
                    if check.y > max_y || check.y < min_y {
                        break;
                    }
                    let s = section_of(check);
                    if !loaded(s) || !nodes.contains_key(&s) {
                        seen_from_camera = false;
                        break;
                    }
                }
                if !seen_from_camera {
                    continue;
                }
            }
            match nodes.get_mut(&next) {
                Some(existing) => existing.sources |= 1 << direction as u8,
                None => {
                    nodes.insert(
                        next,
                        Node {
                            sources: 1 << direction as u8,
                            directions: node.directions | 1 << direction as u8,
                        },
                    );
                    queue.push_back(next);
                }
            }
        }
    }
    Some(nodes.into_keys().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wall_across_a_section_separates_its_two_sides() {
        // Fill the plane x = 7: west can't see east, but north still sees south and
        // west sees up, through the open halves.
        let mut graph = VisGraph::default();
        for y in 0..16 {
            for z in 0..16 {
                graph.set_opaque(7, y, z);
            }
        }
        let v = graph.resolve();
        assert!(!v.between(Dir::West, Dir::East));
        assert!(v.between(Dir::North, Dir::South));
        assert!(v.between(Dir::West, Dir::Up));
        assert!(v.between(Dir::East, Dir::Down));
        // Fewer than 256 solid blocks: everything sees everything, as in vanilla.
        let mut few = VisGraph::default();
        few.set_opaque(0, 0, 0);
        assert_eq!(few.resolve(), Visibility::ALL);
    }

    /// From above the world, every open section below is seen, down to the render
    /// distance, and none beyond it. Each distant section's way back to the camera ends at
    /// the world's top block, y 319, not inside the section above it.
    #[test]
    fn an_open_world_is_seen_from_above_within_the_render_distance() {
        let visibility = |s: IVec3| (s.x.abs() <= 2 && s.z.abs() <= 2).then_some(Visibility::ALL);
        let eye = Vec3::new(0.0, 400.0, 0.0);
        let seen = visible_sections(eye, (-4, 19), 16, true, &visibility).unwrap();
        for y in -4..=19 {
            let s = IVec3::new(0, y, 0);
            assert_eq!(seen.contains(&s), y >= 25 - 16, "section {s}");
        }
    }
}
