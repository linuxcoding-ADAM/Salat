//! `●` / `✕` health mark (§15.5) — a single `Span`, the colour the
//! health implies. The Unknown case is the honest one (§16.4 item 8):
//! a probe that has not run shows `○` in `textDim` rather than a
//! falsely green `●`.

use ratatui::style::Style;
use ratatui::text::Span;

use crate::status::Health;
use crate::ui::theme::Theme;

/// The mark for one component's health.
pub fn span(health: Health, theme: &Theme) -> Span<'static> {
    let (mark, color) = match health {
        Health::Ok => ("●", theme.green),
        Health::Degraded => ("●", theme.yellow),
        Health::Down => ("✕", theme.red),
        Health::Unknown => ("○", theme.text_dim),
    };
    Span::styled(mark, Style::new().fg(color))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The design's `ok ? "●" : "✕"` vocabulary, extended with the
    /// two states the design does not show: degraded keeps the dot and
    /// turns it yellow, unknown is an open circle in `textDim`.
    #[test]
    fn each_health_has_its_mark_and_colour() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let cases = [
            (Health::Ok, "●", theme.green),
            (Health::Degraded, "●", theme.yellow),
            (Health::Down, "✕", theme.red),
            (Health::Unknown, "○", theme.text_dim),
        ];
        for (health, mark, color) in cases {
            let dot = span(health, &theme);
            assert_eq!(dot.content, mark);
            assert_eq!(dot.style.fg, Some(color));
        }
    }
}
