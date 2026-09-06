//! Full-width `─` rule (§15.5): one `Line` repeated to the area's
//! width, `borderDim` by default — the design's `HR`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::ui::theme::Theme;

/// Draw the rule across `area`'s first row, truncated to the width.
pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, color: Option<Color>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rule = "─".repeat(area.width as usize);
    buf.set_stringn(
        area.x,
        area.y,
        rule,
        area.width as usize,
        Style::new().fg(color.unwrap_or(theme.border_dim)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §14.6 TODAY rule spans the full width in `borderDim`.
    #[test]
    fn the_rule_spans_the_width_in_border_dim() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 1));
        render(Rect::new(0, 0, 80, 1), &mut buf, &theme, None);
        let line: String = (0..80).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert_eq!(line, "─".repeat(80));
        assert_eq!(buf[(0, 0)].style().fg, Some(theme.border_dim));
    }

    /// A zero-width area draws nothing and does not panic.
    #[test]
    fn a_zero_area_is_a_no_op() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 0, 0));
        render(Rect::new(0, 0, 0, 0), &mut buf, &theme, None);
    }
}
