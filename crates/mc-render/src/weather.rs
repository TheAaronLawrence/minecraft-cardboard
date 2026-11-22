//! Rain and snow: `WeatherEffectRenderer`'s columns of falling texture around the camera.

use crate::bake::JavaRandom;
use crate::mesh::Vertex;
use crate::tint::Precipitation;
use glam::DVec3;

/// `Options.weatherRadius`'s default: columns this many blocks around the camera.
pub const RADIUS: i32 = 10;

/// The quads to draw with `environment/rain.png` and with `environment/snow.png`.
#[derive(Default)]
pub struct Columns {
    pub rain: Vec<Vertex>,
    pub snow: Vec<Vertex>,
}

/// The world as the columns see it: a column's `MOTION_BLOCKING` height (the y above its
/// highest blocker), whether it rains or snows at a block (`ClientLevel.getPrecipitationAt`),
/// and the sky and block light at a block.
pub trait Weather {
    fn height(&self, x: i32, z: i32) -> i32;
    fn precipitation(&self, pos: [i32; 3]) -> Precipitation;
    fn light(&self, pos: [i32; 3]) -> [u8; 2];
}

/// `WeatherEffectRenderer.extractRenderState` and `prepareInstances`: a column per block
/// around `camera` that rains or snows, from the higher of its blocker and `RADIUS` below
/// the camera to `RADIUS` above, at the rain's `intensity` (`Level.getRainLevel`).
pub fn columns(
    world: &impl Weather,
    camera: DVec3,
    game_time: i64,
    partial: f32,
    intensity: f32,
    radius: i32,
) -> Columns {
    let mut out = Columns::default();
    if intensity <= 0.0 {
        return out;
    }
    let cam = camera.floor().as_ivec3();
    let radius_sq = (radius * radius) as f32;
    for z in cam.z - radius..=cam.z + radius {
        for x in cam.x - radius..=cam.x + radius {
            let terrain = world.height(x, z);
            let y0 = (cam.y - radius).max(terrain);
            let y1 = (cam.y + radius).max(terrain);
            if y1 == y0 {
                continue;
            }
            let precipitation = world.precipitation([x, cam.y, z]);
            if precipitation == Precipitation::None {
                continue;
            }
            let seed = x
                .wrapping_mul(x)
                .wrapping_mul(3121)
                .wrapping_add(x.wrapping_mul(45238971))
                ^ z.wrapping_mul(z)
                    .wrapping_mul(418711)
                    .wrapping_add(z.wrapping_mul(13761));
            let mut random = JavaRandom::new(i64::from(seed));
            let [sky, block] = world.light([x, cam.y.max(terrain), z]);
            let wrapped = (game_time & 131071) as i32;
            let (u, v, light, max_alpha, quads) = match precipitation {
                // `createRainColumnInstance`: each column falls at its own speed.
                Precipitation::Rain => {
                    let tick_offset = (x
                        .wrapping_mul(x)
                        .wrapping_mul(3121)
                        .wrapping_add(x.wrapping_mul(45238971))
                        .wrapping_add(z.wrapping_mul(z).wrapping_mul(418711))
                        .wrapping_add(z.wrapping_mul(13761)))
                        & 0xFF;
                    let speed = 3.0 + random.next_float();
                    let offset = -((wrapped + tick_offset) as f32 + partial) / 32.0 * speed % 32.0;
                    (0.0, offset, [sky, block], 1.0, &mut out.rain)
                }
                // `createSnowColumnInstance`: snow drifts sideways a little and falls
                // slowly, a quarter brighter toward full light.
                _ => {
                    let time = wrapped as f32 + partial;
                    let u = random.next_double();
                    let (gu, gv) = gaussians(&mut random);
                    let u = (u + f64::from(time * 0.01 * gu as f32)) as f32;
                    let v = random.next_double();
                    let v = (v + f64::from(time * gv as f32 * 0.001)) as f32;
                    let offset = -((game_time & 511) as f32 + partial) / 512.0;
                    let bright = |l: u8| (l * 3 + 15) / 4;
                    (
                        u,
                        offset + v,
                        [bright(sky), bright(block)],
                        0.8,
                        &mut out.snow,
                    )
                }
            };
            // `prepareInstances`: fading out toward the edge, a quad facing the camera's
            // column, its texture four blocks a repeat.
            let rel_x = (f64::from(x) + 0.5 - camera.x) as f32;
            let rel_z = (f64::from(z) + 0.5 - camera.z) as f32;
            let distance_sq = rel_x * rel_x + rel_z * rel_z;
            let t = (distance_sq / radius_sq).min(1.0);
            let alpha = (max_alpha + (0.5 - max_alpha) * t) * intensity;
            let color = [255, 255, 255, (alpha * 255.0).floor() as u8];
            let (dx, dz) = ((x - cam.x) as f32, (z - cam.z) as f32);
            let length = (dx * dx + dz * dz).sqrt();
            // The camera's own column has no direction; vanilla's quad there collapses.
            if length == 0.0 {
                continue;
            }
            let (half_x, half_z) = (-dz / length / 2.0, dx / length / 2.0);
            let (u0, u1) = (u, u + 1.0);
            let (v0, v1) = (y0 as f32 * 0.25 + v, y1 as f32 * 0.25 + v);
            let light = [light[0] * 16, light[1] * 16, 0, 0];
            let corner = |cx: f32, y: i32, cz: f32, uv: [f32; 2]| Vertex {
                pos: [
                    (camera.x + f64::from(cx)) as f32,
                    y as f32,
                    (camera.z + f64::from(cz)) as f32,
                ],
                uv,
                color,
                light,
            };
            let corners = [
                corner(rel_x - half_x, y1, rel_z - half_z, [u0, v0]),
                corner(rel_x + half_x, y1, rel_z + half_z, [u1, v0]),
                corner(rel_x + half_x, y0, rel_z + half_z, [u1, v1]),
                corner(rel_x - half_x, y0, rel_z - half_z, [u0, v1]),
            ];
            quads.extend(corners);
        }
    }
    out
}

/// `LightningBoltRenderer.submit`: a bolt struck at `pos`, from its `seed`. Four layers,
/// each wider than the last, of a trunk eight 16-block segments tall that wanders back to
/// the strike, and two branches off its top, each segment a box of four quads.
pub fn lightning(pos: DVec3, seed: i64) -> Vec<Vertex> {
    let mut x_offsets = [0.0f32; 8];
    let mut z_offsets = [0.0f32; 8];
    let (mut x_offset, mut z_offset) = (0.0f32, 0.0f32);
    let mut random = JavaRandom::new(seed);
    for i in (0..8).rev() {
        x_offsets[i] = x_offset;
        z_offsets[i] = z_offset;
        x_offset += (random.next_int(11) - 5) as f32;
        z_offset += (random.next_int(11) - 5) as f32;
    }
    // `setColor(0.45F, 0.45F, 0.5F, 0.3F)`, each channel rounded down to a byte.
    let color = [0.45f32, 0.45, 0.5, 0.3].map(|c| (c * 255.0).floor() as u8);
    let mut out = Vec::new();
    for layer in 0..4 {
        let mut random = JavaRandom::new(seed);
        for branch in 0..3 {
            let trunk = branch == 0;
            let start = 7 - branch;
            let end = if trunk { 0 } else { start - 3 + 1 };
            let mut start_x = x_offsets[start as usize] - x_offset;
            let mut start_z = z_offsets[start as usize] - z_offset;
            for segment in (end..=start).rev() {
                let (end_x, end_z) = (start_x, start_z);
                let spread = if trunk { 5 } else { 15 };
                start_x += (random.next_int(spread * 2 + 1) - spread) as f32;
                start_z += (random.next_int(spread * 2 + 1) - spread) as f32;
                let mut top = 0.1 + layer as f32 * 0.2;
                let mut bottom = top;
                if trunk {
                    top *= segment as f32 * 0.1 + 1.0;
                    bottom *= (segment - 1) as f32 * 0.1 + 1.0;
                }
                // `quad`'s sides: which way each edge's corners sit from the segment.
                for (right_x, right_z, left_x, left_z) in [
                    (false, false, true, false),
                    (true, false, true, true),
                    (true, true, false, true),
                    (false, true, false, false),
                ] {
                    let sign = |positive: bool, r: f32| if positive { r } else { -r };
                    let corner = |x: f32, y: i32, z: f32| Vertex {
                        pos: [
                            (pos.x + f64::from(x)) as f32,
                            (pos.y + f64::from(y)) as f32,
                            (pos.z + f64::from(z)) as f32,
                        ],
                        uv: [0.0; 2],
                        color,
                        light: [0, 0, 0, crate::mesh::UNLIT],
                    };
                    out.extend([
                        corner(
                            start_x + sign(right_x, bottom),
                            segment * 16,
                            start_z + sign(right_z, bottom),
                        ),
                        corner(
                            end_x + sign(right_x, top),
                            (segment + 1) * 16,
                            end_z + sign(right_z, top),
                        ),
                        corner(
                            end_x + sign(left_x, top),
                            (segment + 1) * 16,
                            end_z + sign(left_z, top),
                        ),
                        corner(
                            start_x + sign(left_x, bottom),
                            segment * 16,
                            start_z + sign(left_z, bottom),
                        ),
                    ]);
                }
            }
        }
    }
    out
}

/// `MarsagliaPolarGaussian.nextGaussian` twice, from a fresh source: the pair it makes.
fn gaussians(random: &mut JavaRandom) -> (f64, f64) {
    loop {
        let x = 2.0 * random.next_double() - 1.0;
        let y = 2.0 * random.next_double() - 1.0;
        let radius_sq = x * x + y * y;
        if radius_sq < 1.0 && radius_sq != 0.0 {
            let multiplier = (-2.0 * radius_sq.ln() / radius_sq).sqrt();
            return (x * multiplier, y * multiplier);
        }
    }
}
