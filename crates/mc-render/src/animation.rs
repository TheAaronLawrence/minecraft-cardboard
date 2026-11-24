//! Keyframe animations (`KeyframeAnimation`): channels of keyframes that each move, turn or
//! scale one of a model's parts by an offset from its pose, sampled at a time and blended
//! in by a scale. The animations themselves are in [`crate::animations`].

use crate::entity_model::Node;

/// `AnimationDefinition`: how long it runs, whether it loops, and its channels.
pub(crate) struct Animation {
    pub(crate) length: f32,
    pub(crate) looping: bool,
    pub(crate) channels: &'static [Channel],
}

/// `AnimationChannel`: the keyframes for one part (`bone`) and one of its properties.
pub(crate) struct Channel {
    pub(crate) bone: &'static str,
    pub(crate) target: Target,
    pub(crate) frames: &'static [Frame],
}

/// `AnimationChannel.Targets`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Position,
    Rotation,
    Scale,
}

/// `Keyframe`: when, its value, and whether it's reached along a Catmull-Rom curve
/// rather than linearly.
pub(crate) struct Frame {
    time: f32,
    value: [f32; 3],
    smooth: bool,
}

impl Frame {
    pub(crate) const fn new(time: f32, value: [f32; 3], smooth: bool) -> Self {
        Self {
            time,
            value,
            smooth,
        }
    }
}

/// `KeyframeAnimations.degreeVec`: degrees to radians.
pub(crate) const fn deg(x: f32, y: f32, z: f32) -> [f32; 3] {
    let r = std::f32::consts::PI / 180.0;
    [x * r, y * r, z * r]
}

/// `KeyframeAnimations.posVec`: pixels, with y flipped to model space.
pub(crate) const fn pos(x: f32, y: f32, z: f32) -> [f32; 3] {
    [x, -y, z]
}

/// `KeyframeAnimations.scaleVec`: how much bigger than 1.
pub(crate) const fn scale(x: f32, y: f32, z: f32) -> [f32; 3] {
    [x - 1.0, y - 1.0, z - 1.0]
}

/// `Mth.catmullrom`.
fn catmull_rom(t: f32, [p0, p1, p2, p3]: [f32; 4]) -> f32 {
    0.5 * (2.0 * p1
        + (p2 - p0) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t)
}

impl Channel {
    /// `KeyframeAnimation.Entry.apply`'s value `t` seconds in, times `scale`.
    fn sample(&self, t: f32, scale: f32) -> [f32; 3] {
        let frames = self.frames;
        let last = frames.len() - 1;
        let prev = frames
            .iter()
            .position(|f| t <= f.time)
            .unwrap_or(frames.len())
            .saturating_sub(1);
        let next = (prev + 1).min(last);
        let alpha = if next != prev {
            ((t - frames[prev].time) / (frames[next].time - frames[prev].time)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        std::array::from_fn(|axis| {
            let v = |i: usize| frames[i].value[axis];
            let value = if frames[next].smooth {
                catmull_rom(
                    alpha,
                    [
                        v(prev.saturating_sub(1)),
                        v(prev),
                        v(next),
                        v((next + 1).min(last)),
                    ],
                )
            } else {
                v(prev) + (v(next) - v(prev)) * alpha
            };
            value * scale
        })
    }
}

impl Animation {
    /// `KeyframeAnimation.getElapsedSeconds`: whole milliseconds in, wrapped if it loops.
    fn seconds(&self, millis: i64) -> f32 {
        let seconds = millis as f32 / 1000.0;
        if self.looping {
            seconds % self.length
        } else {
            seconds
        }
    }

    /// The sum of this animation's `target` offsets for `bone`, `millis` in and blended in
    /// by `scale`, for models that pose their parts by hand.
    pub(crate) fn sample(&self, millis: i64, scale: f32, bone: &str, target: Target) -> [f32; 3] {
        let t = self.seconds(millis);
        let mut out = [0.0; 3];
        for c in self
            .channels
            .iter()
            .filter(|c| c.bone == bone && c.target == target)
        {
            let v = c.sample(t, scale);
            for (o, v) in out.iter_mut().zip(v) {
                *o += v;
            }
        }
        out
    }

    /// `KeyframeAnimation.apply(AnimationState, ageInTicks)` once started `ticks` ago:
    /// each channel's offset added to the part named in `bones` (one name per node).
    pub(crate) fn apply(&self, ticks: f32, bones: &[&str], nodes: &mut [Node]) {
        self.apply_at((ticks * 50.0) as i64, 1.0, bones, nodes);
    }

    /// `KeyframeAnimation.applyWalk`: as far along as the walk has gone, blended in as
    /// fast as it's going.
    pub(crate) fn walk_millis(pos: f32, speed: f32, speed_factor: f32, scale: f32) -> (i64, f32) {
        ((pos * 50.0 * speed_factor) as i64, (speed * scale).min(1.0))
    }

    /// `KeyframeAnimation.apply(millis, scale)` on named nodes.
    pub(crate) fn apply_at(&self, millis: i64, scale: f32, bones: &[&str], nodes: &mut [Node]) {
        let t = self.seconds(millis);
        for c in self.channels {
            let Some(i) = bones.iter().position(|b| *b == c.bone) else {
                continue;
            };
            let v = c.sample(t, scale);
            let n = &mut nodes[i];
            for axis in 0..3 {
                match c.target {
                    Target::Position => n.pose[axis] += v[axis],
                    Target::Rotation => n.pose[3 + axis] += v[axis],
                    Target::Scale => n.scale[axis] += v[axis],
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A looping half-second walk whose step swings -35°, +35°, -35° (`FOX_BABY_WALK`'s
    /// legs): linear frames are reached in a straight line, Catmull-Rom frames along the
    /// curve through their neighbours (worked by hand from `Mth.catmullrom`).
    #[test]
    fn samples_between_keyframes() {
        const STEP: &[Frame] = &[
            Frame::new(0.0, [-35.0, 0.0, 0.0], true),
            Frame::new(0.25, [35.0, 0.0, 0.0], false),
            Frame::new(0.5, [-35.0, 0.0, 0.0], true),
        ];
        let c = Channel {
            bone: "leg",
            target: Target::Rotation,
            frames: STEP,
        };
        let at = |t: f32, scale: f32| c.sample(t, scale)[0];
        // Into the linear frame: halfway from -35 to 35, and a frame's time ends its span.
        assert!(at(0.125, 1.0).abs() < 1e-4);
        assert!((at(0.25, 0.5) - 17.5).abs() < 1e-4);
        // Into the Catmull-Rom frame, halfway from 35 to -35 with -35 either side.
        assert!((at(0.375, 1.0) - 4.375).abs() < 1e-3);
        // Past the end it holds the last value.
        assert!((at(9.0, 1.0) + 35.0).abs() < 1e-4);
        let a = Animation {
            length: 0.5,
            looping: true,
            channels: &[],
        };
        assert!((a.seconds(625) - 0.125).abs() < 1e-6);
    }
}
