//! Mipmaps as 26.3 makes them (`MipmapGenerator.generateMipLevels`). Each level averages
//! 2×2 pixels in linear light. Textures with clear pixels first spread their nearest opaque
//! colour into the clear ones (`TextureUtil.solidify`), so edges don't darken, and every
//! level's alpha is rescaled to keep as much of the sprite past the alpha test as the full
//! size has, so leaves and plants don't thin out or vanish in the distance.

use image::RgbaImage;
use std::collections::VecDeque;
use std::sync::OnceLock;

/// A texture's `mipmap_strategy` in its `.mcmeta` (`MipmapStrategy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// `Cutout` if the texture has a fully clear pixel, otherwise `Mean`.
    Auto,
    Mean,
    Cutout,
    /// As `Cutout`, measuring coverage at a lower alpha cutoff.
    StrictCutout,
    /// Clear pixels take a darkened colour, and clear pixels add nothing to the average.
    DarkCutout,
}

impl Strategy {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "auto" => Self::Auto,
            "mean" => Self::Mean,
            "cutout" => Self::Cutout,
            "strict_cutout" => Self::StrictCutout,
            "dark_cutout" => Self::DarkCutout,
            _ => return None,
        })
    }
}

/// `ARGB`'s lookup tables: sRGB byte to linear × 1023, and linear × 1023 to sRGB byte.
fn tables() -> &'static ([u16; 256], [u8; 1024]) {
    static TABLES: OnceLock<([u16; 256], [u8; 1024])> = OnceLock::new();
    TABLES.get_or_init(|| {
        let to_linear = |c: f32| {
            if c >= 0.04045 {
                ((c as f64 + 0.055) / 1.055).powf(2.4) as f32
            } else {
                c / 12.92
            }
        };
        let to_srgb = |c: f32| {
            if c >= 0.0031308 {
                (1.055 * (c as f64).powf(1.0 / 2.4) - 0.055) as f32
            } else {
                12.92 * c
            }
        };
        let linear = std::array::from_fn(|i| (to_linear(i as f32 / 255.0) * 1023.0).round() as u16);
        let srgb = std::array::from_fn(|i| (to_srgb(i as f32 / 1023.0) * 255.0).round() as u8);
        (linear, srgb)
    })
}

fn srgb_to_linear(c: u8) -> f32 {
    tables().0[c as usize] as f32 / 1023.0
}

fn linear_to_srgb(c: f32) -> u8 {
    tables().1[((c * 1023.0).floor() as usize).min(1023)]
}

/// `ARGB.meanLinear`: alpha averaged as is, colour averaged in linear light, clear pixels
/// included.
fn mean_linear(p: [[u8; 4]; 4]) -> [u8; 4] {
    let (linear, srgb) = tables();
    let channel = |i: usize| {
        let sum: u32 = p.iter().map(|px| linear[px[i] as usize] as u32).sum();
        srgb[(sum / 4) as usize]
    };
    let alpha = p.iter().map(|px| px[3] as u32).sum::<u32>() / 4;
    [channel(0), channel(1), channel(2), alpha as u8]
}

/// `MipmapGenerator.darkenedAlphaBlend`: clear pixels add nothing, but still count in the
/// four; alpha is averaged in linear light like the colour.
fn darkened_alpha_blend(p: [[u8; 4]; 4]) -> [u8; 4] {
    let mut sum = [0.0f32; 4];
    for px in p.iter().filter(|px| px[3] != 0) {
        for (s, c) in sum.iter_mut().zip(px) {
            *s += srgb_to_linear(*c);
        }
    }
    sum.map(|s| linear_to_srgb(s / 4.0))
}

/// `TextureUtil.solidify`: each clear pixel takes the colour of its nearest opaque one
/// (breadth first, neighbours in vanilla's order), and stays clear.
fn solidify(img: &mut RgbaImage) {
    const DIRECTIONS: [(i32, i32); 4] = [(0, 1), (1, 0), (0, -1), (-1, 0)];
    let (w, h) = (img.width() as i32, img.height() as i32);
    let pack = |x: i32, y: i32| (x + y * w) as usize;
    let mut color = vec![[0u8; 4]; (w * h) as usize];
    let mut distance = vec![i32::MAX; (w * h) as usize];
    let mut queue = VecDeque::new();
    for x in 0..w {
        for y in 0..h {
            let p = img.get_pixel(x as u32, y as u32).0;
            if p[3] != 0 {
                let i = pack(x, y);
                distance[i] = 0;
                color[i] = p;
                queue.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        let here = pack(x, y);
        for (dx, dy) in DIRECTIONS {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || ny < 0 || nx >= w || ny >= h {
                continue;
            }
            let there = pack(nx, ny);
            if distance[there] > distance[here] + 1 {
                distance[there] = distance[here] + 1;
                color[there] = color[here];
                queue.push_back((nx, ny));
            }
        }
    }
    for x in 0..w {
        for y in 0..h {
            let p = img.get_pixel_mut(x as u32, y as u32);
            if p[3] == 0 {
                let c = color[pack(x, y)];
                *p = image::Rgba([c[0], c[1], c[2], 0]);
            }
        }
    }
}

/// `TextureUtil.fillEmptyAreasWithDarkColor`: clear pixels take three quarters of the
/// darkest opaque pixel's colour.
fn fill_empty_areas_with_dark_color(img: &mut RgbaImage) {
    let mut darkest = [255u8; 4];
    let mut least = u32::MAX;
    // Column by column, as vanilla scans, so ties pick the same pixel.
    for x in 0..img.width() {
        for y in 0..img.height() {
            let p = img.get_pixel(x, y).0;
            let sum = p[0] as u32 + p[1] as u32 + p[2] as u32;
            if p[3] != 0 && sum < least {
                least = sum;
                darkest = p;
            }
        }
    }
    let dark = image::Rgba([
        (3 * darkest[0] as u32 / 4) as u8,
        (3 * darkest[1] as u32 / 4) as u8,
        (3 * darkest[2] as u32 / 4) as u8,
        0,
    ]);
    for p in img.pixels_mut().filter(|p| p[3] == 0) {
        *p = dark;
    }
}

/// `MipmapGenerator.alphaTestCoverage`: how much of the image, sampled bilinearly 4×4 times
/// per pixel, has alpha × `scale` above `cutoff`.
fn alpha_test_coverage(img: &RgbaImage, cutoff: f32, scale: f32) -> f32 {
    let (w, h) = (img.width(), img.height());
    let alpha = |x, y| (img.get_pixel(x, y)[3] as f32 / 255.0 * scale).clamp(0.0, 1.0);
    let mut coverage = 0.0f32;
    for y in 0..h.saturating_sub(1) {
        for x in 0..w.saturating_sub(1) {
            let (a00, a10, a01, a11) = (
                alpha(x, y),
                alpha(x + 1, y),
                alpha(x, y + 1),
                alpha(x + 1, y + 1),
            );
            let mut covered = 0.0f32;
            for sy in 0..4 {
                let fy = (sy as f32 + 0.5) / 4.0;
                for sx in 0..4 {
                    let fx = (sx as f32 + 0.5) / 4.0;
                    let a = a00 * (1.0 - fx) * (1.0 - fy)
                        + a10 * fx * (1.0 - fy)
                        + a01 * (1.0 - fx) * fy
                        + a11 * fx * fy;
                    if a > cutoff {
                        covered += 1.0;
                    }
                }
            }
            coverage += covered / 16.0;
        }
    }
    // A 1-pixel-wide level divides by zero, as vanilla's does; NaN then leaves it unscaled.
    coverage / ((w as i32 - 1) * (h as i32 - 1)) as f32
}

/// `MipmapGenerator.scaleAlphaToCoverage`: a bisection for the alpha scale whose coverage
/// is nearest `target`, then alpha × scale + `bias`, nudged up a little.
fn scale_alpha_to_coverage(img: &mut RgbaImage, target: f32, cutoff: f32, bias: f32) {
    let (mut low, mut high, mut scale) = (0.0f32, 4.0f32, 1.0f32);
    let (mut best_scale, mut best_diff) = (1.0f32, f32::MAX);
    for _ in 0..5 {
        let coverage = alpha_test_coverage(img, cutoff, scale);
        let diff = (coverage - target).abs();
        if diff < best_diff {
            best_diff = diff;
            best_scale = scale;
        }
        if coverage < target {
            low = scale;
        } else if coverage > target {
            high = scale;
        } else {
            break;
        }
        scale = (low + high) * 0.5;
    }
    for p in img.pixels_mut() {
        let a = (p[3] as f32 / 255.0 * best_scale + bias + 0.025).clamp(0.0, 1.0);
        p[3] = (a * 255.0).floor() as u8;
    }
}

/// The image and `levels` mipmaps below it. Clear pixels of the full-size image get colours
/// first for the cutout strategies, which changes nothing visible at full size.
pub fn generate(mut img: RgbaImage, strategy: Strategy, bias: f32, levels: u32) -> Vec<RgbaImage> {
    let strategy = match strategy {
        Strategy::Auto if img.pixels().any(|p| p[3] == 0) => Strategy::Cutout,
        Strategy::Auto => Strategy::Mean,
        s => s,
    };
    match strategy {
        Strategy::Cutout | Strategy::StrictCutout => solidify(&mut img),
        Strategy::DarkCutout => fill_empty_areas_with_dark_color(&mut img),
        _ => {}
    }
    let cutout = matches!(
        strategy,
        Strategy::Cutout | Strategy::StrictCutout | Strategy::DarkCutout
    );
    let cutoff = if strategy == Strategy::StrictCutout {
        0.3
    } else {
        0.5
    };
    let target = if cutout {
        alpha_test_coverage(&img, cutoff, 1.0)
    } else {
        0.0
    };
    let mut out = vec![img];
    for _ in 0..levels {
        let prev = out.last().expect("level 0");
        let (w, h) = (prev.width() >> 1, prev.height() >> 1);
        let mut next = RgbaImage::new(w, h);
        for x in 0..w {
            for y in 0..h {
                let px = |dx, dy| prev.get_pixel(x * 2 + dx, y * 2 + dy).0;
                let four = [px(0, 0), px(1, 0), px(0, 1), px(1, 1)];
                let blended = if strategy == Strategy::DarkCutout {
                    darkened_alpha_blend(four)
                } else {
                    mean_linear(four)
                };
                next.put_pixel(x, y, image::Rgba(blended));
            }
        }
        if cutout {
            scale_alpha_to_coverage(&mut next, target, cutoff, bias);
        }
        out.push(next);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plant-like sprite: a green blob on clear black. Its smaller levels must stay
    /// green where they're visible, rather than mixing in the clear pixels' black, and keep
    /// about as much of the sprite past the alpha test as the full size has.
    #[test]
    fn cutout_levels_keep_colour_and_coverage() {
        let img = RgbaImage::from_fn(16, 16, |x, y| {
            if (4..12).contains(&x) && (2..14).contains(&y) && (x + y) % 3 != 0 {
                image::Rgba([60, 160, 40, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        let full = alpha_test_coverage(&img, 0.5, 1.0);
        let levels = generate(img, Strategy::Auto, 0.0, 2);
        for level in &levels[1..] {
            for p in level.pixels().filter(|p| p[3] >= 128) {
                assert_eq!(p.0[..3], [60, 160, 40], "a visible pixel darkened: {p:?}");
            }
            let coverage = alpha_test_coverage(level, 0.5, 1.0);
            assert!(
                (coverage - full).abs() < 0.15,
                "coverage {coverage} vs {full}"
            );
        }
    }

    /// An opaque texture averages in linear light: black and white make sRGB 187 (vanilla's
    /// lookup tables truncate linear 511.5 to 511), not 128.
    #[test]
    fn mean_averages_in_linear_light() {
        let img = RgbaImage::from_fn(2, 2, |x, _| {
            let c = if x == 0 { 0 } else { 255 };
            image::Rgba([c, c, c, 255])
        });
        let levels = generate(img, Strategy::Auto, 0.0, 1);
        assert_eq!(levels[1].get_pixel(0, 0).0, [187, 187, 187, 255]);
    }
}
