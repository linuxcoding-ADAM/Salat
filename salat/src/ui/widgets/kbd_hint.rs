//! `[KEY] label` — green key, `textSub` label (§15.5, the design's
//! `KH`).

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::theme::Theme;

/// One key hint: `[Q] QUIT`.
pub fn line<'a>(key: &'a str, label: &'a str, theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("[{key}]"), Style::new().fg(theme.green)),
        Span::styled(format!(" {label}"), Style::new().fg(theme.text_sub)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The design's `KH`: brackets and key in green, the label after
    /// one space in `textSub`.
    #[test]
    fn the_hint_is_bracketed_key_then_label() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let spans = line("ENTER", "DISMISS", &theme).spans;
        assert_eq!(spans[0].content, "[ENTER]");
        assert_eq!(spans[0].style.fg, Some(theme.green));
        assert_eq!(spans[1].content, " DISMISS");
        assert_eq!(spans[1].style.fg, Some(theme.text_sub));
    }
}
