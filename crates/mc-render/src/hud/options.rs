//! The options screens: Options (`OptionsScreen`), with its FOV slider and a button for
//! each page of options, of which Music & Sounds and Video Settings work; Music & Sound
//! Options (`SoundOptionsScreen`), whose `OptionsList` holds a volume slider for each sound
//! category; and Video Settings (`VideoSettingsScreen`), its options under the Display,
//! Quality & Performance and Preferences headers, those this game can't change yet shown
//! inactive with the values it uses; Controls (`ControlsScreen`), whose Mouse Settings...
//! opens Mouse Settings (`MouseSettingsScreen`) and Key Binds... opens Key Binds
//! (`KeyBindsScreen`), where each key mapping can be changed or reset. Over the
//! title screen's background from the title screen, over the world's from the pause
//! screen.

use super::keys::{self, Key, MAPPINGS};
use super::{Button, GuiMesh, HudArt, SCREEN};

const MENU_LIST_BACKGROUND: &str = "minecraft:gui/menu_list_background";
const INWORLD_MENU_LIST_BACKGROUND: &str = "minecraft:gui/inworld_menu_list_background";
const INWORLD_HEADER_SEPARATOR: &str = "minecraft:gui/inworld_header_separator";
const INWORLD_FOOTER_SEPARATOR: &str = "minecraft:gui/inworld_footer_separator";

/// The textures drawn whole, by their full ids.
pub(crate) const TEXTURES: [&str; 4] = [
    MENU_LIST_BACKGROUND,
    INWORLD_MENU_LIST_BACKGROUND,
    INWORLD_HEADER_SEPARATOR,
    INWORLD_FOOTER_SEPARATOR,
];

/// `AbstractSliderButton`'s sprites and `AbstractScrollArea`'s scroll bar.
const SLIDER: &str = "widget/slider";
const SLIDER_HANDLE: &str = "widget/slider_handle";
const SLIDER_HANDLE_HIGHLIGHTED: &str = "widget/slider_handle_highlighted";
const SLIDER_HIGHLIGHTED: &str = "widget/slider_highlighted";
const SCROLLER: &str = "widget/scroller";
const SCROLLER_BACKGROUND: &str = "widget/scroller_background";
pub(crate) const SPRITES: [&str; 6] = [
    SLIDER,
    SLIDER_HIGHLIGHTED,
    SLIDER_HANDLE,
    SLIDER_HANDLE_HIGHLIGHTED,
    SCROLLER,
    SCROLLER_BACKGROUND,
];

/// `SoundSource`'s names, in its order: the volumes' captions are `soundCategory.` and these.
pub const CATEGORIES: [&str; 11] = [
    "master", "music", "record", "weather", "block", "hostile", "neutral", "player", "ambient",
    "voice", "ui",
];

/// Which options screen shows.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Page {
    #[default]
    Options,
    Sounds,
    Video,
    Controls,
    Mouse,
    KeyBinds,
}

/// The mouse options, as `Options` keeps them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Mouse {
    /// `sensitivity`: 0 to 1, 0.5 by default.
    pub sensitivity: f64,
    /// `mouseWheelSensitivity`: 10 to the power of a hundredth of -200 to 100, 1 by default.
    pub wheel: f64,
    pub discrete_scroll: bool,
    pub invert_x: bool,
    pub invert_y: bool,
}

impl Default for Mouse {
    fn default() -> Self {
        Self {
            sensitivity: 0.5,
            wheel: 1.0,
            discrete_scroll: false,
            invert_x: false,
            invert_y: false,
        }
    }
}

/// A mouse option a widget changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseOption {
    Sensitivity,
    Wheel,
    DiscreteScroll,
    InvertX,
    InvertY,
}

/// `Options.logMouse` and `unlogMouse`: Scroll Sensitivity's slider steps and its value.
fn log_mouse(step: i32) -> f64 {
    10f64.powf(step as f64 / 100.0)
}

fn unlog_mouse(value: f64) -> i32 {
    (value.log10() * 100.0).floor() as i32
}

impl Mouse {
    /// `CycleButton.onPress`: the option toggled.
    pub fn cycle(&mut self, option: MouseOption) {
        match option {
            MouseOption::DiscreteScroll => self.discrete_scroll = !self.discrete_scroll,
            MouseOption::InvertX => self.invert_x = !self.invert_x,
            MouseOption::InvertY => self.invert_y = !self.invert_y,
            _ => {}
        }
    }

    /// A slider's option set from the slider at `slider` (0 to 1).
    pub fn set_slider(&mut self, option: MouseOption, slider: f64) {
        match option {
            MouseOption::Sensitivity => self.sensitivity = slider,
            MouseOption::Wheel => self.wheel = log_mouse(from_slider(slider, -200, 100)),
            _ => {}
        }
    }

    /// Where a slider's handle sits for the option's value.
    pub fn slider(&self, option: MouseOption) -> f64 {
        match option {
            MouseOption::Sensitivity => self.sensitivity,
            MouseOption::Wheel => to_slider(unlog_mouse(self.wheel), -200, 100),
            _ => 0.0,
        }
    }

    /// `MouseHandler.turnPlayer`: how many degrees the view turns per pixel the mouse
    /// moves (`Entity.turn`'s 0.15 times the sensitivity's cube, times 8), each axis
    /// inverted when its option says.
    pub fn turn(&self, dx: f64, dy: f64) -> (f64, f64) {
        let s = self.sensitivity * 0.6f32 as f64 + 0.2f32 as f64;
        let degrees = s * s * s * 8.0 * 0.15;
        let sign = |inverted: bool| if inverted { -1.0 } else { 1.0 };
        (
            dx * degrees * sign(self.invert_x),
            dy * degrees * sign(self.invert_y),
        )
    }

    /// `MouseHandler.onScroll`: the wheel turned `notches`, one a notch with Discrete
    /// Scrolling, times Scroll Sensitivity.
    pub fn scroll(&self, notches: f64) -> f64 {
        let notches = if self.discrete_scroll {
            if notches == 0.0 {
                0.0
            } else {
                notches.signum()
            }
        } else {
            notches
        };
        notches * self.wheel
    }
}

/// `CloudStatus`, in the order its button cycles through.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Clouds {
    Off,
    Fast,
    #[default]
    Fancy,
}

/// `ParticleStatus`, by its id.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Particles {
    #[default]
    All,
    Decreased,
    Minimal,
}

/// `AttackIndicatorStatus`, by its id.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum AttackIndicator {
    Off,
    #[default]
    Crosshair,
    Hotbar,
}

/// The video options this game uses, as `Options` keeps them.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Video {
    /// `framerateLimit`: 10 to 260, 260 being unlimited.
    pub framerate: i32,
    pub vsync: bool,
    /// `inactivityFpsLimit`: AFK, or only when minimized.
    pub afk_limit: bool,
    /// `guiScale`: 0 for auto, or up to the largest that fits, `gui_scale_max`.
    pub gui_scale: i32,
    pub gui_scale_max: i32,
    pub fullscreen: bool,
    /// `gamma`, the brightness: 0 to 1.
    pub gamma: f64,
    /// `renderDistance`, in chunks.
    pub render_distance: i32,
    pub clouds: Clouds,
    pub particles: Particles,
    /// `cloudRange`, in chunks.
    pub cloud_range: i32,
    pub vignette: bool,
    pub attack_indicator: AttackIndicator,
}

impl Default for Video {
    fn default() -> Self {
        Self {
            framerate: 120,
            vsync: true,
            afk_limit: true,
            gui_scale: 0,
            gui_scale_max: 1,
            fullscreen: false,
            gamma: 0.5,
            render_distance: 12,
            clouds: Clouds::Fancy,
            particles: Particles::All,
            cloud_range: 128,
            vignette: true,
            attack_indicator: AttackIndicator::Crosshair,
        }
    }
}

/// A video option a widget changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VideoOption {
    Framerate,
    Vsync,
    Inactivity,
    GuiScale,
    Fullscreen,
    Gamma,
    RenderDistance,
    Clouds,
    Particles,
    CloudRange,
    Vignette,
    AttackIndicator,
}

impl Video {
    /// `CycleButton.onPress`: the option's next value.
    pub fn cycle(&mut self, option: VideoOption) {
        match option {
            VideoOption::Vsync => self.vsync = !self.vsync,
            VideoOption::Inactivity => self.afk_limit = !self.afk_limit,
            VideoOption::Fullscreen => self.fullscreen = !self.fullscreen,
            VideoOption::Vignette => self.vignette = !self.vignette,
            VideoOption::GuiScale => {
                self.gui_scale = (self.gui_scale + 1) % (self.gui_scale_max + 1);
            }
            VideoOption::Clouds => {
                self.clouds = match self.clouds {
                    Clouds::Off => Clouds::Fast,
                    Clouds::Fast => Clouds::Fancy,
                    Clouds::Fancy => Clouds::Off,
                }
            }
            VideoOption::Particles => {
                self.particles = match self.particles {
                    Particles::All => Particles::Decreased,
                    Particles::Decreased => Particles::Minimal,
                    Particles::Minimal => Particles::All,
                }
            }
            VideoOption::AttackIndicator => {
                self.attack_indicator = match self.attack_indicator {
                    AttackIndicator::Off => AttackIndicator::Crosshair,
                    AttackIndicator::Crosshair => AttackIndicator::Hotbar,
                    AttackIndicator::Hotbar => AttackIndicator::Off,
                }
            }
            _ => {}
        }
    }

    /// A slider's option set from the slider at `slider` (0 to 1), in its steps.
    pub fn set_slider(&mut self, option: VideoOption, slider: f64) {
        match option {
            VideoOption::Framerate => self.framerate = from_slider(slider, 1, 26) * 10,
            VideoOption::Gamma => self.gamma = slider,
            VideoOption::RenderDistance => self.render_distance = from_slider(slider, 2, 32),
            VideoOption::CloudRange => self.cloud_range = from_slider(slider, 2, 128),
            _ => {}
        }
    }

    /// Where a slider's handle sits for the option's value.
    pub fn slider(&self, option: VideoOption) -> f64 {
        match option {
            VideoOption::Framerate => to_slider(self.framerate / 10, 1, 26),
            VideoOption::Gamma => self.gamma,
            VideoOption::RenderDistance => to_slider(self.render_distance, 2, 32),
            VideoOption::CloudRange => to_slider(self.cloud_range, 2, 128),
            _ => 0.0,
        }
    }
}

/// What the options screens show.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Options {
    pub page: Page,
    /// Opened from the pause screen, over the world.
    pub in_world: bool,
    /// Each sound category's volume, 0 to 1, in `CATEGORIES`' order.
    pub volumes: [f64; 11],
    pub fov: i32,
    pub video: Video,
    pub mouse_options: Mouse,
    /// `toggleCrouch` and `toggleSprint`: Sneak and Sprint toggle instead of being held.
    pub toggle_sneak: bool,
    pub toggle_sprint: bool,
    /// `toggleAttack` and `toggleUse`: Attack/Destroy and Use Item/Place Block toggle.
    pub toggle_attack: bool,
    pub toggle_use: bool,
    /// The slider being dragged and where it is, 0 to 1: a stepped option only follows it
    /// in steps, and the handle settles on the step when it's let go.
    pub dragging: Option<(Action, f64)>,
    /// How far the options list has scrolled, in GUI pixels.
    pub scroll: f64,
    /// The widget last clicked or moved to with the keyboard, which stays highlighted
    /// (`Screen.setFocused`).
    pub focused: Option<Action>,
    /// The focused slider takes the left and right arrows
    /// (`AbstractSliderButton.canChangeValue`).
    pub can_change: bool,
    /// The focus of each page under this one, which comes back with it (a screen returned
    /// to keeps its focus).
    pub parent_focus: Vec<Option<Action>>,
    /// Where the mouse is, in GUI pixels, to highlight what's under it.
    pub mouse: Option<[f32; 2]>,
    /// Milliseconds on a steady clock, which scroll labels too wide for their buttons.
    pub millis: u64,
    /// Each key mapping's key, in `keys::MAPPINGS`' order.
    pub keys: Vec<Key>,
    /// The key mapping waiting for a key on Key Binds (`KeyBindsScreen.selectedKey`).
    pub selected: Option<usize>,
}

/// What an options screen's widget does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Fov,
    /// A sound category's volume, by its place in `CATEGORIES`.
    Volume(usize),
    Video(VideoOption),
    Mouse(MouseOption),
    OpenSounds,
    OpenVideo,
    OpenControls,
    OpenMouse,
    OpenKeyBinds,
    ToggleSneak,
    ToggleSprint,
    ToggleAttack,
    ToggleUse,
    /// A key mapping's button, by its place in `keys::MAPPINGS`: it waits for a key.
    KeyChange(usize),
    /// A key mapping's Reset button.
    KeyReset(usize),
    /// Reset Keys: every mapping back to its default.
    ResetKeys,
    Done,
    /// An option or screen this game doesn't have yet.
    None,
}

/// A widget: what it does, where it is, its label, whether it works, its slider's position
/// if it's a slider, and whether it's in the scrolling list, which hides it outside the
/// list.
pub struct Widget {
    pub action: Action,
    pub button: Button,
    pub label: String,
    pub active: bool,
    pub slider: Option<f64>,
    pub in_list: bool,
    /// The label in coloured, maybe underlined pieces, when it isn't plain white.
    pub spans: Vec<Span>,
    /// In the scrolling list, its entry: which, and the entry's top and height.
    pub entry: Option<(usize, i32, i32)>,
}

impl Widget {
    /// What keyboard navigation needs of it.
    pub fn nav(&self) -> super::nav::Item {
        super::nav::Item {
            rect: self.button.rect,
            active: self.active,
            row: self.entry.map(|e| e.0),
        }
    }
}

/// `AbstractSelectionList.scrollToEntry`: the scroll that shows all of the entry with its
/// top at `y` (scrolled by `scroll`) and `height` high, 2 in from the list's ends.
pub fn scroll_to(page: Page, scroll: f64, (y, h): (i32, i32), height: i32) -> f64 {
    let (top, list_height) = list_area(height);
    let mut scroll = scroll;
    let top_delta = y - top - 2;
    if top_delta < 0 {
        scroll += top_delta as f64;
    }
    let bottom_delta = top + list_height - y - h - 2;
    if bottom_delta < 0 {
        scroll -= bottom_delta as f64;
    }
    scroll.clamp(0.0, max_scroll(page, height))
}

/// A piece of a label: its text, colour, and whether it's underlined.
pub struct Span {
    pub text: String,
    pub rgb: [f32; 3],
    pub underline: bool,
}

/// `ChatFormatting.YELLOW`.
const YELLOW: [f32; 3] = [1.0, 1.0, 85.0 / 255.0];

/// An `OptionsList` entry: a header, one option across the list, or two side by side.
enum Entry {
    Header(String),
    Big(Widget),
    Small(Widget, Option<Widget>),
}

/// `OptionsList`'s rows are 25 high and scroll 12 a notch (`defaultEntryHeight / 2`).
const ROW: i32 = 25;
/// `KeyBindsList`'s rows are 20 high, 340 wide.
const KEY_ROW: i32 = 20;
const KEY_ROW_WIDTH: i32 = 340;
pub const SCROLL_RATE: f64 = 12.0;

/// `OptionInstance.IntRangeBase.toSliderValue`.
pub fn to_slider(value: i32, min: i32, max: i32) -> f64 {
    if value == min {
        0.0
    } else if value == max {
        1.0
    } else {
        (value as f64 + 0.5 - min as f64) / (max as f64 + 1.0 - min as f64)
    }
}

/// `OptionInstance.IntRangeBase.fromSliderValue`.
pub fn from_slider(slider: f64, min: i32, max: i32) -> i32 {
    let slider = if slider >= 1.0 {
        0.99999f32 as f64
    } else {
        slider
    };
    (min as f64 + slider * (max as f64 + 1.0 - min as f64)).floor() as i32
}

/// The FOV's range, 30 to 110.
pub fn fov_to_slider(fov: i32) -> f64 {
    to_slider(fov, 30, 110)
}

pub fn fov_from_slider(slider: f64) -> i32 {
    from_slider(slider, 30, 110)
}

/// `AbstractSliderButton.setValueFromMouse`: the position a slider at `rect` takes under
/// the mouse at `x`.
pub fn slider_at(rect: [i32; 4], x: f32) -> f64 {
    ((x as f64 - (rect[0] + 4) as f64) / (rect[2] - 8) as f64).clamp(0.0, 1.0)
}

/// The options list's top and height: between the 33-high header and footer
/// (`HeaderAndFooterLayout`).
pub fn list_area(height: i32) -> (i32, i32) {
    (33, height - 66)
}

/// `OptionsList.addHeader`: 18 high, with 18 more above all but the first.
fn header_height(first: bool) -> i32 {
    if first { 18 } else { 36 }
}

/// `AbstractSelectionList.contentHeight`: the entries' heights and 4 more.
fn content_height(page: Page) -> i32 {
    4 + match page {
        Page::Options => 0,
        // Mouse Settings... and Key Binds..., then four rows of two options.
        Page::Controls => 5 * ROW,
        // Three rows of two.
        Page::Mouse => 3 * ROW,
        // Each category's header over its mappings.
        Page::KeyBinds => (MAPPINGS.len() + keys::category_count()) as i32 * KEY_ROW,
        // Master Volume, five rows of two volumes, the device, and two rows of two.
        Page::Sounds => 9 * ROW,
        // Display: Exclusive Fullscreen Mode and four rows of two; Quality & Performance:
        // Preset and nine rows; Preferences: two rows.
        Page::Video => {
            header_height(true)
                + 5 * ROW
                + header_height(false)
                + 10 * ROW
                + header_height(false)
                + 2 * ROW
        }
    }
}

/// `AbstractScrollArea.maxScrollAmount`.
pub fn max_scroll(page: Page, height: i32) -> f64 {
    (content_height(page) - list_area(height).1).max(0) as f64
}

/// `AbstractScrollArea.scrollerHeight`.
fn scroller_height(page: Page, height: i32) -> i32 {
    let h = list_area(height).1;
    (((h * h) as f32 / content_height(page) as f32) as i32).clamp(32, h - 8)
}

/// The scroll bar's column (x, y, width, height), right of the rows
/// (`AbstractSelectionList.scrollBarX`), when the list scrolls.
pub fn scrollbar(page: Page, width: i32, height: i32) -> Option<[i32; 4]> {
    let (top, h) = list_area(height);
    let row_width = if page == Page::KeyBinds {
        KEY_ROW_WIDTH
    } else {
        310
    };
    (page != Page::Options && max_scroll(page, height) > 0.0).then_some([
        width / 2 + row_width / 2 + 6 + 2,
        top,
        6,
        h,
    ])
}

/// `AbstractScrollArea.mouseDragged`: how far the list scrolls for each GUI pixel the scroll
/// bar is dragged.
pub fn scroll_drag_scale(page: Page, height: i32) -> f64 {
    let bar = scroller_height(page, height);
    (max_scroll(page, height).max(1.0) / (list_area(height).1 - bar) as f64).max(1.0)
}

/// `Component.translatable`'s formatting of a translation with `%s` and `%1$s` arguments.
fn format(template: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut next = 0;
    let mut rest = template;
    while let Some(i) = rest.find('%') {
        out += &rest[..i];
        rest = &rest[i + 1..];
        if let Some(r) = rest.strip_prefix('%') {
            out.push('%');
            rest = r;
        } else if let Some(r) = rest.strip_prefix('s') {
            out += args.get(next).copied().unwrap_or_default();
            next += 1;
            rest = r;
        } else if let Some((n, r)) = rest.split_once("$s")
            && let Ok(n) = n.parse::<usize>()
        {
            out += args.get(n - 1).copied().unwrap_or_default();
            rest = r;
        } else {
            out.push('%');
        }
    }
    out + rest
}

/// The widgets' labels, from the translations.
struct Labels<'a>(&'a HudArt);

impl Labels<'_> {
    fn lang(&self, key: &str) -> String {
        self.0
            .lang
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_owned())
    }

    /// `Options.genericValueLabel` and `CycleButton`'s name and value.
    fn generic(&self, caption: &str, value: &str) -> String {
        format(
            &self.lang("options.generic_value"),
            &[&self.lang(caption), value],
        )
    }

    fn keyed(&self, caption: &str, value_key: &str) -> String {
        self.generic(caption, &self.lang(value_key))
    }

    fn on_off(&self, caption: &str, on: bool) -> String {
        self.keyed(caption, if on { "options.on" } else { "options.off" })
    }

    fn chunks(&self, caption: &str, n: i32) -> String {
        self.generic(
            caption,
            &format(&self.lang("options.chunks"), &[&n.to_string()]),
        )
    }

    fn percent(&self, caption: &str, value: f64) -> String {
        let percent = ((value * 100.0) as i32).to_string();
        format(
            &self.lang("options.percent_value"),
            &[&self.lang(caption), &percent],
        )
    }

    /// The label of the slider or cycle button for `action`, with the slider at `slider`.
    fn of(&self, action: Action, o: &Options, slider: f64) -> String {
        let v = &o.video;
        match action {
            Action::Fov => match fov_from_slider(slider) {
                70 => self.keyed("options.fov", "options.fov.min"),
                110 => self.keyed("options.fov", "options.fov.max"),
                fov => self.generic("options.fov", &fov.to_string()),
            },
            // `Options.percentValueOrOffLabel`.
            Action::Volume(i) => {
                let caption = format!("soundCategory.{}", CATEGORIES[i]);
                match slider {
                    0.0 => self.keyed(&caption, "options.off"),
                    v => self.percent(&caption, v),
                }
            }
            Action::Mouse(option) => {
                let mut m = o.mouse_options;
                m.set_slider(option, slider);
                match option {
                    MouseOption::Sensitivity => match m.sensitivity {
                        0.0 => self.keyed("options.sensitivity", "options.sensitivity.min"),
                        1.0 => self.keyed("options.sensitivity", "options.sensitivity.max"),
                        s => self.percent("options.sensitivity", 2.0 * s),
                    },
                    MouseOption::Wheel => {
                        self.generic("options.mouseWheelSensitivity", &format!("{:.2}", m.wheel))
                    }
                    MouseOption::DiscreteScroll => {
                        self.on_off("options.discrete_mouse_scroll", m.discrete_scroll)
                    }
                    MouseOption::InvertX => self.on_off("options.invertMouseX", m.invert_x),
                    MouseOption::InvertY => self.on_off("options.invertMouseY", m.invert_y),
                }
            }
            Action::Video(option) => {
                let mut v = *v;
                v.set_slider(option, slider);
                match option {
                    VideoOption::Framerate => match v.framerate {
                        260 => self.keyed("options.framerateLimit", "options.framerateLimit.max"),
                        n => self.generic(
                            "options.framerateLimit",
                            &format(&self.lang("options.framerate"), &[&n.to_string()]),
                        ),
                    },
                    VideoOption::Vsync => self.on_off("options.vsync", v.vsync),
                    VideoOption::Inactivity => self.keyed(
                        "options.inactivityFpsLimit",
                        if v.afk_limit {
                            "options.inactivityFpsLimit.afk"
                        } else {
                            "options.inactivityFpsLimit.minimized"
                        },
                    ),
                    VideoOption::GuiScale => match v.gui_scale {
                        0 => self.keyed("options.guiScale", "options.guiScale.auto"),
                        n => self.generic("options.guiScale", &n.to_string()),
                    },
                    VideoOption::Fullscreen => self.on_off("options.fullscreen", v.fullscreen),
                    VideoOption::Gamma => match (v.gamma * 100.0) as i32 {
                        0 => self.keyed("options.gamma", "options.gamma.min"),
                        50 => self.keyed("options.gamma", "options.gamma.default"),
                        100 => self.keyed("options.gamma", "options.gamma.max"),
                        n => self.generic("options.gamma", &n.to_string()),
                    },
                    VideoOption::RenderDistance => {
                        self.chunks("options.renderDistance", v.render_distance)
                    }
                    VideoOption::Clouds => self.keyed(
                        "options.renderClouds",
                        match v.clouds {
                            Clouds::Off => "options.off",
                            Clouds::Fast => "options.clouds.fast",
                            Clouds::Fancy => "options.clouds.fancy",
                        },
                    ),
                    VideoOption::Particles => self.keyed(
                        "options.particles",
                        match v.particles {
                            Particles::All => "options.particles.all",
                            Particles::Decreased => "options.particles.decreased",
                            Particles::Minimal => "options.particles.minimal",
                        },
                    ),
                    VideoOption::CloudRange => {
                        self.chunks("options.renderCloudsDistance", v.cloud_range)
                    }
                    VideoOption::Vignette => self.on_off("options.vignette", v.vignette),
                    VideoOption::AttackIndicator => self.keyed(
                        "options.attackIndicator",
                        match v.attack_indicator {
                            AttackIndicator::Off => "options.off",
                            AttackIndicator::Crosshair => "options.attack.crosshair",
                            AttackIndicator::Hotbar => "options.attack.hotbar",
                        },
                    ),
                }
            }
            _ => String::new(),
        }
    }
}

fn widget(action: Action, width: i32, label: String, active: bool) -> Widget {
    Widget {
        action,
        button: Button {
            rect: [0, 0, width, 20],
            label: "",
        },
        label,
        active,
        slider: None,
        in_list: true,
        spans: Vec::new(),
        entry: None,
    }
}

/// A slider showing its option's value, or where it's being dragged.
fn slider(o: &Options, labels: &Labels, action: Action, width: i32) -> Widget {
    let value = match o.dragging {
        Some((dragged, at)) if dragged == action => at,
        _ => match action {
            Action::Fov => fov_to_slider(o.fov),
            Action::Volume(i) => o.volumes[i],
            Action::Video(option) => o.video.slider(option),
            Action::Mouse(option) => o.mouse_options.slider(option),
            _ => 0.0,
        },
    };
    Widget {
        slider: Some(value),
        ..widget(action, width, labels.of(action, o, value), true)
    }
}

/// An option this game can't change yet: a cycle button, or a slider at `slider`.
fn fixed(label: String, slider: Option<f64>, width: i32) -> Widget {
    Widget {
        slider,
        ..widget(Action::None, width, label, false)
    }
}

/// `VideoSettingsScreen.addOptions`'s entries.
fn video_entries(o: &Options, l: &Labels) -> Vec<Entry> {
    let video = |option| slider_or_cycle(o, l, Action::Video(option));
    let small =
        |key: &str, value: String, slider: Option<f64>| fixed(l.generic(key, &value), slider, 150);
    let on_off = |key: &str, on| fixed(l.on_off(key, on), None, 150);
    let pair = |a, b| Entry::Small(a, Some(b));
    vec![
        Entry::Header(l.lang("options.video.display.header")),
        Entry::Big(fixed(
            l.keyed(
                "options.fullscreen.exclusive.mode",
                "options.fullscreen.current",
            ),
            Some(0.0),
            310,
        )),
        pair(video(VideoOption::Framerate), video(VideoOption::Vsync)),
        pair(video(VideoOption::Inactivity), video(VideoOption::GuiScale)),
        pair(
            video(VideoOption::Fullscreen),
            on_off("options.exclusiveFullscreen", false),
        ),
        pair(
            video(VideoOption::Gamma),
            small(
                "options.graphicsApi",
                l.lang("options.graphicsApi.default"),
                None,
            ),
        ),
        Entry::Header(l.lang("options.video.quality.header")),
        // Never a preset: there are no entity shadows, menu blur or chunk fade.
        Entry::Big(fixed(
            l.keyed("options.graphics.preset", "options.graphics.custom"),
            Some(1.0),
            310,
        )),
        pair(
            small(
                "options.biomeBlendRadius",
                l.lang("options.biomeBlendRadius.5"),
                Some(to_slider(2, 0, 7)),
            ),
            video(VideoOption::RenderDistance),
        ),
        pair(
            small(
                "options.prioritizeChunkUpdates",
                l.lang("options.prioritizeChunkUpdates.none"),
                None,
            ),
            fixed(
                l.chunks("options.simulationDistance", 12),
                Some(to_slider(12, 5, 32)),
                150,
            ),
        ),
        pair(on_off("options.ao", true), video(VideoOption::Clouds)),
        pair(
            video(VideoOption::Particles),
            small("options.mipmapLevels", "4".into(), Some(1.0)),
        ),
        pair(
            on_off("options.entityShadows", false),
            fixed(
                l.percent("options.entityDistanceScaling", 1.0),
                Some(to_slider(4, 2, 20)),
                150,
            ),
        ),
        pair(
            fixed(
                l.keyed(
                    "options.accessibility.menu_background_blurriness",
                    "options.off",
                ),
                Some(0.0),
                150,
            ),
            video(VideoOption::CloudRange),
        ),
        pair(
            on_off("options.cutoutLeaves", true),
            on_off("options.improvedTransparency", false),
        ),
        pair(
            small(
                "options.textureFiltering",
                l.lang("options.textureFiltering.none"),
                None,
            ),
            small(
                "options.maxAnisotropy",
                format(&l.lang("options.multiplier"), &["4"]),
                Some(to_slider(2, 1, 3)),
            ),
        ),
        Entry::Small(
            small(
                "options.weatherRadius",
                format(&l.lang("options.blocks"), &["10"]),
                Some(1.0),
            ),
            None,
        ),
        Entry::Header(l.lang("options.video.preferences.header")),
        pair(
            on_off("options.autosaveIndicator", true),
            video(VideoOption::Vignette),
        ),
        pair(
            video(VideoOption::AttackIndicator),
            fixed(l.lang("options.chunkFade.none"), Some(0.0), 150),
        ),
    ]
}

/// A working option's widget: a slider, or a cycle button.
fn slider_or_cycle(o: &Options, l: &Labels, action: Action) -> Widget {
    match action {
        Action::Video(
            VideoOption::Framerate
            | VideoOption::Gamma
            | VideoOption::RenderDistance
            | VideoOption::CloudRange,
        ) => slider(o, l, action, 150),
        _ => widget(action, 150, l.of(action, o, 0.0), true),
    }
}

/// `SoundOptionsScreen.addOptions`: Master Volume across the list, the other volumes two to
/// a row, the device across, then Closed Captions and Directional Audio, and Music
/// Frequency and Music Toast, which don't work yet.
fn sound_entries(o: &Options, l: &Labels) -> Vec<Entry> {
    let mut entries = vec![Entry::Big(slider(o, l, Action::Volume(0), 310))];
    for i in (1..11).step_by(2) {
        entries.push(Entry::Small(
            slider(o, l, Action::Volume(i), 150),
            Some(slider(o, l, Action::Volume(i + 1), 150)),
        ));
    }
    let cycle = |caption: &str, value: &str, width| fixed(l.keyed(caption, value), None, width);
    entries.push(Entry::Big(cycle(
        "options.audioDevice",
        "options.audioDevice.default",
        310,
    )));
    entries.push(Entry::Small(
        cycle("options.showSubtitles", "options.off", 150),
        Some(cycle("options.directionalAudio", "options.off", 150)),
    ));
    entries.push(Entry::Small(
        cycle(
            "options.music_frequency",
            "options.music_frequency.default",
            150,
        ),
        Some(cycle("options.musicToast", "options.musicToast.never", 150)),
    ));
    entries
}

/// `ControlsScreen.addOptions`: Mouse Settings... and Key Binds..., the Hold or Toggle of
/// Sneak, Sprint, Attack/Destroy and Use Item/Place Block, then the options this game
/// doesn't have yet, at their defaults.
fn controls_entries(o: &Options, l: &Labels) -> Vec<Entry> {
    let mode = |toggle| match toggle {
        true => "options.key.toggle",
        false => "options.key.hold",
    };
    let toggle = |action, caption: &str, on| widget(action, 150, l.keyed(caption, mode(on)), true);
    let on_off = |caption: &str, on| fixed(l.on_off(caption, on), None, 150);
    let pair = |a, b| Entry::Small(a, Some(b));
    let sprint_window = format(&l.lang("options.value"), &["7"]);
    vec![
        pair(
            widget(
                Action::OpenMouse,
                150,
                l.lang("options.mouse_settings"),
                true,
            ),
            widget(Action::OpenKeyBinds, 150, l.lang("controls.keybinds"), true),
        ),
        pair(
            toggle(Action::ToggleSneak, "key.sneak", o.toggle_sneak),
            toggle(Action::ToggleSprint, "key.sprint", o.toggle_sprint),
        ),
        pair(
            toggle(Action::ToggleAttack, "key.attack", o.toggle_attack),
            toggle(Action::ToggleUse, "key.use", o.toggle_use),
        ),
        pair(
            on_off("options.autoJump", false),
            fixed(
                l.generic("options.sprintWindow", &sprint_window),
                Some(to_slider(7, 0, 10)),
                150,
            ),
        ),
        pair(
            on_off("options.quitShortcuts", true),
            on_off("options.operatorItemsTab", false),
        ),
    ]
}

/// `MouseSettingsScreen.addOptions`: Sensitivity and Scroll Sensitivity, Discrete
/// Scrolling and the inverts two to a row, and Allow Cursor Changes, which this game
/// doesn't do.
fn mouse_entries(o: &Options, l: &Labels) -> Vec<Entry> {
    let slider = |option| slider(o, l, Action::Mouse(option), 150);
    let cycle = |option| {
        let action = Action::Mouse(option);
        widget(action, 150, l.of(action, o, 0.0), true)
    };
    vec![
        Entry::Small(
            slider(MouseOption::Sensitivity),
            Some(slider(MouseOption::Wheel)),
        ),
        Entry::Small(
            cycle(MouseOption::DiscreteScroll),
            Some(cycle(MouseOption::InvertX)),
        ),
        Entry::Small(
            cycle(MouseOption::InvertY),
            Some(fixed(
                l.on_off("options.allowCursorChanges", true),
                None,
                150,
            )),
        ),
    ]
}

/// `KeyBindsList`: each category's name, then its mappings, each with its name on the left
/// and on the right its key's button, 75 wide, and Reset, 50 wide, 5 apart and 10 left of
/// the scroll bar (`KeyEntry.extractContent`). A key shared with another, once either has
/// changed, is bracketed in yellow with a yellow stripe beside it; the mapping waiting for
/// a key shows "> key <".
fn key_binds(o: &Options, l: &Labels, width: i32) -> Layout {
    let mut out = Layout::default();
    let mut y = list_area(0).0 + 2 - o.scroll as i32;
    let row_left = width / 2 - KEY_ROW_WIDTH / 2;
    let reset_x = row_left + KEY_ROW_WIDTH + 6 + 2 - 50 - 10;
    let change_x = reset_x - 5 - 75;
    let mut category = None;
    let bound: Vec<Key> = (0..MAPPINGS.len())
        .map(|i| o.keys.get(i).copied().unwrap_or(MAPPINGS[i].default))
        .collect();
    for i in keys::sorted(&l.0.lang) {
        let mapping = &MAPPINGS[i];
        if category != Some(mapping.category) {
            category = Some(mapping.category);
            // `CategoryEntry`: a `FocusableTextWidget`, 17 high, at the entry's content
            // bottom.
            out.headers
                .push((l.lang(mapping.category.label()), y + 2 + 16 - 17 + 4));
            y += KEY_ROW;
        }
        let key = bound[i];
        let name = key.display(&l.0.lang);
        let clash = keys::clashes(&bound, i);
        let span = |text: &str, rgb, underline| Span {
            text: text.to_owned(),
            rgb,
            underline,
        };
        let mut spans = if clash {
            vec![
                span("[ ", YELLOW, false),
                span(&name, [1.0; 3], false),
                span(" ]", YELLOW, false),
            ]
        } else {
            vec![span(&name, [1.0; 3], false)]
        };
        if o.selected == Some(i) {
            for s in &mut spans {
                (s.rgb, s.underline) = ([1.0; 3], true);
            }
            spans.insert(0, span("> ", YELLOW, false));
            spans.push(span(" <", YELLOW, false));
        }
        let label: String = spans.iter().map(|s| s.text.as_str()).collect();
        let mut change = widget(Action::KeyChange(i), 75, label, true);
        change.entry = Some((i, y, KEY_ROW));
        change.button.rect[..2].copy_from_slice(&[change_x, y + 2 - 2]);
        if clash || o.selected == Some(i) {
            change.spans = spans;
        }
        out.widgets.push(change);
        let default = key == mapping.default;
        let mut reset = widget(Action::KeyReset(i), 50, l.lang("controls.reset"), !default);
        reset.button.rect[..2].copy_from_slice(&[reset_x, y + 2 - 2]);
        reset.entry = Some((i, y, KEY_ROW));
        out.widgets.push(reset);
        out.texts
            .push((l.lang(mapping.name), row_left + 2, y + 2 + 8 - 4));
        if clash {
            out.stripes.push([change_x - 6, y + 2 - 1, 3, 17]);
        }
        y += KEY_ROW;
    }
    out
}

/// The list's entries placed (`AbstractSelectionList.repositionEntries`): its widgets, each
/// 2 into its entry and left of the centre by 155 then 160 apart
/// (`OptionsList.Entry.extractContent`); and the headers' text and where it goes, centred
/// in a box padded 4 (`OptionsList.HeaderEntry`).
fn place(entries: Vec<Entry>, scroll: f64, width: i32) -> Layout {
    let mut y = list_area(0).0 + 2 - scroll as i32;
    let (mut widgets, mut headers) = (Vec::new(), Vec::new());
    let left = width / 2 - 155;
    for (i, entry) in entries.into_iter().enumerate() {
        let mut put = |mut w: Widget, x| {
            w.button.rect[0] = x;
            w.button.rect[1] = y + 2;
            w.entry = Some((i, y, ROW));
            widgets.push(w);
        };
        match entry {
            Entry::Header(text) => {
                let height = header_height(i == 0);
                headers.push((text, y + 2 + height - 18 + 4));
                y += height;
                continue;
            }
            Entry::Big(w) => put(w, left),
            Entry::Small(a, b) => {
                put(a, left);
                if let Some(b) = b {
                    put(b, left + 160);
                }
            }
        }
        y += ROW;
    }
    Layout {
        widgets,
        headers,
        ..Default::default()
    }
}

/// What a screen shows: its widgets; the list's headers and where they go; plain text,
/// such as the key mappings' names, and where; and the yellow stripes beside clashing keys.
#[derive(Default)]
struct Layout {
    widgets: Vec<Widget>,
    headers: Vec<(String, i32)>,
    texts: Vec<(String, i32, i32)>,
    stripes: Vec<[i32; 4]>,
}

/// What the screen shows in a GUI `width` × `height` GUI pixels.
fn layout(o: &Options, art: &HudArt, width: i32, height: i32) -> Layout {
    let l = Labels(art);
    // `HeaderAndFooterLayout`'s footer: Done, 200 wide, centred in the bottom 33.
    let done = Widget {
        button: Button {
            rect: [(width - 200) / 2, height - 33 + 7, 200, 20],
            label: "",
        },
        in_list: false,
        ..widget(Action::Done, 200, l.lang("gui.done"), true)
    };
    let mut layout = match o.page {
        // `OptionsScreen.init`: in the 61-high header, the title over the FOV slider and
        // Online..., 8 apart; below it, the buttons two to a row in cells padded 4 left,
        // right and below, 30 under the header if there's room.
        Page::Options => {
            let hx = (width - 308) / 2;
            let at = |mut w: Widget, x, y| {
                w.button.rect[0] = x;
                w.button.rect[1] = y;
                w.in_list = false;
                w
            };
            let mut widgets = vec![
                at(slider(o, &l, Action::Fov, 150), hx, 29),
                at(fixed(l.lang("options.online"), None, 150), hx + 158, 29),
            ];
            let gx = (width - 316) / 2;
            let gy = (61 + 30).min(height - 33 - 5 * 24);
            let buttons = [
                "options.skinCustomisation",
                "options.sounds",
                "options.video",
                "options.controls",
                "options.language",
                "options.chat",
                "options.resourcepack",
                "options.accessibility",
                "options.telemetry",
                "options.credits_and_attribution",
            ];
            for (i, key) in buttons.into_iter().enumerate() {
                let (col, row) = (i as i32 % 2, i as i32 / 2);
                let action = match key {
                    "options.sounds" => Action::OpenSounds,
                    "options.video" => Action::OpenVideo,
                    "options.controls" => Action::OpenControls,
                    _ => Action::None,
                };
                let w = widget(action, 150, l.lang(key), action != Action::None);
                widgets.push(at(w, gx + 4 + col * 158, gy + row * 24));
            }
            Layout {
                widgets,
                ..Default::default()
            }
        }
        Page::Sounds => place(sound_entries(o, &l), o.scroll, width),
        Page::Video => place(video_entries(o, &l), o.scroll, width),
        Page::Controls => place(controls_entries(o, &l), o.scroll, width),
        Page::Mouse => place(mouse_entries(o, &l), o.scroll, width),
        Page::KeyBinds => key_binds(o, &l, width),
    };
    if o.page == Page::KeyBinds {
        // `KeyBindsScreen.addFooter`: Reset Keys, while any key isn't its default, and
        // Done, 150 wide and 8 apart.
        let changed = MAPPINGS
            .iter()
            .enumerate()
            .any(|(i, m)| o.keys.get(i).is_some_and(|&k| k != m.default));
        let x = (width - 308) / 2;
        let y = height - 33 + 7;
        let footer = |action, label, active, x| Widget {
            button: Button {
                rect: [x, y, 150, 20],
                label: "",
            },
            in_list: false,
            ..widget(action, 150, label, active)
        };
        let reset = footer(Action::ResetKeys, l.lang("controls.resetAll"), changed, x);
        layout.widgets.push(reset);
        let done = footer(Action::Done, l.lang("gui.done"), true, x + 158);
        layout.widgets.push(done);
    } else {
        layout.widgets.push(done);
    }
    layout
}

/// The screen's widgets in a GUI `width` × `height` GUI pixels.
pub fn widgets(o: &Options, art: &HudArt, width: i32, height: i32) -> Vec<Widget> {
    layout(o, art, width, height).widgets
}

/// Where `a` and `b` (x, y, width, height) overlap.
fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = (
        (a[0] + a[2]).min(b[0] + b[2]),
        (a[1] + a[3]).min(b[1] + b[3]),
    );
    [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
}

impl GuiMesh {
    pub(crate) fn options(&mut self, art: &HudArt, o: &Options, width: i32, height: i32) {
        // `Screen.extractBackground`: the title screen's background, or over the world the
        // in-world menu background (vanilla blurs the world first, which this doesn't).
        if o.in_world {
            self.inworld_background(art, width, height);
        } else {
            self.background(art, width, height);
        }
        let lang = |k: &str| art.lang.get(k).cloned().unwrap_or_else(|| k.to_owned());
        let title = lang(match o.page {
            Page::Options => "options.title",
            Page::Sounds => "options.sounds.title",
            Page::Video => "options.videoTitle",
            Page::Controls => "controls.title",
            Page::Mouse => "options.mouse_settings.title",
            Page::KeyBinds => "controls.keybinds.title",
        });
        let title_width = Self::text_width(art, &title) as i32;
        let x = match o.page {
            // Centred over the FOV slider and Online..., 308 wide together.
            Page::Options => (width - 308) / 2 + (308 - title_width) / 2,
            _ => (width - title_width) / 2,
        };
        self.shadowed(art, &title, x as f32, 12.0, SCREEN + 2.0, 1.0, [1.0; 3]);
        let (top, list_height) = list_area(height);
        let screen = [0.0, 0.0, width as f32, height as f32];
        let list = [0.0, top as f32, width as f32, list_height as f32];
        if o.page != Page::Options {
            // `AbstractSelectionList.extractListBackground`, its texture's left edge at the
            // list's right and its top at its bottom plus the scroll; then the separators.
            let (background, header, footer) = if o.in_world {
                let (h, f) = (INWORLD_HEADER_SEPARATOR, INWORLD_FOOTER_SEPARATOR);
                (INWORLD_MENU_LIST_BACKGROUND, h, f)
            } else {
                let (h, f) = (super::menu::HEADER_SEPARATOR, super::menu::FOOTER_SEPARATOR);
                (MENU_LIST_BACKGROUND, h, f)
            };
            let origin = [width, top + list_height + o.scroll as i32];
            let rect = [0, top, width, list_height];
            self.tiled_from(rect, [32, 32], origin, art.named[background], SCREEN + 0.25);
            let rect = [0, top - 2, width, 2];
            self.tiled_from(rect, [32, 2], [0, 0], art.named[header], SCREEN + 0.5);
            let rect = [0, top + list_height, width, 2];
            self.tiled_from(rect, [32, 2], [0, 0], art.named[footer], SCREEN + 0.5);
        }
        let Layout {
            widgets,
            headers,
            texts,
            stripes,
        } = layout(o, art, width, height);
        let visible = |y: i32| y + 9 >= top && y <= top + list_height;
        // The list's entries, clipped to the list.
        for (text, x, y) in texts.into_iter().filter(|t| visible(t.2)) {
            let start = self.vertices.len() / 4;
            self.shadowed(art, &text, x as f32, y as f32, SCREEN + 2.0, 1.0, [1.0; 3]);
            self.clip_from(start, list);
        }
        for stripe in stripes.into_iter().filter(|s| visible(s[1])) {
            let start = self.vertices.len() / 4;
            self.rect(art, stripe, SCREEN + 1.0, [255, 255, 0, 255]);
            self.clip_from(start, list);
        }
        for (text, y) in headers {
            if y + 9 < top || y > top + list_height {
                continue;
            }
            let start = self.vertices.len() / 4;
            let text_width = self.header_width(art, &text);
            let x = width / 2 - (text_width + 8) / 2 + 4;
            self.header(art, &text, x as f32, y as f32);
            self.clip_from(start, list);
        }
        for w in &widgets {
            let [_, y, _, h] = w.button.rect;
            if w.in_list && (y + h < top || y > top + list_height) {
                continue;
            }
            self.widget(art, w, o, if w.in_list { list } else { screen });
        }
        // `AbstractScrollArea.extractScrollbar`.
        if let Some([x, y, w, h]) = scrollbar(o.page, width, height) {
            let bar = scroller_height(o.page, height);
            let max = max_scroll(o.page, height);
            let bar_y = (y + (o.scroll * (h - bar) as f64 / max) as i32).max(y);
            let z = SCREEN + 1.5;
            self.nine_slice_tiled(art, SCROLLER_BACKGROUND, [6, 32], 1, [x, y, w, h], z);
            self.nine_slice_tiled(art, SCROLLER, [6, 32], 1, [x, bar_y, w, bar], z + 0.1);
        }
    }

    /// Clips the quads drawn since quad `start` to `rect`.
    fn clip_from(&mut self, start: usize, rect: [f32; 4]) {
        let end = self.vertices.len() / 4;
        if end > start {
            self.clips.push((start as u32..end as u32, rect));
        }
    }

    /// A bold string's width: each glyph a pixel wider.
    fn header_width(&self, art: &HudArt, text: &str) -> i32 {
        Self::text_width(art, text) as i32 + text.chars().count() as i32
    }

    /// `Font.drawString` for a bold, underlined string with its shadow: each glyph drawn
    /// twice, a pixel apart, and a line under them all from a pixel left of the first,
    /// 8 down (`Font.StringRenderOutput.accept`).
    fn header(&mut self, art: &HudArt, text: &str, x: f32, y: f32) {
        let z = SCREEN + 2.0;
        let mut cx = x;
        for c in text.chars() {
            let s = c.to_string();
            self.shadowed(art, &s, cx, y, z, 1.0, [1.0; 3]);
            self.shadowed(art, &s, cx + 1.0, y, z, 1.0, [1.0; 3]);
            cx += Self::text_width(art, &s) + 1.0;
        }
        let line = |dx: i32| [x as i32 - 1 + dx, y as i32 + 8 + dx, (cx - x) as i32 + 1, 1];
        self.rect(art, line(1), z, [63, 63, 63, 255]);
        self.rect(art, line(0), z + 0.5, [255; 4]);
    }

    fn widget(&mut self, art: &HudArt, w: &Widget, o: &Options, clip: [f32; 4]) {
        let [x, y, width, height] = w.button.rect;
        let start = self.vertices.len() / 4;
        let z = SCREEN + 1.0;
        let hovered = o
            .mouse
            .is_some_and(|m| w.button.contains(m) && clip_contains(clip, m));
        match w.slider {
            // `AbstractSliderButton.extractWidgetRenderState`: the track nine-sliced (200 ×
            // 20, a border of 1), highlighted while focused but not taking the arrows; the
            // handle where the value is, highlighted under the mouse or while it takes them.
            Some(value) => {
                let focused = w.active && o.focused == Some(w.action);
                let track = match focused && !o.can_change {
                    true => SLIDER_HIGHLIGHTED,
                    false => SLIDER,
                };
                self.nine_slice_tiled(art, track, [200, 20], 1, w.button.rect, z);
                let handle = if w.active && (hovered || focused && o.can_change) {
                    SLIDER_HANDLE_HIGHLIGHTED
                } else {
                    SLIDER_HANDLE
                };
                let hx = x + (value * (width - 8) as f64) as i32;
                let rect = art.named[handle];
                self.blit(hx as f32, y as f32, z + 0.25, 8.0, 20.0, rect, [1.0; 4]);
            }
            None => {
                let focused = o.focused == Some(w.action);
                self.button_sprite(art, &w.button, w.active, hovered || focused);
            }
        }
        self.clip_from(start, clip);
        // `AbstractWidget.extractScrollingStringOverContents`, 2 in from each side: centred
        // when it fits, otherwise scrolled to and fro inside them.
        let start = self.vertices.len() / 4;
        let text_width = Self::text_width(art, &w.label) as i32;
        let (left, right) = (x + 2, x + width - 2);
        let top = (y + y + height - 9) / 2 + 1;
        let rgb = if w.active {
            [1.0; 3]
        } else {
            [160.0 / 255.0; 3]
        };
        let mut label_clip = clip;
        let text_x = if text_width > right - left {
            let max = (text_width - (right - left)) as f64;
            let time = o.millis as f64 / 1000.0;
            let period = (max * 0.5).max(3.0);
            let alpha =
                (std::f64::consts::FRAC_PI_2 * (std::f64::consts::TAU * time / period).cos()).sin()
                    / 2.0
                    + 0.5;
            let inner = [left as f32, y as f32, (right - left) as f32, height as f32];
            label_clip = intersect(clip, inner);
            left - (alpha * max) as i32
        } else {
            let centre = (x + width / 2).clamp(left + text_width / 2, right - text_width / 2);
            centre - text_width / 2
        };
        if w.spans.is_empty() {
            let (x, y) = (text_x as f32, top as f32);
            self.shadowed(art, &w.label, x, y, SCREEN + 2.0, 1.0, rgb);
        }
        let mut pen = text_x;
        for span in &w.spans {
            let z = SCREEN + 2.0;
            self.shadowed(art, &span.text, pen as f32, top as f32, z, 1.0, span.rgb);
            let span_width = Self::text_width(art, &span.text) as i32;
            if span.underline {
                // `Font.StringRenderOutput`'s underline: from a pixel left, 8 down, with
                // its shadow.
                let line = |d: i32| [pen - 1 + d, top + 8 + d, span_width + 1, 1];
                let colour = |c: f32| (c * 255.0) as u8;
                let [r, g, b] = span.rgb.map(colour);
                self.rect(art, line(1), z, [r / 4, g / 4, b / 4, 255]);
                self.rect(art, line(0), z + 0.5, [r, g, b, 255]);
            }
            pen += span_width;
        }
        self.clip_from(start, label_clip);
    }

    /// `GuiGraphicsExtractor.blitNineSlicedSprite` for a sprite `size` big with the same
    /// `border` all round: the corners and edges as they are, the edges and middle tiled.
    pub(super) fn nine_slice_tiled(
        &mut self,
        art: &HudArt,
        sprite: &str,
        size: [i32; 2],
        border: i32,
        [x, y, w, h]: [i32; 4],
        z: f32,
    ) {
        let [u0, v0, u1, v1] = art.named[sprite];
        let u = |px: i32| u0 + (u1 - u0) * px as f32 / size[0] as f32;
        let v = |px: i32| v0 + (v1 - v0) * px as f32 / size[1] as f32;
        // Each axis: the near border, the middle repeated, the far border, as (where on the
        // screen, where in the sprite, how long).
        let spans = |at: i32, len: i32, size: i32| {
            if len == size {
                return vec![(at, 0, len)];
            }
            let mut spans = vec![(at, 0, border)];
            let mut pos = at + border;
            while pos < at + len - border {
                let n = (at + len - border - pos).min(size - 2 * border);
                spans.push((pos, border, n));
                pos += n;
            }
            spans.push((at + len - border, size - border, border));
            spans
        };
        for (sy, ty, th) in spans(y, h, size[1]) {
            for &(sx, tx, tw) in &spans(x, w, size[0]) {
                let rect = [u(tx), v(ty), u(tx + tw), v(ty + th)];
                self.blit(
                    sx as f32, sy as f32, z, tw as f32, th as f32, rect, [1.0; 4],
                );
            }
        }
    }

    /// A texture `size` GUI pixels big repeated across the rectangle (x, y, w, h), its
    /// texel `origin` (wrapped) at the rectangle's top left, as `GuiGraphicsExtractor.blit`
    /// with UVs past the texture's edge repeats it.
    fn tiled_from(
        &mut self,
        [x, y, w, h]: [i32; 4],
        size: [i32; 2],
        origin: [i32; 2],
        rect: [f32; 4],
        z: f32,
    ) {
        let [u0, v0, u1, v1] = rect;
        let u = |px: i32| u0 + (u1 - u0) * px as f32 / size[0] as f32;
        let v = |px: i32| v0 + (v1 - v0) * px as f32 / size[1] as f32;
        let (mut ty, mut tv) = (0, origin[1].rem_euclid(size[1]));
        while ty < h {
            let ch = (size[1] - tv).min(h - ty);
            let (mut tx, mut tu) = (0, origin[0].rem_euclid(size[0]));
            while tx < w {
                let cw = (size[0] - tu).min(w - tx);
                let r = [u(tu), v(tv), u(tu + cw), v(tv + ch)];
                let (px, py) = ((x + tx) as f32, (y + ty) as f32);
                self.blit(px, py, z, cw as f32, ch as f32, r, [1.0; 4]);
                (tx, tu) = (tx + cw, 0);
            }
            (ty, tv) = (ty + ch, 0);
        }
    }
}

fn clip_contains([x, y, w, h]: [f32; 4], [mx, my]: [f32; 2]) -> bool {
    mx >= x && my >= y && mx < x + w && my < y + h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_sliders_step_like_vanilla() {
        // Normal (70) sits half way; the ends are exact.
        assert_eq!(fov_to_slider(70), 0.5);
        assert_eq!(fov_from_slider(0.5), 70);
        assert_eq!(fov_from_slider(0.0), 30);
        assert_eq!(fov_from_slider(1.0), 110);
        for fov in 30..=110 {
            assert_eq!(fov_from_slider(fov_to_slider(fov)), fov);
        }
        // Max Framerate steps by 10 from 10 to 260 (unlimited).
        let mut v = Video::default();
        v.set_slider(VideoOption::Framerate, 1.0);
        assert_eq!(v.framerate, 260);
        v.set_slider(VideoOption::Framerate, 0.0);
        assert_eq!(v.framerate, 10);
        v.set_slider(VideoOption::Framerate, to_slider(12, 1, 26));
        assert_eq!(v.framerate, 120);
    }

    #[test]
    fn translations_take_their_arguments() {
        assert_eq!(format("%s: %s%%", &["Music", "50"]), "Music: 50%");
        assert_eq!(format("%2$s %1$s", &["a", "b"]), "b a");
        assert_eq!(format("%sx%s@%s", &["1", "2", "3"]), "1x2@3");
    }

    #[test]
    fn gui_scale_cycles_through_auto() {
        let mut v = Video {
            gui_scale_max: 3,
            ..Default::default()
        };
        let seen: Vec<i32> = (0..5)
            .map(|_| {
                v.cycle(VideoOption::GuiScale);
                v.gui_scale
            })
            .collect();
        assert_eq!(seen, [1, 2, 3, 0, 1]);
    }

    /// Scroll Sensitivity steps through powers of ten from 0.01 to 10, 1 at its default;
    /// Sensitivity at its default turns the view vanilla's 0.15 degrees a pixel.
    #[test]
    fn mouse_options_scale_like_vanilla() {
        let mut m = Mouse::default();
        assert_eq!(unlog_mouse(m.wheel), 0);
        m.set_slider(MouseOption::Wheel, 0.0);
        assert!((m.wheel - 0.01).abs() < 1e-12);
        m.set_slider(MouseOption::Wheel, 1.0);
        assert_eq!(m.wheel, 10.0);
        let default = Mouse::default();
        m.set_slider(MouseOption::Wheel, default.slider(MouseOption::Wheel));
        assert_eq!(m.wheel, 1.0);
        let (x, y) = default.turn(10.0, -4.0);
        assert!((x - 1.5).abs() < 1e-6 && (y + 0.6).abs() < 1e-6, "{x} {y}");
        let inverted = Mouse {
            invert_y: true,
            ..default
        };
        assert!(inverted.turn(0.0, 4.0).1 < 0.0);
        let discrete = Mouse {
            discrete_scroll: true,
            wheel: 2.0,
            ..default
        };
        assert_eq!(discrete.scroll(-0.3), -2.0);
        assert_eq!(default.scroll(-0.3), -0.3);
    }
}
