//! Singleplayer: until the game has a server of its own, a world is Mojang's dedicated
//! server, running on this machine and joined like any other. The first time, Mojang's
//! Java runtime and the server jar of the version the game matches are downloaded from
//! Mojang, as Mojang's launcher downloads them, into `runtime/` and `server/` beside the
//! exe. Each world is a folder in `saves/` there, holding the server's files and, in
//! `world/`, the save.

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
/// Mojang's list of Java runtimes, which its launcher reads.
const RUNTIMES_URL: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

/// Reports what starting a world is doing; an error means the player cancelled.
pub type Status<'a> = &'a (dyn Fn(String) -> Result<()> + Sync);

/// The folder beside the exe that `name` lives in.
fn beside_exe(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(name)))
        .unwrap_or_else(|| name.into())
}

fn saves() -> PathBuf {
    beside_exe("saves")
}

/// What a world was created with, kept in its folder's `world.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct NewWorld {
    pub name: String,
    /// `survival`, `hardcore` or `creative`.
    pub game_mode: String,
    /// `peaceful`, `easy`, `normal` or `hard`.
    pub difficulty: String,
    /// Whether the player is an operator, who can run commands such as /gamemode.
    pub commands: bool,
    /// Blank for a random one.
    pub seed: String,
    /// Villages, strongholds and the like.
    pub structures: bool,
}

/// A world in `saves/`.
pub struct World {
    pub dir: PathBuf,
    pub settings: NewWorld,
    /// The Minecraft version it was created in.
    pub version: String,
    pub last_played: SystemTime,
}

impl World {
    pub fn folder(&self) -> String {
        self.dir
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

fn read_settings(dir: &Path) -> Option<(NewWorld, String)> {
    let json: Value = serde_json::from_slice(&std::fs::read(dir.join("world.json")).ok()?).ok()?;
    let text = |key: &str, default: &str| json[key].as_str().unwrap_or(default).to_owned();
    let settings = NewWorld {
        name: json["name"].as_str()?.to_owned(),
        game_mode: text("game_mode", "survival"),
        difficulty: text("difficulty", "normal"),
        commands: json["commands"].as_bool().unwrap_or(false),
        seed: text("seed", ""),
        structures: json["structures"].as_bool().unwrap_or(true),
    };
    Some((settings, text("version", "?")))
}

/// The Minecraft version the game's resources come from.
fn game_version() -> Result<String> {
    let path = mc_assets::default_root().join("source.json");
    let source: Value = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    Ok(str_at(&source, "version")?.to_owned())
}

/// The worlds in `saves/`, most recently played first, as `WorldSelectionList` sorts them.
pub fn worlds() -> Vec<World> {
    let mut worlds: Vec<World> = std::fs::read_dir(saves())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            let (settings, version) = read_settings(&dir)?;
            let modified = |p: PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
            let last_played = modified(dir.join("world/level.dat"))
                .or_else(|| modified(dir.join("world.json")))
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some(World {
                dir,
                settings,
                version,
                last_played,
            })
        })
        .collect();
    worlds.sort_by_key(|w| std::cmp::Reverse(w.last_played));
    worlds
}

/// A folder name for a world called `name` that no world has yet, as
/// `FileUtil.findAvailableName` makes one: characters Windows doesn't allow become `_`,
/// and a taken name gets ` (1)`, ` (2)` and so on.
pub fn folder_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut base: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    base = base.trim().trim_end_matches('.').to_owned();
    if base.is_empty() {
        base = "World".into();
    }
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&base)) {
        base = format!("_{base}_");
    }
    let mut folder = base.clone();
    let mut n = 1;
    while taken(&folder) {
        folder = format!("{base} ({n})");
        n += 1;
    }
    folder
}

/// Makes a new world's folder in `saves/`: its settings, and the server's properties for a
/// singleplayer world, which the server generates on its first start.
pub fn create(new: &NewWorld) -> Result<PathBuf> {
    let saves = saves();
    let folder = folder_name(&new.name, |f| {
        let path = saves.join(f);
        path.exists() || path.with_extension("").exists()
    });
    let dir = saves.join(folder);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let settings = serde_json::json!({
        "name": new.name,
        "game_mode": new.game_mode,
        "difficulty": new.difficulty,
        "commands": new.commands,
        "seed": new.seed,
        "structures": new.structures,
        "version": game_version()?,
    });
    std::fs::write(
        dir.join("world.json"),
        serde_json::to_vec_pretty(&settings)?,
    )?;
    // Escaped as `.properties` files escape (`Properties.store`).
    let escape = |s: &str| {
        s.chars()
            .flat_map(|c| match c {
                '\\' | ':' | '=' | '#' | '!' => vec!['\\', c],
                c => vec![c],
            })
            .collect::<String>()
    };
    let hardcore = new.game_mode == "hardcore";
    let properties = [
        // Hardcore is Survival on Hard, with one life.
        format!(
            "gamemode={}",
            if hardcore { "survival" } else { &new.game_mode }
        ),
        format!("hardcore={hardcore}"),
        format!("difficulty={}", new.difficulty),
        // No spawn protection, which would stop a player who isn't an operator building
        // near spawn.
        "spawn-protection=0".into(),
        format!("level-seed={}", escape(new.seed.trim())),
        format!("generate-structures={}", new.structures),
        "level-name=world".into(),
        // Only this machine can join, without Mojang's sign-in.
        "server-ip=127.0.0.1".into(),
        "online-mode=false".into(),
        "max-players=1".into(),
        format!("motd={}", escape(&new.name)),
    ];
    std::fs::write(dir.join("server.properties"), properties.join("\n") + "\n")?;
    // Running Mojang's server means agreeing to the Minecraft EULA, which playing the
    // game already does; the Create New World screen says so.
    std::fs::write(
        dir.join("eula.txt"),
        "# https://aka.ms/MinecraftEULA\neula=true\n",
    )?;
    Ok(dir)
}

/// Sets `key` in a `.properties` file's text, replacing its line or adding one.
fn set_property(text: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with(&prefix))
        .map(str::to_owned)
        .collect();
    lines.push(format!("{prefix}{value}"));
    lines.join("\n") + "\n"
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
        .with_context(|| format!("Mojang's files have no {key}"))
}

/// Downloads `url`, checking its SHA-1, calling `progress` with the bytes so far.
fn download(url: &str, sha1: &str, mut progress: impl FnMut(u64) -> Result<()>) -> Result<Vec<u8>> {
    let mut response = ureq::get(url)
        .call()
        .with_context(|| format!("GET {url}"))
        .context("Couldn't reach Mojang; check your internet connection")?;
    let mut reader = response.body_mut().with_config().limit(u64::MAX).reader();
    let mut bytes = Vec::new();
    let mut buf = vec![0; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .context("The download stopped; check your internet connection")?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        progress(bytes.len() as u64)?;
    }
    ensure!(
        sha1_hex(&bytes) == sha1,
        "A download from Mojang was damaged; try again"
    );
    Ok(bytes)
}

fn sha1_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Reports "`what`: N%" as N changes.
fn percent(status: Status, what: &str, total: u64) -> impl FnMut(u64) -> Result<()> {
    let mut shown = None;
    move |done| {
        let p = (done * 100).checked_div(total).unwrap_or(0).min(100);
        if shown != Some(p) {
            shown = Some(p);
            status(format!("{what}: {p}%"))?;
        }
        Ok(())
    }
}

/// The version json of the Minecraft version the game's resources come from.
fn version_json() -> Result<Value> {
    let version = game_version()?;
    let manifest = get_json(MANIFEST_URL)?;
    let entry = manifest["versions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|v| v["id"] == version)
        .with_context(|| format!("Mojang has no Minecraft {version}"))?;
    get_json(str_at(entry, "url")?)
}

/// The name Mojang's runtime list gives this platform.
fn platform() -> Result<&'static str> {
    Ok(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64",
        ("windows", "aarch64") => "windows-arm64",
        ("linux", "x86_64") => "linux",
        ("macos", "x86_64") => "mac-os",
        ("macos", "aarch64") => "mac-os-arm64",
        (os, arch) => bail!("Mojang has no Java for {os} on {arch}"),
    })
}

/// Downloads Mojang's Java runtime `component` (such as `java-runtime-epsilon`) into
/// `runtime/`, file by file from its manifest, unless it's already there; returns its
/// `java`.
fn java(component: &str, status: Status) -> Result<PathBuf> {
    let dir = beside_exe("runtime").join(component);
    let marker = dir.join(".cardboard-installed");
    let installed = std::fs::read_to_string(&marker).unwrap_or_default();
    let java_in = |path: &str| dir.join(path);
    if let Some(java) = installed.lines().nth(1)
        && java_in(java).exists()
    {
        return Ok(java_in(java));
    }
    status("Finding Java at Mojang".into())?;
    let runtimes = get_json(RUNTIMES_URL)?;
    let entry = runtimes[platform()?][component]
        .as_array()
        .and_then(|a| a.first())
        .with_context(|| {
            format!(
                "Mojang has no {component} for {}",
                platform().unwrap_or("?")
            )
        })?;
    let manifest = get_json(str_at(&entry["manifest"], "url")?)?;
    let files = manifest["files"]
        .as_object()
        .context("Mojang's Java manifest has no files")?;
    // `bin/java` (in `jre.bundle/Contents/Home/` on macOS), not `javaw`.
    let java_path = files
        .keys()
        .filter(|p| p.ends_with("bin/java") || p.ends_with("bin/java.exe"))
        .min_by_key(|p| p.len())
        .context("Mojang's Java has no java")?
        .clone();
    if dir.exists() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
    }
    let mut downloads = Vec::new();
    for (path, file) in files {
        let target = dir.join(path);
        match file["type"].as_str() {
            Some("directory") => std::fs::create_dir_all(&target)?,
            Some("file") => {
                let raw = &file["downloads"]["raw"];
                downloads.push((
                    target,
                    str_at(raw, "url")?.to_owned(),
                    str_at(raw, "sha1")?.to_owned(),
                    raw["size"].as_u64().unwrap_or(0),
                    file["executable"].as_bool().unwrap_or(false),
                ));
            }
            #[cfg(unix)]
            Some("link") => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::os::unix::fs::symlink(str_at(file, "target")?, &target)?;
            }
            _ => {}
        }
    }
    // Hundreds of small files: a few at once.
    let total: u64 = downloads.iter().map(|d| d.3).sum();
    let done = Mutex::new((0u64, percent(status, "Downloading Java from Mojang", total)));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failed: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                while failed.lock().unwrap().is_none() {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some((target, url, sha1, size, executable)) = downloads.get(i) else {
                        return;
                    };
                    let result = (|| -> Result<()> {
                        let bytes = download(url, sha1, |_| Ok(()))?;
                        if let Some(parent) = target.parent() {
                            std::fs::create_dir_all(parent)?;
                        }
                        std::fs::write(target, bytes)
                            .with_context(|| format!("writing {}", target.display()))?;
                        #[cfg(unix)]
                        if *executable {
                            use std::os::unix::fs::PermissionsExt;
                            std::fs::set_permissions(
                                target,
                                std::fs::Permissions::from_mode(0o755),
                            )?;
                        }
                        let _ = executable;
                        let mut done = done.lock().unwrap();
                        done.0 += size;
                        let bytes = done.0;
                        (done.1)(bytes)
                    })();
                    if let Err(e) = result {
                        failed.lock().unwrap().get_or_insert(e);
                    }
                }
            });
        }
    });
    if let Some(e) = failed.into_inner().unwrap() {
        return Err(e);
    }
    let sha1 = str_at(&entry["manifest"], "sha1")?;
    std::fs::write(&marker, format!("{sha1}\n{java_path}\n"))?;
    Ok(java_in(&java_path))
}

/// Downloads the server jar into `server/` unless it's already there; returns its path.
fn server_jar(version: &Value, status: Status) -> Result<PathBuf> {
    let server = &version["downloads"]["server"];
    let sha1 = str_at(server, "sha1")?;
    let path = beside_exe("server").join("server.jar");
    if std::fs::read(&path).is_ok_and(|bytes| sha1_hex(&bytes) == sha1) {
        return Ok(path);
    }
    let what = format!(
        "Downloading the Minecraft {} server from Mojang",
        str_at(version, "id")?
    );
    let total = server["size"].as_u64().unwrap_or(0);
    let bytes = download(str_at(server, "url")?, sha1, percent(status, &what, total))?;
    std::fs::create_dir_all(path.parent().expect("in server/"))?;
    std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Servers told to stop, which may still be saving their worlds.
static STOPPING: Mutex<Vec<Child>> = Mutex::new(Vec::new());

/// Waits for stopped servers to finish saving, killing any that take over a minute.
pub fn wait_for_stopped() {
    let mut stopping = STOPPING.lock().unwrap();
    for child in stopping.iter_mut() {
        let deadline = Instant::now() + Duration::from_secs(60);
        while matches!(child.try_wait(), Ok(None)) {
            if Instant::now() > deadline {
                let _ = child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    stopping.clear();
}

/// A world's server, running. Dropping it tells the server to save and stop.
pub struct LocalServer {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    pub port: u16,
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.write_all(b"stop\n");
        }
        if let Some(child) = self.child.take() {
            STOPPING.lock().unwrap().push(child);
        }
    }
}

/// The 8-4-4-4-12 form of a UUID.
fn uuid_string(uuid: u128) -> String {
    let h = format!("{uuid:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// Starts the world in `dir` for `player`: downloads Java and the server if they aren't
/// here yet, then runs the server on a free port and waits until it's ready to join.
pub fn start(dir: &Path, player: &str, status: Status) -> Result<LocalServer> {
    let (settings, _) = read_settings(dir).context("the world has no world.json")?;
    status("Checking Minecraft's files".into())?;
    let version = version_json()?;
    let component = str_at(&version["javaVersion"], "component")?.to_owned();
    let java = java(&component, status)?;
    let jar = server_jar(&version, status)?;
    // A server stopped a moment ago may still hold this world's lock.
    status("Saving world".into())?;
    wait_for_stopped();

    let port = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    let properties = dir.join("server.properties");
    let text = std::fs::read_to_string(&properties).unwrap_or_default();
    // A new server turns its whitelist on, which only operators get past.
    let text = set_property(&text, "white-list", "false");
    std::fs::write(
        &properties,
        set_property(&text, "server-port", &port.to_string()),
    )?;
    // The player as an operator, or not, by whether the world allows commands.
    let ops = if settings.commands {
        serde_json::json!([{
            "uuid": uuid_string(mc_protocol::offline_uuid(player)),
            "name": player,
            "level": 4,
            "bypassesPlayerLimit": false,
        }])
    } else {
        serde_json::json!([])
    };
    std::fs::write(dir.join("ops.json"), serde_json::to_vec_pretty(&ops)?)?;

    status("Starting the world".into())?;
    let mut command = Command::new(&java);
    command
        .current_dir(dir)
        .arg("-Xmx2G")
        // The jar unpacks its libraries once, into `server/`, not into every world.
        .arg(format!(
            "-DbundlerRepoDir={}",
            jar.parent().expect("in server/").display()
        ))
        .arg("-jar")
        .arg(&jar)
        .arg("--nogui")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        // `CREATE_NO_WINDOW`: no console window for the server.
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("starting {}", java.display()))?;
    let stdin = child.stdin.take();
    // The server's output: watched for when it's ready, and always read, so it never
    // blocks on a full pipe. The last lines say why, if it stops.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    for pipe in [
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
    }
    drop(tx);
    let server = LocalServer {
        child: Some(child),
        stdin,
        port,
    };
    let mut log: std::collections::VecDeque<String> = Default::default();
    let mut shown = String::new();
    let deadline = Instant::now() + Duration::from_secs(600);
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(line) => {
                if line.contains("]: Done (") {
                    return Ok(server);
                }
                // `Preparing spawn area: 45%`, as the world generates.
                let message = line.rsplit_once("]: ").map_or(line.as_str(), |(_, m)| m);
                if let Some(p) = message.strip_prefix("Preparing spawn area: ") {
                    shown = format!("Preparing spawn area: {p}");
                }
                if log.len() == 8 {
                    log.pop_front();
                }
                log.push_back(message.to_owned());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let last: Vec<String> = log.into_iter().collect();
                bail!("The world's server stopped: {}", last.join(" "));
            }
        }
        // Also how a cancel is noticed: the status can't be shown.
        status(if shown.is_empty() {
            "Loading the world".into()
        } else {
            shown.clone()
        })?;
        ensure!(
            Instant::now() < deadline,
            "The world's server took too long to start"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Names become folders Windows accepts, and a taken one is numbered.
    #[test]
    fn world_folders() {
        let none = |_: &str| false;
        assert_eq!(folder_name("New World", none), "New World");
        assert_eq!(folder_name("a/b:c?", none), "a_b_c_");
        assert_eq!(folder_name("  ...  ", none), "World");
        assert_eq!(folder_name("con", none), "_con_");
        let taken = |f: &str| f == "World" || f == "World (1)";
        assert_eq!(folder_name("World", taken), "World (2)");
    }

    #[test]
    fn properties_replace_their_line() {
        let text = "motd=x\nserver-port=1\npvp=true\n";
        assert_eq!(
            set_property(text, "server-port", "5"),
            "motd=x\npvp=true\nserver-port=5\n"
        );
        assert_eq!(
            uuid_string(0x0123456789abcdef_0123456789abcdef),
            "01234567-89ab-cdef-0123-456789abcdef"
        );
    }
}
