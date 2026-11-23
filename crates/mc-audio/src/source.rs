//! What a voice plays: a whole sound decoded into memory (`SoundBufferLibrary.getCompleteBuffer`),
//! or an Ogg file decoded a packet at a time as it plays (`SoundBufferLibrary.getStream`), as
//! vanilla does for music and other sounds marked `"stream": true`.

use anyhow::{Context, Result};
use lewton::inside_ogg::OggStreamReader;
use std::io::Cursor;
use std::sync::Arc;

/// A decoded sound: its samples, interleaved by channel, from -1 to 1.
pub struct Buffer {
    pub channels: usize,
    pub rate: u32,
    pub samples: Vec<f32>,
}

impl Buffer {
    /// Decodes a whole Ogg Vorbis file.
    pub fn decode(bytes: Vec<u8>) -> Result<Self> {
        let mut reader = OggStreamReader::new(Cursor::new(bytes)).context("not Ogg Vorbis")?;
        let channels = usize::from(reader.ident_hdr.audio_channels);
        let rate = reader.ident_hdr.audio_sample_rate;
        let mut samples = Vec::new();
        while let Some(packet) = reader.read_dec_packet_itl()? {
            samples.extend(packet.into_iter().map(|s| f32::from(s) / 32768.0));
        }
        Ok(Self {
            channels,
            rate,
            samples,
        })
    }

    fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1)
    }
}

/// An Ogg file being decoded as it plays, keeping only the frames around the play head.
pub struct Stream {
    bytes: Arc<[u8]>,
    reader: OggStreamReader<Cursor<Arc<[u8]>>>,
    pub channels: usize,
    pub rate: u32,
    looping: bool,
    /// Decoded frames, interleaved, starting at frame `start` of the whole sound.
    frames: Vec<f32>,
    start: usize,
    /// The file has no more packets (and doesn't loop).
    ended: bool,
    /// Something was decoded since the stream last started over; an empty file ends.
    decoded: bool,
}

impl Stream {
    pub fn open(bytes: Vec<u8>, looping: bool) -> Result<Self> {
        let bytes: Arc<[u8]> = bytes.into();
        let reader = OggStreamReader::new(Cursor::new(bytes.clone())).context("not Ogg Vorbis")?;
        Ok(Self {
            channels: usize::from(reader.ident_hdr.audio_channels),
            rate: reader.ident_hdr.audio_sample_rate,
            bytes,
            reader,
            looping,
            frames: Vec::new(),
            start: 0,
            ended: false,
            decoded: false,
        })
    }

    /// Decodes until frame `until` is held or the sound ends; a looping stream starts over
    /// at its end, as `LoopingAudioStream` does.
    fn fill(&mut self, until: usize) {
        while !self.ended && self.start + self.frames.len() / self.channels <= until {
            match self.reader.read_dec_packet_itl() {
                Ok(Some(packet)) => {
                    self.decoded |= !packet.is_empty();
                    self.frames
                        .extend(packet.into_iter().map(|s| f32::from(s) / 32768.0));
                }
                Ok(None) if self.looping && self.decoded => {
                    self.decoded = false;
                    match OggStreamReader::new(Cursor::new(self.bytes.clone())) {
                        Ok(reader) => self.reader = reader,
                        Err(_) => self.ended = true,
                    }
                }
                _ => self.ended = true,
            }
        }
    }

    /// Lets go of the frames before `frame`.
    fn forget_before(&mut self, frame: usize) {
        let drop = frame.saturating_sub(self.start);
        if drop >= 4096 {
            self.frames.drain(..drop * self.channels);
            self.start += drop;
        }
    }
}

pub enum Source {
    Buffer(Arc<Buffer>),
    Stream(Box<Stream>),
}

impl Source {
    pub fn channels(&self) -> usize {
        match self {
            Source::Buffer(b) => b.channels,
            Source::Stream(s) => s.channels,
        }
    }

    pub fn rate(&self) -> u32 {
        match self {
            Source::Buffer(b) => b.rate,
            Source::Stream(s) => s.rate,
        }
    }

    /// Channel `ch` of frame `frame`, wrapping a looping buffer; silence before the start
    /// and after the end.
    pub fn sample(&mut self, frame: isize, ch: usize, looping: bool) -> f32 {
        match self {
            Source::Buffer(b) => {
                let len = b.frames() as isize;
                if len == 0 {
                    return 0.0;
                }
                let frame = if looping {
                    frame.rem_euclid(len)
                } else {
                    frame
                };
                if (0..len).contains(&frame) {
                    b.samples[frame as usize * b.channels + ch]
                } else {
                    0.0
                }
            }
            Source::Stream(s) => {
                if frame < s.start as isize {
                    return 0.0;
                }
                let frame = frame as usize;
                s.fill(frame);
                s.frames
                    .get((frame - s.start) * s.channels + ch)
                    .copied()
                    .unwrap_or(0.0)
            }
        }
    }

    /// Whether frame `frame` is past the end of a sound that doesn't loop.
    pub fn ended(&mut self, frame: usize, looping: bool) -> bool {
        match self {
            Source::Buffer(b) => !looping && frame >= b.frames(),
            Source::Stream(s) => {
                s.fill(frame);
                s.forget_before(frame.saturating_sub(1));
                s.ended && frame >= s.start + s.frames.len() / s.channels
            }
        }
    }
}
