//! Builds vertex data for loaded chunks: culled faces, smooth lighting, ambient occlusion,
//! biome tints, and simple water and lava.

use crate::atlas::Layer;
use crate::bake::{self, BlockModels, Dir, JavaRandom, LAVA_SPRITE, Quad, WATER_SPRITE};
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
    /// Sky and block light levels × 16 (0..240), then two unused bytes.
    pub light: [u8; 4],
}

/// Four vertices per quad, drawn as two triangles.
#[derive(Default)]
pub struct Mesh {
    pub opaque: Vec<Vertex>,
    pub cutout: Vec<Vertex>,
    pub translucent: Vec<Vertex>,
}

impl Mesh {
    fn layer(&mut self, layer: Layer) -> &mut Vec<Vertex> {
        match layer {
            Layer::Opaque => &mut self.opaque,
            Layer::Cutout => &mut self.cutout,
            Layer::Translucent => &mut self.translucent,
        }
    }

    /// Orders translucent quads far to near from `eye`, so blending looks right.
    pub fn sort_translucent(&mut self, eye: Vec3) {
        let mut quads: Vec<[Vertex; 4]> = self.translucent.as_chunks::<4>().0.to_vec();
        let distance = |q: &[Vertex; 4]| {
            (q.iter().map(|v| Vec3::from(v.pos)).sum::<Vec3>() / 4.0).distance_squared(eye)
        };
        quads.sort_by(|a, b| distance(b).total_cmp(&distance(a)));
        self.translucent = quads.into_iter().flatten().collect();
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
                tint: block_tint(name),
                fluid,
                offset,
                air: name.ends_with("air") && block.properties.is_empty(),
            });
        }
    }
    infos
}

/// The chunks around a point, loaded so the mesher can look across chunk edges.
pub struct WorldView {
    chunks: HashMap<(i32, i32), Chunk>,
    air: u32,
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
        let mut chunks = HashMap::new();
        for cx in ccx - radius..=ccx + radius {
            for cz in ccz - radius..=ccz + radius {
                if let Some(chunk) = world.chunk(registry, cx, cz)? {
                    chunks.insert((cx, cz), chunk);
                }
            }
        }
        let air = registry.block("minecraft:air").expect("air").default_state;
        Ok(Self { chunks, air })
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
    infos: Vec<StateInfo>,
}

impl<'a> Mesher<'a> {
    pub fn new(models: &'a BlockModels, biomes: &'a Biomes, registry: &Registry) -> Self {
        Self {
            models,
            biomes,
            infos: state_infos(registry),
        }
    }

    /// Meshes every loaded chunk whose four neighbours are loaded too, so no face along the
    /// edge of the loaded area is drawn against missing terrain.
    pub fn mesh_world(&self, view: &WorldView) -> Mesh {
        let mut mesh = Mesh::default();
        for (&(cx, cz), chunk) in &view.chunks {
            if ![(1, 0), (-1, 0), (0, 1), (0, -1)]
                .iter()
                .all(|(dx, dz)| view.chunks.contains_key(&(cx + dx, cz + dz)))
            {
                continue;
            }
            for section in &chunk.sections {
                if matches!(section.blocks, mc_world::Blocks::Uniform(s) if self.infos[s as usize].air)
                {
                    continue;
                }
                for ly in 0..16 {
                    for lz in 0..16 {
                        for lx in 0..16 {
                            let state = section.block(lx, ly, lz);
                            if self.infos[state as usize].air {
                                continue;
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
            }
        }
        mesh
    }

    fn occludes(&self, state: u32) -> bool {
        self.models.states[state as usize].occludes
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

        let mut random = JavaRandom::new(bake::position_seed(x, y, z));
        let part_seed = if model.parts.len() > 1 {
            Some(random.next_long())
        } else {
            None
        };
        for part in &model.parts {
            if let Some(seed) = part_seed {
                random = JavaRandom::new(seed);
            }
            let baked = bake::pick(part, &mut random);
            for quad in &baked.quads {
                if let Some(cull) = quad.cull {
                    let other = neighbor(cull);
                    if self.occludes(other) {
                        continue;
                    }
                    // Glass and ice hide the faces between two blocks of the same kind.
                    let same_kind = self.infos[other as usize].block == info.block;
                    if same_kind && quad.layer != Layer::Opaque && quad.on_face {
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

    /// Vanilla's default biome blend: the average over the 5×5 columns around the block.
    fn blended_tint(&self, view: &WorldView, tint: Tint, x: i32, y: i32, z: i32) -> [f32; 3] {
        if matches!(tint, Tint::Fixed(_)) {
            return self.biomes.color(tint, None);
        }
        let mut sum = [0.0; 3];
        for dx in -2..=2 {
            for dz in -2..=2 {
                let c = self.biomes.color(tint, view.biome(x + dx, y, z + dz));
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
        let shade = if quad.shade { quad.dir.shade() } else { 1.0 };
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
        let out = mesh.layer(quad.layer);
        for (i, (brightness, (sky, block_light))) in corners.into_iter().enumerate() {
            let color =
                tint.map(|c| (c * shade * brightness * 255.0).round().clamp(0.0, 255.0) as u8);
            out.push(Vertex {
                pos: (origin + quad.pos[i]).into(),
                uv: quad.uv[i],
                color: [color[0], color[1], color[2], 255],
                light: [(sky * 16.0) as u8, (block_light * 16.0) as u8, 0, 0],
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
            let state = view.state(p[0], p[1], p[2]);
            let shade = if self.occludes(state) { 0.2 } else { 1.0 };
            (shade, view.light(p[0], p[1], p[2]), self.occludes(state))
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

    /// Water and lava are drawn by code in vanilla, not by models: a box whose top sits at
    /// the fluid's height, with faces only where they touch something else.
    fn mesh_fluid(
        &self,
        view: &WorldView,
        mesh: &mut Mesh,
        (fluid, level): (Fluid, u8),
        [x, y, z]: [i32; 3],
    ) {
        let fluid_at = |x, y, z| self.infos[view.state(x, y, z) as usize].fluid.map(|f| f.0);
        let above = fluid_at(x, y + 1, z) == Some(fluid);
        let height = if above || level >= 8 {
            1.0
        } else {
            (8 - level.min(7)) as f32 / 9.0
        };
        let (sprite, layer, tint) = match fluid {
            Fluid::Water => (
                WATER_SPRITE,
                Layer::Translucent,
                self.blended_tint(view, Tint::Water, x, y, z),
            ),
            Fluid::Lava => (LAVA_SPRITE, Layer::Opaque, [1.0; 3]),
        };
        let sprite = self.models.atlas.sprite(sprite);
        let [u0, v0, u1, v1] = sprite.rect;
        let max = Vec3::new(1.0, height, 1.0);
        for dir in Dir::ALL {
            let [dx, dy, dz] = dir.offset();
            let (nx, ny, nz) = (x + dx, y + dy, z + dz);
            if fluid_at(nx, ny, nz) == Some(fluid)
                || (dir != Dir::Up && self.occludes(view.state(nx, ny, nz)))
            {
                continue;
            }
            let pos = bake::corners(dir, Vec3::ZERO, max);
            // Side faces only show the part of the texture the water covers.
            let side_v1 = v0
                + (v1 - v0)
                    * if matches!(dir, Dir::Up | Dir::Down) {
                        1.0
                    } else {
                        height
                    };
            let quad = Quad {
                pos,
                uv: [[u0, v0], [u0, side_v1], [u1, side_v1], [u1, v0]],
                dir,
                cull: None,
                tint: 0,
                shade: true,
                layer,
                on_face: dir != Dir::Up || height >= 1.0,
            };
            self.emit_quad(
                view,
                mesh,
                &quad,
                Vec3::new(x as f32, y as f32, z as f32),
                [x, y, z],
                tint,
                false,
            );
        }
    }
}
