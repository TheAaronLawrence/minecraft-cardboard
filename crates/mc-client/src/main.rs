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
//! Without `--world` or `--server`, the window opens on the title screen: Singleplayer opens
//! the test world, and Multiplayer asks for a server's address (remembered in `options.txt`
//! beside the exe). In a world, Esc (or the window losing focus) opens the pause screen,
//! whose Disconnect, or Save and Quit to Title, comes back to the menus, as losing the
//! connection does.

use anyhow::{Context, Result, bail};
use glam::Vec3;
use mc_render::bake::BlockModels;
use mc_render::hud::menu;

mod chat;
mod entities;
mod hands;
mod inventory;
mod light;
mod particles;
mod skins;
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
            "--distance" => args.distance = value()?.parse()?,
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
    let view = View {
        distance: (args.distance * 16) as f32,
        ticks: args.ticks,
        hud: false,
    };
    match args.screenshot.clone() {
        Some(path) if args.server.is_some() => {
            let scene = load_scene(&args)?;
            let models = scene.models.clone();
            let mut streamer = spawn_streamer(scene, &args);
            let mut renderer = Renderer::headless(&models)?;
            server_screenshot(&mut args, &mut streamer, &mut renderer)?;
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
                menu: None,
                loading: None,
                view: View { hud: true, ..view },
                buttons: Default::default(),
                outlined: None,
                shown_hotbar: None,
                shown_cracks: Vec::new(),
                state: None,
                keys: Default::default(),
                last_frame: Instant::now(),
                captured: false,
                cursor: None,
                focused_at: Instant::now(),
                paused: false,
                chat: Default::default(),
                modifiers: Default::default(),
            };
            if app.start.server.is_some() || app.start.world_given {
                app.play(app.start.server.clone());
            } else {
                app.menu = Some(menu::Menu::default());
            }
            event_loop.run_app(&mut app)?;
            Ok(())
        }
    }
}

/// `options.txt` beside the exe, where vanilla keeps its options.
fn options_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("options.txt")))
        .unwrap_or_else(|| "options.txt".into())
}

/// `Options.lastMpIp`, saved as vanilla saves it: `lastServer:` in `options.txt`.
fn last_server() -> String {
    std::fs::read_to_string(options_path())
        .unwrap_or_default()
        .lines()
        .find_map(|l| l.strip_prefix("lastServer:"))
        .unwrap_or_default()
        .to_owned()
}

fn save_last_server(address: &str) {
    let old = std::fs::read_to_string(options_path()).unwrap_or_default();
    let mut lines: Vec<String> = old
        .lines()
        .filter(|l| !l.starts_with("lastServer:"))
        .map(str::to_owned)
        .collect();
    lines.push(format!("lastServer:{address}"));
    if let Err(e) = std::fs::write(options_path(), lines.join("\n") + "\n") {
        eprintln!("saving options.txt: {e}");
    }
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

/// Points the camera where the server put the player (unless `--pos`/`--look` say
/// otherwise), and uploads chunks until none has arrived for a few seconds.
fn server_screenshot(
    args: &mut Args,
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
                stream::Update::Teleport(eye, yaw, pitch) => {
                    args.place(eye, yaw, pitch);
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

struct WindowState {
    window: Arc<winit::window::Window>,
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
    /// The menu screen showing, when not playing.
    menu: Option<menu::Menu>,
    /// A world loading or a server being joined, in the background.
    loading: Option<std::sync::mpsc::Receiver<Result<Scene>>>,
    /// When the menu's text box was last focused or edited, which restarts its blink.
    focused_at: Instant,
    /// The pause screen is open over the world.
    paused: bool,
    /// The chat on a server, and its screen when open.
    chat: chat::Chat,
    /// Shift, Ctrl and the like, held now.
    modifiers: winit::keyboard::ModifiersState,
    view: View,
    state: Option<WindowState>,
    keys: std::collections::HashSet<winit::keyboard::KeyCode>,
    /// Mouse buttons held while the mouse is captured.
    buttons: walk::Buttons,
    /// The block the outline is drawn around, and its state.
    outlined: Option<(glam::IVec3, u32)>,
    /// What the hotbar and the cracks last showed.
    shown_hotbar: Option<mc_render::hud::Hotbar>,
    shown_cracks: Vec<([i32; 3], u32, u8)>,
    last_frame: Instant,
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
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        let renderer = Renderer::new(device, queue, format, &self.models);
        self.state = Some(WindowState {
            window,
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
        self.args = args.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(load_scene(&args));
        });
        self.loading = Some(rx);
        let menu = self.menu.get_or_insert_with(Default::default);
        menu.screen = menu::Screen::Connecting(key);
    }

    /// The world finished loading: draw it with a renderer for its models.
    fn enter_world(&mut self, scene: Scene) -> Result<()> {
        let state = self.state.as_mut().context("no window")?;
        self.walker = match scene.outbox.clone() {
            // An offline-mode server, which is all `connect` joins, gives the player the
            // offline UUID for their name.
            Some(outbox) => Some(walk::Walker::new(
                outbox,
                &scene.registry,
                scene.biomes.clone(),
                mc_protocol::offline_uuid(&self.args.name),
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
        (self.menu, self.paused) = (None, false);
        self.chat = Default::default();
        (self.outlined, self.shown_hotbar) = (None, None);
        self.shown_cracks.clear();
        self.view.ticks = self.args.ticks;
        Ok(())
    }

    /// Leaves the world in play (closing the server connection) for this menu screen.
    fn leave_world(&mut self, screen: menu::Screen) {
        if let Some(walker) = &self.walker {
            walker.leave();
        }
        (self.walker, self.streamer, self.loading) = (None, None, None);
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
        (self.captured, self.buttons, self.paused) = (false, Default::default(), false);
        self.keys.clear();
        self.chat = Default::default();
        self.menu = Some(menu::Menu {
            screen,
            ..Default::default()
        });
    }

    /// Where a menu goes back to: Direct Connection from joining a server, the title
    /// screen from everything else.
    fn back_screen(&self) -> menu::Screen {
        match self.menu.as_ref().map(|m| &m.screen) {
            Some(menu::Screen::DirectConnect) | None => menu::Screen::Title,
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
        match action {
            menu::Action::Singleplayer => self.play(None),
            menu::Action::Multiplayer => {
                m.screen = menu::Screen::DirectConnect;
                m.address = menu::Field::new(&last_server());
                self.focused_at = Instant::now();
            }
            menu::Action::Options => {}
            menu::Action::Quit => event_loop.exit(),
            menu::Action::Join => {
                let address = m.address.value.trim().to_owned();
                if menu::valid_address(&address) {
                    save_last_server(&address);
                    self.play(Some(address));
                }
            }
            menu::Action::Back => {
                self.loading = None;
                let screen = back;
                if screen == menu::Screen::DirectConnect {
                    m.address = menu::Field::new(&last_server());
                }
                m.screen = screen;
            }
        }
    }

    fn menu_click(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        let Some(state) = &self.state else { return };
        let (width, height, Some(mouse)) = state.gui(self.cursor) else {
            return;
        };
        let Some(m) = &mut self.menu else { return };
        let art = &self.menu_models.hud;
        if m.screen == menu::Screen::DirectConnect {
            let rect = menu::address_rect(width);
            let [x, y, w, h] = rect.map(|c| c as f32);
            if mouse[0] >= x && mouse[1] >= y && mouse[0] < x + w && mouse[1] < y + h {
                m.address.cursor = menu::clicked_position(art, &m.address, rect, mouse[0]);
                self.focused_at = Instant::now();
            }
        }
        let action = menu::buttons(m, art, width, height)
            .into_iter()
            .find(|(_, button, active)| *active && button.contains(mouse))
            .map(|(action, ..)| action);
        if let Some(action) = action {
            self.menu_action(action, event_loop);
        }
    }

    /// A key on a menu: Direct Connection's box edits as `EditBox` does, Enter joins and
    /// Esc goes back.
    fn menu_key(
        &mut self,
        event: &winit::event::KeyEvent,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        use winit::keyboard::{Key, NamedKey};
        let Some(m) = &mut self.menu else { return };
        if m.screen != menu::Screen::DirectConnect {
            return;
        }
        let field = &mut m.address;
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => return self.menu_action(menu::Action::Join, event_loop),
            Key::Named(NamedKey::Escape) => {
                return self.menu_action(menu::Action::Back, event_loop);
            }
            Key::Named(NamedKey::Backspace) => field.backspace(),
            Key::Named(NamedKey::Delete) => field.delete(),
            Key::Named(NamedKey::ArrowLeft) => field.step(-1),
            Key::Named(NamedKey::ArrowRight) => field.step(1),
            Key::Named(NamedKey::Home) => field.home(),
            Key::Named(NamedKey::End) => field.end(),
            _ => field.insert(event.text.as_deref().unwrap_or(""), 128),
        }
        self.focused_at = Instant::now();
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
        (self.paused, self.captured, self.buttons) = (true, false, Default::default());
        self.keys.clear();
    }

    /// Back to Game: closes the pause screen and captures the mouse again.
    fn resume(&mut self) {
        self.paused = false;
        if let Some(state) = &self.state {
            self.captured = state.grab();
        }
    }

    fn pause_click(&mut self) {
        let Some(state) = &self.state else { return };
        let (width, height, Some(mouse)) = state.gui(self.cursor) else {
            return;
        };
        let pause = mc_render::hud::pause::Pause {
            local: self.walker.is_none(),
            mouse: None,
        };
        let action = mc_render::hud::pause::buttons(&pause, width, height)
            .into_iter()
            .find(|(_, button, active, _)| *active && button.contains(mouse))
            .map(|(action, ..)| action);
        match action {
            Some(mc_render::hud::pause::Action::ReturnToGame) => self.resume(),
            Some(mc_render::hud::pause::Action::Leave) => self.leave_world(menu::Screen::Title),
            _ => {}
        }
    }

    fn inventory_open(&self) -> bool {
        self.walker
            .as_ref()
            .is_some_and(walk::Walker::inventory_open)
    }

    /// A key in the open inventory: E or Esc closes it and captures the mouse again; 1 to 9
    /// and F swap the hovered slot with that hotbar slot or the offhand, and Q throws from
    /// it (`AbstractContainerScreen.keyPressed`).
    fn inventory_key(&mut self, event: &winit::event::KeyEvent) {
        use crate::inventory::InventoryKey;
        use winit::keyboard::{KeyCode as K, PhysicalKey};
        let (Some(state), Some(walker)) = (&self.state, &mut self.walker) else {
            return;
        };
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        let digits = [
            K::Digit1,
            K::Digit2,
            K::Digit3,
            K::Digit4,
            K::Digit5,
            K::Digit6,
            K::Digit7,
            K::Digit8,
            K::Digit9,
        ];
        let key = match code {
            K::Escape | K::KeyE => {
                walker.close_inventory();
                self.captured = state.grab();
                return;
            }
            K::KeyF => InventoryKey::Offhand,
            K::KeyQ => InventoryKey::Drop {
                all: self.modifiers.control_key(),
            },
            _ => match digits.iter().position(|&d| d == code) {
                Some(n) => InventoryKey::Hotbar(n as u8),
                None => return,
            },
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
            }
        }
    }

    fn fly(&mut self, dt: f32) {
        use winit::keyboard::KeyCode as K;
        let camera = &mut self.args.camera;
        let motion = fly_direction(camera.yaw, &self.keys);
        let speed = if self.keys.contains(&K::ControlLeft) {
            30.0
        } else {
            10.0
        };
        camera.eye += motion.normalize_or_zero() * speed * dt;
    }
}

impl winit::application::ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.state.is_none()
            && let Err(e) = self.create_window(event_loop)
        {
            eprintln!("could not open a window: {e:#}");
            event_loop.exit();
        }
    }

    fn device_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        _: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        if let (true, winit::event::DeviceEvent::MouseMotion { delta: (dx, dy) }) =
            (self.captured, event)
        {
            let camera = &mut self.args.camera;
            camera.yaw += dx as f32 * 0.15;
            camera.pitch = (camera.pitch + dy as f32 * 0.15).clamp(-90.0, 90.0);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        use winit::event::{ElementState, WindowEvent};
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
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
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
                let lines = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                let shift = self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft);
                let notches = lines.clamp(-1.0, 1.0) * if shift { 1.0 } else { 7.0 };
                self.chat.scroll_by(notches as i32);
            }
            WindowEvent::MouseInput { .. } if self.chat.is_open() => {}
            // Esc closes the pause screen (`Screen.shouldCloseOnEsc`), back to the game.
            WindowEvent::KeyboardInput { event, .. } if self.paused => {
                if event.state == ElementState::Pressed
                    && event.physical_key
                        == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::Escape)
                {
                    self.resume();
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
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } if !self.captured
                && self.menu.is_none()
                && !self.paused
                && !self.walker.as_ref().is_some_and(walk::Walker::screen_open) =>
            {
                self.captured = state.grab();
            }
            WindowEvent::MouseInput {
                state: pressed,
                button,
                ..
            } => {
                use winit::event::MouseButton;
                let down = pressed == ElementState::Pressed;
                match button {
                    MouseButton::Left => self.buttons.attack = down,
                    MouseButton::Right => self.buttons.use_item = down,
                    _ => {}
                }
                let click = match button {
                    MouseButton::Left => Some(walk::Click::Attack),
                    MouseButton::Right => Some(walk::Click::Use),
                    MouseButton::Middle => Some(walk::Click::Pick),
                    _ => None,
                };
                if down && let (Some(click), Some(walker)) = (click, &mut self.walker) {
                    walker.click(click);
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.captured => {
                let lines = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                // Scrolling down moves right along the hotbar.
                if let Some(walker) = &mut self.walker
                    && lines != 0.0
                {
                    walker.scroll(if lines < 0.0 { 1 } else { -1 });
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
                    // `Gui.handleKeybinds`: E opens the inventory.
                    if code == winit::keyboard::KeyCode::KeyE
                        && event.state == ElementState::Pressed
                        && let Some(walker) = &mut self.walker
                        && !walker.screen_open()
                    {
                        walker.open_inventory();
                        if walker.inventory_open() {
                            state.release();
                            (self.captured, self.buttons) = (false, Default::default());
                            self.keys.clear();
                        }
                        return;
                    }
                    // `Gui.handleKeybinds`: T opens the chat, / opens it with a slash.
                    let prefix = match code {
                        winit::keyboard::KeyCode::KeyT => Some(""),
                        winit::keyboard::KeyCode::Slash => Some("/"),
                        _ => None,
                    };
                    if let Some(prefix) = prefix
                        && event.state == ElementState::Pressed
                        && self.walker.as_ref().is_some_and(|w| !w.screen_open())
                    {
                        self.chat.open(prefix);
                        state.release();
                        (self.captured, self.buttons) = (false, Default::default());
                        self.keys.clear();
                        return;
                    }
                    let digits = [
                        winit::keyboard::KeyCode::Digit1,
                        winit::keyboard::KeyCode::Digit2,
                        winit::keyboard::KeyCode::Digit3,
                        winit::keyboard::KeyCode::Digit4,
                        winit::keyboard::KeyCode::Digit5,
                        winit::keyboard::KeyCode::Digit6,
                        winit::keyboard::KeyCode::Digit7,
                        winit::keyboard::KeyCode::Digit8,
                        winit::keyboard::KeyCode::Digit9,
                    ];
                    if event.state == ElementState::Pressed
                        && let Some(slot) = digits.iter().position(|&k| k == code)
                        && let Some(walker) = &mut self.walker
                    {
                        walker.select(slot as u8);
                    }
                    match event.state {
                        ElementState::Pressed => self.keys.insert(code),
                        ElementState::Released => self.keys.remove(&code),
                    };
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;
                // 20 game ticks a second.
                self.view.ticks += dt * 20.0;
                if let Some(loading) = &self.loading {
                    match loading.try_recv() {
                        Ok(Ok(scene)) => {
                            self.loading = None;
                            if let Err(e) = self.enter_world(scene) {
                                eprintln!("entering the world: {e:#}");
                            }
                        }
                        Ok(Err(e)) => {
                            eprintln!("couldn't join: {e:#}");
                            self.leave_world(menu::Screen::Disconnected {
                                title: "connect.failed",
                                reason: format!("{e:#}"),
                            });
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {}
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => self.loading = None,
                    }
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
                        stream::Update::Teleport(eye, yaw, pitch) => {
                            self.args.place(eye, yaw, pitch);
                            // Only the first placement is a choice; later ones are moves.
                            (self.args.pos_given, self.args.look_given) = (false, false);
                            if let Some(walker) = &mut self.walker {
                                let camera = &self.args.camera;
                                walker.teleport(eye - Vec3::Y * 1.62, camera.yaw, camera.pitch);
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
                            (walk::input(&self.keys), self.buttons)
                        } else {
                            Default::default()
                        };
                        if let Some(eye) =
                            walker.frame(dt, input, buttons, camera.yaw, camera.pitch)
                        {
                            camera.eye = eye;
                            camera.bob = walker.view_bob();
                        }
                        if let Some(state) = &mut self.state {
                            let mut hotbar = walker.hotbar();
                            hotbar.chat = self.chat.view(self.view.ticks as i32);
                            if let Some(death) = &mut hotbar.death {
                                // A screen frees the mouse, as `Gui.setScreen` does.
                                if self.captured {
                                    state.release();
                                    self.captured = false;
                                    self.buttons = Default::default();
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
                            renderer.set_in_fluid(walker.in_fluid(
                                camera.eye,
                                camera.yaw,
                                camera.pitch,
                            ));
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
                let Some(state) = &mut self.state else { return };
                // Dying closes the pause screen for the death screen.
                if self.walker.as_ref().is_some_and(walk::Walker::screen_open) {
                    self.paused = false;
                }
                let mouse = state.gui(self.cursor).2;
                let screen = match &self.menu {
                    Some(m) => Some(mc_render::hud::Screen::Menu(menu::Menu {
                        mouse,
                        // `TextCursorUtils.isCursorVisible`: on for 300 ms, off for 300.
                        cursor_on: (self.focused_at.elapsed().as_millis() / 300).is_multiple_of(2),
                        ..m.clone()
                    })),
                    None if self.paused => Some(mc_render::hud::Screen::Pause(
                        mc_render::hud::pause::Pause {
                            local: self.walker.is_none(),
                            mouse,
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
                state.renderer.render(
                    &target,
                    state.config.width,
                    state.config.height,
                    &self.args.camera,
                    &self.view,
                );
                state.renderer.queue.present(frame);
                state.window.request_redraw();
            }
            _ => {}
        }
    }
}

/// Which way the held keys fly, before normalising, for a camera turned `yaw` degrees:
/// W and S along the view's level direction, A and D to its left and right, Space and
/// Shift up and down.
fn fly_direction(yaw: f32, keys: &std::collections::HashSet<winit::keyboard::KeyCode>) -> Vec3 {
    use winit::keyboard::KeyCode as K;
    let yaw = yaw.to_radians();
    let forward = Vec3::new(-yaw.sin(), 0.0, yaw.cos());
    // Forward × up: facing south (+Z), the right hand points west (-X).
    let right = forward.cross(Vec3::Y);
    [
        (K::KeyW, forward),
        (K::KeyS, -forward),
        (K::KeyD, right),
        (K::KeyA, -right),
        (K::Space, Vec3::Y),
        (K::ShiftLeft, -Vec3::Y),
    ]
    .into_iter()
    .filter(|(key, _)| keys.contains(key))
    .map(|(_, dir)| dir)
    .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode as K;

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
            let held = |k| std::collections::HashSet::from([k]);
            let d = fly_direction(yaw, &held(K::KeyD));
            let a = fly_direction(yaw, &held(K::KeyA));
            assert!(screen_x(ahead + d) > 0.0, "yaw {yaw}: D goes left");
            assert!(screen_x(ahead + a) < 0.0, "yaw {yaw}: A goes right");
        }
    }
}
