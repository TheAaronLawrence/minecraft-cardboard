//! minecraft-cardboard-launcher: a window in the style of Mojang's Minecraft Launcher, and
//! the one file a player downloads. It installs the newest release into the game folder, with Minecraft's own textures and data
//! from Mojang, and keeps it up to date, then Play starts the game as the player named,
//! opening on its title screen. It hides while the game runs and comes back when the game
//! closes, with the game's error if it failed.
//!
//! Its own art comes from a few vanilla files built into it, so it draws before the game is
//! installed. Built beside the game, as in this repository's `target/`, it plays that game
//! and installs nothing.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod github;
mod install;
mod mojang;

use anyhow::{Context, Result};
use mc_render::gpu::{Camera, Renderer, View};
use mc_render::hud::HudArt;
use mc_render::hud::launcher::{self, Launcher};
use mc_render::hud::menu::Field;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

/// `StringUtil.isValidPlayerName`'s length.
const NAME_MAX: usize = 16;

/// The vanilla files the launcher draws with, by their paths under `resources/`.
macro_rules! embedded {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!("../../../resources/", $path)))),*]
    };
}
const ART: &[(&str, &[u8])] = embedded![
    "assets/minecraft/font/include/default.json",
    "assets/minecraft/textures/font/ascii.png",
    "assets/minecraft/textures/gui/menu_background.png",
    "assets/minecraft/textures/gui/title/background/panorama_0.png",
    "assets/minecraft/textures/gui/title/minecraft.png",
];

fn main() -> Result<()> {
    let home = home();
    let models = mc_render::bake::BlockModels::gui(&mc_assets::Resources::new(unpack_art(&home)?))?;
    let settings: serde_json::Value = std::fs::read(home.join("launcher.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let name = settings["name"].as_str().unwrap_or("cardboard");
    // A game built beside the launcher is played as it is; otherwise it's installed.
    let beside = std::env::current_exe()?
        .parent()
        .context("the launcher's folder")?
        .to_owned();
    let managed = !install::game_exe(&beside).exists();
    let game_dir = if managed { home.join("game") } else { beside };
    let mut app = App {
        hud: models.hud.clone(),
        models: Some(models),
        screen: Launcher {
            name: Field::new(name),
            installed: if managed {
                install::installed(&game_dir)
            } else {
                Some("Built from source".into())
            },
            ..Default::default()
        },
        state: None,
        cursor: None,
        focused_at: Instant::now(),
        shift: false,
        game: None,
        home,
        game_dir,
        managed,
        worker: None,
    };
    if managed {
        app.start_worker();
    }
    let event_loop = winit::event_loop::EventLoop::new()?;
    event_loop.run_app(&mut app)?;
    Ok(())
}

/// The launcher's folder, which holds its settings and the game: `.minecraft-cardboard` in
/// the roaming app data folder, as `.minecraft` is, or in the home folder off Windows.
/// `CARDBOARD_HOME` overrides it.
fn home() -> PathBuf {
    if let Some(dir) = std::env::var_os("CARDBOARD_HOME") {
        return dir.into();
    }
    let base = std::env::var_os(if cfg!(windows) { "APPDATA" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join(".minecraft-cardboard")
}

/// Writes the built-in art where the asset loaders read files, and returns that folder.
fn unpack_art(home: &Path) -> Result<PathBuf> {
    let root = home.join("launcher").join("resources");
    let files = ART
        .iter()
        .copied()
        // The menus' translations load with the art, but the launcher needs none.
        .chain([("assets/minecraft/lang/en_us.json", &b"{}"[..])]);
    for (path, bytes) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().context("an art file's folder")?)?;
        std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(root)
}

/// `StringUtil.isValidPlayerName`, and not empty.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= NAME_MAX
        && name.chars().all(|c| (33..127).contains(&(c as u32)))
}

/// What the worker thread, which installs and updates the game, tells the window.
enum Message {
    Status(String, Option<f32>),
    /// The game is installed and up to date, at this release.
    Ready(String),
    Failed(String),
}

/// Brings the game in `dir` up to date.
fn work(tx: &Sender<Message>, dir: &Path) -> Result<String> {
    install::update(dir, &mut |step| {
        let (status, progress) = match step {
            install::Step::Checking => ("Checking for updates...".to_owned(), None),
            install::Step::Downloading { what, done, total } => {
                let mb = |b: u64| b as f64 / 1_000_000.0;
                let status = format!("Downloading {what}: {:.1} of {:.1} MB", mb(done), mb(total));
                (status, Some(done as f32 / total.max(1) as f32))
            }
            install::Step::Installing => ("Installing...".to_owned(), None),
        };
        let _ = tx.send(Message::Status(status, progress));
    })
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
    /// Shift is held, for Shift+Tab.
    shift: bool,
    /// The game, while it runs.
    game: Option<Child>,
    /// The launcher's folder, with its settings.
    home: PathBuf,
    /// Where the game is installed, or built.
    game_dir: PathBuf,
    /// The launcher installs and updates the game, rather than playing one built beside it.
    managed: bool,
    /// While installing or updating, what the worker thread says.
    worker: Option<Receiver<Message>>,
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

    /// The big button tries installing again, rather than playing: installing failed.
    fn retries(&self) -> bool {
        self.managed && self.worker.is_none() && self.screen.installed.is_none()
    }

    fn button_active(&self) -> bool {
        self.retries()
            || (self.worker.is_none()
                && self.screen.installed.is_some()
                && valid_name(&self.screen.name.value))
    }

    fn focus(&mut self, focused: bool) {
        self.screen.focused = focused;
        self.screen.play_focused = false;
        self.focused_at = Instant::now();
    }

    /// Saves the name in `launcher.json`.
    fn save(&self) {
        let settings = serde_json::json!({ "name": self.screen.name.value });
        let path = self.home.join("launcher.json");
        let result = std::fs::create_dir_all(&self.home).and_then(|()| {
            std::fs::write(
                &path,
                serde_json::to_vec_pretty(&settings).unwrap_or_default(),
            )
        });
        if let Err(e) = result {
            eprintln!("saving {}: {e}", path.display());
        }
    }

    /// Installs or updates the game, on a thread of its own.
    fn start_worker(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (dir, log) = (self.game_dir.clone(), self.home.join("launcher.log"));
        std::thread::spawn(move || {
            let message = match work(&tx, &dir) {
                Ok(tag) => Message::Ready(tag),
                Err(e) => {
                    // The window shows what went wrong; the log has every detail, such as
                    // the URL or file.
                    if let Err(log_error) = std::fs::write(&log, format!("{e:#}\n")) {
                        eprintln!("writing {}: {log_error}", log.display());
                    }
                    Message::Failed(e.to_string())
                }
            };
            let _ = tx.send(message);
        });
        self.worker = Some(rx);
        self.screen.error = None;
    }

    /// Takes in what the worker has said since the last frame.
    fn check_worker(&mut self) {
        let Some(rx) = &self.worker else { return };
        let messages: Vec<Message> = rx.try_iter().collect();
        for message in messages {
            match message {
                Message::Status(status, progress) => {
                    self.screen.status = Some(status);
                    self.screen.progress = progress;
                }
                Message::Ready(tag) => {
                    self.worker = None;
                    self.screen.installed = Some(tag);
                    (self.screen.status, self.screen.progress) = (None, None);
                }
                Message::Failed(error) => {
                    self.worker = None;
                    (self.screen.status, self.screen.progress) = (None, None);
                    self.screen.error = Some(error);
                    // The game may have been left half updated.
                    self.screen.installed = install::installed(&self.game_dir);
                }
            }
        }
    }

    /// The big button: tries installing again, or plays.
    fn press(&mut self) {
        if !self.button_active() {
            return;
        }
        if self.retries() {
            return self.start_worker();
        }
        self.play();
    }

    /// Starts the game as the player named, remembering the name, and hides until the game
    /// closes.
    fn play(&mut self) {
        let name = self.screen.name.value.clone();
        self.save();
        let exe = install::game_exe(&self.game_dir);
        // The game writes its log only when it fails, so an old one would be mistaken for
        // this run's.
        let _ = std::fs::remove_file(self.game_dir.join("minecraft-cardboard.log"));
        match Command::new(&exe)
            .args(["--name", &name])
            .current_dir(&self.game_dir)
            .spawn()
        {
            Ok(child) => {
                self.game = Some(child);
                self.screen.error = None;
                if let Some(state) = &self.state {
                    state.window.set_visible(false);
                }
            }
            Err(e) => {
                eprintln!("starting {}: {e}", exe.display());
                self.screen.error = Some(format!("Couldn't start the game: {e}"));
            }
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
            let log = std::fs::read_to_string(self.game_dir.join("minecraft-cardboard.log"))
                .unwrap_or_default();
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

    /// Tab or an arrow: the focus to the name box or Play (when it works).
    fn navigate(&mut self, code: winit::keyboard::KeyCode) {
        use mc_render::hud::nav;
        use winit::keyboard::KeyCode as K;
        let Some((width, height, _)) = self.gui() else {
            return;
        };
        let layout = launcher::layout(width, height);
        let item = |rect, active| nav::Item {
            rect,
            active,
            row: None,
        };
        let items = [
            item(layout.name, true),
            item(layout.play, self.button_active()),
        ];
        let focused = match (self.screen.focused, self.screen.play_focused) {
            (true, _) => Some(0),
            (_, true) if items[1].active => Some(1),
            _ => None,
        };
        let direction = match code {
            K::Tab => {
                let next = nav::tab(&items, focused, !self.shift);
                return self.focus_on(next);
            }
            K::ArrowUp => nav::Direction::Up,
            K::ArrowDown => nav::Direction::Down,
            K::ArrowLeft => nav::Direction::Left,
            K::ArrowRight => nav::Direction::Right,
            _ => return,
        };
        let next = nav::arrow(&items, focused, direction, [0, 0, width, height], [0; 4]);
        self.focus_on(next);
    }

    /// Moves the focus to the name box (0) or Play (1); nowhere keeps it where it is.
    fn focus_on(&mut self, next: Option<usize>) {
        match next {
            Some(0) => self.focus(true),
            Some(_) => {
                self.focus(false);
                self.screen.play_focused = true;
            }
            None => {}
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
            self.press();
        }
    }

    /// A key: Enter plays, and so does Space on Play; Tab (Shift+Tab back) and the arrows
    /// move the focus between the name box and Play, as the game's menus do; in the name
    /// box, editing as `EditBox` does, where Left and Right move the cursor.
    fn key(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::{Key, KeyCode as K, NamedKey, PhysicalKey};
        let PhysicalKey::Code(code) = event.physical_key else {
            return;
        };
        if matches!(code, K::Enter | K::NumpadEnter) || code == K::Space && self.screen.play_focused
        {
            return self.press();
        }
        let editing = self.screen.focused && !matches!(code, K::Tab | K::ArrowUp | K::ArrowDown);
        if !editing {
            return self.navigate(code);
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
        self.check_worker();
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
            WindowEvent::ModifiersChanged(modifiers) => {
                self.shift = modifiers.state().shift_key();
            }
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
                self.screen.play_active = self.button_active();
                self.screen.button = if self.retries() { "RETRY" } else { "" }.into();
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
                    bob: glam::Mat4::IDENTITY,
                };
                let view = View::new(16.0, 0.0, false);
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
