//! Chat as the player reads it: components (in their JSON form, which NBT components convert
//! to) flattened into runs of styled text, as `Component.visit` and the font's
//! `StringDecomposer.iterateFormatted` walk them; and chat types, which decorate a player's
//! message as `ChatTypeDecoration.decorate` does, such as `<name> message`.

use anyhow::{Context, Result};
use serde_json::{Map, Value};

/// A run of text in one style.
#[derive(Clone, PartialEq, Debug)]
pub struct Span {
    pub text: String,
    pub rgb: [u8; 3],
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    rgb: [u8; 3],
    bold: bool,
    italic: bool,
}

/// Unstyled text draws white.
const PLAIN: Style = Style {
    rgb: [255; 3],
    bold: false,
    italic: false,
};

/// `ChatFormatting`'s colours, by name, in the order of their `§` codes `0` to `f`.
const COLORS: [(&str, u32); 16] = [
    ("black", 0x000000),
    ("dark_blue", 0x0000AA),
    ("dark_green", 0x00AA00),
    ("dark_aqua", 0x00AAAA),
    ("dark_red", 0xAA0000),
    ("dark_purple", 0xAA00AA),
    ("gold", 0xFFAA00),
    ("gray", 0xAAAAAA),
    ("dark_gray", 0x555555),
    ("blue", 0x5555FF),
    ("green", 0x55FF55),
    ("aqua", 0x55FFFF),
    ("red", 0xFF5555),
    ("light_purple", 0xFF55FF),
    ("yellow", 0xFFFF55),
    ("white", 0xFFFFFF),
];

fn rgb(color: u32) -> [u8; 3] {
    [(color >> 16) as u8, (color >> 8) as u8, color as u8]
}

/// `TextColor.parseColor`: a `ChatFormatting` name or `#RRGGBB`.
fn parse_color(name: &str) -> Option<[u8; 3]> {
    match name.strip_prefix('#') {
        Some(hex) => u32::from_str_radix(hex, 16).ok().map(rgb),
        None => COLORS
            .iter()
            .find(|(n, _)| *n == name)
            .map(|&(_, c)| rgb(c)),
    }
}

/// A style flag: JSON has booleans, and NBT bytes.
fn flag(v: &Value) -> Option<bool> {
    v.as_bool().or_else(|| v.as_i64().map(|n| n != 0))
}

impl Style {
    /// This style with what a component sets over it (`Style.applyTo`).
    fn with(mut self, c: &Value) -> Self {
        if let Some(rgb) = c["color"].as_str().and_then(parse_color) {
            self.rgb = rgb;
        }
        if let Some(bold) = flag(&c["bold"]) {
            self.bold = bold;
        }
        if let Some(italic) = flag(&c["italic"]) {
            self.italic = italic;
        }
        self
    }
}

/// A component as runs of styled text, its translations from `lang`.
pub fn spans(c: &Value, lang: &Map<String, Value>) -> Vec<Span> {
    let mut out = Vec::new();
    visit(c, lang, PLAIN, &mut out);
    out
}

/// The spans' text, without their styles.
pub fn plain(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// `Component.visit`: the component's own text (`text`, or `translate` filled with `with`),
/// then each of `extra`, each in its style over its parent's. In a list, the first entry
/// is the parent of the rest.
fn visit(c: &Value, lang: &Map<String, Value>, style: Style, out: &mut Vec<Span>) {
    match c {
        Value::String(s) => push(out, s, style),
        Value::Array(parts) => {
            if let Some((first, rest)) = parts.split_first() {
                let parent = match first {
                    Value::Object(_) => style.with(first),
                    _ => style,
                };
                visit(first, lang, style, out);
                for part in rest {
                    visit(part, lang, parent, out);
                }
            }
        }
        Value::Object(_) => {
            let style = style.with(c);
            // A string in a list of mixed tags is a compound with an empty key.
            if let Some(text) = c["text"].as_str().or(c[""].as_str()) {
                push(out, text, style);
            } else if let Some(key) = c["translate"].as_str() {
                let format = lang
                    .get(key)
                    .and_then(Value::as_str)
                    .or(c["fallback"].as_str())
                    .unwrap_or(key);
                let args = c["with"].as_array().map_or(&[][..], Vec::as_slice);
                for piece in pieces(format) {
                    match piece {
                        Piece::Text(text) => push(out, &text, style),
                        Piece::Arg(i) => {
                            if let Some(arg) = args.get(i) {
                                visit(arg, lang, style, out);
                            }
                        }
                    }
                }
            } else if let Some(key) = c["keybind"].as_str() {
                push(out, key, style);
            }
            if let Some(extra) = c["extra"].as_array() {
                for e in extra {
                    visit(e, lang, style, out);
                }
            }
        }
        Value::Null => {}
        other => push(out, &other.to_string(), style),
    }
}

/// Text in `style`, its `§` codes changing the style as `StringDecomposer.iterateFormatted`
/// does: a colour also clears bold and italic, `l` is bold, `o` italic and `r` goes back to
/// `style`. Joins the last span when the style is the same.
fn push(out: &mut Vec<Span>, text: &str, style: Style) {
    let mut current = style;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\u{a7}' {
            let Some(code) = chars.next() else { break };
            let code = code.to_ascii_lowercase();
            if let Some(i) = code.to_digit(16) {
                current = Style {
                    rgb: rgb(COLORS[i as usize].1),
                    ..PLAIN
                };
            } else {
                match code {
                    'l' => current.bold = true,
                    'o' => current.italic = true,
                    'r' => current = style,
                    _ => {}
                }
            }
            continue;
        }
        match out.last_mut() {
            Some(last)
                if last.rgb == current.rgb
                    && last.bold == current.bold
                    && last.italic == current.italic =>
            {
                last.text.push(c);
            }
            _ => out.push(Span {
                text: c.to_string(),
                rgb: current.rgb,
                bold: current.bold,
                italic: current.italic,
            }),
        }
    }
}

enum Piece {
    Text(String),
    Arg(usize),
}

/// A translation split as `TranslatableContents.decomposeTemplate` splits it: text, `%s`
/// (the next argument), `%N$s` (argument N) and `%%` (a percent sign).
fn pieces(format: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut next = 0;
    let mut rest = format;
    while let Some(at) = rest.find('%') {
        text.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        let arg = if let Some(after) = rest.strip_prefix('%') {
            text.push('%');
            rest = after;
            None
        } else if let Some(after) = rest.strip_prefix('s') {
            next += 1;
            rest = after;
            Some(next - 1)
        } else if let Some((n, after)) = rest
            .split_once("$s")
            .and_then(|(n, after)| Some((n.parse::<usize>().ok()?, after)))
        {
            rest = after;
            Some(n.wrapping_sub(1))
        } else {
            text.push('%');
            None
        };
        if let Some(i) = arg {
            out.push(Piece::Text(std::mem::take(&mut text)));
            out.push(Piece::Arg(i));
        }
    }
    text.push_str(rest);
    out.push(Piece::Text(text));
    out
}

/// A chat type's `ChatTypeDecoration` for chat: its translation, which of `sender`,
/// `target` and `content` fill it, in order, and the style over it all.
pub(crate) struct ChatType {
    key: String,
    parameters: Vec<String>,
    style: Value,
}

/// The chat types by registry id, from the data the server sent or, for the ones it left
/// out as known, vanilla's own.
pub(crate) fn chat_types(entries: &[(String, Option<mc_nbt::Tag>)]) -> Vec<Option<ChatType>> {
    let res = mc_assets::Resources::new(mc_assets::default_root());
    entries
        .iter()
        .map(|(name, data)| {
            let json = match data {
                Some(data) => crate::nbt_json(data),
                None => {
                    let path = res.data_path("chat_type", name, "json");
                    serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?
                }
            };
            let chat = &json["chat"];
            Some(ChatType {
                key: chat["translation_key"].as_str()?.to_owned(),
                parameters: chat["parameters"]
                    .as_array()?
                    .iter()
                    .filter_map(|p| Some(p.as_str()?.to_owned()))
                    .collect(),
                style: chat["style"].clone(),
            })
        })
        .collect()
}

impl ChatType {
    /// `ChatTypeDecoration.decorate`: the translation, filled with the sender's name, the
    /// target's and the message, in the decoration's style.
    pub(crate) fn decorate(&self, name: Value, target: Option<Value>, content: Value) -> Value {
        let with: Vec<Value> = self
            .parameters
            .iter()
            .map(|p| match p.as_str() {
                "sender" => name.clone(),
                "target" => target.clone().unwrap_or_else(|| "".into()),
                _ => content.clone(),
            })
            .collect();
        let mut c = match &self.style {
            Value::Object(style) => style.clone(),
            _ => Map::new(),
        };
        c.insert("translate".into(), self.key.clone().into());
        c.insert("with".into(), with.into());
        Value::Object(c)
    }
}

/// `StringUtil.trimChatMessage` after `StringUtils.normalizeSpace`: what `ChatScreen` sends
/// for what was typed, at most 256 characters.
pub fn normalize(typed: &str) -> String {
    let joined = typed.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(256).collect()
}

/// A `ServerboundChatPacket` for an unsigned message (offline servers take them): the
/// message, when it was sent, a salt, no signature, and the last-seen update, which
/// acknowledges `offset` more signed messages and none of the window's (checksum 0 skips
/// the server's check).
pub(crate) fn chat_packet(message: &str, offset: i32) -> Result<Vec<u8>> {
    use crate::codec::Put;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("the clock is before 1970")?;
    // `SaltSupplier.getLong`; an unsigned message's salt isn't checked.
    let salt = {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(now.as_nanos());
        h.finish() as i64
    };
    let mut body = Vec::new();
    body.put_string(message)
        .put_i64(now.as_millis() as i64)
        .put_i64(salt)
        .put_bool(false)
        .put_varint(offset);
    // `LastSeenMessages.Update.acknowledged`: a fixed bit set of 20 bits, in 3 bytes.
    body.extend([0, 0, 0]);
    body.put_u8(0);
    Ok(body)
}

/// Reads the message (or name) components a chat packet carries as JSON.
pub(crate) fn component(tag: Option<mc_nbt::Tag>) -> Value {
    tag.as_ref().map_or(Value::Null, crate::nbt_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str, color: u32, bold: bool, italic: bool) -> Span {
        Span {
            text: text.into(),
            rgb: rgb(color),
            bold,
            italic,
        }
    }

    /// Styles pass to translation arguments and `extra` unless they set their own, `§`
    /// codes restyle literal text, and a chat type wraps a player's message in its
    /// translation and style.
    #[test]
    fn components_keep_their_styles() {
        let lang = serde_json::json!({
            "multiplayer.player.joined": "%s joined the game",
            "chat.type.text": "<%s> %s",
            "commands.message.display.incoming": "%s whispers to you: %s"
        });
        let lang = lang.as_object().unwrap();
        let joined = serde_json::json!({
            "translate": "multiplayer.player.joined",
            "color": "yellow",
            "with": [{ "text": "Alex", "color": "#123456" }],
            "extra": [{ "text": "!", "bold": 1 }]
        });
        assert_eq!(
            spans(&joined, lang),
            [
                span("Alex", 0x123456, false, false),
                span(" joined the game", 0xFFFF55, false, false),
                span("!", 0xFFFF55, true, false),
            ]
        );
        assert_eq!(
            spans(&"a\u{a7}lb\u{a7}cc\u{a7}rd".into(), lang),
            [
                span("a", 0xFFFFFF, false, false),
                span("b", 0xFFFFFF, true, false),
                span("c", 0xFF5555, false, false),
                span("d", 0xFFFFFF, false, false),
            ]
        );
        let whisper = ChatType {
            key: "commands.message.display.incoming".into(),
            parameters: vec!["sender".into(), "content".into()],
            style: serde_json::json!({ "color": "gray", "italic": true }),
        };
        let decorated = whisper.decorate("Alex".into(), None, "hi".into());
        assert_eq!(
            spans(&decorated, lang),
            [span("Alex whispers to you: hi", 0xAAAAAA, false, true)]
        );
        assert_eq!(normalize("  hello \t  there  "), "hello there");
    }
}
