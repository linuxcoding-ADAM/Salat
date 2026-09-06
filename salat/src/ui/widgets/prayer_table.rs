//! The TODAY table's prayer rows (§15.5, §16.14 item 10).
//!
//! Quick mode's geometry is the design's small 3-column grid —
//! `90px 52px 1fr` translated to `Length(9) Length(7) Min(0)`: a
//! 9-column name, a 7-column time, the status in whatever remains.
//! The `NOTIF` column (medium/large) and the `▶` cursor rail belong
//! to the full TUI's table and arrive with the screens that use them.
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
/// `· in 01:42:37` — the design's status cell for the highlighted row.
#[derive(Debug, Clone)]
pub struct PrayerRow {
    /// The prayer's name, already formatted (`Fajr`).
    pub name: String,
    /// The prayer's time, already formatted (`05:09`).
    pub time: String,
    /// Its state at `now`.
    pub state: PrayerState,
    /// The next row's extra note, if this is the next row.
    pub note: Option<String>,
}

/// The name column's width.
pub const NAME_WIDTH: usize = 9;
/// The time column's width.
pub const TIME_WIDTH: usize = 7;

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
pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, rows: &[PrayerRow]) {
    for (index, row) in rows.iter().enumerate() {
        let y = area.y + index as u16;
        if y >= area.bottom() {
            break;
        }
        render_row(area, buf, theme, y, row);
    }
}

fn render_row(area: Rect, buf: &mut Buffer, theme: &Theme, y: u16, row: &PrayerRow) {
    let (name_color, bold) = match row.state {
        PrayerState::Next => (theme.green, true),
        PrayerState::Passed => (theme.text_dim, false),
        _ => (theme.text, false),
    };
    let mut style = Style::new().fg(name_color);
    if bold {
        style = style.add_modifier(ratatui::style::Modifier::BOLD);
    }

    let mut x = area.x;
    let end = area.x + area.width;
    x = put(
        buf,
        x,
        y,
        end - x,
        &format!("{:<NAME_WIDTH$}", row.name),
        style,
    );
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
    // when this is the next row.
    let (glyph, word, badge_color) = theme.state_badge(row.state);
    x = put(buf, x, y, end - x, glyph, Style::new().fg(badge_color));
    x = put(buf, x, y, end - x, " ", Style::new().fg(badge_color));
    x = put(buf, x, y, end - x, word, Style::new().fg(badge_color));
    if let Some(note) = &row.note {
        let remaining = end.saturating_sub(x);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(buf: &Buffer, width: usize) -> String {
        (0..width)
            .map(|x| buf[(x as u16, 0)].symbol().to_string())
            .collect()
    }

    /// §14.6's rows, verbatim: 9-column name, 7-column time, then the
    /// badge — `Fajr     05:09  ✓ passed`.
    #[test]
    fn the_specimens_rows_render_verbatim() {
        let theme = Theme::for_mode(crate::ui::theme::RenderMode::Truecolor);
        let rows = [
            PrayerRow {
                name: "Fajr".into(),
                time: "05:09".into(),
                state: PrayerState::Passed,
                note: None,
            },
            PrayerRow {
                name: "Maghrib".into(),
                time: "19:21".into(),
                state: PrayerState::Next,
                note: Some("· in 01:42:37".into()),
            },
            PrayerRow {
                name: "Isha".into(),
                time: "20:47".into(),
                state: PrayerState::Upcoming,
                note: None,
            },
        ];
        let expected = [
            "Fajr     05:09  ✓ passed",
            "Maghrib  19:21  ● next · in 01:42:37",
            "Isha     20:47  ○ upcoming",
        ];
        for (index, row) in rows.iter().enumerate() {
            let mut buf = Buffer::empty(Rect::new(0, 0, 80, 1));
            render_row(Rect::new(0, 0, 80, 1), &mut buf, &theme, 0, row);
            let text = row_text(&buf, 80);
            assert_eq!(
                &text[..expected[index].len()],
                expected[index],
                "row {index}"
            );
        }
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
                &PrayerRow {
                    name: "Asr".into(),
                    time: "16:23".into(),
                    state,
                    note: None,
                },
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
            &PrayerRow {
                name: "Maghrib".into(),
                time: "19:21".into(),
                state: PrayerState::Next,
                note: Some("· in 01:42:37".into()),
            },
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
            })
            .collect();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 3));
        render(Rect::new(0, 0, 80, 2), &mut buf, &theme, &rows);
        assert_eq!(buf[(0, 0)].symbol(), "A");
        assert_eq!(buf[(0, 1)].symbol(), "A");
        assert_eq!(buf[(0, 2)].symbol(), " ");
    }
}
