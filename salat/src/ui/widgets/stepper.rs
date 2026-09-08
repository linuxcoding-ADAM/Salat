//! `Stepper` — `◂ 5 min ▸`, the settings value control (§16.3's
//! `StepCtrl`, §16.5's `NumCell`).
//!
//! One shape, two uses: the Prayer Configuration panel's before/after
//! minutes (`◂ 5 min ▸`) and the Settings screen's cycleable values
//! (`◂ Abdul Basit ▸`, `◂ 80% ▸`). The arrows dim at the clamp bounds
//! so the affordance tells the truth — a `◂` you cannot press is not
//! drawn as one you can.
//!
//! The number it displays is the caller's business: the widget takes
//! the already-formatted value (`5 min`, `80%`, a preset name) and the
//! clamp state, never the raw config integer, so the ±1 and ±5 steps
//! and the `0..=60` clamp live with the config-editing code that owns
//! them (§16.3 item 7, §16.5 item 7).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget as _;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::theme::Theme;

/// A `◂ value ▸` control.
#[derive(Debug, Default)]
pub struct Stepper<'a> {
    value: &'a str,
    focused: bool,
    at_min: bool,
    at_max: bool,
}

impl<'a> Stepper<'a> {
    /// The control showing `value`.
    pub fn new(value: &'a str) -> Self {
        Self {
            value,
            ..Self::default()
        }
    }

    /// The focused cell: `rowSel` ground across the cell.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// The value is at the lower clamp — `◂` dims.
    pub fn at_min(mut self, at_min: bool) -> Self {
        self.at_min = at_min;
        self
    }

    /// The value is at the upper clamp — `▸` dims.
    pub fn at_max(mut self, at_max: bool) -> Self {
        self.at_max = at_max;
        self
    }

    /// Draw the control, centred in `area`.
    pub fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width < 5 || area.height < 1 {
            return;
        }
        let ground = if self.focused {
            Style::new().bg(theme.row_sel)
        } else {
            Style::new()
        };
        let line = Line::from(vec![
            arrow_span("◂", self.at_min, theme, ground),
            Span::raw(" "),
            Span::styled(self.value, Style::new().fg(theme.text_bright).patch(ground)),
            Span::raw(" "),
            arrow_span("▸", self.at_max, theme, ground),
        ]);
        let width = u16::try_from(line.width()).unwrap_or(area.width);
        let x = area.x + area.width.saturating_sub(width) / 2;
        // The focused ground covers the whole cell, not just the
        // glyphs — the cell is what reads as selected in the grid.
        buf.set_style(area, ground);
        line.render(Rect::new(x, area.y, width.min(area.width), 1), buf);
    }
}

/// One arrow span — `text` when the step it offers is available,
/// `textDim` at its clamp.
fn arrow_span<'a>(glyph: &'a str, clamped: bool, theme: &Theme, ground: Style) -> Span<'a> {
    let color = if clamped { theme.text_dim } else { theme.text };
    Span::styled(glyph, Style::new().fg(color).patch(ground))
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

    /// §16.3's shape: `◂ 5 min ▸`, centred in its cell.
    #[test]
    fn the_control_is_arrows_around_the_value() {
        let mut buf = buffer(11, 1);
        Stepper::new("5 min").render(buf.area, &mut buf, &test_theme());
        let line = line_at(&buf, 0);
        assert!(line.contains("◂ 5 min ▸"), "line was {line:?}");
        // Centred: nine glyphs in an eleven-cell row, one blank each side.
        assert_eq!(line, " ◂ 5 min ▸ ");
    }

    /// The focused cell carries `rowSel` across its full width.
    #[test]
    fn the_focused_cell_carries_the_selection_ground() {
        let theme = test_theme();
        let mut buf = buffer(11, 1);
        Stepper::new("5 min")
            .focused(true)
            .render(buf.area, &mut buf, &theme);
        for cell in buf.content.iter() {
            assert_eq!(cell.style().bg, Some(theme.row_sel));
        }
    }

    /// The arrows dim at their clamp bounds — the affordance never
    /// offers a step that cannot be taken.
    #[test]
    fn the_arrows_dim_at_the_clamps() {
        let theme = test_theme();
        // `◂ 0 min ▸` is nine cells, so the control starts at x=1.
        let mut buf = buffer(11, 1);
        Stepper::new("0 min")
            .at_min(true)
            .render(buf.area, &mut buf, &theme);
        assert_eq!(buf.content[1].style().fg, Some(theme.text_dim));
        assert_eq!(buf.content[9].style().fg, Some(theme.text));

        // `◂ 60 min ▸` is ten cells, so the control starts at x=0.
        let mut buf = buffer(11, 1);
        Stepper::new("60 min")
            .at_max(true)
            .render(buf.area, &mut buf, &theme);
        assert_eq!(buf.content[9].style().fg, Some(theme.text_dim));
        assert_eq!(buf.content[0].style().fg, Some(theme.text));
    }

    /// A preset name steps too (§16.2 item 7) — the shape is not
    /// numeric.
    #[test]
    fn a_preset_name_renders_the_same_shape() {
        let mut buf = buffer(20, 1);
        Stepper::new("Abdul Basit").render(buf.area, &mut buf, &test_theme());
        let line = line_at(&buf, 0);
        assert!(line.contains("◂ Abdul Basit ▸"), "line was {line:?}");
    }

    fn test_theme() -> Theme {
        Theme::for_mode(crate::ui::theme::RenderMode::Truecolor)
    }
}
