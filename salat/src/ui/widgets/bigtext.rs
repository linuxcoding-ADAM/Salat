//! 3-row block digits for the countdown (A1) — §15.5's pinned
//! geometry: 3-column glyphs, 3 terminal rows, a 1-column gap, 4
//! columns of advance per character, drawn only from `█ ▀ ▄` and
//! space. `width(s) = 4n − 1`.
//!
//! Two invariants the tests pin: an unmapped character renders the
//! space glyph and emits one `warn!` — `bigtext` never panics and
//! never returns an error, because a malformed countdown must not take
//! down the frame; and when `4n − 1 > available_width` the block does
//! not render at all (the caller falls back to the single-row bold
//! form) — it never truncates, never wraps, and never drops the gap.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

/// The 38-glyph table, §15.5's storage verbatim: `0`–`9`, `:`, `A`–`Z`,
/// space — one `[&str; 3]` per glyph, three 3-character rows.
pub const GLYPHS: [[&str; 3]; 38] = [
    // 0
    ["███", "█ █", "███"],
    // 1
    [" █ ", " █ ", "▄█▄"],
    // 2
    ["▀▀█", "▄▄█", "█▄▄"],
    // 3
    ["▀▀█", " ▀▀", "▄▄█"],
    // 4
    ["█ █", "█▀█", "  █"],
    // 5
    ["█▀▀", "▀▀▄", "▄▄█"],
    // 6
    ["█▀▀", "█▀▄", "█▄▄"],
    // 7
    ["▀▀▀", "  █", "  █"],
    // 8
    ["█▀█", "█▀█", "█▄█"],
    // 9
    ["█▀█", "█▀█", "▄▄▀"],
    // :
    [" ▄ ", "   ", " ▀ "],
    // A
    ["█▀█", "█▀█", "█ █"],
    // B
    ["█▀▀", "█▀▄", "█▄█"],
    // C
    ["█▀▀", "█  ", "█▄▄"],
    // D
    ["█▀▄", "█ █", "█▄▀"],
    // E
    ["█▀▀", "█▀▀", "█▄▄"],
    // F
    ["█▀▀", "█▀▀", "█  "],
    // G
    ["█▀▀", "█ ▀", "█▄▄"],
    // H
    ["█ █", "█▀█", "█ █"],
    // I
    ["▀▀▀", " █ ", "▄▄▄"],
    // J
    ["▀▀█", "  █", "▄▄█"],
    // K
    ["█ █", "█▀▄", "█ ▀"],
    // L
    ["█  ", "█  ", "█▄▄"],
    // M
    ["█▄█", "█▀█", "█ █"],
    // N
    ["█▄█", "█▀▄", "█ ▀"],
    // O
    ["█▀█", "█ █", "█▄█"],
    // P
    ["█▀█", "█▀▀", "█  "],
    // Q
    ["█▀█", "█ █", "█▄▀"],
    // R
    ["█▀█", "█▀▄", "█ ▀"],
    // S
    ["█▀▀", "▀▀▄", "▄▄█"],
    // T
    ["▀▀▀", " █ ", " █ "],
    // U
    ["█ █", "█ █", "█▄█"],
    // V
    ["█ █", "█ █", "▀ ▀"],
    // W
    ["█ █", "█▀█", "█▄█"],
    // X
    ["█ █", "▄▀▄", "█ █"],
    // Y
    ["█ █", " █ ", " █ "],
    // Z
    ["▀▀█", "▄▀ ", "█▄▄"],
    // space
    ["   ", "   ", "   "],
];

/// The space glyph's index — the fallback for unmapped characters.
const SPACE: usize = 37;

fn glyph_index(ch: char) -> Option<usize> {
    match ch {
        '0'..='9' => Some(ch as usize - '0' as usize),
        ':' => Some(10),
        'A'..='Z' => Some(11 + ch as usize - 'A' as usize),
        ' ' => Some(SPACE),
        _ => None,
    }
}

/// The block width of `text`: `4n − 1` for `n` characters (§15.5) —
/// every glyph is 3 columns and every gap but the last is 1.
pub fn width(text: &str) -> u16 {
    let n = text.chars().count() as u64;
    if n == 0 {
        0
    } else {
        u16::try_from(4 * n - 1).unwrap_or(u16::MAX)
    }
}

/// Draw the block into `area`'s top-left in one colour. Returns
/// `false` when it does not fit (`4n − 1 > area.width` or fewer than
/// 3 rows) and draws nothing — the caller's fallback is the single-row
/// bold form, never a half-drawn glyph.
pub fn render(area: Rect, buf: &mut Buffer, text: &str, color: Color) -> bool {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return true;
    }
    if width(text) > area.width || area.height < 3 {
        return false;
    }
    let mut x = area.x;
    for &ch in &chars {
        let glyph = match glyph_index(ch) {
            Some(index) => &GLYPHS[index],
            None => {
                tracing::warn!(
                    target: "salat::ui",
                    "bigtext: unmapped character {ch:?} rendered as space"
                );
                &GLYPHS[SPACE]
            }
        };
        for (row, cells) in glyph.iter().enumerate() {
            for (col, cell) in cells.chars().enumerate() {
                let target = &mut buf[(x + col as u16, area.y + row as u16)];
                target.set_char(cell);
                // Spaces carry no colour — they are the gap, not ink.
                if cell != ' ' {
                    target.set_style(Style::new().fg(color));
                }
            }
        }
        x += 4;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §15.5's five pinned glyphs, exactly as the section prints them.
    #[test]
    fn the_five_pinned_glyphs_are_verbatim() {
        assert_eq!(GLYPHS[0], ["███", "█ █", "███"]);
        assert_eq!(GLYPHS[1], [" █ ", " █ ", "▄█▄"]);
        assert_eq!(GLYPHS[2], ["▀▀█", "▄▄█", "█▄▄"]);
        assert_eq!(GLYPHS[10], [" ▄ ", "   ", " ▀ "]);
        assert_eq!(GLYPHS[23], ["█▄█", "█▀█", "█ █"]);
    }

    /// Every glyph is three rows of exactly three characters drawn
    /// from the four-character alphabet — the integrity invariant the
    /// geometry rests on.
    #[test]
    fn every_glyph_is_3x3_from_the_alphabet() {
        for (index, glyph) in GLYPHS.iter().enumerate() {
            assert_eq!(glyph.len(), 3, "glyph {index}");
            for row in glyph {
                assert_eq!(row.chars().count(), 3, "glyph {index} row {row:?}");
                for cell in row.chars() {
                    assert!(
                        matches!(cell, '█' | '▀' | '▄' | ' '),
                        "glyph {index} row {row:?} cell {cell:?}"
                    );
                }
            }
        }
    }

    /// The mapping covers exactly the 38-character set.
    #[test]
    fn the_charset_maps_and_nothing_else() {
        let charset: String = "0123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZ ".into();
        assert_eq!(charset.chars().count(), 38);
        for ch in charset.chars() {
            assert!(glyph_index(ch).is_some(), "{ch:?}");
        }
        for ch in ['!', 'a', '·', 'ٔ', '\t'] {
            assert!(glyph_index(ch).is_none(), "{ch:?}");
        }
    }

    /// `width(s) = 4n − 1` over the whole charset, single characters
    /// and the two widths the document quotes: `01:42:37` → 31,
    /// `MAGHRIB` → 27.
    #[test]
    fn the_width_formula_holds() {
        assert_eq!(width("0"), 3);
        assert_eq!(width("01"), 7);
        assert_eq!(width("01:42:37"), 31);
        assert_eq!(width("MAGHRIB"), 27);
        let charset: String = "0123456789:ABCDEFGHIJKLMNOPQRSTUVWXYZ ".into();
        assert_eq!(width(&charset), 4 * 38 - 1);
        assert_eq!(width(""), 0);
    }

    /// Too narrow (or too short) renders nothing and returns `false`;
    /// exactly wide enough renders and returns `true`.
    #[test]
    fn too_narrow_renders_nothing() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 3));
        assert!(!render(
            Rect::new(0, 0, 30, 3),
            &mut buf,
            "01:42:37",
            Color::Green
        ));
        assert_eq!(buf[(0, 0)].symbol(), " ");
        let mut buf = Buffer::empty(Rect::new(0, 0, 31, 3));
        assert!(render(
            Rect::new(0, 0, 31, 3),
            &mut buf,
            "01:42:37",
            Color::Green
        ));
        assert_eq!(buf[(0, 0)].symbol(), "█");

        let mut buf = Buffer::empty(Rect::new(0, 0, 31, 2));
        assert!(!render(
            Rect::new(0, 0, 31, 2),
            &mut buf,
            "01:42:37",
            Color::Green
        ));
    }

    /// The whole "MAGHRIB" block: 27 columns of ink, one style, the
    /// gap columns blank, nothing past column 27.
    #[test]
    fn a_full_block_advances_four_columns_per_glyph() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 3));
        assert!(render(
            Rect::new(0, 0, 80, 3),
            &mut buf,
            "MAGHRIB",
            Color::White
        ));
        let ink = |x: u16, y: u16| buf[(x, y)].symbol() != " ";
        // The M at x 0..3, its gap at 3, the A at 4..7 …
        assert!(ink(0, 0) && ink(1, 0) && ink(2, 0));
        assert!(!ink(3, 0));
        assert!(ink(4, 0));
        // … the final B ends at 26, 27 columns of block, nothing past.
        assert!(ink(26, 0));
        assert!(!ink(27, 0) && !ink(27, 1) && !ink(27, 2));
        // All three rows are inked, one style.
        for y in 0..3u16 {
            assert!(ink(0, y), "row {y}");
        }
        assert_eq!(buf[(0, 0)].style().fg, Some(Color::White));
        assert_eq!(buf[(26, 2)].style().fg, Some(Color::White));
    }

    /// An unmapped character renders the space glyph — identical ink
    /// to a real space, no panic, no error.
    #[test]
    fn an_unmapped_character_renders_as_space() {
        let draw = |text: &str| {
            let mut buf = Buffer::empty(Rect::new(0, 0, 80, 3));
            assert!(render(Rect::new(0, 0, 80, 3), &mut buf, text, Color::Green));
            (0..80u16)
                .flat_map(|x| (0..3u16).map(move |y| (x, y)))
                .map(|(x, y)| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        assert_eq!(draw("4!2"), draw("4 2"));
    }
}
