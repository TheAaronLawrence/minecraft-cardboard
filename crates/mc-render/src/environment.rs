//! A dimension's environment attributes: what colours its sky, fog, clouds and light are,
//! where its sun, moon and stars are, and so on (`EnvironmentAttributeSystem`). Each
//! attribute starts at its default, then takes the dimension type's value, then the biomes'
//! around the camera, blended, then each of the dimension's timelines in turn: keyframed
//! tracks sampled at a world clock's ticks (`Timeline`, `AttributeTrackSampler`), such as
//! `timeline/day.json`, which turns the sun and darkens the sky through the day.
//!
//! Only the visual attributes the renderer draws are read. Weather's layers aren't here,
//! as the client has no weather yet.

use anyhow::{Context, Result};
use mc_assets::Resources;
use serde_json::Value;
use std::collections::HashMap;

/// An attribute's value: a float in `[0]`, a colour's red, green, blue and alpha, or a moon
/// phase's index in `[0]`.
type Vals = [f32; 4];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Float,
    /// `ANGLE_DEGREES`: a float, but blended between ticks the short way round.
    Angle,
    /// `RGB_COLOR`, whose alpha stays 1.
    Rgb,
    /// `ARGB_COLOR`.
    Argb,
    MoonPhase,
}

/// The attributes read, as `EnvironmentAttributes` registers them: name, type, default, and
/// the range its value is clamped to.
/// An attribute: its id, kind, default values and range, if it has one.
type Attribute = (&'static str, Kind, Vals, Option<(f32, f32)>);

const ATTRIBUTES: [Attribute; 17] = [
    ("sky_color", Kind::Rgb, [0.0, 0.0, 0.0, 1.0], None),
    ("fog_color", Kind::Rgb, [0.0, 0.0, 0.0, 1.0], None),
    ("fog_start_distance", Kind::Float, [0.0; 4], None),
    (
        "fog_end_distance",
        Kind::Float,
        [1024.0, 0.0, 0.0, 0.0],
        NON_NEGATIVE,
    ),
    (
        "sky_fog_end_distance",
        Kind::Float,
        [512.0, 0.0, 0.0, 0.0],
        NON_NEGATIVE,
    ),
    (
        "cloud_fog_end_distance",
        Kind::Float,
        [2048.0, 0.0, 0.0, 0.0],
        NON_NEGATIVE,
    ),
    ("cloud_color", Kind::Argb, [0.0; 4], None),
    ("cloud_height", Kind::Float, [192.33, 0.0, 0.0, 0.0], None),
    ("sun_angle", Kind::Angle, [0.0; 4], None),
    ("moon_angle", Kind::Angle, [0.0; 4], None),
    ("star_angle", Kind::Angle, [0.0; 4], None),
    ("star_brightness", Kind::Float, [0.0; 4], UNIT),
    ("sunrise_sunset_color", Kind::Argb, [0.0; 4], None),
    ("moon_phase", Kind::MoonPhase, [0.0; 4], None),
    ("sky_light_color", Kind::Rgb, [1.0; 4], None),
    ("sky_light_factor", Kind::Float, [1.0, 0.0, 0.0, 0.0], UNIT),
    // `ambient_light_color` and `block_light_tint` follow.
    ("ambient_light_color", Kind::Rgb, [0.0, 0.0, 0.0, 1.0], None),
];
const BLOCK_LIGHT_TINT: usize = ATTRIBUTES.len();
/// `gameplay/sky_light_level`, from 0 to 15, which the sky light seems by
/// (`Level.updateSkyBrightness`).
const SKY_LIGHT_LEVEL: usize = ATTRIBUTES.len() + 1;
const COUNT: usize = ATTRIBUTES.len() + 2;
const NON_NEGATIVE: Option<(f32, f32)> = Some((0.0, f32::INFINITY));
const UNIT: Option<(f32, f32)> = Some((0.0, 1.0));

fn kind(index: usize) -> Kind {
    if index == BLOCK_LIGHT_TINT {
        Kind::Rgb
    } else if index == SKY_LIGHT_LEVEL {
        Kind::Float
    } else {
        ATTRIBUTES[index].1
    }
}

fn attribute_index(name: &str) -> Option<usize> {
    if name == "minecraft:gameplay/sky_light_level" {
        return Some(SKY_LIGHT_LEVEL);
    }
    let name = name.strip_prefix("minecraft:visual/")?;
    if name == "block_light_tint" {
        return Some(BLOCK_LIGHT_TINT);
    }
    ATTRIBUTES.iter().position(|a| a.0 == name)
}

/// `MoonPhase`, in index order.
const MOON_PHASES: [&str; 8] = [
    "full_moon",
    "waning_gibbous",
    "third_quarter",
    "waning_crescent",
    "new_moon",
    "waxing_crescent",
    "first_quarter",
    "waxing_gibbous",
];

/// The visual attributes at a moment and place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visuals {
    pub sky_color: [f32; 3],
    pub fog_color: [f32; 3],
    /// The environment's fog, by distance from the eye: none at the start, complete at the
    /// end.
    pub fog_start: f32,
    pub fog_end: f32,
    pub sky_fog_end: f32,
    pub cloud_fog_end: f32,
    /// With alpha; clouds aren't drawn when it's 0.
    pub cloud_color: [f32; 4],
    pub cloud_height: f32,
    /// In degrees; 0 is overhead.
    pub sun_angle: f32,
    pub moon_angle: f32,
    pub star_angle: f32,
    pub star_brightness: f32,
    pub sunrise_color: [f32; 4],
    /// `MoonPhase.index`: 0 full to 4 new and back.
    pub moon_phase: u8,
    pub sky_light_color: [f32; 3],
    pub sky_light_factor: f32,
    pub ambient_color: [f32; 3],
    pub block_light_tint: [f32; 3],
    /// `gameplay/sky_light_level`: 15 by day, 4 at night, less in rain.
    pub sky_light_level: f32,
}

impl Visuals {
    fn from(values: &[Vals; COUNT]) -> Self {
        let v = |name: &str| values[attribute_index(&format!("minecraft:visual/{name}")).unwrap()];
        let rgb = |name: &str| {
            let [r, g, b, _] = v(name);
            [r, g, b]
        };
        Self {
            sky_color: rgb("sky_color"),
            fog_color: rgb("fog_color"),
            fog_start: v("fog_start_distance")[0],
            fog_end: v("fog_end_distance")[0],
            sky_fog_end: v("sky_fog_end_distance")[0],
            cloud_fog_end: v("cloud_fog_end_distance")[0],
            cloud_color: v("cloud_color"),
            cloud_height: v("cloud_height")[0],
            sun_angle: v("sun_angle")[0],
            moon_angle: v("moon_angle")[0],
            star_angle: v("star_angle")[0],
            star_brightness: v("star_brightness")[0],
            sunrise_color: v("sunrise_sunset_color"),
            moon_phase: v("moon_phase")[0] as u8,
            sky_light_color: rgb("sky_light_color"),
            sky_light_factor: v("sky_light_factor")[0],
            ambient_color: rgb("ambient_light_color"),
            block_light_tint: rgb("block_light_tint"),
            sky_light_level: values[SKY_LIGHT_LEVEL][0].clamp(0.0, 15.0),
        }
    }

    /// `EnvironmentAttributeProbe.ValueProbe.get`: between the last tick's values and this
    /// one's, by `partial` of a tick, each as its type blends (`partialTickLerp`).
    pub fn lerp(&self, next: &Self, partial: f32) -> Self {
        let lerp = |a: f32, b: f32| a + (b - a) * partial;
        let lerp3 = |a: [f32; 3], b: [f32; 3]| std::array::from_fn(|i| lerp(a[i], b[i]));
        let lerp4 = |a: [f32; 4], b: [f32; 4]| std::array::from_fn(|i| lerp(a[i], b[i]));
        // `LerpFunction.ofDegrees(90)`: the short way round, unless it jumped.
        let degrees = |a: f32, b: f32| {
            let delta = wrap_degrees(b - a);
            if delta.abs() >= 90.0 {
                b
            } else {
                a + partial * delta
            }
        };
        Self {
            sky_color: lerp3(self.sky_color, next.sky_color),
            fog_color: lerp3(self.fog_color, next.fog_color),
            fog_start: lerp(self.fog_start, next.fog_start),
            fog_end: lerp(self.fog_end, next.fog_end),
            sky_fog_end: lerp(self.sky_fog_end, next.sky_fog_end),
            cloud_fog_end: lerp(self.cloud_fog_end, next.cloud_fog_end),
            cloud_color: lerp4(self.cloud_color, next.cloud_color),
            cloud_height: lerp(self.cloud_height, next.cloud_height),
            sun_angle: degrees(self.sun_angle, next.sun_angle),
            moon_angle: degrees(self.moon_angle, next.moon_angle),
            star_angle: degrees(self.star_angle, next.star_angle),
            star_brightness: lerp(self.star_brightness, next.star_brightness),
            sunrise_color: lerp4(self.sunrise_color, next.sunrise_color),
            moon_phase: if partial >= 1.0 {
                next.moon_phase
            } else {
                self.moon_phase
            },
            sky_light_color: lerp3(self.sky_light_color, next.sky_light_color),
            sky_light_factor: lerp(self.sky_light_factor, next.sky_light_factor),
            ambient_color: lerp3(self.ambient_color, next.ambient_color),
            block_light_tint: lerp3(self.block_light_tint, next.block_light_tint),
            sky_light_level: lerp(self.sky_light_level, next.sky_light_level),
        }
    }
}

impl Visuals {
    /// `WeatherAttributes.addLayer`: after the timelines, rain without thunder (`rain -
    /// thunder`) blends toward `RAIN`'s modifiers and thunder toward `THUNDER`'s, each by
    /// its level. `thunder` is `Level.getThunderLevel`, already times the rain.
    pub fn weather(self, rain: f32, thunder: f32) -> Self {
        // `ARGB.colorFromFloat` and `ARGB.color(alpha, …)` round each channel down to a byte.
        let byte = |v: f32| (v * 255.0).floor() / 255.0;
        // Brightness and factor for the sky, then the clouds; the fog's multiplier; and the
        // alpha the sky light blends toward night's (`Timelines.NIGHT_SKY_LIGHT_COLOR`) by.
        let layers = [
            (
                rain - thunder,
                [0.6, 0.75],
                [0.24, 0.5],
                [0.5, 0.5, 0.6],
                0.3125,
            ),
            (
                thunder,
                [0.24, 0.94],
                [0.095, 0.94],
                [0.25, 0.25, 0.3],
                0.52734375,
            ),
        ];
        let mut v = self;
        for (level, sky, cloud, fog, alpha) in layers {
            if level <= 0.0 {
                continue;
            }
            let lerp = |a: f32, b: f32| a + (b - a) * level;
            let lerp_n = |a: Vals, b: Vals| -> Vals { std::array::from_fn(|i| lerp(a[i], b[i])) };
            let rgb = |c: [f32; 3]| [c[0], c[1], c[2], 1.0];
            let fog = fog.map(byte);
            let fog_arg = [fog[0], fog[1], fog[2], 1.0];
            let night = [byte(0.48), byte(0.48), 1.0, byte(alpha)];
            let modified_sky =
                Op::BlendToGray.apply(Kind::Rgb, rgb(v.sky_color), [sky[0], sky[1], 0.0, 0.0]);
            let s = lerp_n(rgb(v.sky_color), modified_sky);
            v.sky_color = [s[0], s[1], s[2]];
            let f = lerp_n(
                rgb(v.fog_color),
                Op::Multiply.apply(Kind::Rgb, rgb(v.fog_color), fog_arg),
            );
            v.fog_color = [f[0], f[1], f[2]];
            let modified_cloud =
                Op::BlendToGray.apply(Kind::Argb, v.cloud_color, [cloud[0], cloud[1], 0.0, 0.0]);
            v.cloud_color = lerp_n(v.cloud_color, modified_cloud);
            let l = lerp_n(
                rgb(v.sky_light_color),
                Op::AlphaBlend.apply(Kind::Rgb, rgb(v.sky_light_color), night),
            );
            v.sky_light_color = [l[0], l[1], l[2]];
            let factor = v.sky_light_factor + (0.24 - v.sky_light_factor) * alpha;
            v.sky_light_factor = lerp(v.sky_light_factor, factor);
            // `SKY_LIGHT_LEVEL`'s `FloatWithAlpha(4.0F, alpha)`.
            let level = v.sky_light_level + (4.0 - v.sky_light_level) * alpha;
            v.sky_light_level = lerp(v.sky_light_level, level);
            v.star_brightness = lerp(v.star_brightness, 0.0);
            let sunrise = Op::Multiply.apply(Kind::Argb, v.sunrise_color, fog_arg);
            v.sunrise_color = lerp_n(v.sunrise_color, sunrise);
        }
        v
    }

    /// `ClientLevel`'s lightning layers: while the sky flashes, the sky turns toward pale
    /// blue and the sky light is full.
    pub fn flash(self) -> Self {
        let flash = [0.8, 0.8, 1.0];
        Self {
            sky_color: std::array::from_fn(|i| {
                self.sky_color[i] + (flash[i] - self.sky_color[i]) * 0.22
            }),
            sky_light_factor: 1.0,
            ..self
        }
    }
}

/// `Mth.wrapDegrees`.
fn wrap_degrees(angle: f32) -> f32 {
    let mut a = angle % 360.0;
    if a >= 180.0 {
        a -= 360.0;
    }
    if a < -180.0 {
        a += 360.0;
    }
    a
}

/// `AttributeModifier.OperationId`s, applied to a value with their argument.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Override,
    Add,
    Subtract,
    Multiply,
    Minimum,
    Maximum,
    AlphaBlend,
    /// `ColorModifier.BlendToGray`, whose argument is its brightness and factor.
    BlendToGray,
}

impl Op {
    fn parse(name: Option<&str>) -> Option<Self> {
        Some(match name.unwrap_or("override") {
            "override" => Self::Override,
            "add" => Self::Add,
            "subtract" => Self::Subtract,
            "multiply" => Self::Multiply,
            "minimum" => Self::Minimum,
            "maximum" => Self::Maximum,
            "alpha_blend" => Self::AlphaBlend,
            "blend_to_gray" => Self::BlendToGray,
            _ => return None,
        })
    }

    /// `FloatModifier` and `ColorModifier`'s operations on `value`.
    fn apply(self, kind: Kind, value: Vals, arg: Vals) -> Vals {
        let each = |f: &dyn Fn(f32, f32) -> f32, n: usize| -> Vals {
            std::array::from_fn(|i| if i < n { f(value[i], arg[i]) } else { value[i] })
        };
        let channels = match kind {
            Kind::Rgb | Kind::Argb => 3,
            _ => 1,
        };
        match self {
            Self::Override => arg,
            // `ARGB.addRgb` and `subtractRgb` clamp, and keep the alpha.
            Self::Add if channels == 3 => each(&|a, b| (a + b).min(1.0), 3),
            Self::Subtract if channels == 3 => each(&|a, b| (a - b).max(0.0), 3),
            Self::Add => each(&|a, b| a + b, 1),
            Self::Subtract => each(&|a, b| a - b, 1),
            // `ARGB.multiply` on four channels for ARGB colours, three for RGB.
            Self::Multiply => each(&|a, b| a * b, if kind == Kind::Argb { 4 } else { channels }),
            Self::Minimum => each(&f32::min, 1),
            Self::Maximum => each(&f32::max, 1),
            Self::AlphaBlend => match kind {
                // `FloatModifier.ALPHA_BLEND`: the argument is the value and its alpha.
                Kind::Float | Kind::Angle => {
                    [value[0] + (arg[0] - value[0]) * arg[1], 0.0, 0.0, 0.0]
                }
                Kind::Rgb => {
                    let a = arg[3];
                    std::array::from_fn(|i| {
                        if i < 3 {
                            value[i] + (arg[i] - value[i]) * a
                        } else {
                            1.0
                        }
                    })
                }
                // `ARGB.alphaBlend` on two translucent colours.
                _ => {
                    let (da, sa) = (value[3], arg[3]);
                    if sa == 1.0 {
                        arg
                    } else if sa == 0.0 {
                        value
                    } else {
                        let alpha = sa + da * (1.0 - sa);
                        let mut out = [0.0, 0.0, 0.0, alpha];
                        for i in 0..3 {
                            out[i] = (arg[i] * sa + value[i] * (alpha - sa)) / alpha;
                        }
                        out
                    }
                }
            },
            Self::BlendToGray => {
                // `ARGB.greyscale` times the brightness, mixed in by the factor.
                let grey = (value[0] * 0.3 + value[1] * 0.59 + value[2] * 0.11) * arg[0];
                each(&|v, _| v + (grey - v) * arg[1], 3)
            }
        }
    }
}

/// An attribute's argument as data writes it: a number, a `#rrggbb` or `#aarrggbb` colour, a
/// moon phase's name, or `BlendToGray`'s brightness and factor.
fn parse_value(kind: Kind, op: Op, v: &Value) -> Option<Vals> {
    if op == Op::BlendToGray {
        return Some([
            v["brightness"].as_f64()? as f32,
            v["factor"].as_f64()? as f32,
            0.0,
            0.0,
        ]);
    }
    if let Some(n) = v.as_f64() {
        return Some([n as f32, 0.0, 0.0, 0.0]);
    }
    let s = v.as_str()?;
    if kind == Kind::MoonPhase {
        let i = MOON_PHASES.iter().position(|p| *p == s)?;
        return Some([i as f32, 0.0, 0.0, 0.0]);
    }
    let hex = s.strip_prefix('#')?;
    let n = u32::from_str_radix(hex, 16).ok()?;
    let byte = |shift: u32| (n >> shift & 0xFF) as f32 / 255.0;
    match hex.len() {
        6 => Some([byte(16), byte(8), byte(0), 1.0]),
        8 => Some([byte(16), byte(8), byte(0), byte(24)]),
        _ => None,
    }
}

/// A dimension's or biome's entry for an attribute: a plain value overrides, or an object
/// gives a modifier and its argument (`EnvironmentAttributeMap.Entry`).
fn parse_entry(index: usize, v: &Value) -> Option<(Op, Vals)> {
    let kind = kind(index);
    if let Some(op) = v.get("modifier") {
        let op = Op::parse(op.as_str())?;
        return Some((op, parse_value(kind, op, &v["argument"])?));
    }
    Some((Op::Override, parse_value(kind, Op::Override, v)?))
}

fn parse_attributes(attributes: &Value) -> Vec<(usize, Op, Vals)> {
    attributes
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, v)| {
            let index = attribute_index(name)?;
            let (op, vals) = parse_entry(index, v)?;
            Some((index, op, vals))
        })
        .collect()
}

/// `EasingType`: the ones vanilla's data uses (and its default, linear); others ease
/// linearly.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Easing {
    Constant,
    Linear,
    CubicBezier([f32; 4]),
}

impl Easing {
    fn parse(v: &Value) -> Self {
        match v {
            Value::String(s) if s == "constant" => Self::Constant,
            Value::Object(o) => match o.get("cubic_bezier").and_then(Value::as_array) {
                Some(c) if c.len() == 4 => {
                    Self::CubicBezier(std::array::from_fn(|i| c[i].as_f64().unwrap_or(0.0) as f32))
                }
                _ => Self::Linear,
            },
            _ => Self::Linear,
        }
    }

    fn apply(self, x: f32) -> f32 {
        match self {
            Self::Constant => 0.0,
            Self::Linear => x,
            Self::CubicBezier([x1, y1, x2, y2]) => {
                // `EasingType.CubicBezier`: solve the x curve for t, then sample the y curve.
                let curve =
                    |v1: f32, v2: f32| (3.0 * v1 - 3.0 * v2 + 1.0, -6.0 * v1 + 3.0 * v2, 3.0 * v1);
                let sample = |(a, b, c): (f32, f32, f32), t: f32| ((a * t + b) * t + c) * t;
                let gradient = |(a, b, c): (f32, f32, f32), t: f32| (3.0 * a * t + 2.0 * b) * t + c;
                let (xc, yc) = (curve(x1, x2), curve(y1, y2));
                let mut t = x;
                let mut solved = false;
                for _ in 0..4 {
                    let error = sample(xc, t) - x;
                    if error.abs() < 1.0e-5 {
                        solved = true;
                        break;
                    }
                    let g = gradient(xc, t);
                    if g < 1.0e-5 {
                        break;
                    }
                    t -= (error / g).clamp(-0.25, 0.25);
                }
                if !solved {
                    // `solveTBisect`.
                    let (mut t0, mut t1) = (0.0f32, 1.0f32);
                    while t0 < t1 {
                        let error = sample(xc, t) - x;
                        if error.abs() < 1.0e-5 {
                            break;
                        }
                        if error < 0.0 {
                            t0 = t;
                        } else {
                            t1 = t;
                        }
                        t = (t1 + t0) / 2.0;
                    }
                }
                sample(yc, t)
            }
        }
    }
}

/// One attribute's track in a timeline (`AttributeTrack`), baked into segments as
/// `KeyframeTrackSampler` bakes them.
#[derive(Debug)]
struct Track {
    attribute: usize,
    op: Op,
    period: Option<i64>,
    /// Easing, from value and tick, to value and tick.
    segments: Vec<(Easing, Vals, i64, Vals, i64)>,
}

impl Track {
    fn parse(attribute: usize, period: Option<i64>, v: &Value) -> Option<Self> {
        let op = Op::parse(v["modifier"].as_str())?;
        let easing = Easing::parse(&v["ease"]);
        let keyframes: Vec<(i64, Vals)> = v["keyframes"]
            .as_array()?
            .iter()
            .map(|k| {
                Some((
                    k["ticks"].as_i64()?,
                    parse_value(kind(attribute), op, &k["value"])?,
                ))
            })
            .collect::<Option<_>>()?;
        let (first, last) = (*keyframes.first()?, *keyframes.last()?);
        let mut segments = Vec::new();
        if keyframes.len() == 1 {
            segments.push((Easing::Constant, first.1, 0, first.1, 0));
        } else {
            if let Some(period) = period {
                segments.push((easing, last.1, last.0 - period, first.1, first.0));
            }
            for pair in keyframes.windows(2) {
                segments.push((easing, pair[0].1, pair[0].0, pair[1].1, pair[1].0));
            }
            if let Some(period) = period {
                segments.push((easing, last.1, last.0, first.1, first.0 + period));
            }
        }
        Some(Self {
            attribute,
            op,
            period,
            segments,
        })
    }

    /// `KeyframeTrackSampler.sample`.
    fn sample(&self, ticks: i64) -> Vals {
        let ticks = match self.period {
            Some(p) => ticks.rem_euclid(p),
            None => ticks,
        };
        let &(easing, from, from_ticks, to, to_ticks) = self
            .segments
            .iter()
            .find(|s| ticks < s.4)
            .unwrap_or_else(|| self.segments.last().expect("a track has a keyframe"));
        if ticks <= from_ticks {
            return from;
        }
        if ticks >= to_ticks {
            return to;
        }
        let alpha = easing.apply((ticks - from_ticks) as f32 / (to_ticks - from_ticks) as f32);
        match kind(self.attribute) {
            // Moon phases aren't interpolated: each holds until the next keyframe.
            Kind::MoonPhase => from,
            _ => std::array::from_fn(|i| from[i] + (to[i] - from[i]) * alpha),
        }
    }
}

/// How the sky is drawn (`DimensionType.skybox`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skybox {
    Overworld,
    End,
    None,
}

/// A dimension's attribute layers.
pub struct Environment {
    pub skybox: Skybox,
    /// The dimension's lowest block, the horizon of a superflat world.
    pub min_y: i32,
    /// `DimensionType.ambientLight`, which brightens the dark (0 in the overworld).
    pub ambient_light: f32,
    /// The defaults with the dimension type's values applied.
    base: [Vals; COUNT],
    /// Each biome's entries, by name.
    biomes: HashMap<String, Vec<(usize, Op, Vals)>>,
    /// Each timeline's world clock and tracks, in the dimension's order.
    timelines: Vec<(String, Vec<Track>)>,
}

impl Environment {
    /// The layers of the dimension type `dimension`, such as `minecraft:overworld`, from
    /// vanilla's data.
    pub fn load(res: &Resources, dimension: &str) -> Result<Self> {
        let read = |kind: &str, name: &str| -> Result<Value> {
            let path = res.data_path(kind, name, "json");
            serde_json::from_slice(
                &std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
            )
            .with_context(|| format!("{kind} {name}"))
        };
        let dim = read("dimension_type", dimension)?;
        let mut base: [Vals; COUNT] = std::array::from_fn(|i| match i {
            BLOCK_LIGHT_TINT => [1.0, 0xD8 as f32 / 255.0, 0x8C as f32 / 255.0, 1.0],
            SKY_LIGHT_LEVEL => [15.0, 0.0, 0.0, 0.0],
            _ => ATTRIBUTES[i].2,
        });
        for (index, op, arg) in parse_attributes(&dim["attributes"]) {
            base[index] = op.apply(kind(index), base[index], arg);
        }
        let skybox = match dim["skybox"].as_str() {
            Some("end") => Skybox::End,
            Some("none") => Skybox::None,
            _ => Skybox::Overworld,
        };

        let mut biomes = HashMap::new();
        let dir = res.data_path("worldgen/biome", "minecraft:plains", "json");
        if let Some(dir) = dir.parent() {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let name = format!("minecraft:{stem}");
                let json = read("worldgen/biome", &name)?;
                biomes.insert(name, parse_attributes(&json["attributes"]));
            }
        }

        let mut timelines = Vec::new();
        let ids = match dim["timelines"].as_str() {
            Some(tag) => match tag.strip_prefix('#') {
                Some(tag) => timeline_tag(res, tag)?,
                None => vec![tag.to_owned()],
            },
            None => Vec::new(),
        };
        for id in ids {
            let json = read("timeline", &id)?;
            let clock = json["clock"]
                .as_str()
                .unwrap_or("minecraft:overworld")
                .to_owned();
            let period = json["period_ticks"].as_i64();
            let tracks = json["tracks"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(name, v)| Track::parse(attribute_index(name)?, period, v))
                .collect();
            timelines.push((clock, tracks));
        }
        Ok(Self {
            skybox,
            min_y: dim["min_y"].as_i64().unwrap_or(0) as i32,
            ambient_light: dim["ambient_light"].as_f64().unwrap_or(0.0) as f32,
            base,
            biomes,
            timelines,
        })
    }

    /// The attributes where the biomes around the camera weigh `biomes` (as
    /// `GaussianSampler` weighs them, in the order they're first met), when each world clock
    /// has run the ticks `clock` gives for its name (`ValueSampler.computeValuePositional`).
    pub fn sample(&self, biomes: &[(&str, f64)], clock: impl Fn(&str) -> i64) -> Visuals {
        let mut values = self.base;
        // `SpatialAttributeInterpolator`: each biome applied to the dimension's value, blended
        // by weight.
        for (index, value) in values.iter_mut().enumerate() {
            let mut blended: Option<Vals> = None;
            let mut total = 0.0;
            for &(biome, weight) in biomes {
                let mut v = *value;
                if let Some(entries) = self.biomes.get(biome) {
                    for &(i, op, arg) in entries {
                        if i == index {
                            v = op.apply(kind(index), v, arg);
                        }
                    }
                }
                total += weight;
                blended = Some(match blended {
                    None => v,
                    Some(b) => {
                        let t = (weight / total) as f32;
                        std::array::from_fn(|c| b[c] + (v[c] - b[c]) * t)
                    }
                });
            }
            if let Some(b) = blended
                && kind(index) != Kind::MoonPhase
            {
                *value = b;
            }
        }
        for (clock_name, tracks) in &self.timelines {
            let ticks = clock(clock_name);
            for track in tracks {
                let i = track.attribute;
                values[i] = track.op.apply(kind(i), values[i], track.sample(ticks));
            }
        }
        // `EnvironmentAttribute.sanitizeValue`.
        for (i, value) in values.iter_mut().enumerate().take(ATTRIBUTES.len()) {
            if let Some((low, high)) = ATTRIBUTES[i].3 {
                value[0] = value[0].clamp(low, high);
            }
        }
        Visuals::from(&values)
    }
}

/// The timelines a timeline tag names, following the tags it includes.
fn timeline_tag(res: &Resources, tag: &str) -> Result<Vec<String>> {
    let path = res.data_path("tags/timeline", tag, "json");
    let json: Value = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    let mut ids = Vec::new();
    for v in json["values"].as_array().into_iter().flatten() {
        let Some(id) = v.as_str().or(v["id"].as_str()) else {
            continue;
        };
        match id.strip_prefix('#') {
            Some(inner) => ids.extend(timeline_tag(res, inner)?),
            None => ids.push(id.to_owned()),
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overworld() -> Environment {
        Environment::load(
            &Resources::new(mc_assets::default_root()),
            "minecraft:overworld",
        )
        .unwrap()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.002
    }

    /// Plains at noon is the sky the renderer drew before there was time; midnight's
    /// colours are `day.json`'s night keyframes times plains' colours; the sun sets and the
    /// moon rises through the day; and the moon's phase steps a day at a time.
    #[test]
    fn overworld_day_and_night() {
        let env = overworld();
        let at = |ticks: i64| env.sample(&[("minecraft:plains", 1.0)], |_| ticks);
        let noon = at(6000);
        assert_eq!(
            noon.sky_color,
            [0x78 as f32 / 255.0, 0xA7 as f32 / 255.0, 1.0]
        );
        assert_eq!(
            noon.fog_color,
            [0xC0 as f32 / 255.0, 0xD8 as f32 / 255.0, 1.0]
        );
        assert!(close(wrap_degrees(noon.sun_angle), 0.0));
        assert_eq!(noon.sky_light_factor, 1.0);
        assert_eq!(noon.star_brightness, 0.0);
        assert_eq!(noon.cloud_color, [1.0, 1.0, 1.0, 0.8]);

        let midnight = at(18000);
        assert_eq!(midnight.sky_color, [0.0; 3]);
        // #0f0f16 times plains' #c0d8ff.
        assert!(close(
            midnight.fog_color[0],
            15.0 / 255.0 * 0xC0 as f32 / 255.0
        ));
        assert!(close(midnight.fog_color[2], 22.0 / 255.0));
        assert!(close(midnight.sky_light_factor, 0.24));
        assert!(close(midnight.star_brightness, 0.5));
        // `day.json` multiplies the sky light level by 1 at noon and 0.26666668 at night;
        // full rain blends it 0.3125 of the way to 4.
        assert_eq!(noon.sky_light_level, 15.0);
        assert!(close(midnight.sky_light_level, 4.0));
        assert!(close(
            noon.weather(1.0, 0.0).sky_light_level,
            15.0 - 11.0 * 0.3125
        ));
        assert!(close(wrap_degrees(midnight.sun_angle).abs(), 180.0));
        // Midway between keyframes 11867 (1.0) and 13670 (0.26666668): linear.
        let dusk = at(12768);
        assert!(close(
            dusk.sky_light_factor,
            0.24 + (1.0 - 0.24) * (1.0 - (12768.0 - 11270.0) / (13140.0 - 11270.0))
        ));

        // The sun turns westward (positive angles) through the afternoon.
        assert!(at(9000).sun_angle > 20.0 && at(9000).sun_angle < 90.0);
        // The moon is opposite the sun.
        assert!(close(wrap_degrees(at(6000).moon_angle).abs(), 180.0));

        assert_eq!(at(0).moon_phase, 0);
        assert_eq!(at(23999).moon_phase, 0);
        assert_eq!(at(24000).moon_phase, 1);
        assert_eq!(at(24000 * 4 + 100).moon_phase, 4);
        assert_eq!(at(24000 * 8).moon_phase, 0);
    }

    /// A biome's own sky colour replaces the dimension's before the day darkens it, and
    /// biomes blend by weight.
    #[test]
    fn biomes_blend_before_the_day() {
        let env = overworld();
        let desert = env.sample(&[("minecraft:desert", 1.0)], |_| 6000);
        assert_eq!(
            desert.sky_color,
            [0x6E as f32 / 255.0, 0xB1 as f32 / 255.0, 1.0]
        );
        let half = env.sample(
            &[("minecraft:plains", 1.0), ("minecraft:desert", 1.0)],
            |_| 6000,
        );
        assert!(close(half.sky_color[0], (0x78 + 0x6E) as f32 / 2.0 / 255.0));
    }

    /// `EasingType.CubicBezier` against values Java 25 printed for day.json's sun curve.
    #[test]
    fn cubic_bezier_matches_vanilla() {
        let e = Easing::CubicBezier([0.362, 0.241, 0.638, 0.759]);
        for (x, y) in [
            (0.1, 0.074882306),
            (0.25, 0.21562685),
            (0.75, 0.78437316),
            (0.9, 0.92511773),
        ] {
            assert!(
                (e.apply(x) - y).abs() < 1.0e-6,
                "{x}: {} vs {y}",
                e.apply(x)
            );
        }
    }
}
