//! `████░░░░` gauge with a width parameter (§15.5) — a `Paragraph` of
//! `█`/`░` spans, not a `Gauge`, because the design has no percentage
//! label for `Gauge` to center.
//!
//! The filled count is the design's `Math.round(pct × width)` with
//! `pct` clamped to `0..=1`, so the §14.6 specimen's 42.35 % of 34
//! cells is 14 `█` and 20 `░`, not 13.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};

use crate::ui::theme::Theme;

/// Draw the gauge into `area`'s first row: `filled` cells of `█` in
/// `color` (green by default), the rest `░` in `borderDim`.
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    progress: f64,
    width: u16,
    color: Option<Color>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = width.min(area.width) as usize;
    let filled = (progress.clamp(0.0, 1.0) * width as f64).round() as usize;
    buf.set_stringn(
        area.x,
        area.y,
        "█".repeat(filled),
        filled,
        Style::new().fg(color.unwrap_or(theme.green)),
    );
    buf.set_stringn(
        area.x + filled as u16,
        area.y,
        "░".repeat(width - filled),
        width - filled,
        Style::new().fg(theme.border_dim),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gauge_text(buf: &Buffer, width: usize) -> String {
        (0..width)
            .map(|x| buf[(x as u16, 0)].symbol().to_string())
            .collect()
    }

    /// §14.6's gauge: 4523 of 10680 seconds elapsed over 34 cells is
    /// exactly 14 `█` and 20 `░`.
    #[test]
    fn the_specimens_gauge_is_14_and_20() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 1));
        render(
            Rect::new(0, 0, 80, 1),
            &mut buf,
            &theme,
            4523.0 / 10680.0,
            34,
            None,
        );
        assert_eq!(
            gauge_text(&buf, 34),
            format!("{}{}", "█".repeat(14), "░".repeat(20))
        );
        assert_eq!(buf[(0, 0)].style().fg, Some(theme.green));
        assert_eq!(buf[(13, 0)].style().fg, Some(theme.green));
        assert_eq!(buf[(14, 0)].style().fg, Some(theme.border_dim));
    }

    /// Clamping and the extremes: negative and > 1 saturate, and a
    /// full gauge has no `░` run.
    #[test]
    fn the_gauge_clamps() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        for (progress, expected) in [(-1.0, 0usize), (0.0, 0), (1.0, 34), (2.0, 34)] {
            let mut buf = Buffer::empty(Rect::new(0, 0, 80, 1));
            render(Rect::new(0, 0, 80, 1), &mut buf, &theme, progress, 34, None);
            let filled = gauge_text(&buf, 34).chars().filter(|c| *c == '█').count();
            assert_eq!(filled, expected, "progress {progress}");
        }
    }

    /// §16.14 item 10: the gauge is fixed at 34 cells even when the
    /// terminal is wider — extra width stays empty.
    #[test]
    fn the_gauge_leaves_extra_width_empty() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 1));
        render(Rect::new(0, 0, 80, 1), &mut buf, &theme, 0.5, 34, None);
        assert_eq!(buf[(34, 0)].symbol(), " ");
        assert_eq!(buf[(79, 0)].symbol(), " ");
    }
}
