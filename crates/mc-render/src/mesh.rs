//! Builds vertex data for loaded chunks: culled faces, smooth lighting, ambient occlusion,
//! biome tints, and simple water and lava.

use crate::atlas::Layer;
use crate::bake::{
    self, BlockModels, Dir, JavaRandom, LAVA_FLOW_SPRITE, LAVA_SPRITE, Quad, WATER_FLOW_SPRITE,
    WATER_OVERLAY_SPRITE, WATER_SPRITE,
};
use crate::tint::{Biomes, Tint, block_tint};
use glam::Vec3;
use mc_registry::Registry;
use mc_world::{Chunk, World};
use std::collections::HashMap;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    /// Tint × directional shade × ambient occlusion, RGB, then alpha 255.
    pub color: [u8; 4],
    /// Sky and block light levels × 16 (0..240), an end portal's layer count, then flags:
    /// [`RED`] and [`UNLIT`].
    pub light: [u8; 4],
}

/// A vertex flag: an entity's red hurt flash.
pub const RED: u8 = 1;
/// A vertex flag: drawn at full brightness, whatever the light.
pub const UNLIT: u8 = 2;

/// Four vertices per quad, drawn as two triangles.
#[derive(Default)]
pub struct Mesh {
    pub opaque: Vec<Vertex>,
    pub cutout: Vec<Vertex>,
    pub translucent: Vec<Vertex>,
    /// End portal and gateway faces, for the portal shader; the third light byte holds
    /// the number of star layers.
    pub portal: Vec<Vertex>,
    /// Each 16-block section, bottom to top, for culling: its quads in each layer above,
    /// in order, and which of its faces see each other. Only `Mesher::mesh_chunk` fills it.
    pub sections: Vec<MeshSection>,
}

/// A section's part of a chunk's mesh.
#[derive(Debug, Clone)]
pub struct MeshSection {
    /// Section y: block y divided by 16.
    pub y: i32,
    /// Its quads in the opaque, cutout, translucent and portal layers: first and past last.
    pub quads: [[u32; 2]; 4],
    pub visibility: crate::cull::Visibility,
}

impl Mesh {
    pub fn quads(&self) -> usize {
        (self.opaque.len() + self.cutout.len() + self.translucent.len() + self.portal.len()) / 4
    }

    fn append(&mut self, other: Mesh) {
        self.opaque.extend(other.opaque);
        self.cutout.extend(other.cutout);
        self.translucent.extend(other.translucent);
        self.portal.extend(other.portal);
    }

    fn layer(&mut self, layer: Layer) -> &mut Vec<Vertex> {
        match layer {
            Layer::Opaque => &mut self.opaque,
            Layer::Cutout => &mut self.cutout,
            Layer::Translucent => &mut self.translucent,
            Layer::EndPortal(_) => &mut self.portal,
        }
    }

    /// Orders translucent quads far to near from `eye`, so blending looks right; within
    /// each section, as sections draw on their own.
    pub fn sort_translucent(&mut self, eye: Vec3) {
        let distance = |q: &[Vertex; 4]| {
            (q.iter().map(|v| Vec3::from(v.pos)).sum::<Vec3>() / 4.0).distance_squared(eye)
        };
        let quads = self.translucent.as_chunks_mut::<4>().0;
        let ranges: Vec<[u32; 2]> = if self.sections.is_empty() {
            vec![[0, quads.len() as u32]]
        } else {
            self.sections.iter().map(|s| s.quads[2]).collect()
        };
        for [start, end] in ranges {
            quads[start as usize..end as usize].sort_by(|a, b| distance(b).total_cmp(&distance(a)));
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Fluid {
    Water,
    Lava,
}

/// What the mesher needs to know about a state beyond its models.
#[derive(Clone, Copy)]
struct StateInfo {
    block: u32,
    tint: Option<Tint>,
    fluid: Option<(Fluid, u8)>,
    /// Plants vanilla nudges sideways by a per-position amount.
    offset: bool,
    air: bool,
    /// Leaves draw the faces between them: `LeavesBlock.skipRendering` only hides them when
    /// the Cutout Leaves option is off, and it is on by default.
    leaves: bool,
    /// A `HalfTransparentBlock`: glass, stained and tinted glass, ice, frosted and blue ice,
    /// slime, honey and copper grates (vanilla's classes in `Blocks`).
    half_transparent: bool,
    /// Its block entity's data can change how it looks (see `entity_model::uses_data`).
    data: bool,
}

fn state_infos(registry: &Registry) -> Vec<StateInfo> {
    let mut infos = Vec::with_capacity(registry.state_count() as usize);
    for block in registry.blocks() {
        let name = block.name.strip_prefix("minecraft:").unwrap_or(&block.name);
        for state in block.first_state..block.first_state + block.state_count() {
            let values = block.property_values(state);
            let value = |p: &str| values.iter().find(|(n, _)| *n == p).map(|(_, v)| *v);
            let level = value("level").and_then(|l| l.parse().ok()).unwrap_or(0);
            let fluid = match name {
                "water" => Some((Fluid::Water, level)),
                "lava" => Some((Fluid::Lava, level)),
                "kelp" | "kelp_plant" | "seagrass" | "tall_seagrass" | "bubble_column" => {
                    Some((Fluid::Water, 0))
                }
                _ if value("waterlogged") == Some("true") => Some((Fluid::Water, 0)),
                _ => None,
            };
            let offset = matches!(
                name,
                "short_grass"
                    | "tall_grass"
                    | "fern"
                    | "large_fern"
                    | "dandelion"
                    | "poppy"
                    | "blue_orchid"
                    | "allium"
                    | "azure_bluet"
                    | "red_tulip"
                    | "orange_tulip"
                    | "white_tulip"
                    | "pink_tulip"
                    | "oxeye_daisy"
                    | "cornflower"
                    | "lily_of_the_valley"
                    | "torchflower"
                    | "wither_rose"
                    | "bush"
            );
            infos.push(StateInfo {
                block: block.id,
                tint: block_tint(name, &values),
                fluid,
                offset,
                air: name.ends_with("air") && block.properties.is_empty(),
                leaves: name.ends_with("_leaves"),
                half_transparent: matches!(
                    name,
                    "glass"
                        | "tinted_glass"
                        | "ice"
                        | "frosted_ice"
                        | "blue_ice"
                        | "slime_block"
                        | "honey_block"
                ) || name.ends_with("_stained_glass")
                    || name.ends_with("copper_grate"),
                data: crate::entity_model::uses_data(name),
            });
        }
    }
    infos
}

/// The chunks around a point, loaded so the mesher can look across chunk edges.
pub struct WorldView {
    chunks: HashMap<(i32, i32), Chunk>,
    air: u32,
    /// Chests whose lids are open, which are meshed without them: open, or closing until
    /// this time, once the lid has shut and the game draws it no more.
    open_lids: HashMap<[i32; 3], Option<std::time::Instant>>,
}

impl WorldView {
    /// Loads chunks within `radius` chunks of the block at `center_x`, `center_z`.
    pub fn load(
        world: &mut World,
        registry: &Registry,
        center_x: i32,
        center_z: i32,
        radius: i32,
    ) -> anyhow::Result<Self> {
        let (ccx, ccz) = (center_x.div_euclid(16), center_z.div_euclid(16));
        let mut view = Self::empty(registry);
        for cx in ccx - radius..=ccx + radius {
            for cz in ccz - radius..=ccz + radius {
                if let Some(chunk) = world.chunk(registry, cx, cz)? {
                    view.insert(chunk);
                }
            }
        }
        Ok(view)
    }

    /// No chunks yet; add them with `insert` as they load.
    pub fn empty(registry: &Registry) -> Self {
        let air = registry.block("minecraft:air").expect("air").default_state;
        Self {
            chunks: HashMap::new(),
            air,
            open_lids: HashMap::new(),
        }
    }

    /// `ChestBlockEntity.triggerEvent(1, viewers)`: a chest's lid opens, or with no
    /// viewers left starts to close, and its chunk is meshed again without it until
    /// `shut` has passed. False when nothing changed.
    pub fn set_lid(&mut self, pos: [i32; 3], open: bool, shut: std::time::Instant) -> bool {
        let new = if open { None } else { Some(shut) };
        match self.open_lids.get(&pos) {
            None if !open => false,
            Some(old) if *old == new => false,
            _ => {
                self.open_lids.insert(pos, new);
                true
            }
        }
    }

    /// The chests whose lids have shut since: their chunks have them again.
    pub fn take_shut_lids(&mut self, now: std::time::Instant) -> Vec<[i32; 3]> {
        let shut: Vec<_> = self
            .open_lids
            .iter()
            .filter(|(_, t)| t.is_some_and(|t| t <= now))
            .map(|(p, _)| *p)
            .collect();
        for p in &shut {
            self.open_lids.remove(p);
        }
        shut
    }

    fn lid_open(&self, pos: [i32; 3]) -> bool {
        self.open_lids.contains_key(&pos)
    }

    pub fn insert(&mut self, chunk: Chunk) {
        self.chunks.insert((chunk.x, chunk.z), chunk);
    }

    pub fn remove(&mut self, cx: i32, cz: i32) {
        self.chunks.remove(&(cx, cz));
    }

    pub fn contains(&self, cx: i32, cz: i32) -> bool {
        self.chunks.contains_key(&(cx, cz))
    }

    /// Sets a block's state, dropping the block entity it had; false when its chunk or
    /// section isn't loaded.
    pub fn set_block(&mut self, [x, y, z]: [i32; 3], state: u32) -> bool {
        let Some(chunk) = self.chunks.get_mut(&(x.div_euclid(16), z.div_euclid(16))) else {
            return false;
        };
        chunk.block_entities.remove(&[x, y, z]);
        self.open_lids.remove(&[x, y, z]);
        let Some(section) = chunk.section_mut(y.div_euclid(16)) else {
            return false;
        };
        let local = [x, y, z].map(|c| c.rem_euclid(16) as usize);
        section.set_block(local[0], local[1], local[2], state);
        true
    }

    /// Replaces the sky or block light of a section of a loaded chunk.
    pub fn set_light(
        &mut self,
        (cx, cz): (i32, i32),
        section_y: i32,
        sky: bool,
        light: Box<[u8; 2048]>,
    ) {
        let Some(section) = self
            .chunks
            .get_mut(&(cx, cz))
            .and_then(|c| c.section_mut(section_y))
        else {
            return;
        };
        if sky {
            section.sky_light = Some(light);
        } else {
            section.block_light = Some(light);
        }
    }

    /// A loaded chunk, by chunk coordinates.
    pub fn column(&self, cx: i32, cz: i32) -> Option<&Chunk> {
        self.chunks.get(&(cx, cz))
    }

    pub fn column_mut(&mut self, cx: i32, cz: i32) -> Option<&mut Chunk> {
        self.chunks.get_mut(&(cx, cz))
    }

    /// The loaded chunks' positions, in chunk coordinates.
    pub fn chunk_positions(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.chunks.keys().copied()
    }

    /// The saved data of the block entity at a block, if it has one.
    pub fn block_entity(&self, x: i32, y: i32, z: i32) -> Option<&mc_nbt::Compound> {
        self.chunk(x, z)?.block_entities.get(&[x, y, z])
    }

    /// Every loaded block entity's saved data.
    pub fn block_entities(&self) -> impl Iterator<Item = &mc_nbt::Compound> + '_ {
        self.chunks.values().flat_map(|c| c.block_entities.values())
    }

    pub fn biome_names(&self) -> impl Iterator<Item = String> + '_ {
        self.chunks.values().flat_map(|c| {
            c.sections
                .iter()
                .flat_map(|s| s.biome_palette.iter().cloned())
        })
    }

    fn chunk(&self, x: i32, z: i32) -> Option<&Chunk> {
        self.chunks.get(&(x.div_euclid(16), z.div_euclid(16)))
    }

    pub fn state(&self, x: i32, y: i32, z: i32) -> u32 {
        self.chunk(x, z)
            .and_then(|c| c.block(x, y, z))
            .unwrap_or(self.air)
    }

    /// Sky and block light at a block. A section the save left without sky light takes it
    /// from the nearest section above that has some, as vanilla does; above them all it's 15.
    pub fn light(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        let Some(chunk) = self.chunk(x, z) else {
            return (15, 0);
        };
        let (lx, lz) = (x.rem_euclid(16) as usize, z.rem_euclid(16) as usize);
        let section_y = y.div_euclid(16);
        let block = chunk
            .section(section_y)
            .and_then(|s| s.block_light(lx, y.rem_euclid(16) as usize, lz))
            .unwrap_or(0);
        let sky = match chunk
            .section(section_y)
            .and_then(|s| s.sky_light(lx, y.rem_euclid(16) as usize, lz))
        {
            Some(sky) => sky,
            None => chunk
                .sections
                .iter()
                .filter(|s| s.y > section_y)
                .find_map(|s| s.sky_light(lx, 0, lz))
                .unwrap_or(15),
        };
        (sky, block)
    }

    fn biome(&self, x: i32, y: i32, z: i32) -> Option<&str> {
        let chunk = self.chunk(x, z)?;
        chunk.section(y.div_euclid(16))?.biome(
            x.rem_euclid(16) as usize,
            y.rem_euclid(16) as usize,
            z.rem_euclid(16) as usize,
        )
    }
}

pub struct Mesher<'a> {
    models: &'a BlockModels,
    biomes: &'a Biomes,
    registry: &'a Registry,
    infos: Vec<StateInfo>,
}

impl<'a> Mesher<'a> {
    pub fn new(models: &'a BlockModels, biomes: &'a Biomes, registry: &'a Registry) -> Self {
        Self {
            models,
            biomes,
            registry,
            infos: state_infos(registry),
        }
    }

    /// Meshes every loaded chunk whose four neighbours are loaded too, so no face along the
    /// edge of the loaded area is drawn against missing terrain.
    pub fn mesh_world(&self, view: &WorldView) -> Mesh {
        let mut mesh = Mesh::default();
        for &(cx, cz) in view.chunks.keys() {
            if let Some(chunk) = self.mesh_chunk(view, cx, cz) {
                mesh.append(chunk);
            }
        }
        mesh
    }

    /// One chunk's mesh, or `None` when it or one of its four neighbours isn't loaded.
    pub fn mesh_chunk(&self, view: &WorldView, cx: i32, cz: i32) -> Option<Mesh> {
        let chunk = view.chunks.get(&(cx, cz))?;
        if ![(1, 0), (-1, 0), (0, 1), (0, -1)]
            .iter()
            .all(|(dx, dz)| view.contains(cx + dx, cz + dz))
        {
            return None;
        }
        let mut mesh = Mesh::default();
        let lengths = |m: &Mesh| {
            [&m.opaque, &m.cutout, &m.translucent, &m.portal].map(|l| l.len() as u32 / 4)
        };
        for section in &chunk.sections {
            let start = lengths(&mesh);
            if matches!(section.blocks, mc_world::Blocks::Uniform(s) if self.infos[s as usize].air)
            {
                mesh.sections.push(MeshSection {
                    y: section.y,
                    quads: start.map(|q| [q, q]),
                    visibility: crate::cull::Visibility::ALL,
                });
                continue;
            }
            // `SectionCompiler.compile`: blocks that are `isSolidRender` close the section.
            let mut graph = crate::cull::VisGraph::default();
            for ly in 0..16 {
                for lz in 0..16 {
                    for lx in 0..16 {
                        let state = section.block(lx, ly, lz);
                        if self.infos[state as usize].air {
                            continue;
                        }
                        if self.solid_render(state) {
                            graph.set_opaque(lx, ly, lz);
                        }
                        let (x, y, z) = (
                            cx * 16 + lx as i32,
                            section.y * 16 + ly as i32,
                            cz * 16 + lz as i32,
                        );
                        self.mesh_block(view, &mut mesh, state, x, y, z);
                    }
                }
            }
            let end = lengths(&mesh);
            mesh.sections.push(MeshSection {
                y: section.y,
                quads: std::array::from_fn(|i| [start[i], end[i]]),
                visibility: graph.resolve(),
            });
        }
        Some(mesh)
    }

    /// `getFaceOcclusionShape(dir)`: which sixteenths of the state's `dir` face it fills.
    fn face(&self, state: u32, dir: Dir) -> [u64; 4] {
        self.models.states[state as usize]
            .faces
            .as_ref()
            .map_or([0; 4], |faces| faces[dir as usize])
    }

    /// The state's `dir` face is filled, as `Shapes.block()` fills it.
    fn full_face(&self, state: u32, dir: Dir) -> bool {
        self.face(state, dir) == [u64::MAX; 4]
    }

    /// `BlockState.isSolidRender`: its occlusion shape fills the whole block.
    fn solid_render(&self, state: u32) -> bool {
        Dir::ALL.iter().all(|&d| self.full_face(state, d))
    }

    /// `Block.shouldRenderFace`: whether `state`'s face toward `dir` shows past `neighbor`,
    /// which hides the part of it that `neighbor`'s facing occlusion shape covers. `skip`
    /// is `skipRendering`, as between two blocks of glass.
    fn should_render_face(&self, state: u32, neighbor: u32, dir: Dir, skip: bool) -> bool {
        let occluder = self.face(neighbor, dir.opposite());
        if occluder == [u64::MAX; 4] || skip {
            return false;
        }
        let shape = self.face(state, dir);
        occluder == [0; 4] || shape == [0; 4] || (0..4).any(|i| shape[i] & !occluder[i] != 0)
    }

    fn mesh_block(&self, view: &WorldView, mesh: &mut Mesh, state: u32, x: i32, y: i32, z: i32) {
        let info = self.infos[state as usize];
        let model = &self.models.states[state as usize];
        let neighbor = |d: Dir| {
            let [dx, dy, dz] = d.offset();
            view.state(x + dx, y + dy, z + dz)
        };
        let mut origin = Vec3::new(x as f32, y as f32, z as f32);
        if info.offset {
            // Block.getOffset: up to a quarter block either way, from the column's seed.
            let seed = bake::position_seed(x, 0, z);
            let nudge = |shift: i64| {
                ((((seed >> shift) & 15) as f32 / 15.0 - 0.5) * 0.5).clamp(-0.25, 0.25)
            };
            origin += Vec3::new(nudge(0), 0.0, nudge(8));
        }

        if let Some(quads) = self.block_entity_quads(view, state, x, y, z) {
            for (quad, color) in &quads {
                self.emit_quad(view, mesh, quad, origin, [x, y, z], *color, false);
            }
            if let Some((fluid, level)) = info.fluid {
                self.mesh_fluid(view, mesh, (fluid, level), [x, y, z]);
            }
            return;
        }

        let mut random = JavaRandom::new(bake::position_seed(x, y, z));
        let part_seed = if model.parts.len() > 1 {
            Some(random.next_long())
        } else {
            None
        };
        // An open chest's lid is drawn by the game, at its openness.
        let open;
        let parts = match &model.body {
            Some(body) if view.lid_open([x, y, z]) => {
                open = [vec![(1, body.clone())]];
                &open[..]
            }
            _ => &model.parts[..],
        };
        for part in parts {
            if let Some(seed) = part_seed {
                random = JavaRandom::new(seed);
            }
            let baked = bake::pick(part, &mut random);
            for quad in &baked.quads {
                if let Some(cull) = quad.cull {
                    let other = neighbor(cull);
                    // Glass and ice hide the faces between two blocks of the same kind.
                    let same_kind = self.infos[other as usize].block == info.block;
                    let skip =
                        same_kind && !info.leaves && quad.layer != Layer::Opaque && quad.on_face;
                    if !self.should_render_face(state, other, cull, skip) {
                        continue;
                    }
                }
                let tint = if quad.tint >= 0 {
                    info.tint.map(|t| self.blended_tint(view, t, x, y, z))
                } else {
                    None
                };
                self.emit_quad(
                    view,
                    mesh,
                    quad,
                    origin,
                    [x, y, z],
                    tint.unwrap_or([1.0; 3]),
                    baked.ambient_occlusion,
                );
            }
        }
        if let Some((fluid, level)) = info.fluid {
            self.mesh_fluid(view, mesh, (fluid, level), [x, y, z]);
        }
    }

    /// The block drawn from its block entity's data, when that data changes its look.
    fn block_entity_quads(
        &self,
        view: &WorldView,
        state: u32,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<Vec<(Quad, [f32; 3])>> {
        if !self.infos[state as usize].data {
            return None;
        }
        let data = view.block_entity(x, y, z)?;
        let block = self.registry.block_of_state(state)?;
        let values = block.property_values(state);
        crate::entity_model::with_data(&block.name, &values, data, &self.models.atlas)
    }

    /// Vanilla's default biome blend: the average over the 5×5 columns around the block.
    fn blended_tint(&self, view: &WorldView, tint: Tint, x: i32, y: i32, z: i32) -> [f32; 3] {
        if matches!(tint, Tint::Fixed(_)) {
            return self.biomes.color(tint, None, [x, z]);
        }
        let mut sum = [0.0; 3];
        for dx in -2..=2 {
            for dz in -2..=2 {
                let c = self
                    .biomes
                    .color(tint, view.biome(x + dx, y, z + dz), [x + dx, z + dz]);
                (0..3).for_each(|i| sum[i] += c[i]);
            }
        }
        sum.map(|s| s / 25.0)
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_quad(
        &self,
        view: &WorldView,
        mesh: &mut Mesh,
        quad: &Quad,
        origin: Vec3,
        block: [i32; 3],
        tint: [f32; 3],
        ao: bool,
    ) {
        let shade = quad.shade.unwrap_or(quad.dir).shade();
        let [dx, dy, dz] = quad.dir.offset();
        let [x, y, z] = block;
        // Light comes from the block the face looks into when it lies on the block's edge.
        let base = if quad.on_face {
            [x + dx, y + dy, z + dz]
        } else {
            block
        };
        let corners: [(f32, (f32, f32)); 4] = if ao {
            std::array::from_fn(|i| self.smooth_corner(view, quad.dir, base, quad.pos[i]))
        } else {
            let (sky, block_light) = view.light(base[0], base[1], base[2]);
            [(1.0, (sky as f32, block_light as f32)); 4]
        };
        let layers = match quad.layer {
            Layer::EndPortal(n) => n,
            _ => 0,
        };
        let out = mesh.layer(quad.layer);
        for (i, (brightness, (sky, block_light))) in corners.into_iter().enumerate() {
            let color =
                tint.map(|c| (c * shade * brightness * 255.0).round().clamp(0.0, 255.0) as u8);
            out.push(Vertex {
                pos: (origin + quad.pos[i]).into(),
                uv: quad.uv[i],
                color: [color[0], color[1], color[2], 255],
                light: [(sky * 16.0) as u8, (block_light * 16.0) as u8, layers, 0],
            });
        }
    }

    /// Vanilla's smooth lighting for one corner: the face's own light and that of the two
    /// edge neighbours and the diagonal one, averaged, with full blocks darkening the corner.
    fn smooth_corner(
        &self,
        view: &WorldView,
        dir: Dir,
        base: [i32; 3],
        corner: Vec3,
    ) -> (f32, (f32, f32)) {
        let axis = |d: Dir| -> usize {
            match d {
                Dir::Down | Dir::Up => 1,
                Dir::North | Dir::South => 2,
                Dir::West | Dir::East => 0,
            }
        };
        let normal = axis(dir);
        let (t1, t2) = match normal {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        let sign = |v: f32| if v < 0.5 { -1 } else { 1 };
        let corner = corner.to_array();
        let mut e1 = base;
        e1[t1] += sign(corner[t1]);
        let mut e2 = base;
        e2[t2] += sign(corner[t2]);
        let mut diagonal = e1;
        diagonal[t2] += sign(corner[t2]);

        let sample = |p: [i32; 3]| {
            let model = &self.models.states[view.state(p[0], p[1], p[2]) as usize];
            let shade = if model.shades { 0.2 } else { 1.0 };
            (shade, view.light(p[0], p[1], p[2]), model.blocks_light)
        };
        let center = sample(base);
        let s1 = sample(e1);
        let s2 = sample(e2);
        // With both edges blocked the diagonal can't be seen; vanilla uses the first edge.
        let d = if s1.2 && s2.2 { s1 } else { sample(diagonal) };
        let brightness = (center.0 + s1.0 + s2.0 + d.0) / 4.0;
        // Vanilla's blend treats an unlit (zero) neighbour as having the face's own light.
        let blend = |pick: fn(&(u8, u8)) -> u8| {
            let c = pick(&center.1) as f32;
            let v = |s: &(f32, (u8, u8), bool)| {
                if pick(&s.1) == 0 {
                    c
                } else {
                    pick(&s.1) as f32
                }
            };
            (c + v(&s1) + v(&s2) + v(&d)) / 4.0
        };
        (brightness, (blend(|l| l.0), blend(|l| l.1)))
    }

    /// Water and lava are drawn by code in vanilla (`LiquidBlockRenderer`), not by models.
    /// Each top corner sits at a weighted average of the fluid heights around it, the top
    /// uses the flowing texture turned to the direction the fluid flows, and faces are only
    /// drawn where they touch something other than the same fluid. Faces are drawn from
    /// both sides, so they show from inside the fluid, except water's against glass and
    /// leaves, which shows its overlay texture from outside only. Sides and the bottom sit
    /// 0.001 inside the block, off the faces of the blocks they touch.
    fn mesh_fluid(
        &self,
        view: &WorldView,
        mesh: &mut Mesh,
        (fluid, level): (Fluid, u8),
        [x, y, z]: [i32; 3],
    ) {
        let info = |x, y, z| self.infos[view.state(x, y, z) as usize];
        let same = |x, y, z| info(x, y, z).fluid.is_some_and(|f| f.0 == fluid);
        let own = |x, y, z| {
            info(x, y, z)
                .fluid
                .filter(|f| f.0 == fluid)
                .map(|f| own_height(f.1))
        };
        // Vanilla's `getHeight`: 1 under more of the same fluid, the fluid's own height,
        // 0 for open space, -1 for a solid block. (Vanilla's "solid" is a per-block flag;
        // here anything that isn't air, fluid or a plant counts.)
        let height = |x, y, z| -> f32 {
            if let Some(h) = own(x, y, z) {
                return if same(x, y + 1, z) { 1.0 } else { h };
            }
            let i = info(x, y, z);
            if i.air || i.offset { 0.0 } else { -1.0 }
        };
        let above = same(x, y + 1, z);
        let corner = |dx: i32, dz: i32| {
            if above {
                return 1.0;
            }
            average_height(
                height(x, y, z),
                height(x + dx, y, z),
                height(x, y, z + dz),
                || height(x + dx, y, z + dz),
            )
        };
        // Indexed [x][z], x and z each 0 (west, north) or 1 (east, south).
        let heights = [
            [corner(-1, -1), corner(-1, 1)],
            [corner(1, -1), corner(1, 1)],
        ];

        let (still, flowing, layer, tint) = match fluid {
            Fluid::Water => (
                WATER_SPRITE,
                WATER_FLOW_SPRITE,
                Layer::Translucent,
                self.blended_tint(view, Tint::Water, x, y, z),
            ),
            Fluid::Lava => (LAVA_SPRITE, LAVA_FLOW_SPRITE, Layer::Opaque, [1.0; 3]),
        };
        let (still, flowing, overlay) = (
            self.models.atlas.sprite(still).rect,
            self.models.atlas.sprite(flowing).rect,
            self.models.atlas.sprite(WATER_OVERLAY_SPRITE).rect,
        );
        let at = |rect: [f32; 4], u: f32, v: f32| {
            [
                rect[0] + (rect[2] - rect[0]) * u,
                rect[1] + (rect[3] - rect[1]) * v,
            ]
        };
        // Vanilla takes the brighter of each light at the fluid and the block above it.
        let light_at = |x, y, z, with_above: bool| {
            let (s, b) = view.light(x, y, z);
            let (sa, ba) = if with_above {
                view.light(x, y + 1, z)
            } else {
                (0, 0)
            };
            [s.max(sa) * 16, b.max(ba) * 16, 0, 0]
        };
        let origin = Vec3::new(x as f32, y as f32, z as f32);
        let out = mesh.layer(layer);
        let mut emit =
            |dir: Dir, pos: [Vec3; 4], uv: [[f32; 2]; 4], light: [u8; 4], both_sides: bool| {
                let shade = dir.shade();
                let color = tint.map(|c| (c * shade * 255.0).round().clamp(0.0, 255.0) as u8);
                let vertex = |i: usize| Vertex {
                    pos: (origin + pos[i]).into(),
                    uv: uv[i],
                    color: [color[0], color[1], color[2], 255],
                    light,
                };
                out.extend((0..4).map(vertex));
                if both_sides {
                    out.extend([0, 3, 2, 1].map(vertex));
                }
            };

        if !above {
            let mut pos = bake::corners(Dir::Up, Vec3::ZERO, Vec3::ONE);
            for p in &mut pos {
                p.y = heights[p.x as usize][p.z as usize];
            }
            let flow = self.flow(view, fluid, own_height(level), [x, y, z]);
            let uv = if flow == [0.0, 0.0] {
                [
                    at(still, 0.0, 0.0),
                    at(still, 0.0, 1.0),
                    at(still, 1.0, 1.0),
                    at(still, 1.0, 0.0),
                ]
            } else {
                // Half the flowing texture, turned to face downstream.
                let angle = flow[1].atan2(flow[0]) - std::f32::consts::FRAC_PI_2;
                let (s, c) = (angle.sin() * 0.25, angle.cos() * 0.25);
                [
                    at(flowing, 0.5 + (-c - s), 0.5 + (-c + s)),
                    at(flowing, 0.5 + (-c + s), 0.5 + (c + s)),
                    at(flowing, 0.5 + (c + s), 0.5 + (c - s)),
                    at(flowing, 0.5 + (c - s), 0.5 + (-c - s)),
                ]
            };
            // `FluidState.shouldRenderBackwardUpFace`: seen from below too, unless the fluid
            // or solid blocks fill all nine blocks over it.
            let backward = (-1..=1).any(|dx| {
                (-1..=1).any(|dz| {
                    let (bx, bz) = (x + dx, z + dz);
                    !same(bx, y + 1, bz) && !self.solid_render(view.state(bx, y + 1, bz))
                })
            });
            emit(Dir::Up, pos, uv, light_at(x, y, z, true), backward);
        }
        // `FluidRenderer.isFaceOccludedByNeighbor`, for a neighbour whose face is full.
        if !same(x, y - 1, z) && !self.full_face(view.state(x, y - 1, z), Dir::Up) {
            let pos =
                bake::corners(Dir::Down, Vec3::ZERO, Vec3::ONE).map(|p| Vec3::new(p.x, 0.001, p.z));
            let uv = [
                at(still, 0.0, 1.0),
                at(still, 0.0, 0.0),
                at(still, 1.0, 0.0),
                at(still, 1.0, 1.0),
            ];
            emit(Dir::Down, pos, uv, light_at(x, y - 1, z, false), false);
        }
        for dir in [Dir::North, Dir::South, Dir::West, Dir::East] {
            let [dx, _, dz] = dir.offset();
            if same(x + dx, y, z + dz)
                || self.full_face(view.state(x + dx, y, z + dz), dir.opposite())
            {
                continue;
            }
            // Top corners at the fluid's height there; the texture is the top-left quarter
            // of the flowing texture (or water's overlay), cut off where the fluid ends.
            let neighbour = info(x + dx, y, z + dz);
            let is_overlay =
                fluid == Fluid::Water && (neighbour.half_transparent || neighbour.leaves);
            let sprite = if is_overlay { overlay } else { flowing };
            let mut pos = bake::corners(dir, Vec3::ZERO, Vec3::ONE);
            let mut uv = [[0.0; 2]; 4];
            for (i, p) in pos.iter_mut().enumerate() {
                let v = if p.y > 0.5 {
                    p.y = heights[p.x as usize][p.z as usize];
                    (1.0 - p.y) * 0.5
                } else {
                    0.5
                };
                uv[i] = at(sprite, if i < 2 { 0.0 } else { 0.5 }, v);
                // The 0.001 inset, toward the block's middle.
                *p -= Vec3::new(dx as f32, 0.0, dz as f32) * 0.001;
            }
            emit(
                dir,
                pos,
                uv,
                light_at(x + dx, y, z + dz, false),
                !is_overlay,
            );
        }
    }

    /// `FlowingFluid.getFlow`: which way the fluid runs, toward lower neighbours and over
    /// edges. Zero for still fluid, which draws the still texture.
    fn flow(&self, view: &WorldView, fluid: Fluid, own: f32, [x, y, z]: [i32; 3]) -> [f32; 2] {
        let neighbour = |x, y, z| {
            let info = self.infos[view.state(x, y, z) as usize];
            match info.fluid {
                Some((f, level)) if f == fluid => Some(own_height(level)),
                Some(_) => None,
                // Fluid flows toward empty space as if it held fluid of height 0.
                None => Some(0.0),
            }
        };
        let mut flow = [0.0f32; 2];
        for dir in [Dir::North, Dir::South, Dir::West, Dir::East] {
            let [dx, _, dz] = dir.offset();
            let Some(h) = neighbour(x + dx, y, z + dz) else {
                continue;
            };
            let diff = if h == 0.0 {
                let open = self.infos[view.state(x + dx, y, z + dz) as usize].air;
                match neighbour(x + dx, y - 1, z + dz) {
                    Some(below) if open && below > 0.0 => own - (below - 8.0 / 9.0),
                    _ => 0.0,
                }
            } else {
                own - h
            };
            flow[0] += dx as f32 * diff;
            flow[1] += dz as f32 * diff;
        }
        flow
    }
}

/// A fluid's own height in a block: amount / 9, where a source or falling fluid holds 8.
fn own_height(level: u8) -> f32 {
    let amount = if level == 0 || level >= 8 {
        8
    } else {
        8 - level
    };
    amount as f32 / 9.0
}

/// `LiquidBlockRenderer.calculateAverageHeight` for one top corner: the fluid's height and
/// its two edge neighbours' (and the diagonal's, when an edge neighbour holds fluid),
/// where heights of at least 0.8 count ten times and solid blocks (-1) don't count.
fn average_height(center: f32, side1: f32, side2: f32, diagonal: impl Fn() -> f32) -> f32 {
    if side1 >= 1.0 || side2 >= 1.0 {
        return 1.0;
    }
    let (mut sum, mut weight) = (0.0, 0.0);
    let mut add = |h: f32| {
        if h >= 0.8 {
            (sum, weight) = (sum + h * 10.0, weight + 10.0);
        } else if h >= 0.0 {
            (sum, weight) = (sum + h, weight + 1.0);
        }
    };
    if side1 > 0.0 || side2 > 0.0 {
        let d = diagonal();
        if d >= 1.0 {
            return 1.0;
        }
        add(d);
    }
    add(center);
    add(side1);
    add(side2);
    sum / weight
}

#[cfg(test)]
mod tests {
    use super::{average_height, own_height};

    /// Expected values worked by hand from vanilla's weighting.
    #[test]
    fn corner_heights_weight_tall_fluid_tenfold() {
        let source = own_height(0);
        assert_eq!(source, 8.0 / 9.0);
        assert_eq!(own_height(3), 5.0 / 9.0);
        assert_eq!(
            own_height(8),
            8.0 / 9.0,
            "falling fluid holds a full amount"
        );
        // A source next to level-3 water and air, with air diagonally: (8.889 + 5/9 + 0 + 0) / 13.
        let h = average_height(source, own_height(3), 0.0, || 0.0);
        assert!((h - (80.0 / 9.0 + 5.0 / 9.0) / 13.0).abs() < 1e-6, "{h}");
        // Against solid blocks only the fluid itself counts.
        assert_eq!(
            average_height(own_height(3), -1.0, -1.0, || panic!("no diagonal")),
            5.0 / 9.0
        );
        // Fluid above a neighbour makes the corner full.
        assert_eq!(average_height(source, 1.0, 0.0, || 0.0), 1.0);
    }
}
