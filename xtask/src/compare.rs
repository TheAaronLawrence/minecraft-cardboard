//! `cargo xtask compare [views]`: renders the views in `fixtures/compare.views` (or the file
//! given) in vanilla and in our client, and measures how far apart they are.
//!
//! Vanilla runs for real: its server on a copy of the world `cargo xtask gen-world` left in
//! `target/gen-world/world`, and its client under Xvfb, joined in spectator mode, taking an F2
//! screenshot at each view. Then our client renders the same views. `target/compare/` gets
//! both sets of screenshots, a vanilla | ours | difference image per view, and `report.md`
//! with each view's RMSE. Needs Linux with Xvfb and xdotool, and Java 25 (`java` on PATH or
//! JAVA). Running the server accepts Mojang's EULA.

use crate::{DEFAULT_VERSION, DOWNLOAD_THREADS, get_verified, java, str_at, version_info, write};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

const SIZE: (u32, u32) = (960, 540);
/// Render distance in chunks, the same in both clients.
const DISTANCE: u32 = 5;
const DISPLAY: &str = ":91";
const PORT: u16 = 25601;
/// Vanilla's player eye height; views give the eye, `/tp` takes the feet.
const EYE_HEIGHT: f64 = 1.62;
/// How long vanilla gets after each teleport to load and mesh the chunks around it.
const SETTLE: Duration = Duration::from_secs(20);

struct View {
    name: String,
    eye: [f64; 3],
    yaw: f64,
    pitch: f64,
}

pub fn compare(views: Option<&str>) -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()?;
    let views_path = views.map_or_else(|| root.join("fixtures/compare.views"), PathBuf::from);
    let views = read_views(&views_path)?;
    let target = root.join("target");
    let world = target.join("gen-world");
    ensure!(
        world.join("world/level.dat").exists(),
        "no world in target/gen-world; run `cargo xtask gen-world` first"
    );
    let out = target.join("compare");
    if out.exists() {
        std::fs::remove_dir_all(&out)?;
    }
    for dir in ["vanilla", "ours", "side-by-side"] {
        std::fs::create_dir_all(out.join(dir))?;
    }

    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(&root)
        .args(["build", "--release", "-p", "mc-client"])
        .status()?;
    ensure!(status.success(), "building mc-client failed");
    let launch = vanilla_client(DEFAULT_VERSION, &target.join("vanilla"))?;

    let _xvfb = Kill(
        Command::new("Xvfb")
            .args([
                DISPLAY,
                "-screen",
                "0",
                &format!("{}x{}x24", SIZE.0, SIZE.1),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("running Xvfb; install it (apt install xvfb)")?,
    );
    std::thread::sleep(Duration::from_secs(2));
    vanilla_shots(&launch, &world, &target.join("vanilla"), &views, &out)?;

    let ours = root.join("target/release/minecraft-cardboard");
    for view in &views {
        let [x, y, z] = view.eye;
        let status = Command::new(&ours)
            .env("DISPLAY", DISPLAY)
            .args([
                "--pos",
                &format!("{x},{y},{z}"),
                "--look",
                &format!("{},{}", view.yaw, view.pitch),
                "--size",
                &format!("{}x{}", SIZE.0, SIZE.1),
                "--distance",
                &DISTANCE.to_string(),
                "--screenshot",
            ])
            .arg(out.join("ours").join(format!("{}.png", view.name)))
            .stdout(Stdio::null())
            .status()?;
        ensure!(status.success(), "our client failed on view {}", view.name);
    }

    let mut report = String::from(
        "# Vanilla comparison\n\nRMSE of each view's RGB, 0 to 1. \
         `side-by-side/` shows vanilla | ours | difference ×4.\n\n| View | RMSE |\n|---|---|\n",
    );
    for view in &views {
        let file = format!("{}.png", view.name);
        let vanilla = out.join("vanilla").join(&file);
        if !vanilla.exists() {
            report.push_str(&format!("| {} | no vanilla screenshot |\n", view.name));
            continue;
        }
        let vanilla = image::open(&vanilla)?.to_rgb8();
        let ours = image::open(out.join("ours").join(&file))?.to_rgb8();
        let (rmse, side) = difference(&vanilla, &ours)?;
        side.save(out.join("side-by-side").join(&file))?;
        report.push_str(&format!("| {} | {rmse:.4} |\n", view.name));
    }
    write(&out.join("report.md"), report.as_bytes())?;
    print!("{report}");
    println!("\nscreenshots and report in {}", out.display());
    Ok(())
}

fn read_views(path: &Path) -> Result<Vec<View>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [name, x, y, z, yaw, pitch] = fields[..] else {
                bail!("{line:?}: expected name x y z yaw pitch");
            };
            Ok(View {
                name: name.to_owned(),
                eye: [x.parse()?, y.parse()?, z.parse()?],
                yaw: yaw.parse()?,
                pitch: pitch.parse()?,
            })
        })
        .collect()
}

/// The RMSE of two images' RGB, and the two side by side with their difference, scaled ×4.
fn difference(a: &image::RgbImage, b: &image::RgbImage) -> Result<(f64, image::RgbImage)> {
    ensure!(
        a.dimensions() == b.dimensions(),
        "screenshots differ in size"
    );
    let (w, h) = a.dimensions();
    let mut side = image::RgbImage::new(w * 3, h);
    let mut sum = 0.0;
    for (x, y, pa) in a.enumerate_pixels() {
        let pb = b.get_pixel(x, y);
        let diff = image::Rgb(std::array::from_fn(|i| {
            let d = (pa[i] as f64 - pb[i] as f64) / 255.0;
            sum += d * d;
            (pa[i].abs_diff(pb[i]) as u32 * 4).min(255) as u8
        }));
        side.put_pixel(x, y, *pa);
        side.put_pixel(w + x, y, *pb);
        side.put_pixel(2 * w + x, y, diff);
    }
    Ok(((sum / (w * h * 3) as f64).sqrt(), side))
}

/// What launching the vanilla client needs.
struct Launch {
    classpath: String,
    assets: PathBuf,
    asset_index: String,
}

/// Downloads the vanilla client jar, its Linux libraries and its assets into `dir`, keeping
/// files whose SHA-1 already matches.
fn vanilla_client(version: &str, dir: &Path) -> Result<Launch> {
    let info = version_info(version)?;
    let mut files = Vec::new();
    let mut classpath = Vec::new();
    for lib in info["libraries"].as_array().context("no libraries")? {
        let Some(artifact) = lib["downloads"].get("artifact") else {
            continue;
        };
        if !linux_allowed(&lib["rules"]) {
            continue;
        }
        let path = dir.join("libraries").join(str_at(artifact, "path")?);
        files.push((
            str_at(artifact, "url")?.to_owned(),
            str_at(artifact, "sha1")?.to_owned(),
            path.clone(),
        ));
        classpath.push(path);
    }
    let client = &info["downloads"]["client"];
    let jar = dir.join(format!("versions/{version}/{version}.jar"));
    files.push((
        str_at(client, "url")?.to_owned(),
        str_at(client, "sha1")?.to_owned(),
        jar.clone(),
    ));
    classpath.push(jar);

    let index_ref = &info["assetIndex"];
    let asset_index = str_at(index_ref, "id")?.to_owned();
    let assets = dir.join("assets");
    let index_path = assets.join(format!("indexes/{asset_index}.json"));
    fetch_all(&[(
        str_at(index_ref, "url")?.to_owned(),
        str_at(index_ref, "sha1")?.to_owned(),
        index_path.clone(),
    )])?;
    let index: Value = serde_json::from_slice(&std::fs::read(&index_path)?)?;
    for object in index["objects"].as_object().context("no objects")?.values() {
        let hash = str_at(object, "hash")?;
        files.push((
            format!("{}/{}/{hash}", crate::OBJECTS_URL, &hash[..2]),
            hash.to_owned(),
            assets.join("objects").join(&hash[..2]).join(hash),
        ));
    }
    fetch_all(&files)?;
    let classpath = classpath
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(":");
    Ok(Launch {
        classpath,
        assets,
        asset_index,
    })
}

/// The launcher's library rules, for Linux on x86-64 with no optional features.
fn linux_allowed(rules: &Value) -> bool {
    let Some(rules) = rules.as_array() else {
        return true;
    };
    let mut allowed = false;
    for rule in rules.iter().filter(|r| r.get("features").is_none()) {
        let os = &rule["os"];
        let matches = os.is_null()
            || (os["name"] == "linux" && (os["arch"].is_null() || os["arch"] == "x86_64"));
        if matches {
            allowed = rule["action"] == "allow";
        }
    }
    allowed
}

/// Downloads `(url, sha1, path)` files on several threads, skipping ones already present.
fn fetch_all(files: &[(String, String, PathBuf)]) -> Result<()> {
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..DOWNLOAD_THREADS)
            .map(|_| {
                scope.spawn(|| -> Result<()> {
                    while let Some((url, sha1, path)) =
                        files.get(next.fetch_add(1, Ordering::Relaxed))
                    {
                        let present = std::fs::read(path).is_ok_and(|bytes| {
                            sha1_smol::Sha1::from(&bytes).digest().to_string() == *sha1
                        });
                        if !present {
                            write(path, &get_verified(url, sha1)?)?;
                        }
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

/// Runs the vanilla server and client and saves an F2 screenshot of each view to
/// `out/vanilla/`.
fn vanilla_shots(
    launch: &Launch,
    world: &Path,
    dir: &Path,
    views: &[View],
    out: &Path,
) -> Result<()> {
    let server_dir = dir.join("server");
    if server_dir.exists() {
        std::fs::remove_dir_all(&server_dir)?;
    }
    copy_dir(&world.join("world"), &server_dir.join("world"))?;
    std::fs::copy(world.join("server.jar"), server_dir.join("server.jar"))?;
    write(&server_dir.join("eula.txt"), b"eula=true\n")?;
    let properties = format!(
        "online-mode=false\nwhite-list=false\nenforce-whitelist=false\nserver-port={PORT}\n\
         spawn-protection=0\nview-distance={}\n\
         simulation-distance={DISTANCE}\npause-when-empty-seconds=-1\nspawn-monsters=false\n",
        DISTANCE + 1
    );
    write(&server_dir.join("server.properties"), properties.as_bytes())?;
    let mut server = Kill(
        java()
            .current_dir(&server_dir)
            .args(["-Xmx1G", "-jar", "server.jar", "--nogui"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context("running java; install Java 25 or set JAVA")?,
    );
    let mut stdin = server.0.stdin.take().expect("piped");
    let lines = server_lines(&mut server.0);
    wait_for(&lines, "Done (", Duration::from_secs(600))?;

    // A game directory with fixed options: 70° FOV with no flying widening, the HUD and
    // clouds off, and everything else at vanilla's defaults.
    let game = dir.join("game");
    let screenshots = game.join("screenshots");
    if screenshots.exists() {
        std::fs::remove_dir_all(&screenshots)?;
    }
    let options = [
        "fov:0.0",
        "fovEffectScale:0.0",
        &format!("renderDistance:{DISTANCE}"),
        &format!("simulationDistance:{DISTANCE}"),
        "guiScale:2",
        "bobView:false",
        "renderClouds:\"false\"",
        "entityShadows:false",
        "mipmapLevels:4",
        "onboardAccessibility:false",
        "skipMultiplayerWarning:true",
        "joinedFirstServer:true",
        "tutorialStep:none",
        "pauseOnLostFocus:false",
        "narrator:0",
        "soundCategory_master:0.0",
        "gamma:0.5",
        "ao:true",
        "biomeBlendRadius:2",
        "fullscreen:false",
    ]
    .join("\n");
    write(&game.join("options.txt"), options.as_bytes())?;
    let natives = dir.join("natives");
    let log = std::fs::File::create(out.join("vanilla/client.log"))?;
    let _client = Kill(
        java()
            .current_dir(&game)
            .env("DISPLAY", DISPLAY)
            .args([
                "-Xmx2G",
                "-XX:StackShadowPages=32",
                "--enable-native-access=ALL-UNNAMED",
            ])
            .args(["--add-exports", "java.base/jdk.internal.misc=ALL-UNNAMED"])
            .arg(format!(
                "-Djava.library.path={}",
                natives.join("java").display()
            ))
            .arg(format!("-Djna.tmpdir={}", natives.join("jna").display()))
            .arg(format!(
                "-Dorg.lwjgl.system.SharedLibraryExtractPath={}",
                natives.join("lwjgl").display()
            ))
            .arg(format!(
                "-Dio.netty.native.workdir={}",
                natives.join("netty").display()
            ))
            .args(["-cp", &launch.classpath, "net.minecraft.client.main.Main"])
            .args([
                "--username",
                "cardboard",
                "--version",
                DEFAULT_VERSION,
                "--gameDir",
            ])
            .arg(&game)
            .arg("--assetsDir")
            .arg(&launch.assets)
            .args(["--assetIndex", &launch.asset_index])
            .args([
                "--uuid",
                "00000000000000000000000000000001",
                "--accessToken",
                "0",
            ])
            .args([
                "--width",
                &SIZE.0.to_string(),
                "--height",
                &SIZE.1.to_string(),
            ])
            .args(["--quickPlayMultiplayer", &format!("localhost:{PORT}")])
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?,
    );
    wait_for(
        &lines,
        "cardboard joined the game",
        Duration::from_secs(900),
    )?;

    let mut send = |command: &str| -> Result<()> { Ok(writeln!(stdin, "{command}")?) };
    for command in [
        "gamemode spectator cardboard",
        "time set 6000",
        "weather clear",
        "gamerule advance_time false",
        "gamerule advance_weather false",
    ] {
        send(command)?;
    }
    std::thread::sleep(Duration::from_secs(5));
    key("F1")?; // hide the HUD
    for view in views {
        let [x, y, z] = view.eye;
        // Coordinates with a decimal point, or vanilla centres them on the block.
        send(&format!(
            "tp cardboard {x:.4} {:.4} {z:.4} {} {}",
            y - EYE_HEIGHT,
            view.yaw,
            view.pitch
        ))?;
        std::thread::sleep(SETTLE);
        let before = png_files(&screenshots);
        key("F2")?;
        let deadline = Instant::now() + Duration::from_secs(15);
        let shot = loop {
            std::thread::sleep(Duration::from_millis(500));
            if let Some(new) = png_files(&screenshots)
                .into_iter()
                .find(|p| !before.contains(p))
            {
                break Some(new);
            }
            if Instant::now() > deadline {
                break None;
            }
        };
        match shot {
            Some(path) => {
                std::thread::sleep(Duration::from_secs(1)); // let vanilla finish writing it
                std::fs::copy(path, out.join("vanilla").join(format!("{}.png", view.name)))?;
                println!("vanilla: {}", view.name);
            }
            None => println!("vanilla: no screenshot for {}", view.name),
        }
    }
    send("stop")?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while server.0.try_wait()?.is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

fn server_lines(server: &mut Child) -> Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let stdout = server.stdout.take().expect("piped");
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            println!("server: {line}");
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

fn wait_for(lines: &Receiver<String>, needle: &str, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = lines
            .recv_timeout(left)
            .with_context(|| format!("the server never printed {needle:?}"))?;
        if line.contains(needle) {
            return Ok(());
        }
    }
}

fn key(name: &str) -> Result<()> {
    let status = Command::new("xdotool")
        .env("DISPLAY", DISPLAY)
        .args(["key", name])
        .status()
        .context("running xdotool; install it (apt install xdotool)")?;
    ensure!(status.success(), "xdotool key {name} failed");
    Ok(())
}

fn png_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| Some(e.ok()?.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "png"))
                .collect()
        })
        .unwrap_or_default()
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

/// A child process killed when this goes out of scope, so an error never leaves the server,
/// the client or Xvfb running.
struct Kill(Child);

impl Drop for Kill {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
