//! `● next` — glyph + word + colour, one `Line` of two `Span`s (§15.5).
//!
//! The vocabulary itself (which glyph, which word, which colour per
//! `PrayerState`) lives in [`crate::ui::theme`]; this widget is only
//! its two-span shape.

use ratatui::style::Style;
use ratatui::text::Line;

use crate::domain::PrayerState;
use crate::ui::theme::Theme;

/// The badge line: `✓ passed`, `● next`, `◈ soon`, …
pub fn line(state: PrayerState, theme: &Theme) -> Line<'static> {
    let (glyph, word, color) = theme.state_badge(state);
    Line::from(vec![
        ratatui::text::Span::styled(glyph, Style::new().fg(color)),
        ratatui::text::Span::styled(format!(" {word}"), Style::new().fg(color)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §14.6 rows' status cells, verbatim.
    #[test]
    fn the_badge_is_glyph_space_word() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let cases = [
            (PrayerState::Passed, "✓ passed"),
            (PrayerState::Next, "● next"),
            (PrayerState::Upcoming, "○ upcoming"),
        ];
        for (state, expected) in cases {
            let spans = line(state, &theme).spans;
            let text: String = spans.iter().flat_map(|s| s.content.chars()).collect();
            assert_eq!(text, expected);
            // Both spans carry the state's colour — the badge is one
            // signal, not two.
            let (.., color) = theme.state_badge(state);
            for span in &spans {
                assert_eq!(span.style.fg, Some(color));
            }
        }
    }
}
