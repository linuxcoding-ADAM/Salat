//! The ERROR screen (§16.10) — a typed [`SalatError`] rendered as a
//! centered red panel in the design's idiom: `✕` title, code, body,
//! trace block, buttons.
//!
//! Layout (§16.10 item 4): `Vertical[ Length(2) title, Min(3) body,
//! Length(6) trace, Length(2) buttons ]`. The trace is the error's
//! `details()` lines on the `#100808` inset — the "what happened"
//! stack, kept distinct from the body's "what it means and what to
//! do". `details()` strings already carry their `→ ` prefixes, so
//! they render verbatim (§16.10 item 6); the body's note points at
//! the log file that §16.11 says holds the full chain.
//!
//! The retry state (item 8) is the one mutable piece: while a retry
//! runs, the crumb line shows `RETRYING…` with the attempt counter.
//! The boot-failure loop owns the actual re-run (§15.7); this screen
//! only renders the state it is handed.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::Widget as _;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::error::{ErrorAction, SalatError};
use crate::ui::layout::Breakpoint;
use crate::ui::theme::Theme;
use crate::ui::widgets::kbd_hint;

/// §16.10 item 8: while the failed operation is being retried, the
/// screen shows `RETRYING…` and the attempt counter — the retry
/// policy visible while it happens, not only afterwards in the trace.
/// `attempt` is 1-based; `None` is the idle (no retry in flight)
/// state, which is every frame outside the boot-retry beat.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetryState {
    /// `Some(n)` when attempt *n* is running — the label reads
    /// `attempt n of 3`.
    pub attempt: Option<u32>,
}

impl RetryState {
    /// The idle state — no retry in flight.
    pub fn idle() -> Self {
        Self { attempt: None }
    }
    /// The label the crumb shows: `RETRYING… · attempt n of 3`, or
    /// `None` when idle.
    fn label(&self) -> Option<String> {
        self.attempt.map(|attempt| {
            // The §16.10/§15.2 retry budget is three attempts.
            format!("RETRYING… · attempt {attempt} of 3")
        })
    }
}

/// Render the error panel over the body area.
pub fn render(
    error: &SalatError,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    render_with_retry(error, &RetryState::idle(), area, buf, theme, breakpoint);
}

/// The render with §16.10 item 8's retry state — the boot-failure
/// loop's RETRYING beat calls this directly; every other caller is
/// idle (a retry cannot be in flight while the screen is on the
/// post-boot stack, where `r` is a no-op by design).
pub fn render_with_retry(
    error: &SalatError,
    retry: &RetryState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    // §16.10 item 9: `min(72, w − 4)` at 80×24, clamped there above
    // it (item 10 — a wider error is not a better error).
    let width = 72.min(area.width.saturating_sub(4)).max(8);
    let x = area.x + (area.width.saturating_sub(width)) / 2;

    // Red border, red-washed interior (§16.10 item 5), then §16.10
    // item 4's four-part vertical over the inner rows: title (2),
    // body (Min 3), trace (6), buttons (2).
    let panel = crate::ui::widgets::panel::Panel::new()
        .title("ERROR")
        .border(theme.red)
        .background(theme.red_bg);
    let inner = panel.render(Rect::new(x, area.y, width, area.height), buf, theme);
    let [title_rows, body_rows, trace_rows, buttons_rows] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(6),
        Constraint::Length(2),
    ])
    .areas(inner);

    // Title (2 rows): `✕ TITLE` over `error[CODE]` — the RETRYING
    // label (item 8) rides the code row.
    put_line(
        title_rows.y,
        title_rows.x,
        title_rows.width,
        Line::from(vec![
            Span::styled(
                "✕ ",
                Style::new().fg(theme.red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                error.title().to_uppercase(),
                Style::new().fg(theme.red).add_modifier(Modifier::BOLD),
            ),
        ]),
        buf,
    );
    let mut code_line = vec![Span::styled(
        format!("error[{}]", error.code()),
        Style::new().fg(theme.text_dim),
    )];
    if let Some(label) = retry.label() {
        code_line.push(Span::raw("  "));
        code_line.push(Span::styled(label, Style::new().fg(theme.yellow)));
    }
    put_line(
        title_rows.y + 1,
        title_rows.x,
        title_rows.width,
        Line::from(code_line),
        buf,
    );

    // Body (Min 3): the title's meaning in prose (Display), the
    // help line, and — at 80×24 only — the log pointer (item 9).
    let mut body_lines = vec![
        Line::from(Span::styled(error.to_string(), Style::new().fg(theme.text))),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("help: {}", error.help()),
            Style::new().fg(theme.text_dim),
        )),
    ];
    if breakpoint == Breakpoint::Small {
        body_lines.push(Line::from(Span::styled(
            "full detail in ~/.local/state/salat/salat.log",
            Style::new().fg(theme.text_dim),
        )));
    }
    let body_par: ratatui::text::Text = body_lines.into();
    body_par.render(body_rows, buf);

    // Trace (Length 6): the `→` lines on the `#100808` inset — the
    // `details()` strings verbatim (they carry their own prefixes),
    // truncated to 4 lines + `…` at 80×24 (item 9), full 6 above it
    // (item 10). More lines than the budget scroll off — the log
    // file holds everything.
    render_trace(error, trace_rows, buf, theme, breakpoint);

    // Buttons (Length 2): the error's own actions plus QUIT.
    put_line(
        buttons_rows.y,
        buttons_rows.x,
        buttons_rows.width,
        hints_line(error, theme),
        buf,
    );
}

/// The trace block: the `→` lines on the `#100808` inset.
fn render_trace(
    error: &SalatError,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    if area.width < 2 || area.height < 1 {
        return;
    }
    // One column of padding each side; the block paints its own
    // ground so it reads as an inset inside the red wash. The budget
    // rows are the trace lines themselves (§16.10 items 9–10: 4 at
    // 80×24, 6 above) — a padding row would eat one of them.
    let block = ratatui::widgets::Block::new().style(Style::new().bg(theme.error_trace));
    block.render(area, buf);
    let inner_x = area.x + 1;
    let inner_w = area.width.saturating_sub(2);
    let max_lines = if breakpoint == Breakpoint::Small {
        4
    } else {
        6
    } as usize;
    let details = error.details();
    let shown = details.len().min(max_lines);
    let truncated = details.len() > max_lines;
    for (index, detail) in details.iter().take(shown).enumerate() {
        let y = area.y + index as u16;
        if y >= area.bottom() {
            break;
        }
        let mut line = truncate_cols(detail, inner_w as usize);
        if truncated && index + 1 == shown {
            // The cut is marked and the reader told where the rest
            // went (§16.10 item 9's promise).
            if line.chars().count() >= inner_w as usize {
                line = truncate_cols(&format!("{line}…"), inner_w as usize);
            } else {
                line.push('…');
            }
        }
        put_line(
            y,
            inner_x,
            inner_w,
            Line::from(Span::styled(line, Style::new().fg(theme.text_sub))),
            buf,
        );
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

/// Write one line at `(x, y)` with a `width` budget.
fn put_line(y: u16, x: u16, width: u16, line: Line<'_>, buf: &mut Buffer) {
    if width == 0 {
        return;
    }
    line.render(Rect::new(x, y, width, 1), buf);
}

/// Truncate to a cell width, marking the cut with `…`.
fn truncate_cols(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §16.10's shape for a boot failure: title, code, verbatim
    /// `→` details (never doubled — item 6), the trace inset, help
    /// and hints — all inside a red-bordered panel.
    #[test]
    fn a_boot_error_renders_code_details_and_help() {
        let error = SalatError::ConfigInvalid {
            problems: vec!["unknown method 'foo'".to_owned()],
        };
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
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
        // details() strings already carry `→ ` — the screen must
        // render them verbatim, never prefix a second arrow.
        assert!(text.contains("→ unknown method 'foo'"), "{text}");
        assert!(!text.contains("→ →"), "doubled arrow: {text}");
        assert!(text.contains("help:"), "{text}");
        assert!(text.contains("[R] RETRY"), "{text}");
        assert!(text.contains("[Q] QUIT"), "{text}");
        // §16.10 item 9: the 80×24 body points at the log file.
        assert!(text.contains("salat.log"), "{text}");
    }

    /// E001 — the §18.2 design specimen — renders with its exact
    /// detail lines and the [C] COORDINATES action button.
    #[test]
    fn e001_renders_the_designs_example() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(
            &SalatError::LocationLookupFailed,
            buf.area,
            &mut buf,
            &theme,
            Breakpoint::Small,
        );
        let text = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("error[E001]"), "{text}");
        assert!(text.contains("LOCATION LOOKUP FAILED"), "{text}");
        assert!(text.contains("→ HTTP 503 Service Unavailable"), "{text}");
        assert!(text.contains("→ retried 3× (1s · 2s · 4s)"), "{text}");
        assert!(text.contains("→ giving up"), "{text}");
        assert!(text.contains("[C] COORDINATES"), "{text}");
    }

    /// §16.10 item 8: the retry state paints `RETRYING…` and the
    /// attempt counter into the code row.
    #[test]
    fn the_retry_state_shows_attempt_counter() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render_with_retry(
            &SalatError::ApiUnreachable,
            &RetryState { attempt: Some(2) },
            buf.area,
            &mut buf,
            &theme,
            Breakpoint::Small,
        );
        let text = (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("RETRYING…"), "{text}");
        assert!(text.contains("attempt 2 of 3"), "{text}");
    }

    /// §16.10 item 9: at 80×24 a trace longer than four lines cuts at
    /// four and marks the cut with `…`; at 100×30 all six show.
    #[test]
    fn the_trace_truncates_at_small_and_shows_six_above() {
        let problems: Vec<String> = (1..=8).map(|n| format!("problem {n}")).collect();
        let error = SalatError::ConfigInvalid { problems };
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);

        let mut small = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(&error, small.area, &mut small, &theme, Breakpoint::Small);
        let small_text = (0..small.area.height)
            .map(|y| {
                (0..small.area.width)
                    .map(|x| small[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(small_text.contains("→ problem 4"), "{small_text}");
        assert!(!small_text.contains("→ problem 5"), "{small_text}");

        let mut medium = Buffer::empty(Rect::new(0, 0, 100, 30));
        render(&error, medium.area, &mut medium, &theme, Breakpoint::Medium);
        let medium_text = (0..medium.area.height)
            .map(|y| {
                (0..medium.area.width)
                    .map(|x| medium[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(medium_text.contains("→ problem 6"), "{medium_text}");
        assert!(!medium_text.contains("→ problem 7"), "{medium_text}");
    }
}
