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
    /// RGBA, row by row.
    pub pixels: Vec<u8>,
    sprites: HashMap<String, Sprite>,
}

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
    /// Loads and packs the sprites. A sprite without a texture file, and the built-in
    /// `missingno`, become the purple and black checkerboard.
    pub fn build(res: &Resources, ids: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut images: Vec<(String, image::RgbaImage)> = Vec::new();
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

        // Shelf packing, tallest first. Block sprites are mostly 16×16.
        images.sort_by_key(|(_, img)| std::cmp::Reverse((img.height(), img.width())));
        const WIDTH: u32 = 2048;
        let (mut x, mut y, mut shelf) = (0, 0, 0);
        let mut placed = Vec::with_capacity(images.len());
        for (id, img) in images {
            if x + img.width() > WIDTH {
                (x, y, shelf) = (0, y + shelf, 0);
            }
            placed.push((id, img, x, y));
            shelf = shelf.max(placed.last().unwrap().1.height());
            x += placed.last().unwrap().1.width();
        }
        let height = (y + shelf).next_power_of_two();

        let mut pixels = vec![0u8; (WIDTH * height * 4) as usize];
        let mut sprites = HashMap::new();
        for (id, img, x, y) in placed {
            for row in 0..img.height() {
                let src = &img.as_raw()
                    [(row * img.width() * 4) as usize..((row + 1) * img.width() * 4) as usize];
                let start = (((y + row) * WIDTH + x) * 4) as usize;
                pixels[start..start + src.len()].copy_from_slice(src);
            }
            let alphas = img.pixels().map(|p| p[3]);
            let layer = alphas.fold(Layer::Opaque, |layer, a| match a {
                255 => layer,
                0 => layer.max(Layer::Cutout),
                _ => Layer::Translucent,
            });
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
            pixels,
            sprites,
        })
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
