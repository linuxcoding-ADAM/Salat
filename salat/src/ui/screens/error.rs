//! The ERROR screen (§16.10) — a typed [`SalatError`] rendered as a
//! centered red panel. Phase 6 implements the boot-failure shape: the
//! `✕` title line, the code, every detail, the help line and the
//! `[R] RETRY` / `[Q] QUIT` hints (the keys the boot failure loop
//! answers). Phase 12 completes the screen (the `#100808` trace block,
//! the `c`/`d` actions).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget as _;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::error::{ErrorAction, SalatError};
use crate::ui::layout::Breakpoint;
use crate::ui::theme::Theme;
use crate::ui::widgets::kbd_hint;

/// Render the error panel over the body area.
pub fn render(
    error: &SalatError,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    _breakpoint: Breakpoint,
) {
    // §16.10 item 9: `min(72, w − 4)` at 80×24, clamped there above it.
    let width = 72.min(area.width.saturating_sub(4)).max(8);
    let x = area.x + (area.width.saturating_sub(width)) / 2;

    // Title, code, one row per detail, help, then the hints.
    let mut lines: Vec<Line> = vec![Line::from(vec![
        Span::styled(
            "✕ ",
            Style::new().fg(theme.red).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            error.title().to_uppercase(),
            Style::new().fg(theme.red).add_modifier(Modifier::BOLD),
        ),
    ])];
    lines.push(Line::from(Span::styled(
        format!("error[{}]", error.code()),
        Style::new().fg(theme.text_dim),
    )));
    if !error.details().is_empty() {
        lines.push(Line::from(Span::raw("")));
    }
    for detail in error.details() {
        lines.push(Line::from(Span::styled(
            format!("→ {detail}"),
            Style::new().fg(theme.text_sub),
        )));
    }
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        format!("help: {}", error.help()),
        Style::new().fg(theme.text_dim),
    )));
    lines.push(Line::from(Span::raw("")));
    lines.push(hints_line(error, theme));

    let height = (lines.len() as u16 + 2).min(area.height);
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    // Red border, red-washed interior (§16.10 item 3).
    let panel = crate::ui::widgets::panel::Panel::new()
        .title("ERROR")
        .border(theme.red)
        .background(theme.red_bg);
    let rect = Rect::new(x, y, width, height);
    let inner = panel.render(rect, buf, theme);
    for (index, line) in lines.iter().enumerate() {
        let row = inner.y + index as u16;
        if row >= inner.bottom() {
            break;
        }
        let line_width = line.width() as u16;
        let start = inner.x + inner.width.saturating_sub(line_width.max(1)) / 2;
        line.render(Rect::new(start.min(inner.x), row, inner.width, 1), buf);
    }
}

/// The action hints: `[R] RETRY [C] COORDINATES [D] USE DEFAULTS
/// [ESC] BACK` — one row from the error's own actions, plus QUIT.
fn hints_line(error: &SalatError, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for action in error.actions() {
        let (key, label): (&str, &str) = match action {
            ErrorAction::Retry => ("R", "RETRY"),
            ErrorAction::Coordinates => ("C", "COORDINATES"),
            ErrorAction::UseDefaults => ("D", "USE DEFAULTS"),
            ErrorAction::Back => ("ESC", "BACK"),
        };
        if !spans.is_empty() {
            spans.push(Span::raw("  "));
        }
        spans.extend(kbd_hint::line(key, label, theme).spans);
    }
    spans.push(Span::raw("  "));
    spans.extend(kbd_hint::line("Q", "QUIT", theme).spans);
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §16.10's shape for a boot failure: title, code, details, help,
    /// hints — all inside a red-bordered panel.
    #[test]
    fn a_boot_error_renders_code_details_and_help() {
        let error = SalatError::ConfigInvalid {
            problems: vec!["unknown method 'foo'".to_owned()],
        };
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 20));
        render(&error, buf.area, &mut buf, &theme, Breakpoint::Small);
        let text = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("ERROR"), "{text}");
        assert!(text.contains("error[E201]"), "{text}");
        assert!(text.contains("→ unknown method 'foo'"), "{text}");
        assert!(text.contains("help:"), "{text}");
        assert!(text.contains("[R] RETRY"), "{text}");
        assert!(text.contains("[Q] QUIT"), "{text}");
    }
}
