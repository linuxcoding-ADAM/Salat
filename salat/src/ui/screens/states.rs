//! The STATES screen (§16.12) — `?` from any main screen: the
//! seven-state reference, one row per state with its badge, the
//! `MAGHRIB · 19:21` example, a one-line description and the live
//! visual (a real countdown for `next`, the blinking `◉ LIVE` for
//! `adhan`). Without live data the countdown columns show `--:--:--`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget as _;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::domain::PrayerState;
use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::theme::{Blink, Theme, blink};
use crate::ui::widgets::badge;
use crate::ui::widgets::crumb;

/// The seven rows, §10.1's order (§16.12 item 3).
const ROWS: [PrayerState; 7] = [
    PrayerState::Upcoming,
    PrayerState::Next,
    PrayerState::PreAdhan,
    PrayerState::Adhan,
    PrayerState::PostAdhan,
    PrayerState::Passed,
    PrayerState::Completed,
];

/// One row's fixed text (the design's example column and copy).
const EXAMPLE: &str = "MAGHRIB · 19:21";

/// The state's background and border colours (§16.12 item 7). `None`
/// background is the transparent row — the frame's ground.
fn colors(
    state: PrayerState,
    theme: &Theme,
) -> (Option<ratatui::style::Color>, ratatui::style::Color) {
    match state {
        PrayerState::Upcoming => (None, theme.border),
        PrayerState::Next => (Some(theme.row_active), theme.border_grn),
        PrayerState::PreAdhan => (Some(theme.yellow_bg), theme.yellow),
        PrayerState::Adhan => (Some(theme.red_bg), theme.red),
        PrayerState::PostAdhan => (Some(theme.red_bg), theme.red_dim),
        PrayerState::Passed => (None, theme.border_dim),
        PrayerState::Completed => (Some(theme.green_bg), theme.border_grn),
    }
}

fn description(state: PrayerState) -> &'static str {
    match state {
        PrayerState::Upcoming => "a later prayer today — nothing to do yet",
        PrayerState::Next => "the earliest future prayer — the countdown target",
        PrayerState::PreAdhan => "inside the pre-Adhan notification window",
        PrayerState::Adhan => "the call is live — the visual adhan window",
        PrayerState::PostAdhan => "the post-Adhan reminder window",
        PrayerState::Passed => "its time has passed, no completion marked",
        PrayerState::Completed => "marked done from the dashboard (c)",
    }
}

/// The live visual for a state (§16.12 item 5): the real countdown for
/// `next`, the fixed `12:47` for pre-Adhan, the blinking `◉ LIVE` for
/// adhan, `+15m` for post-Adhan — nothing for the rest.
fn visual(
    state: PrayerState,
    model: &UiModel,
    now: chrono::DateTime<chrono_tz::Tz>,
    theme: &Theme,
) -> Option<(String, Style)> {
    match state {
        PrayerState::Next => {
            let countdown = if model.next.is_some() {
                model.countdown.format()
            } else {
                "--:--:--".to_owned()
            };
            Some((countdown, Style::new().fg(theme.green)))
        }
        PrayerState::PreAdhan => Some(("12:47".to_owned(), Style::new().fg(theme.yellow))),
        PrayerState::Adhan => {
            let on = Blink::new(blink::ADHAN_BADGE_MS).on(now);
            let style = if on {
                Style::new().fg(theme.red).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.text_dim)
            };
            Some(("◉ LIVE".to_owned(), style))
        }
        PrayerState::PostAdhan => Some(("+15m".to_owned(), Style::new().fg(theme.red))),
        _ => None,
    }
}

/// Render the state reference into the body area.
pub fn render(
    model: &UiModel,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    let small = breakpoint == Breakpoint::Small;
    let mut y = area.y;
    if y >= area.bottom() {
        return;
    }

    // The crumb: the design's `PRAYER STATES` + `7 states` hint.
    crumb::render(
        Rect::new(area.x + 2, y, area.width.saturating_sub(2), 1),
        buf,
        theme,
        "PRAYER STATES",
        Some("7 states"),
    );
    y += 2;

    let now = model.now;
    for state in ROWS {
        let (background, border_color) = colors(state, theme);
        let visual = visual(state, model, now, theme);
        if small {
            // §16.12 item 9: two-row entries — badge + example + visual,
            // then the description truncated with `…`.
            let rows = Rect::new(area.x + 2, y, area.width.saturating_sub(2), 2);
            if rows.bottom() > area.bottom() {
                break;
            }
            if let Some(background) = background {
                buf.set_style(rows, Style::new().bg(background));
            }
            render_content_row(state, theme, rows, buf, visual, true);
            let desc = description(state);
            let truncated = truncate(desc, rows.width.saturating_sub(2) as usize);
            Line::from(Span::styled(truncated, Style::new().fg(theme.text_dim))).render(
                Rect::new(rows.x + 1, rows.y + 1, rows.width.saturating_sub(1), 1),
                buf,
            );
            y += 2;
        } else {
            // Length(3): border, content, border (§16.12 item 2).
            let rows = Rect::new(area.x + 2, y, area.width.saturating_sub(2), 3);
            if rows.bottom() > area.bottom() {
                break;
            }
            let rule = "─".repeat(rows.width as usize);
            Line::from(Span::styled(rule.clone(), Style::new().fg(border_color))).render(rows, buf);
            Line::from(Span::styled(rule, Style::new().fg(border_color)))
                .render(Rect::new(rows.x, rows.y + 2, rows.width, 1), buf);
            let content = Rect::new(rows.x, rows.y + 1, rows.width, 1);
            if let Some(background) = background {
                buf.set_style(content, Style::new().bg(background));
            }
            render_content_row(state, theme, content, buf, visual, false);
            y += 3;
        }
    }

    // §16.12 item 10: at Large, the footer note.
    if breakpoint == Breakpoint::Large && y < area.bottom() {
        Line::from(Span::styled(
            "press [c] on the dashboard to mark a prayer completed",
            Style::new().fg(theme.text_dim),
        ))
        .render(
            Rect::new(area.x + 2, y + 1, area.width.saturating_sub(2), 1),
            buf,
        );
    }
}

/// One entry's content row: `[badge] MAGHRIB · 19:21  description …
/// visual` — the description flexes, the visual hugs the right edge.
fn render_content_row(
    state: PrayerState,
    theme: &Theme,
    row: Rect,
    buf: &mut Buffer,
    visual: Option<(String, Style)>,
    small: bool,
) {
    let badge_text = {
        let spans = badge::line(state, theme).spans;
        spans
            .iter()
            .flat_map(|span| span.content.chars())
            .collect::<String>()
    };
    let badge_width = badge_text.chars().count() as u16;
    let mut spans = badge::line(state, theme).spans;
    let mut used = badge_width;

    // The example column shows at every width (§16.12 item 9 keeps it
    // at Small; only the descriptions flex).
    spans.push(Span::raw("  "));
    spans.push(Span::styled(EXAMPLE, Style::new().fg(theme.text_sub)));
    used += 2 + EXAMPLE.len() as u16;

    if let Some((visual_text, visual_style)) = visual {
        let visual_width = visual_text.chars().count() as u16;
        let reserved = visual_width + 2;
        if used + reserved <= row.width {
            // The description takes what remains between the example
            // and the visual.
            let desc_width = row.width - used - reserved;
            if desc_width > 4 {
                let desc = truncate(description(state), desc_width.saturating_sub(2) as usize);
                spans.push(Span::raw("  "));
                spans.push(Span::styled(desc, Style::new().fg(theme.text_dim)));
                used += desc_width;
            }
            spans.push(Span::raw(
                " ".repeat((row.width - used - visual_width) as usize),
            ));
            spans.push(Span::styled(visual_text, visual_style));
        }
    } else if !small {
        let desc_width = row.width.saturating_sub(used + 2);
        let desc = truncate(description(state), desc_width as usize);
        spans.push(Span::raw("  "));
        spans.push(Span::styled(desc, Style::new().fg(theme.text_dim)));
    }
    Line::from(spans).render(row, buf);
}

/// Truncate to a character width, marking the cut with `…`.
fn truncate(text: &str, width: usize) -> String {
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
    use crate::app::runtime::testing;

    fn rendered(width: u16, height: u16) -> Buffer {
        let fixture = testing::snapshot_runtime();
        let model = crate::ui::model::build_model(&fixture.runtime);
        let breakpoint = Breakpoint::of_size(width, height);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        render(
            &model,
            buf.area,
            &mut buf,
            &Theme::for_mode(crate::ui::theme::RenderMode::Truecolor),
            breakpoint,
        );
        buf
    }

    fn lines(buf: &Buffer) -> Vec<String> {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_owned())
                    .collect()
            })
            .collect()
    }

    /// All seven states appear, each with its badge — at Small as
    /// 2-row entries (§16.12 item 9).
    #[test]
    fn all_seven_states_show_at_80x24() {
        let buf = rendered(80, 24);
        let text = lines(&buf).join("\n");
        for word in [
            "upcoming",
            "next",
            "soon",
            "adhan",
            "reminder",
            "passed",
            "completed",
        ] {
            assert!(text.contains(word), "missing {word} in:\n{text}");
        }
        assert!(text.contains("7 states"), "the crumb hint");
        assert!(text.contains("MAGHRIB · 19:21"), "the example column");
    }

    /// The live countdown for `next` comes from the model: the §20.3
    /// fixture's Maghrib countdown.
    #[test]
    fn the_next_row_shows_the_real_countdown() {
        let buf = rendered(100, 30);
        let text = lines(&buf).join("\n");
        assert!(
            text.contains("01:42:37"),
            "the snapshot fixture's countdown: {text}"
        );
    }

    /// Without live data (`--:--:--`), nothing panics.
    #[test]
    fn truncated_descriptions_never_panic() {
        let buf = rendered(80, 24);
        assert_eq!(buf.area.height, 24);
    }

    /// §16.12's snapshot: the full screen at 100×30.
    #[test]
    fn the_states_screen_snapshot() {
        let buf = rendered(100, 30);
        let text = lines(&buf).join("\n");
        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
        settings.bind(|| insta::assert_snapshot!("states_screen_100x30", text));
    }
}
