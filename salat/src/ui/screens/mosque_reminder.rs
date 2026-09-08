//! The mosque reminder overlay (§16.8) and its §16.15 inline variant.
//!
//! **Full screen (§16.8).** A *centered panel*, not a full-frame
//! takeover — clamped to `min(56, w − 4)` columns at 80×24 and to 64
//! columns at 100×30/120×35 (it does not grow with the terminal;
//! the extra space is empty, which is the point). The panel carries
//! the title `MOSQUE REMINDER`, the 800 ms blinking banner
//! `◉ ── TIME TO HEAD OUT ── ◉` (shortening to `◉ TIME TO HEAD OUT ◉`
//! when the panel is narrow), `hr` ×2, the prayer name as one bold
//! row (bigtext-lite — the panel is small), the data line
//! `Adhan was N minutes ago · HH:MM` (with the `(delayed notice)`
//! suffix on a late fire, §16.8 item 8), the literal
//! `Iqama in approximately ~5 minutes`, and the two key hints.
//!
//! **Inline (§16.15).** The overlay owns the whole cleared viewport;
//! a 9-row panel of the same `min(56, w − 4)` width, centered
//! horizontally and vertically by `top_pad = (v − block) / 2` with the
//! remainder below, down the drop ladder 9 → 8 (− the upper rule) → 7
//! (− the prayer-name row) → 6 (− the lower rule, floor). Below the
//! floor: the single truncated line. Mosque drops its prayer-name row
//! before its rules because the banner already names the event.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use unicode_width::UnicodeWidthStr as _;

use chrono::Duration;
use chrono_tz::Tz;

use crate::domain::Notifiable;
use crate::ui::model::UiModel;
use crate::ui::theme::{Blink, Theme, blink, hhmm};
use crate::ui::widgets::hr;
use crate::ui::widgets::kbd_hint;
use crate::ui::widgets::panel::Panel;

use super::super::shell::Overlay;

/// The full banner, for panels wide enough to hold it.
const BANNER: &str = "◉ ── TIME TO HEAD OUT ── ◉";

/// §16.8 item 9's shortened banner for an 80×24 panel.
const BANNER_SHORT: &str = "◉ TIME TO HEAD OUT ◉";

/// The Iqama line (item 6) — a literal: Salat has no Iqama data, so
/// the design's `~` and the word "approximately" stay, verbatim.
const IQAMA: &str = "Iqama in approximately ~5 minutes";

/// The Adhan instant is recovered from the live config's
/// `after_minutes` — the payload stores only the reminder's own fire
/// time, so no extra state rides along (§16.8 item 2's `{prayer,
/// adhan_at, blink}` shape, reconstructed).
pub(crate) struct MosqueReminderData {
    pub prayer: Notifiable,
    pub adhan_at: TzInstant,
    pub late: bool,
}

/// A wall-clock instant in the window's zone — an alias to keep the
/// payload struct's field types short.
pub(crate) type TzInstant = chrono::DateTime<Tz>;

impl MosqueReminderData {
    /// Destructure the overlay into the payload; `None` for the other
    /// variant — the dispatcher never asks for what it did not match.
    fn of(overlay: Overlay, model: &UiModel) -> Option<Self> {
        let Overlay::MosqueReminder {
            prayer,
            reminder_at,
            late,
        } = overlay
        else {
            return None;
        };
        let after = prayer.settings(&model.config.prayers).after_minutes;
        let adhan_at = reminder_at - Duration::minutes(i64::from(after));
        Some(Self {
            prayer,
            adhan_at,
            late,
        })
    }
}

/// Render the overlay (§16.8) into the whole frame. The panel is
/// centered in empty space on the app's normal ground — the caller
/// has already painted it.
pub fn render(overlay: Overlay, model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let Some(data) = MosqueReminderData::of(overlay, model) else {
        return;
    };

    // The panel's clamped width (items 9–10): `min(56, w − 4)` at
    // 80×24, capped at 64 for the 100×30/120×35 class — it does not
    // grow with the terminal.
    let cap = if area.width >= 100 { 64 } else { 56 };
    let width = cap.min(area.width.saturating_sub(4)).max(20);
    let block = 7; // banner, rule, name, data line, data line, rule, hints
    let height = (block + 2).min(area.height); // + 2 border rows
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let panel = Rect::new(x, y, width, height);

    let inner = Panel::new()
        .title("MOSQUE REMINDER")
        .active(true)
        .render(panel, buf, theme);

    let mut cursor = inner.y;
    cursor = banner_row(inner, cursor, buf, theme, model.now);
    cursor = rule(inner, cursor, buf, theme);
    cursor = name_row(inner, cursor, buf, theme, data.prayer);
    cursor = data_row(inner, cursor, buf, theme, &data, model);
    cursor = iqama_row(inner, cursor, buf, theme);
    cursor = rule(inner, cursor, buf, theme);
    hints_row(inner, cursor, buf, theme, "SNOOZE 5m");
}

/// Render the §16.15 inline variant into the cleared viewport.
pub fn render_inline(
    overlay: Overlay,
    model: &UiModel,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let Some(data) = MosqueReminderData::of(overlay, model) else {
        return;
    };

    // §16.15: nine rows in a panel of `min(56, w − 4)` columns,
    // centered horizontally and vertically — the panel keeps its
    // designed character inline, on the viewport's normal ground.
    let width = 56.min(area.width.saturating_sub(4)).max(20);
    let x = area.x + (area.width.saturating_sub(width)) / 2;

    // The ladder's row set: banner, rule, name, data, data, rule, hints
    // — the panel's own borders never drop (they are what makes the
    // panel a panel), so they sit outside the ladder's accounting.
    let mut rows = vec![
        InlineRow::Banner,
        InlineRow::Rule,
        InlineRow::Name,
        InlineRow::Data,
        InlineRow::Iqama,
        InlineRow::Rule,
        InlineRow::Hints,
    ];
    // §16.15's floor is 6: the lower rule drops last, and below 6 the
    // panel is not attempted at all — the single truncated line below.
    while rows.len() as u16 + 2 > area.height {
        if let Some(index) = inline_drop(&rows) {
            rows.remove(index);
        } else {
            break;
        }
    }
    if (rows.len() as u16 + 2) > area.height {
        // The Iqama figure is the literal `~5 min` — the same fixed
        // number the panel's own IQAMA line carries (§16.8 item 6:
        // Salat has no Iqama data), never `after_minutes`, which is
        // the *reminder's* offset, not the Iqama's.
        let line = format!(
            "◉ TIME TO HEAD OUT · {} · Iqama ~5 min · [ENTER] dismiss  [S] snooze",
            data.prayer.display(),
        );
        buf.set_stringn(
            area.x,
            area.y,
            line,
            area.width as usize,
            Style::new().fg(theme.yellow),
        );
        return;
    }

    let height = rows.len() as u16 + 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let panel = Rect::new(x, y, width, height);
    let inner = Panel::new()
        .title("MOSQUE REMINDER")
        .active(true)
        .render(panel, buf, theme);

    let mut cursor = inner.y;
    for row in &rows {
        cursor = match row {
            InlineRow::Banner => banner_row(inner, cursor, buf, theme, model.now),
            InlineRow::Rule => rule(inner, cursor, buf, theme),
            InlineRow::Name => name_row(inner, cursor, buf, theme, data.prayer),
            InlineRow::Data => data_row(inner, cursor, buf, theme, &data, model),
            InlineRow::Iqama => iqama_row(inner, cursor, buf, theme),
            InlineRow::Hints => hints_row(inner, cursor, buf, theme, "SNOOZE 5m"),
        };
    }
}

/// One content row of the inline layout, tagged for the dropper.
enum InlineRow {
    /// The blinking banner.
    Banner,
    /// A full-width `hr`.
    Rule,
    /// The prayer name — the row the ladder drops second.
    Name,
    /// The data line (and its `(delayed notice)` sub-line).
    Data,
    /// The Iqama line.
    Iqama,
    /// The two key hints.
    Hints,
}

/// §16.15's Mosque ladder: 9 → 8 drops the *upper* rule (the banner
/// already frames the panel's top), 8 → 7 the prayer-name row (the
/// banner already names the event), 7 → 6 the lower rule — the floor.
/// The drops are ordered, so the vec's own shape encodes progress: two
/// rules left means the first is the upper one; one rule left with the
/// name still present means the name goes next; one rule left without
/// it is the lower rule, the last drop. Returns the index of the next
/// row to drop, `None` at the floor.
fn inline_drop(rows: &[InlineRow]) -> Option<usize> {
    let rules = rows
        .iter()
        .filter(|row| matches!(row, InlineRow::Rule))
        .count();
    if rules == 2 {
        return rows.iter().position(|row| matches!(row, InlineRow::Rule));
    }
    if rules == 1 {
        if let Some(index) = rows.iter().position(|row| matches!(row, InlineRow::Name)) {
            return Some(index);
        }
        return rows.iter().position(|row| matches!(row, InlineRow::Rule));
    }
    None
}

/// The blinking banner row (item 5): the 800 ms blink picks visibility;
/// the panel's width picks the long or short form (item 9).
fn banner_row(area: Rect, y: u16, buf: &mut Buffer, theme: &Theme, now: TzInstant) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    let on = Blink::new(blink::MOSQUE_BANNER_MS).on(now);
    let full_width = BANNER.width() as u16;
    let text = if full_width <= area.width {
        BANNER
    } else {
        BANNER_SHORT
    };
    if on {
        let x = area.x + (area.width.saturating_sub(text.width() as u16)) / 2;
        buf.set_stringn(
            x,
            y,
            text,
            area.width as usize,
            Style::new().fg(theme.yellow),
        );
    }
    y + 1
}

/// A full-width `hr` inside the panel. Returns the next row's y.
fn rule(area: Rect, y: u16, buf: &mut Buffer, theme: &Theme) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    hr::render(Rect::new(area.x, y, area.width, 1), buf, theme, None);
    y + 1
}

/// The prayer name as one bold row — bigtext-lite (item 5: the panel
/// is small, so 1.6em, not 3-row blocks).
fn name_row(area: Rect, y: u16, buf: &mut Buffer, theme: &Theme, prayer: Notifiable) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    let name = prayer.display();
    let x = area.x + (area.width.saturating_sub(name.width() as u16)) / 2;
    buf.set_stringn(
        x,
        y,
        name,
        area.width as usize,
        Style::new()
            .fg(theme.text_bright)
            .add_modifier(Modifier::BOLD),
    );
    y + 1
}

/// The data line (item 6): `Adhan was N minutes ago · HH:MM`, with the
/// `(delayed notice)` suffix on a late fire (item 8) — a suffix, not a
/// second row: the panel's row budget (7) has no spare row, and the
/// design's own specimen renders it inline.
fn data_row(
    area: Rect,
    y: u16,
    buf: &mut Buffer,
    theme: &Theme,
    data: &MosqueReminderData,
    model: &UiModel,
) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    let after = data.prayer.settings(&model.config.prayers).after_minutes;
    let time = hhmm(data.adhan_at, &model.config.display.time_format);
    let delayed = if data.late { " (delayed notice)" } else { "" };
    let elapsed = format!("Adhan was {after} minutes ago · {time}{delayed}");
    let line = Line::from(Span::styled(elapsed, Style::new().fg(theme.text)));
    let x = area.x + (area.width.saturating_sub(line.width() as u16)) / 2;
    buf.set_line(x, y, &line, area.width);
    y + 1
}

/// The Iqama line (item 6), centered. Returns the next row's y.
fn iqama_row(area: Rect, y: u16, buf: &mut Buffer, theme: &Theme) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    let x = area.x + (area.width.saturating_sub(IQAMA.width() as u16)) / 2;
    buf.set_stringn(
        x,
        y,
        IQAMA,
        area.width as usize,
        Style::new().fg(theme.text),
    );
    y + 1
}

/// The two key hints on one row (item 9).
fn hints_row(area: Rect, y: u16, buf: &mut Buffer, theme: &Theme, snooze_label: &str) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    let dismiss = kbd_hint::line("ENTER", "DISMISS", theme);
    let snooze = kbd_hint::line("S", snooze_label, theme);
    let total = dismiss.width() + 3 + snooze.width();
    if (total as u16) <= area.width {
        let x = area.x + (area.width.saturating_sub(total as u16)) / 2;
        buf.set_line(x, y, &dismiss, area.width);
        buf.set_line(x + dismiss.width() as u16 + 3, y, &snooze, area.width);
    }
    y + 1
}

// ---------------------------------------------------------------------------
// §23 Phase 10 item 8's mosque suite: snapshots at the three full-screen
// sizes and in the inline viewport (§16.15), the panel-width clamps, the
// top_pad arithmetic, the ladder's floor, the below-floor line and the
// §16.8 item 8 `(delayed notice)` suffix.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::testing;
    use crate::domain::testing as dt;
    use crate::ui::theme::RenderMode;

    /// The §20.3 fixture model — Maghrib next, `after = 15`.
    fn model() -> UiModel {
        let fixture = testing::snapshot_runtime();
        crate::ui::model::build_model(&fixture.runtime)
    }

    /// The §20.3 specimen reminder — Maghrib's, firing at 19:36
    /// (19:21 + the default 15).
    fn overlay(late: bool) -> Overlay {
        Overlay::MosqueReminder {
            prayer: Notifiable::Maghrib,
            reminder_at: dt::at(dt::TODAY, 19, 36, 0),
            late,
        }
    }

    fn theme() -> Theme {
        Theme::for_mode(RenderMode::Truecolor)
    }

    /// The full-screen §16.8 overlay at one terminal size.
    fn rendered_full(width: u16, height: u16, late: bool) -> Buffer {
        let model = model();
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        render(overlay(late), &model, buf.area, &mut buf, &theme());
        buf
    }

    /// The §16.15 inline overlay in a `v`-row viewport.
    fn rendered_inline(v: u16, late: bool) -> Buffer {
        let model = model();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, v));
        render_inline(overlay(late), &model, buf.area, &mut buf, &theme());
        buf
    }

    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_owned())
            .collect()
    }

    fn text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| row_text(buf, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn row_is_blank(buf: &Buffer, y: u16) -> bool {
        (0..buf.area.width).all(|x| buf[(x, y)].symbol() == " ")
    }

    fn snap(name: &str, buf: &Buffer) {
        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
        settings.bind(|| insta::assert_snapshot!(name, text(buf)));
    }

    /// §16.8 item 6's content: the banner, the bold name, the data line
    /// recovered from the config's offset, the literal Iqama line, and
    /// the two hints — on time, so no `(delayed notice)`.
    #[test]
    fn full_screen_carries_the_designed_content() {
        let text = text(&rendered_full(100, 30, false));
        for word in [
            "MOSQUE REMINDER",
            "TIME TO HEAD OUT",
            "MAGHRIB",
            "Adhan was 15 minutes ago · 19:21",
            IQAMA,
            "[ENTER] DISMISS",
            "[S] SNOOZE 5m",
        ] {
            assert!(text.contains(word), "missing {word:?} in:\n{text}");
        }
        assert!(!text.contains("(delayed notice)"), "on time:\n{text}");
    }

    /// The three full-screen sizes, snapshotted (§23 item 8).
    #[test]
    fn full_screen_snapshot_80x24() {
        snap("mosque_reminder_80x24", &rendered_full(80, 24, false));
    }

    #[test]
    fn full_screen_snapshot_100x30() {
        snap("mosque_reminder_100x30", &rendered_full(100, 30, false));
    }

    #[test]
    fn full_screen_snapshot_120x35() {
        snap("mosque_reminder_120x35", &rendered_full(120, 35, false));
    }

    /// §16.8 items 9–10: the panel is 56 columns at 80 wide and clamped
    /// to 64 at 100 and 120 — it does not grow with the terminal; the
    /// extra space stays empty.
    #[test]
    fn the_panel_width_clamps() {
        let narrow = rendered_full(80, 24, false);
        // 24 rows, panel 9 tall → top border at row 7, x = (80−56)/2 = 12.
        let top = row_text(&narrow, 7);
        assert!(top.starts_with("            ╭"), "56-wide at 80:\n{top}");
        // `str::len` is bytes and the box-drawing border is 3 bytes per
        // column — count columns, not bytes: 12 of pad + the 56-wide panel.
        assert_eq!(top.trim_end().chars().count(), 68, "the right edge:\n{top}");

        for (width, height) in [(100u16, 30u16), (120, 35)] {
            let wide = rendered_full(width, height, false);
            let y = (height - 9) / 2;
            let top = row_text(&wide, y);
            let pad = (width - 64) / 2;
            assert!(
                top.starts_with(&" ".repeat(pad as usize)),
                "64-wide at {width}:\n{top}"
            );
            assert!(
                top.chars().nth(pad as usize) == Some('╭'),
                "the border at {pad}:\n{top}"
            );
        }
    }

    /// §16.15's inline snapshots at the three ladder points §23 names:
    /// v = 14 (the full 9-row panel), v = 9 (exactly the panel), v = 6
    /// (the floor — upper rule, name and lower rule all dropped).
    #[test]
    fn inline_snapshot_v14() {
        snap("overlay_inline_mosque_v14", &rendered_inline(14, false));
    }

    #[test]
    fn inline_snapshot_v9() {
        snap("overlay_inline_mosque_v9", &rendered_inline(9, false));
    }

    #[test]
    fn inline_snapshot_v6() {
        snap("overlay_inline_mosque_v6", &rendered_inline(6, false));
    }

    /// `top_pad = (v − block) / 2`, remainder below (§16.15): at v = 14
    /// the 9-row panel sits two rows down with three below; at v = 9 it
    /// fills the viewport exactly.
    #[test]
    fn top_pad_puts_the_remainder_below() {
        let v14 = rendered_inline(14, false);
        assert!(
            row_is_blank(&v14, 0) && row_is_blank(&v14, 1),
            "two rows above"
        );
        assert!(row_text(&v14, 2).contains('╭'), "the panel's top border");
        assert!(
            row_is_blank(&v14, 11) && row_is_blank(&v14, 12) && row_is_blank(&v14, 13),
            "three rows below"
        );

        let v9 = rendered_inline(9, false);
        assert!(
            row_text(&v9, 0).contains('╭'),
            "the panel at row 0 at v = 9"
        );
        assert!(
            row_text(&v9, 8).contains('╰'),
            "the panel's foot at the last row"
        );
    }

    /// The ladder at the floor (v = 6): banner, data, Iqama, hints —
    /// the name row is gone (the banner already names the event) and
    /// so are both rules; below the floor (v = 5) the single truncated
    /// line, never a skipped overlay (§16.15).
    #[test]
    fn the_ladder_floor_and_the_line_below_it() {
        let v6 = text(&rendered_inline(6, false));
        assert!(v6.contains("TIME TO HEAD OUT"), "the banner holds:\n{v6}");
        assert!(
            v6.contains("Adhan was 15 minutes ago · 19:21"),
            "the data line:\n{v6}"
        );
        assert!(v6.contains(IQAMA), "the Iqama line:\n{v6}");
        assert!(
            !v6.to_uppercase().contains("MAGHRIB"),
            "the name row dropped:\n{v6}"
        );

        let v5 = rendered_inline(5, false);
        assert!(
            row_text(&v5, 0).contains(
                "◉ TIME TO HEAD OUT · MAGHRIB · Iqama ~5 min · [ENTER] dismiss  [S] snooze"
            ),
            "the stand-in line:\n{}",
            row_text(&v5, 0)
        );
        assert!(row_is_blank(&v5, 1), "nothing else renders");
    }

    /// §16.8 item 8: a late fire suffixes the data line — and the panel
    /// still fits its seven-row budget, so the hints survive.
    #[test]
    fn a_late_fire_carries_the_delayed_notice_suffix() {
        let full = text(&rendered_full(80, 24, true));
        assert!(
            full.contains("Adhan was 15 minutes ago · 19:21 (delayed notice)"),
            "the suffixed data line:\n{full}"
        );
        assert!(
            full.contains("[ENTER] DISMISS"),
            "the hints still fit:\n{full}"
        );

        let inline = text(&rendered_inline(14, true));
        assert!(inline.contains("(delayed notice)"), "inline:\n{inline}");
    }
}
