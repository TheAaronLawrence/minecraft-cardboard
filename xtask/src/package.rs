//! `cargo xtask package-windows [TAG]`: cross-compiles the client for 64-bit Windows and
//! writes a release's files to `target/package/`:
//!
//! - `minecraft-cardboard-VERSION-windows-x64.zip`: the game and its launcher, the files they read
//!   at run time and the updater (`update.cmd`, `update.ps1`).
//! - `...-windows-x64-exe-only.zip`: the exe and updater, for updates that keep the resources.
//! - `assets.txt`: a fingerprint of the resources, which tells the updater which zip it needs.
//!
//! TAG is the GitHub release tag the files are for, written to `version.txt` so the updater
//! knows what's installed. Publish them with
//! `gh release create TAG target/package/*.zip target/package/assets.txt --prerelease`.

use anyhow::{Context, Result, ensure};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const TARGET: &str = "x86_64-pc-windows-gnu";
const LINKER: &str = "x86_64-w64-mingw32-gcc";
const EXE: &str = "minecraft-cardboard.exe";
const LAUNCHER: &str = "minecraft-cardboard-launcher.exe";

/// What the client reads beside its exe (see `mc_assets::locate`), as folders or files.
/// Sounds and other languages aren't used yet, so they stay out and the zip stays small.
const FILES: &[&str] = &[
    "resources/assets/minecraft/blockstates",
    "resources/assets/minecraft/font/include/default.json",
    "resources/assets/minecraft/items",
    "resources/assets/minecraft/lang/en_us.json",
    "resources/assets/minecraft/models",
    "resources/assets/minecraft/textures",
    "resources/data",
    "resources/reports",
    "fixtures/worlds/test-world",
];

pub fn package_windows(tag: Option<&str>) -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let version = env!("CARGO_PKG_VERSION");
    let name = format!("minecraft-cardboard-{version}-windows-x64");
    let exe_only = format!("{name}-exe-only");

    ensure!(
        Command::new(LINKER).arg("--version").output().is_ok(),
        "{LINKER} not found; install MinGW-w64 (Debian/Ubuntu: apt install gcc-mingw-w64-x86-64)"
    );
    run(Command::new("rustup").args(["target", "add", TARGET]))?;
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let linker_var = format!(
        "CARGO_TARGET_{}_LINKER",
        TARGET.to_uppercase().replace('-', "_")
    );
    let mut build = Command::new(cargo);
    build
        .current_dir(&repo)
        .args(["build", "--release", "-p", "mc-client", "-p", "mc-launcher"])
        .args(["--target", TARGET]);
    if std::env::var_os(&linker_var).is_none() {
        build.env(&linker_var, LINKER);
    }
    run(&mut build)?;

    let out = repo.join("target/package");
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    let readme = include_str!("../windows/README.txt")
        .replace("{version}", version)
        .replace("{commit}", &commit(&repo)?)
        .replace('\n', "\r\n");
    let assets = assets_fingerprint(&repo)?;
    // What both zips hold: the game and the launcher, the updater, and what's installed.
    let common = |dir: &Path| -> Result<()> {
        std::fs::create_dir_all(dir)?;
        for exe in [EXE, LAUNCHER] {
            std::fs::copy(
                repo.join("target").join(TARGET).join("release").join(exe),
                dir.join(exe),
            )?;
        }
        std::fs::write(dir.join("README.txt"), &readme)?;
        std::fs::write(
            dir.join("update.ps1"),
            include_str!("../windows/update.ps1").replace('\n', "\r\n"),
        )?;
        std::fs::write(
            dir.join("update.cmd"),
            include_str!("../windows/update.cmd"),
        )?;
        if let Some(tag) = tag {
            std::fs::write(dir.join("version.txt"), tag)?;
        }
        Ok(())
    };

    let dir = out.join(&name);
    common(&dir)?;
    for path in FILES {
        copy_dir(&repo.join(path), &dir.join(path))?;
    }
    std::fs::write(dir.join("assets.txt"), &assets)?;
    common(&out.join(&exe_only))?;
    std::fs::write(out.join("assets.txt"), &assets)?;

    for folder in [&name, &exe_only] {
        let zip_path = out.join(format!("{folder}.zip"));
        zip_dir(&out, folder, &zip_path)?;
        let size = std::fs::metadata(&zip_path)?.len();
        println!(
            "{}: {:.1} MB",
            zip_path.display(),
            size as f64 / 1_000_000.0
        );
    }
    if tag.is_none() {
        println!("no tag given, so the zips have no version.txt and the updater will update them");
    }
    Ok(())
}

/// A hash of every file in `FILES`, names and contents, so it changes exactly when a
/// release's resources do.
fn assets_fingerprint(repo: &Path) -> Result<String> {
    let mut files = Vec::new();
    for path in FILES {
        collect(&repo.join(path), &mut files)?;
    }
    files.sort();
    let mut hash = sha1_smol::Sha1::new();
    for file in files {
        let relative = file
            .strip_prefix(repo)?
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = std::fs::read(&file)?;
        hash.update(relative.as_bytes());
        hash.update(&[0]);
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    Ok(hash.digest().to_string())
}

fn run(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("running {command:?}"))?;
    ensure!(status.success(), "{command:?} failed: {status}");
    Ok(())
}

/// The short commit hash, marked `-dirty` when the worktree has uncommitted changes, so
/// a zip says exactly what it was built from.
fn commit(repo: &Path) -> Result<String> {
    let git = |args: &[&str]| -> Result<String> {
        let out = Command::new("git").current_dir(repo).args(args).output()?;
        ensure!(out.status.success(), "git {args:?} failed");
        Ok(String::from_utf8(out.stdout)?.trim().to_owned())
    };
    let hash = git(&["rev-parse", "--short", "HEAD"])?;
    let dirty = !git(&["status", "--porcelain"])?.is_empty();
    Ok(if dirty { format!("{hash}-dirty") } else { hash })
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    if from.is_file() {
        std::fs::create_dir_all(to.parent().unwrap())?;
        std::fs::copy(from, to)?;
        return Ok(());
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from).with_context(|| format!("{}", from.display()))? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Zips `base/name` with `name/` as the top folder. Entries are sorted and carry a fixed
/// timestamp, so the same inputs make the same zip.
fn zip_dir(base: &Path, name: &str, zip_path: &Path) -> Result<()> {
    let mut files = Vec::new();
    collect(&base.join(name), &mut files)?;
    files.sort();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(zip_path)?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(9))
        .last_modified_time(zip::DateTime::default());
    for file in files {
        let relative = file.strip_prefix(base)?;
        // Zip paths use forward slashes on every platform.
        let entry = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        zip.start_file(entry, options)?;
        zip.write_all(&std::fs::read(&file)?)?;
    }
    zip.finish()?;
    Ok(())
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if dir.is_file() {
        files.push(dir.to_owned());
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}
