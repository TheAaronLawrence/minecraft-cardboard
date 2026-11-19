//! The wire types packets are built from, as vanilla's `FriendlyByteBuf` writes them:
//! big-endian numbers, VarInts, length-prefixed UTF-8 and byte arrays, and network NBT.

use anyhow::{Context, Result, bail, ensure};

/// Reads one packet's body.
pub struct Reader<'a> {
    bytes: &'a [u8],
}

macro_rules! numbers {
    ($($name:ident: $t:ty),*) => {$(
        pub fn $name(&mut self) -> Result<$t> {
            Ok(<$t>::from_be_bytes(self.take(size_of::<$t>())?.try_into()?))
        }
    )*};
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len()
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        ensure!(n <= self.bytes.len(), "packet ended early");
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Ok(head)
    }

    numbers!(u8: u8, i8: i8, u16: u16, i16: i16, i32: i32, i64: i64, f32: f32, f64: f64);

    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }

    pub fn varint(&mut self) -> Result<i32> {
        let mut value = 0u32;
        for i in 0..5 {
            let byte = self.u8()?;
            value |= ((byte & 0x7F) as u32) << (7 * i);
            if byte & 0x80 == 0 {
                return Ok(value as i32);
            }
        }
        bail!("VarInt longer than 5 bytes")
    }

    pub fn varlong(&mut self) -> Result<i64> {
        let mut value = 0u64;
        for i in 0..10 {
            let byte = self.u8()?;
            value |= ((byte & 0x7F) as u64) << (7 * i);
            if byte & 0x80 == 0 {
                return Ok(value as i64);
            }
        }
        bail!("VarLong longer than 10 bytes")
    }

    /// A `BlockPos` packed into a long: x in the top 26 bits, then z in 26, then y in 12.
    pub fn block_pos(&mut self) -> Result<[i32; 3]> {
        let v = self.i64()?;
        Ok([
            (v >> 38) as i32,
            (v << 52 >> 52) as i32,
            (v << 26 >> 38) as i32,
        ])
    }

    /// A VarInt that counts something, so it can't be negative.
    pub fn count(&mut self) -> Result<usize> {
        usize::try_from(self.varint()?).context("negative length")
    }

    pub fn string(&mut self) -> Result<String> {
        let n = self.count()?;
        Ok(std::str::from_utf8(self.take(n)?)?.to_owned())
    }

    pub fn byte_array(&mut self) -> Result<&'a [u8]> {
        let n = self.count()?;
        self.take(n)
    }

    /// `n` longs with no length before them (`writeFixedSizeLongArray`).
    pub fn longs(&mut self, n: usize) -> Result<Vec<u64>> {
        (0..n).map(|_| Ok(self.i64()? as u64)).collect()
    }

    /// A VarInt count, then that many longs, as `LONG_ARRAY` and `BIT_SET` write them.
    pub fn long_array(&mut self) -> Result<Vec<u64>> {
        let n = self.count()?;
        self.longs(n)
    }

    /// `BIT_SET`: since 26.3, a length-prefixed byte array, `BitSet.toByteArray`'s little-
    /// endian bytes. Bit i is byte i / 8, bit i % 8.
    pub fn bit_set(&mut self) -> Result<&'a [u8]> {
        self.byte_array()
    }

    /// Network NBT: a type byte and the payload with no name; an end tag means none.
    pub fn nbt(&mut self) -> Result<Option<mc_nbt::Tag>> {
        let (tag, used) = mc_nbt::read_network(self.bytes)?;
        self.bytes = &self.bytes[used..];
        Ok(tag)
    }

    /// The rest of the body.
    pub fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.bytes)
    }
}

/// Builds a packet's body.
pub trait Put {
    fn put_u8(&mut self, v: u8) -> &mut Self;
    fn put_bool(&mut self, v: bool) -> &mut Self;
    fn put_u16(&mut self, v: u16) -> &mut Self;
    fn put_i32(&mut self, v: i32) -> &mut Self;
    fn put_i64(&mut self, v: i64) -> &mut Self;
    fn put_f32(&mut self, v: f32) -> &mut Self;
    fn put_f64(&mut self, v: f64) -> &mut Self;
    fn put_varint(&mut self, v: i32) -> &mut Self;
    fn put_string(&mut self, v: &str) -> &mut Self;
    fn put_uuid(&mut self, v: u128) -> &mut Self;
    fn put_block_pos(&mut self, pos: [i32; 3]) -> &mut Self;
}

impl Put for Vec<u8> {
    fn put_u8(&mut self, v: u8) -> &mut Self {
        self.push(v);
        self
    }
    fn put_bool(&mut self, v: bool) -> &mut Self {
        self.put_u8(v as u8)
    }
    fn put_u16(&mut self, v: u16) -> &mut Self {
        self.extend(v.to_be_bytes());
        self
    }
    fn put_i32(&mut self, v: i32) -> &mut Self {
        self.extend(v.to_be_bytes());
        self
    }
    fn put_i64(&mut self, v: i64) -> &mut Self {
        self.extend(v.to_be_bytes());
        self
    }
    fn put_f32(&mut self, v: f32) -> &mut Self {
        self.extend(v.to_be_bytes());
        self
    }
    fn put_f64(&mut self, v: f64) -> &mut Self {
        self.extend(v.to_be_bytes());
        self
    }
    fn put_varint(&mut self, v: i32) -> &mut Self {
        let mut v = v as u32;
        loop {
            if v < 0x80 {
                self.push(v as u8);
                return self;
            }
            self.push((v & 0x7F) as u8 | 0x80);
            v >>= 7;
        }
    }
    fn put_string(&mut self, v: &str) -> &mut Self {
        self.put_varint(v.len() as i32);
        self.extend_from_slice(v.as_bytes());
        self
    }
    fn put_block_pos(&mut self, [x, y, z]: [i32; 3]) -> &mut Self {
        self.put_i64(
            (x as i64 & 0x3FF_FFFF) << 38 | (z as i64 & 0x3FF_FFFF) << 12 | (y as i64 & 0xFFF),
        )
    }

    fn put_uuid(&mut self, v: u128) -> &mut Self {
        self.extend(v.to_be_bytes());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VarInts as the protocol documents them, including negatives, which take 5 bytes.
    #[test]
    fn varints_round_trip() {
        for (value, bytes) in [
            (0, vec![0x00]),
            (127, vec![0x7F]),
            (128, vec![0x80, 0x01]),
            (25565, vec![0xDD, 0xC7, 0x01]),
            (i32::MAX, vec![0xFF, 0xFF, 0xFF, 0xFF, 0x07]),
            (-1, vec![0xFF, 0xFF, 0xFF, 0xFF, 0x0F]),
            (i32::MIN, vec![0x80, 0x80, 0x80, 0x80, 0x08]),
        ] {
            let mut out = Vec::new();
            out.put_varint(value);
            assert_eq!(out, bytes, "{value}");
            assert_eq!(Reader::new(&bytes).varint().unwrap(), value);
        }
    }
}
