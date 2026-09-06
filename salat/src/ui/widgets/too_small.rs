//! The §16.13 guard: when the terminal is below 80×24, no screen
//! renders — just this centered ASCII panel. Plain `+--|` drawing and
//! default colours only, so it is legible on any terminal of any size,
//! down to a 20×5 window (the lines are clipped, never wrapped).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// The §16.13 floor.
pub const MIN_WIDTH: u16 = 80;
pub const MIN_HEIGHT: u16 = 24;

/// Whether the guard owns the frame.
pub fn is_too_small(width: u16, height: u16) -> bool {
    width < MIN_WIDTH || height < MIN_HEIGHT
}

/// Draw the guard over the whole frame area. The three text lines come
/// from the section verbatim; the box is pure ASCII so it renders even
/// where the line-drawing glyphs may not.
pub fn render(area: Rect, buf: &mut Buffer) {
    let lines = [
        "TERMINAL TOO SMALL",
        &format!(
            "salat needs 80×24 · this terminal is {}×{}",
            area.width, area.height
        ),
        "resize to continue",
    ];
    // The box hugs the widest line plus one cell of padding each side —
    // measured in cells, not bytes (`×` and `·` are one cell each but
    // multi-byte in UTF-8).
    let inner_width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
    let box_width = (inner_width + 4).min(area.width);
    let box_height = 5.min(area.height);

    let x = area.x + (area.width.saturating_sub(box_width)) / 2;
    let y = area.y + (area.height.saturating_sub(box_height)) / 2;
    let box_area = Rect::new(x, y, box_width, box_height);

    if box_height >= 5 {
        let top: String = format!("+{}+", "-".repeat(box_width.saturating_sub(2) as usize));
        let bottom = top.clone();
        put_line(buf, box_area.x, box_area.y, box_area.width, &top);
        put_line(buf, box_area.x, box_area.y + 4, box_area.width, &bottom);
        for row in 1..4 {
            put_line(buf, box_area.x, box_area.y + row, box_area.width, "|");
            put_right(buf, box_area.x, box_area.y + row, box_area.width, "|");
        }
    }
    // The text rows, centered inside the box — or, when the area is too
    // short for the box, centered in what there is.
    let text_y = if box_height >= 5 {
        box_area.y + 1
    } else {
        area.y + (area.height.saturating_sub(lines.len() as u16)) / 2
    };
    let (center_x, center_width) = if box_height >= 5 {
        (box_area.x, box_width)
    } else {
        (area.x, area.width)
    };
    let avail = area.right().saturating_sub(center_x);
    for (index, line) in lines.iter().enumerate() {
        let row = text_y + index as u16;
        if row >= area.bottom() {
            break;
        }
        put_centered(buf, center_x, row, avail, center_width, line);
    }
}

/// Write `text` at (x, y), clipped to `max_width` cells.
fn put_line(buf: &mut Buffer, x: u16, y: u16, max_width: u16, text: &str) {
    for (index, symbol) in text.chars().enumerate() {
        if index as u16 >= max_width {
            break;
        }
        if let Some(cell) = buf.cell_mut((x + index as u16, y)) {
            cell.set_symbol(&symbol.to_string());
        }
    }
}

/// Write `text` right-aligned at (x, y), clipped.
fn put_right(buf: &mut Buffer, x: u16, y: u16, width: u16, text: &str) {
    let text_len = text.chars().count() as u16;
    let start = width.saturating_sub(text_len);
    put_line(buf, x + start, y, width - start, text);
}

/// Write `text` centered within `width` cells starting at `x`.
fn put_centered(buf: &mut Buffer, x: u16, y: u16, avail: u16, width: u16, text: &str) {
    let inner = width.saturating_sub(2);
    let text_len = text.chars().count() as u16;
    let pad = inner.saturating_sub(text_len.min(inner)) / 2;
    let start = (width.saturating_sub(inner)) / 2 + pad;
    let clip = (avail.saturating_sub(start)).min(inner);
    put_line(buf, x + start, y, clip, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered(width: u16, height: u16) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        render(buf.area, &mut buf);
        buf
    }

    fn row(buf: &Buffer, y: u16) -> String {
        buf.content
            .iter()
            .skip((y * buf.area.width) as usize)
            .take(buf.area.width as usize)
            .map(|cell| cell.symbol().to_owned())
            .collect()
    }

    /// §16.13's own example: a 79×23 terminal shows the guard, title
    /// centered on the first text row.
    #[test]
    fn the_title_is_centered_at_79x23() {
        let buf = rendered(79, 23);
        let box_top = row(&buf, 9); // (23 − 5) / 2 = 9
        let title_row = row(&buf, 10);
        assert!(title_row.contains("TERMINAL TOO SMALL"), "{title_row}");
        assert!(box_top.contains("+-"), "the box top is ASCII: {box_top}");
    }

    /// The section's floor check, verbatim.
    #[test]
    fn the_floor_is_below_80x24() {
        assert!(is_too_small(79, 24));
        assert!(is_too_small(80, 23));
        assert!(!is_too_small(80, 24));
    }

    /// A 20×5 terminal still renders clipped text and never panics.
    #[test]
    fn a_20x5_terminal_renders_without_panicking() {
        let buf = rendered(20, 5);
        let joined = (0..5).map(|y| row(&buf, y)).collect::<Vec<_>>().join("\n");
        assert!(!joined.trim().is_empty(), "something rendered");
    }

    /// The measured size is reported in the message — at a width the
    /// message fits in.
    #[test]
    fn the_measured_size_is_reported() {
        let buf = rendered(60, 10);
        let joined = (0..10).map(|y| row(&buf, y)).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("60×10"), "{joined}");
    }

    /// At a width the size line cannot fit, it is clipped — never
    /// wrapped, never spilling outside the box (§16.13).
    #[test]
    fn a_too_narrow_width_clips_the_size_line() {
        let buf = rendered(40, 10);
        let joined = (0..10).map(|y| row(&buf, y)).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("salat needs 80×24"), "{joined}");
        assert!(
            !joined.contains("40×10"),
            "the line is clipped, not wrapped"
        );
    }
}
