//! minecraft-cardboard: opens a vanilla save and lets you fly around it, or renders one
//! frame to a PNG.
//!
//! ```text
//! minecraft-cardboard [--world DIR] [--pos X,Y,Z] [--look YAW,PITCH] [--distance CHUNKS]
//!                     [--fov DEG] [--screenshot OUT.png] [--size WxH] [--ticks N]
//! ```
//! In the window: click to capture the mouse, WASD to move, Space and Shift to go up and
//! down, Ctrl to fly faster, Esc to release the mouse.

use anyhow::{Context, Result, bail};
use glam::Vec3;
use mc_render::bake::BlockModels;

mod skins;
mod stream;
use mc_render::gpu::{Camera, Renderer, View};
use mc_render::mesh::{Mesher, WorldView};
use mc_render::tint::Biomes;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

struct Args {
    world: PathBuf,
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
            "--pos" => match numbers(&value()?)?[..] {
                [x, y, z] => args.camera.eye = Vec3::new(x, y, z),
                _ => bail!("--pos takes X,Y,Z"),
            },
            "--look" => match numbers(&value()?)?[..] {
                [yaw, pitch] => (args.camera.yaw, args.camera.pitch) = (yaw, pitch),
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
/// around the start position already loaded.
struct Scene {
    models: Arc<BlockModels>,
    biomes: Arc<Biomes>,
    registry: Arc<mc_registry::Registry>,
    world: mc_world::World,
    view: WorldView,
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
    let registry = mc_registry::Registry::vanilla()?;
    let mut world = mc_world::World::open(&args.world);
    let eye = args.camera.eye;
    // One chunk more than drawn, so the outermost drawn chunks have neighbours to cull against.
    let view = WorldView::load(
        &mut world,
        &registry,
        eye.x.floor() as i32,
        eye.z.floor() as i32,
        args.distance + 1,
    )?;
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
        registry: Arc::new(registry),
        world,
        view,
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
    let args = parse_args()?;
    let scene = load_scene(&args)?;
    let view = View {
        distance: (args.distance * 16) as f32,
        ticks: args.ticks,
    };
    match &args.screenshot {
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
                .save(path)
                .with_context(|| format!("saving {}", path.display()))?;
            eprintln!("wrote {}", path.display());
            Ok(())
        }
        None => {
            let event_loop = winit::event_loop::EventLoop::new()?;
            let models = scene.models.clone();
            let streamer = stream::Streamer::spawn(
                stream::Sources {
                    world: scene.world,
                    view: scene.view,
                    models: scene.models,
                    biomes: scene.biomes,
                    registry: scene.registry,
                },
                args.distance,
                args.camera.eye,
            );
            let mut app = App {
                args,
                models,
                streamer,
                view,
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
    view: View,
    state: Option<WindowState>,
    keys: std::collections::HashSet<winit::keyboard::KeyCode>,
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
                self.fly(dt);
                // 20 game ticks a second.
                self.view.ticks += dt * 20.0;
                let Some(state) = &mut self.state else { return };
                self.streamer.follow(self.args.camera.eye);
                for update in self.streamer.updates() {
                    match update {
                        stream::Update::Mesh(key, mesh) => state.renderer.upload_chunk(key, &mesh),
                        stream::Update::Remove(key) => state.renderer.remove_chunk(key),
                        stream::Update::Idle => {}
                    }
                }
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
