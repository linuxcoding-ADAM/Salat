//! The app frame's fixed chrome (§3.4): the two-row header and
//! two-row footer the full UI draws around every screen body. Colour
//! and visibility rules come from §3.4; the component shapes follow
//! the design's `TUIHeader`/`TUIFooter`.
//!
//! Only the full UI renders chrome; the overlays and the loading
//! screen suppress it (§3.3), and quick mode never enters this module.

use chrono::Timelike as _;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget as _;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::config::TimeFormat;
use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::theme::Theme;

use crate::ui::shell::Screen;

/// The app version the header shows beside the wordmark.
const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

/// The header's tab buttons, in `[1]`–`[4]` order (§3.4). Design labels
/// for the UI; the code identifiers live in `Screen`.
const TABS: [(&str, Screen); 4] = [
    ("TODAY", Screen::Dashboard),
    ("SETTINGS", Screen::Settings),
    ("PRAYERS", Screen::PrayerConfig),
    ("STATUS", Screen::Status),
];

/// Row 0: the wordmark, version, and the right group — location, date,
/// clock (§3.4). At `Small` (80×24) the version and the location+date
/// drop, leaving `SALAT … 19:21:03`. Row 1 is the border rule.
///
/// `breakpoint` is the *frame's* classification, computed by the caller
/// — the header's own two-row strip would classify `TooSmall` and can
/// never be the basis of the §3.4 rules.
pub fn render_header(
    model: &UiModel,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    if area.height < 2 || area.width < 8 {
        return;
    }
    let small = breakpoint == Breakpoint::Small;
    buf.set_style(area, Style::new().bg(theme.term_bg));

    // Left: `SALAT` green bold, ` v0.1.0` borderHi — hidden at Small.
    let mut left = vec![Span::styled(
        "SALAT",
        Style::new().fg(theme.green).add_modifier(Modifier::BOLD),
    )];
    if !small {
        left.push(Span::styled(
            format!(" {VERSION}"),
            Style::new().fg(theme.border_hi),
        ));
    }
    Line::from(left).render(
        Rect::new(area.x + 2, area.y, area.width.saturating_sub(2), 1),
        buf,
    );

    // Right: `SÉTIF, ALGERIA │ 05 SEP 2026 │ 19:21:03` — location and
    // date textSub with borderDim separators, the clock green; at Small
    // only the clock survives.
    let clock = clock_string(model.now, &model.config.display.time_format);
    let mut right = vec![Span::styled(
        clock,
        Style::new().fg(theme.green).add_modifier(Modifier::BOLD),
    )];
    if !small {
        let date = model.now.format("%d %b %Y").to_string().to_uppercase();
        let location = model.config.location.label.to_uppercase();
        right.insert(
            0,
            Span::styled(
                format!("{location} │ {date}"),
                Style::new().fg(theme.text_sub),
            ),
        );
        right.insert(1, Span::styled(" │ ", Style::new().fg(theme.border_dim)));
    }
    let right_width: usize = right.iter().map(|span| span.content.len()).sum();
    let right_x = area.x + area.width.saturating_sub(right_width as u16 + 2);
    Line::from(right).render(Rect::new(right_x, area.y, area.width, 1), buf);

    // Row 1: the border rule.
    let rule = "─".repeat(area.width as usize);
    Line::from(Span::styled(rule, Style::new().fg(theme.border)))
        .render(Rect::new(area.x, area.y + 1, area.width, 1), buf);
}

/// Rows 0–1 of the footer area: the border rule, then either the four
/// tab buttons (a main screen is focused) or the sub-screen key hints
/// (§3.4). At `Small` the labels drop, leaving `[1] [2] [3] [4]` and a
/// bare `[Q]`; the sub-screen row drops `[S] save`. Like the header,
/// `breakpoint` is the frame's classification, passed by the caller.
pub fn render_footer(
    active: &Screen,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    if area.height < 2 || area.width < 8 {
        return;
    }
    let small = breakpoint == Breakpoint::Small;
    buf.set_style(area, Style::new().bg(theme.term_bg));

    let rule = "─".repeat(area.width as usize);
    Line::from(Span::styled(rule, Style::new().fg(theme.border)))
        .render(Rect::new(area.x, area.y, area.width, 1), buf);

    let row = Rect::new(area.x + 2, area.y + 1, area.width.saturating_sub(2), 1);
    if active.is_main() {
        render_main_footer(active, row, buf, theme, small);
    } else {
        render_sub_footer(row, buf, theme, small);
    }
}

/// `[1] TODAY  [2] SETTINGS …` with the active tab inverted (green
/// ground, `termBg` text), then `[Q] QUIT` at the right edge.
fn render_main_footer(active: &Screen, area: Rect, buf: &mut Buffer, theme: &Theme, small: bool) {
    let mut spans = Vec::new();
    for (index, (label, screen)) in TABS.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let is_active = screen == active;
        let style = if is_active {
            Style::new().fg(theme.term_bg).bg(theme.green)
        } else {
            Style::new().fg(theme.green)
        };
        spans.push(Span::styled(format!("[{}]", index + 1), style));
        if !small {
            let label_style = if is_active {
                Style::new().fg(theme.term_bg).bg(theme.green)
            } else {
                Style::new().fg(theme.text_sub)
            };
            spans.push(Span::styled(format!(" {label}"), label_style));
        }
    }
    let tabs_width: usize = spans.iter().map(|span| span.content.len()).sum();
    // Right-align QUIT with a two-cell margin; if it cannot fit, the
    // tabs stand alone.
    let quit_width = if small { 3 } else { 8 };
    if tabs_width + 2 + quit_width + 2 <= area.width as usize {
        spans.push(Span::raw(
            " ".repeat(area.width as usize - tabs_width - quit_width - 2),
        ));
        spans.push(Span::styled("[Q]", Style::new().fg(theme.red)));
        if !small {
            spans.push(Span::styled(" QUIT", Style::new().fg(theme.text_sub)));
        }
    }
    Line::from(spans).render(area, buf);
}

/// `↑↓ move · ←→ adjust · ENTER select · ESC back · S save` — the
/// design's `KH` hints (§15.4's uniform vocabulary).
fn render_sub_footer(area: Rect, buf: &mut Buffer, theme: &Theme, small: bool) {
    let mut hints: Vec<(&str, &str)> = vec![
        ("↑↓", "move"),
        ("←→", "adjust"),
        ("ENTER", "select"),
        ("ESC", "back"),
    ];
    if !small {
        hints.push(("S", "save"));
    }
    let mut spans = Vec::new();
    for (index, (key, label)) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            format!("[{key}]"),
            Style::new().fg(theme.green),
        ));
        spans.push(Span::styled(
            format!(" {label}"),
            Style::new().fg(theme.text_sub),
        ));
    }
    Line::from(spans).render(area, buf);
}

/// `19:21:03` in the configured format (the design clock is 24-hour;
/// 12-hour shows `7:21:03 PM`).
fn clock_string(now: chrono::DateTime<chrono_tz::Tz>, format: &TimeFormat) -> String {
    let (hour, minute, second) = (now.hour(), now.minute(), now.second());
    match format {
        TimeFormat::TwentyFourHour => format!("{hour:02}:{minute:02}:{second:02}"),
        TimeFormat::TwelveHour => {
            let (hour12, meridiem) = match hour % 12 {
                0 => (12, if hour < 12 { "AM" } else { "PM" }),
                hour12 => (hour12, if hour < 12 { "AM" } else { "PM" }),
            };
            format!("{hour12:02}:{minute:02}:{second:02} {meridiem}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The clock is `HH:MM:SS` in the 24-hour default.
    #[test]
    fn the_clock_shows_seconds() {
        use chrono::TimeZone as _;
        let now = chrono_tz::Africa::Algiers
            .with_ymd_and_hms(2026, 9, 5, 19, 21, 3)
            .unwrap();
        assert_eq!(clock_string(now, &TimeFormat::TwentyFourHour), "19:21:03");
    }

    /// §3.4's degradation: at Small the labels drop, `[n]` stays.
    #[test]
    fn tabs_and_quit_survive_at_small() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 2));
        render_footer(
            &Screen::Dashboard,
            buf.area,
            &mut buf,
            &theme,
            Breakpoint::Small,
        );
        let row: String = buf
            .content
            .iter()
            .skip(80)
            .take(80)
            .map(|cell| cell.symbol().to_owned())
            .collect();
        assert!(row.contains("[1]"), "{row}");
        assert!(row.contains("[Q]"), "{row}");
        assert!(!row.contains("TODAY"), "the label dropped at 80×24: {row}");
    }

    /// The active tab is inverted (green ground), the others are not.
    #[test]
    fn the_active_tab_inverts() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 2));
        render_footer(
            &Screen::PrayerConfig,
            buf.area,
            &mut buf,
            &theme,
            Breakpoint::Medium,
        );
        let row: String = buf
            .content
            .iter()
            .skip(120)
            .take(120)
            .map(|cell| cell.symbol().to_owned())
            .collect();
        let active_start = row.find("[3] PRAYERS").unwrap_or_default();
        let cell = &buf.content[120 + active_start + 1];
        assert_eq!(cell.style().bg, Some(theme.green), "row: {row}");
    }

    /// A sub-screen footer is the key-hint vocabulary, `S save` intact
    /// above Small.
    #[test]
    fn sub_footers_show_the_vocabulary() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 2));
        render_footer(
            &Screen::States,
            buf.area,
            &mut buf,
            &theme,
            Breakpoint::Medium,
        );
        let row: String = buf
            .content
            .iter()
            .skip(120)
            .take(120)
            .map(|cell| cell.symbol().to_owned())
            .collect();
        for expected in ["[↑↓] move", "[ENTER] select", "[S] save"] {
            assert!(row.contains(expected), "missing {expected} in {row}");
        }
    }
}
