//! The chat, as vanilla's `ChatComponent` draws it above the hotbar: each message wrapped
//! to the chat's width, newest at the bottom, fading 10 seconds after it arrived. With the
//! chat open (`ChatScreen`), the last 20 lines show in full, and the line being typed sits
//! in a bar along the bottom of the screen.

use super::menu::Field;
use super::{GuiMesh, HudArt, TEXT};
use mc_protocol::chat::Span;

/// `ChatComponent.getWidth` at the default chat width (1.0) and scale (1.0).
pub const WIDTH: i32 = 320;
/// Lines a page shows (`getLinesPerPage`): closed, at the default unfocused height (90 GUI
/// pixels), and open, at the focused height (180), 9 pixels a line.
const CLOSED_LINES: usize = 10;
pub const OPEN_LINES: usize = 20;
/// `MAX_CHAT_HISTORY`: lines kept.
pub const MAX_LINES: usize = 100;
/// `textBackgroundOpacity`'s default.
const BACKGROUND_OPACITY: f32 = 0.5;

/// A line of chat: its text and the tick its message arrived (`GuiMessage.addedTime`).
#[derive(Clone, PartialEq, Debug)]
pub struct Line {
    pub spans: Vec<Span>,
    pub added: i32,
}

/// What the chat shows.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Chat {
    /// Lines, newest first.
    pub lines: Vec<Line>,
    /// The tick now, which fades the lines.
    pub tick: i32,
    /// Lines scrolled up from the newest (`chatScrollbarPos`), with the chat open.
    pub scroll: usize,
    /// A message arrived while scrolled up, which turns the scroll bar red.
    pub new_since_scroll: bool,
    /// The chat screen's box, while it's open.
    pub input: Option<Field>,
    /// The box's cursor is in the visible half of its blink.
    pub cursor_on: bool,
}

/// How far a character advances in its style: one more when bold.
fn advance(art: &HudArt, c: char, bold: bool) -> i32 {
    art.glyphs
        .get(&c)
        .map_or(0, |&(_, a)| a as i32 + bold as i32)
}

/// `GuiMessage.splitLines` (`ComponentRenderUtils.wrapComponents`, through
/// `StringSplitter.splitLines`): the message broken into lines at most `max` GUI pixels
/// wide, at the last space that fits (which the break swallows) or else before the
/// character that doesn't, and at line breaks. A line that wraps from the one before
/// starts with a space.
pub fn wrap(art: &HudArt, spans: &[Span], max: i32) -> Vec<Vec<Span>> {
    let chars: Vec<(char, usize)> = spans
        .iter()
        .enumerate()
        .flat_map(|(i, s)| s.text.chars().map(move |c| (c, i)))
        .collect();
    let line = |range: &[(char, usize)], wrapped: bool| {
        let mut out: Vec<Span> = Vec::new();
        if wrapped {
            out.push(Span {
                text: " ".into(),
                ..spans[range.first().map_or(0, |&(_, i)| i)].clone()
            });
        }
        for &(c, i) in range {
            match out.last_mut() {
                Some(last)
                    if last.rgb == spans[i].rgb
                        && last.bold == spans[i].bold
                        && last.italic == spans[i].italic =>
                {
                    last.text.push(c)
                }
                _ => out.push(Span {
                    text: c.to_string(),
                    ..spans[i].clone()
                }),
            }
        }
        out
    };
    let mut lines = Vec::new();
    let (mut start, mut wrapped) = (0, false);
    loop {
        // `LineBreakFinder`: where this line ends, and how much after it to skip.
        let (mut width, mut space, mut visible) = (0, None, false);
        let mut end = None;
        for (i, &(c, s)) in chars.iter().enumerate().skip(start) {
            if c == '\n' {
                end = Some((i, 1, true));
                break;
            }
            if c == ' ' {
                space = Some(i);
            }
            let w = advance(art, c, spans[s].bold);
            width += w;
            if visible && width > max {
                end = Some(match space {
                    Some(at) => (at, 1, false),
                    None => (i, 0, false),
                });
                break;
            }
            visible |= w != 0;
        }
        match end {
            Some((at, skip, newline)) => {
                lines.push(line(&chars[start..at], wrapped));
                wrapped = !newline;
                start = at + skip;
            }
            None => {
                lines.push(line(&chars[start..], wrapped));
                return lines;
            }
        }
    }
}

/// `ChatComponent.AlphaCalculator.timeBased`: full for nine seconds, then fading over one.
fn fade(tick: i32, added: i32) -> f32 {
    let t = ((1.0 - (tick - added) as f64 / 200.0) * 10.0).clamp(0.0, 1.0);
    (t * t) as f32
}

impl GuiMesh {
    /// `ChatComponent.extractRenderState` in a GUI `width` × `height`, then with the chat
    /// open `ChatScreen.extractRenderState`'s bar and the box's text.
    pub(crate) fn chat(&mut self, art: &HudArt, chat: &Chat, width: i32, height: i32) {
        let open = chat.input.is_some();
        let bottom = height - 40;
        let per_page = if open { OPEN_LINES } else { CLOSED_LINES };
        let shown: Vec<(usize, &Line, f32)> = chat
            .lines
            .iter()
            .skip(chat.scroll)
            .take(per_page)
            .enumerate()
            .map(|(i, line)| {
                let alpha = if open {
                    1.0
                } else {
                    fade(chat.tick, line.added)
                };
                (i, line, alpha)
            })
            .filter(|&(.., alpha)| alpha > 1.0e-5)
            .collect();
        let black = |alpha: f32| [0, 0, 0, (alpha * 255.0) as u8];
        for &(i, line, alpha) in &shown {
            let entry_bottom = bottom - i as i32 * 9;
            let top = entry_bottom - 9;
            // Vanilla's message tags ("Not Secure", "System") draw no bar here.
            self.rect(
                art,
                [0, top, WIDTH + 12, 9],
                TEXT - 2.0,
                black(alpha * BACKGROUND_OPACITY),
            );
            let mut x = 4;
            for span in &line.spans {
                let rgb = span.rgb.map(|c| c as f32 / 255.0);
                let y = (entry_bottom - 8) as f32;
                self.styled(art, span, x as f32, y, TEXT, rgb, alpha);
                x += span
                    .text
                    .chars()
                    .map(|c| advance(art, c, span.bold))
                    .sum::<i32>();
            }
        }
        // The scroll bar, while the chat is open and longer than a page.
        let total = chat.lines.len() as i32;
        let count = shown.len() as i32;
        if open && total > 0 && total != count {
            let page = count * 9;
            let y = chat.scroll as i32 * page / total - bottom;
            let bar = page * page / (total * 9);
            let alpha = if y > 0 { 170 } else { 96 };
            let rgb = if chat.new_since_scroll {
                [0xCC, 0x33, 0x33]
            } else {
                [0x33, 0x33, 0xAA]
            };
            let x = WIDTH + 8;
            self.rect(
                art,
                [x, -y - bar, 2, bar],
                TEXT - 1.0,
                [rgb[0], rgb[1], rgb[2], alpha],
            );
            self.rect(
                art,
                [x + 1, -y - bar, 1, bar],
                TEXT,
                [0xCC, 0xCC, 0xCC, alpha],
            );
        }
        if let Some(field) = &chat.input {
            // `ChatScreen`: the bar, `options.getBackgroundColor(0x80000000)`, and the box,
            // borderless, 4 in from the left, whose text is at its top (`EditBox.textY`):
            // as a bordered box 8 high, 4 further left.
            self.rect(art, [2, height - 14, width - 4, 12], TEXT - 2.0, black(0.5));
            let rect = [0, height - 12, width + 4, 8];
            let rgb = [224.0 / 255.0; 3];
            self.field_text(art, field, rect, chat.cursor_on, true, rgb);
        }
    }

    /// A span with its shadow, faded to `alpha`: bold draws it twice, a pixel apart, and
    /// italic slants it (`BakedGlyph`). At depth `z`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn styled(
        &mut self,
        art: &HudArt,
        span: &Span,
        x: f32,
        y: f32,
        z: f32,
        rgb: [f32; 3],
        alpha: f32,
    ) {
        let shadow = rgb.map(|c| c * 0.25);
        for (dx, rgb, z) in [(1.0, shadow, z), (0.0, rgb, z + 0.5)] {
            let mut pen = x + dx;
            for c in span.text.chars() {
                let Some(&(cell, a)) = art.glyphs.get(&c) else {
                    continue;
                };
                if c != ' ' {
                    let rect = art.glyph_rect(cell);
                    let slant = if span.italic { 1.0 } else { 0.0 };
                    let color = [rgb[0], rgb[1], rgb[2], alpha];
                    for bold in 0..=span.bold as u8 {
                        let (gx, gy) = (pen + bold as f32, y + dx);
                        let [u0, v0, u1, v1] = rect;
                        self.quad(
                            [
                                [gx + slant, gy, z],
                                [gx - slant, gy + 8.0, z],
                                [gx + 8.0 - slant, gy + 8.0, z],
                                [gx + 8.0 + slant, gy, z],
                            ],
                            [[u0, v0], [u0, v1], [u1, v1], [u1, v0]],
                            color,
                        );
                    }
                }
                pen += (a as i32 + span.bold as i32) as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines break at the last space that fits, which goes; a word with no space before it
    /// breaks where it stops fitting; a wrapped line starts with a space; `\n` breaks too.
    /// Widths come from the real font: lowercase letters advance 6, `i` and `l` less.
    #[test]
    fn wraps_like_string_splitter() {
        let res = mc_assets::Resources::new(mc_assets::default_root());
        let art = HudArt {
            items: Default::default(),
            using: Default::default(),
            hotbar: [0.0; 4],
            selection: [0.0; 4],
            font: [0.0; 4],
            font_extra: [0.0; 4],
            glyphs: super::super::glyphs(&res).unwrap(),
            cracks: [[0.0; 4]; 10],
            named: Default::default(),
            white: [0.0; 2],
            lang: Default::default(),
            item_names: Default::default(),
        };
        let plain = |t: &str| Span {
            text: t.into(),
            rgb: [255; 3],
            bold: false,
            italic: false,
        };
        let text = |lines: Vec<Vec<Span>>| -> Vec<String> {
            lines.iter().map(|l| mc_protocol::chat::plain(l)).collect()
        };
        // "aaaa bbbb" is 4 × 6 + 4 + 4 × 6 = 52 wide: at 40 it breaks at the space.
        assert_eq!(
            text(wrap(&art, &[plain("aaaa bbbb")], 40)),
            ["aaaa", " bbbb"]
        );
        // No space: 6 a's fit in 36, the seventh goes over.
        assert_eq!(
            text(wrap(&art, &[plain("aaaaaaaa")], 36)),
            ["aaaaaa", " aa"]
        );
        assert_eq!(text(wrap(&art, &[plain("ab\ncd")], 100)), ["ab", "cd"]);
        // Styles carry over the break; bold is a pixel wider a character.
        let bold = Span {
            bold: true,
            ..plain("aaaa")
        };
        let lines = wrap(&art, &[plain("aa "), bold], 30);
        assert_eq!(text(lines.clone()), ["aa", " aaaa"]);
        assert!(lines[1].iter().all(|s| s.bold));
    }
}
