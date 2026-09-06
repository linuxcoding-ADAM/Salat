//! The Dashboard screen — the design's home (§16.2). Phase 7 ships
//! the real layout: the NEXT PRAYER focal panel, the TODAY table with
//! its rail and NOTIF column, the greeting and TONIGHT blocks the
//! wider breakpoints add, and the service strip along the bottom.
//!
//! Geometry is §16.1 item 4 verbatim: small stacks the two panels;
//! medium and large split the body 45/55 with a three-row `bigtext`
//! countdown (A1's single-row form is the 80×24 form). The strip is
//! the dashboard's own last row — the design draws it inside this
//! screen, not in the chrome.

use chrono::{DateTime, Offset as _};
use chrono_tz::Tz;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr as _;

use crate::config::{PrayerSettings, PrayersConfig};
use crate::domain::{Notifiable, Prayer};
use crate::status::{ComponentId, Health};

use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::shell::DashboardState;
use crate::ui::theme::{Theme, hhmm, quick_name};
use crate::ui::widgets::bigtext;
use crate::ui::widgets::hr;
use crate::ui::widgets::panel::Panel;
use crate::ui::widgets::prayer_table::{self, Columns, TIME_WIDTH, TableSpec};
use crate::ui::widgets::{bar, status_dot, timeline};

/// The strip's four groups (§16.1 item 6): component, full label.
const STRIP_FULL: [(ComponentId, &str); 4] = [
    (ComponentId::PrayerService, "service"),
    (ComponentId::Notifications, "notifications"),
    (ComponentId::AdhanSound, "adhan"),
    (ComponentId::AudioSystem, "audio"),
];

/// The small strip's three short groups — the design's 80×24 strip
/// drops `audio` to fit (§16.1 item 9).
const STRIP_SHORT: [(ComponentId, &str); 3] = [
    (ComponentId::PrayerService, "svc"),
    (ComponentId::Notifications, "notif"),
    (ComponentId::AdhanSound, "adhan"),
];

/// One prayer's notification settings — the config's five named
/// fields behind one `Notifiable`-keyed lookup.
fn settings(prayers: &PrayersConfig, prayer: Notifiable) -> &PrayerSettings {
    match prayer {
        Notifiable::Fajr => &prayers.fajr,
        Notifiable::Dhuhr => &prayers.dhuhr,
        Notifiable::Asr => &prayers.asr,
        Notifiable::Maghrib => &prayers.maghrib,
        Notifiable::Isha => &prayers.isha,
    }
}

/// Whether the day is served from cache after a failed refresh — the
/// TODAY footer's `· cached` marker (§16.1 item 8).
fn service_degraded(model: &UiModel) -> bool {
    model
        .status
        .component(ComponentId::PrayerService)
        .is_some_and(|component| component.health == Health::Degraded)
}

/// `UTC+1` / `UTC+5:30` from the tick's own offset — the strip's
/// right-hand group, computed rather than configured so it can never
/// disagree with the clock the countdown runs on.
fn utc_offset(now: DateTime<Tz>) -> String {
    let seconds = now.offset().fix().local_minus_utc();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if minutes == 0 {
        format!("UTC{hours:+}")
    } else {
        format!("UTC{hours:+}:{minutes:02}")
    }
}

/// Write one clipped, styled string at `(x, y)` and return the next
/// column — the screens' shared truncation story (§16.14 item 10).
fn put(buf: &mut Buffer, x: u16, y: u16, limit: u16, text: &str, style: Style) -> u16 {
    if limit == 0 {
        return x;
    }
    let (next, _) = buf.set_stringn(x, y, text, limit as usize, style);
    next
}

/// A one-row string centred in `width` — the design's `textAlign:
/// center` blocks (the small panel, the countdown, REMAINING).
fn put_centered(buf: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    let text_width = text.width() as u16;
    let at = x + width.saturating_sub(text_width) / 2;
    put(buf, at, y, x + width - at, text, style);
}

/// A one-row string flush to the right edge of `width`.
fn put_right(buf: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    let text_width = text.width() as u16;
    let at = x + width.saturating_sub(text_width);
    put(buf, at, y, x + width - at, text, style);
}

/// A sub-rect of `area`'s row `y`, narrowed to `width` cells and
/// centred — how every fixed-width element (the gauges, the block
/// digits) sits mid-panel as the design centres them.
fn centred(area: Rect, y: u16, width: u16) -> Rect {
    let width = width.min(area.width);
    Rect::new(area.x + (area.width - width) / 2, y, width, 1)
}

/// Render the dashboard body (§16.1 item 4's layouts).
pub fn render(
    model: &UiModel,
    state: &DashboardState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match breakpoint {
        Breakpoint::Small => {
            let [next_area, today_area, strip_area] = Layout::vertical([
                Constraint::Length(6),
                Constraint::Min(11),
                Constraint::Length(1),
            ])
            .areas(area);
            next_prayer_small(model, next_area, buf, theme);
            todays_panel(model, state, today_area, buf, theme, Columns::Three, false);
            strip(model, strip_area, buf, theme, breakpoint);
        }
        Breakpoint::Medium | Breakpoint::Large => {
            let large = breakpoint == Breakpoint::Large;
            let [greeting_area, body_area, strip_area] = Layout::vertical([
                // The greeting row exists only at large; at medium the
                // constraint collapses to zero and hands the row back.
                Constraint::Length(u16::from(large)),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .areas(area);
            if large {
                greeting(model, greeting_area, buf, theme);
            }
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(45), Constraint::Min(0)])
                    .areas(body_area);
            next_prayer_wide(model, left, buf, theme, breakpoint);
            todays_panel(model, state, right, buf, theme, Columns::Four, large);
            strip(model, strip_area, buf, theme, breakpoint);
        }
        // The too-small guard (§16.13) never routes a frame here.
        Breakpoint::TooSmall => {}
    }
}

// ──────────────────────────────  panels  ───────────────────────────────

/// The greeting line (large only, §16.1 item 10):
/// `ASSALAMU ALAIKUM · Saturday · 23 Rabīʿ al-awwal 1448 AH`.
fn greeting(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let hijri = &model.window.today.hijri;
    let line = format!(
        "ASSALAMU ALAIKUM · {} · {} {} {} AH",
        model.window.today.date.format("%A"),
        hijri.day,
        hijri.month_en,
        hijri.year
    );
    put(
        buf,
        area.x,
        area.y,
        area.width,
        &line,
        Style::new().fg(theme.text_dim),
    );
}

/// The 80×24 NEXT PRAYER panel (§16.1 item 9): name row, A1's
/// single-row countdown, REMAINING, gauge — four inner rows.
fn next_prayer_small(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let inner = Panel::new()
        .title("NEXT PRAYER")
        .active(true)
        .render(area, buf, theme);
    if inner.height < 4 {
        return;
    }
    match model.next {
        Some(next) => {
            let name_row = format!(
                "● {} · {}",
                quick_name(next.prayer).to_uppercase(),
                hhmm(next.at, &model.config.display.time_format)
            );
            put_centered(
                buf,
                inner.x,
                inner.y,
                inner.width,
                &name_row,
                Style::new().fg(theme.green).add_modifier(Modifier::BOLD),
            );
            put_centered(
                buf,
                inner.x,
                inner.y + 1,
                inner.width,
                &model.countdown.format(),
                Style::new()
                    .fg(theme.text_bright)
                    .add_modifier(Modifier::BOLD),
            );
        }
        // The exhausted-window sliver: honest placeholders, never a
        // panic (§6.5).
        None => {
            let dim = Style::new().fg(theme.text_dim);
            put_centered(buf, inner.x, inner.y, inner.width, "● -- · --:--", dim);
            put_centered(buf, inner.x, inner.y + 1, inner.width, "--:--:--", dim);
        }
    }
    put_centered(
        buf,
        inner.x,
        inner.y + 2,
        inner.width,
        "REMAINING",
        Style::new().fg(theme.text_sub),
    );
    bar::render(
        centred(inner, inner.y + 3, 34),
        buf,
        theme,
        model.progress,
        34,
        None,
    );
}

/// The medium/large NEXT PRAYER panel (§16.1 item 10): the name and
/// SCHEDULED pair, the three-row `bigtext` countdown, the gauged
/// timeline with its legend, and the notification schedule under the
/// rule.
fn next_prayer_wide(
    model: &UiModel,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    let inner = Panel::new()
        .title("NEXT PRAYER")
        .active(true)
        .render(area, buf, theme);
    let format = &model.config.display.time_format;
    let mut y = inner.y;

    match model.next {
        Some(next) => {
            // The name block: MAGHRIB green bold over ● NEXT PRAYER,
            // SCHEDULED textDim over the bold time, flush right.
            put(
                buf,
                inner.x,
                y,
                inner.width,
                &quick_name(next.prayer).to_uppercase(),
                Style::new().fg(theme.green).add_modifier(Modifier::BOLD),
            );
            put_right(
                buf,
                inner.x,
                y,
                inner.width,
                "SCHEDULED",
                Style::new().fg(theme.text_dim),
            );
            if inner.height < 2 {
                return;
            }
            put(
                buf,
                inner.x,
                y + 1,
                inner.width,
                "● NEXT PRAYER",
                Style::new().fg(theme.green),
            );
            put_right(
                buf,
                inner.x,
                y + 1,
                inner.width,
                &hhmm(next.at, format),
                Style::new().fg(theme.text).add_modifier(Modifier::BOLD),
            );
            y += 2;

            // The countdown: three rows of block digits centred in
            // the panel (it has the room from 100×30 up), A1's
            // single-row form as the fallback.
            let countdown = model.countdown.format();
            let block = bigtext::width(&countdown);
            let x = inner.x + inner.width.saturating_sub(block) / 2;
            let digits = Rect::new(x, y, inner.right() - x, inner.bottom() - y);
            if bigtext::render(digits, buf, &countdown, theme.text_bright) {
                y += 3;
            } else {
                put(
                    buf,
                    inner.x,
                    y,
                    inner.width,
                    &countdown,
                    Style::new()
                        .fg(theme.text_bright)
                        .add_modifier(Modifier::BOLD),
                );
                y += 1;
            }
            put_centered(
                buf,
                inner.x,
                y,
                inner.width,
                "REMAINING",
                Style::new().fg(theme.text_sub),
            );
            y += 1;
        }
        None => {
            let dim = Style::new().fg(theme.text_dim);
            put(buf, inner.x, y, inner.width, "--", dim);
            put_right(buf, inner.x, y, inner.width, "SCHEDULED", dim);
            if inner.height < 2 {
                return;
            }
            put(buf, inner.x, y + 1, inner.width, "● NEXT PRAYER", dim);
            put_right(buf, inner.x, y + 1, inner.width, "--:--", dim);
            y += 2;
            put(
                buf,
                inner.x,
                y,
                inner.width,
                "--:--:--",
                Style::new().fg(theme.text_dim).add_modifier(Modifier::BOLD),
            );
            y += 1;
        }
    }

    if y >= inner.bottom() {
        return;
    }
    // The gauged timeline: bar row plus legend row, the width the
    // design pins per breakpoint.
    let gauge_width = match breakpoint {
        Breakpoint::Large => 40,
        _ => 32,
    };
    let previous = model
        .previous
        .map(|previous| (quick_name(previous.prayer), hhmm(previous.at, format)));
    let next = model
        .next
        .map(|next| (quick_name(next.prayer), hhmm(next.at, format)));
    let legend = timeline::Timeline {
        previous: previous.as_ref().map(|(name, time)| (*name, time.as_str())),
        next: next.as_ref().map(|(name, time)| (*name, time.as_str())),
    };
    let rows = (inner.bottom() - y).min(2);
    timeline::render(
        Rect::new(inner.x, y, inner.width, rows),
        buf,
        theme,
        model.progress,
        &legend,
        gauge_width,
    );
    y += rows;
    if y >= inner.bottom() {
        return;
    }
    hr::render(Rect::new(inner.x, y, inner.width, 1), buf, theme, None);
    y += 1;

    // The notification schedule: the next prayer's two configured
    // offsets, the yellow `at` instant straight from the summary.
    if y < inner.bottom()
        && let Some(next) = model.next
    {
        let configured = settings(&model.config.prayers, next.prayer);
        let notify = match model.summary.next_notify {
            Some(at) => format!(
                "{} min · at {}",
                configured.before_minutes,
                hhmm(at, format)
            ),
            None => "--".to_owned(),
        };
        put(
            buf,
            inner.x,
            y,
            inner.width,
            "Notify before",
            Style::new().fg(theme.text_dim),
        );
        put_right(
            buf,
            inner.x,
            y,
            inner.width,
            &notify,
            Style::new().fg(theme.yellow),
        );
        y += 1;
        if y < inner.bottom() {
            put(
                buf,
                inner.x,
                y,
                inner.width,
                "Mosque reminder",
                Style::new().fg(theme.text_dim),
            );
            put_right(
                buf,
                inner.x,
                y,
                inner.width,
                &format!("+{} min after", configured.after_minutes),
                Style::new().fg(theme.text_sub),
            );
        }
    }
}

/// The TODAY / TODAY'S PRAYERS panel (§16.1 items 9–10): the column
/// header, the table with its rail, and the Hijri · Qibla footer; at
/// large, the TONIGHT block follows under its own rule.
fn todays_panel(
    model: &UiModel,
    state: &DashboardState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    columns: Columns,
    tonight: bool,
) {
    let title = if columns == Columns::Three {
        "TODAY"
    } else {
        "TODAY'S PRAYERS"
    };
    let inner = Panel::new().title(title).render(area, buf, theme);
    let format = &model.config.display.time_format;
    let mut y = inner.y;
    let dim = Style::new().fg(theme.text_dim);

    // The header row: PRAYER/TIME/STATUS(+/NOTIF), aligned to the
    // columns the table renders — the name text starts three cells in
    // (rail + prefix) and spans NAME_WIDTH, the time column after it.
    let name_x = inner.x + 3;
    let time_x = name_x + prayer_table::NAME_WIDTH as u16;
    let status_x = time_x + TIME_WIDTH as u16;
    if y < inner.bottom() {
        put(buf, name_x, y, inner.width, "PRAYER", dim);
        put(buf, time_x, y, inner.width, "TIME", dim);
        put(buf, status_x, y, inner.width, "STATUS", dim);
        if columns == Columns::Four {
            put_right(buf, inner.x, y, inner.width, "NOTIF", dim);
        }
        y += 1;
    }
    if y < inner.bottom() {
        hr::render(Rect::new(inner.x, y, inner.width, 1), buf, theme, None);
        y += 1;
    }

    let rows: Vec<prayer_table::PrayerRow> = Notifiable::ALL
        .iter()
        .enumerate()
        .map(|(index, prayer)| {
            let is_next = model.next.is_some_and(|next| next.prayer == *prayer);
            prayer_table::PrayerRow {
                name: quick_name(*prayer).to_owned(),
                time: hhmm(model.window.today.time(prayer.as_prayer()), format),
                state: model.states[index],
                note: is_next.then(|| format!("· in {}", model.countdown.format())),
                notif: Some(format!(
                    "-{}m",
                    settings(&model.config.prayers, *prayer).before_minutes
                )),
            }
        })
        .collect();
    let table_rows = (inner.bottom() - y).min(rows.len() as u16);
    prayer_table::render(
        Rect::new(inner.x, y, inner.width, table_rows),
        buf,
        theme,
        &rows,
        &TableSpec::dashboard(columns, Some(state.cursor)),
    );
    y += table_rows;

    if y < inner.bottom() {
        hr::render(Rect::new(inner.x, y, inner.width, 1), buf, theme, None);
        y += 1;
    }

    // The footer: `Hijri: … · Qibla: …` (+ `· cached` when the day is
    // served from cache), the Qibla group omitted until a launch has
    // resolved the bearing.
    if y < inner.bottom() {
        let hijri = &model.window.today.hijri;
        let mut footer = format!("Hijri: {} {} {} AH", hijri.day, hijri.month_en, hijri.year);
        if let Some(degrees) = model.qibla {
            footer.push_str(&format!(" · Qibla: {:.1}°", degrees));
        }
        if service_degraded(model) {
            footer.push_str(" · cached");
        }
        put(buf, inner.x, y, inner.width, &footer, dim);
        y += 1;
    }

    // TONIGHT (large only): under its own rule, the design's two
    // rows — tonight's midnight and tomorrow's Fajr.
    if !tonight || y >= inner.bottom() {
        return;
    }
    hr::render(Rect::new(inner.x, y, inner.width, 1), buf, theme, None);
    y += 1;
    let sub = Style::new().fg(theme.text_sub);
    if y < inner.bottom() {
        put(buf, inner.x, y, inner.width, "TONIGHT", dim);
        y += 1;
    }
    let mut tonight_row = |label: &str, time: &str, y: u16| {
        put(buf, inner.x, y, inner.width, label, sub);
        put_right(buf, inner.x, y, inner.width, time, dim);
    };
    if y < inner.bottom() {
        tonight_row(
            "Midnight (est.)",
            &hhmm(model.window.today.midnight, format),
            y,
        );
        y += 1;
    }
    if y < inner.bottom() {
        tonight_row(
            "Fajr tomorrow",
            &hhmm(model.window.tomorrow.time(Prayer::Fajr), format),
            y,
        );
    }
}

/// The service strip (§16.1 item 10): the four labelled health dots —
/// three short ones at small — with `borderDim` separators at
/// medium/large, and at large the method · zone group flush right.
fn strip(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme, breakpoint: Breakpoint) {
    let end = area.x + area.width;
    let wide = matches!(breakpoint, Breakpoint::Medium | Breakpoint::Large);
    let label_style = Style::new().fg(theme.text_dim);
    let separator_style = Style::new().fg(theme.border_dim);
    let groups = if wide {
        &STRIP_FULL[..]
    } else {
        &STRIP_SHORT[..]
    };
    let mut x = area.x;
    for (index, (id, label)) in groups.iter().enumerate() {
        if index > 0 {
            if wide {
                x = put(buf, x, area.y, end - x, " ", separator_style);
                x = put(buf, x, area.y, end - x, "│", separator_style);
                x = put(buf, x, area.y, end - x, " ", separator_style);
            } else {
                x = put(buf, x, area.y, end - x, "  ", label_style);
            }
        }
        let health = model
            .status
            .component(*id)
            .map(|component| component.health)
            .unwrap_or(Health::Unknown);
        let dot = status_dot::span(health, theme);
        x = put(buf, x, area.y, end - x, dot.content.as_ref(), dot.style);
        x = put(buf, x, area.y, end - x, " ", label_style);
        x = put(buf, x, area.y, end - x, label, label_style);
    }
    let left_end = x;

    // The large strip's right group: the method the API actually used
    // (§6.2's honest echo), the zone name, and the computed offset —
    // dropped whole when it would touch the dots.
    if breakpoint == Breakpoint::Large {
        let meta = &model.window.today.meta;
        let right_text = format!(
            "{} · {} ({})",
            meta.method_name,
            meta.tz_name,
            utc_offset(model.now)
        );
        let right_x = end.saturating_sub(right_text.width() as u16);
        if right_x > left_end {
            put(
                buf,
                right_x,
                area.y,
                end - right_x,
                &right_text,
                label_style,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::testing;
    use crate::domain::PrayerState;

    /// The §20.3 fixture dashboard at one size, cursor on Fajr — the
    /// fresh shell's position — and the §17.1 Qibla bearing served.
    fn rendered(width: u16, height: u16) -> Buffer {
        let fixture = testing::snapshot_runtime();
        fixture.set_qibla(106.78);
        let model = crate::ui::model::build_model(&fixture.runtime);
        let state = DashboardState::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        render(
            &model,
            &state,
            buf.area,
            &mut buf,
            &Theme::for_mode(crate::ui::theme::RenderMode::Truecolor),
            Breakpoint::of_size(width, height),
        );
        buf
    }

    /// The fixture's distinct-state moment: 16:30 local with Fajr
    /// completed lands the five rows on Completed, Passed, PostAdhan
    /// (Asr's reminder window), Next and Upcoming — one of each.
    fn rendered_distinct() -> Buffer {
        let fixture = testing::snapshot_runtime();
        fixture.set_qibla(106.78);
        fixture.set_completions(vec![Notifiable::Fajr]);
        // 16:30 local — inside Asr's PostAdhan window (16:24:30…16:43),
        // before Maghrib's PreAdhan one.
        fixture.clock.set(crate::domain::testing::utc(
            crate::domain::testing::TODAY,
            16,
            30,
            0,
        ));
        let model = crate::ui::model::build_model(&fixture.runtime);
        let state = DashboardState::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 120, 35));
        render(
            &model,
            &state,
            buf.area,
            &mut buf,
            &Theme::for_mode(crate::ui::theme::RenderMode::Truecolor),
            Breakpoint::of_size(120, 35),
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

    fn text(buf: &Buffer) -> String {
        lines(buf).join("\n")
    }

    /// The 80×24 dashboard (§16.1 item 9): A1's single-row countdown,
    /// the three-column TODAY, the short strip — and none of the wide
    /// furniture.
    #[test]
    fn the_80x24_form() {
        let text = text(&rendered(80, 24));
        for word in [
            "NEXT PRAYER",
            "● MAGHRIB · 19:21",
            "01:42:37",
            "REMAINING",
            "TODAY",
            "▶ Maghrib",
            "Hijri: 23 Rabīʿ al-awwal 1448 AH",
            "Qibla: 106.8°",
        ] {
            assert!(text.contains(word), "missing {word:?} in:\n{text}");
        }
        assert!(!text.contains("NOTIF"), "3-column grid: {text}");
        assert!(!text.contains("ASSALAMU ALAIKUM"), "no greeting at small");
        assert!(!text.contains("TONIGHT"), "no TONIGHT at small");
        assert!(text.contains("svc"), "the short strip: {text}");
        assert!(!text.contains("audio"), "audio dropped at small: {text}");
    }

    /// The 100×30 dashboard (§16.1 item 10, medium): the bigtext
    /// countdown, the four-column table with its NOTIF cells, the
    /// notify schedule — but still no greeting or TONIGHT.
    #[test]
    fn the_100x30_form() {
        let text = text(&rendered(100, 30));
        for word in [
            "MAGHRIB",
            "● NEXT PRAYER",
            "SCHEDULED",
            "19:21",
            "REMAINING",
            "TODAY'S PRAYERS",
            "NOTIF",
            "▶ Maghrib",
            "-5m",
            "Notify before",
            "5 min · at 19:16",
            "Mosque reminder",
            "+15 min after",
        ] {
            assert!(text.contains(word), "missing {word:?} in:\n{text}");
        }
        assert!(!text.contains("ASSALAMU ALAIKUM"), "greeting is large-only");
        assert!(!text.contains("TONIGHT"), "TONIGHT is large-only");
    }

    /// The 120×35 dashboard (large): the greeting, TONIGHT and the
    /// strip's method · zone group join the medium form.
    #[test]
    fn the_120x35_form() {
        let text = text(&rendered(120, 35));
        for word in [
            "ASSALAMU ALAIKUM · Saturday · 23 Rabīʿ al-awwal 1448 AH",
            "TONIGHT",
            "Midnight (est.)",
            "00:37",
            "Fajr tomorrow",
            "05:10",
            "Union Organization Islamic de France · Africa/Algiers (UTC+1)",
            "notifications",
            "audio",
        ] {
            assert!(text.contains(word), "missing {word:?} in:\n{text}");
        }
    }

    /// §23 Phase 7's DoD: the five rows carry five distinct states, each
    /// distinguishable by glyph and word alone — colour is decoration,
    /// never the only difference.
    #[test]
    fn five_states_render_distinctly_without_colour() {
        let buf = rendered_distinct();
        let rows = lines(&buf);
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let expected = [
            (PrayerState::Completed, "Fajr"),
            (PrayerState::Passed, "Dhuhr"),
            (PrayerState::PostAdhan, "Asr"),
            (PrayerState::Next, "Maghrib"),
            (PrayerState::Upcoming, "Isha"),
        ];
        let mut badges = Vec::new();
        for (state, name) in expected {
            let (glyph, word, _) = theme.state_badge(state);
            let row = rows
                .iter()
                .find(|line| line.contains(name) && line.contains(word))
                .unwrap_or_else(|| panic!("{name} row with {word:?} missing"));
            assert!(row.contains(glyph), "{name}'s glyph {glyph:?}: {row:?}");
            badges.push((glyph, word));
        }
        let mut sorted = badges.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 5, "five distinct badges: {badges:?}");
    }

    /// The NOTIF column appears from 100 columns and never at 80
    /// (§16.1 items 9–10) — the same fixture, one breakpoint apart.
    #[test]
    fn the_notif_column_arrives_at_medium() {
        let narrow = text(&rendered(80, 24));
        let wide = text(&rendered(100, 30));
        assert!(!narrow.contains("-5m"), "80 columns: {narrow}");
        assert!(wide.contains("-5m"), "100 columns: {wide}");
    }

    /// Nothing overflows: the NOTIF cell sits flush against the right
    /// panel edge at both wide breakpoints, and the strip never runs
    /// past its own row.
    #[test]
    fn every_fixed_element_fits() {
        for (width, height) in [(100, 30), (120, 35)] {
            let buf = rendered(width, height);
            let rows = lines(&buf);
            let maghrib = rows
                .iter()
                .find(|line| line.contains("▶ Maghrib"))
                .unwrap_or_else(|| panic!("the next row at {width}x{height}"));
            assert!(
                maghrib.trim_end().ends_with("-5m│"),
                "NOTIF flush right at {width}: {maghrib:?}"
            );
        }
        // The strip's right group never overwrites the dots: at 120 the
        // two groups coexist on the bottom row.
        let buf = rendered(120, 35);
        let rows = lines(&buf);
        let strip = &rows[rows.len() - 1];
        assert!(strip.contains("service"), "the dots hold: {strip:?}");
        assert!(
            strip.contains("Africa/Algiers"),
            "the right group: {strip:?}"
        );
    }

    /// The three §16.1 snapshots, reviewed against the design.
    #[test]
    fn the_dashboard_snapshots() {
        for (name, width, height) in [
            ("dashboard_80x24", 80u16, 24u16),
            ("dashboard_100x30", 100, 30),
            ("dashboard_120x35", 120, 35),
        ] {
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
            settings.bind(|| {
                insta::assert_snapshot!(name, text(&rendered(width, height)));
            });
        }
    }
}
