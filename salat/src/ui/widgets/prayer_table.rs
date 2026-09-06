//! The TODAY table's prayer rows (§15.5, §16.1 item 9/10).
//!
//! Quick mode's geometry is the design's small 3-column grid —
//! `90px 52px 1fr` translated to `Length(9) Length(7) Min(0)`: a
//! 9-column name, a 7-column time, the status in whatever remains.
//! The full TUI's dashboard adds the design's larger grid
//! (`100px 52px 1fr 44px` — a `NOTIF` column, right-aligned) and the
//! row furniture the design draws on the next prayer: the `▶ ` name
//! prefix, the 2px green left edge (one `▎` cell — a terminal has no
//! sub-cell borders) and the `rowActive` ground. The dashboard's
//! cursor — the `c` complete action's target (A3) — is the `rowSel`
//! ground on its row.
//!
//! Truncation is by display width, never wrapping (§16.14 item 10):
//! every column write is clamped to the area, and the status column
//! ends in `…` when the note does not fit — the one place the design
//! already shows an ellipsis.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::domain::PrayerState;
use crate::ui::theme::Theme;

/// One row of the table. `note` is the next prayer's
/// `· in 01:42:37` — the design's status cell for the highlighted row;
/// `notif` is the NOTIF cell (`-5m`), rendered by the 4-column grid
/// only.
#[derive(Debug, Clone)]
pub struct PrayerRow {
    /// The prayer's name, already formatted (`Fajr`, or the
    /// dashboard's short `MAG`).
    pub name: String,
    /// The prayer's time, already formatted (`05:09`).
    pub time: String,
    /// Its state at `now`.
    pub state: PrayerState,
    /// The next row's extra note, if this is the next row.
    pub note: Option<String>,
    /// The NOTIF cell — the prayer's `before` offset, `Some` only on
    /// the dashboard's rows.
    pub notif: Option<String>,
}

/// The name column's width.
pub const NAME_WIDTH: usize = 9;
/// The time column's width.
pub const TIME_WIDTH: usize = 7;
/// The NOTIF column's width (the design's `44px`).
pub const NOTIF_WIDTH: usize = 6;

/// Which column grid a table renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Columns {
    /// `PRAYER · TIME · STATUS` — quick mode, and the 80×24 dashboard
    /// (§16.1 item 9: the NOTIF column collapses).
    Three,
    /// `… · NOTIF` right-aligned — the ≥100-column dashboard.
    Four,
}

/// The table's rendering mode beyond its rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableSpec {
    /// The column grid.
    pub columns: Columns,
    /// The cursor row (the `c` action's target, A3) — `rowSel` ground.
    pub cursor: Option<usize>,
    /// The full-TUI row furniture: `▶` on the next row, the green `▎`
    /// edge, the `rowActive` ground.
    pub rail: bool,
}

impl TableSpec {
    /// Quick mode's table: three columns, no cursor, no rail.
    pub const QUICK: Self = Self {
        columns: Columns::Three,
        cursor: None,
        rail: false,
    };

    /// The dashboard's table at one breakpoint.
    pub const fn dashboard(columns: Columns, cursor: Option<usize>) -> Self {
        Self {
            columns,
            cursor,
            rail: true,
        }
    }
}

/// Write one clipped, styled string at `(x, y)` and return the next
/// column. This is the whole truncation story: `set_stringn` cuts by
/// display width.
fn put(buf: &mut Buffer, x: u16, y: u16, limit: u16, text: &str, style: Style) -> u16 {
    if limit == 0 {
        return x;
    }
    let (next, _) = buf.set_stringn(x, y, text, limit as usize, style);
    next
}

/// Draw the rows, one per area row, stopping at the area's bottom —
/// the quick-mode ladder's dropped rows are simply never passed in.
pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, rows: &[PrayerRow], spec: &TableSpec) {
    for (index, row) in rows.iter().enumerate() {
        let y = area.y + index as u16;
        if y >= area.bottom() {
            break;
        }
        render_row(area, buf, theme, y, index, row, spec);
    }
}

fn render_row(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    y: u16,
    index: usize,
    row: &PrayerRow,
    spec: &TableSpec,
) {
    let is_next = row.state == PrayerState::Next;

    // The row grounds, painted across the full row first so every
    // later write sits on them: the next prayer's `rowActive`, the
    // cursor's `rowSel` — the design's active highlight outranks the
    // cursor's when they coincide (the ▶ already marks that row).
    if spec.rail {
        let ground = if is_next {
            Some(theme.row_active)
        } else if spec.cursor == Some(index) {
            Some(theme.row_sel)
        } else {
            None
        };
        if let Some(ground) = ground {
            buf.set_style(Rect::new(area.x, y, area.width, 1), Style::new().bg(ground));
        }
    }

    let (name_color, bold) = match row.state {
        PrayerState::Next => (theme.green, true),
        PrayerState::Passed => (theme.text_dim, false),
        _ => (theme.text, false),
    };
    let mut style = Style::new().fg(name_color);
    if bold {
        style = style.add_modifier(ratatui::style::Modifier::BOLD);
    }

    let end = area.x + area.width;
    let mut x = area.x;

    // The green edge: one `▎` cell where the design's 2px border is.
    if spec.rail {
        x = put(
            buf,
            x,
            y,
            end - x,
            if is_next { "▎" } else { " " },
            if is_next {
                Style::new().fg(theme.green)
            } else {
                Style::new()
            },
        );
    }

    // The name: `▶ Maghrib` on the next row (the design's prefix), `  `
    // elsewhere — the prefix rides *outside* the padded NAME_WIDTH field
    // so the time column stays put whether or not the rail is drawn.
    let name = if spec.rail {
        format!(
            "{}{:<NAME_WIDTH$}",
            if is_next { "▶ " } else { "  " },
            row.name
        )
    } else {
        format!("{:<NAME_WIDTH$}", row.name)
    };
    x = put(buf, x, y, end - x, &format!("{name:<NAME_WIDTH$}"), style);

    x = put(
        buf,
        x,
        y,
        end - x,
        &format!("{:<TIME_WIDTH$}", row.time),
        Style::new().fg(if row.state == PrayerState::Passed {
            theme.text_dim
        } else {
            theme.text_sub
        }),
    );

    // The status cell: the badge, then the note in the badge's colour
    // when this is the next row. The 4-column grid reserves the NOTIF
    // cell's six columns so the note can never run under it.
    let status_end = end - u16::from(spec.columns == Columns::Four) * NOTIF_WIDTH as u16;
    let (glyph, word, badge_color) = theme.state_badge(row.state);
    x = put(
        buf,
        x,
        y,
        status_end - x,
        glyph,
        Style::new().fg(badge_color),
    );
    x = put(buf, x, y, status_end - x, " ", Style::new().fg(badge_color));
    x = put(
        buf,
        x,
        y,
        status_end - x,
        word,
        Style::new().fg(badge_color),
    );
    if let Some(note) = &row.note {
        let remaining = status_end.saturating_sub(x);
        let fits = note.width() < remaining as usize;
        if fits {
            put(buf, x, y, remaining, " ", Style::new().fg(badge_color));
            put(
                buf,
                x + 1,
                y,
                remaining - 1,
                note,
                Style::new().fg(badge_color),
            );
        } else if remaining >= 2 {
            // The design's ellipsis: better a marked cut than a
            // half-drawn word.
            put(buf, x, y, remaining, "…", Style::new().fg(theme.text_dim));
        }
    }

    // The NOTIF column (the 4-column grid only), right-aligned at the
    // row's edge in textDim.
    if spec.columns == Columns::Four
        && let Some(notif) = &row.notif
        && area.width >= NOTIF_WIDTH as u16
    {
        put(
            buf,
            end - NOTIF_WIDTH as u16,
            y,
            NOTIF_WIDTH as u16,
            &format!("{notif:>NOTIF_WIDTH$}"),
            Style::new().fg(theme.text_dim),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(buf: &Buffer, width: usize) -> String {
        (0..width)
            .map(|x| buf[(x as u16, 0)].symbol().to_string())
            .collect()
    }

    fn passed(name: &str) -> PrayerRow {
        PrayerRow {
            name: name.into(),
            time: "05:09".into(),
            state: PrayerState::Passed,
            note: None,
            notif: None,
        }
    }

    fn next_row() -> PrayerRow {
        PrayerRow {
            name: "Maghrib".into(),
            time: "19:21".into(),
            state: PrayerState::Next,
            note: Some("· in 01:42:37".into()),
            notif: Some("-5m".into()),
        }
    }

    /// §14.6's rows, verbatim: 9-column name, 7-column time, then the
    /// badge — `Fajr     05:09  ✓ passed`.
    #[test]
    fn the_specimens_rows_render_verbatim() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let rows = [
            passed("Fajr"),
            next_row(),
            PrayerRow {
                name: "Isha".into(),
                time: "20:47".into(),
                state: PrayerState::Upcoming,
                note: None,
                notif: None,
            },
        ];
        let expected = [
            "Fajr     05:09  ✓ passed",
            "Maghrib  19:21  ● next · in 01:42:37",
            "Isha     20:47  ○ upcoming",
        ];
        for (index, row) in rows.iter().enumerate() {
            let mut buf = Buffer::empty(Rect::new(0, 0, 80, 1));
            render_row(
                Rect::new(0, 0, 80, 1),
                &mut buf,
                &theme,
                0,
                index,
                row,
                &TableSpec::QUICK,
            );
            let text = row_text(&buf, 80);
            assert_eq!(
                &text[..expected[index].len()],
                expected[index],
                "row {index}"
            );
        }
    }

    /// The dashboard's row (§16.1): the green `▎` edge and `▶ ` prefix
    /// on the next prayer, the NOTIF cell right-aligned at 6 columns.
    #[test]
    fn the_dashboard_row_carries_the_rail_and_notif() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let row = PrayerRow {
            name: "MAG".into(),
            time: "19:21".into(),
            state: PrayerState::Next,
            note: Some("· in 01:42:37".into()),
            notif: Some("-5m".into()),
        };
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 1));
        render_row(
            Rect::new(0, 0, 60, 1),
            &mut buf,
            &theme,
            0,
            3,
            &row,
            &TableSpec::dashboard(Columns::Four, None),
        );
        let text = row_text(&buf, 60);
        assert!(text.starts_with("▎▶ MAG"), "the rail and prefix: {text:?}");
        assert!(text.contains("19:21"), "the time: {text:?}");
        assert!(text.ends_with("   -5m"), "NOTIF right-aligned: {text:?}");
        // The edge and the name are green; the name is bold.
        assert_eq!(buf[(0, 0)].style().fg, Some(theme.green));
        assert!(buf[(2, 0)].style().add_modifier == ratatui::style::Modifier::BOLD);
        // The next row's ground is rowActive.
        assert_eq!(buf[(30, 0)].style().bg, Some(theme.row_active));
    }

    /// The 3-column grid (the 80×24 dashboard, §16.1 item 9) drops the
    /// NOTIF cell even when the row carries one.
    #[test]
    fn the_three_column_grid_drops_notif() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 1));
        render_row(
            Rect::new(0, 0, 60, 1),
            &mut buf,
            &theme,
            0,
            3,
            &next_row(),
            &TableSpec::dashboard(Columns::Three, None),
        );
        let text = row_text(&buf, 60);
        assert!(!text.contains("-5m"), "NOTIF hidden: {text:?}");
    }

    /// The cursor row (the `c` action's target) gets the `rowSel`
    /// ground; the next row keeps its `rowActive` even when it is the
    /// cursor's — the ▶ already marks it.
    #[test]
    fn the_cursor_row_gets_its_ground() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let spec = TableSpec::dashboard(Columns::Three, Some(0));
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 1));
        render_row(
            Rect::new(0, 0, 60, 1),
            &mut buf,
            &theme,
            0,
            0,
            &passed("FAJ"),
            &spec,
        );
        assert_eq!(buf[(30, 0)].style().bg, Some(theme.row_sel));

        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 1));
        render_row(
            Rect::new(0, 0, 60, 1),
            &mut buf,
            &theme,
            0,
            3,
            &next_row(),
            &spec,
        );
        assert_eq!(buf[(30, 0)].style().bg, Some(theme.row_active));
    }

    /// The next row's name is green and bold; a passed row's name and
    /// time are dim.
    #[test]
    fn the_states_colour_their_rows() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let rows = [
            (PrayerState::Passed, PrayerState::Next),
            (PrayerState::Upcoming, PrayerState::Next),
        ];
        for (state, next) in rows {
            let mut buf = Buffer::empty(Rect::new(0, 0, 80, 1));
            render_row(
                Rect::new(0, 0, 80, 1),
                &mut buf,
                &theme,
                0,
                0,
                &PrayerRow {
                    name: "Asr".into(),
                    time: "16:23".into(),
                    state,
                    note: None,
                    notif: None,
                },
                &TableSpec::QUICK,
            );
            let expected_fg = if state == PrayerState::Passed {
                theme.text_dim
            } else if state == next {
                theme.green
            } else {
                theme.text
            };
            assert_eq!(buf[(0, 0)].style().fg, Some(expected_fg));
        }
    }

    /// §16.14 item 10: the note is dropped whole (with an ellipsis
    /// marker) when it would not fit — never half-drawn, never past
    /// the area's edge.
    #[test]
    fn a_note_that_does_not_fit_marks_the_cut() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 27, 1));
        render_row(
            Rect::new(0, 0, 27, 1),
            &mut buf,
            &theme,
            0,
            0,
            &PrayerRow {
                name: "Maghrib".into(),
                time: "19:21".into(),
                state: PrayerState::Next,
                note: Some("· in 01:42:37".into()),
                notif: None,
            },
            &TableSpec::QUICK,
        );
        let text = row_text(&buf, 27);
        // 9 + 7 + "● next" leaves 10 columns; the note needs 14 — the
        // ellipsis marks the cut and nothing renders past 27.
        assert!(text.trim_end().ends_with('…'), "got {text:?}");
        assert!(text.starts_with("Maghrib  19:21  ● next"), "got {text:?}");
    }

    /// The table stops at the area's bottom.
    #[test]
    fn the_table_never_writes_below_its_area() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let rows: Vec<PrayerRow> = (0..5)
            .map(|_| PrayerRow {
                name: "Asr".into(),
                time: "16:23".into(),
                state: PrayerState::Upcoming,
                note: None,
                notif: None,
            })
            .collect();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 3));
        render(
            Rect::new(0, 0, 80, 2),
            &mut buf,
            &theme,
            &rows,
            &TableSpec::QUICK,
        );
        assert_eq!(buf[(0, 0)].symbol(), "A");
        assert_eq!(buf[(0, 1)].symbol(), "A");
        assert_eq!(buf[(0, 2)].symbol(), " ");
    }
}
