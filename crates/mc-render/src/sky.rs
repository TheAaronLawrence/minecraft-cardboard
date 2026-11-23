//! The overworld's sky as `SkyRenderer` draws it, around the eye: the sunrise and sunset fan,
//! the sun, the moon in its phase, and the stars; and the clouds as `CloudRenderer` lays
//! them out, a cell of `clouds.png` to every 12 × 12 blocks, drifting east with the game
//! time. The sky disc and the dark disc below the horizon are drawn in the shader.

use crate::environment::{Skybox, Visuals};
use crate::mesh::{UNLIT, Vertex};
use glam::{Mat4, Vec3};

pub const SUN_SPRITE: &str = "minecraft:environment/celestial/sun";

/// `MoonPhase`'s textures, by index.
const MOON_PHASES: [&str; 8] = [
    "full_moon",
    "waning_gibbous",
    "third_quarter",
    "waning_crescent",
    "new_moon",
    "waxing_crescent",
    "first_quarter",
    "waxing_gibbous",
];

fn moon_sprite(phase: u8) -> String {
    format!(
        "minecraft:environment/celestial/moon/{}",
        MOON_PHASES[phase as usize % 8]
    )
}

/// The celestial sprites, for the atlas.
pub fn sprites() -> impl Iterator<Item = String> {
    std::iter::once(SUN_SPRITE.to_owned()).chain((0..8).map(moon_sprite))
}

/// What the sky looks like this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyState {
    pub visuals: Visuals,
    pub skybox: Skybox,
    /// The eye is below the horizon (sea level, or a superflat world's floor), so the dark
    /// disc covers the sky below it (`SkyRenderer.shouldRenderDarkDisc`).
    pub dark_disc: bool,
    /// How far the eye is into the void's darkness below the world, 0 to 1, which darkens
    /// the fog (`FogRenderer.computeFogColor`).
    pub void_darkness: f32,
    /// The lightmap's block light factor: 1.4 and a flicker
    /// (`LightmapRenderStateExtractor.blockLightFlicker`).
    pub block_factor: f32,
    /// The game time and the partial tick, which move the clouds.
    pub game_time: i64,
    pub partial_tick: f32,
    /// `Level.getRainLevel` and `getThunderLevel`: the rain hides the sun and moon and
    /// darkens the fog.
    pub rain: f32,
    pub thunder: f32,
    /// `AtmosphericFogEnvironment.rainFogMultiplier`: how far rain pulls the fog in.
    pub rain_fog: f32,
    /// `Hud.vignetteBrightness`: how much the vignette darkens the view's edges, 0 to 1.
    pub vignette: f32,
}

impl Default for SkyState {
    /// Noon in the plains, for a world without a clock, such as a save.
    fn default() -> Self {
        let hex = |c: u32| [(c >> 16) as u8, (c >> 8) as u8, c as u8].map(|v| v as f32 / 255.0);
        Self {
            visuals: Visuals {
                sky_color: hex(0x78A7FF),
                fog_color: hex(0xC0D8FF),
                fog_start: 0.0,
                fog_end: 1024.0,
                sky_fog_end: 512.0,
                cloud_fog_end: 2048.0,
                cloud_color: [1.0, 1.0, 1.0, 0.8],
                cloud_height: 192.33,
                sun_angle: 0.0,
                moon_angle: 180.0,
                star_angle: 0.0,
                star_brightness: 0.0,
                sunrise_color: [0.0; 4],
                moon_phase: 0,
                sky_light_color: [1.0; 3],
                sky_light_factor: 1.0,
                ambient_color: [10.0 / 255.0; 3],
                block_light_tint: [1.0, 0xD8 as f32 / 255.0, 0x8C as f32 / 255.0],
                sky_light_level: 15.0,
            },
            skybox: Skybox::Overworld,
            dark_disc: false,
            void_darkness: 0.0,
            block_factor: 1.4,
            game_time: 6000,
            partial_tick: 0.0,
            rain: 0.0,
            thunder: 0.0,
            rain_fog: 0.0,
            vignette: 0.0,
        }
    }
}

/// `SkyRenderer.buildStars`: 1500 tries at a star, from the same seed, as quads 100 blocks
/// out, before the sky turns them.
pub fn stars() -> Vec<[Vec3; 4]> {
    let mut random = crate::bake::JavaRandom::new(10842);
    let mut stars = Vec::new();
    for _ in 0..1500 {
        let x = random.next_float() * 2.0 - 1.0;
        let y = random.next_float() * 2.0 - 1.0;
        let z = random.next_float() * 2.0 - 1.0;
        let size = 0.15 + random.next_float() * 0.1;
        let length_sq = x * x + y * y + z * z;
        if length_sq <= 0.010000001 || length_sq >= 1.0 {
            continue;
        }
        let center = Vec3::new(x, y, z).normalize() * 100.0;
        let z_rot = (random.next_double() * std::f32::consts::PI as f64 * 2.0) as f32;
        // `Matrix3f.rotateTowards(-center, up)`: +z to the centre's opposite, then turned
        // about z.
        let dir = (-center).normalize();
        let left = Vec3::Y.cross(dir).normalize();
        let up = dir.cross(left);
        let rotation = glam::Mat3::from_cols(left, up, dir) * glam::Mat3::from_rotation_z(-z_rot);
        stars.push(
            [(size, -size), (size, size), (-size, size), (-size, -size)]
                .map(|(a, b)| rotation * Vec3::new(a, b, 0.0) + center),
        );
    }
    stars
}

/// The sky's triangles, by how they blend, in eye-relative positions.
#[derive(Default)]
pub struct SkyMesh {
    /// The sunrise and sunset fan, blended over the sky.
    pub sunrise: Vec<Vertex>,
    /// The sun and moon, textured and added to the sky.
    pub celestial: Vec<Vertex>,
    /// The stars, added to the sky.
    pub stars: Vec<Vertex>,
}

fn vertex(pos: Vec3, uv: [f32; 2], color: [f32; 4]) -> Vertex {
    Vertex {
        pos: pos.to_array(),
        uv,
        color: color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8),
        light: [0, 0, 0, UNLIT],
    }
}

/// Two triangles for a quad's corners in order.
fn quad(out: &mut Vec<Vertex>, corners: [Vertex; 4]) {
    out.extend([0, 1, 2, 0, 2, 3].map(|i| corners[i]));
}

/// `SkyRenderer.render`'s sunrise, sun, moon and stars, for the overworld sky.
pub fn mesh(state: &SkyState, sprite: &dyn Fn(&str) -> [f32; 4], stars: &[[Vec3; 4]]) -> SkyMesh {
    let mut mesh = SkyMesh::default();
    if state.skybox != Skybox::Overworld {
        return mesh;
    }
    let v = &state.visuals;
    let sun = v.sun_angle.to_radians();

    // `renderSunriseAndSunset`: a fan from straight ahead of the sun's side of the sky.
    let sunrise = v.sunrise_color;
    if sunrise[3] > 0.001 {
        let angle: f32 = if sun.sin() < 0.0 { 180.0 } else { 0.0 };
        let m = Mat4::from_rotation_x(90f32.to_radians())
            * Mat4::from_rotation_z((angle + 90.0).to_radians())
            * Mat4::from_scale(Vec3::new(1.0, 1.0, sunrise[3]));
        let center = vertex(
            m.transform_point3(Vec3::new(0.0, 100.0, 0.0)),
            [0.0; 2],
            sunrise,
        );
        let ring: Vec<Vertex> = (0..=16)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 16.0;
                let p = Vec3::new(a.sin() * 120.0, a.cos() * 120.0, -a.cos() * 40.0);
                vertex(
                    m.transform_point3(p),
                    [0.0; 2],
                    [sunrise[0], sunrise[1], sunrise[2], 0.0],
                )
            })
            .collect();
        for pair in ring.windows(2) {
            mesh.sunrise.extend([center, pair[0], pair[1]]);
        }
    }

    // `renderSunMoonAndStars`: all turned a quarter west, then each by its own angle.
    let turn = Mat4::from_rotation_y(-90f32.to_radians());
    let body = |angle: f32, size: f32, name: &str, flip: bool| -> [Vertex; 4] {
        let m = turn
            * Mat4::from_rotation_x(angle.to_radians())
            * Mat4::from_translation(Vec3::new(0.0, 100.0, 0.0))
            * Mat4::from_scale(Vec3::new(size, 1.0, size));
        let [u0, v0, u1, v1] = sprite(name);
        // The moon's texture is turned half round (`buildMoonPhases`).
        let uvs = if flip {
            [[u1, v1], [u0, v1], [u0, v0], [u1, v0]]
        } else {
            [[u0, v0], [u1, v0], [u1, v1], [u0, v1]]
        };
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        std::array::from_fn(|i| {
            let (x, z) = corners[i];
            // The rain fades them out (`SkyRenderState.rainBrightness`).
            let color = [1.0, 1.0, 1.0, 1.0 - state.rain];
            vertex(m.transform_point3(Vec3::new(x, 0.0, z)), uvs[i], color)
        })
    };
    quad(
        &mut mesh.celestial,
        body(v.sun_angle, 30.0, SUN_SPRITE, false),
    );
    let moon = moon_sprite(v.moon_phase);
    quad(&mut mesh.celestial, body(v.moon_angle, 20.0, &moon, true));
    if v.star_brightness > 0.0 {
        let m = turn * Mat4::from_rotation_x(v.star_angle.to_radians());
        let b = v.star_brightness;
        for star in stars {
            let corners = star.map(|p| vertex(m.transform_point3(p), [0.0; 2], [b; 4]));
            quad(&mut mesh.stars, corners);
        }
    }
    mesh
}

/// `clouds.png` as `CloudRenderer` reads it: per cell, 0 when clear, else 1 and a bit for
/// each side whose neighbour is clear (`packCellData`, without the colour, which isn't used).
pub struct Clouds {
    cells: Vec<u8>,
    width: i32,
    height: i32,
}

const CELL: f32 = 12.0;
/// Bits for clear neighbours: west, south, east, north.
const WEST: u8 = 2;
const SOUTH: u8 = 4;
const EAST: u8 = 8;
const NORTH: u8 = 16;

/// Where the camera is against the cloud layer (`CloudRenderer.RelativeCameraPos`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relative {
    Above,
    Inside,
    Below,
}

/// What a cloud mesh depends on: the cell the camera is over and where it is against the
/// layer. The mesh is rebuilt when either changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloudKey {
    pub cell: (i32, i32),
    pub relative: Relative,
}

impl Clouds {
    pub fn new(img: &image::RgbaImage) -> Self {
        let (width, height) = (img.width() as i32, img.height() as i32);
        let clear = |x: i32, y: i32| img.get_pixel(x as u32, y as u32)[3] < 10;
        let mut cells = vec![0u8; (width * height) as usize];
        for y in 0..height {
            for x in 0..width {
                if clear(x, y) {
                    continue;
                }
                let mut cell = 1;
                // Vanilla wraps x by the height, which is the same for its square texture.
                for (bit, (nx, ny)) in [
                    (NORTH, (x, (y - 1).rem_euclid(height))),
                    (EAST, ((x + 1).rem_euclid(height), y)),
                    (SOUTH, (x, (y + 1).rem_euclid(height))),
                    (WEST, ((x - 1).rem_euclid(height), y)),
                ] {
                    if clear(nx, ny) {
                        cell |= bit;
                    }
                }
                cells[(x + y * width) as usize] = cell;
            }
        }
        Self {
            cells,
            width,
            height,
        }
    }

    /// `CloudRenderer.prepare`: the cell the camera is over, where it is against the
    /// layer, and the offset (eye-relative) of that cell's corner, with clouds at `height`
    /// that have drifted for `ticks` game ticks.
    pub fn place(&self, eye: [f64; 3], height: f32, ticks: i64, partial: f32) -> (CloudKey, Vec3) {
        let relative_bottom = (height as f64 - eye[1]) as f32;
        let relative_top = relative_bottom + 4.0;
        let relative = if relative_top < 0.0 {
            Relative::Above
        } else if relative_bottom > 0.0 {
            Relative::Below
        } else {
            Relative::Inside
        };
        let offset = (ticks.rem_euclid(self.width as i64 * 400)) as f32 + partial;
        let mut x = eye[0] + (offset * 0.030000001) as f64;
        let mut z = eye[2] + 3.96f32 as f64;
        let (w, h) = (self.width as f64 * 12.0, self.height as f64 * 12.0);
        x -= (x / w).floor() * w;
        z -= (z / h).floor() * h;
        let (cx, cz) = ((x / 12.0).floor() as i32, (z / 12.0).floor() as i32);
        let in_x = (x - cx as f64 * 12.0) as f32;
        let in_z = (z - cz as f64 * 12.0) as f32;
        (
            CloudKey {
                cell: (cx, cz),
                relative,
            },
            Vec3::new(-in_x, relative_bottom, -in_z),
        )
    }

    /// `CloudRenderer.buildMesh` for clouds `radius` cells around the camera's: each face
    /// packed for the cloud shader as x and z (relative cells, plus 512), the `Direction`'s
    /// 3D value, whether it faces in, and whether it takes the top's colour. Fancy clouds
    /// are boxes (`buildExtrudedCell`); fast ones a flat bottom face per cell in the top's
    /// colour (`buildFlatCell`). Nearer cells come first.
    pub fn faces(&self, key: CloudKey, radius: i32, fancy: bool) -> Vec<u32> {
        let mut faces = Vec::new();
        let (cx, cz) = key.cell;
        let mut cell = |x: i32, z: i32| {
            let ix = (cx + x).rem_euclid(self.width);
            let iz = (cz + z).rem_euclid(self.height);
            let data = self.cells[(ix + iz * self.width) as usize];
            if data == 0 {
                return;
            }
            if !fancy {
                faces.push((x + 512) as u32 | ((z + 512) as u32) << 10 | 1 << 24);
                return;
            }
            let mut face = |direction: u32, inside: bool| {
                faces.push(
                    (x + 512) as u32
                        | ((z + 512) as u32) << 10
                        | direction << 20
                        | u32::from(inside) << 23,
                );
            };
            if key.relative != Relative::Below {
                face(1, false);
            }
            if key.relative != Relative::Above {
                face(0, false);
            }
            if data & NORTH != 0 && z > 0 {
                face(2, false);
            }
            if data & SOUTH != 0 && z < 0 {
                face(3, false);
            }
            if data & WEST != 0 && x > 0 {
                face(4, false);
            }
            if data & EAST != 0 && x < 0 {
                face(5, false);
            }
            if x.abs() <= 1 && z.abs() <= 1 {
                for direction in 0..6 {
                    face(direction, true);
                }
            }
        };
        for ring in 0..=2 * radius {
            for x in -ring..=ring {
                let z = ring - x.abs();
                if z >= 0 && z <= radius && x * x + z * z <= radius * radius {
                    if z != 0 {
                        cell(x, -z);
                    }
                    cell(x, z);
                }
            }
        }
        faces
    }
}

/// Vanilla's cloud cell size in blocks, as the shader takes it.
pub const CLOUD_CELL: [f32; 3] = [CELL, 4.0, CELL];
