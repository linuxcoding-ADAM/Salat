//! The SETTINGS screen (§16.2) — the single entry point to every
//! configurable value, four panels of fifteen rows in the design's
//! order and wording.
//!
//! The row model is one enum ([`Row`]) shared with the input side, so
//! the flat index `SettingsState.row` addresses and the key handler
//! switch on the same identity — no magic numbers cross the boundary.
//! Every value renders from `pending_config()`: the draft's working
//! copy once an edit has been made (§8.5), the effective config
//! otherwise.
//!
//! The 80×24 problem is the screen's defining constraint (§16.2 item
//! 9): twenty body rows cannot hold fifteen rows plus eight border
//! rows, so the panels must scroll — and they scroll **by whole
//! panels**: the viewport shows the earliest run of complete panels
//! that fits and contains the focused row's panel, so the fold never
//! cuts a panel in half and the `▲`/`▼` markers always sit in a panel
//! border, exactly as the section words it. The window is derived
//! from `row` alone each frame, so nothing extra lives in the state
//! and a snapshot with a pinned row is deterministic.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::Widget as _;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::config::{Config, TimeFormat};
use crate::domain::prayer::Notifiable;
use crate::status::{ComponentId, Health};
use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::shell::SettingsState;
use crate::ui::theme::Theme;
use crate::ui::widgets::crumb;
use crate::ui::widgets::panel::Panel;
use crate::ui::widgets::setting_row::{SettingRow, ValueKind};

use super::adhan_alert::preset_display;

/// One of the fifteen rows — the flat index `SettingsState.row`
/// addresses, in the design's order (GENERAL, NOTIFICATIONS, ADHAN,
/// DISPLAY). The input side switches on this identity rather than a
/// number, so inserting a row cannot silently rebind a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// GENERAL — the location, with its coordinates as the sub-label.
    Location,
    /// GENERAL — the AlAdhan calculation method id.
    Method,
    /// GENERAL — 24h or 12h.
    TimeFormat,
    /// GENERAL — English.
    Language,
    /// NOTIFICATIONS — the master switch.
    EnableNotifications,
    /// NOTIFICATIONS — the per-prayer grid, on the sub-screen.
    PerPrayerTiming,
    /// NOTIFICATIONS — the post-Adhan reminder.
    MosqueReminder,
    /// ADHAN — the master switch.
    EnableAdhan,
    /// ADHAN — which file `test-adhan` and the scheduler play.
    SoundPreset,
    /// ADHAN — playback volume, 0–100.
    Volume,
    /// ADHAN — play the configured Adhan now.
    TestAdhan,
    /// DISPLAY — Midnight Blue.
    Theme,
    /// DISPLAY — the denser layouts.
    CompactMode,
    /// DISPLAY — seconds in the clock and countdown.
    ShowSeconds,
    /// DISPLAY — the gauge under the countdown.
    CountdownProgress,
}

impl Row {
    /// The fifteen rows, in the design's order — the index order of
    /// `SettingsState.row`.
    pub const ALL: [Row; 15] = [
        Row::Location,
        Row::Method,
        Row::TimeFormat,
        Row::Language,
        Row::EnableNotifications,
        Row::PerPrayerTiming,
        Row::MosqueReminder,
        Row::EnableAdhan,
        Row::SoundPreset,
        Row::Volume,
        Row::TestAdhan,
        Row::Theme,
        Row::CompactMode,
        Row::ShowSeconds,
        Row::CountdownProgress,
    ];

    /// The row a flat index addresses.
    pub fn at(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
}

/// The AlAdhan method ids the config accepts (§8.4's set, 6 excluded),
/// each with the short name the row shows and the full name its
/// sub-label carries. The design's own short name for method 12 —
/// `Union of Islamic Orgs` — anchors the vocabulary.
const METHODS: [(u32, &str, &str); 24] = [
    (0, "Jafari", "Shia Ithna-Ashari"),
    (1, "Karachi", "University of Islamic Sciences, Karachi"),
    (2, "ISNA", "Islamic Society of North America"),
    (3, "MWL", "Muslim World League"),
    (4, "Umm al-Qura", "Umm al-Qura University, Makkah"),
    (5, "Egyptian", "Egyptian General Authority of Survey"),
    (7, "Tehran", "Institute of Geophysics, University of Tehran"),
    (8, "Gulf", "Gulf Region"),
    (9, "Kuwait", "Kuwait"),
    (10, "Qatar", "Qatar"),
    (11, "Singapore", "Majlis Ugama Islam Singapura"),
    (
        12,
        "Union of Islamic Orgs",
        "Union Organization Islamic de France",
    ),
    (13, "Diyanet", "Diyanet İşleri Başkanlığı, Turkey"),
    (
        14,
        "Russia",
        "Spiritual Administration of Muslims of Russia",
    ),
    (15, "Moonsighting", "Moonsighting Committee Worldwide"),
    (16, "Dubai", "Dubai"),
    (17, "JAKIM", "Jabatan Kemajuan Islam Malaysia"),
    (18, "Tunisia", "Tunisia"),
    (19, "Algeria", "Algeria"),
    (20, "KEMENAG", "Kementerian Agama, Indonesia"),
    (21, "Morocco", "Morocco"),
    (22, "Lisbon", "Comunidade Islamica de Lisboa"),
    (23, "Jordan", "Jordan"),
    (99, "Custom", "Custom angles"),
];

/// The short name a method id shows, `Custom` for an id the table does
/// not carry (the config's validation keeps those out; this is the
/// honest fallback, not a reachable state).
pub(crate) fn method_short(id: u32) -> &'static str {
    METHODS
        .iter()
        .find(|(method, _, _)| *method == id)
        .map(|(_, short, _)| *short)
        .unwrap_or("Custom")
}

/// The full name a method id's sub-label carries.
fn method_full(id: u32) -> &'static str {
    METHODS
        .iter()
        .find(|(method, _, _)| *method == id)
        .map(|(_, _, full)| *full)
        .unwrap_or("Custom angles")
}

/// The next/previous method id in the table — `None` when the current
/// id is not in it (nothing to cycle from, and nothing to cycle to).
pub(crate) fn cycle_method(id: u32, forward: bool) -> Option<u32> {
    let at = METHODS.iter().position(|(method, _, _)| *method == id)?;
    let next = if forward {
        (at + 1) % METHODS.len()
    } else {
        (at + METHODS.len() - 1) % METHODS.len()
    };
    Some(METHODS[next].0)
}

/// Render the settings body (§16.2): crumb, the scrolling panel stack,
/// the SAVE/CANCEL row.
pub fn render(
    model: &UiModel,
    state: &SettingsState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    let small = breakpoint == Breakpoint::Small;
    let large = breakpoint == Breakpoint::Large;
    let rows = row_views(model);

    let [crumb_area, viewport, buttons] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
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
        "SETTINGS",
        Some("↑↓ move · ENTER select · S save · ESC back"),
    );

    // §16.2 item 10: at 120×35 the four panels sit in two columns —
    // [GENERAL, ADHAN] left, [NOTIFICATIONS, DISPLAY] right — while
    // the flat index keeps the design's single-column order, so `↑↓`
    // runs down the left column and continues into the right.
    if large {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(viewport);
        render_column(
            &rows,
            state.row,
            &[("GENERAL", 0..4), ("ADHAN", 7..11)],
            left,
            buf,
            theme,
            small,
        );
        render_column(
            &rows,
            state.row,
            &[("NOTIFICATIONS", 4..7), ("DISPLAY", 11..15)],
            right,
            buf,
            theme,
            small,
        );
    } else {
        render_column(
            &rows,
            state.row,
            &[
                ("GENERAL", 0..4),
                ("NOTIFICATIONS", 4..7),
                ("ADHAN", 7..11),
                ("DISPLAY", 11..15),
            ],
            viewport,
            buf,
            theme,
            small,
        );
    }

    render_buttons(buttons, buf, theme);
}

/// One of the fifteen rows as a frame renders it — every value derived
/// from the pending configuration and the runtime's status, never
/// stored.
pub(crate) struct RowView {
    pub label: &'static str,
    pub value: String,
    pub kind: ValueKind,
    pub sub: Option<String>,
    pub action: bool,
    pub unavailable: bool,
}

/// Build the fifteen row views (§16.2 items 5–6): the draft's (or
/// config's) values, with the status components deciding the
/// audio rows' availability (item 8 — dimmed value, reason as the
/// sub-label, never a disappearing row).
pub(crate) fn row_views(model: &UiModel) -> Vec<RowView> {
    let config = model.pending_config();
    let down = |id: ComponentId| {
        matches!(
            model.status.component(id).map(|component| component.health),
            Some(Health::Degraded) | Some(Health::Down)
        )
    };
    let no_player = down(ComponentId::AudioSystem);
    let no_file = down(ComponentId::AdhanSound);

    let before: Vec<u8> = Notifiable::ALL
        .map(|prayer| prayer.settings(&config.prayers).before_minutes)
        .to_vec();
    let after: Vec<u8> = Notifiable::ALL
        .map(|prayer| prayer.settings(&config.prayers).after_minutes)
        .to_vec();

    let plain = |label: &'static str, value: String| RowView {
        label,
        value,
        kind: ValueKind::Plain,
        sub: None,
        action: false,
        unavailable: false,
    };

    vec![
        RowView {
            label: "Location",
            value: config.location.label.clone(),
            kind: ValueKind::Plain,
            sub: Some(coords_sub(config)),
            action: true,
            unavailable: false,
        },
        RowView {
            label: "Calculation method",
            value: method_short(config.calculation.method).to_owned(),
            kind: ValueKind::Plain,
            sub: Some(method_full(config.calculation.method).to_owned()),
            action: false,
            unavailable: false,
        },
        plain("Time format", time_format_value(config.display.time_format)),
        plain("Language", language_display(&config.display.language)),
        RowView {
            label: "Enable notifications",
            value: enabled_value(config.notifications.enabled),
            kind: if config.notifications.enabled {
                ValueKind::Enabled
            } else {
                ValueKind::Disabled
            },
            sub: None,
            action: true,
            unavailable: false,
        },
        RowView {
            label: "Per-prayer timing",
            value: "CONFIGURED".to_owned(),
            kind: ValueKind::Configured,
            sub: Some(range_sub(&before, "before")),
            action: true,
            unavailable: false,
        },
        RowView {
            label: "Mosque reminder",
            value: enabled_value(config.notifications.mosque_reminder),
            kind: if config.notifications.mosque_reminder {
                ValueKind::Enabled
            } else {
                ValueKind::Disabled
            },
            sub: Some(range_sub(&after, "after")),
            action: false,
            unavailable: false,
        },
        RowView {
            label: "Enable adhan",
            value: enabled_value(config.adhan.enabled),
            kind: if config.adhan.enabled {
                ValueKind::Enabled
            } else {
                ValueKind::Disabled
            },
            sub: None,
            action: false,
            unavailable: false,
        },
        plain("Sound preset", preset_display(&config.adhan.preset)),
        RowView {
            label: "Volume",
            value: format!("{}%", config.adhan.volume),
            kind: ValueKind::Plain,
            sub: no_player.then(|| "no player found".to_owned()),
            action: false,
            unavailable: no_player,
        },
        RowView {
            label: "Test adhan",
            value: "[ PLAY ]".to_owned(),
            kind: ValueKind::Action,
            sub: if no_player {
                Some("no player found".to_owned())
            } else if no_file {
                Some("no adhan file".to_owned())
            } else {
                None
            },
            action: true,
            unavailable: no_player || no_file,
        },
        plain("Theme", preset_display(&config.display.theme)),
        plain("Compact mode", on_off(config.display.compact_mode)),
        plain("Show seconds", on_off(config.display.show_seconds)),
        plain(
            "Countdown progress",
            on_off(config.display.countdown_progress),
        ),
    ]
}

/// The coordinates sub-label, the design's `36.19°N · 5.41°E` with the
/// hemispheres earned by the signs.
fn coords_sub(config: &Config) -> String {
    let latitude = config.location.latitude;
    let longitude = config.location.longitude;
    let ns = if latitude >= 0.0 { "N" } else { "S" };
    let ew = if longitude >= 0.0 { "E" } else { "W" };
    format!("{:.2}°{ns} · {:.2}°{ew}", latitude.abs(), longitude.abs())
}

/// `5–15 min before` — the range across the five prayers, collapsed
/// when it is not a range.
fn range_sub(values: &[u8], what: &str) -> String {
    let Some(min) = values.iter().min() else {
        return String::new();
    };
    let max = values.iter().max().unwrap_or(min);
    if min == max {
        format!("{min} min {what}")
    } else {
        format!("{min}–{max} min {what}")
    }
}

/// The design's value vocabulary.
fn enabled_value(enabled: bool) -> String {
    if enabled { "ENABLED" } else { "DISABLED" }.to_owned()
}

/// The display rows' `ON`/`OFF` — plain, not green/red, exactly as the
/// design colours them.
fn on_off(on: bool) -> String {
    if on { "ON" } else { "OFF" }.to_owned()
}

fn time_format_value(format: TimeFormat) -> String {
    match format {
        TimeFormat::TwentyFourHour => "24h",
        TimeFormat::TwelveHour => "12h",
    }
    .to_owned()
}

/// `en` → `English`; an unknown code shows itself rather than a guess.
fn language_display(code: &str) -> String {
    match code {
        "en" => "English".to_owned(),
        other => other.to_owned(),
    }
}

/// One column of panels: the earliest run of whole panels that fits
/// the viewport and contains the focused row's panel (§16.2 item 9's
/// scrolling region). Scrolling by whole panels is what keeps the
/// `▲`/`▼` markers in a border — a fold that cut a panel mid-content
/// would have to overwrite a value to mark itself, and the section
/// puts the markers in the border.
fn render_column(
    rows: &[RowView],
    focused: usize,
    panels: &[(&str, std::ops::Range<usize>)],
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    small: bool,
) {
    if area.width < 8 || area.height < 3 {
        return;
    }

    // The widgets first — their own `height` is the one source of
    // truth for how tall each row is.
    let built: Vec<Vec<SettingRow<'_>>> = panels
        .iter()
        .map(|(_, range)| {
            range
                .clone()
                .map(|index| {
                    let view = &rows[index];
                    let mut widget = SettingRow::new(view.label, &view.value)
                        .kind(view.kind)
                        .action(view.action)
                        .active(index == focused)
                        .compact(small)
                        .unavailable(view.unavailable);
                    if let Some(sub) = view.sub.as_deref() {
                        widget = widget.subtitle(sub);
                    }
                    widget
                })
                .collect()
        })
        .collect();

    // Each panel: 2 border rows plus its rows.
    let heights: Vec<u16> = built
        .iter()
        .map(|panel| 2 + panel.iter().map(|widget| widget.height()).sum::<u16>())
        .collect();

    // The focused row's panel, if it renders in this column — in the
    // large layout the other column's rows leave every panel shown.
    let focus_panel = panels
        .iter()
        .position(|(_, range)| range.contains(&focused));

    // Can the run starting at `s` reach panel `p` whole, within the
    // viewport?
    let reaches = |start: usize, focus: usize| {
        let mut used = 0u16;
        for height in &heights[start..=focus] {
            used += *height;
            if used > area.height {
                return false;
            }
        }
        true
    };
    // The earliest such start is the minimal scroll: everything above
    // that still fits stays on screen. No start reaches the panel only
    // when the panel alone overflows the viewport, which no legal
    // breakpoint produces — falling back to the panel itself clips
    // rows rather than vanishing.
    let start = match focus_panel {
        Some(focus) => (0..=focus).find(|&s| reaches(s, focus)).unwrap_or(focus),
        None => 0,
    };
    // The run extends while whole panels still fit.
    let mut end = start;
    let mut used = 0u16;
    while end < panels.len() && used + heights[end] <= area.height {
        used += heights[end];
        end += 1;
    }

    let mut y = area.y;
    for (index, (title, _)) in panels.iter().enumerate().skip(start).take(end - start) {
        let panel_area = Rect::new(area.x, y, area.width, heights[index]);
        let inner = Panel::new().title(title).render(panel_area, buf, theme);
        let mut row_y = inner.y;
        for widget in &built[index] {
            widget.render(
                Rect::new(inner.x, row_y, inner.width, widget.height()),
                buf,
                theme,
            );
            row_y += widget.height();
        }
        y += heights[index];
    }

    // §16.2 item 9's `▲`/`▼` markers, in the fold's borders — whole
    // panels guarantee the rows they land on are borders.
    if start > 0 {
        marker(buf, area, theme, area.y, "▲");
    }
    if end < panels.len() {
        marker(buf, area, theme, y - 1, "▼");
    }
}

/// One scroll marker, overwriting a border row's last interior cell.
/// `y` is absolute — both call sites hold rows already positioned in
/// `buf`.
fn marker(buf: &mut Buffer, area: Rect, theme: &Theme, y: u16, glyph: &str) {
    if area.width < 2 {
        return;
    }
    if let Some(cell) = buf.cell_mut((area.right() - 2, y)) {
        cell.set_symbol(glyph);
        cell.set_style(Style::new().fg(theme.border_hi));
    }
}

/// The SAVE/CANCEL row (§16.2 item 7's `s`/`Esc`, as buttons) — the
/// design's identical footer on the NOTIF SETTINGS and PRAYERS
/// screens, so they share it.
pub(crate) fn render_buttons(area: Rect, buf: &mut Buffer, theme: &Theme) {
    let save = "[S] SAVE";
    let cancel = "[ESC] CANCEL";
    let gap = "   ";
    let width = (save.chars().count() + gap.len() + cancel.chars().count()) as u16;
    if width > area.width {
        return;
    }
    let x = area.x + (area.width - width) / 2;
    Line::from(vec![
        Span::styled(save, Style::new().fg(theme.green)),
        Span::raw(gap),
        Span::styled(cancel, Style::new().fg(theme.text_sub)),
    ])
    .render(Rect::new(x, area.y, width, 1), buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::testing;
    use crate::config::draft::ConfigDraft;

    fn rendered(width: u16, height: u16, state: &SettingsState) -> Vec<String> {
        let fixture = testing::snapshot_runtime();
        let model = crate::ui::model::build_model(&fixture.runtime);
        render_model(&model, width, height, state)
    }

    /// Render into the body the app hands the screen — the frame less
    /// the chrome's 2 + 2 rows (§3.4) — because the scroll region's
    /// budget is the body's twenty rows at 80×24, not the frame's
    /// twenty-four.
    fn render_model(
        model: &UiModel,
        width: u16,
        height: u16,
        state: &SettingsState,
    ) -> Vec<String> {
        let breakpoint = Breakpoint::of_size(width, height);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height.saturating_sub(4)));
        render(
            model,
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

    /// §16.2's snapshot: the screen at all three sizes, cursor on the
    /// first row.
    #[test]
    fn the_settings_screen_snapshots() {
        let state = SettingsState::default();
        for (width, height, name) in [(80, 24, "80x24"), (100, 30, "100x30"), (120, 35, "120x35")] {
            let text = rendered(width, height, &state).join("\n");
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
            settings.bind(|| insta::assert_snapshot!(format!("settings_{name}"), text));
        }
    }

    /// §16.2 item 9: at 80×24 the stack scrolls to keep the focused
    /// row visible, with the `▲`/`▼` markers in the fold's edges. The
    /// last row forces the scroll and drops the `▼` (the fold's bottom
    /// is the stack's bottom); row 0 shows the `▼` alone.
    #[test]
    fn the_scroll_keeps_the_cursor_visible() {
        let last = SettingsState {
            row: Row::ALL.len() - 1,
        };
        let text = rendered(80, 24, &last).join("\n");
        assert!(
            text.contains("Countdown progress"),
            "last row visible:\n{text}"
        );
        assert!(text.contains("▲"), "clipped-top marker:\n{text}");
        assert!(!text.contains("▼"), "bottom is the stack's bottom:\n{text}");

        let first = SettingsState::default();
        let text = rendered(80, 24, &first).join("\n");
        assert!(text.contains("Location"), "first row visible:\n{text}");
        assert!(text.contains("▼"), "clipped-bottom marker:\n{text}");
        assert!(!text.contains("▲"), "top is the stack's top:\n{text}");
    }

    /// Every value renders from the draft's working copy once one is
    /// open (§8.5) — a bumped volume and a flipped time format show
    /// as the pending values, not the saved ones, and the saved ones
    /// are gone.
    #[test]
    fn draft_values_render() {
        let fixture = testing::snapshot_runtime();
        let mut model = crate::ui::model::build_model(&fixture.runtime);
        let mut draft = ConfigDraft::from(model.pending_config());
        draft.working().adhan.volume = 85;
        draft.working().display.time_format = TimeFormat::TwelveHour;
        model.draft = Some(draft);

        let state = SettingsState::default();
        let text = render_model(&model, 100, 30, &state).join("\n");
        assert!(text.contains("85%"), "draft volume:\n{text}");
        assert!(!text.contains("80%"), "the saved volume is gone:\n{text}");
        assert!(text.contains("12h"), "draft time format:\n{text}");
        assert!(
            !text.contains("24h"),
            "the saved time format is gone:\n{text}"
        );
    }

    /// All fifteen labels in the design's vocabulary render at the
    /// large size — the one breakpoint where every panel is visible
    /// at once (at 100×30 the stacked panels with sub-labels run 27
    /// rows against a 24-row viewport, so DISPLAY sits below the
    /// fold until the cursor walks down).
    #[test]
    fn all_fifteen_rows_render() {
        let state = SettingsState::default();
        let text = rendered(120, 35, &state).join("\n");
        for label in Row::ALL.iter().map(|row| row_views_label(row)) {
            assert!(text.contains(label), "missing {label}:\n{text}");
        }
    }

    fn row_views_label(row: &Row) -> &'static str {
        match row {
            Row::Location => "Location",
            Row::Method => "Calculation method",
            Row::TimeFormat => "Time format",
            Row::Language => "Language",
            Row::EnableNotifications => "Enable notifications",
            Row::PerPrayerTiming => "Per-prayer timing",
            Row::MosqueReminder => "Mosque reminder",
            Row::EnableAdhan => "Enable adhan",
            Row::SoundPreset => "Sound preset",
            Row::Volume => "Volume",
            Row::TestAdhan => "Test adhan",
            Row::Theme => "Theme",
            Row::CompactMode => "Compact mode",
            Row::ShowSeconds => "Show seconds",
            Row::CountdownProgress => "Countdown progress",
        }
    }
}
