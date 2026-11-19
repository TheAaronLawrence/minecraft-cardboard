//! `cargo xtask package-windows`: cross-compiles the client for 64-bit Windows and zips it
//! with the files it reads at run time, into `target/package/`.

use anyhow::{Context, Result, ensure};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const TARGET: &str = "x86_64-pc-windows-gnu";
const LINKER: &str = "x86_64-w64-mingw32-gcc";
const EXE: &str = "minecraft-cardboard.exe";

/// What the client reads beside its exe (see `mc_assets::locate`). Sounds and language
/// files aren't used yet, so they stay out and the zip stays small.
const FILES: &[&str] = &[
    "resources/assets/minecraft/blockstates",
    "resources/assets/minecraft/models",
    "resources/assets/minecraft/textures",
    "resources/data",
    "resources/reports",
    "fixtures/worlds/test-world",
];

pub fn package_windows() -> Result<()> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let version = env!("CARGO_PKG_VERSION");
    let name = format!("minecraft-cardboard-{version}-windows-x64");

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
        .args(["build", "--release", "-p", "mc-client", "--target", TARGET]);
    if std::env::var_os(&linker_var).is_none() {
        build.env(&linker_var, LINKER);
    }
    run(&mut build)?;

    let out = repo.join("target/package");
    let dir = out.join(&name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::copy(
        repo.join("target").join(TARGET).join("release").join(EXE),
        dir.join(EXE),
    )?;
    for path in FILES {
        copy_dir(&repo.join(path), &dir.join(path))?;
    }
    let readme = include_str!("../windows-README.txt")
        .replace("{version}", version)
        .replace("{commit}", &commit(&repo)?)
        .replace('\n', "\r\n");
    std::fs::write(dir.join("README.txt"), readme)?;

    let zip_path = out.join(format!("{name}.zip"));
    zip_dir(&out, &name, &zip_path)?;
    let size = std::fs::metadata(&zip_path)?.len();
    println!(
        "{}: {:.1} MB",
        zip_path.display(),
        size as f64 / 1_000_000.0
    );
    Ok(())
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
