//! The Status screen (§16.4) — the `[4] STATUS` tab: the ten
//! components' health, the NEXT EVENT countdown, TODAY's counters and
//! the RECENT EVENTS log.
//!
//! Rendering is pure in the model: every fact comes from
//! `model.status` (the §15.8 `Collector`'s assembly — this screen
//! never reaches a service), the clock from `model.now`. The one
//! piece of shell state is the log's scroll offset and the `r`-driven
//! `refreshing` flag, which paints `REFRESHING…` into the crumb while
//! the tick arm re-probes (the flag itself lives in the shell; the
//! screen only reads it).
//!
//! Sizes (§16.4 item 9–10): at 80×24 the right column and the event
//! sub-lines are omitted — ten component rows and a one-line-per-event
//! log; from 100×30 the three panels sit in two columns, the event log
//! grows to 7 (100×30) then 11 (120×35) rows with sub-lines, and the
//! crumb gains the live `HH:MM:SS` clock.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::Widget as _;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use chrono::Timelike as _;

use crate::domain::countdown::Countdown;
use crate::status::{Component, EventCategory};
use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::theme::{Theme, hhmm};
use crate::ui::widgets::bar;
use crate::ui::widgets::crumb;
use crate::ui::widgets::panel::Panel;
use crate::ui::widgets::status_dot;

use crate::ui::shell::StatusState;

/// Render the status body (§16.4).
pub fn render(
    model: &UiModel,
    state: &StatusState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    if area.width < 12 || area.height < 8 {
        return;
    }
    let small = breakpoint == Breakpoint::Small;
    if small {
        // §16.4 item 9: the whole right column is omitted — SERVICES
        // takes the height, RECENT EVENTS is a 6-row scroll region
        // with one line per event.
        let [services, events] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(8)]).areas(area);
        render_services(model, inset(services), buf, theme);
        render_events(model, state, inset(events), buf, theme, false);
    } else {
        // §16.4 item 4: crumb, the two-column top, the 9-row event
        // log (7 event rows + 2 border).
        let [crumb_area, top, events] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(9),
        ])
        .areas(area);
        crumb::render(
            Rect::new(
                crumb_area.x + 2,
                crumb_area.y,
                crumb_area.width.saturating_sub(2),
                1,
            ),
            buf,
            theme,
            "SYSTEM STATUS",
            Some(&live_clock(model, state)),
        );
        let [left, right] =
            Layout::horizontal([Constraint::Min(0), Constraint::Percentage(42)]).areas(top);
        render_services(model, inset(left), buf, theme);
        let [next_area, today_area] =
            Layout::vertical([Constraint::Length(6), Constraint::Min(0)]).areas(inset(right));
        render_next_event(model, next_area, buf, theme);
        render_today(model, today_area, buf, theme, breakpoint);
        render_events(model, state, inset(events), buf, theme, true);
    }
}

/// The crumb's right side (§16.4 item 10): the live `HH:MM:SS` clock —
/// or `REFRESHING…` while the `r`-driven re-probe runs.
fn live_clock(model: &UiModel, state: &StatusState) -> String {
    if state.refreshing {
        return "REFRESHING…".to_owned();
    }
    let now = model.now;
    format!("{:02}:{:02}:{:02}", now.hour(), now.minute(), now.second())
}

/// The SERVICES panel (§16.4 item 6): the ten components, `○`/`●`/`✕`
/// mark, the design's label and detail one row each.
fn render_services(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let Some(inner) = panel_inner(Panel::new().title("SERVICES"), area, buf, theme) else {
        return;
    };
    for (index, component) in model.status.components.iter().enumerate() {
        let y = inner.y + index as u16;
        if y >= inner.bottom() {
            break;
        }
        render_service_row(component, inner.x, y, inner.width, buf, theme);
    }
}

/// One SERVICES row: `mark label` left, the detail right-aligned in
/// `text_sub` — the detail never crowds the label.
fn render_service_row(
    component: &Component,
    x: u16,
    y: u16,
    width: u16,
    buf: &mut Buffer,
    theme: &Theme,
) {
    // Two columns of inset inside the panel, then the mark and its
    // space; label, gap and detail share the rest, the detail ending at
    // the panel's edge.
    let row_x = x + 2;
    let row_width = width.saturating_sub(2);
    let budget = row_width.saturating_sub(2) as usize;
    let label = truncate(component.label, budget);
    let detail = truncate(&component.detail, budget);
    let gap = right_gap(&label, &detail, budget);
    let mut spans = vec![
        status_dot::span(component.health, theme),
        Span::raw(" "),
        Span::styled(label, Style::new().fg(theme.text)),
    ];
    if let Some(gap) = gap {
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(detail, Style::new().fg(theme.text_sub)));
    }
    Line::from(spans).render(Rect::new(row_x, y, row_width, 1), buf);
}

/// The gap that right-aligns `detail` after `label` within `width`
/// budget columns — `None` when the two cannot both fit (the label
/// wins; the detail is dropped rather than wrapped).
fn right_gap(label: &str, detail: &str, width: usize) -> Option<usize> {
    let used = label.chars().count() + detail.chars().count();
    if used + 1 > width {
        return None;
    }
    Some(width - used)
}

/// The NEXT EVENT panel (§16.4 item 6): what fires next and when —
/// active, with the 2-row countdown and its gauge at 100×30+.
fn render_next_event(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let Some(inner) = panel_inner(
        Panel::new().title("NEXT EVENT").active(true),
        area,
        buf,
        theme,
    ) else {
        return;
    };
    match &model.status.next_event {
        Some(next) => {
            let when = hhmm(next.fire_at, &model.config.display.time_format);
            let what = match next.prayer {
                Some(prayer) => format!("{} · {}", event_label(next.kind), prayer.display()),
                None => event_label(next.kind).to_owned(),
            };
            put_line(
                inner.x + 2,
                inner.y,
                inner.width.saturating_sub(2),
                vec![
                    Span::styled(what, Style::new().fg(theme.text)),
                    Span::raw(" "),
                    Span::styled(when, Style::new().fg(theme.green)),
                ],
                buf,
            );
            // The 2-row countdown and its gauge (§16.4 item 10).
            let until = Countdown::between(next.fire_at, model.now);
            put_line(
                inner.x + 2,
                inner.y + 2,
                inner.width.saturating_sub(2),
                vec![Span::styled(
                    until.format(),
                    Style::new().fg(theme.text_bright),
                )],
                buf,
            );
            bar::render(
                Rect::new(inner.x + 2, inner.y + 4, inner.width.saturating_sub(2), 1),
                buf,
                theme,
                countdown_progress(next.fire_at, model),
                26,
                None,
            );
        }
        None => put_line(
            inner.x + 2,
            inner.y,
            inner.width.saturating_sub(2),
            vec![Span::styled(
                "no upcoming event",
                Style::new().fg(theme.text_dim),
            )],
            buf,
        ),
    }
}

/// The gauge's fill for the NEXT EVENT countdown: how much of the span
/// from `previous` to the firing has elapsed. Without a `previous`
/// (or a degenerate span) the gauge is empty — the honest answer
/// rather than a made-up fill.
fn countdown_progress(fire_at: chrono::DateTime<chrono_tz::Tz>, model: &UiModel) -> f64 {
    let Some(previous) = &model.previous else {
        return 0.0;
    };
    crate::domain::countdown::progress(previous.at, fire_at, model.now)
}

/// The TODAY panel (§16.4 item 6): the six counters. At 120×35 the
/// design's two-column × three grid; the single column otherwise.
fn render_today(
    model: &UiModel,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    let Some(inner) = panel_inner(Panel::new().title("TODAY"), area, buf, theme) else {
        return;
    };
    let today = &model.status.today;
    let next_notify = today
        .next_notify
        .map(|at| hhmm(at, &model.config.display.time_format))
        .unwrap_or_else(|| "—".to_owned());
    let stats: [(&str, String); 6] = [
        ("Total prayers", today.total.to_string()),
        ("Passed", today.passed.to_string()),
        ("Remaining", today.remaining.to_string()),
        ("Notifications sent", today.notifications_sent.to_string()),
        ("Adhan plays", today.adhan_plays.to_string()),
        ("Next notify", next_notify),
    ];
    if breakpoint == Breakpoint::Large {
        // Two columns of three: the design's 120×35 grid.
        let half = inner.width / 2;
        for (index, (label, value)) in stats.iter().enumerate() {
            let (column, row) = (index / 3, index % 3);
            let x = inner.x + 2 + (column as u16) * half;
            let width = (half.saturating_sub(2)) as usize;
            put_stat(x, inner.y + row as u16, width, label, value, buf, theme);
        }
    } else {
        let width = inner.width.saturating_sub(2) as usize;
        for (index, (label, value)) in stats.iter().enumerate() {
            put_stat(
                inner.x + 2,
                inner.y + index as u16,
                width,
                label,
                value,
                buf,
                theme,
            );
        }
    }
}

/// One TODAY stat row: `label` in `text_sub`, the value right-aligned
/// in `text`.
fn put_stat(
    x: u16,
    y: u16,
    width: usize,
    label: &str,
    value: &str,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let label = truncate(label, width);
    let value = truncate(value, width);
    let gap = right_gap(&label, &value, width);
    let mut spans = vec![Span::styled(label, Style::new().fg(theme.text_sub))];
    if let Some(gap) = gap {
        spans.push(Span::raw(" ".repeat(gap)));
        spans.push(Span::styled(value, Style::new().fg(theme.text)));
    }
    Line::from(spans).render(Rect::new(x, y, width as u16, 1), buf);
}

/// The RECENT EVENTS panel (§16.4 item 6): today's log, newest first,
/// `time │ label` per row with the dim sub-line where the size allows.
/// Scrolled by `state.scroll`; an empty log shows the design's
/// `no events yet today` placeholder (item 8), never a blank panel.
#[allow(clippy::too_many_lines)]
fn render_events(
    model: &UiModel,
    state: &StatusState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    sublines: bool,
) {
    let events = &model.status.recent;
    let Some(inner) = panel_inner(
        Panel::new().title("RECENT EVENTS · TODAY"),
        area,
        buf,
        theme,
    ) else {
        return;
    };
    if events.is_empty() {
        put_line(
            inner.x + 2,
            inner.y,
            inner.width.saturating_sub(2),
            vec![Span::styled(
                "no events yet today",
                Style::new().fg(theme.text_dim),
            )],
            buf,
        );
        return;
    }

    // Each event renders one line, or two when the size allows: a row
    // and its dim sub-line. The scroll offset is in *event* units —
    // the up/down keys move by events, not by rendered lines.
    let per_event: u16 = if sublines { 2 } else { 1 };
    let visible = (inner.height / per_event.max(1)).max(1);
    let scroll = (state.scroll as usize).min(events.len().saturating_sub(visible as usize));
    for (slot, event) in events
        .iter()
        .skip(scroll)
        .take(visible as usize)
        .enumerate()
    {
        let y = inner.y + (slot as u16) * per_event;
        let time = hhmm(event.at, &model.config.display.time_format);
        let color = match event.category {
            EventCategory::Notification => theme.yellow,
            EventCategory::Adhan => theme.blue,
            EventCategory::State | EventCategory::System | EventCategory::Error => theme.text_dim,
        };
        let label = if sublines {
            event.label.clone()
        } else {
            // §16.4 item 9: one line per event at 80×24 — the prayer
            // prefix rides the time line, since there is no sub-line.
            match event.prayer {
                Some(prayer) => format!("{} {}", prayer.display(), event.label),
                None => event.label.clone(),
            }
        };
        put_line(
            inner.x + 2,
            y,
            inner.width.saturating_sub(2),
            vec![
                Span::styled(time, Style::new().fg(color)),
                Span::raw(" │ "),
                Span::styled(label, Style::new().fg(theme.text)),
            ],
            buf,
        );
        if sublines {
            let sub = event
                .detail
                .as_deref()
                .map(|detail| truncate(detail, inner.width.saturating_sub(4) as usize))
                .unwrap_or_default();
            if !sub.is_empty() {
                put_line(
                    inner.x + 4,
                    y + 1,
                    inner.width.saturating_sub(4),
                    vec![Span::styled(sub, Style::new().fg(theme.text_dim))],
                    buf,
                );
            }
        }
    }
}

/// The event kinds' one-line names, as the NEXT EVENT panel shows
/// them (§16.4 item 6).
fn event_label(kind: crate::domain::EventKind) -> &'static str {
    use crate::domain::EventKind;
    match kind {
        EventKind::PreNotify => "Pre-Adhan notification",
        EventKind::Adhan => "Adhan",
        EventKind::MosqueReminder => "Mosque reminder",
        EventKind::DayRollover => "Day rollover",
        EventKind::AdhanSnooze => "Adhan (snoozed)",
        EventKind::ReminderSnooze => "Reminder (snoozed)",
    }
}

/// The screen's blocks sit one column inside the body, so the panels
/// never touch the chrome frame's edge.
fn inset(area: Rect) -> Rect {
    Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(2),
        area.height,
    )
}

/// Render a titled panel and hand back its inner area — skipped when
/// the space is too small to hold a border and a row.
fn panel_inner<'p>(panel: Panel<'p>, area: Rect, buf: &mut Buffer, theme: &Theme) -> Option<Rect> {
    if area.width < 12 || area.height < 3 {
        return None;
    }
    Some(panel.render(area, buf, theme))
}

/// Write one span line at `(x, y)` with a `width` budget.
fn put_line(x: u16, y: u16, width: u16, spans: Vec<Span<'_>>, buf: &mut Buffer) {
    if width == 0 {
        return;
    }
    Line::from(spans).render(Rect::new(x, y, width, 1), buf);
}

/// Truncate to a cell width, marking the cut with `…`.
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

/// The `Health::Unknown` mark is the open circle — pinned here so the
/// screen's honesty rule (§16.4 item 8) has a render-side test.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{ComponentId, Health};

    /// The right-alignment gap: exact fit, no fit, and the one-space
    /// minimum separation.
    #[test]
    fn the_detail_gap_math() {
        assert_eq!(right_gap("Network", "ONLINE", 14), Some(1));
        assert_eq!(right_gap("Network", "ONLINE", 18), Some(5));
        assert_eq!(right_gap("Prayer calculation", "OK", 12), None);
    }

    /// A component row renders the mark, the label, the detail.
    #[test]
    fn a_service_row_shows_mark_label_detail() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 1));
        render_service_row(
            &Component::new(ComponentId::Network, Health::Unknown, "NOT PROBED"),
            0,
            0,
            40,
            &mut buf,
            &theme,
        );
        let row: String = (0..40).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert!(row.contains("○"));
        assert!(row.contains("Network"));
        assert!(row.contains("NOT PROBED"));
    }
}
