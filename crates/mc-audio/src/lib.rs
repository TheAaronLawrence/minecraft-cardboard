//! Sound output, standing in for the OpenAL that vanilla plays through (`com.mojang.blaze3d.audio`):
//! voices that play Ogg Vorbis files at a position, with a gain, a pitch and a fade over
//! distance, heard by a listener, mixed as OpenAL Soft 1.25 mixes them for a stereo device and
//! played on the system's default output with `cpal`. Which sounds play, and when, is the
//! game's business.

mod mix;
mod pan;
mod source;

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use mix::{Command, Mixer};
use source::{Buffer, Source, Stream};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

/// How a voice plays, as vanilla sets up an OpenAL source (`Channel`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Voice {
    /// In the world, or from the listener when `relative`, in the listener's frame (x right,
    /// y up, z back).
    pub pos: [f64; 3],
    pub relative: bool,
    /// `Channel.linearAttenuation`: silent at this distance, full at the listener; `None`
    /// for `disableAttenuation`.
    pub attenuation: Option<f32>,
    /// 0 to 1.
    pub gain: f32,
    /// 0.5 to 2: how fast it plays, which raises it as much.
    pub pitch: f32,
    pub looping: bool,
}

/// The listener, where the camera is (`Listener.setTransform`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub pos: [f64; 3],
    pub forward: [f32; 3],
    pub up: [f32; 3],
}

impl Default for Listener {
    /// `Listener.reset`.
    fn default() -> Self {
        Self {
            pos: [0.0; 3],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
        }
    }
}

enum Request {
    Play {
        id: u64,
        path: PathBuf,
        stream: bool,
        voice: Voice,
    },
    Pass(Command),
}

/// The open audio device. Voices are numbered as they're started; the files load on a
/// thread of their own, as `SoundBufferLibrary` loads them off the game's thread, and each
/// starts once loaded.
pub struct Audio {
    requests: Sender<Request>,
    done: Receiver<u64>,
    next: u64,
    _stream: cpal::Stream,
}

impl Audio {
    /// Opens the default output device, as `Library.init` opens OpenAL's.
    pub fn open() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no audio output device")?;
        let config = device.default_output_config()?;
        let channels = usize::from(config.channels());
        let rate = config.sample_rate();
        let (commands, mixer_rx) = channel();
        let (done_tx, done) = channel();
        let mut mixer = Mixer::new(mixer_rx, done_tx.clone(), rate);
        let err = |e: cpal::Error| eprintln!("audio: {e}");
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config.into(),
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| mixer.render(out, channels),
                err,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_output_stream(
                config.into(),
                move |out: &mut [i16], _: &cpal::OutputCallbackInfo| mixer.render(out, channels),
                err,
                None,
            )?,
            cpal::SampleFormat::I32 => device.build_output_stream(
                config.into(),
                move |out: &mut [i32], _: &cpal::OutputCallbackInfo| mixer.render(out, channels),
                err,
                None,
            )?,
            cpal::SampleFormat::U16 => device.build_output_stream(
                config.into(),
                move |out: &mut [u16], _: &cpal::OutputCallbackInfo| mixer.render(out, channels),
                err,
                None,
            )?,
            other => anyhow::bail!("unsupported audio sample format {other}"),
        };
        stream.play()?;

        let (requests, rx) = channel();
        std::thread::Builder::new()
            .name("sound loader".into())
            .spawn(move || load(rx, commands, done_tx))?;
        Ok(Self {
            requests,
            done,
            next: 0,
            _stream: stream,
        })
    }

    /// Starts playing the Ogg file at `path`, decoded whole first unless `stream`. Returns
    /// the voice's number.
    pub fn play(&mut self, path: &Path, stream: bool, voice: Voice) -> u64 {
        self.next += 1;
        let _ = self.requests.send(Request::Play {
            id: self.next,
            path: path.to_owned(),
            stream,
            voice,
        });
        self.next
    }

    /// Moves a voice or changes its gain or pitch.
    pub fn update(&self, id: u64, voice: Voice) {
        let _ = self
            .requests
            .send(Request::Pass(Command::Update { id, voice }));
    }

    pub fn stop(&self, id: u64) {
        let _ = self.requests.send(Request::Pass(Command::Stop { id }));
    }

    pub fn listener(&self, listener: Listener) {
        let _ = self
            .requests
            .send(Request::Pass(Command::Listener(listener)));
    }

    /// The voices that have stopped since the last call: played to the end, stopped, or
    /// whose file couldn't be read.
    pub fn finished(&self) -> Vec<u64> {
        self.done.try_iter().collect()
    }
}

/// The loading thread: reads and decodes files, keeping every decoded sound for next time,
/// and passes everything on to the mixer in the order it was asked for.
fn load(requests: Receiver<Request>, mixer: Sender<Command>, done: Sender<u64>) {
    let mut cache: HashMap<PathBuf, Arc<Buffer>> = HashMap::new();
    for request in requests {
        let command = match request {
            Request::Play {
                id,
                path,
                stream,
                voice,
            } => {
                let source = if stream {
                    std::fs::read(&path)
                        .map_err(anyhow::Error::from)
                        .and_then(|bytes| Stream::open(bytes, voice.looping))
                        .map(|s| Source::Stream(Box::new(s)))
                } else if let Some(buffer) = cache.get(&path) {
                    Ok(Source::Buffer(buffer.clone()))
                } else {
                    std::fs::read(&path)
                        .map_err(anyhow::Error::from)
                        .and_then(Buffer::decode)
                        .map(|b| {
                            let b = Arc::new(b);
                            cache.insert(path.clone(), b.clone());
                            Source::Buffer(b)
                        })
                };
                match source {
                    Ok(source) => Command::Start { id, source, voice },
                    Err(e) => {
                        eprintln!("can't play {}: {e:#}", path.display());
                        let _ = done.send(id);
                        continue;
                    }
                }
            }
            Request::Pass(command) => command,
        };
        if mixer.send(command).is_err() {
            return;
        }
    }
}
