//! `PRAYER STATES                    7 states` — the sub-screen
//! breadcrumb bar (§15.5, the design's `Crumb`): the path in `textDim`
//! letter-spaced caps on the left, an optional hint right-aligned in
//! the same style.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget as _;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

use crate::ui::theme::Theme;

/// Draw the crumb row: `path` at the left, `hint` (if it fits) at the
/// right edge. Both legs are `textDim` — the crumb is orientation, not
/// content.
pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, path: &str, hint: Option<&str>) {
    if area.width < 4 || area.height < 1 {
        return;
    }
    let path_span = Span::styled(
        truncate_to(path, area.width as usize),
        Style::new().fg(theme.text_dim),
    );
    let mut spans = vec![path_span];
    if let Some(hint) = hint {
        let hint_width = hint.width() + 2; // at least two spaces of separation
        let path_width = path.width().min(area.width as usize);
        if (path_width + hint_width) <= area.width as usize {
            let gap = area.width as usize - path_width - hint.width();
            spans.push(Span::raw(" ".repeat(gap)));
            spans.push(Span::styled(
                hint.to_owned(),
                Style::new().fg(theme.text_dim),
            ));
        }
    }
    Line::from(spans).render(area, buf);
}

/// Truncate to a cell width, marking the cut with `…`.
fn truncate_to(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    for ch in text.chars() {
        if out.width() + ch.width().unwrap_or(0) + 1 > width {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(width: u16, height: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, width, height))
    }

    fn line_at(buf: &Buffer, y: u16) -> String {
        buf.content
            .iter()
            .skip((y * buf.area.width) as usize)
            .take(buf.area.width as usize)
            .map(|cell| cell.symbol().to_owned())
            .collect()
    }

    /// §15.5's shape: path left, hint right, both `textDim`.
    #[test]
    fn the_hint_lands_on_the_right_edge() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = buffer(40, 1);
        render(
            buf.area,
            &mut buf,
            &theme,
            "PRAYER STATES",
            Some("7 states"),
        );
        let line = line_at(&buf, 0);
        assert!(line.starts_with("PRAYER STATES"));
        assert!(line.ends_with("7 states"));
        assert_eq!(
            buf.content[0].style().fg,
            Some(theme.text_dim),
            "the path is textDim"
        );
    }

    /// A hint that does not fit is dropped — the path then owns the
    /// full width; a path wider than the row is truncated with `…`.
    #[test]
    fn a_hint_that_does_not_fit_is_dropped() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = buffer(13, 1);
        render(
            buf.area,
            &mut buf,
            &theme,
            "PRAYER STATES",
            Some("7 states"),
        );
        let line = line_at(&buf, 0);
        assert!(
            line.starts_with("PRAYER STATES"),
            "the path fits exactly: {line}"
        );
        assert!(!line.contains("7 states"), "the hint is dropped: {line}");

        let mut buf = buffer(10, 1);
        render(buf.area, &mut buf, &theme, "PRAYER STATES", None);
        // Nine letters plus the ellipsis: exactly the ten cells wide.
        assert_eq!(line_at(&buf, 0), "PRAYER ST…");
    }
}
