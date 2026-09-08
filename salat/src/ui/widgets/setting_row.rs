//! `SettingRow` — one settings row (§16.2, §16.4's setting_row ×15).
//!
//! The design's row is `label … value` with a sub-label beneath and,
//! on the focused row, a 2px `borderHi` left edge, the `› ` prefix
//! and a `rowActive` ground. A terminal has no 2px edge, so the
//! half-block `▌` carries it; actionable rows append `↵` after the
//! value — the design's action affordance, in one glyph.
//!
//! The value's colour is semantic, not chosen by the caller's mood:
//! `ENABLED` green, `DISABLED` red, `[ PLAY ]` cyan, `CONFIGURED`
//! blue, everything else `textSub` — the design's exact value
//! vocabulary, expressed as one enum so a screen cannot invent a
//! sixth kind.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget as _;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

use crate::ui::theme::Theme;

/// What a row's value means — decides its colour (the design's value
/// classes, verbatim).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ValueKind {
    /// `ENABLED` — `green`.
    Enabled,
    /// `DISABLED` — `red`.
    Disabled,
    /// `[ PLAY ]` — `cyan`, an action rather than a state.
    Action,
    /// `CONFIGURED` — `blue`.
    Configured,
    /// Everything else — `textSub`, the default.
    #[default]
    Plain,
}

impl ValueKind {
    /// The theme colour this kind renders in.
    fn color(self, theme: &Theme) -> ratatui::style::Color {
        match self {
            Self::Enabled => theme.green,
            Self::Disabled => theme.red,
            Self::Action => theme.cyan,
            Self::Configured => theme.blue,
            Self::Plain => theme.text_sub,
        }
    }
}

/// One row of a settings panel. Height is 1 or 2 — call
/// [`SettingRow::height`] and lay the next row below it.
#[derive(Debug, Default)]
pub struct SettingRow<'a> {
    label: &'a str,
    value: &'a str,
    kind: ValueKind,
    sub: Option<&'a str>,
    action: bool,
    active: bool,
    /// Suppress the sub-label (§16.2 item 9: small hides sub-labels,
    /// which is what makes 15 rows fit a 24-row terminal).
    compact: bool,
    /// The row's subsystem is unavailable (§16.2 item 8) — the value
    /// dims and the reason rides the sub-label.
    unavailable: bool,
}

impl<'a> SettingRow<'a> {
    /// `label … value` in `textSub`.
    pub fn new(label: &'a str, value: &'a str) -> Self {
        Self {
            label,
            value,
            ..Self::default()
        }
    }

    /// The value's semantic colour class.
    pub fn kind(mut self, kind: ValueKind) -> Self {
        self.kind = kind;
        self
    }

    /// The sub-label under the label, `textSub`.
    pub fn subtitle(mut self, sub: &'a str) -> Self {
        self.sub = Some(sub);
        self
    }

    /// Mark the row actionable — appends `↵`.
    pub fn action(mut self, action: bool) -> Self {
        self.action = action;
        self
    }

    /// The focused row: `▌` edge, `› ` prefix, `rowActive` ground.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Hide the sub-label (small breakpoints).
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// The row's subsystem is missing — value dims, reason shows.
    pub fn unavailable(mut self, unavailable: bool) -> Self {
        self.unavailable = unavailable;
        self
    }

    /// The rows this widget occupies: 2 with a visible sub-label, else 1.
    pub fn height(&self) -> u16 {
        if self.sub.is_some() && !self.compact {
            2
        } else {
            1
        }
    }

    /// Draw the row (and its sub-label) at `area`.
    pub fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme) {
        if area.width < 5 || area.height < 1 {
            return;
        }
        let value_color = if self.unavailable {
            theme.text_dim
        } else {
            self.kind.color(theme)
        };
        let ground = if self.active {
            Style::new().bg(theme.row_active)
        } else {
            Style::new()
        };

        // `▌› ` on the focused row, three blanks otherwise — the
        // label column stays aligned either way.
        let edge = if self.active {
            Span::styled("▌", Style::new().fg(theme.border_hi).patch(ground))
        } else {
            Span::raw(" ")
        };
        let prefix = if self.active { "› " } else { "  " };

        // The value right-aligns; `↵` rides after it when the row
        // opens a sub-screen.
        let marker = if self.action { " ↵" } else { "" };
        let label_color = if self.active {
            theme.text_bright
        } else {
            theme.text
        };

        let mut line = vec![edge, Span::styled(prefix, ground)];
        let used = 3; // edge + 2-char prefix
        let marker_width = marker.width();
        let label_budget =
            (area.width as usize).saturating_sub(used + self.value.width() + marker_width + 1);
        let label = truncate_to(self.label, label_budget);
        line.push(Span::styled(
            label.clone(),
            Style::new().fg(label_color).patch(ground),
        ));
        let pad = area.width as usize - used - label.width() - self.value.width() - marker_width;
        line.push(Span::styled(" ".repeat(pad), ground));
        line.push(Span::styled(
            self.value,
            Style::new().fg(value_color).patch(ground),
        ));
        if self.action {
            line.push(Span::styled(
                marker,
                Style::new().fg(theme.text_sub).patch(ground),
            ));
        }
        Line::from(line).render(Rect::new(area.x, area.y, area.width, 1), buf);

        // The sub-label, indented under the label and hidden when
        // compact (§16.2 item 9). An unavailable row's reason rides
        // the sub-label — at small the reason hides with it, per the
        // spec; the dimmed value alone carries the unavailability.
        if self.sub.is_some() && !self.compact && area.height >= 2 {
            let sub = self.sub.unwrap_or_default();
            let mut sub_line = vec![Span::raw("   "), Span::raw(" ")];
            sub_line.push(Span::styled(
                truncate_to(sub, area.width as usize - 5),
                Style::new().fg(theme.text_sub),
            ));
            Line::from(sub_line).render(Rect::new(area.x, area.y + 1, area.width, 1), buf);
        }
    }
}

/// Truncate to a cell width, marking the cut with `…` — the crumb's
/// rule, applied to labels that outgrow their budget.
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

    fn render_into(buf: &mut Buffer, row: &SettingRow<'_>) {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        row.render(buf.area, buf, &theme);
    }

    /// The plain row: `  Time format … 24h`, value in `textSub`.
    #[test]
    fn a_plain_row_is_label_and_value() {
        let mut buf = buffer(40, 1);
        render_into(&mut buf, &SettingRow::new("Time format", "24h"));
        let line = line_at(&buf, 0);
        assert!(line.starts_with("   Time format"), "row was {line:?}");
        assert!(line.ends_with("24h"), "row was {line:?}");
        assert_eq!(
            buf.content.last().map(|cell| cell.style().fg),
            Some(Some(
                Theme::for_mode(crate::ui::theme::RenderMode::Truecolor).text_sub
            )),
        );
    }

    /// The active row: `▌› ` prefix, `rowActive` ground, bright label,
    /// and the `↵` action marker after the value.
    #[test]
    fn the_active_row_carries_edge_prefix_ground_and_marker() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = buffer(40, 1);
        render_into(
            &mut buf,
            &SettingRow::new("Location", "Sétif, Algeria")
                .action(true)
                .active(true),
        );
        let line = line_at(&buf, 0);
        assert!(line.starts_with("▌› Location"), "row was {line:?}");
        assert!(line.ends_with("Sétif, Algeria ↵"), "row was {line:?}");
        assert_eq!(buf.content[0].symbol(), "▌");
        assert_eq!(buf.content[0].style().fg, Some(theme.border_hi));
        assert_eq!(buf.content[4].style().bg, Some(theme.row_active));
    }

    /// The value vocabulary colours (the design's four classes plus
    /// the default), and the unavailable dim.
    #[test]
    fn the_value_kinds_carry_their_colours() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        for (kind, expected) in [
            (ValueKind::Enabled, theme.green),
            (ValueKind::Disabled, theme.red),
            (ValueKind::Action, theme.cyan),
            (ValueKind::Configured, theme.blue),
            (ValueKind::Plain, theme.text_sub),
        ] {
            let mut buf = buffer(40, 1);
            render_into(&mut buf, &SettingRow::new("x", "V").kind(kind));
            let last = buf.content.last().unwrap();
            assert_eq!(last.style().fg, Some(expected), "{kind:?}");
        }
        let mut buf = buffer(40, 1);
        render_into(
            &mut buf,
            &SettingRow::new("Volume", "80%").unavailable(true),
        );
        let last = buf.content.last().unwrap();
        assert_eq!(last.style().fg, Some(theme.text_dim));
    }

    /// The sub-label: a second row, indented, `textSub` — and gone in
    /// compact, which is how 15 rows fit 24 terminals (§16.2 item 9).
    #[test]
    fn the_sub_label_takes_a_second_row_unless_compact() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let row = SettingRow::new("Mosque reminder", "ENABLED")
            .kind(ValueKind::Enabled)
            .subtitle("15–30 min after");
        assert_eq!(row.height(), 2);
        let mut buf = buffer(40, 2);
        render_into(&mut buf, &row);
        let sub = line_at(&buf, 1);
        assert!(sub.contains("15–30 min after"), "sub was {sub:?}");
        assert!(sub.starts_with("    "), "sub was {sub:?}");
        // The sub-label's first cell sits on row 1, under the label.
        let sub_first = (buf.area.width + 4) as usize;
        assert_eq!(buf.content[sub_first].style().fg, Some(theme.text_sub));

        let compact = row.compact(true);
        assert_eq!(compact.height(), 1);
        let mut buf = buffer(40, 2);
        render_into(&mut buf, &compact);
        assert_eq!(line_at(&buf, 1), " ".repeat(40));
    }

    /// A long label truncates with `…` rather than pushing the value
    /// off the row.
    #[test]
    fn a_long_label_truncates() {
        let mut buf = buffer(20, 1);
        render_into(
            &mut buf,
            &SettingRow::new("Per-prayer timing", "CONFIGURED"),
        );
        let line = line_at(&buf, 0);
        assert!(line.contains('…'), "row was {line:?}");
        assert!(line.ends_with("CONFIGURED"), "row was {line:?}");
        assert_eq!(line.chars().count(), 20);
    }
}
