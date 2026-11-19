//! Draws a world: block models baked into quads, meshed per chunk section with vanilla-style
//! lighting, and rendered with wgpu.

pub mod atlas;
pub mod bake;
pub mod chest;
pub mod entity_model;
pub mod gpu;
pub mod hud;
pub mod item;
pub mod mesh;
pub mod mipmap;
pub mod tint;
