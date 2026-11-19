//! Java Edition NBT: big-endian binary tags with Java's modified UTF-8 strings.
//!
//! Files (`level.dat`, region chunks, structures) hold a named root compound; network
//! packets since 1.20.2 hold an unnamed root of any tag type. Compression (gzip, zlib) is
//! the caller's job.

use indexmap::IndexMap;
use std::fmt;

/// Compound entries keep their file order, so a value written back out is byte-identical.
pub type Compound = IndexMap<String, Tag>;

/// Vanilla refuses deeper nesting with the same limit.
const MAX_DEPTH: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(List),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

/// A list of tags that all have the same type. `element_type` is kept for empty lists,
/// which older files write with a type other than End.
#[derive(Debug, Clone, PartialEq)]
pub struct List {
    pub element_type: u8,
    pub items: Vec<Tag>,
}

impl List {
    /// Panics when `items` mixes tag types, which NBT cannot encode.
    pub fn new(items: Vec<Tag>) -> Self {
        let element_type = items.first().map_or(END, Tag::id);
        assert!(
            items.iter().all(|t| t.id() == element_type),
            "NBT list items must share one type"
        );
        Self {
            element_type,
            items,
        }
    }
}

const END: u8 = 0;

impl Tag {
    pub fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => 1,
            Tag::Short(_) => 2,
            Tag::Int(_) => 3,
            Tag::Long(_) => 4,
            Tag::Float(_) => 5,
            Tag::Double(_) => 6,
            Tag::ByteArray(_) => 7,
            Tag::String(_) => 8,
            Tag::List(_) => 9,
            Tag::Compound(_) => 10,
            Tag::IntArray(_) => 11,
            Tag::LongArray(_) => 12,
        }
    }

    pub fn as_compound(&self) -> Option<&Compound> {
        if let Tag::Compound(c) = self {
            Some(c)
        } else {
            None
        }
    }

    pub fn as_list(&self) -> Option<&[Tag]> {
        if let Tag::List(l) = self {
            Some(&l.items)
        } else {
            None
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let Tag::String(s) = self {
            Some(s)
        } else {
            None
        }
    }

    /// Byte, Short, Int or Long as an i64.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Tag::Byte(v) => Some(v.into()),
            Tag::Short(v) => Some(v.into()),
            Tag::Int(v) => Some(v.into()),
            Tag::Long(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid NBT: {}", self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn err<T>(message: impl Into<String>) -> Result<T> {
    Err(Error(message.into()))
}

/// Reads a file's root: a compound with a name (usually empty). Trailing bytes are an error.
pub fn from_bytes(bytes: &[u8]) -> Result<(String, Compound)> {
    let mut r = Reader { bytes, depth: 0 };
    if r.u8()? != 10 {
        return err("root is not a compound");
    }
    let name = r.string()?;
    let root = r.compound()?;
    r.finish()?;
    Ok((name, root))
}

/// Reads a network root: a type byte, then the payload with no name.
pub fn from_network_bytes(bytes: &[u8]) -> Result<Tag> {
    let mut r = Reader { bytes, depth: 0 };
    let id = r.u8()?;
    let tag = r.payload(id)?;
    r.finish()?;
    Ok(tag)
}

/// Reads a network root at the start of `bytes`, as packets embed it, and how many bytes it
/// took. A lone end tag (0) is how packets write "no tag", read as `None`.
pub fn read_network(bytes: &[u8]) -> Result<(Option<Tag>, usize)> {
    let mut r = Reader { bytes, depth: 0 };
    let tag = match r.u8()? {
        0 => None,
        id => Some(r.payload(id)?),
    };
    Ok((tag, bytes.len() - r.bytes.len()))
}

pub fn to_bytes(name: &str, root: &Compound) -> Vec<u8> {
    let mut out = vec![10];
    write_string(&mut out, name);
    write_compound(&mut out, root);
    out
}

pub fn to_network_bytes(root: &Tag) -> Vec<u8> {
    let mut out = vec![root.id()];
    write_payload(&mut out, root);
    out
}

struct Reader<'a> {
    bytes: &'a [u8],
    depth: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N]> {
        let Some((head, rest)) = self.bytes.split_first_chunk::<N>() else {
            return err("unexpected end of data");
        };
        self.bytes = rest;
        Ok(*head)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take::<1>()?[0])
    }

    fn finish(&self) -> Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            err(format!("{} bytes after the root tag", self.bytes.len()))
        }
    }

    /// An array or list length, checked against the bytes left so a bad length can't
    /// make us allocate gigabytes.
    fn len(&mut self, min_item_size: usize) -> Result<usize> {
        let len = i32::from_be_bytes(self.take()?);
        let Ok(len) = usize::try_from(len) else {
            return err(format!("negative length {len}"));
        };
        if len.saturating_mul(min_item_size) > self.bytes.len() {
            return err(format!("length {len} is longer than the data"));
        }
        Ok(len)
    }

    fn string(&mut self) -> Result<String> {
        let len = u16::from_be_bytes(self.take()?) as usize;
        if len > self.bytes.len() {
            return err("string is longer than the data");
        }
        let (raw, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        decode_modified_utf8(raw)
    }

    fn compound(&mut self) -> Result<Compound> {
        self.nest()?;
        let mut map = Compound::new();
        loop {
            let id = self.u8()?;
            if id == END {
                break;
            }
            let name = self.string()?;
            let value = self.payload(id)?;
            map.insert(name, value);
        }
        self.depth -= 1;
        Ok(map)
    }

    fn nest(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            err(format!("nested deeper than {MAX_DEPTH}"))
        } else {
            Ok(())
        }
    }

    fn payload(&mut self, id: u8) -> Result<Tag> {
        Ok(match id {
            1 => Tag::Byte(i8::from_be_bytes(self.take()?)),
            2 => Tag::Short(i16::from_be_bytes(self.take()?)),
            3 => Tag::Int(i32::from_be_bytes(self.take()?)),
            4 => Tag::Long(i64::from_be_bytes(self.take()?)),
            5 => Tag::Float(f32::from_be_bytes(self.take()?)),
            6 => Tag::Double(f64::from_be_bytes(self.take()?)),
            7 => {
                let n = self.len(1)?;
                (0..n)
                    .map(|_| Ok(i8::from_be_bytes(self.take()?)))
                    .collect::<Result<_>>()
                    .map(Tag::ByteArray)?
            }
            8 => Tag::String(self.string()?),
            9 => {
                self.nest()?;
                let element_type = self.u8()?;
                if element_type > 12 {
                    return err(format!("unknown list element type {element_type}"));
                }
                // An End-typed list must be empty; every other element takes at least a byte.
                let n = self.len(1)?;
                if element_type == END && n > 0 {
                    return err("non-empty list of End tags");
                }
                let items = (0..n)
                    .map(|_| self.payload(element_type))
                    .collect::<Result<_>>()?;
                self.depth -= 1;
                Tag::List(List {
                    element_type,
                    items,
                })
            }
            10 => Tag::Compound(self.compound()?),
            11 => {
                let n = self.len(4)?;
                (0..n)
                    .map(|_| Ok(i32::from_be_bytes(self.take()?)))
                    .collect::<Result<_>>()
                    .map(Tag::IntArray)?
            }
            12 => {
                let n = self.len(8)?;
                (0..n)
                    .map(|_| Ok(i64::from_be_bytes(self.take()?)))
                    .collect::<Result<_>>()
                    .map(Tag::LongArray)?
            }
            _ => return err(format!("unknown tag type {id}")),
        })
    }
}

fn write_payload(out: &mut Vec<u8>, tag: &Tag) {
    match tag {
        Tag::Byte(v) => out.extend(v.to_be_bytes()),
        Tag::Short(v) => out.extend(v.to_be_bytes()),
        Tag::Int(v) => out.extend(v.to_be_bytes()),
        Tag::Long(v) => out.extend(v.to_be_bytes()),
        Tag::Float(v) => out.extend(v.to_be_bytes()),
        Tag::Double(v) => out.extend(v.to_be_bytes()),
        Tag::ByteArray(v) => {
            write_len(out, v.len());
            out.extend(v.iter().map(|b| *b as u8));
        }
        Tag::String(s) => write_string(out, s),
        Tag::List(list) => {
            out.push(list.element_type);
            write_len(out, list.items.len());
            for item in &list.items {
                debug_assert_eq!(
                    item.id(),
                    list.element_type,
                    "NBT list items must share one type"
                );
                write_payload(out, item);
            }
        }
        Tag::Compound(map) => write_compound(out, map),
        Tag::IntArray(v) => {
            write_len(out, v.len());
            v.iter().for_each(|x| out.extend(x.to_be_bytes()));
        }
        Tag::LongArray(v) => {
            write_len(out, v.len());
            v.iter().for_each(|x| out.extend(x.to_be_bytes()));
        }
    }
}

fn write_compound(out: &mut Vec<u8>, map: &Compound) {
    for (name, value) in map {
        out.push(value.id());
        write_string(out, name);
        write_payload(out, value);
    }
    out.push(END);
}

fn write_len(out: &mut Vec<u8>, len: usize) {
    out.extend(
        i32::try_from(len)
            .expect("NBT array longer than i32::MAX")
            .to_be_bytes(),
    );
}

/// Panics on strings over 65,535 encoded bytes, which Java's `writeUTF` also refuses.
fn write_string(out: &mut Vec<u8>, s: &str) {
    let encoded = encode_modified_utf8(s);
    out.extend(
        u16::try_from(encoded.len())
            .expect("NBT string longer than 65535 bytes")
            .to_be_bytes(),
    );
    out.extend(encoded);
}

/// Java's modified UTF-8: NUL takes two bytes and characters above U+FFFF are written as
/// two three-byte surrogates instead of one four-byte sequence.
fn encode_modified_utf8(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for unit in s.encode_utf16() {
        match unit {
            0x01..=0x7F => out.push(unit as u8),
            0x00 | 0x80..=0x7FF => {
                out.extend([0xC0 | (unit >> 6) as u8, 0x80 | (unit & 0x3F) as u8])
            }
            _ => out.extend([
                0xE0 | (unit >> 12) as u8,
                0x80 | ((unit >> 6) & 0x3F) as u8,
                0x80 | (unit & 0x3F) as u8,
            ]),
        }
    }
    out
}

/// A lone surrogate, which Java strings allow but Rust's don't, becomes U+FFFD.
fn decode_modified_utf8(bytes: &[u8]) -> Result<String> {
    // Plain ASCII without NUL is the same in both encodings, and is most strings.
    if bytes.iter().all(|b| (1..0x80).contains(b)) {
        return Ok(std::str::from_utf8(bytes)
            .expect("ASCII is UTF-8")
            .to_owned());
    }
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let cont = |i: usize| match bytes.get(i) {
        Some(b) if b & 0xC0 == 0x80 => Ok(u16::from(b & 0x3F)),
        _ => err("bad modified UTF-8"),
    };
    while i < bytes.len() {
        let b = bytes[i];
        let (unit, width) = match b {
            0x01..=0x7F => (u16::from(b), 1),
            0xC0..=0xDF => ((u16::from(b & 0x1F) << 6) | cont(i + 1)?, 2),
            0xE0..=0xEF => (
                (u16::from(b & 0x0F) << 12) | (cont(i + 1)? << 6) | cont(i + 2)?,
                3,
            ),
            _ => return err("bad modified UTF-8"),
        };
        units.push(unit);
        i += width;
    }
    Ok(char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_use_java_modified_utf8() {
        // NUL is C0 80, é is two bytes, and 😀 (U+1F600) is two 3-byte surrogates, not F0 9F 98 80.
        let s = "a\0é😀";
        let expected = [
            b'a', 0xC0, 0x80, 0xC3, 0xA9, 0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80,
        ];
        assert_eq!(encode_modified_utf8(s), expected);
        assert_eq!(decode_modified_utf8(&expected).unwrap(), s);
    }

    #[test]
    fn empty_lists_keep_their_element_type() {
        // An empty list of compounds, as older saves write them; vanilla's current files use End.
        let bytes = [9, 10, 0, 0, 0, 0];
        let tag = from_network_bytes(&bytes).unwrap();
        assert_eq!(
            tag,
            Tag::List(List {
                element_type: 10,
                items: vec![]
            })
        );
        assert_eq!(to_network_bytes(&tag), bytes);
    }

    #[test]
    fn rejects_lengths_longer_than_the_data() {
        // A network Int Array claiming 2^31-1 elements with no data behind it.
        assert!(from_network_bytes(&[11, 0x7F, 0xFF, 0xFF, 0xFF]).is_err());
    }
}
