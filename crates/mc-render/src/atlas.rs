//! One texture holding every block sprite.

use anyhow::Result;
use mc_assets::{MISSING_SPRITE, Resources};
use std::collections::HashMap;

/// How a sprite blends, which decides the pass it's drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Opaque,
    /// Pixels are fully opaque or fully clear (leaves, glass, flowers).
    Cutout,
    /// Some pixels are partly see-through (stained glass, ice, water).
    Translucent,
}

pub struct Atlas {
    pub width: u32,
    pub height: u32,
    /// RGBA, row by row: the full-size atlas, then each mipmap level at half the size of
    /// the one before.
    pub levels: Vec<Vec<u8>>,
    sprites: HashMap<String, Sprite>,
}

/// Vanilla's default "Mipmap Levels" option.
pub const MIP_LEVELS: u32 = 4;

#[derive(Debug, Clone, Copy)]
pub struct Sprite {
    /// u0, v0, u1, v1 in 0..1 atlas coordinates.
    pub rect: [f32; 4],
    pub layer: Layer,
}

/// `block/stone` and `minecraft:block/stone` are the same sprite.
pub fn normalize(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

impl Atlas {
    /// Loads and packs the sprites, plus `extra` images that aren't resource files, such as
    /// downloaded skins. A sprite without a texture file, and the built-in `missingno`,
    /// become the purple and black checkerboard.
    pub fn build(
        res: &Resources,
        ids: impl IntoIterator<Item = String>,
        extra: &[(String, image::RgbaImage)],
    ) -> Result<Self> {
        let mut images: Vec<(String, image::RgbaImage)> = extra.to_vec();
        let mut ids: Vec<String> = ids.into_iter().map(|id| normalize(&id)).collect();
        ids.push(MISSING_SPRITE.to_owned());
        ids.sort();
        ids.dedup();
        for id in ids {
            let image = if id == MISSING_SPRITE {
                None
            } else {
                load_first_frame(res, &id)
            };
            images.push((id, image.unwrap_or_else(missing_texture)));
        }

        // Shelf packing, tallest first. Slots are whole multiples of 16 pixels, so every
        // sprite starts on a boundary that stays a whole pixel in each mipmap level.
        images.sort_by_key(|(_, img)| std::cmp::Reverse((img.height(), img.width())));
        const WIDTH: u32 = 2048;
        let slot = |n: u32| n.next_multiple_of(1 << MIP_LEVELS);
        let (mut x, mut y, mut shelf) = (0, 0, 0);
        let mut placed = Vec::with_capacity(images.len());
        for (id, img) in images {
            if x + slot(img.width()) > WIDTH {
                (x, y, shelf) = (0, y + shelf, 0);
            }
            shelf = shelf.max(slot(img.height()));
            let next = x + slot(img.width());
            placed.push((id, img, x, y));
            x = next;
        }
        let height = (y + shelf).next_power_of_two();

        let mut levels: Vec<Vec<u8>> = (0..=MIP_LEVELS)
            .map(|l| vec![0u8; ((WIDTH >> l) * (height >> l) * 4) as usize])
            .collect();
        let mut sprites = HashMap::new();
        for (id, img, x, y) in placed {
            let layer = img.pixels().fold(Layer::Opaque, |layer, p| match p[3] {
                255 => layer,
                0 => layer.max(Layer::Cutout),
                _ => Layer::Translucent,
            });
            // Each sprite gets its own mipmaps, so neighbours never bleed into it.
            let mut mip = img.clone();
            for (level, pixels) in levels.iter_mut().enumerate() {
                if level > 0 {
                    mip = downsample(&mip, layer != Layer::Opaque);
                }
                let (lx, ly, level_width) = (x >> level, y >> level, WIDTH >> level);
                for row in 0..mip.height() {
                    let src = &mip.as_raw()
                        [(row * mip.width() * 4) as usize..((row + 1) * mip.width() * 4) as usize];
                    let start = (((ly + row) * level_width + lx) * 4) as usize;
                    pixels[start..start + src.len()].copy_from_slice(src);
                }
            }
            let rect = [
                x as f32 / WIDTH as f32,
                y as f32 / height as f32,
                (x + img.width()) as f32 / WIDTH as f32,
                (y + img.height()) as f32 / height as f32,
            ];
            sprites.insert(id, Sprite { rect, layer });
        }
        Ok(Self {
            width: WIDTH,
            height,
            levels,
            sprites,
        })
    }

    pub fn contains(&self, id: &str) -> bool {
        self.sprites.contains_key(&normalize(id))
    }

    pub fn sprite(&self, id: &str) -> Sprite {
        let id = normalize(id);
        self.sprites
            .get(&id)
            .or_else(|| self.sprites.get(MISSING_SPRITE))
            .copied()
            .expect("missingno is always in the atlas")
    }
}

/// An animated texture stacks its frames vertically; we draw the first.
fn load_first_frame(res: &Resources, id: &str) -> Option<image::RgbaImage> {
    let path = res.asset_path("textures", id, "png");
    let img = image::open(&path).ok()?.to_rgba8();
    let meta: Option<serde_json::Value> = std::fs::read(path.with_extension("png.mcmeta"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let animation = meta.as_ref().and_then(|m| m.get("animation"));
    let Some(animation) = animation else {
        return Some(img);
    };
    let size = |key: &str| {
        animation
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as u32)
    };
    let width = size("width").unwrap_or(img.width().min(img.height()));
    let height = size("height").unwrap_or(width);
    Some(
        image::imageops::crop_imm(&img, 0, 0, width.min(img.width()), height.min(img.height()))
            .to_image(),
    )
}

fn missing_texture() -> image::RgbaImage {
    image::RgbaImage::from_fn(16, 16, |x, y| {
        if (x < 8) ^ (y < 8) {
            image::Rgba([248, 0, 248, 255])
        } else {
            image::Rgba([0, 0, 0, 255])
        }
    })
}

/// One mipmap step, as vanilla's `MipmapGenerator`: each pixel is the gamma-2.2 average of
/// four. For sprites with see-through pixels, clear pixels add nothing and a result less
/// than 96/255 opaque becomes fully clear, so leaves and glass keep crisp edges.
fn downsample(img: &image::RgbaImage, transparent: bool) -> image::RgbaImage {
    let pow22 = |c: u8| (c as f32 / 255.0).powf(2.2);
    let back = |v: f32| ((v / 4.0).powf(1.0 / 2.2) * 255.0) as u8;
    let (w, h) = ((img.width() / 2).max(1), (img.height() / 2).max(1));
    image::RgbaImage::from_fn(w, h, |x, y| {
        let px = |dx: u32, dy: u32| {
            img.get_pixel(
                (x * 2 + dx).min(img.width() - 1),
                (y * 2 + dy).min(img.height() - 1),
            )
            .0
        };
        let four = [px(0, 0), px(1, 0), px(0, 1), px(1, 1)];
        let mut sum = [0.0f32; 4];
        for p in four {
            if transparent && p[3] == 0 {
                continue;
            }
            (0..4).for_each(|i| sum[i] += pow22(p[i]));
        }
        let mut out = sum.map(back);
        if transparent && out[3] < 96 {
            out[3] = 0;
        }
        image::Rgba(out)
    })
}
