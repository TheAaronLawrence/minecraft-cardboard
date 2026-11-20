//! Player-head skins, fetched the way vanilla's `ProfileResolver` and skin manager do: a
//! name becomes a UUID through Mojang's profile API, the session server gives the profile's
//! signed `textures` property, and that names the skin's URL. Downloads are cached on disk.
//! A head whose skin can't be found keeps vanilla's default skin for its UUID.
//!
//! Other players' skins come with their profiles in the tab list, as `SkinManager` takes
//! them: only a `textures` property Mojang signed counts (`PlayerInfo` requires a secure
//! skin for anyone but the local player), checked against Mojang's profile property keys.
//! An offline-mode server sends no textures, so there everyone keeps their default skin.

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
    match textures {
        Some(textures) => Ok(download(&textures)?.map(|(img, _)| img)),
        None => Ok(None),
    }
}

/// The skin a `textures` property names, processed as vanilla processes downloaded skins,
/// and whether it's for the slim (three-pixel-arm) model.
fn download(textures: &str) -> Result<Option<(image::RgbaImage, bool)>> {
    let json = base64::engine::general_purpose::STANDARD.decode(textures.trim())?;
    let json: serde_json::Value = serde_json::from_slice(&json)?;
    let skin = &json["textures"]["SKIN"];
    let Some(url) = skin["url"].as_str() else {
        return Ok(None);
    };
    let slim = skin["metadata"]["model"] == "slim";
    // Mojang only serves skins from textures.minecraft.net.
    anyhow::ensure!(
        url.starts_with("http://textures.minecraft.net/")
            || url.starts_with("https://textures.minecraft.net/"),
        "skin URL {url} isn't on textures.minecraft.net"
    );
    let bytes = cached(url)?;
    let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?.to_rgba8();
    anyhow::ensure!(
        img.width() == 64 && (img.height() == 32 || img.height() == 64),
        "skin {url} is {}×{}, not 64×64 or 64×32",
        img.width(),
        img.height()
    );
    Ok(Some((process_legacy_skin(img), slim)))
}

/// Downloads other players' skins in the background.
pub struct Loader {
    requests: std::sync::mpsc::Sender<(u128, mc_protocol::Textures)>,
    done: std::sync::mpsc::Receiver<(u128, image::RgbaImage, bool)>,
}

impl Loader {
    pub fn new() -> Self {
        let (requests, rx) = std::sync::mpsc::channel::<(u128, mc_protocol::Textures)>();
        let (tx, done) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("skins".into())
            .spawn(move || {
                let mut keys: Option<Vec<Vec<u8>>> = None;
                for (uuid, textures) in rx {
                    let result = (|| {
                        let keys = match &mut keys {
                            Some(k) => k,
                            None => keys.insert(profile_property_keys()?),
                        };
                        if !signed(&textures, keys) {
                            anyhow::bail!("its textures aren't signed by Mojang");
                        }
                        download(&textures.value)
                    })();
                    match result {
                        Ok(Some((img, slim))) => {
                            if tx.send((uuid, img, slim)).is_err() {
                                return;
                            }
                        }
                        Ok(None) => {}
                        Err(e) => eprintln!("skin for {uuid:032x}: {e:#}; using the default skin"),
                    }
                }
            })
            .expect("spawning the skin thread");
        Self { requests, done }
    }

    /// Starts fetching a player's skin.
    pub fn request(&self, uuid: u128, textures: mc_protocol::Textures) {
        let _ = self.requests.send((uuid, textures));
    }

    /// Skins that have arrived: the player's UUID, the skin, and whether it's slim.
    pub fn arrived(&self) -> impl Iterator<Item = (u128, image::RgbaImage, bool)> + '_ {
        self.done.try_iter()
    }
}

/// `MinecraftServicesKeyInfo.validateProperty` with any of the keys: Mojang's SHA-1 RSA
/// signature of the property's value. Keys are `RSAPublicKey` DER.
fn signed(textures: &mc_protocol::Textures, keys: &[Vec<u8>]) -> bool {
    use ring::signature::{RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY, UnparsedPublicKey};
    let Some(signature) = textures.signature.as_deref().and_then(|s| {
        base64::engine::general_purpose::STANDARD
            .decode(s.trim())
            .ok()
    }) else {
        return false;
    };
    keys.iter().any(|key| {
        UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY, key)
            .verify(textures.value.as_bytes(), &signature)
            .is_ok()
    })
}

/// Mojang's keys for profile properties (`api.minecraftservices.com/publickeys`'s
/// `profilePropertyKeys`), remembered for a day, as `RSAPublicKey` DER.
fn profile_property_keys() -> Result<Vec<Vec<u8>>> {
    let path = cache_dir().join("publickeys.json");
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() < 24 * 60 * 60);
    let json: serde_json::Value = match std::fs::read(&path) {
        Ok(bytes) if fresh => serde_json::from_slice(&bytes)?,
        _ => {
            let json = get_json("https://api.minecraftservices.com/publickeys")?
                .context("no public keys")?;
            std::fs::create_dir_all(cache_dir())?;
            std::fs::write(&path, json.to_string())?;
            json
        }
    };
    keys_from(&json)
}

fn keys_from(json: &serde_json::Value) -> Result<Vec<Vec<u8>>> {
    json["profilePropertyKeys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|k| k["publicKey"].as_str())
        .map(|k| {
            let spki = base64::engine::general_purpose::STANDARD.decode(k)?;
            rsa_public_key(&spki).context("malformed public key")
        })
        .collect()
}

/// The `RSAPublicKey` inside an X.509 `SubjectPublicKeyInfo`: a sequence of the algorithm
/// and a bit string holding it.
fn rsa_public_key(spki: &[u8]) -> Option<Vec<u8>> {
    /// One DER element: its tag, its contents, and what follows it.
    fn element(der: &[u8]) -> Option<(u8, &[u8], &[u8])> {
        let (&tag, rest) = der.split_first()?;
        let (&first, rest) = rest.split_first()?;
        let (len, rest) = if first < 0x80 {
            (first as usize, rest)
        } else {
            let n = (first & 0x7f) as usize;
            let (bytes, rest) = rest.split_at_checked(n)?;
            (bytes.iter().fold(0, |l, &b| l << 8 | b as usize), rest)
        };
        let (content, rest) = rest.split_at_checked(len)?;
        Some((tag, content, rest))
    }
    let (0x30, info, _) = element(spki)? else {
        return None;
    };
    let (0x30, _, rest) = element(info)? else {
        return None;
    };
    let (0x03, bits, _) = element(rest)? else {
        return None;
    };
    // The bit string's first byte counts unused bits, which a key has none of.
    match bits.split_first()? {
        (0, key) => Some(key.to_vec()),
        _ => None,
    }
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

/// `SkinTextureDownloader.processLegacySkin`: a pre-1.8 64×32 skin is padded to 64×64 and
/// its right arm and leg are mirrored into the left limbs' places; its hat layer is cleared
/// if it's fully opaque, since old skins filled it with a solid colour
/// (`doNotchTransparencyHack`); and the head, body and limbs' base layers are made opaque.
fn process_legacy_skin(img: image::RgbaImage) -> image::RgbaImage {
    if img.width() != 64 || !(img.height() == 32 || img.height() == 64) {
        return img;
    }
    let legacy = img.height() == 32;
    let mut out = image::RgbaImage::new(64, 64);
    image::imageops::replace(&mut out, &img, 0, 0);
    if legacy {
        // `NativeImage.copyRect(x, y, dx, dy, w, h, mirrorX, false)`.
        let copies = [
            (4, 16, 16, 32, 4, 4),
            (8, 16, 16, 32, 4, 4),
            (0, 20, 24, 32, 4, 12),
            (4, 20, 16, 32, 4, 12),
            (8, 20, 8, 32, 4, 12),
            (12, 20, 16, 32, 4, 12),
            (44, 16, -8, 32, 4, 4),
            (48, 16, -8, 32, 4, 4),
            (40, 20, 0, 32, 4, 12),
            (44, 20, -8, 32, 4, 12),
            (48, 20, -16, 32, 4, 12),
            (52, 20, -8, 32, 4, 12),
        ];
        for (x, y, dx, dy, w, h) in copies {
            for row in 0..h {
                for col in 0..w {
                    let p = *out.get_pixel((x + col) as u32, (y + row) as u32);
                    let to = (x + dx + w - 1 - col, y + dy + row);
                    out.put_pixel(to.0 as u32, to.1 as u32, p);
                }
            }
        }
    }
    let pixels = |x0, y0, x1, y1| (y0..y1).flat_map(move |y| (x0..x1).map(move |x| (x, y)));
    let opaque = |out: &mut image::RgbaImage, x0, y0, x1, y1| {
        for (x, y) in pixels(x0, y0, x1, y1) {
            out.get_pixel_mut(x, y)[3] = 255;
        }
    };
    opaque(&mut out, 0, 0, 32, 16);
    if legacy && pixels(32, 0, 64, 32).all(|(x, y)| out.get_pixel(x, y)[3] >= 128) {
        for (x, y) in pixels(32, 0, 64, 32) {
            out.get_pixel_mut(x, y)[3] = 0;
        }
    }
    opaque(&mut out, 0, 16, 64, 32);
    opaque(&mut out, 16, 48, 48, 64);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Notch's textures as the session server signed them verify against the keys Mojang
    /// published then; changing a byte of the value, or leaving the signature off, fails.
    #[test]
    fn mojang_signatures_verify() {
        let fixture: serde_json::Value = serde_json::from_slice(
            &std::fs::read(mc_assets::locate("fixtures/skins/notch_textures.json")).unwrap(),
        )
        .unwrap();
        let keys = keys_from(&fixture).unwrap();
        let textures = mc_protocol::Textures {
            value: fixture["value"].as_str().unwrap().into(),
            signature: Some(fixture["signature"].as_str().unwrap().into()),
        };
        assert!(signed(&textures, &keys));
        let mut tampered = textures.clone();
        tampered.value.replace_range(10..11, "A");
        assert!(!signed(&tampered, &keys));
        let unsigned = mc_protocol::Textures {
            signature: None,
            ..textures
        };
        assert!(!signed(&unsigned, &keys));
    }

    /// A legacy 64×32 skin's right arm becomes the left arm mirrored: the arm's front
    /// (x 44..48, y 20..32) lands on the left arm's front (x 36..40, y 52..64) flipped.
    #[test]
    fn legacy_skins_get_left_limbs() {
        let mut img = image::RgbaImage::new(64, 32);
        img.put_pixel(44, 20, image::Rgba([10, 20, 30, 255]));
        let out = process_legacy_skin(img);
        assert_eq!(out.dimensions(), (64, 64));
        assert_eq!(*out.get_pixel(39, 52), image::Rgba([10, 20, 30, 255]));
    }
}
