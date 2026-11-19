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
//! down, Ctrl to fly faster, Esc to release the mouse. On a server, left click mines, right
//! click places or uses, middle click picks the block, and 1 to 9 or the wheel choose the
//! hotbar slot.

use anyhow::{Context, Result, bail};
use glam::Vec3;
use mc_render::bake::BlockModels;

mod skins;
mod stream;
mod walk;
use mc_render::gpu::{Camera, Renderer, View};
use mc_render::mesh::{Mesher, WorldView};
use mc_render::tint::Biomes;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

struct Args {
    world: PathBuf,
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
        server: None,
        name: "cardboard".into(),
        pos_given: false,
        look_given: false,
        camera: Camera {
            eye: Vec3::new(0.5, 100.0, 0.5),
            yaw: 0.0,
            pitch: 20.0,
            fov: 70.0,
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
            "--world" => args.world = value()?.into(),
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

fn load_scene(args: &Args) -> Result<Scene> {
    let root = mc_assets::default_root();
    anyhow::ensure!(
        root.join("reports/blocks.json").exists(),
        "can't find the game's resources (looked in {}). If you downloaded the zip, extract \
         the whole of it, then run minecraft-cardboard.exe from the extracted folder.",
        root.display()
    );
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
    let scene = load_scene(&args)?;
    let view = View {
        distance: (args.distance * 16) as f32,
        ticks: args.ticks,
        hud: false,
    };
    match args.screenshot.clone() {
        Some(path) if args.server.is_some() => {
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
            let event_loop = winit::event_loop::EventLoop::new()?;
            let models = scene.models.clone();
            let walker = match scene.outbox.clone() {
                Some(outbox) => Some(walk::Walker::new(outbox, &scene.registry)?),
                None => None,
            };
            let streamer = spawn_streamer(scene, &args);
            let mut app = App {
                args,
                models,
                streamer,
                walker,
                view: View { hud: true, ..view },
                buttons: Default::default(),
                outlined: None,
                shown_hotbar: None,
                shown_cracks: Vec::new(),
                state: None,
                keys: Default::default(),
                last_frame: Instant::now(),
                captured: false,
            };
            event_loop.run_app(&mut app)?;
            Ok(())
        }
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
    args: Args,
    models: Arc<BlockModels>,
    streamer: stream::Streamer,
    /// On a server the player walks with vanilla's physics; on a save the camera flies.
    walker: Option<walk::Walker>,
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
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } if !self.captured => {
                let grab = state
                    .window
                    .set_cursor_grab(winit::window::CursorGrabMode::Locked)
                    .or_else(|_| {
                        state
                            .window
                            .set_cursor_grab(winit::window::CursorGrabMode::Confined)
                    });
                self.captured = grab.is_ok();
                state.window.set_cursor_visible(!self.captured);
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
                        let _ = state
                            .window
                            .set_cursor_grab(winit::window::CursorGrabMode::None);
                        state.window.set_cursor_visible(true);
                        self.captured = false;
                        self.buttons = Default::default();
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
                let Some(state) = &mut self.state else { return };
                for update in self.streamer.updates() {
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
                        stream::Update::Blocks(changes) => {
                            if let Some(walker) = &mut self.walker {
                                walker.blocks(&changes);
                            }
                        }
                        stream::Update::Server(event) => {
                            if let Some(walker) = &mut self.walker {
                                walker.server_event(event);
                            }
                        }
                        stream::Update::Disconnected(reason) => {
                            eprintln!("disconnected: {reason}");
                            state.window.set_title(&format!(
                                "minecraft-cardboard 0.0.1 (disconnected: {reason})"
                            ));
                        }
                        stream::Update::Idle => {}
                    }
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
                        }
                        if let Some(state) = &mut self.state {
                            let renderer = &mut state.renderer;
                            let target = walker.target();
                            if target != self.outlined {
                                self.outlined = target;
                                renderer.set_outline(&walker.outline_edges());
                            }
                            let hotbar = Some(walker.hotbar());
                            if hotbar != self.shown_hotbar {
                                renderer.set_hotbar(hotbar.clone());
                                self.shown_hotbar = hotbar;
                            }
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
                    None => self.fly(dt),
                }
                self.streamer.follow(self.args.camera.eye);
                let Some(state) = &mut self.state else { return };
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
