//! minecraft-cardboard: opens a vanilla save, or joins an offline-mode vanilla server, and
//! lets you fly around the world; or renders one frame to a PNG.
//!
//! ```text
//! minecraft-cardboard [--world DIR | --server HOST:PORT [--name NAME]]
//!                     [--pos X,Y,Z] [--look YAW,PITCH] [--distance CHUNKS]
//!                     [--fov DEG] [--screenshot OUT.png] [--size WxH] [--ticks N]
//! ```
//! On a server, the camera starts where the server puts the player, unless `--pos` and
//! `--look` say otherwise.
//! In the window: click to capture the mouse, WASD to move, Space and Shift to go up and
//! down, Ctrl to fly faster, Esc to pause. On a server, left click mines, right
//! click places or uses, middle click picks the block, and 1 to 9 or the wheel choose the
//! hotbar slot. E opens the inventory, where clicks, shift-clicks, drags and 1 to 9 move
//! items. T opens the chat and / opens it to type a command; Enter sends. Dying
//! opens the death screen, whose buttons respawn or leave.
//!
//! Without `--world` or `--server`, the window opens on the title screen: Singleplayer lists
//! the worlds in `saves/` beside the exe and creates new ones, each played on Mojang's
//! server run on this machine (see `local`), and Multiplayer asks for a server's address
//! (remembered in `options.txt` beside the exe). In a world, Esc (or the window losing focus) opens the pause screen,
//! whose Disconnect, or Save and Quit to Title, comes back to the menus, as losing the
//! connection does. Options, from the title or pause screen, sets the FOV and each sound
//! category's volume, saved in `options.txt` as vanilla saves them.

use anyhow::{Context, Result, bail};
use glam::Vec3;
use mc_render::bake::BlockModels;
use mc_render::hud::keys::Key;
use mc_render::hud::{menu, nav, options};

mod chat;
mod entities;
mod hands;
mod inventory;
mod keys;
mod light;
mod local;
mod particles;
mod skins;
mod sound;
mod stream;
mod walk;
use mc_render::gpu::{Camera, Renderer, View};
use mc_render::mesh::{Mesher, WorldView};
use mc_render::tint::Biomes;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
struct Args {
    world: PathBuf,
    /// `--world` was given, so the window opens that world rather than the title screen.
    world_given: bool,
    server: Option<String>,
    name: String,
    /// `--distance` was given, so it wins over the saved Render Distance.
    distance_given: bool,
    /// `--pos` and `--look` were given, so they win over the server's placement.
    pos_given: bool,
    look_given: bool,
    camera: Camera,
    distance: i32,
    /// Game ticks at the start, which set where end portals' stars have drifted to.
    ticks: f32,
    screenshot: Option<PathBuf>,
    size: (u32, u32),
}

fn parse_args() -> Result<Args> {
    let mut args = Args {
        world: mc_assets::locate("fixtures/worlds/test-world"),
        world_given: false,
        distance_given: false,
        server: None,
        name: "cardboard".into(),
        pos_given: false,
        look_given: false,
        camera: Camera {
            eye: Vec3::new(0.5, 100.0, 0.5),
            yaw: 0.0,
            pitch: 20.0,
            fov: 70.0,
            bob: glam::Mat4::IDENTITY,
        },
        distance: 5,
        ticks: 0.0,
        screenshot: None,
        size: (1280, 720),
    };
    let numbers = |s: &str| -> Result<Vec<f32>> {
        s.split(',')
            .map(|n| {
                n.trim()
                    .parse::<f32>()
                    .with_context(|| format!("not a number: {n}"))
            })
            .collect()
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--world" => (args.world, args.world_given) = (value()?.into(), true),
            "--server" => args.server = Some(value()?),
            "--name" => args.name = value()?,
            "--pos" => match numbers(&value()?)?[..] {
                [x, y, z] => (args.camera.eye, args.pos_given) = (Vec3::new(x, y, z), true),
                _ => bail!("--pos takes X,Y,Z"),
            },
            "--look" => match numbers(&value()?)?[..] {
                [yaw, pitch] => {
                    (args.camera.yaw, args.camera.pitch) = (yaw, pitch);
                    args.look_given = true;
                }
                _ => bail!("--look takes YAW,PITCH"),
            },
            "--distance" => (args.distance, args.distance_given) = (value()?.parse()?, true),
            "--ticks" => args.ticks = value()?.parse()?,
            "--fov" => args.camera.fov = value()?.parse()?,
            "--screenshot" => args.screenshot = Some(value()?.into()),
            "--size" => {
                let v = value()?;
                let (w, h) = v.split_once('x').context("--size takes WxH")?;
                args.size = (w.parse()?, h.parse()?);
            }
            _ => bail!("unknown option {flag}"),
        }
    }
    Ok(args)
}

/// Everything the renderer needs, built from the resources and the save, with the chunks
/// around the start position already loaded; or, on a server, joined and with no chunks yet.
struct Scene {
    models: Arc<BlockModels>,
    biomes: Arc<Biomes>,
    registry: Arc<mc_registry::Registry>,
    source: stream::Source,
    view: WorldView,
    /// On a server, sends the game's own packets.
    outbox: Option<mc_protocol::Outbox>,
}

/// Fails when the resources the game reads aren't where it looks.
fn check_resources() -> Result<()> {
    let root = mc_assets::default_root();
    anyhow::ensure!(
        root.join("reports/blocks.json").exists(),
        "can't find the game's resources (looked in {}). If you downloaded the zip, extract \
         the whole of it, then run minecraft-cardboard.exe from the extracted folder.",
        root.display()
    );
    Ok(())
}

fn load_scene(args: &Args) -> Result<Scene> {
    check_resources()?;
    let root = mc_assets::default_root();
    let res = mc_assets::Resources::new(root);
    let registry = Arc::new(mc_registry::Registry::vanilla()?);
    let (source, view, outbox) = match &args.server {
        Some(address) => {
            // One chunk more than drawn, so the outermost drawn chunks have neighbours.
            let session = mc_protocol::connect(
                address,
                &args.name,
                (args.distance + 1).clamp(2, 32) as u8,
                registry.clone(),
            )?;
            eprintln!("joined {address} as {}", args.name);
            (
                stream::Source::Server(session.events),
                WorldView::empty(&registry),
                Some(session.outbox),
            )
        }
        None => {
            let mut world = mc_world::World::open(&args.world);
            let eye = args.camera.eye;
            // One chunk more than drawn, so the outermost drawn chunks have neighbours to
            // cull against.
            let view = WorldView::load(
                &mut world,
                &registry,
                eye.x.floor() as i32,
                eye.z.floor() as i32,
                args.distance + 1,
            )?;
            (stream::Source::Save(world), view, None)
        }
    };
    // Player heads' skins go in the atlas, so they're fetched before baking. Heads streamed
    // in later wear the default skin for their UUID.
    let skins = skins::fetch(
        view.block_entities()
            .filter_map(|e| mc_render::entity_model::Profile::from_tag(e.get("profile")?)),
    );
    let started = Instant::now();
    let models = BlockModels::bake(&res, &registry, &skins)?;
    // Every biome, since chunks streamed in later can be in any of them.
    let biome_file = res.data_path("worldgen/biome", "minecraft:plains", "json");
    let biome_dir = biome_file.parent().context("no biome folder")?;
    let names: Vec<String> = std::fs::read_dir(biome_dir)
        .with_context(|| format!("{}", biome_dir.display()))?
        .filter_map(|e| {
            let file = e.ok()?.file_name().into_string().ok()?;
            Some(format!("minecraft:{}", file.strip_suffix(".json")?))
        })
        .collect();
    let biomes = Biomes::load(&res, names)?;
    eprintln!(
        "baked {} states in {:.1?}",
        models.states.len(),
        started.elapsed()
    );
    Ok(Scene {
        models: Arc::new(models),
        biomes: Arc::new(biomes),
        registry,
        source,
        view,
        outbox,
    })
}

fn main() -> std::process::ExitCode {
    // A panic's message also goes in the log, and the window waits, as for an error.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        report_failure(&format!("{info}"));
    }));
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {e:?}");
            report_failure(&format!("{e:?}"));
            std::process::ExitCode::FAILURE
        }
    }
}

/// Writes what went wrong to `minecraft-cardboard.log` beside the exe. On Windows, started
/// by double-clicking, the console closes as soon as the program exits, so it also waits
/// for Enter before closing.
fn report_failure(message: &str) {
    let log = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("minecraft-cardboard.log")))
        .unwrap_or_else(|| "minecraft-cardboard.log".into());
    if std::fs::write(&log, format!("{message}\n")).is_ok() {
        eprintln!("(also written to {})", log.display());
    }
    if cfg!(windows) && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        eprintln!("Press Enter to close.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}

fn run() -> Result<()> {
    let mut args = parse_args()?;
    let view = View::new((args.distance * 16) as f32, args.ticks, false);
    match args.screenshot.clone() {
        Some(path) if args.server.is_some() => {
            let scene = load_scene(&args)?;
            let models = scene.models.clone();
            let outbox = scene
                .outbox
                .clone()
                .context("joined without a connection")?;
            let mut streamer = spawn_streamer(scene, &args);
            let mut renderer = Renderer::headless(&models)?;
            server_screenshot(&mut args, &outbox, &mut streamer, &mut renderer)?;
            let image = renderer.screenshot(args.size.0, args.size.1, &args.camera, &view)?;
            image
                .save(&path)
                .with_context(|| format!("saving {}", path.display()))?;
            eprintln!("wrote {}", path.display());
            Ok(())
        }
        Some(path) => {
            let scene = load_scene(&args)?;
            let mut renderer = Renderer::headless(&scene.models)?;
            let started = Instant::now();
            let mesher = Mesher::new(&scene.models, &scene.biomes, &scene.registry);
            let (ex, ez) = stream::chunk_of(args.camera.eye);
            let mut quads = 0;
            let keys: Vec<_> = scene.view.chunk_positions().collect();
            for (cx, cz) in keys {
                if (cx - ex).abs().max((cz - ez).abs()) > args.distance {
                    continue;
                }
                if let Some(mut mesh) = mesher.mesh_chunk(&scene.view, cx, cz) {
                    mesh.sort_translucent(args.camera.eye);
                    quads += mesh.quads();
                    renderer.upload_chunk((cx, cz), &mesh);
                }
            }
            eprintln!("meshed {quads} quads in {:.1?}", started.elapsed());
            let image = renderer.screenshot(args.size.0, args.size.1, &args.camera, &view)?;
            image
                .save(&path)
                .with_context(|| format!("saving {}", path.display()))?;
            eprintln!("wrote {}", path.display());
            Ok(())
        }
        None => {
            check_resources()?;
            // The menus draw with the GUI's sprites and font, from a bake of their own.
            let res = mc_assets::Resources::new(mc_assets::default_root());
            let registry = mc_registry::Registry::vanilla()?;
            let menu_models = Arc::new(BlockModels::bake(&res, &registry, &[])?);
            let event_loop = winit::event_loop::EventLoop::new()?;
            let mut app = App {
                start: args.clone(),
                args,
                models: menu_models.clone(),
                menu_models,
                streamer: None,
                walker: None,
                vignette: 1.0,
                menu: None,
                loading: None,
                local_server: None,
                singleplayer: false,
                last_row_click: None,
                view: View { hud: true, ..view },
                bindings: Default::default(),
                toggles: Default::default(),
                outlined: None,
                shown_hotbar: None,
                shown_cracks: Vec::new(),
                state: None,
                keys: Default::default(),
                last_frame: Instant::now(),
                sounds: sound::Sounds::new(),
                options: None,
                fov: 70,
                drag: None,
                video: Default::default(),
                mouse: Default::default(),
                wheel: 0.0,
                last_input: Instant::now(),
                frame_due: Instant::now(),
                started: Instant::now(),
                sound_clock: 0.0,
                captured: false,
                cursor: None,
                focused_at: Instant::now(),
                paused: false,
                pause_focus: None,
                input_type: None,
                chat: Default::default(),
                modifiers: Default::default(),
            };
            app.load_options();
            if app.start.server.is_some() || app.start.world_given {
                app.play(app.start.server.clone());
            } else {
                app.menu = Some(menu::Menu::default());
            }
            event_loop.run_app(&mut app)?;
            // Quitting in a world stops its server, which saves it before the game ends.
            drop(app);
            local::wait_for_stopped();
            Ok(())
        }
    }
}

/// The VSync option: presenting in step with the display (`Fifo`), or not, if the surface
/// can.
fn present_mode(modes: &[wgpu::PresentMode], vsync: bool) -> wgpu::PresentMode {
    use wgpu::PresentMode::{Fifo, Immediate, Mailbox};
    match vsync {
        true => Fifo,
        false => [Immediate, Mailbox]
            .into_iter()
            .find(|m| modes.contains(m))
            .unwrap_or(Fifo),
    }
}

/// `options.txt` beside the exe, where vanilla keeps its options.
fn options_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("options.txt")))
        .unwrap_or_else(|| "options.txt".into())
}

/// An option saved in `options.txt` as vanilla saves them, `name:value` a line.
fn option(name: &str) -> Option<String> {
    std::fs::read_to_string(options_path())
        .ok()?
        .lines()
        .find_map(|l| Some(l.strip_prefix(name)?.strip_prefix(':')?.to_owned()))
}

/// Saves `options` into `options.txt`, keeping the other lines there.
fn save_options(options: &[(String, String)]) {
    let old = std::fs::read_to_string(options_path()).unwrap_or_default();
    let mut lines: Vec<String> = old
        .lines()
        .filter(|l| {
            let name = l.split(':').next().unwrap_or_default();
            !options.iter().any(|(n, _)| n == name)
        })
        .map(str::to_owned)
        .collect();
    lines.extend(options.iter().map(|(n, v)| format!("{n}:{v}")));
    if let Err(e) = std::fs::write(options_path(), lines.join("\n") + "\n") {
        eprintln!("saving options.txt: {e}");
    }
}

/// `Options.lastMpIp`, saved as `lastServer:`.
fn last_server() -> String {
    option("lastServer").unwrap_or_default()
}

fn save_last_server(address: &str) {
    save_options(&[("lastServer".into(), address.into())]);
}

fn spawn_streamer(scene: Scene, args: &Args) -> stream::Streamer {
    stream::Streamer::spawn(
        stream::Sources {
            source: scene.source,
            view: scene.view,
            models: scene.models,
            biomes: scene.biomes,
            registry: scene.registry,
        },
        args.distance,
        args.camera.eye,
    )
}

/// Where the server put the player's eyes: vanilla's standing eye height over the feet.
fn teleport_eye(teleport: &mc_protocol::Teleport) -> Vec3 {
    let [x, y, z] = teleport.pos.map(|c| c as f32);
    Vec3::new(x, y + 1.62, z)
}

/// Points the camera where the server put the player (unless `--pos`/`--look` say
/// otherwise), and uploads chunks until none has arrived for a few seconds.
fn server_screenshot(
    args: &mut Args,
    outbox: &mc_protocol::Outbox,
    streamer: &mut stream::Streamer,
    renderer: &mut Renderer,
) -> Result<()> {
    const QUIET: std::time::Duration = std::time::Duration::from_secs(3);
    let deadline = Instant::now() + std::time::Duration::from_secs(120);
    let (mut placed, mut last_mesh, mut chunks) = (false, Instant::now(), 0);
    while Instant::now() < deadline && !(placed && last_mesh.elapsed() > QUIET) {
        for update in streamer.updates() {
            match update {
                stream::Update::Mesh(key, mesh) => {
                    renderer.upload_chunk(key, &mesh);
                    (last_mesh, chunks) = (Instant::now(), chunks + 1);
                }
                stream::Update::Remove(key) => renderer.remove_chunk(key),
                stream::Update::Teleport(teleport) => {
                    outbox.accept_teleport(&teleport)?;
                    args.place(teleport_eye(&teleport), teleport.yaw, teleport.pitch);
                    (placed, last_mesh) = (true, Instant::now());
                }
                stream::Update::Disconnected(reason) => bail!("disconnected: {reason}"),
                _ => {}
            }
        }
        streamer.follow(args.camera.eye);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    anyhow::ensure!(placed, "the server never placed the player");
    eprintln!("drew {chunks} chunks from the server");
    Ok(())
}

impl Args {
    /// Moves the camera where the server put the player, except what `--pos` and `--look`
    /// fixed.
    fn place(&mut self, eye: Vec3, yaw: f32, pitch: f32) {
        if !self.pos_given {
            self.camera.eye = eye;
        }
        if !self.look_given {
            (self.camera.yaw, self.camera.pitch) = (yaw, pitch);
        }
    }
}

/// What loading a world in the background reports.
enum Loading {
    /// What it's doing, shown on the loading screen.
    Status(String),
    /// The world, with its server when it's a singleplayer world.
    Done(Box<Result<(Scene, Option<local::LocalServer>)>>),
}

/// `WorldSelectionList.WorldListEntry`'s line under the name: the folder, and when the
/// world was last played (in UTC, as `yyyy-MM-dd HH:mm`).
fn folder_and_date(world: &local::World) -> String {
    let secs = world
        .last_played
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    // Howard Hinnant's `civil_from_days`.
    let z = secs.div_euclid(86400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    let time = secs.rem_euclid(86400);
    format!(
        "{} ({year:04}-{month:02}-{day:02} {:02}:{:02})",
        world.folder(),
        time / 3600,
        time / 60 % 60
    )
}

/// What the mouse is dragging on an options screen.
#[derive(Clone, Copy)]
enum Drag {
    Slider(options::Action),
    /// The scroll bar, from this height in GUI pixels.
    Scrollbar(f32),
}

struct WindowState {
    window: Arc<winit::window::Window>,
    /// The ways the surface can present, for VSync on or off.
    present_modes: Vec<wgpu::PresentMode>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

struct App {
    /// The arguments the game started with, which each world starts from.
    start: Args,
    args: Args,
    /// The models the world in play was baked with, or the menus' when there's none.
    models: Arc<BlockModels>,
    /// The models the menus draw with.
    menu_models: Arc<BlockModels>,
    /// The world in play, if any.
    streamer: Option<stream::Streamer>,
    /// On a server the player walks with vanilla's physics; on a save the camera flies.
    walker: Option<walk::Walker>,
    /// `Hud.vignetteBrightness`, kept from one world to the next as vanilla's `Hud` lives
    /// for the whole game: dark at launch, then wherever the last world left it.
    vignette: f32,
    /// The menu screen showing, when not playing.
    menu: Option<menu::Menu>,
    /// A world loading or a server being joined, in the background.
    loading: Option<std::sync::mpsc::Receiver<Loading>>,
    /// The singleplayer world's server, while it's in play or being joined.
    local_server: Option<local::LocalServer>,
    /// The world being started or played is a singleplayer world.
    singleplayer: bool,
    /// Select World's last click on a row, for a double click to play it.
    last_row_click: Option<(usize, Instant)>,
    /// When the menu's text box was last focused or edited, which restarts its blink.
    focused_at: Instant,
    /// The pause screen is open over the world.
    paused: bool,
    /// The pause screen's focused button.
    pause_focus: Option<mc_render::hud::pause::Action>,
    /// `Minecraft.lastInputType`: the mouse, Tab or an arrow key, which decides whether a
    /// screen opens with a widget focused and whether a slider takes the arrows.
    input_type: Option<InputType>,
    /// The chat on a server, and its screen when open.
    chat: chat::Chat,
    /// Shift, Ctrl and the like, held now.
    modifiers: winit::keyboard::ModifiersState,
    view: View,
    state: Option<WindowState>,
    /// The keys and mouse buttons held in the game.
    keys: std::collections::HashSet<Key>,
    /// Which key each key mapping is on.
    bindings: keys::Bindings,
    /// Sneak and Sprint, held or toggled.
    toggles: keys::Toggles,
    /// The block the outline is drawn around, and its state.
    outlined: Option<(glam::IVec3, u32)>,
    /// What the hotbar and the cracks last showed.
    shown_hotbar: Option<mc_render::hud::Hotbar>,
    shown_cracks: Vec<([i32; 3], u32, u8)>,
    last_frame: Instant,
    /// The sound engine and the music.
    sounds: sound::Sounds,
    /// The options screens, open over the title screen or the pause screen.
    options: Option<options::Options>,
    /// `Options.fov`, in degrees.
    fov: i32,
    /// The slider or scroll bar being dragged on an options screen.
    drag: Option<Drag>,
    /// The video options.
    video: options::Video,
    mouse: options::Mouse,
    /// `ScrollWheelHandler`'s scrolling not yet a whole notch.
    wheel: f64,
    /// When the player last pressed a key or moved or clicked the mouse
    /// (`FramerateLimitTracker.latestInputTime`), and when the last frame started.
    last_input: Instant,
    frame_due: Instant,
    /// When the game started, which scrolls labels too wide for their buttons.
    started: Instant,
    /// Time not yet ticked by the sounds, which tick 20 times a second with the game.
    sound_clock: f32,
    captured: bool,
    /// Where the mouse pointer is in the window, in pixels.
    cursor: Option<[f32; 2]>,
}

impl WindowState {
    /// Captures the mouse for looking around; false if the window can't.
    fn grab(&self) -> bool {
        let grab = self
            .window
            .set_cursor_grab(winit::window::CursorGrabMode::Locked)
            .or_else(|_| {
                self.window
                    .set_cursor_grab(winit::window::CursorGrabMode::Confined)
            });
        self.window.set_cursor_visible(grab.is_err());
        grab.is_ok()
    }

    /// Lets go of the mouse.
    fn release(&self) {
        let _ = self
            .window
            .set_cursor_grab(winit::window::CursorGrabMode::None);
        self.window.set_cursor_visible(true);
    }

    /// The GUI's size and the pointer's place in it, in GUI pixels.
    fn gui(&self, cursor: Option<[f32; 2]>) -> (i32, i32, Option<[f32; 2]>) {
        let (w, h) = (self.config.width, self.config.height);
        let scale = mc_render::gpu::gui_scale(w, h);
        let mouse = cursor.map(|c| c.map(|v| v / scale as f32));
        (w.div_ceil(scale) as i32, h.div_ceil(scale) as i32, mouse)
    }
}

impl App {
    fn create_window(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) -> Result<()> {
        let attrs = winit::window::Window::default_attributes()
            .with_title("minecraft-cardboard 0.0.1")
            .with_inner_size(winit::dpi::PhysicalSize::new(
                self.args.size.0,
                self.args.size.1,
            ));
        let window = Arc::new(event_loop.create_window(attrs)?);
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
        let caps = surface.get_capabilities(&adapter);
        // Vanilla draws into a plain framebuffer; prefer a non-sRGB format so colours match.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: present_mode(&caps.present_modes, self.video.vsync),
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        let renderer = Renderer::new(device, queue, format, &self.models);
        self.state = Some(WindowState {
            window,
            present_modes: caps.present_modes.clone(),
            surface,
            config,
            renderer,
        });
        Ok(())
    }

    /// Starts loading the test world (or `--world`), or joining `server`, in the
    /// background, showing the loading screen meanwhile.
    fn play(&mut self, server: Option<String>) {
        let mut args = self.start.clone();
        let key = if server.is_some() {
            "connect.connecting"
        } else {
            "connect.joining"
        };
        args.server = server;
        (self.args, self.singleplayer) = (args.clone(), false);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(Loading::Done(Box::new(
                load_scene(&args).map(|scene| (scene, None)),
            )));
        });
        self.loading = Some(rx);
        let menu = self.menu.get_or_insert_with(Default::default);
        menu.screen = menu::Screen::Connecting(key.into());
    }

    /// Starts the singleplayer world in `dir`: its server (downloading Mojang's Java and
    /// server first if need be), then joins it, showing what's happening meanwhile.
    fn play_local(&mut self, dir: PathBuf) {
        let mut args = self.start.clone();
        args.world_given = false;
        (self.args, self.singleplayer) = (args.clone(), true);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // The loading screen closing (Cancel) drops the receiver, which stops this.
            let status = |message: String| {
                tx.send(Loading::Status(message))
                    .map_err(|_| anyhow::anyhow!("cancelled"))
            };
            let result = (|| {
                let server = local::start(&dir, &args.name, &status)?;
                status("Joining world".into())?;
                args.server = Some(format!("127.0.0.1:{}", server.port));
                Ok((load_scene(&args)?, Some(server)))
            })();
            let _ = tx.send(Loading::Done(Box::new(result)));
        });
        self.loading = Some(rx);
        let menu = self.menu.get_or_insert_with(Default::default);
        menu.screen = menu::Screen::Connecting("Checking Minecraft's files".into());
    }

    /// Select World's list, read again from `saves/`.
    fn world_rows(worlds: &[local::World]) -> Vec<menu::WorldRow> {
        worlds
            .iter()
            .map(|w| menu::WorldRow {
                name: w.settings.name.clone(),
                folder: w
                    .dir
                    .file_name()
                    .map_or(String::new(), |f| f.to_string_lossy().into()),
                folder_and_date: folder_and_date(w),
                mode: menu::GameMode::from_name(&w.settings.game_mode),
                commands: w.settings.commands,
                version: w.version.clone(),
            })
            .collect()
    }

    /// Singleplayer: Select World, or, with no worlds yet, Create New World, as
    /// `CreateWorldScreen.openFresh` opens it then.
    fn open_worlds(&mut self) {
        let rows = Self::world_rows(&local::worlds());
        let Some(m) = &mut self.menu else { return };
        m.screen = if rows.is_empty() {
            m.new_world = Default::default();
            self.focused_at = Instant::now();
            menu::Screen::CreateWorld
        } else {
            menu::Screen::SelectWorld
        };
        (m.worlds, m.selected, m.search) = (rows, None, Default::default());
    }

    /// The world finished loading: draw it with a renderer for its models.
    fn enter_world(&mut self, scene: Scene, server: Option<local::LocalServer>) -> Result<()> {
        self.local_server = server;
        // `Minecraft.updateLevelInEngines`: a new level stops every sound.
        self.sounds.stop_all();
        let state = self.state.as_mut().context("no window")?;
        if let Some(walker) = &self.walker {
            self.vignette = walker.vignette();
        }
        self.walker = match scene.outbox.clone() {
            // An offline-mode server, which is all `connect` joins, gives the player the
            // offline UUID for their name.
            Some(outbox) => Some(walk::Walker::new(
                outbox,
                &scene.registry,
                scene.biomes.clone(),
                mc_protocol::offline_uuid(&self.args.name),
                self.vignette,
                scene.models.hud.items.clone(),
            )?),
            None => None,
        };
        state.renderer = Renderer::new(
            state.renderer.device.clone(),
            state.renderer.queue.clone(),
            state.config.format,
            &scene.models,
        );
        self.models = scene.models.clone();
        self.streamer = Some(spawn_streamer(scene, &self.args));
        (self.menu, self.paused, self.options) = (None, false, None);
        self.chat = Default::default();
        (self.outlined, self.shown_hotbar) = (None, None);
        self.shown_cracks.clear();
        self.view.ticks = self.args.ticks;
        // The walker's options.
        self.apply_video(Some(self.video));
        Ok(())
    }

    /// Leaves the world in play (closing the server connection) for this menu screen.
    fn leave_world(&mut self, screen: menu::Screen) {
        if let Some(walker) = &self.walker {
            walker.leave();
            self.vignette = walker.vignette();
        }
        (self.walker, self.streamer, self.loading) = (None, None, None);
        // `Minecraft.disconnect` stops every sound.
        self.sounds.stop_all();
        // Stops the world's server, which saves the world.
        self.local_server = None;
        self.args.camera.bob = glam::Mat4::IDENTITY;
        if let Some(state) = &mut self.state {
            state.release();
            state.renderer = Renderer::new(
                state.renderer.device.clone(),
                state.renderer.queue.clone(),
                state.config.format,
                &self.menu_models,
            );
        }
        self.models = self.menu_models.clone();
        (self.captured, self.paused) = (false, false);
        (self.options, self.drag) = (None, None);
        self.keys.clear();
        self.toggles.release();
        self.chat = Default::default();
        self.menu = Some(menu::Menu {
            screen,
            ..Default::default()
        });
    }

    /// Where a menu goes back to: Direct Connection from joining a server, Select World
    /// from creating or starting a singleplayer world (the title screen when there are no
    /// worlds), and the title screen from everything else.
    fn back_screen(&self) -> menu::Screen {
        let worlds = || match local::worlds().is_empty() {
            true => menu::Screen::Title,
            false => menu::Screen::SelectWorld,
        };
        match self.menu.as_ref().map(|m| &m.screen) {
            Some(menu::Screen::DirectConnect | menu::Screen::SelectWorld | menu::Screen::Title)
            | None => menu::Screen::Title,
            Some(menu::Screen::CreateWorld) => worlds(),
            Some(_) if self.singleplayer => worlds(),
            Some(_) if self.args.server.is_some() => menu::Screen::DirectConnect,
            Some(_) => menu::Screen::Title,
        }
    }

    fn menu_action(
        &mut self,
        action: menu::Action,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        let back = self.back_screen();
        let Some(m) = &mut self.menu else { return };
        let before = m.screen.clone();
        match action {
            menu::Action::Singleplayer => self.open_worlds(),
            menu::Action::OpenCreateWorld => {
                (m.screen, m.new_world) = (menu::Screen::CreateWorld, Default::default());
                self.focused_at = Instant::now();
            }
            menu::Action::PlaySelected => {
                let worlds = local::worlds();
                if let Some(world) = m.selected.and_then(|i| worlds.get(i)) {
                    self.play_local(world.dir.clone());
                }
            }
            menu::Action::Create => {
                let new = &m.new_world;
                let settings = local::NewWorld {
                    name: new.name.value.clone(),
                    game_mode: new.mode.name().into(),
                    difficulty: new.difficulty().into(),
                    commands: new.commands(),
                    seed: new.seed.value.clone(),
                    structures: new.structures,
                };
                match local::create(&settings) {
                    Ok(dir) => self.play_local(dir),
                    Err(e) => {
                        m.screen = menu::Screen::Disconnected {
                            title: "selectWorld.create",
                            reason: format!("{e:#}"),
                        }
                    }
                }
            }
            menu::Action::CycleGameMode
            | menu::Action::CycleDifficulty
            | menu::Action::ToggleCommands
            | menu::Action::ToggleStructures => m.cycle(action),
            menu::Action::Unavailable => {}
            menu::Action::Multiplayer => {
                m.screen = menu::Screen::DirectConnect;
                m.address = menu::Field::new(&last_server());
                self.focused_at = Instant::now();
            }
            menu::Action::Options => self.open_options(false),
            menu::Action::Quit => event_loop.exit(),
            menu::Action::Join => {
                let address = m.address.value.trim().to_owned();
                if menu::valid_address(&address) {
                    save_last_server(&address);
                    self.play(Some(address));
                }
            }
            menu::Action::Back => {
                // Cancelling a world's start stops its server too.
                (self.loading, self.local_server) = (None, None);
                let screen = back;
                if screen == menu::Screen::DirectConnect {
                    m.address = menu::Field::new(&last_server());
                }
                if screen == menu::Screen::SelectWorld {
                    (m.worlds, m.selected) = (Self::world_rows(&local::worlds()), None);
                }
                m.screen = screen;
            }
        }
        // `Gui.setScreen` clears the focus of the screen it leaves, and a screen gone back
        // to has none; a new one takes its initial focus. A button that leaves the screen
        // as it is keeps the focus.
        let Some(m) = &mut self.menu else { return };
        match action {
            menu::Action::Back | menu::Action::Options => m.focused = None,
            _ if m.screen != before => self.menu_initial_focus(),
            _ => {}
        }
    }

    /// `Screen.setInitialFocus` for a menu screen just opened: Direct Connection's, Select
    /// World's and Create New World's box; on the others, the first widget if the keyboard
    /// was last used.
    fn menu_initial_focus(&mut self) {
        let (items, focuses) = self.menu_items();
        let keyboard = self.input_type.is_some_and(InputType::keyboard);
        let Some(m) = &mut self.menu else { return };
        m.focused = match m.screen {
            menu::Screen::DirectConnect | menu::Screen::SelectWorld | menu::Screen::CreateWorld => {
                Some(menu::Focus::Box)
            }
            _ if keyboard => nav::tab(&items, None, true).map(|i| focuses[i]),
            _ => None,
        };
        self.focused_at = Instant::now();
    }

    /// The menu screen's widgets that may take the focus, in tab order (`Screen.children`
    /// by `getTabOrderGroup`), and which each is.
    fn menu_items(&self) -> (Vec<nav::Item>, Vec<menu::Focus>) {
        let Some(m) = &self.menu else {
            return Default::default();
        };
        let [_, _, width, height] = self.gui_rect();
        let item = |rect, active| nav::Item {
            rect,
            active,
            row: None,
        };
        let mut items = Vec::new();
        match m.screen {
            // The tab bar, the tab's widgets row by row, then the footer's.
            menu::Screen::CreateWorld => {
                let tabs = menu::tab_rects(width);
                let bar = [tabs[0][0], 0, tabs[2][0] + tabs[2][2] - tabs[0][0], 24];
                items.push((item(bar, true), menu::Focus::Tabs));
            }
            // The search box, the list, then the buttons.
            menu::Screen::SelectWorld => {
                items.push((item(menu::search_rect(width), true), menu::Focus::Box));
                let worlds = !menu::shown_worlds(m, height).is_empty();
                let list = menu::list_rect(width, height);
                items.push((item(list, worlds), menu::Focus::List));
            }
            _ => {}
        }
        if m.screen != menu::Screen::SelectWorld
            && let Some(rect) = m.field_rect(width, height)
        {
            items.push((item(rect, true), menu::Focus::Box));
        }
        let buttons = menu::buttons(m, &self.menu_models.hud, width, height);
        let footer = (items.len() + buttons.len()).saturating_sub(2);
        items.extend(
            buttons
                .into_iter()
                .map(|(action, b, active)| (item(b.rect, active), menu::Focus::Button(action))),
        );
        if m.screen == menu::Screen::CreateWorld {
            items[1..footer].sort_by_key(|(item, _)| item.rect[1]);
        }
        items.into_iter().unzip()
    }

    /// `TabNavigationBar.selectTab`: Create New World shows tab `tab` with a click, and
    /// the focus leaves the widgets of the tab it hides.
    fn select_tab(&mut self, tab: usize) {
        use menu::{Action, Focus};
        let Some(m) = &mut self.menu else { return };
        if m.new_world.tab == tab {
            return;
        }
        m.new_world.tab = tab;
        if !matches!(
            m.focused,
            Some(Focus::Tabs | Focus::Button(Action::Create | Action::Back))
        ) {
            m.focused = None;
        }
        self.click_sound();
    }

    /// A click on a menu (`ContainerEventHandler.mouseClicked`): the widget clicked takes
    /// the focus, and a click on nothing leaves it where it is.
    fn menu_click(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let Some(state) = &self.state else { return };
        let (width, height, Some(mouse)) = state.gui(self.cursor) else {
            return;
        };
        let Some(m) = &mut self.menu else { return };
        let art = &self.menu_models.hud;
        let inside = |rect: [i32; 4]| {
            let [x, y, w, h] = rect.map(|c| c as f32);
            mouse[0] >= x && mouse[1] >= y && mouse[0] < x + w && mouse[1] < y + h
        };
        // A click on the box moves its cursor there.
        if let Some(rect) = m.field_rect(width, height)
            && inside(rect)
            && let Some(field) = m.field()
        {
            field.cursor = menu::clicked_position(art, field, rect, mouse[0]);
            m.focused = Some(menu::Focus::Box);
            self.focused_at = Instant::now();
            return;
        }
        match m.screen {
            menu::Screen::CreateWorld => {
                if let Some(tab) = menu::tab_rects(width).iter().position(|&r| inside(r)) {
                    m.focused = Some(menu::Focus::Tabs);
                    return self.select_tab(tab);
                }
            }
            // A click selects a world; a second within a quarter of a second plays it.
            menu::Screen::SelectWorld => {
                if let Some(row) = menu::world_at(m, width, height, mouse) {
                    (m.selected, m.focused) = (Some(row), Some(menu::Focus::List));
                    let double = self
                        .last_row_click
                        .is_some_and(|(r, at)| r == row && at.elapsed().as_millis() < 250);
                    self.last_row_click = Some((row, Instant::now()));
                    if double {
                        self.menu_action(menu::Action::PlaySelected, event_loop);
                    }
                    return;
                }
            }
            _ => {}
        }
        let action = menu::buttons(m, art, width, height)
            .into_iter()
            .find(|(_, button, active)| *active && button.contains(mouse))
            .map(|(action, ..)| action);
        if let Some(action) = action {
            m.focused = Some(menu::Focus::Button(action));
            self.click_sound();
            self.menu_action(action, event_loop);
        }
    }

    /// `AbstractWidget.playDownSound`: a button pressed.
    fn click_sound(&mut self) {
        let seed = self.sounds.seed();
        self.sounds
            .play(sound::Instance::ui("minecraft:ui.button.click", 1.0, seed));
    }

    /// The sounds the world queued, the listener at the camera, and the sound engine and
    /// music ticked for the time that's passed (`Minecraft.tick`'s `musicManager.tick` and
    /// `soundManager.tick`).
    fn tick_sounds(&mut self, dt: f32) {
        if let Some(walker) = &mut self.walker {
            for instance in walker.take_sounds() {
                self.sounds.play(instance);
            }
        }
        if self.streamer.is_some() {
            let camera = &self.args.camera;
            let look = |pitch: f32| mc_physics::view_vector(camera.yaw, pitch).as_vec3();
            self.sounds
                .listener(camera.eye, look(camera.pitch), look(camera.pitch - 90.0));
        }
        self.sound_clock += dt;
        while self.sound_clock >= 0.05 {
            self.sound_clock -= 0.05;
            // `getSituationalMusic`: the world's music in a world, the menu's out of one.
            let track = match &self.walker {
                Some(walker) => walker.music(),
                None if self.streamer.is_some() => None,
                None => Some(sound::Track::menu()),
            };
            self.sounds
                .tick_music(track.as_ref(), 1.0, self.loading.is_some());
            let walker = self.walker.as_ref();
            self.sounds
                .tick(&|id| walker.and_then(|w| w.entity_position(id)));
        }
    }

    /// A key on a menu (`Screen.keyPressed`): Esc goes back from the screens with a box,
    /// the focused box edits as `EditBox` does, Enter or Space presses the focused button,
    /// and Tab and the arrows move the focus.
    fn menu_key(
        &mut self,
        event: &winit::event::KeyEvent,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        use menu::{Action, Focus};
        use winit::keyboard::{KeyCode as K, PhysicalKey};
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        let (items, focuses) = self.menu_items();
        let height = self.gui_rect()[3];
        let control = self.modifiers.control_key();
        let back_tab = self.modifiers.shift_key();
        let Some(m) = &mut self.menu else { return };
        let screen = m.screen.clone();
        let has_box = matches!(
            screen,
            menu::Screen::DirectConnect | menu::Screen::SelectWorld | menu::Screen::CreateWorld
        );
        if has_box && code == K::Escape {
            return self.menu_action(Action::Back, event_loop);
        }
        // `TabNavigationBar.keyPressed`: Ctrl+Tab shows the next tab (with Shift, the one
        // before), and Ctrl and a digit that tab, the last for any past it.
        if screen == menu::Screen::CreateWorld && control {
            let tab = m.new_world.tab;
            let next = match code {
                K::Tab if back_tab => Some((tab + 2) % 3),
                K::Tab => Some((tab + 1) % 3),
                _ => digit(code).map(|n| ((n + 9) % 10).min(2)),
            };
            if let Some(next) = next {
                return self.select_tab(next);
            }
        }
        if m.focused == Some(Focus::Box) {
            let join = screen == menu::Screen::DirectConnect
                && confirmation(code)
                && menu::valid_address(&m.address.value);
            let Some(field) = m.field() else { return };
            let max = if screen == menu::Screen::DirectConnect {
                128
            } else {
                32
            };
            let typed = event
                .text
                .as_deref()
                .filter(|t| t.chars().all(|c| c >= ' '));
            match (code, typed) {
                (K::Backspace, _) => field.backspace(),
                (K::Delete, _) => field.delete(),
                (K::ArrowLeft, _) => field.step(-1),
                (K::ArrowRight, _) => field.step(1),
                (K::Home, _) => field.home(),
                (K::End, _) => field.end(),
                (_, Some(text)) if !text.is_empty() => field.insert(text, max),
                // `DirectJoinServerScreen.keyPressed`: Enter in the box joins.
                _ if join => return self.menu_action(Action::Join, event_loop),
                _ if screen == menu::Screen::CreateWorld && confirmation(code) => {
                    return self.menu_action(Action::Create, event_loop);
                }
                _ => return self.menu_navigate(code),
            }
            // `WorldSelectionList.fillLevels`: the selection goes if the search hides it.
            let shown = menu::shown_worlds(m, height);
            m.selected = m.selected.filter(|s| shown.contains(s));
            self.focused_at = Instant::now();
            return;
        }
        let focused = focuses
            .iter()
            .zip(&items)
            .find(|(f, item)| item.active && Some(**f) == m.focused)
            .map(|(f, _)| *f);
        if selection(code) {
            match focused {
                Some(Focus::Button(action)) => {
                    self.click_sound();
                    return self.menu_action(action, event_loop);
                }
                // `WorldListEntry.keyPressed`: plays the world.
                Some(Focus::List) if m.selected.is_some() => {
                    self.click_sound();
                    return self.menu_action(Action::PlaySelected, event_loop);
                }
                _ => {}
            }
        }
        // `CreateWorldScreen.keyPressed`: Enter that nothing else took creates the world.
        if screen == menu::Screen::CreateWorld && confirmation(code) {
            return self.menu_action(Action::Create, event_loop);
        }
        self.menu_navigate(code);
    }

    /// Tab or an arrow on a menu: first inside the focused list or tab bar, then from
    /// widget to widget.
    fn menu_navigate(&mut self, code: winit::keyboard::KeyCode) {
        use menu::Focus;
        use winit::keyboard::KeyCode as K;
        let screen = self.gui_rect();
        let back_tab = code == K::Tab && self.modifiers.shift_key();
        let Some(m) = &mut self.menu else { return };
        let shown = menu::shown_worlds(m, screen[3]);
        match (m.focused, code) {
            // `ObjectSelectionList.nextFocusPath`: up or down the list from the selected
            // world (from none, the first or the last); off its end, or with Left or
            // Right, the list loses its selection and the focus moves on from it.
            (Some(Focus::List), K::ArrowUp | K::ArrowDown | K::ArrowLeft | K::ArrowRight) => {
                let at = m.selected.and_then(|s| shown.iter().position(|&i| i == s));
                let next = match (code, at) {
                    (K::ArrowUp, Some(a)) => a.checked_sub(1),
                    (K::ArrowDown, Some(a)) => Some(a + 1),
                    (K::ArrowUp, None) => shown.len().checked_sub(1),
                    (K::ArrowDown, None) => Some(0),
                    _ => None,
                };
                m.selected = next.and_then(|i| shown.get(i)).copied();
                if m.selected.is_some() {
                    self.input_type = Some(InputType::Arrow);
                    return;
                }
            }
            // The tab bar's own arrow navigation: focusing a tab shows it.
            (Some(Focus::Tabs), K::ArrowLeft | K::ArrowRight) => {
                let step = if code == K::ArrowLeft { -1 } else { 1 };
                let tab = m.new_world.tab as i32 + step;
                if (0..3).contains(&tab) {
                    self.input_type = Some(InputType::Arrow);
                    return self.select_tab(tab as usize);
                }
            }
            _ => {}
        }
        let (items, focuses) = self.menu_items();
        let Some(m) = &self.menu else { return };
        let focused = focuses
            .iter()
            .zip(&items)
            .position(|(f, item)| item.active && Some(*f) == m.focused);
        let Some(next) = self.navigate(code, &items, focused, screen, [0; 4]) else {
            return;
        };
        let Some(m) = &mut self.menu else { return };
        m.focused = Some(focuses[next]);
        match focuses[next] {
            // Into the list: its selected world, or the first (the last going up).
            Focus::List if !m.selected.is_some_and(|s| shown.contains(&s)) => {
                let up = code == K::ArrowUp || back_tab;
                m.selected = if up { shown.last() } else { shown.first() }.copied();
            }
            Focus::Box => self.focused_at = Instant::now(),
            _ => {}
        }
    }

    /// `Minecraft.pauseGame`: opens the pause screen over the world in play, letting go of
    /// the mouse. Not over another screen.
    fn pause(&mut self) {
        let screen_open = self.walker.as_ref().is_some_and(walk::Walker::screen_open);
        if self.streamer.is_none()
            || self.menu.is_some()
            || self.paused
            || screen_open
            || self.chat.is_open()
            || self.inventory_open()
        {
            return;
        }
        if let Some(state) = &self.state {
            state.release();
        }
        (self.paused, self.captured) = (true, false);
        self.pause_focus = None;
        let items = self.pause_items().0;
        if self.input_type.is_some_and(InputType::keyboard)
            && let Some(i) = nav::tab(&items, None, true)
        {
            self.pause_focus = Some(self.pause_items().1[i]);
        }
        self.keys.clear();
        self.toggles.release();
    }

    /// Back to Game: closes the pause screen and captures the mouse again.
    fn resume(&mut self) {
        self.paused = false;
        if let Some(state) = &self.state {
            self.captured = state.grab();
            self.toggles.restore(&self.bindings);
        }
    }

    fn pause_click(&mut self) {
        let Some(state) = &self.state else { return };
        let (width, height, Some(mouse)) = state.gui(self.cursor) else {
            return;
        };
        let pause = mc_render::hud::pause::Pause {
            local: self.walker.is_none() || self.singleplayer,
            mouse: None,
            focused: None,
        };
        let action = mc_render::hud::pause::buttons(&pause, width, height)
            .into_iter()
            .find(|(_, button, active, _)| *active && button.contains(mouse))
            .map(|(action, ..)| action);
        if let Some(action) = action {
            self.pause_focus = Some(action);
            self.pause_activate(action);
        }
    }

    /// A pause screen button pressed, by the mouse or the keyboard.
    fn pause_activate(&mut self, action: mc_render::hud::pause::Action) {
        self.click_sound();
        match Some(action) {
            Some(mc_render::hud::pause::Action::ReturnToGame) => self.resume(),
            Some(mc_render::hud::pause::Action::Options) => self.open_options(true),
            Some(mc_render::hud::pause::Action::Leave) => self.leave_world(menu::Screen::Title),
            _ => {}
        }
    }

    /// The pause screen's buttons for navigation, and what each does.
    fn pause_items(&self) -> (Vec<nav::Item>, Vec<mc_render::hud::pause::Action>) {
        let Some(state) = &self.state else {
            return Default::default();
        };
        let (width, height, _) = state.gui(None);
        let pause = mc_render::hud::pause::Pause {
            local: self.walker.is_none() || self.singleplayer,
            mouse: None,
            focused: None,
        };
        mc_render::hud::pause::buttons(&pause, width, height)
            .into_iter()
            .map(|(action, b, active, _)| {
                let item = nav::Item {
                    rect: b.rect,
                    active,
                    row: None,
                };
                (item, action)
            })
            .unzip()
    }

    /// A key on the pause screen: Enter or Space presses the focused button, Tab and the
    /// arrows move the focus.
    fn pause_key(&mut self, code: winit::keyboard::KeyCode) {
        let (items, actions) = self.pause_items();
        let focused = actions
            .iter()
            .zip(&items)
            .position(|(a, item)| item.active && Some(*a) == self.pause_focus);
        if selection(code) {
            if let Some(f) = focused {
                self.pause_activate(actions[f]);
            }
            return;
        }
        let screen = self.gui_rect();
        if let Some(next) = self.navigate(code, &items, focused, screen, [0; 4]) {
            self.pause_focus = Some(actions[next]);
        }
    }

    /// A key on the death screen or its confirmation: Enter or Space presses the focused
    /// button once the buttons work, Tab and the arrows move the focus.
    fn death_key(&mut self, code: winit::keyboard::KeyCode) {
        let screen = self.gui_rect();
        let Some(death) = self.walker.as_ref().and_then(walk::Walker::death_screen) else {
            return;
        };
        let items: Vec<nav::Item> = mc_render::hud::death_buttons(&death, screen[2], screen[3])
            .iter()
            .map(|b| nav::Item {
                rect: b.rect,
                active: death.active,
                row: None,
            })
            .collect();
        if selection(code) {
            let Some(walker) = &mut self.walker else {
                return;
            };
            if let Some(f) = death.focused
                && walker.press_screen(f) == walk::ScreenAction::Quit
            {
                return self.leave_world(menu::Screen::Title);
            }
            // `TitleConfirmScreen` opened by keyboard: `setInitialFocus` focuses its first
            // button before `setDelay` turns them off.
            if walker
                .death_screen()
                .is_some_and(|d| d.confirm && d.focused.is_none())
            {
                walker.focus_screen(Some(0));
            }
            return;
        }
        if let Some(next) = self.navigate(code, &items, death.focused, screen, [0; 4])
            && let Some(walker) = &mut self.walker
        {
            walker.focus_screen(Some(next));
        }
    }

    /// The GUI's rectangle, in GUI pixels.
    fn gui_rect(&self) -> [i32; 4] {
        let (w, h) = self.state.as_ref().map_or((0, 0), |s| {
            let (w, h, _) = s.gui(None);
            (w, h)
        });
        [0, 0, w, h]
    }

    /// `Screen.keyPressed`'s navigation: Tab (Shift+Tab back) or an arrow from the item
    /// `focused` among `items`, noting the keyboard as the last input.
    fn navigate(
        &mut self,
        code: winit::keyboard::KeyCode,
        items: &[nav::Item],
        focused: Option<usize>,
        screen: [i32; 4],
        list: [i32; 4],
    ) -> Option<usize> {
        use winit::keyboard::KeyCode as K;
        let direction = match code {
            K::Tab => {
                self.input_type = Some(InputType::Tab);
                return nav::tab(items, focused, !self.modifiers.shift_key());
            }
            K::ArrowUp => nav::Direction::Up,
            K::ArrowDown => nav::Direction::Down,
            K::ArrowLeft => nav::Direction::Left,
            K::ArrowRight => nav::Direction::Right,
            _ => return None,
        };
        self.input_type = Some(InputType::Arrow);
        nav::arrow(items, focused, direction, screen, list)
    }

    /// `Options.load` from `options.txt`: the FOV (saved as `(fov - 70) / 40`), the sound
    /// volumes and the video options. `--fov` and `--distance` win over what's saved.
    fn load_options(&mut self) {
        let number = |name: &str| option(name)?.trim().parse::<f64>().ok();
        let flag = |name: &str| option(name).map(|v| v.trim() == "true");
        let text = |name: &str| option(name).map(|v| v.trim().trim_matches('"').to_owned());
        if let Some(v) = number("fov") {
            self.fov = ((v * 40.0 + 70.0) as i32).clamp(30, 110);
        }
        if self.start.camera.fov == 70.0 {
            self.set_fov(self.fov);
        } else {
            self.fov = self.start.camera.fov as i32;
        }
        for (i, name) in options::CATEGORIES.iter().enumerate() {
            if let Some(v) = number(&format!("soundCategory_{name}")) {
                self.sounds
                    .set_volume(sound::Category::from_id(i as i32), v.clamp(0.0, 1.0));
            }
        }
        let v = &mut self.video;
        let int = |name: &str| number(name).map(|n| n as i32);
        v.framerate = int("maxFps").map_or(v.framerate, |n| n.clamp(10, 260));
        v.vsync = flag("enableVsync").unwrap_or(v.vsync);
        v.afk_limit = text("inactivityFpsLimit").is_none_or(|t| t != "minimized");
        v.gui_scale = int("guiScale").map_or(v.gui_scale, |n| n.max(0));
        v.fullscreen = flag("fullscreen").unwrap_or(v.fullscreen);
        v.gamma = number("gamma").map_or(v.gamma, |g| g.clamp(0.0, 1.0));
        v.render_distance = match self.start.distance_given {
            true => self.start.distance,
            false => int("renderDistance").map_or(v.render_distance, |n| n.clamp(2, 32)),
        };
        v.clouds = match text("renderClouds").as_deref() {
            Some("false") => options::Clouds::Off,
            Some("fast") => options::Clouds::Fast,
            _ => options::Clouds::Fancy,
        };
        v.particles = match int("particles") {
            Some(1) => options::Particles::Decreased,
            Some(2) => options::Particles::Minimal,
            _ => options::Particles::All,
        };
        v.cloud_range = int("cloudRange").map_or(v.cloud_range, |n| n.clamp(2, 128));
        v.vignette = flag("vignette").unwrap_or(v.vignette);
        v.attack_indicator = match int("attackIndicator") {
            Some(0) => options::AttackIndicator::Off,
            Some(2) => options::AttackIndicator::Hotbar,
            _ => options::AttackIndicator::Crosshair,
        };
        let m = &mut self.mouse;
        m.sensitivity = number("mouseSensitivity").map_or(m.sensitivity, |n| n.clamp(0.0, 1.0));
        m.wheel = number("mouseWheelSensitivity").map_or(m.wheel, |n| n.clamp(0.01, 10.0));
        m.discrete_scroll = flag("discrete_mouse_scroll").unwrap_or(m.discrete_scroll);
        m.invert_x = flag("invertXMouse").unwrap_or(m.invert_x);
        m.invert_y = flag("invertYMouse").unwrap_or(m.invert_y);
        self.toggles.sneak.toggle = flag("toggleCrouch").unwrap_or(false);
        self.toggles.sprint.toggle = flag("toggleSprint").unwrap_or(false);
        self.toggles.attack.toggle = flag("toggleAttack").unwrap_or(false);
        self.toggles.use_item.toggle = flag("toggleUse").unwrap_or(false);
        self.bindings = keys::Bindings::load(option);
        self.apply_video(None);
    }

    /// `Options.save`, for the options these screens change, written as vanilla writes
    /// them (Java's `Double.toString`: "1.0", not "1"); in a world, `broadcastOptions`
    /// tells the server the view distance.
    fn save_options(&self) {
        let fov = (self.fov as f64 - 70.0) / 40.0;
        let mut saved = vec![("fov".to_owned(), format!("{fov:?}"))];
        for (i, name) in options::CATEGORIES.iter().enumerate() {
            let volume = self.sounds.volumes[i];
            saved.push((format!("soundCategory_{name}"), format!("{volume:?}")));
        }
        let v = &self.video;
        let clouds = match v.clouds {
            options::Clouds::Off => "\"false\"",
            options::Clouds::Fast => "\"fast\"",
            options::Clouds::Fancy => "\"true\"",
        };
        let inactivity = if v.afk_limit {
            "\"afk\""
        } else {
            "\"minimized\""
        };
        for (name, value) in [
            ("maxFps", v.framerate.to_string()),
            ("enableVsync", v.vsync.to_string()),
            ("inactivityFpsLimit", inactivity.into()),
            ("guiScale", v.gui_scale.to_string()),
            ("fullscreen", v.fullscreen.to_string()),
            ("gamma", format!("{:?}", v.gamma)),
            ("renderDistance", v.render_distance.to_string()),
            ("renderClouds", clouds.into()),
            ("particles", (v.particles as i32).to_string()),
            ("cloudRange", v.cloud_range.to_string()),
            ("vignette", v.vignette.to_string()),
            ("attackIndicator", (v.attack_indicator as i32).to_string()),
        ] {
            saved.push((name.to_owned(), value));
        }
        let m = &self.mouse;
        for (name, value) in [
            ("mouseSensitivity", format!("{:?}", m.sensitivity)),
            ("mouseWheelSensitivity", format!("{:?}", m.wheel)),
            ("discrete_mouse_scroll", m.discrete_scroll.to_string()),
            ("invertXMouse", m.invert_x.to_string()),
            ("invertYMouse", m.invert_y.to_string()),
            ("toggleCrouch", self.toggles.sneak.toggle.to_string()),
            ("toggleSprint", self.toggles.sprint.toggle.to_string()),
            ("toggleAttack", self.toggles.attack.toggle.to_string()),
            ("toggleUse", self.toggles.use_item.toggle.to_string()),
        ] {
            saved.push((name.to_owned(), value));
        }
        saved.extend(self.bindings.saved());
        save_options(&saved);
        if let Some(walker) = &self.walker {
            walker.send_view_distance(v.render_distance);
        }
    }

    fn set_fov(&mut self, fov: i32) {
        self.fov = fov;
        self.args.camera.fov = fov as f32;
        self.start.camera.fov = fov as f32;
    }

    /// The video options take effect: each that changed from `old`, or all of them.
    fn apply_video(&mut self, old: Option<options::Video>) {
        let v = self.video;
        let changed = |f: fn(&options::Video) -> bool| old.is_none_or(|o| f(&o) != f(&v));
        mc_render::gpu::set_gui_scale(v.gui_scale as u32);
        (self.view.gamma, self.view.vignette) = (v.gamma as f32, v.vignette);
        (self.view.clouds, self.view.cloud_range) = (v.clouds, v.cloud_range);
        if old.is_none_or(|o| o.render_distance != v.render_distance) {
            (self.args.distance, self.start.distance) = (v.render_distance, v.render_distance);
            self.view.distance = (v.render_distance * 16) as f32;
            if let Some(streamer) = &mut self.streamer {
                streamer.set_distance(v.render_distance);
            }
        }
        if let Some(walker) = &mut self.walker {
            walker.set_particle_status(v.particles);
            walker.attack_indicator_mode = v.attack_indicator;
        }
        if let Some(state) = &mut self.state {
            if changed(|v| v.vsync) {
                state.config.present_mode = present_mode(&state.present_modes, v.vsync);
                state
                    .surface
                    .configure(&state.renderer.device, &state.config);
            }
            // `Window.setFullscreen`: borderless on the monitor the window is on.
            if changed(|v| v.fullscreen) {
                let mode = v
                    .fullscreen
                    .then_some(winit::window::Fullscreen::Borderless(None));
                state.window.set_fullscreen(mode);
            }
        }
    }

    /// `FramerateLimitTracker.getFramerateLimit`: 10 a second minimized or after ten idle
    /// minutes, at most 30 after one (unless only minimized windows are limited), 60 on a
    /// menu out of a world, otherwise the Max Framerate option.
    fn framerate_limit(&self) -> i32 {
        let minimized = self
            .state
            .as_ref()
            .and_then(|s| s.window.is_minimized())
            .unwrap_or(false);
        let idle = self.last_input.elapsed().as_secs();
        if minimized || (self.video.afk_limit && idle > 600) {
            10
        } else if self.video.afk_limit && idle > 60 {
            self.video.framerate.min(30)
        } else if self.streamer.is_none() && (self.menu.is_some() || self.options.is_some()) {
            60
        } else {
            self.video.framerate
        }
    }

    /// `RenderSystem.limitDisplayFPS`: under the limit (260 being none), waits out the rest
    /// of the frame's time.
    fn limit_frame_rate(&mut self) {
        let limit = self.framerate_limit();
        if limit < 260 {
            let due = self.frame_due + std::time::Duration::from_secs_f64(1.0 / limit as f64);
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        self.frame_due = Instant::now();
    }

    /// Opens Options, from the title screen or, over the world, from the pause screen.
    fn open_options(&mut self, in_world: bool) {
        self.options = Some(options::Options {
            in_world,
            ..Default::default()
        });
        self.options_initial_focus();
    }

    /// `Screen.setInitialFocus`: with the keyboard last used, the first widget.
    fn options_initial_focus(&mut self) {
        if !self.input_type.is_some_and(InputType::keyboard) {
            return;
        }
        let Some(state) = &self.state else { return };
        let (width, height, _) = state.gui(None);
        let Some(o) = self.options_view() else { return };
        let widgets = options::widgets(&o, &self.menu_models.hud, width, height);
        let items: Vec<_> = widgets.iter().map(options::Widget::nav).collect();
        if let Some(i) = nav::tab(&items, None, true) {
            self.options_focus(&widgets[i], height);
        }
    }

    /// The focus moves to `w` (`AbstractSliderButton.setFocused`: a slider takes the arrows
    /// when focused by the mouse or Tab), and the list scrolls to show it.
    fn options_focus(&mut self, w: &options::Widget, height: i32) {
        let tab_or_mouse = matches!(self.input_type, Some(InputType::Mouse | InputType::Tab));
        let Some(o) = &mut self.options else { return };
        (o.focused, o.can_change) = (Some(w.action), tab_or_mouse);
        if let Some((_, y, h)) = w.entry {
            o.scroll = options::scroll_to(o.page, o.scroll, (y, h), height);
        }
    }

    /// A key on an options screen (`Screen.keyPressed`): the focused widget's first, Enter
    /// or Space pressing a button or making a slider take the arrows (or let go of them),
    /// and the left and right arrows moving a slider that takes them a pixel's worth; then
    /// Tab and the arrows move the focus.
    fn options_key(&mut self, code: winit::keyboard::KeyCode) {
        use winit::keyboard::KeyCode as K;
        let Some(state) = &self.state else { return };
        let (width, height, _) = state.gui(None);
        let Some(o) = self.options_view() else { return };
        let widgets = options::widgets(&o, &self.menu_models.hud, width, height);
        let focused = widgets
            .iter()
            .position(|w| w.active && Some(w.action) == o.focused);
        if let Some(w) = focused.map(|f| &widgets[f]) {
            match (code, w.slider) {
                (_, Some(_)) if selection(code) => {
                    if let Some(o) = &mut self.options {
                        o.can_change = !o.can_change;
                    }
                    return;
                }
                (_, None) if selection(code) => {
                    self.click_sound();
                    return self.options_activate(w.action);
                }
                (K::ArrowLeft | K::ArrowRight, Some(value)) if o.can_change => {
                    let sign = if code == K::ArrowLeft { -1.0 } else { 1.0 };
                    let step = sign / (w.button.rect[2] - 8) as f64;
                    return self.set_slider(w.action, (value + step).clamp(0.0, 1.0));
                }
                _ => {}
            }
        }
        let items: Vec<_> = widgets.iter().map(options::Widget::nav).collect();
        let (top, list_height) = options::list_area(height);
        let list = [0, top, width, list_height];
        if let Some(next) = self.navigate(code, &items, focused, [0, 0, width, height], list) {
            self.options_focus(&widgets[next], height);
        }
    }

    /// Opens another options page, keeping this one's focus for coming back.
    fn options_page(&mut self, page: options::Page) {
        if let Some(o) = &mut self.options {
            o.parent_focus.push(o.focused);
            (o.page, o.scroll, o.focused, o.dragging) = (page, 0.0, None, None);
        }
        self.options_initial_focus();
    }

    /// The options screen with the options as they are now.
    fn options_view(&self) -> Option<options::Options> {
        let mut o = self.options.clone()?;
        (o.fov, o.volumes, o.video) = (self.fov, self.sounds.volumes, self.video);
        o.mouse_options = self.mouse;
        o.toggle_sneak = self.toggles.sneak.toggle;
        o.toggle_sprint = self.toggles.sprint.toggle;
        o.toggle_attack = self.toggles.attack.toggle;
        o.toggle_use = self.toggles.use_item.toggle;
        if let Some(state) = &self.state {
            let (w, h) = (state.config.width, state.config.height);
            o.video.gui_scale_max = mc_render::gpu::calculate_scale(w, h, 0) as i32;
        }
        o.millis = self.started.elapsed().as_millis() as u64;
        o.keys = self.bindings.0.clone();
        Some(o)
    }

    /// `OptionsSubScreen.onClose` and `OptionsScreen.onClose`: Mouse Settings and Key Binds
    /// go back to Controls, the other pages to Options, and Options to the screen it opened from,
    /// saving the options (`removed`).
    fn options_back(&mut self) {
        let Some(o) = &mut self.options else { return };
        let back = match o.page {
            options::Page::KeyBinds | options::Page::Mouse => options::Page::Controls,
            _ => options::Page::Options,
        };
        match o.page {
            // Back on the pause screen, nothing is focused, or with the keyboard last used,
            // its first button (`Screen.setInitialFocus`).
            options::Page::Options => {
                self.options = None;
                self.pause_focus = None;
                if self.paused && self.input_type.is_some_and(InputType::keyboard) {
                    let (items, actions) = self.pause_items();
                    self.pause_focus = nav::tab(&items, None, true).map(|i| actions[i]);
                }
            }
            _ => {
                let focused = o.parent_focus.pop().flatten();
                (o.page, o.scroll, o.focused, o.selected) = (back, 0.0, focused, None);
                o.dragging = None;
            }
        }
        self.drag = None;
        self.save_options();
    }

    /// A slider moved to `value` (`AbstractSliderButton.setValue`): the FOV and the video
    /// options change in their steps; a volume changes the sounds playing and, out of a
    /// world, plays its category's preview.
    fn set_slider(&mut self, action: options::Action, value: f64) {
        let Some(o) = &mut self.options else { return };
        o.dragging = Some((action, value));
        match action {
            options::Action::Fov => {
                let fov = options::fov_from_slider(value);
                if fov != self.fov {
                    self.set_fov(fov);
                }
            }
            options::Action::Volume(i) => {
                if self.sounds.volumes[i] != value {
                    let category = sound::Category::from_id(i as i32);
                    self.sounds.set_volume(category, value);
                    if self.streamer.is_none() {
                        self.sounds.preview(category, value as f32);
                    }
                }
            }
            options::Action::Mouse(option) => self.mouse.set_slider(option, value),
            options::Action::Video(option) => {
                let old = self.video;
                self.video.set_slider(option, value);
                if self.video != old {
                    self.apply_video(Some(old));
                }
            }
            _ => {}
        }
    }

    fn options_click(&mut self) {
        let Some(state) = &self.state else { return };
        let (width, height, Some(mouse)) = state.gui(self.cursor) else {
            return;
        };
        let Some(o) = self.options_view() else { return };
        // `AbstractScrollArea.mouseClicked`: on the scroll bar, it drags.
        if let Some([x, y, w, h]) = options::scrollbar(o.page, width, height)
            && mouse[0] >= x as f32
            && mouse[0] <= (x + w) as f32
            && mouse[1] >= y as f32
            && mouse[1] < (y + h) as f32
        {
            self.drag = Some(Drag::Scrollbar(mouse[1]));
            return;
        }
        let (top, list_height) = options::list_area(height);
        let in_list = mouse[1] >= top as f32 && mouse[1] < (top + list_height) as f32;
        let hit = options::widgets(&o, &self.menu_models.hud, width, height)
            .into_iter()
            .find(|w| w.active && w.button.contains(mouse) && (!w.in_list || in_list));
        // `ContainerEventHandler.mouseClicked`: the widget clicked takes the focus; a click
        // on nothing leaves it.
        let Some(w) = hit else { return };
        self.options_focus(&w, height);
        if w.slider.is_some() {
            // `AbstractSliderButton.onClick`: the handle jumps to the mouse; the click
            // sounds when it's let go.
            self.drag = Some(Drag::Slider(w.action));
            self.set_slider(w.action, options::slider_at(w.button.rect, mouse[0]));
            return;
        }
        self.click_sound();
        self.options_activate(w.action);
    }

    /// A button on an options screen pressed, by the mouse or the keyboard.
    fn options_activate(&mut self, action: options::Action) {
        match action {
            options::Action::OpenSounds => self.options_page(options::Page::Sounds),
            options::Action::OpenVideo => self.options_page(options::Page::Video),
            options::Action::OpenControls => self.options_page(options::Page::Controls),
            options::Action::OpenMouse => self.options_page(options::Page::Mouse),
            options::Action::OpenKeyBinds => self.options_page(options::Page::KeyBinds),
            options::Action::Mouse(option) => self.mouse.cycle(option),
            options::Action::ToggleSneak => {
                self.toggles.sneak.toggle = !self.toggles.sneak.toggle;
            }
            options::Action::ToggleSprint => {
                self.toggles.sprint.toggle = !self.toggles.sprint.toggle;
            }
            options::Action::ToggleAttack => {
                self.toggles.attack.toggle = !self.toggles.attack.toggle;
            }
            options::Action::ToggleUse => {
                self.toggles.use_item.toggle = !self.toggles.use_item.toggle;
            }
            // `KeyBindsList.KeyEntry`'s buttons and `KeyBindsScreen`'s Reset Keys.
            options::Action::KeyChange(i) => {
                if let Some(o) = &mut self.options {
                    o.selected = Some(i);
                }
            }
            options::Action::KeyReset(i) => {
                self.bindings.0[i] = mc_render::hud::keys::MAPPINGS[i].default;
            }
            options::Action::ResetKeys => self.bindings = Default::default(),
            options::Action::Video(option) => {
                let old = self.video;
                self.video.cycle(option);
                self.apply_video(Some(old));
            }
            options::Action::Done => self.options_back(),
            _ => {}
        }
    }

    fn options_drag(&mut self) {
        let Some(state) = &self.state else { return };
        let (width, height, Some(mouse)) = state.gui(self.cursor) else {
            return;
        };
        let Some(o) = self.options_view() else { return };
        match self.drag {
            Some(Drag::Slider(action)) => {
                let rect = options::widgets(&o, &self.menu_models.hud, width, height)
                    .into_iter()
                    .find(|w| w.action == action)
                    .map(|w| w.button.rect);
                if let Some(rect) = rect {
                    self.set_slider(action, options::slider_at(rect, mouse[0]));
                }
            }
            // `AbstractScrollArea.mouseDragged`: past the list's ends, all the way.
            Some(Drag::Scrollbar(last)) => {
                let Some(o) = &mut self.options else { return };
                let (top, list_height) = options::list_area(height);
                let max = options::max_scroll(o.page, height);
                o.scroll = if mouse[1] < top as f32 {
                    0.0
                } else if mouse[1] > (top + list_height) as f32 {
                    max
                } else {
                    let dy = (mouse[1] - last) as f64;
                    let scale = options::scroll_drag_scale(o.page, height);
                    (o.scroll + dy * scale).clamp(0.0, max)
                };
                self.drag = Some(Drag::Scrollbar(mouse[1]));
            }
            None => {}
        }
    }

    /// `AbstractSliderButton.onRelease`: the click sounds, and the handle settles on the
    /// option's step.
    fn options_release(&mut self) {
        if let Some(Drag::Slider(_)) = self.drag.take() {
            self.click_sound();
            if let Some(o) = &mut self.options {
                o.dragging = None;
            }
        }
    }

    /// `AbstractScrollArea.mouseScrolled`: 12 GUI pixels a notch. With Ctrl held on Video
    /// Settings, the GUI scale steps instead, through the scales that fit, not to Auto
    /// (`VideoSettingsScreen.mouseScrolled`).
    /// `KeyBindsScreen.mouseClicked` and `keyPressed` while a mapping waits for a key: it
    /// takes the button or key, Esc unbinding it. Whether it was waiting.
    fn bind_key(&mut self, key: Key) -> bool {
        let Some(o) = &mut self.options else {
            return false;
        };
        let Some(i) = o.selected.take() else {
            return false;
        };
        self.bindings.0[i] = if key == keys::from_code(winit::keyboard::KeyCode::Escape) {
            mc_render::hud::keys::UNBOUND
        } else {
            key
        };
        true
    }

    fn options_scroll(&mut self, notches: f32) {
        let Some(state) = &self.state else { return };
        let height = state.gui(None).1;
        let Some(o) = self.options_view() else { return };
        if o.page == options::Page::Video && self.modifiers.control_key() {
            let max = o.video.gui_scale_max;
            let current = if o.video.gui_scale == 0 {
                max + 1
            } else {
                o.video.gui_scale
            };
            let new = current + notches.signum() as i32;
            if new != 0 && (1..=max).contains(&new) {
                let old = self.video;
                self.video.gui_scale = new;
                self.apply_video(Some(old));
                if let Some(o) = &mut self.options {
                    o.scroll = 0.0;
                }
            }
            return;
        }
        if let Some(o) = &mut self.options {
            let max = options::max_scroll(o.page, height);
            o.scroll = (o.scroll - notches as f64 * options::SCROLL_RATE).clamp(0.0, max);
        }
    }

    fn inventory_open(&self) -> bool {
        self.walker
            .as_ref()
            .is_some_and(walk::Walker::inventory_open)
    }

    /// A key in the open inventory: Inventory or Esc closes it and captures the mouse again;
    /// the hotbar keys and Swap Item With Offhand swap the hovered slot with that hotbar
    /// slot or the offhand, and Drop throws from it (`AbstractContainerScreen.keyPressed`).
    fn inventory_key(&mut self, event: &winit::event::KeyEvent) {
        use crate::inventory::InventoryKey;
        let (Some(state), Some(walker)) = (&self.state, &mut self.walker) else {
            return;
        };
        let winit::keyboard::PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        let (b, key) = (&self.bindings, keys::from_code(code));
        let key = if code == winit::keyboard::KeyCode::Escape || b.is("key.inventory", key) {
            walker.close_inventory();
            self.captured = state.grab();
            self.toggles.restore(&self.bindings);
            return;
        } else if let Some(n) = b.hotbar(key) {
            InventoryKey::Hotbar(n)
        } else if b.is("key.swapOffhand", key) {
            InventoryKey::Offhand
        } else if b.is("key.drop", key) {
            InventoryKey::Drop {
                all: self.modifiers.control_key(),
            }
        } else {
            return;
        };
        let (width, height, mouse) = state.gui(self.cursor);
        walker.inventory_key(key, mouse, width, height);
    }

    /// A key in the open chat: Enter sends what was typed and closes it, as Esc does,
    /// capturing the mouse again.
    fn chat_key(&mut self, event: &winit::event::KeyEvent) {
        let close = match self.chat.key(&event.logical_key, event.text.as_deref()) {
            chat::Key::Send(message) => {
                if let Some(walker) = &self.walker
                    && let Err(e) = walker.send_chat(&message)
                {
                    eprintln!("sending chat: {e:#}");
                }
                true
            }
            chat::Key::Close => true,
            chat::Key::None => false,
        };
        if close {
            self.chat.close();
            if let Some(state) = &self.state {
                self.captured = state.grab();
                self.toggles.restore(&self.bindings);
            }
        }
    }

    /// A key or mouse button pressed or let go in the game: it's held while down, and with
    /// no screen open a press does what its mappings do (`Minecraft.handleKeybinds`):
    /// Attack, Use and Pick Block click, Inventory opens the inventory, Chat and Command
    /// open the chat, the latter with a slash, and the hotbar keys select their slot.
    fn game_input(&mut self, key: Key, down: bool) {
        if !down {
            self.keys.remove(&key);
            self.toggles.set(&self.bindings, key, false);
            return;
        }
        self.keys.insert(key);
        let b = &self.bindings;
        let Some(walker) = &mut self.walker else {
            return;
        };
        if walker.screen_open() {
            return;
        }
        self.toggles.set(b, key, true);
        let click = [
            ("key.attack", walk::Click::Attack),
            ("key.use", walk::Click::Use),
            ("key.pickItem", walk::Click::Pick),
        ];
        for (name, click) in click {
            if b.is(name, key) {
                walker.click(click);
            }
        }
        if let Some(slot) = b.hotbar(key) {
            walker.select(slot);
        }
        let screen = if b.is("key.inventory", key) {
            walker.open_inventory();
            walker.inventory_open()
        } else if b.is("key.chat", key) || b.is("key.command", key) {
            let prefix = if b.is("key.chat", key) { "" } else { "/" };
            self.chat.open(prefix);
            true
        } else {
            false
        };
        // A screen frees the mouse and lets go of the keys (`KeyMapping.releaseAll`).
        if screen {
            if let Some(state) = &self.state {
                state.release();
            }
            self.captured = false;
            self.keys.clear();
            self.toggles.release();
        }
    }

    fn fly(&mut self, dt: f32) {
        let camera = &mut self.args.camera;
        let input = self.bindings.input(&self.keys, &self.toggles);
        let motion = fly_direction(camera.yaw, input);
        let speed = if input.sprint { 30.0 } else { 10.0 };
        camera.eye += motion.normalize_or_zero() * speed * dt;
    }
}

impl winit::application::ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.state.is_none() {
            match self.create_window(event_loop) {
                // The options that need the window, such as fullscreen.
                Ok(()) => self.apply_video(None),
                Err(e) => {
                    eprintln!("could not open a window: {e:#}");
                    event_loop.exit();
                }
            }
        }
    }

    fn device_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        _: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if let winit::event::DeviceEvent::MouseMotion { .. } = event {
            self.last_input = Instant::now();
        }
        if let (true, winit::event::DeviceEvent::MouseMotion { delta: (dx, dy) }) =
            (self.captured, event)
        {
            let (dx, dy) = self.mouse.turn(dx, dy);
            let camera = &mut self.args.camera;
            camera.yaw += dx as f32;
            camera.pitch = (camera.pitch + dy as f32).clamp(-90.0, 90.0);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        use winit::event::{ElementState, WindowEvent};
        // `FramerateLimitTracker.onInputReceived`.
        if matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::CursorMoved { .. }
        ) {
            self.last_input = Instant::now();
        }
        if let WindowEvent::MouseInput {
            state: ElementState::Pressed,
            ..
        } = event
        {
            self.input_type = Some(InputType::Mouse);
        }
        let Some(state) = &mut self.state else { return };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.config.width = size.width.max(1);
                state.config.height = size.height.max(1);
                state
                    .surface
                    .configure(&state.renderer.device, &state.config);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some([position.x as f32, position.y as f32]);
                let (width, height, mouse) = state.gui(self.cursor);
                if let (Some(walker), Some(mouse)) = (&mut self.walker, mouse)
                    && walker.inventory_open()
                {
                    walker.inventory_drag(mouse, width, height);
                }
                if self.drag.is_some() {
                    self.options_drag();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } if self.options.as_ref().is_some_and(|o| o.selected.is_some()) => {
                self.bind_key(keys::from_mouse(button));
            }
            WindowEvent::MouseInput {
                state: pressed,
                button: winit::event::MouseButton::Left,
                ..
            } if self.options.is_some() => match pressed {
                ElementState::Pressed => self.options_click(),
                ElementState::Released => self.options_release(),
            },
            WindowEvent::MouseInput { .. } if self.options.is_some() => {}
            WindowEvent::MouseWheel { delta, .. } if self.options.is_some() => {
                let notches = self.mouse.scroll(wheel_notches(delta));
                self.options_scroll(notches as f32);
            }
            // Esc closes the screen, as Done does, unless a key mapping waits for a key.
            WindowEvent::KeyboardInput { event, .. } if self.options.is_some() => {
                if event.state == ElementState::Pressed
                    && let winit::keyboard::PhysicalKey::Code(code) = event.physical_key
                    && !self.bind_key(keys::from_code(code))
                {
                    match code {
                        winit::keyboard::KeyCode::Escape => self.options_back(),
                        _ => self.options_key(code),
                    }
                }
            }
            WindowEvent::MouseInput {
                state: pressed,
                button,
                ..
            } if self
                .walker
                .as_ref()
                .is_some_and(walk::Walker::inventory_open) =>
            {
                use winit::event::MouseButton;
                let button = match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                    _ => return,
                };
                let (width, height, mouse) = state.gui(self.cursor);
                let shift = self.modifiers.shift_key();
                if let (Some(walker), Some(mouse)) = (&mut self.walker, mouse) {
                    let down = pressed == ElementState::Pressed;
                    walker.inventory_mouse(button, down, shift, mouse, width, height);
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if self
                    .walker
                    .as_ref()
                    .is_some_and(walk::Walker::inventory_open) =>
            {
                if event.state == ElementState::Pressed {
                    self.inventory_key(&event);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } if self.menu.is_some() => self.menu_click(event_loop),
            WindowEvent::KeyboardInput { event, .. }
                if self.menu.is_some() && event.state == ElementState::Pressed =>
            {
                self.menu_key(&event, event_loop);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } if self.paused => self.pause_click(),
            WindowEvent::KeyboardInput { event, .. } if self.chat.is_open() => {
                if event.state == ElementState::Pressed {
                    self.chat_key(&event);
                }
            }
            // `ChatScreen.mouseScrolled`: seven lines a notch, one with Shift.
            WindowEvent::MouseWheel { delta, .. } if self.chat.is_open() => {
                let lines = self.mouse.scroll(wheel_notches(delta)) as f32;
                let shift = self.modifiers.shift_key();
                let notches = lines.clamp(-1.0, 1.0) * if shift { 1.0 } else { 7.0 };
                self.chat.scroll_by(notches as i32);
            }
            WindowEvent::MouseInput { .. } if self.chat.is_open() => {}
            // Esc closes the pause screen (`Screen.shouldCloseOnEsc`), back to the game.
            WindowEvent::KeyboardInput { event, .. } if self.paused => {
                if event.state == ElementState::Pressed
                    && let winit::keyboard::PhysicalKey::Code(code) = event.physical_key
                {
                    match code {
                        winit::keyboard::KeyCode::Escape => self.resume(),
                        _ => self.pause_key(code),
                    }
                }
            }
            // `Minecraft.setWindowActive`: with `pauseOnLostFocus`, losing focus pauses.
            WindowEvent::Focused(false) => self.pause(),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: winit::event::MouseButton::Left,
                ..
            } if self.walker.as_ref().is_some_and(walk::Walker::screen_open) => {
                let (width, height, mouse) = state.gui(self.cursor);
                if let (Some(walker), Some(mouse)) = (&mut self.walker, mouse)
                    && walker.click_screen(mouse, width, height) == walk::ScreenAction::Quit
                {
                    self.leave_world(menu::Screen::Title);
                }
            }
            // The death screen's buttons by keyboard; Esc and releases go on below.
            WindowEvent::KeyboardInput { event, .. }
                if self.walker.as_ref().is_some_and(walk::Walker::screen_open)
                    && event.state == ElementState::Pressed
                    && event.physical_key
                        != winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Escape) =>
            {
                if let winit::keyboard::PhysicalKey::Code(code) = event.physical_key {
                    self.death_key(code);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } if !self.captured
                && self.menu.is_none()
                && !self.paused
                && !self.walker.as_ref().is_some_and(walk::Walker::screen_open) =>
            {
                self.captured = state.grab();
                self.toggles.restore(&self.bindings);
            }
            WindowEvent::MouseInput {
                state: pressed,
                button,
                ..
            } => self.game_input(keys::from_mouse(button), pressed == ElementState::Pressed),
            WindowEvent::MouseWheel { delta, .. } if self.captured => {
                // `ScrollWheelHandler.onMouseScroll`: whole notches, the rest kept for the
                // next scroll the same way. Scrolling down moves right along the hotbar.
                let scrolled = self.mouse.scroll(wheel_notches(delta));
                if self.wheel != 0.0 && scrolled.signum() != self.wheel.signum() {
                    self.wheel = 0.0;
                }
                self.wheel += scrolled;
                let notches = self.wheel as i32;
                self.wheel -= notches as f64;
                if let Some(walker) = &mut self.walker
                    && notches != 0
                {
                    walker.scroll(if notches < 0 { 1 } else { -1 });
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let winit::keyboard::PhysicalKey::Code(code) = event.physical_key {
                    if code == winit::keyboard::KeyCode::Escape
                        && event.state == ElementState::Pressed
                    {
                        match &mut self.walker {
                            Some(walker) if walker.screen_open() => walker.escape(),
                            _ => self.pause(),
                        }
                        return;
                    }
                    let down = event.state == ElementState::Pressed;
                    self.game_input(keys::from_code(code), down);
                }
            }
            WindowEvent::RedrawRequested => {
                self.limit_frame_rate();
                let now = Instant::now();
                let elapsed = (now - self.last_frame).as_secs_f32();
                // The game's own ticks get the real time, and run at most 10 a frame
                // (`Walker::frame`); flying and animations here cap a long frame at 0.1 s.
                let dt = elapsed.min(0.1);
                self.last_frame = now;
                // 20 game ticks a second.
                self.view.ticks += dt * 20.0;
                while let Some(loading) = &self.loading {
                    match loading.try_recv() {
                        Ok(Loading::Status(status)) => {
                            if let Some(m) = &mut self.menu {
                                m.screen = menu::Screen::Connecting(status);
                            }
                            continue;
                        }
                        Ok(Loading::Done(result)) => match *result {
                            Ok((scene, server)) => {
                                self.loading = None;
                                if let Err(e) = self.enter_world(scene, server) {
                                    eprintln!("entering the world: {e:#}");
                                }
                            }
                            Err(e) => {
                                eprintln!("couldn't join: {e:#}");
                                self.leave_world(menu::Screen::Disconnected {
                                    title: "connect.failed",
                                    reason: format!("{e:#}"),
                                });
                            }
                        },
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => self.loading = None,
                    }
                    break;
                }
                let updates: Vec<_> = self
                    .streamer
                    .as_ref()
                    .map(|s| s.updates().collect())
                    .unwrap_or_default();
                let mut lost = None;
                let Some(state) = &mut self.state else { return };
                for update in updates {
                    match update {
                        stream::Update::Mesh(key, mesh) => state.renderer.upload_chunk(key, &mesh),
                        stream::Update::Remove(key) => state.renderer.remove_chunk(key),
                        stream::Update::Teleport(teleport) => {
                            let eye = teleport_eye(&teleport);
                            self.args.place(eye, teleport.yaw, teleport.pitch);
                            // Only the first placement is a choice; later ones are moves.
                            (self.args.pos_given, self.args.look_given) = (false, false);
                            if let Some(walker) = &mut self.walker {
                                let camera = &self.args.camera;
                                walker.teleport(eye - Vec3::Y * 1.62, camera.yaw, camera.pitch);
                                // `ClientPacketListener.handleMovePlayer`: accepted from the
                                // game's thread, once the player is there.
                                walker.accept_teleport(&teleport);
                            }
                        }
                        stream::Update::Collision(key, blocks) => {
                            if let Some(walker) = &mut self.walker {
                                walker.chunk(key, blocks);
                            }
                        }
                        stream::Update::Light {
                            chunk,
                            sections,
                            whole,
                        } => {
                            if let Some(walker) = &mut self.walker {
                                walker.light(chunk, sections, whole);
                            }
                        }
                        stream::Update::Blocks(changes) => {
                            if let Some(walker) = &mut self.walker {
                                walker.blocks(&changes);
                            }
                        }
                        // `handleStopSoundEvent`, after the sounds queued before it.
                        stream::Update::Server(mc_protocol::Event::StopSound {
                            event,
                            category,
                        }) => {
                            if let Some(walker) = &mut self.walker {
                                for instance in walker.take_sounds() {
                                    self.sounds.play(instance);
                                }
                            }
                            self.sounds.stop_matching(
                                event.as_deref(),
                                category.map(sound::Category::from_id),
                            );
                        }
                        stream::Update::Server(mc_protocol::Event::Chat(spans)) => {
                            self.chat
                                .add(&self.models.hud, &spans, self.view.ticks as i32);
                        }
                        stream::Update::Server(event) => {
                            if let Some(walker) = &mut self.walker {
                                walker.server_event(event);
                            }
                        }
                        stream::Update::Disconnected(reason) => {
                            eprintln!("disconnected: {reason}");
                            lost = Some(reason);
                        }
                        stream::Update::Idle => {}
                    }
                }
                if let Some(reason) = lost {
                    self.leave_world(menu::Screen::Disconnected {
                        title: "disconnect.lost",
                        reason,
                    });
                }
                match &mut self.walker {
                    Some(walker) => {
                        let camera = &mut self.args.camera;
                        let (input, buttons) = if self.captured {
                            let b = &self.bindings;
                            (
                                b.input(&self.keys, &self.toggles),
                                keys::Bindings::buttons(&self.toggles),
                            )
                        } else {
                            Default::default()
                        };
                        if let Some(eye) =
                            walker.frame(elapsed, input, buttons, camera.yaw, camera.pitch)
                        {
                            camera.eye = eye;
                            camera.bob = walker.view_bob();
                        }
                        if let Some(state) = &mut self.state {
                            let mut hotbar = walker.hotbar();
                            hotbar.chat = self.chat.view(self.view.ticks as i32);
                            // A screen the server opens frees the mouse, as `Gui.setScreen`
                            // does, and the server closing it takes the mouse back.
                            if walker.inventory_open() && self.captured {
                                state.release();
                                self.captured = false;
                                self.keys.clear();
                                self.toggles.release();
                            }
                            if walker.take_closed_screen() && !self.paused && self.menu.is_none() {
                                self.captured = state.grab();
                                self.toggles.restore(&self.bindings);
                            }
                            if let Some(death) = &mut hotbar.death {
                                // A screen frees the mouse, as `Gui.setScreen` does.
                                if self.captured {
                                    state.release();
                                    self.captured = false;
                                    self.keys.clear();
                                    self.toggles.release();
                                }
                                death.mouse = state.gui(self.cursor).2;
                            }
                            let hotbar = Some(hotbar);
                            let renderer = &mut state.renderer;
                            let target = walker.target();
                            if target != self.outlined {
                                self.outlined = target;
                                renderer.set_outline(&walker.outline_edges());
                            }
                            if hotbar != self.shown_hotbar {
                                renderer.set_hotbar(hotbar.clone());
                                self.shown_hotbar = hotbar;
                            }
                            for (sprite, skin) in walker.new_skins() {
                                renderer.write_sprite(self.models.atlas.sprite(&sprite), &skin);
                            }
                            let entities = walker.entities();
                            let mut meshes = mc_render::entity::vertices(
                                &self.models,
                                camera,
                                self.view.hud,
                                &entities,
                            );
                            meshes.cutout.extend(mc_render::particle::vertices(
                                &self.models.atlas,
                                camera,
                                &walker.particles(),
                            ));
                            meshes.lightning = walker.lightning();
                            for (pos, state, open, light) in walker.chest_lids() {
                                if let Some(lid) = self
                                    .models
                                    .states
                                    .get(state as usize)
                                    .and_then(|s| s.lid.as_ref())
                                {
                                    meshes.cutout.extend(lid.vertices(pos, open, light));
                                }
                            }
                            renderer.set_entities(&meshes);
                            // F1 hides the hands with the HUD.
                            let hand = walker
                                .first_person(camera.yaw, camera.pitch, camera.eye)
                                .filter(|_| self.view.hud)
                                .map(|(hands, light)| {
                                    mc_render::entity::first_person(
                                        &self.models,
                                        camera,
                                        &hands,
                                        light,
                                    )
                                });
                            renderer.set_hand(&hand.unwrap_or_default());
                            let fire = walker.sees_fire().then(|| {
                                mc_render::entity::fire_overlay(&self.models.atlas, camera)
                            });
                            renderer.set_fire_overlay(&fire.unwrap_or_default());
                            renderer.set_smart_cull(walker.smart_cull(camera.eye));
                            renderer.set_in_fluid(walker.in_fluid(
                                camera.eye,
                                camera.yaw,
                                camera.pitch,
                            ));
                            renderer.set_sky(walker.sky(camera.eye).unwrap_or_default());
                            renderer.set_weather(&walker.weather(camera.eye));
                            let cracks = walker.cracks();
                            if cracks != self.shown_cracks {
                                let vertices: Vec<_> = cracks
                                    .iter()
                                    .flat_map(|&(pos, block, stage)| {
                                        mc_render::hud::cracks(&self.models, block, pos, stage)
                                    })
                                    .collect();
                                renderer.set_cracks(&vertices);
                                self.shown_cracks = cracks;
                            }
                        }
                    }
                    None if self.streamer.is_some() => self.fly(dt),
                    None => {}
                }
                if let Some(streamer) = &mut self.streamer {
                    streamer.follow(self.args.camera.eye);
                }
                self.tick_sounds(dt);
                let options_view = self.options_view();
                let Some(state) = &mut self.state else { return };
                // Dying closes the pause screen for the death screen.
                if self.walker.as_ref().is_some_and(walk::Walker::screen_open) {
                    self.paused = false;
                    if self.options.as_ref().is_some_and(|o| o.in_world) {
                        (self.options, self.drag) = (None, None);
                    }
                }
                let (_, gui_height, mouse) = state.gui(self.cursor);
                let screen = match &self.menu {
                    _ if let Some(o) = options_view => {
                        Some(mc_render::hud::Screen::Options(options::Options {
                            mouse,
                            // A smaller window scrolls less far.
                            scroll: o.scroll.min(options::max_scroll(o.page, gui_height)),
                            ..o
                        }))
                    }
                    Some(m) => Some(mc_render::hud::Screen::Menu(menu::Menu {
                        mouse,
                        // `TextCursorUtils.isCursorVisible`: on for 300 ms, off for 300.
                        cursor_on: (self.focused_at.elapsed().as_millis() / 300).is_multiple_of(2),
                        ..m.clone()
                    })),
                    None if self.paused => Some(mc_render::hud::Screen::Pause(
                        mc_render::hud::pause::Pause {
                            local: self.walker.is_none() || self.singleplayer,
                            mouse,
                            focused: self.pause_focus,
                        },
                    )),
                    None => self
                        .walker
                        .as_ref()
                        .filter(|w| w.inventory_open())
                        .map(|w| mc_render::hud::Screen::Inventory(w.inventory_view(mouse))),
                };
                state.renderer.set_screen(screen);
                let frame = match state.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    _ => {
                        state
                            .surface
                            .configure(&state.renderer.device, &state.config);
                        state.window.request_redraw();
                        return;
                    }
                };
                let target = frame.texture.create_view(&Default::default());
                // `GameRenderer.getFov`: the FOV setting, narrowed while a bow is drawn.
                let fov = self.walker.as_ref().map_or(1.0, |w| w.fov_modifier());
                let camera = Camera {
                    fov: self.args.camera.fov * fov,
                    ..self.args.camera
                };
                state.renderer.render(
                    &target,
                    state.config.width,
                    state.config.height,
                    &camera,
                    &self.view,
                );
                state.renderer.queue.present(frame);
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

/// `InputType`: what was last used for input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InputType {
    Mouse,
    Tab,
    Arrow,
}

impl InputType {
    fn keyboard(self) -> bool {
        self != InputType::Mouse
    }
}

/// `InputWithModifiers.isSelection`: Enter, Space or the keypad's Enter.
/// `InputWithModifiers.isConfirmation`: Enter.
fn confirmation(code: winit::keyboard::KeyCode) -> bool {
    use winit::keyboard::KeyCode as K;
    matches!(code, K::Enter | K::NumpadEnter)
}

/// `InputConstants.Key.getNumericKeyValue`: a digit key's digit.
fn digit(code: winit::keyboard::KeyCode) -> Option<usize> {
    use winit::keyboard::KeyCode as K;
    [K::Digit0, K::Digit1, K::Digit2, K::Digit3, K::Digit4]
        .into_iter()
        .chain([K::Digit5, K::Digit6, K::Digit7, K::Digit8, K::Digit9])
        .position(|k| k == code)
}

fn selection(code: winit::keyboard::KeyCode) -> bool {
    use winit::keyboard::KeyCode as K;
    matches!(code, K::Enter | K::Space | K::NumpadEnter)
}

/// How many notches the wheel turned: a line each, or 40 pixels for a touchpad.
fn wheel_notches(delta: winit::event::MouseScrollDelta) -> f64 {
    match delta {
        winit::event::MouseScrollDelta::LineDelta(_, y) => y as f64,
        winit::event::MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
    }
}

/// Which way the held movement keys fly, before normalising, for a camera turned `yaw`
/// degrees: forward and back along the view's level direction, left and right to its
/// sides, jump and sneak up and down.
fn fly_direction(yaw: f32, input: mc_physics::Input) -> Vec3 {
    let yaw = yaw.to_radians();
    let forward = Vec3::new(-yaw.sin(), 0.0, yaw.cos());
    // Forward × up: facing south (+Z), the right hand points west (-X).
    let right = forward.cross(Vec3::Y);
    [
        (input.forward, forward),
        (input.backward, -forward),
        (input.right, right),
        (input.left, -right),
        (input.jump, Vec3::Y),
        (input.sneak, -Vec3::Y),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .map(|(_, dir)| dir)
    .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D moves toward whatever is on the right of the screen, A toward the left. The
    /// screen's right is worked out from the camera's own view matrix, not from
    /// `fly_direction`'s maths, and checked at a turned yaw so a sign slip can't hide.
    #[test]
    fn d_strafes_toward_the_right_of_the_screen() {
        for yaw in [0.0, 37.0, 90.0, 200.0] {
            let camera = Camera {
                eye: Vec3::ZERO,
                yaw,
                pitch: 0.0,
                fov: 70.0,
                bob: glam::Mat4::IDENTITY,
            };
            let view_proj = camera.view_proj(1.0, 100.0);
            let screen_x = |p: Vec3| view_proj.project_point3(p).x;
            let ahead = camera.forward() * 10.0;
            let d = fly_direction(
                yaw,
                mc_physics::Input {
                    right: true,
                    ..Default::default()
                },
            );
            let a = fly_direction(
                yaw,
                mc_physics::Input {
                    left: true,
                    ..Default::default()
                },
            );
            assert!(screen_x(ahead + d) > 0.0, "yaw {yaw}: D goes left");
            assert!(screen_x(ahead + a) < 0.0, "yaw {yaw}: A goes right");
        }
    }
}
