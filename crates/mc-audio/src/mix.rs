//! The mixer the audio device calls for samples: every playing voice resampled to the
//! device's rate with OpenAL Soft's default Catmull-Rom spline, panned and faded as `pan`
//! works out, and summed, as OpenAL Soft mixes its sources.

use crate::source::Source;
use crate::{Listener, Voice, pan};
use std::sync::mpsc::{Receiver, Sender};

pub(crate) enum Command {
    Start {
        id: u64,
        source: Source,
        voice: Voice,
    },
    Update {
        id: u64,
        voice: Voice,
    },
    Stop {
        id: u64,
    },
    Listener(Listener),
}

struct Playing {
    id: u64,
    source: Source,
    voice: Voice,
    /// The play head, in the source's frames.
    pos: f64,
    /// The gains the last block ended on, which the next one ramps from so a moving sound
    /// doesn't click.
    gains: Option<[[f32; 2]; 2]>,
}

pub(crate) struct Mixer {
    commands: Receiver<Command>,
    done: Sender<u64>,
    voices: Vec<Playing>,
    frame: pan::Frame,
    rate: f64,
    mix: Vec<[f32; 2]>,
}

impl Mixer {
    pub fn new(commands: Receiver<Command>, done: Sender<u64>, rate: u32) -> Self {
        Self {
            commands,
            done,
            voices: Vec::new(),
            frame: pan::Frame::new(&Listener::default()),
            rate: f64::from(rate),
            mix: Vec::new(),
        }
    }

    fn take_commands(&mut self) {
        while let Ok(command) = self.commands.try_recv() {
            match command {
                Command::Start { id, source, voice } => self.voices.push(Playing {
                    id,
                    source,
                    voice,
                    pos: 0.0,
                    gains: None,
                }),
                Command::Update { id, voice } => {
                    if let Some(p) = self.voices.iter_mut().find(|p| p.id == id) {
                        p.voice = voice;
                    }
                }
                Command::Stop { id } => {
                    if let Some(i) = self.voices.iter().position(|p| p.id == id) {
                        self.voices.swap_remove(i);
                    }
                    let _ = self.done.send(id);
                }
                Command::Listener(l) => self.frame = pan::Frame::new(&l),
            }
        }
    }

    /// Fills `out`, interleaved for a device with `channels` channels: left and right go to
    /// the first two, a mono device gets their average.
    pub fn render<T: cpal::Sample + cpal::FromSample<f32>>(
        &mut self,
        out: &mut [T],
        channels: usize,
    ) {
        self.take_commands();
        let frames = out.len() / channels.max(1);
        self.mix.clear();
        self.mix.resize(frames, [0.0; 2]);
        let mut finished = Vec::new();
        for (index, p) in self.voices.iter_mut().enumerate() {
            let source_channels = p.source.channels().clamp(1, 2);
            let target = pan::gains(&self.frame, &p.voice, source_channels);
            let start = p.gains.unwrap_or(target);
            let step = f64::from(p.voice.pitch) * f64::from(p.source.rate()) / self.rate;
            let looping = p.voice.looping;
            for (i, out) in self.mix.iter_mut().enumerate() {
                let base = p.pos.floor();
                if p.source.ended(base as usize, looping) {
                    finished.push(index);
                    break;
                }
                let mu = (p.pos - base) as f32;
                let base = base as isize;
                let t = (i + 1) as f32 / frames as f32;
                for ch in 0..source_channels {
                    let g = [0, 1].map(|s| start[ch][s] + (target[ch][s] - start[ch][s]) * t);
                    let v = spline(
                        [-1, 0, 1, 2].map(|k| p.source.sample(base + k, ch, looping)),
                        mu,
                    );
                    out[0] += v * g[0];
                    out[1] += v * g[1];
                }
                p.pos += step;
            }
            p.gains = Some(target);
        }
        for &index in finished.iter().rev() {
            let p = self.voices.swap_remove(index);
            let _ = self.done.send(p.id);
        }
        for (frame, [l, r]) in out.chunks_mut(channels.max(1)).zip(&self.mix) {
            let (l, r) = (l.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0));
            for (c, sample) in frame.iter_mut().enumerate() {
                *sample = T::from_sample(match (channels, c) {
                    (1, _) => (l + r) * 0.5,
                    (_, 0) => l,
                    (_, 1) => r,
                    _ => 0.0,
                });
            }
        }
    }
}

/// OpenAL Soft's `Spline` resampler: a Catmull-Rom spline through four frames, `mu` of the
/// way from the second to the third.
fn spline([a, b, c, d]: [f32; 4], mu: f32) -> f32 {
    let c0 = ((-0.5 * mu + 1.0) * mu - 0.5) * mu;
    let c1 = (1.5 * mu - 2.5) * mu * mu + 1.0;
    let c2 = ((-1.5 * mu + 2.0) * mu + 0.5) * mu;
    let c3 = (0.5 * mu - 0.5) * mu * mu;
    a * c0 + b * c1 + c * c2 + d * c3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Buffer;
    use std::sync::Arc;
    use std::sync::mpsc::channel;

    #[test]
    fn the_spline_passes_through_its_frames() {
        assert_eq!(spline([3.0, 1.0, 2.0, 5.0], 0.0), 1.0);
        assert!((spline([3.0, 1.0, 2.0, 5.0], 1.0) - 2.0).abs() < 1e-6);
        // A straight line stays straight.
        assert!((spline([0.0, 1.0, 2.0, 3.0], 0.25) - 1.25).abs() < 1e-6);
    }

    /// A mono voice at the listener plays its samples at the front-centre gain, doubled in
    /// length at half pitch, and reports itself done after its last frame.
    #[test]
    fn voices_play_resample_and_finish() {
        let (tx, rx) = channel();
        let (done_tx, done) = channel();
        let mut mixer = Mixer::new(rx, done_tx, 100);
        let buffer = Arc::new(Buffer {
            channels: 1,
            rate: 100,
            samples: vec![0.5; 10],
        });
        let voice = Voice {
            pos: [0.0; 3],
            relative: true,
            attenuation: None,
            gain: 1.0,
            pitch: 0.5,
            looping: false,
        };
        tx.send(Command::Start {
            id: 7,
            source: Source::Buffer(buffer),
            voice,
        })
        .unwrap();
        let mut out = vec![0.0f32; 2 * 30];
        mixer.render(&mut out, 2);
        // Frame 0's spline reaches back to silence; the middle is steady.
        let middle = out[2 * 8];
        assert!((middle - 0.5 * 0.595_662).abs() < 1e-4, "{middle}");
        assert_eq!(out[2 * 8], out[2 * 8 + 1]);
        // 10 frames at half speed last 20; after that, silence and done.
        assert_eq!(out[2 * 25], 0.0);
        assert_eq!(done.try_recv(), Ok(7));
    }
}
