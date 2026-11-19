//! Player-head skins, fetched the way vanilla's `ProfileResolver` and skin manager do: a
//! name becomes a UUID through Mojang's profile API, the session server gives the profile's
//! signed `textures` property, and that names the skin's URL. Downloads are cached on disk.
//! A head whose skin can't be found keeps vanilla's default skin for its UUID.

use anyhow::{Context, Result};
use base64::Engine;
use mc_render::entity_model::{Profile, skin_sprite};
use std::path::PathBuf;

/// Each distinct profile's skin, keyed by the atlas id `skin_sprite` gives it.
pub fn fetch(profiles: impl IntoIterator<Item = Profile>) -> Vec<(String, image::RgbaImage)> {
    let mut out: Vec<(String, image::RgbaImage)> = Vec::new();
    for profile in profiles {
        let id = skin_sprite(&profile);
        if out.iter().any(|(s, _)| *s == id) {
            continue;
        }
        match skin(&profile) {
            Ok(Some(img)) => out.push((id, img)),
            Ok(None) => {}
            Err(e) => eprintln!("skin for {id}: {e:#}; using the default skin"),
        }
    }
    out
}

fn skin(profile: &Profile) -> Result<Option<image::RgbaImage>> {
    let textures = match &profile.textures {
        Some(t) => Some(t.clone()),
        None => resolved(&skin_sprite(profile), || {
            let id = match (profile.id, &profile.name) {
                (Some(id), _) => Some(format!("{id:032x}")),
                (None, Some(name)) => uuid_of(name)?,
                (None, None) => None,
            };
            match id {
                Some(id) => textures_of(&id),
                None => Ok(None),
            }
        })?,
    };
    let Some(textures) = textures else {
        return Ok(None);
    };
    let json = base64::engine::general_purpose::STANDARD.decode(textures.trim())?;
    let json: serde_json::Value = serde_json::from_slice(&json)?;
    let Some(url) = json["textures"]["SKIN"]["url"].as_str() else {
        return Ok(None);
    };
    // Mojang only serves skins from textures.minecraft.net.
    anyhow::ensure!(
        url.starts_with("http://textures.minecraft.net/")
            || url.starts_with("https://textures.minecraft.net/"),
        "skin URL {url} isn't on textures.minecraft.net"
    );
    let bytes = cached(url)?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?.to_rgba8();
    Ok(Some(process_legacy_skin(img)))
}

/// A profile's `textures` property, remembered for a day so repeated runs don't hit
/// Mojang's rate limits; an empty file remembers that the player has no skin.
fn resolved(
    sprite: &str,
    lookup: impl FnOnce() -> Result<Option<String>>,
) -> Result<Option<String>> {
    let key = sprite.rsplit('/').next().unwrap_or_default();
    let path = cache_dir().join(format!("{key}.textures"));
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < 24 * 60 * 60);
    if fresh && let Ok(text) = std::fs::read_to_string(&path) {
        return Ok((!text.is_empty()).then_some(text));
    }
    let textures = lookup()?;
    std::fs::create_dir_all(cache_dir())?;
    std::fs::write(&path, textures.as_deref().unwrap_or_default())?;
    Ok(textures)
}

/// `SkinTextureDownloader.processLegacySkin`, for the head: the head's base layer is made
/// opaque, and a pre-1.8 64×32 skin (padded to 64×64 here) has its hat layer cleared if it's
/// fully opaque, since old skins filled it with a solid colour (`doNotchTransparencyHack`).
fn process_legacy_skin(img: image::RgbaImage) -> image::RgbaImage {
    if img.width() != 64 || !(img.height() == 32 || img.height() == 64) {
        return img;
    }
    let legacy = img.height() == 32;
    let mut out = image::RgbaImage::new(64, 64);
    image::imageops::replace(&mut out, &img, 0, 0);
    let pixels = |x0, y0, x1, y1| (y0..y1).flat_map(move |y| (x0..x1).map(move |x| (x, y)));
    for (x, y) in pixels(0, 0, 32, 16) {
        out.get_pixel_mut(x, y)[3] = 255;
    }
    if legacy && pixels(32, 0, 64, 32).all(|(x, y)| out.get_pixel(x, y)[3] >= 128) {
        for (x, y) in pixels(32, 0, 64, 32) {
            out.get_pixel_mut(x, y)[3] = 0;
        }
    }
    out
}

/// A player name's UUID, or `None` if no player has that name.
fn uuid_of(name: &str) -> Result<Option<String>> {
    let url = format!("https://api.mojang.com/users/profiles/minecraft/{name}");
    let Some(body) = get_json(&url)? else {
        return Ok(None);
    };
    Ok(body["id"].as_str().map(str::to_owned))
}

/// A profile's `textures` property from the session server.
fn textures_of(uuid: &str) -> Result<Option<String>> {
    let url = format!("https://sessionserver.mojang.com/session/minecraft/profile/{uuid}");
    let Some(body) = get_json(&url)? else {
        return Ok(None);
    };
    Ok(body["properties"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["name"] == "textures")
        .and_then(|p| p["value"].as_str())
        .map(str::to_owned))
}

/// `None` for 204 and 404, which is how Mojang says there's no such player.
fn get_json(url: &str) -> Result<Option<serde_json::Value>> {
    let mut response = ureq::get(url)
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .with_context(|| format!("GET {url}"))?;
    match response.status().as_u16() {
        200 => Ok(Some(response.body_mut().read_json()?)),
        204 | 404 => Ok(None),
        status => anyhow::bail!("GET {url}: HTTP {status}"),
    }
}

/// A skin's bytes, from the cache or downloaded into it. Skin URLs end in their content
/// hash, so a cached file never goes stale.
fn cached(url: &str) -> Result<Vec<u8>> {
    let hash = url.rsplit('/').next().unwrap_or_default();
    anyhow::ensure!(
        !hash.is_empty() && hash.chars().all(|c| c.is_ascii_alphanumeric()),
        "unexpected skin URL {url}"
    );
    let path = cache_dir().join(format!("{hash}.png"));
    if let Ok(bytes) = std::fs::read(&path) {
        return Ok(bytes);
    }
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    let bytes = response.body_mut().read_to_vec()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, &bytes)?;
    Ok(bytes)
}

fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("minecraft-cardboard/skins")
}
