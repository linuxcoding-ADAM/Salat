//! The LOCATION screen (§16.6) — pushed from Settings: the current
//! location's six facts, the three mode buttons, and the active mode's
//! panel — city search against the embedded index (§9.2), manual
//! coordinate entry with inline validation (§9.3's E101), or the
//! AUTO-DETECT proposal from the system timezone chain (§9.3).
//!
//! Rendering is pure in the state: the search list is whatever the
//! input handler last searched, the coordinate error is derived by
//! parsing the buffers again at draw time, and the AUTO proposal was
//! computed once on entering that mode. Nothing here writes anything —
//! committing is the input side's job, into the settings draft (§8.5).

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::Widget as _;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::location::cities;
use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::shell::{LocationMode, LocationState};
use crate::ui::theme::{Blink, Theme, blink};
use crate::ui::widgets::crumb;
use crate::ui::widgets::panel::Panel;

/// Render the location body (§16.6): crumb, CURRENT LOCATION, the
/// mode buttons, the mode panel.
#[allow(clippy::too_many_lines)]
pub fn render(
    model: &UiModel,
    state: &LocationState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    let small = breakpoint == Breakpoint::Small;
    let [crumb_area, current, buttons, mode] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(8),
        Constraint::Length(3),
        Constraint::Min(0),
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
        "SETTINGS › LOCATION",
        Some("esc back"),
    );

    render_current(model, inset(current), buf, theme);
    render_buttons(state, inset(buttons), buf, theme, small);
    match state.mode {
        LocationMode::Search => render_search(model, state, inset(mode), buf, theme, small),
        LocationMode::Coords => render_coords(model, state, inset(mode), buf, theme, small),
        LocationMode::Auto => render_auto(state, inset(mode), buf, theme),
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

/// The CURRENT LOCATION panel (§16.6 item 4): the six facts of the
/// configuration being edited — the draft's, once one is open (§8.5)
/// — with the coordinates green and the method the served window
/// carries.
fn render_current(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let Some(inner) = panel_inner(Panel::new().title("CURRENT LOCATION"), area, buf, theme, 8)
    else {
        return;
    };
    let location = &model.pending_config().location;
    let rows: [(&str, String, bool); 6] = [
        ("City", location.label.clone(), false),
        ("Country", location.country.clone(), false),
        ("Latitude", format!("{:.4}", location.latitude), true),
        ("Longitude", format!("{:.4}", location.longitude), true),
        ("Timezone", location.timezone.clone(), false),
        ("Method", model.window.today.meta.method_name.clone(), false),
    ];
    let value_x = 12usize; // past the longest label plus a blank, `Longitude`
    for (index, (label, value, green)) in rows.iter().enumerate() {
        let y = inner.y + index as u16;
        if y >= inner.bottom() {
            break;
        }
        let color = if *green { theme.green } else { theme.text };
        let budget = inner.width.saturating_sub(value_x as u16) as usize;
        Line::from(vec![
            Span::raw("  "),
            Span::styled(*label, Style::new().fg(theme.text_sub)),
            Span::raw(" ".repeat(value_x.saturating_sub(label.chars().count() + 2))),
            Span::styled(truncate(value, budget), Style::new().fg(color)),
        ])
        .render(Rect::new(inner.x, y, inner.width, 1), buf);
    }
}

/// The mode buttons (§16.6 item 5): three bracketed toggles, the
/// active one on `rowSel`; short labels at 80×24, the design's full
/// labels from 100×30.
fn render_buttons(state: &LocationState, area: Rect, buf: &mut Buffer, theme: &Theme, small: bool) {
    let Some(inner) = panel_inner(Panel::new(), area, buf, theme, 1) else {
        return;
    };
    let modes = [
        (
            if small { "SEARCH" } else { "SEARCH CITY" },
            LocationMode::Search,
        ),
        (
            if small { "COORDS" } else { "COORDINATES" },
            LocationMode::Coords,
        ),
        (
            if small { "AUTO" } else { "AUTO-DETECT" },
            LocationMode::Auto,
        ),
    ];
    let mut spans = Vec::new();
    for (index, (label, mode)) in modes.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        let style = if state.mode == *mode {
            Style::new().fg(theme.text_bright).bg(theme.row_sel)
        } else {
            Style::new().fg(theme.text_sub)
        };
        spans.push(Span::styled(format!("[ {label} ]"), style));
    }
    let line = Line::from(spans);
    let width = u16::try_from(line.width()).unwrap_or(inner.width);
    if width <= inner.width {
        let x = inner.x + (inner.width - width) / 2;
        line.render(Rect::new(x, inner.y, width, 1), buf);
    }
}

/// The SEARCH panel (§16.6 item 6): the query line with its blinking
/// cursor, then up to three results — `○`/`●` for the selection, the
/// population and zone right-aligned from 100×30.
fn render_search(
    model: &UiModel,
    state: &LocationState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    small: bool,
) {
    let Some(inner) = panel_inner(Panel::new(), area, buf, theme, 4) else {
        return;
    };
    let cursor = blink_cursor(model);
    Line::from(vec![
        Span::raw("  "),
        Span::styled("> ", Style::new().fg(theme.text_sub)),
        Span::styled(
            truncate(&state.query, inner.width.saturating_sub(6) as usize),
            Style::new().fg(theme.text_bright),
        ),
        Span::styled(cursor, Style::new().fg(theme.green)),
    ])
    .render(Rect::new(inner.x, inner.y, inner.width, 1), buf);

    let y = inner.y + 2;

    if let Err(error) = cities::load() {
        tracing::warn!(target: "salat::ui", ?error, "the city index is unusable");
        put_line(
            buf,
            inner,
            y,
            vec![error_line("city index unavailable", "E001", theme)],
        );
        return;
    }

    if state.query.is_empty() {
        put_line(
            buf,
            inner,
            y,
            vec![Span::styled(
                "type a city name",
                Style::new().fg(theme.text_dim),
            )],
        );
        return;
    }

    if state.results.is_empty() {
        // §16.6 item 6's own wording, the query verbatim in quotes.
        let text = format!(
            "no cities matched \"{}\" · try [ COORDINATES ]",
            truncate(&state.query, (inner.width as usize).saturating_sub(40))
        );
        put_line(
            buf,
            inner,
            y,
            vec![Span::styled(text, Style::new().fg(theme.text_sub))],
        );
        return;
    }

    let first_row = y;
    for (index, city) in state.results.iter().enumerate() {
        let y = first_row.saturating_add(u16::try_from(index).unwrap_or(u16::MAX));
        if y >= inner.bottom() {
            break;
        }
        let selected = index == state.sel;
        let ground = if selected {
            Style::new().bg(theme.row_sel)
        } else {
            Style::new()
        };
        let marker = if selected { "●" } else { "○" };
        let mut spans = vec![
            Span::raw("  "),
            Span::styled(marker, Style::new().fg(theme.text_sub).patch(ground)),
            Span::raw(" "),
            Span::styled(
                city.label(),
                Style::new().fg(theme.text_bright).patch(ground),
            ),
        ];
        if !small {
            // The design's right-aligned hint: `288k · Africa/Algiers`.
            let hint = format!("{} · {}", population(city.population), city.tz);
            let used = 4 + city.label().chars().count();
            let hint_width = hint.chars().count();
            if inner.width as usize > used + hint_width + 2 {
                spans.push(Span::styled(
                    " ".repeat(inner.width as usize - used - hint_width),
                    ground,
                ));
                spans.push(Span::styled(
                    hint,
                    Style::new().fg(theme.text_sub).patch(ground),
                ));
            }
        }
        buf.set_style(Rect::new(inner.x, y, inner.width, 1), ground);
        put_line(buf, inner, y, spans);
    }
}

/// The COORDINATES panel (§16.6 item 6): LAT/LNG buffers — stacked at
/// 80×24, side by side from 100×30 — the inline range error in red
/// while the buffers parse to out-of-range values, and the nearest
/// city's honesty line once they are valid.
fn render_coords(
    model: &UiModel,
    state: &LocationState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    small: bool,
) {
    // Five rows: the box plus both stacked inputs at small — the
    // footer clips first, the inputs never.
    let Some(inner) = panel_inner(Panel::new(), area, buf, theme, 5) else {
        return;
    };
    let cursor = blink_cursor(model);
    if small {
        coord_input(
            Rect::new(inner.x, inner.y, inner.width, 1),
            buf,
            theme,
            "LAT",
            &state.lat_buf,
            state.focus == crate::ui::shell::CoordFocus::Lat,
            &cursor,
        );
        coord_input(
            Rect::new(inner.x, inner.y + 2, inner.width, 1),
            buf,
            theme,
            "LNG",
            &state.lng_buf,
            state.focus == crate::ui::shell::CoordFocus::Lng,
            &cursor,
        );
        coord_footer(
            inner.y + 4,
            &state.lat_buf,
            &state.lng_buf,
            inner,
            buf,
            theme,
        );
    } else {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(inner);
        coord_input(
            Rect::new(left.x, left.y, left.width, 1),
            buf,
            theme,
            "LAT",
            &state.lat_buf,
            state.focus == crate::ui::shell::CoordFocus::Lat,
            &cursor,
        );
        coord_input(
            Rect::new(right.x, right.y, right.width, 1),
            buf,
            theme,
            "LNG",
            &state.lng_buf,
            state.focus == crate::ui::shell::CoordFocus::Lng,
            &cursor,
        );
        coord_footer(
            inner.y + 2,
            &state.lat_buf,
            &state.lng_buf,
            inner,
            buf,
            theme,
        );
    }
}

/// One coordinate input line: `LAT  36.19█` when focused, the whole
/// line on `rowSel`.
fn coord_input(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    label: &str,
    buffer: &str,
    focused: bool,
    cursor: &str,
) {
    let ground = if focused {
        Style::new().bg(theme.row_sel)
    } else {
        Style::new()
    };
    let label_color = if focused {
        theme.text_bright
    } else {
        theme.text_sub
    };
    let budget = area.width.saturating_sub(8) as usize;
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(label, Style::new().fg(label_color).patch(ground)),
        Span::styled("  ", ground),
        Span::styled(
            truncate(buffer, budget),
            Style::new().fg(theme.text).patch(ground),
        ),
    ];
    if focused {
        spans.push(Span::styled(
            cursor,
            Style::new().fg(theme.green).patch(ground),
        ));
    }
    buf.set_style(area, ground);
    Line::from(spans).render(area, buf);
}

/// The coordinates' third row: the red range error (§9.3's E101,
/// inline — never a screen change) or, once both buffers parse inside
/// their ranges, the nearest city and its distance.
fn coord_footer(
    y: u16,
    lat_buf: &str,
    lng_buf: &str,
    inner: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    if y >= inner.bottom() {
        return;
    }
    let area = Rect::new(inner.x, y, inner.width, 1);
    if let Some(error) = coords_error(lat_buf, lng_buf) {
        Line::from(error_line(error, "E101", theme)).render(area, buf);
        return;
    }
    let (lat, lng) = (
        lat_buf.parse::<f64>().unwrap_or_default(),
        lng_buf.parse::<f64>().unwrap_or_default(),
    );
    if let Ok(index) = cities::load()
        && let Some((city, km)) = index.nearest(lat, lng)
    {
        Line::from(Span::styled(
            format!("nearest: {}, {} km", city.name, km.round() as u64),
            Style::new().fg(theme.text_dim),
        ))
        .render(area, buf);
    }
}

/// The AUTO-DETECT panel (§16.6 item 6): the proposal computed on
/// entering the mode — the detected zone and its largest city, `enter
/// to accept` — or E103's inline wording when the chain (§9.3) came
/// back empty.
fn render_auto(state: &LocationState, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let Some(inner) = panel_inner(Panel::new(), area, buf, theme, 4) else {
        return;
    };
    match &state.auto {
        Some(Ok(city)) => {
            put_line(
                buf,
                inner,
                inner.y,
                vec![
                    Span::raw("  "),
                    Span::styled("● ", Style::new().fg(theme.text_sub)),
                    Span::styled(city.label(), Style::new().fg(theme.text_bright)),
                ],
            );
            put_line(
                buf,
                inner,
                inner.y + 1,
                vec![
                    Span::raw("     "),
                    Span::styled(
                        format!("{} · {}", city.tz, population(city.population)),
                        Style::new().fg(theme.text_sub),
                    ),
                ],
            );
            put_line(
                buf,
                inner,
                inner.y + 3,
                vec![Span::styled(
                    "enter to accept",
                    Style::new().fg(theme.text_dim),
                )],
            );
        }
        Some(Err(message)) => {
            put_line(
                buf,
                inner,
                inner.y,
                vec![error_line(message, "E103", theme)],
            );
        }
        // Not yet computed — the input side computes on entry; render
        // stays pure and simply shows nothing.
        None => {}
    }
}

/// One left-aligned line of a panel's interior, clipped at the bottom.
fn put_line(buf: &mut Buffer, inner: Rect, y: u16, spans: Vec<Span<'_>>) {
    if y < inner.bottom() {
        Line::from(spans).render(Rect::new(inner.x, y, inner.width, 1), buf);
    }
}

/// Draw a panel that needs at least `min_height` rows including its
/// borders, returning its interior — `None` when the area cannot hold
/// the box. Content beyond the interior's height clips, not crashes:
/// the small mode panel shows fewer results, never none.
fn panel_inner<'p>(
    panel: Panel<'p>,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    min_height: u16,
) -> Option<Rect> {
    if area.height < min_height.max(3) || area.width < 12 {
        return None;
    }
    Some(panel.render(area, buf, theme))
}

/// The blinking block cursor (§3.2's 560 ms cadence).
fn blink_cursor(model: &UiModel) -> String {
    if Blink::new(blink::CURSOR_MS).on(model.now) {
        "█".to_owned()
    } else {
        " ".to_owned()
    }
}

/// One red inline error line: `message · CODE`.
fn error_line<'a>(message: &'a str, code: &'a str, theme: &Theme) -> Span<'a> {
    Span::styled(format!("  {message} · {code}"), Style::new().fg(theme.red))
}

/// GeoNames populations the design renders as `288k`.
fn population(pop: u32) -> String {
    if pop >= 1_000_000 {
        format!("{:.1}m", f64::from(pop) / 1e6)
    } else if pop >= 1_000 {
        format!("{}k", pop / 1_000)
    } else {
        pop.to_string()
    }
}

/// The inline coordinate error (§9.3's E101), derived by parsing the
/// buffers — no error flag in the state to go stale.
pub(crate) fn coords_error(lat_buf: &str, lng_buf: &str) -> Option<&'static str> {
    match lat_buf.parse::<f64>() {
        Err(_) => Some("latitude must be a number"),
        Ok(lat) if !(-90.0..=90.0).contains(&lat) => Some("latitude must be between -90 and 90"),
        Ok(_) => match lng_buf.parse::<f64>() {
            Err(_) => Some("longitude must be a number"),
            Ok(lng) if !(-180.0..=180.0).contains(&lng) => {
                Some("longitude must be between -180 and 180")
            }
            Ok(_) => None,
        },
    }
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
    use crate::location::cities::CityMatch;

    fn rendered(width: u16, height: u16, state: &LocationState) -> Vec<String> {
        let fixture = testing::snapshot_runtime();
        let model = crate::ui::model::build_model(&fixture.runtime);
        let breakpoint = Breakpoint::of_size(width, height);
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
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

    fn setif_result() -> CityMatch {
        CityMatch {
            name: "Sétif".to_owned(),
            country: "Algeria".to_owned(),
            cc: "DZ".to_owned(),
            lat: 36.19,
            lng: 5.41,
            tz: "Africa/Algiers".to_owned(),
            population: 288_196,
        }
    }

    /// The CURRENT LOCATION panel carries the fixture's six facts,
    /// the coordinates green.
    #[test]
    fn the_current_location_panel_shows_the_six_facts() {
        let state = LocationState::default();
        let text = rendered(100, 30, &state).join("\n");
        for word in [
            "CURRENT LOCATION",
            "City",
            "Country",
            "Latitude",
            "Longitude",
            "Timezone",
            "Method",
        ] {
            assert!(text.contains(word), "missing {word} in:\n{text}");
        }
    }

    /// §16.6 item 6: three results with the selection marker, the
    /// population and zone right-aligned from 100×30.
    #[test]
    fn search_results_render_with_hints() {
        let state = LocationState {
            query: "Setif".to_owned(),
            results: vec![setif_result()],
            ..LocationState::default()
        };
        let text = rendered(100, 30, &state).join("\n");
        assert!(
            text.contains("● Sétif, Algeria"),
            "the selected row:\n{text}"
        );
        assert!(text.contains("288k · Africa/Algiers"), "the hint:\n{text}");
    }

    /// At 80×24 the hint is dropped (§16.6 item 6's small form).
    #[test]
    fn small_drops_the_result_hints() {
        let state = LocationState {
            query: "Setif".to_owned(),
            results: vec![setif_result()],
            ..LocationState::default()
        };
        let text = rendered(80, 24, &state).join("\n");
        assert!(text.contains("● Sétif, Algeria"), "the row:\n{text}");
        assert!(!text.contains("288k"), "no hint at small:\n{text}");
    }

    /// §16.6 item 6's own no-match wording.
    #[test]
    fn no_match_shows_the_designs_wording() {
        let state = LocationState {
            query: "zzz".to_owned(),
            ..LocationState::default()
        };
        let text = rendered(100, 30, &state).join("\n");
        assert!(
            text.contains("no cities matched \"zzz\" · try [ COORDINATES ]"),
            "wording:\n{text}"
        );
    }

    /// The coordinate error is inline and red-worded, never a screen
    /// change (§16.6 item 6).
    #[test]
    fn out_of_range_coordinates_show_the_inline_error() {
        let state = LocationState {
            mode: LocationMode::Coords,
            lat_buf: "999".to_owned(),
            ..LocationState::default()
        };
        let text = rendered(100, 30, &state).join("\n");
        assert!(
            text.contains("latitude must be between -90 and 90 · E101"),
            "error:\n{text}"
        );
    }

    /// Valid coordinates show the nearest city instead.
    #[test]
    fn valid_coordinates_show_the_nearest_city() {
        let state = LocationState {
            mode: LocationMode::Coords,
            lat_buf: "36.19".to_owned(),
            lng_buf: "5.41".to_owned(),
            ..LocationState::default()
        };
        let text = rendered(100, 30, &state).join("\n");
        assert!(text.contains("nearest: Sétif"), "hint:\n{text}");
    }

    /// The AUTO panel's accept hint, and E103's inline wording when
    /// the chain failed.
    #[test]
    fn auto_shows_the_proposal_or_e103() {
        let mut state = LocationState {
            mode: LocationMode::Auto,
            auto: Some(Ok(setif_result())),
            ..LocationState::default()
        };
        let text = rendered(100, 30, &state).join("\n");
        assert!(text.contains("● Sétif, Algeria"), "proposal:\n{text}");
        assert!(text.contains("enter to accept"), "hint:\n{text}");

        state.auto = Some(Err("timezone could not be detected".to_owned()));
        let text = rendered(100, 30, &state).join("\n");
        assert!(
            text.contains("timezone could not be detected · E103"),
            "error:\n{text}"
        );
    }

    /// The 80×24 button row uses the short labels (§16.6 item 6).
    #[test]
    fn small_buttons_use_the_short_labels() {
        let state = LocationState::default();
        let text = rendered(80, 24, &state).join("\n");
        assert!(
            text.contains("[ SEARCH ] [ COORDS ] [ AUTO ]"),
            "buttons:\n{text}"
        );
    }

    /// §16.6's snapshot: the screen at all three sizes.
    #[test]
    fn the_location_screen_snapshots() {
        let state = LocationState {
            query: "Setif".to_owned(),
            results: vec![setif_result()],
            ..LocationState::default()
        };
        for (width, height, name) in [(80, 24, "80x24"), (100, 30, "100x30"), (120, 35, "120x35")] {
            let text = rendered(width, height, &state).join("\n");
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
            settings.bind(|| insta::assert_snapshot!(format!("location_{name}"), text));
        }
    }
}
