//! minecraft-cardboard-launcher: a window in the style of Mojang's Minecraft Launcher, where
//! you set your player name and press Play to start the game, which opens on its title
//! screen. It remembers the name in `launcher.json` beside it, hides while the game runs,
//! and comes back when the game closes, with the game's error if it failed.

#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, Result};
use mc_render::gpu::{Camera, Renderer, View};
use mc_render::hud::HudArt;
use mc_render::hud::launcher::{self, Launcher};
use mc_render::hud::menu::Field;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// `StringUtil.isValidPlayerName`'s length.
const NAME_MAX: usize = 16;

fn main() -> Result<()> {
    let res = mc_assets::Resources::new(mc_assets::default_root());
    let registry = mc_registry::Registry::vanilla()?;
    let models = mc_render::bake::BlockModels::bake(&res, &registry, &[])?;
    let name = load_name();
    let mut app = App {
        hud: models.hud.clone(),
        models: Some(models),
        screen: Launcher {
            name: Field::new(&name),
            ..Default::default()
        },
        state: None,
        cursor: None,
        focused_at: Instant::now(),
        game: None,
    };
    let event_loop = winit::event_loop::EventLoop::new()?;
    event_loop.run_app(&mut app)?;
    Ok(())
}

/// A file beside the launcher.
fn beside_exe(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(name)))
        .unwrap_or_else(|| name.into())
}

/// The saved player name, or the default.
fn load_name() -> String {
    let saved: serde_json::Value = std::fs::read(beside_exe("launcher.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    saved["name"].as_str().unwrap_or("cardboard").to_owned()
}

/// `StringUtil.isValidPlayerName`, and not empty.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= NAME_MAX
        && name.chars().all(|c| (33..127).contains(&(c as u32)))
}

struct WindowState {
    window: Arc<winit::window::Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
}

struct App {
    /// Taken when the window opens, for its renderer.
    models: Option<mc_render::bake::BlockModels>,
    hud: HudArt,
    screen: Launcher,
    state: Option<WindowState>,
    /// Where the mouse pointer is in the window, in pixels.
    cursor: Option<[f32; 2]>,
    /// When a box was last focused or edited, which restarts the cursor's blink.
    focused_at: Instant,
    /// The game, while it runs.
    game: Option<Child>,
}

impl App {
    fn create_window(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) -> Result<()> {
        let attrs = winit::window::Window::default_attributes()
            .with_title("minecraft-cardboard launcher")
            .with_inner_size(winit::dpi::PhysicalSize::new(854, 480));
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
        // A non-sRGB format, as the game uses, so colours match vanilla's.
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
        let models = self.models.take().context("the window is already open")?;
        let renderer = Renderer::new(device, queue, format, &models);
        self.state = Some(WindowState {
            window,
            surface,
            config,
            renderer,
        });
        Ok(())
    }

    /// The GUI's size and the pointer's place in it, in GUI pixels.
    fn gui(&self) -> Option<(i32, i32, Option<[f32; 2]>)> {
        let state = self.state.as_ref()?;
        let (w, h) = (state.config.width, state.config.height);
        let scale = mc_render::gpu::gui_scale(w, h);
        let mouse = self.cursor.map(|c| c.map(|v| v / scale as f32));
        Some((w.div_ceil(scale) as i32, h.div_ceil(scale) as i32, mouse))
    }

    fn play_active(&self) -> bool {
        valid_name(&self.screen.name.value)
    }

    fn focus(&mut self, focused: bool) {
        self.screen.focused = focused;
        self.focused_at = Instant::now();
    }

    /// Starts the game as the player named, remembering the name, and hides until the game
    /// closes.
    fn play(&mut self) {
        if !self.play_active() {
            return;
        }
        let name = self.screen.name.value.clone();
        let settings = serde_json::json!({ "name": name });
        if let Err(e) = std::fs::write(
            beside_exe("launcher.json"),
            serde_json::to_vec_pretty(&settings).unwrap_or_default(),
        ) {
            eprintln!("saving launcher.json: {e}");
        }
        let exe = beside_exe(&format!(
            "minecraft-cardboard{}",
            std::env::consts::EXE_SUFFIX
        ));
        // The game writes its log only when it fails, so an old one would be mistaken for
        // this run's.
        let _ = std::fs::remove_file(beside_exe("minecraft-cardboard.log"));
        match Command::new(&exe).args(["--name", &name]).spawn() {
            Ok(child) => {
                self.game = Some(child);
                self.screen.error = None;
                if let Some(state) = &self.state {
                    state.window.set_visible(false);
                }
            }
            Err(e) => self.screen.error = Some(format!("Couldn't start {}: {e}", exe.display())),
        }
    }

    /// Notices the game closing: shows the launcher again, with the game's error if it
    /// failed: the first line of the log it writes then, or how it ended without one.
    fn check_game(&mut self) {
        let Some(game) = &mut self.game else { return };
        let Ok(Some(status)) = game.try_wait() else {
            return;
        };
        self.game = None;
        if !status.success() {
            let log =
                std::fs::read_to_string(beside_exe("minecraft-cardboard.log")).unwrap_or_default();
            let mut error = match log.lines().find(|l| !l.trim().is_empty()) {
                Some(line) => format!("The game stopped: {}", line.trim()),
                None => format!("The game closed unexpectedly ({status})"),
            };
            if error.chars().count() > 70 {
                error = error.chars().take(67).collect::<String>() + "...";
            }
            self.screen.error = Some(error);
        }
        if let Some(state) = &self.state {
            state.window.set_visible(true);
            state.window.request_redraw();
        }
    }

    fn click(&mut self) {
        let Some((width, height, Some(mouse))) = self.gui() else {
            return;
        };
        let layout = launcher::layout(width, height);
        if launcher::contains(layout.name, mouse) {
            self.focus(true);
            let field = &self.screen.name;
            let cursor =
                mc_render::hud::menu::clicked_position(&self.hud, field, layout.name, mouse[0]);
            self.screen.name.cursor = cursor;
        } else {
            self.focus(false);
        }
        if launcher::contains(layout.play, mouse) {
            self.play();
        }
    }

    /// A key: Enter plays; in the name box, editing as `EditBox` does.
    fn key(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::{Key, NamedKey};
        if event.logical_key == Key::Named(NamedKey::Enter) {
            return self.play();
        }
        if !self.screen.focused {
            return;
        }
        let field = &mut self.screen.name;
        match &event.logical_key {
            Key::Named(NamedKey::Backspace) => field.backspace(),
            Key::Named(NamedKey::Delete) => field.delete(),
            Key::Named(NamedKey::ArrowLeft) => field.step(-1),
            Key::Named(NamedKey::ArrowRight) => field.step(1),
            Key::Named(NamedKey::Home) => field.home(),
            Key::Named(NamedKey::End) => field.end(),
            _ => field.insert(event.text.as_deref().unwrap_or(""), NAME_MAX),
        }
        self.focused_at = Instant::now();
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

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.check_game();
        // Redraw often enough for the cursor's blink, and notice the game closing.
        event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(50),
        ));
        if let Some(state) = &self.state
            && self.game.is_none()
        {
            state.window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        use winit::event::{ElementState, MouseButton, WindowEvent};
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(state) = &mut self.state {
                    state.config.width = size.width.max(1);
                    state.config.height = size.height.max(1);
                    state
                        .surface
                        .configure(&state.renderer.device, &state.config);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Some([position.x as f32, position.y as f32]);
            }
            WindowEvent::CursorLeft { .. } => self.cursor = None,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if self.game.is_none() => self.click(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && self.game.is_none() =>
            {
                self.key(&event);
            }
            WindowEvent::RedrawRequested => {
                let mouse = self.gui().and_then(|g| g.2);
                self.screen.mouse = mouse;
                self.screen.play_active = self.play_active();
                // `TextCursorUtils.isCursorVisible`: on for 300 ms, off for 300.
                self.screen.cursor_on =
                    (self.focused_at.elapsed().as_millis() / 300).is_multiple_of(2);
                let Some(state) = &mut self.state else { return };
                state
                    .renderer
                    .set_screen(Some(mc_render::hud::Screen::Launcher(self.screen.clone())));
                let frame = match state.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
                    _ => {
                        state
                            .surface
                            .configure(&state.renderer.device, &state.config);
                        return;
                    }
                };
                let target = frame.texture.create_view(&Default::default());
                let camera = Camera {
                    eye: glam::Vec3::ZERO,
                    yaw: 0.0,
                    pitch: 0.0,
                    fov: 70.0,
                };
                let view = View {
                    distance: 16.0,
                    ticks: 0.0,
                    hud: false,
                };
                state.renderer.render(
                    &target,
                    state.config.width,
                    state.config.height,
                    &camera,
                    &view,
                );
                state.renderer.queue.present(frame);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::valid_name;

    /// Vanilla's player names: 1 to 16 printable ASCII characters, no spaces.
    #[test]
    fn player_names_follow_vanilla() {
        assert!(valid_name("cardboard"));
        assert!(valid_name("a_b-C.9~"));
        assert!(valid_name("sixteen_chars_ok"));
        assert!(!valid_name("seventeen_chars_x"));
        assert!(!valid_name(""));
        assert!(!valid_name("has space"));
        assert!(!valid_name("naïve"));
    }
}
