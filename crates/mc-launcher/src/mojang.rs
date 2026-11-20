//! Minecraft's own files, which releases don't carry: the launcher downloads the client jar
//! of the version the game matches from Mojang, as Mojang's launcher does, and unpacks the
//! textures, models and data the game reads into `resources/`. Mojang's code stays in the
//! jar, which isn't kept.

use crate::install::{Step, WRITE_FAILED, read_text};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::io::Read;
use std::path::Path;

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// What the game reads from the jar, as paths or folders (ending in `/`) in it.
const WANTED: &[&str] = &[
    "assets/minecraft/blockstates/",
    "assets/minecraft/font/include/default.json",
    "assets/minecraft/items/",
    "assets/minecraft/lang/en_us.json",
    "assets/minecraft/models/",
    "assets/minecraft/textures/",
    "data/",
];

/// Written once the jar's files are unpacked: the jar's SHA-1.
const MARKER: &str = "mojang.txt";

/// The client jar the release in `dir` was built against, from its `resources/source.json`:
/// the version and the jar's SHA-1.
fn source(dir: &Path) -> Result<(String, String)> {
    let path = dir.join("resources/source.json");
    let json: Value = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    let text = |key: &str| -> Result<String> {
        Ok(json[key]
            .as_str()
            .with_context(|| format!("{} has no {key}", path.display()))?
            .to_owned())
    };
    Ok((text("version")?, text("client_jar_sha1")?))
}

/// Mojang's files for the release in `dir` are unpacked.
pub fn installed(dir: &Path) -> bool {
    source(dir).is_ok_and(|(_, sha1)| read_text(&dir.join(MARKER)) == sha1)
}

fn get_json(url: &str) -> Result<Value> {
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))
        .context("Couldn't reach Mojang; check your internet connection")?;
    Ok(response
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_json()?)
}

fn str_at<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("Mojang's version list has no {key}"))
}

/// Downloads the client jar and unpacks what the game reads into `dir/resources`.
pub fn install(dir: &Path, step: &mut dyn FnMut(Step)) -> Result<()> {
    let (version, sha1) = source(dir)?;
    let manifest = get_json(MANIFEST_URL)?;
    let entry = manifest["versions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["id"] == version.as_str())
        .with_context(|| format!("Mojang has no Minecraft {version}"))?;
    let client = get_json(str_at(entry, "url")?)?["downloads"]["client"].clone();
    ensure!(
        str_at(&client, "sha1")? == sha1,
        "Mojang's Minecraft {version} isn't the one this release was built for"
    );
    let total = client["size"].as_u64().unwrap_or(0);
    let what = format!("Minecraft {version} from Mojang");
    let url = str_at(&client, "url")?;
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))
        .context("Couldn't reach Mojang; check your internet connection")?;
    let jar = crate::github::read_body(&mut response, &what, |done| {
        step(Step::Downloading {
            what: &what,
            done,
            total,
        })
    })?;
    let actual = sha1_smol::Sha1::from(&jar).digest().to_string();
    ensure!(
        actual == sha1,
        "The Minecraft {version} download was damaged; try again"
    );

    step(Step::Installing);
    let _ = std::fs::remove_file(dir.join(MARKER));
    let resources = dir.join("resources");
    // Replaced whole, so nothing from another version lingers.
    for folder in ["assets", "data"] {
        let path = resources.join(folder);
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .with_context(|| format!("removing {}", path.display()))
                .context(WRITE_FAILED)?;
        }
    }
    unpack(&jar, &resources).context(WRITE_FAILED)?;
    std::fs::write(dir.join(MARKER), &sha1).context(WRITE_FAILED)?;
    Ok(())
}

/// Unpacks the `WANTED` files of the jar into `resources`.
fn unpack(jar: &[u8], resources: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(jar))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() || !wanted(entry.name()) {
            continue;
        }
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let target = resources.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes)?;
        std::fs::write(&target, bytes).with_context(|| format!("writing {}", target.display()))?;
    }
    Ok(())
}

fn wanted(name: &str) -> bool {
    WANTED.iter().any(|w| match w.strip_suffix('/') {
        Some(_) => name.starts_with(w),
        None => name == *w,
    })
}
