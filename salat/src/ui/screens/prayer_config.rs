//! The Prayer Configuration screen (§16.3) — the `[3] PRAYERS` tab:
//! one prayer at a time, with more room than the dense per-prayer
//! grid, its sound preset, and a timeline preview.
//!
//! Values render from `pending_config()` (§8.5) — the draft's working
//! copy while an edit session is in flight, the effective config
//! otherwise — and the panel title carries the prayer's actual time
//! today (`MAGHRIB · 19:21`), because the numbers being edited never
//! depend on the API.
//!
//! The cursor's row 0 is the tab bar's own focus (§16.3 item 7's
//! dedicated first row): `←→` there switches the prayer tab, below it
//! `←→` adjusts the focused stepper by the design's ±5, clamped
//! `0..=60`. `Tab`/`Shift-Tab` always switch tabs regardless of row —
//! the escape hatch that keeps both behaviours reachable from any
//! row, and the reason the crumb hint states which mode `←→` is in.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::Widget as _;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr as _;

use chrono::Duration;

use crate::domain::prayer::Notifiable;
use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::shell::PrayerConfigState;
use crate::ui::theme::{Theme, hhmm};
use crate::ui::widgets::crumb;
use crate::ui::widgets::hr;
use crate::ui::widgets::panel::Panel;
use crate::ui::widgets::setting_row::{SettingRow, ValueKind};
use crate::ui::widgets::stepper::Stepper;

use super::adhan_alert::preset_display;

/// The cursor's rows (§16.3 item 7): row 0 is the tab bar's focus —
/// the row whose `←→` switches the prayer — and the rows below it
/// walk the panel top-to-bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Row {
    Tab,
    Before,
    After,
    Adhan,
    Notif,
    Preset,
}

impl Row {
    /// Cursor order — the tab bar first, then the panel top-to-bottom.
    pub(crate) const ALL: [Row; 6] = [
        Row::Tab,
        Row::Before,
        Row::After,
        Row::Adhan,
        Row::Notif,
        Row::Preset,
    ];

    /// The row a flat `usize` cursor addresses, `None` past the end.
    pub(crate) fn at(index: usize) -> Option<Row> {
        Row::ALL.get(index).copied()
    }
}

/// Render the prayer-config body (§16.3 item 4): crumb, tab bar, the
/// prayer's panel, the notification timeline, the SAVE/CANCEL row.
pub fn render(
    model: &UiModel,
    state: &PrayerConfigState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    let small = breakpoint == Breakpoint::Small;
    let large = breakpoint == Breakpoint::Large;
    let prayer = tab_prayer(state.tab);

    // The crumb hint states the row-dependent `←→` meaning (§16.3
    // item 7): tab at the tab bar's row, adjust below it. At small the
    // preset row is gone, so the pointer to where it lives rides every
    // hint (§16.3 item 9).
    let hint = match (state.row == 0, small) {
        (true, false) => "←→ tab · ENTER · ESC back",
        (true, true) => "←→ tab · ESC back · presets in SETTINGS",
        (false, true) => "←→ adjust · TAB tab · presets in SETTINGS",
        (false, false) => "←→ adjust · TAB tab",
    };

    // 80×24 takes the 4-row single-line timeline (§16.3 item 9);
    // medium and large keep the 7-row panel with the label line.
    let [
        crumb_area,
        tabs_area,
        panel_area,
        timeline_area,
        buttons_area,
    ] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(if small { 4 } else { 7 }),
        Constraint::Length(1),
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
        "SETTINGS › PRAYER CONFIGURATION",
        Some(hint),
    );

    render_tabs(state, tabs_area, buf, theme, small);
    render_panel(model, state, prayer, panel_area, buf, theme, small, large);
    render_timeline(model, prayer, timeline_area, buf, theme, small, large);
    super::settings::render_buttons(buttons_area, buf, theme);
}

/// The prayer `tab` addresses, clamped — a stale cursor from a resize
/// or a hand-edited state must never index past the five.
fn tab_prayer(tab: usize) -> Notifiable {
    Notifiable::ALL
        .get(tab.min(Notifiable::ALL.len() - 1))
        .copied()
        .unwrap_or(Notifiable::Maghrib)
}

/// The prayer tab bar (§16.3 items 4, 9): short names at 80×24, the
/// active tab green on `rowActive` with a `▔▔▔` underline (a terminal
/// has no border-bottom), the others `textDim` over the bar's rule.
/// Row 0's focus brightens the active tab's label and marks it `▸`.
fn render_tabs(
    state: &PrayerConfigState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    small: bool,
) {
    if area.height < 2 || area.width < 10 {
        return;
    }
    let names: Vec<&str> = Notifiable::ALL
        .iter()
        .map(|p| if small { p.short() } else { p.display() })
        .collect();
    let label_widths: Vec<u16> = names.iter().map(|n| n.len() as u16 + 4).collect();
    let total: u16 = label_widths.iter().sum();
    if total > area.width {
        // Cannot happen at the breakpoints (53 ≤ 80), but a guard is
        // cheaper than a panic on a future size.
        return;
    }

    let mut x = area.x;
    for (index, name) in names.iter().enumerate() {
        let active = index == state.tab.min(Notifiable::ALL.len() - 1);
        let focused = active && state.row == 0;
        let width = label_widths[index];
        let ground = if active {
            Style::new().bg(theme.row_active)
        } else {
            Style::new()
        };
        // `  NAME  ` normally; the focused tab carries `▸ NAME ` — the
        // same width, so the underline stays aligned.
        let label = if focused {
            format!("▸ {name} ")
        } else {
            format!("  {name}  ")
        };
        let color = if focused {
            theme.text_bright
        } else if active {
            theme.green
        } else {
            theme.text_dim
        };
        Line::from(Span::styled(label, Style::new().fg(color).patch(ground)))
            .render(Rect::new(x, area.y, width, 1), buf);
        // The underline row: green under the active label, the bar's
        // rule continuing under the rest.
        Line::from(Span::styled(
            "▔".repeat(width as usize),
            Style::new().fg(if active { theme.green } else { theme.border }),
        ))
        .render(Rect::new(x, area.y + 1, width, 1), buf);
        x += width;
    }
    // The rule continues to the right edge.
    let rule_width = area.width.saturating_sub(total);
    if rule_width > 0 {
        Line::from(Span::styled(
            "─".repeat(rule_width as usize),
            Style::new().fg(theme.border),
        ))
        .render(Rect::new(area.x + total, area.y + 1, rule_width, 1), buf);
    }
}

/// The prayer's own panel (§16.3 items 4–6): title `MAGHRIB · 19:21`
/// in green over a `borderGrn` box, the two steppers, a rule, the
/// two toggle rows and the preset echo.
#[allow(clippy::too_many_arguments)]
fn render_panel(
    model: &UiModel,
    state: &PrayerConfigState,
    prayer: Notifiable,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    small: bool,
    large: bool,
) {
    if area.height < 3 {
        return;
    }
    let config = model.pending_config();
    let settings = prayer.settings(&config.prayers);
    // The time is today's actual instant — whatever the draft did to
    // the minutes, the title anchors the panel to the real clock.
    let time = model.window.today.time(prayer.as_prayer());
    let format = &model.config.display.time_format;
    let title = format!("{} · {}", prayer.display(), hhmm(time, format));
    let inner = Panel::new()
        .title(&title)
        .border(theme.border_grn)
        .title_color(theme.green)
        .render(area, buf, theme);

    // The panel's rows, top-to-bottom: the steppers, the rule, the
    // toggles, the preset (hidden at small, §16.3 item 9). Row 0 of
    // the cursor is the tab bar, so the panel's focused row is
    // `state.row - 1`.
    let panel_rows: &[Row] = if small {
        &[Row::Before, Row::After, Row::Adhan, Row::Notif]
    } else {
        &[Row::Before, Row::After, Row::Adhan, Row::Notif, Row::Preset]
    };
    let focused_row = state.row.saturating_sub(1);

    let mut y = inner.y;
    for row in panel_rows {
        if y >= inner.bottom() {
            break;
        }
        let focused = *row == Row::ALL[focused_row.min(Row::ALL.len() - 1)];
        let row_area = Rect::new(inner.x, y, inner.width, 1);
        let ground = if focused {
            Style::new().bg(theme.row_active)
        } else {
            Style::new()
        };
        buf.set_style(
            Rect::new(inner.x + 1, y, inner.width.saturating_sub(2), 1),
            ground,
        );

        match row {
            Row::Before | Row::After => {
                let (label, minutes, resolved) = if *row == Row::Before {
                    (
                        "Before Adhan notification",
                        settings.before_minutes,
                        time - Duration::minutes(i64::from(settings.before_minutes)),
                    )
                } else {
                    (
                        "After Adhan mosque reminder",
                        settings.after_minutes,
                        time + Duration::minutes(i64::from(settings.after_minutes)),
                    )
                };
                // Label left, stepper right (the design's
                // `space-between` StepCtrl); the resolved absolute
                // time rides after the stepper at 120×35 (§16.3
                // item 10).
                let columns = if large {
                    Layout::horizontal([
                        Constraint::Length(28),
                        Constraint::Length(13),
                        Constraint::Min(0),
                    ])
                    .areas(row_area)
                } else {
                    let areas: [Rect; 3] = Layout::horizontal([
                        Constraint::Length(28),
                        Constraint::Min(0),
                        Constraint::Min(0),
                    ])
                    .areas(row_area);
                    [areas[0], areas[1], Rect::ZERO]
                };
                Line::from(vec![
                    cursor_edge(focused, theme, ground),
                    Span::styled(label, Style::new().fg(theme.text_sub).patch(ground)),
                ])
                .render(Rect::new(columns[0].x, y, columns[0].width, 1), buf);
                Stepper::new(&format!("{minutes} min"))
                    .focused(focused)
                    .at_min(minutes == 0)
                    .at_max(minutes == 60)
                    .render(Rect::new(columns[1].x, y, columns[1].width, 1), buf, theme);
                if large {
                    Line::from(Span::styled(
                        format!("→ {}", hhmm(resolved, format)),
                        Style::new().fg(theme.text_dim).patch(ground),
                    ))
                    .render(Rect::new(columns[2].x, y, columns[2].width, 1), buf);
                }
            }
            Row::Adhan | Row::Notif => {
                let (label, on) = if *row == Row::Adhan {
                    ("Adhan sound", settings.adhan)
                } else {
                    ("Notifications", settings.notify)
                };
                SettingRow::new(label, if on { "ENABLED" } else { "DISABLED" })
                    .kind(if on {
                        ValueKind::Enabled
                    } else {
                        ValueKind::Disabled
                    })
                    .active(focused)
                    .compact(true)
                    .render(row_area, buf, theme);
            }
            Row::Preset => {
                // The preset's display echo — `◂ Abdul Basit ▸` in the
                // design. The value itself is owned by SETTINGS →
                // Sound preset (a free-form file name has no set to
                // walk, §16.2 item 7), so this row is display-only.
                SettingRow::new("Sound preset", &preset_display(&config.adhan.preset))
                    .active(focused)
                    .compact(true)
                    .render(row_area, buf, theme);
            }
            Row::Tab => {}
        }

        // The rule between the steppers and the toggles (the design's
        // `HR`), on its own row.
        if *row == Row::After {
            y += 1;
            if y < inner.bottom() {
                hr::render(
                    Rect::new(inner.x, y, inner.width, 1),
                    buf,
                    theme,
                    Some(theme.border_dim),
                );
            }
        }
        y += 1;
    }
}

/// The focused row's `▌` edge, blank otherwise — the same three-cell
/// lead-in [`SettingRow`] draws, so the manual stepper rows and the
/// setting rows stay aligned.
fn cursor_edge(focused: bool, theme: &Theme, ground: Style) -> Span<'static> {
    if focused {
        Span::styled(
            "▌› ".to_owned(),
            Style::new().fg(theme.border_hi).patch(ground),
        )
    } else {
        Span::raw("   ")
    }
}

/// The notification timeline (§16.3 items 4, 8, 10): `─15m─ ◆ NOTIFY
/// ── ◉ ADHAN 19:21 ── ◉ MOSQUE ─+15m` — the relative offsets being
/// edited, made visible as the distances they are. At 120×35 a second
/// line carries the clock labels beneath the markers.
#[allow(clippy::too_many_arguments)]
fn render_timeline(
    model: &UiModel,
    prayer: Notifiable,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    small: bool,
    large: bool,
) {
    if area.height < 3 {
        return;
    }
    let _ = small;
    let config = model.pending_config();
    let settings = prayer.settings(&config.prayers);
    let time = model.window.today.time(prayer.as_prayer());
    let format = &model.config.display.time_format;
    let notify_at = time - Duration::minutes(i64::from(settings.before_minutes));
    let mosque_at = time + Duration::minutes(i64::from(settings.after_minutes));

    let inner = Panel::new()
        .title("NOTIFICATION TIMELINE")
        .render(area, buf, theme);
    let line_y = inner.y + 1;

    // The single timeline line, with the marker positions tracked for
    // the large label line.
    let before = format!("─{}m─", settings.before_minutes);
    let adhan = format!("◉ ADHAN {}", hhmm(time, format));
    let after = format!("─+{}m", settings.after_minutes);
    let dash_run = "─".repeat(8);

    let mut spans = Vec::new();
    let mut x = 0usize;
    spans.push(Span::styled(
        before.clone(),
        Style::new().fg(theme.text_dim),
    ));
    x += before.width();
    let notify_x = x + 1;
    spans.push(Span::styled(
        " ◆ NOTIFY".to_owned(),
        Style::new().fg(theme.yellow),
    ));
    x += " ◆ NOTIFY".width();
    spans.push(Span::styled(
        format!(" {dash_run} "),
        Style::new().fg(theme.border_dim),
    ));
    x += dash_run.width() + 2;
    let adhan_x = x;
    spans.push(Span::styled(adhan.clone(), Style::new().fg(theme.red)));
    x += adhan.width();
    spans.push(Span::styled(
        format!(" {dash_run} "),
        Style::new().fg(theme.border_dim),
    ));
    x += dash_run.width() + 2;
    let mosque_x = x;
    spans.push(Span::styled(
        "◉ MOSQUE".to_owned(),
        Style::new().fg(theme.red),
    ));
    x += "◉ MOSQUE".width();
    spans.push(Span::styled(
        format!(" {after}"),
        Style::new().fg(theme.text_dim),
    ));
    let _ = x;

    Line::from(spans).render(
        Rect::new(inner.x + 1, line_y, inner.width.saturating_sub(2), 1),
        buf,
    );

    // The clock labels beneath the markers (§16.3 item 10, large).
    if large {
        let labels = [
            (notify_x, hhmm(notify_at, format)),
            (adhan_x, hhmm(time, format)),
            (mosque_x, hhmm(mosque_at, format)),
        ];
        for (offset, label) in labels {
            let x = inner.x + 1 + u16::try_from(offset).unwrap_or(u16::MAX);
            if x < inner.right() {
                Line::from(Span::styled(label, Style::new().fg(theme.text_dim)))
                    .render(Rect::new(x, line_y + 1, 8, 1), buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::testing;

    fn rendered(width: u16, height: u16, state: &PrayerConfigState) -> Vec<String> {
        let fixture = testing::snapshot_runtime();
        let model = crate::ui::model::build_model(&fixture.runtime);
        let breakpoint = Breakpoint::of_size(width, height);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height.saturating_sub(4)));
        render(
            &model,
            state,
            buf.area,
            &mut buf,
            &Theme::for_mode(crate::ui::theme::RenderMode::Truecolor),
            breakpoint,
        );
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_owned())
                    .collect()
            })
            .collect()
    }

    /// §16.3's snapshot: the screen at all three sizes.
    #[test]
    fn the_prayer_config_screen_snapshots() {
        let state = PrayerConfigState::default();
        for (width, height, name) in [(80, 24, "80x24"), (100, 30, "100x30"), (120, 35, "120x35")] {
            let text = rendered(width, height, &state).join("\n");
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
            settings.bind(|| insta::assert_snapshot!(format!("prayer_config_{name}"), text));
        }
    }

    /// §16.3 item 9: short tab names, no preset row, and the crumb's
    /// `presets in SETTINGS` pointer at 80×24.
    #[test]
    fn small_hides_the_preset_row_and_shortens_tabs() {
        let state = PrayerConfigState { tab: 3, row: 0 };
        let small = rendered(80, 24, &state).join("\n");
        // The tab line carries the five short names, never a full one.
        let tabs = small
            .lines()
            .find(|line| line.contains("FAJ"))
            .unwrap_or_default();
        assert!(
            tabs.contains("DHU") && tabs.contains("ISH"),
            "short tabs:\n{small}"
        );
        assert!(!tabs.contains("FAJR"), "no full names:\n{small}");
        assert!(
            small.contains("presets in SETTINGS"),
            "crumb hint:\n{small}"
        );
        // The preset row is gone.
        assert!(!small.contains("Sound preset"), "no preset row:\n{small}");

        let medium = rendered(100, 30, &state).join("\n");
        let tabs = medium
            .lines()
            .find(|line| line.contains("FAJR"))
            .unwrap_or_default();
        assert!(tabs.contains("MAGHRIB"), "full tabs:\n{medium}");
        assert!(medium.contains("Sound preset"), "preset row:\n{medium}");
    }

    /// §16.3 item 7's conflict resolution: row 0's `←→` is the tab
    /// switch, deeper rows adjust — the render side of the hint.
    #[test]
    fn row_zero_hints_tab_and_deeper_rows_hint_adjust() {
        let at_zero = rendered(100, 30, &PrayerConfigState { tab: 3, row: 0 }).join("\n");
        assert!(at_zero.contains("←→ tab"), "row 0 hint:\n{at_zero}");
        // The focused tab carries the ▸ marker.
        assert!(at_zero.contains("▸ MAGHRIB"), "focused tab:\n{at_zero}");
        let deeper = rendered(100, 30, &PrayerConfigState { tab: 3, row: 2 }).join("\n");
        assert!(deeper.contains("←→ adjust"), "deep row hint:\n{deeper}");
        assert!(!deeper.contains("←→ tab · ENTER"), "no tab hint:\n{deeper}");
        assert!(!deeper.contains("▸ MAGHRIB"), "tab unfocused:\n{deeper}");
    }

    /// The timeline shows the offsets being edited around the prayer's
    /// actual time, and the panel title carries that time (§16.3
    /// item 6).
    #[test]
    fn the_timeline_carries_the_offsets_and_time() {
        let state = PrayerConfigState::default(); // Maghrib: 5 before, 15 after
        let text = rendered(100, 30, &state).join("\n");
        assert!(text.contains("─5m─"), "before offset:\n{text}");
        assert!(text.contains("─+15m"), "after offset:\n{text}");
        assert!(text.contains("◆ NOTIFY"), "notify marker:\n{text}");
        assert!(text.contains("◉ ADHAN"), "adhan marker:\n{text}");
        assert!(text.contains("◉ MOSQUE"), "mosque marker:\n{text}");
        assert!(text.contains("MAGHRIB · "), "panel title:\n{text}");
    }
}
