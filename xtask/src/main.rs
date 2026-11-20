//! Project tasks. `cargo xtask fetch-assets [version]` copies Mojang's assets for a
//! vanilla Java Edition version into `resources/`. They aren't committed: run it once after
//! cloning.
//! `cargo xtask package-windows [tag]` builds a Windows release (see `package.rs`).
//! `cargo xtask compare [views]` renders views in vanilla and in our client and measures the
//! difference (see `compare.rs`).

mod compare;
mod package;

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The vanilla version our data, protocol and saves match.
const DEFAULT_VERSION: &str = "26.3";
const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const OBJECTS_URL: &str = "https://resources.download.minecraft.net";
const DOWNLOAD_THREADS: usize = 16;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("fetch-assets") => fetch_assets(args.get(1).map_or(DEFAULT_VERSION, String::as_str)),
        Some("gen-reports") => gen_reports(args.get(1).map_or(DEFAULT_VERSION, String::as_str)),
        Some("gen-world") => gen_world(args.get(1).map_or(DEFAULT_VERSION, String::as_str)),
        Some("package-windows") => package::package_windows(args.get(1).map(String::as_str)),
        Some("compare") => compare::compare(args.get(1).map(String::as_str)),
        _ => bail!(
            "usage: cargo xtask fetch-assets|gen-reports|gen-world [version], \
             cargo xtask package-windows [tag], or cargo xtask compare [views]"
        ),
    }
}

fn resources_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../resources")
}

/// The launcher's description of a version: its downloads, asset index and Java version.
fn version_info(version: &str) -> Result<Value> {
    let manifest: Value = serde_json::from_slice(&get(MANIFEST_URL)?)?;
    let entry = manifest["versions"]
        .as_array()
        .context("manifest has no versions")?
        .iter()
        .find(|v| v["id"] == version)
        .with_context(|| format!("version {version} not in the manifest"))?;
    Ok(serde_json::from_slice(&get_verified(
        str_at(entry, "url")?,
        str_at(entry, "sha1")?,
    )?)?)
}

/// Runs vanilla's data generator on the server jar and copies the reports we use into
/// `resources/reports/`: every block's properties and state ids, every registry's protocol
/// ids, and every packet's id in each protocol state. Vanilla defines these in code, so
/// they aren't in the data packs. `items.json` condenses the items' default components to
/// what the inventory needs (see `item_report`). `source.json` records the protocol version. It also
/// runs `xtask/java/DumpPhysics.java` on the unpacked server for `physics.json`: every state's
/// collision shape and the blocks' friction, speed and jump factors. Needs a JDK's `java`
/// (the version's Java, 25 for 26.3) on PATH, or set JAVA to its path.
fn gen_reports(version: &str) -> Result<()> {
    const REPORTS: [&str; 3] = ["blocks.json", "registries.json", "packets.json"];
    let (work, server_sha1) = server_workdir(version, "gen-reports")?;
    let status = java()
        .current_dir(&work)
        .args([
            "-DbundlerMainClass=net.minecraft.data.Main",
            "-jar",
            "server.jar",
            "--reports",
            "--output",
            "out",
        ])
        .status()
        .context("running java; install Java or set JAVA")?;
    ensure!(status.success(), "the data generator failed: {status}");

    let reports = resources_dir().join("reports");
    for name in REPORTS {
        write(
            &reports.join(name),
            &std::fs::read(work.join("out/reports").join(name))?,
        )?;
    }
    write(
        &reports.join("items.json"),
        item_report(&work.join("out/reports/minecraft/components/item"))?.as_bytes(),
    )?;
    // The data generator unpacked the server and its libraries; run the dumper on them.
    let mut classpath = Vec::new();
    collect_jars(&work.join("versions"), &mut classpath)?;
    collect_jars(&work.join("libraries"), &mut classpath)?;
    let classpath = std::env::join_paths(classpath)?;
    let dumper = Path::new(env!("CARGO_MANIFEST_DIR")).join("java/DumpPhysics.java");
    let status = java()
        .current_dir(&work)
        .arg("-cp")
        .arg(&classpath)
        .arg(&dumper)
        .arg(reports.join("physics.json"))
        .status()?;
    ensure!(
        status.success(),
        "DumpPhysics failed: {status}. It needs a JDK, not just a JRE"
    );
    // The jar's version.json holds the protocol version a client sends in its handshake.
    let jar = std::fs::read(work.join("server.jar"))?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(jar))?;
    let version_json: Value = serde_json::from_reader(archive.by_name("version.json")?)?;
    let protocol = version_json["protocol_version"]
        .as_u64()
        .context("version.json has no protocol_version")?;
    let source = serde_json::json!({
        "version": version,
        "server_jar_sha1": server_sha1,
        "protocol_version": protocol,
    });
    write(
        &reports.join("source.json"),
        serde_json::to_string_pretty(&source)?.as_bytes(),
    )?;
    println!(
        "resources/reports/: {} and items.json from the {version} data generator, and physics.json",
        REPORTS.join(", ")
    );
    Ok(())
}

/// Each item's `[name key, max stack size, rarity, equipment slot or null, equipment
/// asset or null]`, from the
/// components report's file per item, sorted by name.
fn item_report(dir: &Path) -> Result<String> {
    let mut items = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_stem().context("item file")?.to_string_lossy();
        let json: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let c = &json["components"];
        let entry = serde_json::json!([
            c["minecraft:item_name"]["translate"],
            c["minecraft:max_stack_size"],
            c["minecraft:rarity"],
            c["minecraft:equippable"]["slot"],
            c["minecraft:equippable"]["asset_id"],
        ]);
        items.insert(format!("minecraft:{name}"), entry.to_string());
    }
    let lines: Vec<String> = items
        .iter()
        .map(|(k, v)| format!("  \"{k}\": {v}"))
        .collect();
    Ok(format!("{{\n{}\n}}\n", lines.join(",\n")))
}

fn collect_jars(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_jars(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "jar") {
            out.push(path);
        }
    }
    Ok(())
}

/// A fresh `target/<name>/` holding the version's server jar, and the jar's SHA-1.
fn server_workdir(version: &str, name: &str) -> Result<(PathBuf, String)> {
    let server = &version_info(version)?["downloads"]["server"];
    let work = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(name);
    if work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    let sha1 = str_at(server, "sha1")?;
    write(
        &work.join("server.jar"),
        &get_verified(str_at(server, "url")?, sha1)?,
    )?;
    Ok((work, sha1.to_owned()))
}

/// `java` from PATH, or the JAVA environment variable. 26.3 needs Java 25.
fn java() -> std::process::Command {
    std::process::Command::new(std::env::var("JAVA").unwrap_or_else(|_| "java".into()))
}

/// Has the vanilla server generate `fixtures/worlds/test-world`: a fixed seed, 12×12 chunks
/// around the origin (four region files), and the blocks listed in
/// `fixtures/worlds/test-world.blocks` placed in the sky, so tests can check our reader
/// against what vanilla itself saved. Running the server accepts Mojang's EULA.
fn gen_world(version: &str) -> Result<()> {
    use std::io::{BufRead, Write};
    const SEED: &str = "20261119";
    const RADIUS: i32 = 96; // blocks either side of the origin: 12×12 chunks

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/worlds");
    let placements = std::fs::read_to_string(fixtures.join("test-world.blocks"))?;
    let (work, _) = server_workdir(version, "gen-world")?;
    write(&work.join("eula.txt"), b"eula=true\n")?;
    let properties = format!(
        "level-seed={SEED}\nonline-mode=false\nspawn-protection=0\nmax-tick-time=-1\n\
         sync-chunk-writes=true\nserver-port=25599\nenable-rcon=false\npause-when-empty-seconds=-1\n"
    );
    write(&work.join("server.properties"), properties.as_bytes())?;

    let mut child = java()
        .current_dir(&work)
        .args(["-Xmx1G", "-jar", "server.jar", "--nogui"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .context("running java; install Java or set JAVA")?;
    let mut stdin = child.stdin.take().expect("piped");
    let (lines_tx, lines) = std::sync::mpsc::channel::<String>();
    let stdout = child.stdout.take().expect("piped");
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            println!("server: {line}");
            if lines_tx.send(line).is_err() {
                break;
            }
        }
    });
    // The first line containing any of `needles`, or an error after ten minutes.
    let wait_for_any = |needles: &[&str]| -> Result<String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let line = lines
                .recv_timeout(left)
                .with_context(|| format!("server never printed {needles:?}"))?;
            if needles.iter().any(|n| line.contains(n)) {
                return Ok(line);
            }
        }
    };
    let wait_for = |needle: &str| wait_for_any(&[needle]).map(drop);
    let mut send = |command: &str| -> Result<()> { Ok(writeln!(stdin, "{command}")?) };

    wait_for("Done (")?;
    // Forceloading only queues the chunks. Ask whether each one is loaded (fully generated
    // and ticking) until all are.
    send(&format!(
        "forceload add -{RADIUS} -{RADIUS} {} {}",
        RADIUS - 1,
        RADIUS - 1
    ))?;
    wait_for("Marked")?;
    let centers: Vec<(i32, i32)> = (-RADIUS / 16..RADIUS / 16)
        .flat_map(|cx| (-RADIUS / 16..RADIUS / 16).map(move |cz| (cx * 16 + 8, cz * 16 + 8)))
        .collect();
    loop {
        let mut loaded = 0;
        for (x, z) in &centers {
            send(&format!("execute if loaded {x} 0 {z}"))?;
            loaded +=
                usize::from(wait_for_any(&["Test passed", "Test failed"])?.contains("passed"));
        }
        println!("{loaded}/{} chunks loaded", centers.len());
        if loaded == centers.len() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
    for line in placements
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        send(&format!("setblock {line}"))?;
        wait_for("Changed the block")?;
    }
    send("save-all flush")?;
    wait_for("Saved the game")?;
    send("stop")?;
    ensure!(child.wait()?.success(), "the server exited with an error");

    let out = fixtures.join("test-world");
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    // Only what the reader needs: level.dat and the overworld's block data, in vanilla's layout.
    write(
        &out.join("level.dat"),
        &std::fs::read(work.join("world/level.dat"))?,
    )?;
    let regions = Path::new("dimensions/minecraft/overworld/region");
    for entry in std::fs::read_dir(work.join("world").join(regions))? {
        let path = entry?.path();
        write(
            &out.join(regions).join(path.file_name().expect("file")),
            &std::fs::read(&path)?,
        )?;
    }
    println!("fixtures/worlds/test-world: written");
    Ok(())
}

fn fetch_assets(version: &str) -> Result<()> {
    let root = resources_dir();
    let version_info = version_info(version)?;

    // Start from an empty tree so files removed upstream don't linger.
    for dir in ["assets", "data"] {
        let path = root.join(dir);
        if path.exists() {
            std::fs::remove_dir_all(&path)?;
        }
    }

    // Sounds, languages, fonts and icons come from the asset index. Written first, so
    // anything the client jar also ships (such as en_us.json) ends up as the jar's copy.
    let index_ref = &version_info["assetIndex"];
    let index: Value = serde_json::from_slice(&get_verified(
        str_at(index_ref, "url")?,
        str_at(index_ref, "sha1")?,
    )?)?;
    let objects: Vec<(String, String)> = index["objects"]
        .as_object()
        .context("asset index has no objects")?
        .iter()
        .filter(|(key, _)| key.contains('/'))
        .map(|(key, obj)| Ok((key.clone(), str_at(obj, "hash")?.to_owned())))
        .collect::<Result<_>>()?;
    download_objects(&root.join("assets"), &objects)?;

    // Textures, models, blockstates, shaders and data packs come from the client jar.
    // Only `assets/` and `data/` are copied; the game's code is not.
    let client = &version_info["downloads"]["client"];
    let jar = get_verified(str_at(client, "url")?, str_at(client, "sha1")?)?;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(jar))?;
    let mut jar_files = 0;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let Some(name) = file.enclosed_name() else {
            continue;
        };
        if file.is_dir() || !(name.starts_with("assets") || name.starts_with("data")) {
            continue;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        write(&root.join(name), &bytes)?;
        jar_files += 1;
    }

    let source = serde_json::json!({
        "version": version,
        "client_jar_sha1": str_at(client, "sha1")?,
        "asset_index": str_at(index_ref, "id")?,
        "asset_index_sha1": str_at(index_ref, "sha1")?,
    });
    write(
        &root.join("source.json"),
        serde_json::to_string_pretty(&source)?.as_bytes(),
    )?;
    println!(
        "resources/: {jar_files} files from the client jar, {} from the asset index",
        objects.len()
    );
    Ok(())
}

fn download_objects(assets: &Path, objects: &[(String, String)]) -> Result<()> {
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..DOWNLOAD_THREADS)
            .map(|_| {
                scope.spawn(|| -> Result<()> {
                    while let Some((key, hash)) = objects.get(next.fetch_add(1, Ordering::Relaxed))
                    {
                        let bytes =
                            get_verified(&format!("{OBJECTS_URL}/{}/{hash}", &hash[..2]), hash)
                                .with_context(|| format!("downloading {key}"))?;
                        write(&assets.join(key), &bytes)?;
                    }
                    Ok(())
                })
            })
            .collect();
        workers
            .into_iter()
            .try_for_each(|w| w.join().expect("download thread panicked"))
    })
}

fn get(url: &str) -> Result<Vec<u8>> {
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))?;
    Ok(response
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_to_vec()?)
}

fn get_verified(url: &str, sha1: &str) -> Result<Vec<u8>> {
    let bytes = get(url)?;
    let actual = sha1_smol::Sha1::from(&bytes).digest().to_string();
    ensure!(actual == sha1, "{url}: sha1 {actual}, expected {sha1}");
    Ok(bytes)
}

fn write(path: &PathBuf, bytes: &[u8]) -> Result<()> {
    std::fs::create_dir_all(path.parent().expect("file path has a parent"))?;
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

fn str_at<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("missing string field {key}"))
}
