//! `SimplexNoise` in two dimensions, as biomes sample it for their temperature: the
//! permutation shuffled by a legacy random source from a fixed seed, with no offset
//! (`discardNoiseOffset`).

use crate::bake::JavaRandom;

/// `GradientNoise.GRADIENT`'s first twelve, x and y; simplex noise in two dimensions uses
/// no others.
const GRADIENT: [[f64; 2]; 12] = [
    [1.0, 1.0],
    [-1.0, 1.0],
    [1.0, -1.0],
    [-1.0, -1.0],
    [1.0, 0.0],
    [-1.0, 0.0],
    [1.0, 0.0],
    [-1.0, 0.0],
    [0.0, 1.0],
    [0.0, -1.0],
    [0.0, 1.0],
    [0.0, -1.0],
];

pub struct SimplexNoise {
    perms: [u8; 256],
}

impl SimplexNoise {
    /// `new SimplexNoise(new WorldgenRandom(new LegacyRandomSource(seed)), true)`.
    pub fn new(seed: i64) -> Self {
        Self::from_random(&mut JavaRandom::new(seed))
    }

    /// `GradientNoise`'s constructor: three doubles for an offset (discarded), then the
    /// shuffle.
    pub fn from_random(random: &mut JavaRandom) -> Self {
        for _ in 0..3 {
            random.next_double();
        }
        let mut perms: [u8; 256] = std::array::from_fn(|i| i as u8);
        for i in 0..256 {
            let offset = random.next_int(256 - i as i32) as usize;
            perms.swap(i, offset + i);
        }
        Self { perms }
    }

    fn permute(&self, x: i32) -> i32 {
        i32::from(self.perms[(x & 0xFF) as usize])
    }

    /// `SimplexNoise.get(x, y)`.
    pub fn get(&self, x: f64, y: f64) -> f32 {
        let sqrt3 = 3f64.sqrt();
        let f2 = 0.5 * (sqrt3 - 1.0);
        let g2 = (3.0 - sqrt3) / 6.0;
        let s = (x + y) * f2;
        let i = (x + s).floor() as i32;
        let j = (y + s).floor() as i32;
        let t = f64::from(i + j) * g2;
        let x0 = x - (f64::from(i) - t);
        let y0 = y - (f64::from(j) - t);
        let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
        let x1 = x0 - f64::from(i1) + g2;
        let y1 = y0 - f64::from(j1) + g2;
        let x2 = x0 - 1.0 + 2.0 * g2;
        let y2 = y0 - 1.0 + 2.0 * g2;
        let (ii, jj) = (i & 0xFF, j & 0xFF);
        let gi0 = self.permute(ii + self.permute(jj)) % 12;
        let gi1 = self.permute(ii + i1 + self.permute(jj + j1)) % 12;
        let gi2 = self.permute(ii + 1 + self.permute(jj + 1)) % 12;
        let corner = |gi: i32, x: f64, y: f64| {
            let t = 0.5 - x * x - y * y;
            if t < 0.0 {
                0.0
            } else {
                let [gx, gy] = GRADIENT[gi as usize];
                t * t * t * t * (gx * x + gy * y)
            }
        };
        (70.0 * (corner(gi0, x0, y0) + corner(gi1, x1, y1) + corner(gi2, x2, y2))) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::SimplexNoise;

    /// Values Java 25 printed for vanilla's `Biome.TEMPERATURE_NOISE` and
    /// `BIOME_INFO_NOISE`.
    #[test]
    fn matches_vanilla() {
        let temperature = SimplexNoise::new(1234);
        let info = SimplexNoise::new(2345);
        for (noise, x, y, expected) in [
            (&temperature, 0.5, 0.25, TEMPERATURE[0]),
            (&temperature, -123.875, 456.125, TEMPERATURE[1]),
            (&temperature, 3750.0, -1250.5, TEMPERATURE[2]),
            (&info, 10.2, -7.4, INFO[0]),
        ] {
            assert_eq!(noise.get(x, y), expected, "{x}, {y}");
        }
    }

    const TEMPERATURE: [f32; 3] = [-0.522_368_97, 0.440_347_58, 0.194_783_02];
    const INFO: [f32; 1] = [-0.139_453_13];
}
