//! Packet framing: each packet is a VarInt length, then its id and body. Once the server
//! sets a threshold, the length is followed by the uncompressed size (0 when the packet was
//! sent as is) and zlib data.

use crate::codec::{Put, Reader};
use anyhow::{Context, Result, ensure};
use std::io::{BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

/// Vanilla rejects packets larger than this (`CompressionDecoder.MAXIMUM_UNCOMPRESSED_LENGTH`).
const MAX_PACKET: usize = 8 * 1024 * 1024;

pub struct Connection {
    reader: BufReader<TcpStream>,
    writer: Writer,
    threshold: Option<usize>,
}

/// The sending half, shareable between threads; each packet is written whole.
#[derive(Clone)]
pub struct Writer {
    stream: Arc<Mutex<TcpStream>>,
    threshold: Option<usize>,
}

impl Connection {
    pub fn open(address: &str) -> Result<Self> {
        let stream =
            TcpStream::connect(address).with_context(|| format!("connecting to {address}"))?;
        stream.set_nodelay(true)?;
        Ok(Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: Writer {
                stream: Arc::new(Mutex::new(stream)),
                threshold: None,
            },
            threshold: None,
        })
    }

    /// Packets this size or larger are compressed from now on; a negative threshold turns
    /// compression off.
    pub fn compress(&mut self, threshold: i32) {
        self.threshold = usize::try_from(threshold).ok();
        self.writer.threshold = self.threshold;
    }

    pub fn send(&mut self, id: i32, body: &[u8]) -> Result<()> {
        self.writer.send(id, body)
    }

    /// Another handle for sending, with the current compression.
    pub fn writer(&self) -> Writer {
        self.writer.clone()
    }
}

impl Writer {
    pub fn send(&self, id: i32, body: &[u8]) -> Result<()> {
        let mut packet = Vec::with_capacity(body.len() + 5);
        packet.put_varint(id);
        packet.extend_from_slice(body);
        let payload = match self.threshold {
            None => packet,
            Some(threshold) if packet.len() < threshold => {
                let mut out = vec![0];
                out.extend(packet);
                out
            }
            Some(_) => {
                let mut out = Vec::new();
                out.put_varint(packet.len() as i32);
                let mut z = flate2::write::ZlibEncoder::new(out, flate2::Compression::default());
                z.write_all(&packet)?;
                z.finish()?
            }
        };
        let mut frame = Vec::with_capacity(payload.len() + 5);
        frame.put_varint(payload.len() as i32);
        frame.extend(payload);
        self.stream
            .lock()
            .expect("writer poisoned")
            .write_all(&frame)?;
        Ok(())
    }
}

impl Connection {
    /// The next packet's id and body.
    pub fn recv(&mut self) -> Result<(i32, Vec<u8>)> {
        let length = self.read_varint()?;
        ensure!(
            length <= MAX_PACKET,
            "packet of {length} bytes is too large"
        );
        let mut payload = vec![0; length];
        self.reader
            .read_exact(&mut payload)
            .context("the server closed the connection")?;
        let packet = match self.threshold {
            None => payload,
            Some(_) => {
                let mut r = Reader::new(&payload);
                let size = r.count()?;
                if size == 0 {
                    r.rest().to_vec()
                } else {
                    ensure!(size <= MAX_PACKET, "packet of {size} bytes is too large");
                    let mut out = Vec::with_capacity(size);
                    flate2::read::ZlibDecoder::new(r.rest()).read_to_end(&mut out)?;
                    ensure!(out.len() == size, "packet inflated to the wrong size");
                    out
                }
            }
        };
        let mut r = Reader::new(&packet);
        let id = r.varint()?;
        Ok((id, r.rest().to_vec()))
    }

    fn read_varint(&mut self) -> Result<usize> {
        let mut value = 0u32;
        for i in 0..5 {
            let mut byte = [0];
            self.reader
                .read_exact(&mut byte)
                .context("the server closed the connection")?;
            value |= ((byte[0] & 0x7F) as u32) << (7 * i);
            if byte[0] & 0x80 == 0 {
                return Ok(value as usize);
            }
        }
        anyhow::bail!("VarInt longer than 5 bytes")
    }
}
