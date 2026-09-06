//! `╭─ TITLE ────╮` box, `active` → `borderHi` (§15.5).
//!
//! The design draws the border on the terminal ground and the panel's
//! own ground only inside it, so [`Panel::render`] paints the interior
//! separately instead of styling the whole `Block` — the border cells
//! keep whatever ground the caller laid down.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType};

use crate::ui::theme::Theme;

/// A titled rounded box. Render returns the inner `Rect` so the caller
/// fills the content in the same pass.
pub struct Panel<'a> {
    title: Option<&'a str>,
    active: bool,
    border: Option<Color>,
    background: Option<Color>,
}

impl Default for Panel<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> Panel<'a> {
    /// An untitled, inactive panel.
    pub fn new() -> Self {
        Self {
            title: None,
            active: false,
            border: None,
            background: None,
        }
    }

    /// The `╭─ TITLE ─` inset title.
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// The active variant: `borderHi` instead of `border`.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// A border colour override (the Adhan overlay's red, the
    /// prayer-config panel's green).
    pub fn border(mut self, color: Color) -> Self {
        self.border = Some(color);
        self
    }

    /// An interior ground override (the Adhan overlay's `redBg` wash).
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Draw the box and return its interior.
    pub fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) -> Rect {
        let border_color = if self.active {
            theme.border_hi
        } else {
            self.border.unwrap_or(theme.border)
        };
        let mut block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border_color));
        if let Some(title) = self.title {
            // `╭─ TITLE ` — the dashes inherit the border colour, the
            // title is textSub (the design's `titleCol` default).
            block = block.title_top(
                Line::from(vec![
                    Span::raw("─ "),
                    Span::styled(title, Style::new().fg(theme.text_sub)),
                    Span::raw(" "),
                ])
                .style(Style::new().fg(border_color)),
            );
        }
        let inner = block.inner(area);
        block.render(area, buf);
        // The interior ground, inside the border only.
        buf.set_style(
            inner,
            Style::new().bg(self.background.unwrap_or(theme.panel_bg)),
        );
        inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(width: u16, height: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, width, height))
    }

    /// The §14.6 specimen's top border, exactly: `╭─ NEXT PRAYER `
    /// then dashes to the corner, 80 columns total, interior one row
    /// shorter on each side.
    #[test]
    fn an_active_titled_panel_matches_the_specimen_geometry() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = buffer(80, 7);
        let inner = Panel::new().title("NEXT PRAYER").active(true).render(
            Rect::new(0, 0, 80, 7),
            &mut buf,
            &theme,
        );
        let row = |y: u16| -> String {
            (0..80)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        };
        let top = row(0);
        assert!(top.starts_with("╭─ NEXT PRAYER "), "top border was {top:?}");
        assert_eq!(top.chars().count(), 80);
        assert!(top.ends_with('╮'));
        assert!(top.contains(&"─".repeat(64)));
        for y in 1..6 {
            assert!(row(y).starts_with('│') && row(y).ends_with('│'), "row {y}");
        }
        let bottom = row(6);
        assert!(bottom.starts_with('╰') && bottom.ends_with('╯'));
        assert_eq!(inner, Rect::new(1, 1, 78, 5));
    }

    /// An inactive panel uses `border`, an active one `borderHi` —
    /// the one styling decision the widget owns.
    #[test]
    fn active_selects_the_highlight_border() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = buffer(10, 3);
        Panel::new()
            .title("X")
            .render(Rect::new(0, 0, 10, 3), &mut buf, &theme);
        assert_eq!(buf[(0, 0)].style().fg, Some(theme.border));
        let mut buf = buffer(10, 3);
        Panel::new()
            .title("X")
            .active(true)
            .render(Rect::new(0, 0, 10, 3), &mut buf, &theme);
        assert_eq!(buf[(0, 0)].style().fg, Some(theme.border_hi));
    }

    /// The interior ground is `panelBg` by default, the override wins,
    /// and the border cells do not carry the panel ground.
    #[test]
    fn the_ground_is_painted_inside_only() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = buffer(10, 4);
        Panel::new().render(Rect::new(0, 0, 10, 4), &mut buf, &theme);
        assert_eq!(buf[(1, 1)].style().bg, Some(theme.panel_bg));
        assert_ne!(buf[(0, 0)].style().bg, Some(theme.panel_bg));
        assert_ne!(buf[(9, 3)].style().bg, Some(theme.panel_bg));
        let mut buf = buffer(10, 4);
        Panel::new()
            .background(theme.red_bg)
            .render(Rect::new(0, 0, 10, 4), &mut buf, &theme);
        assert_eq!(buf[(1, 1)].style().bg, Some(theme.red_bg));
    }
}
