//! Where a voice is heard and how loud: OpenAL Soft 1.25's maths for the way vanilla sets up
//! its sources (`Channel.linearAttenuation`, `Channel.disableAttenuation`,
//! `Channel.setRelative`) on a stereo device with its default `basic` stereo encoding. A
//! source's position turns into the listener's frame (`UpdateContextProps`), fades over its
//! distance (`CalcAttnSourceParams`'s `LinearClamped` model), and pans as `CalcPanningAndFilters`
//! does in its `Pairwise` render mode: the direction's azimuth is stretched toward the sides,
//! encoded as first-order 2D ambisonics, and decoded to the two speakers with `StereoConfig`.

use crate::{Listener, Voice};

/// `StereoConfig`'s decoder rows for the left speaker, on N3D-scaled W, Y and X; the right
/// speaker's has Y negated.
const DECODE: [f32; 3] = [0.5, 0.288_675_13, 0.055_230_565];

/// Where `ALCcontext` keeps its listener: position and the rotation into its frame.
pub(crate) struct Frame {
    pos: [f64; 3],
    right: [f64; 3],
    up: [f64; 3],
    back: [f64; 3],
}

impl Frame {
    /// `UpdateContextProps`: forward (`AL_ORIENTATION`'s at) and up are normalised and right
    /// is their cross product; the frame's z points backward.
    pub fn new(listener: &Listener) -> Self {
        let n = normalize(listener.forward.map(f64::from));
        let v = normalize(listener.up.map(f64::from));
        let u = normalize(cross(n, v));
        Self {
            pos: listener.pos,
            right: u,
            up: v,
            back: n.map(|c| -c),
        }
    }

    /// A source's position in the listener's frame: x right, y up, z back.
    fn local(&self, voice: &Voice) -> [f64; 3] {
        if voice.relative {
            return voice.pos;
        }
        let d = [0, 1, 2].map(|i| voice.pos[i] - self.pos[i]);
        [dot(d, self.right), dot(d, self.up), dot(d, self.back)]
    }
}

/// The gain from each of a voice's channels (1 or 2) to the left and right speakers.
pub(crate) fn gains(frame: &Frame, voice: &Voice, channels: usize) -> [[f32; 2]; 2] {
    let local = frame.local(voice);
    let distance = dot(local, local).sqrt() as f32;
    // `CalcAttnSourceParams`: `AL_LINEAR_DISTANCE_CLAMPED` with a reference distance of 0
    // and a rolloff of 1, or no fading at all; the source's gain is clamped to `AL_MAX_GAIN`.
    let fade = match voice.attenuation {
        Some(max) if max >= 0.0 => {
            let clamped = distance.clamp(0.0, max);
            if max > 0.0 {
                (1.0 - clamped / max).max(0.0)
            } else {
                1.0
            }
        }
        _ => 1.0,
    };
    let gain = (voice.gain * fade).clamp(0.0, 1.0);

    let pan = |pos: [f32; 3]| -> [f32; 2] {
        // `CalcDirectionCoeffs`: ambisonic Y is left and X is front; N3D's first order
        // carries sqrt(3).
        let y = 3f32.sqrt() * -pos[0];
        let x = 3f32.sqrt() * -pos[2];
        [
            DECODE[0] + DECODE[1] * y + DECODE[2] * x,
            DECODE[0] - DECODE[1] * y + DECODE[2] * x,
        ]
    };
    let side = |pan: [f32; 2]| pan.map(|g| g * gain);
    if distance > f32::EPSILON {
        let dir = local.map(|c| c as f32 / distance);
        if channels == 1 {
            let p = side(pan(scale_azimuth_front3_2(dir)));
            [p, p]
        } else {
            // With no spread every channel sits at the source's position.
            let p = side(pan(scale_azimuth_front3(dir)));
            [p, p]
        }
    } else if channels == 1 {
        // `MonoMap`: front centre.
        let p = side(pan(scale_azimuth_front3([0.0, 0.0, -1.0])));
        [p, p]
    } else {
        // `StereoMap`: front left and right at 30 degrees.
        let (sin30, cos30) = (0.5, 3f32.sqrt() / 2.0);
        [
            side(pan(scale_azimuth_front3([-sin30, 0.0, -cos30]))),
            side(pan(scale_azimuth_front3([sin30, 0.0, -cos30]))),
        ]
    }
}

/// `ScaleAzimuthFront3`: in front, the azimuth is tripled, so ±30 degrees becomes ±90;
/// beyond that it is clamped to 90. Behind is left alone.
fn scale_azimuth_front3(mut pos: [f32; 3]) -> [f32; 3] {
    if pos[2] < 0.0 {
        let len2d = pos[0].hypot(pos[2]);
        let z = -pos[2] / len2d;
        if z > 0.866_025_4 {
            let x = pos[0] / len2d;
            pos[0] = (x * 3.0 - x * x * x * 4.0) * len2d;
            pos[2] = -(z * z * z * 4.0 - z * 3.0) * len2d;
        } else {
            pos[0] = len2d.copysign(pos[0]);
            pos[2] = 0.0;
        }
    }
    pos
}

/// `ScaleAzimuthFront3_2`: in front, the azimuth is scaled by 1.5, so ±60 degrees becomes
/// ±90; beyond that it is clamped to 90. Behind is left alone.
fn scale_azimuth_front3_2(mut pos: [f32; 3]) -> [f32; 3] {
    if pos[2] < 0.0 {
        let len2d = pos[0].hypot(pos[2]);
        let z = -pos[2] / len2d;
        if z > 0.5 {
            let x = pos[0] / len2d;
            // Halve the angle, then triple it.
            let x = ((1.0 - z) * 0.5).sqrt().copysign(x);
            let z = ((1.0 + z) * 0.5).sqrt();
            pos[0] = (x * 3.0 - x * x * x * 4.0) * len2d;
            pos[2] = -(z * z * z * 4.0 - z * 3.0) * len2d;
        } else {
            pos[0] = len2d.copysign(pos[0]);
            pos[2] = 0.0;
        }
    }
    pos
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f64; 3]) -> [f64; 3] {
    let len = dot(v, v).sqrt();
    if len > 0.0 { v.map(|c| c / len) } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A listener at the origin facing north (-z), as vanilla's camera faces at yaw 180.
    fn north() -> Frame {
        Frame::new(&Listener {
            pos: [0.0; 3],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
        })
    }

    fn at(pos: [f64; 3]) -> Voice {
        Voice {
            pos,
            relative: false,
            attenuation: Some(16.0),
            gain: 1.0,
            pitch: 1.0,
            looping: false,
        }
    }

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
    }

    /// Recorded from vanilla 26.3 on OpenAL Soft 1.25.2: the far speaker is silent for a
    /// sound at 90 degrees, a sound at 30 degrees is about 12 dB louder on its side, and one
    /// straight ahead plays equally at about 0.6 of one at the side.
    #[test]
    fn mono_sources_pan_as_openal_soft_pairs_them() {
        let f = north();
        // 4 blocks east, to the listener's right: a quarter of the way to silence.
        let [right, _] = gains(&f, &at([4.0, 0.0, 0.0]), 1);
        assert!(close(right, [0.0, 0.75]), "{right:?}");
        // Straight ahead: W plus a little X on both.
        let [ahead, _] = gains(&f, &at([0.0, 0.0, -4.0]), 1);
        assert!(
            close(ahead, [0.75 * 0.595_662, 0.75 * 0.595_662]),
            "{ahead:?}"
        );
        // 30 degrees right is panned as 45: L 0.5 - 0.5 sin 45 + 0.0957 cos 45.
        let (s, c) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
        let [thirty, _] = gains(&f, &at([4.0 * s, 0.0, -4.0 * c]), 1);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let want = [0.5 - 0.5 * h + 0.095_662 * h, 0.5 + 0.5 * h + 0.095_662 * h];
        assert!(close(thirty, want.map(|g| g * 0.75)), "{thirty:?}");
        let db = 20.0 * (thirty[1] / thirty[0]).log10();
        assert!((12.0..13.5).contains(&db), "{db} dB");
        // Behind is quieter than ahead, and the far side of 60 degrees is hard right.
        let [behind, _] = gains(&f, &at([0.0, 0.0, 4.0]), 1);
        let back = 0.75 * (0.5 - 0.095_662);
        assert!(close(behind, [back, back]), "{behind:?}");
        let [sixty, _] = gains(&f, &at([4.0 * 0.9, 0.0, -4.0 * 0.4]), 1);
        assert!(sixty[0].abs() < 1e-6, "{sixty:?}");
    }

    #[test]
    fn the_listener_turning_moves_the_sound() {
        // Facing east, a sound to the east is ahead and one to the north is on the left.
        let f = Frame::new(&Listener {
            pos: [10.0, 64.0, 10.0],
            forward: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
        });
        let [ahead, _] = gains(&f, &at([14.0, 64.0, 10.0]), 1);
        assert!((ahead[0] - ahead[1]).abs() < 1e-5);
        let [left, _] = gains(&f, &at([10.0, 64.0, 6.0]), 1);
        assert!(left[1].abs() < 1e-6 && left[0] > 0.7, "{left:?}");
    }

    #[test]
    fn sounds_fade_linearly_to_their_distance_and_no_further() {
        let f = north();
        let far = |d: f64, attenuation| {
            let v = Voice {
                attenuation,
                ..at([0.0, d, 0.0])
            };
            gains(&f, &v, 1)[0][0]
        };
        // Overhead pans to W alone, 0.5 a speaker.
        assert!((far(4.0, Some(16.0)) - 0.5 * 0.75).abs() < 1e-5);
        assert_eq!(far(16.0, Some(16.0)), 0.0);
        assert_eq!(far(40.0, Some(16.0)), 0.0);
        assert!((far(40.0, None) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn music_at_the_listener_plays_its_channels_on_their_own_speakers() {
        let v = Voice {
            relative: true,
            attenuation: None,
            ..at([0.0; 3])
        };
        let [left, right] = gains(&north(), &v, 2);
        assert!(
            close(left, [1.0, 0.0]) && close(right, [0.0, 1.0]),
            "{left:?} {right:?}"
        );
        // A mono sound there plays in the middle.
        let [mono, _] = gains(&north(), &v, 1);
        assert!(close(mono, [0.595_662, 0.595_662]), "{mono:?}");
    }
}
