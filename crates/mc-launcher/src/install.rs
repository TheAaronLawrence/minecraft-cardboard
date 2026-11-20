//! Installs the newest release into the game folder and keeps it up to date, with
//! Minecraft's own files from Mojang (see `mojang`). Of the release, just the exe-only zip unless the release's `assets.txt` says the
//! resources changed, then the full zip once.

use crate::github::{self, Release};
use anyhow::{Context, Result};
use std::io::Read;
use std::path::Path;

/// The release zips, by the ends of their names.
const FULL: &str = "-windows-x64.zip";
const EXE_ONLY: &str = "-windows-x64-exe-only.zip";

/// What the launcher shows when it can't put the game's files in place, such as with the
/// disk full; the log has which file.
pub(crate) const WRITE_FAILED: &str = "Couldn't install the game's files";

/// The game's executable in `dir`.
pub fn game_exe(dir: &Path) -> std::path::PathBuf {
    dir.join(format!(
        "minecraft-cardboard{}",
        std::env::consts::EXE_SUFFIX
    ))
}

/// The release installed in `dir`, if it's installed with Mojang's files.
pub fn installed(dir: &Path) -> Option<String> {
    let tag = read_text(&dir.join("version.txt"));
    (!tag.is_empty() && game_exe(dir).exists() && crate::mojang::installed(dir)).then_some(tag)
}

pub(crate) fn read_text(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// What an update is doing, for the launcher to show.
pub enum Step<'a> {
    Checking,
    /// Downloading `what`, `done` of `total` bytes.
    Downloading {
        what: &'a str,
        done: u64,
        total: u64,
    },
    Installing,
}

/// Brings `dir` up to the newest release, with the Mojang files it needs, and returns its
/// tag.
pub fn update(dir: &Path, step: &mut dyn FnMut(Step)) -> Result<String> {
    step(Step::Checking);
    let release = github::latest_release()?;
    if read_text(&dir.join("version.txt")) != release.tag || !game_exe(dir).exists() {
        install_release(dir, &release, step)?;
    }
    if !crate::mojang::installed(dir) {
        crate::mojang::install(dir, step)?;
    }
    Ok(release.tag)
}

fn install_release(dir: &Path, release: &Release, step: &mut dyn FnMut(Step)) -> Result<()> {
    let assets = github::download(release.asset("assets.txt")?, |_| {})?;
    let assets = String::from_utf8_lossy(&assets).trim().to_owned();
    let full = assets != read_text(&dir.join("assets.txt"));
    let zip = download(release, if full { FULL } else { EXE_ONLY }, step)?;
    step(Step::Installing);
    std::fs::create_dir_all(dir).context(WRITE_FAILED)?;
    // Without these, an install cut short is redone, in full, next time.
    for file in ["version.txt", "assets.txt"] {
        let _ = std::fs::remove_file(dir.join(file));
    }
    if full {
        // Replaced whole, so files a release dropped don't linger. Mojang's files in
        // `resources` stay.
        for folder in ["resources/reports", "fixtures"] {
            let path = dir.join(folder);
            if path.exists() {
                std::fs::remove_dir_all(&path)
                    .with_context(|| format!("removing {}", path.display()))
                    .context(WRITE_FAILED)?;
            }
        }
    }
    unzip(&zip, dir).context(WRITE_FAILED)?;
    std::fs::write(dir.join("assets.txt"), &assets).context(WRITE_FAILED)?;
    std::fs::write(dir.join("version.txt"), &release.tag).context(WRITE_FAILED)?;
    Ok(())
}

fn download(release: &Release, suffix: &str, step: &mut dyn FnMut(Step)) -> Result<Vec<u8>> {
    let asset = release.asset(suffix)?;
    let what = if suffix == FULL {
        release.tag.clone()
    } else {
        format!("the {} update", release.tag)
    };
    github::download(asset, |done| {
        step(Step::Downloading {
            what: &what,
            done,
            total: asset.size,
        })
    })
}

/// Unpacks a release zip into `dir`, without its top folder.
fn unzip(zip: &[u8], dir: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        // `enclosed_name` refuses paths that would climb out of the folder.
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let relative: std::path::PathBuf = path.iter().skip(1).collect();
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = dir.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes)?;
        std::fs::write(&target, bytes).with_context(|| format!("writing {}", target.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    /// A release zip unpacks without its top folder, and nothing lands outside the game
    /// folder.
    #[test]
    fn unzip_strips_the_top_folder() {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, body) in [
            ("top/game.exe", "exe"),
            ("top/resources/a/b.json", "{}"),
            ("../escaped.txt", "no"),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let root = std::env::temp_dir().join(format!("cardboard-unzip-{}", std::process::id()));
        let dir = root.join("game");
        super::unzip(&bytes, &dir).unwrap();
        let read = |p: &str| std::fs::read_to_string(dir.join(p)).ok();
        assert_eq!(read("game.exe").as_deref(), Some("exe"));
        assert_eq!(read("resources/a/b.json").as_deref(), Some("{}"));
        assert!(!dir.join("top").exists());
        assert!(!root.join("escaped.txt").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
