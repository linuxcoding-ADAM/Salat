//! The Adhan alert overlay (§16.7) — the full-screen call to prayer —
//! and its §16.15 inline variant.
//!
//! **Full screen (§16.7).** The whole frame, `redBg` wash, red border,
//! no header and no footer (the design's `isOverlay`).
//! `Vertical[ Length(3) top bar, Min(0) center, Length(2) bottom bar ]`,
//! the center vertically centered. The top bar carries `SALAT`, the
//! 500 ms blinking `◉ ADHAN` badge and the instant — `(delayed)`
//! beside it on a §11.5 late fire; the center the
//! labelled rule `─── PRAYER TIME ───`, the 3-row block prayer name,
//! the instant and location, the Arabic line (measured with
//! `unicode-width`, dropped — never wrapped — when it does not fit,
//! item 9), the transliteration, a rule and `Mosque reminder in N min`;
//! the bottom bar the `Adhan audio:` line and the two key hints.
//!
//! **Inline (§16.15).** The overlay owns the whole viewport, cleared
//! including the header. Eleven content rows — badge+time, blank,
//! 3-row name, blank, Arabic, location, blank, rule, hints — placed
//! by `top_pad = (v − block) / 2` with the remainder below, down the
//! drop ladder 11 → 9 → 8 → 7 → 6 → 5 → 3 to the bold-row floor, and
//! a single truncated line below that. The §16.15 diagram fixes the
//! inline geometry at those 11 content rows (its ladder arithmetic
//! only works with them), so the border box belongs to the full-frame
//! variant; inline, the red wash plus the red-styled content carry the
//! designed character.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use chrono::DateTime;
use chrono_tz::Tz;
use unicode_width::UnicodeWidthStr as _;

use crate::domain::Notifiable;
use crate::status::{ComponentId, Health};
use crate::ui::model::UiModel;
use crate::ui::theme::{Blink, Theme, blink, hhmm};
use crate::ui::widgets::bigtext;
use crate::ui::widgets::kbd_hint;
use crate::ui::widgets::panel::Panel;

use super::super::shell::Overlay;

/// §16.7 item 5's inline label for the rule.
const RULE_LABEL: &str = " PRAYER TIME ";

/// The Arabic line (item 9): measured with `unicode-width`, dropped —
/// never wrapped — when it does not fit. Broken shaping is worse than
/// absence; the transliteration below carries the meaning.
const ARABIC: &str = "اللَّهُ أَكْبَرُ · اللَّهُ أَكْبَرُ";

/// The transliteration that carries the Arabic line's meaning.
const TRANSLITERATION: &str = "Allahu Akbar · Prayer time has begun";

/// The audio line's fixed prefix (item 6's bottom bar).
const AUDIO_PREFIX: &str = "Adhan audio: ";

/// The Adhan overlay's payload, destructured for both renderers — the
/// dispatching `match` already proved the variant, so no refutable
/// pattern rides along.
pub(crate) struct AdhanAlertData {
    pub prayer: Notifiable,
    pub at: DateTime<Tz>,
    /// §11.5's late-fire flag — `true` renders `(delayed)` beside the
    /// instant in the top bar, so the user is never misled about
    /// timing. Only a live tick can hand the overlay a late fire; a
    /// catch-up fire raises no overlay at all.
    pub late: bool,
}

impl AdhanAlertData {
    /// Destructure the overlay into the payload; `None` for the other
    /// variant — the dispatcher never asks for what it did not match.
    fn of(overlay: Overlay) -> Option<Self> {
        match overlay {
            Overlay::AdhanAlert { prayer, at, late } => Some(Self { prayer, at, late }),
            Overlay::MosqueReminder { .. } => None,
        }
    }
}

/// Render the overlay (§16.7) into the whole frame — the caller has
/// already suppressed the header and footer.
pub fn render(overlay: Overlay, model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let Some(data) = AdhanAlertData::of(overlay) else {
        return;
    };

    // Full frame, red wash: `redBg` everywhere, then the red border.
    buf.set_style(area, Style::new().bg(theme.red_bg));
    let inner = Panel::new()
        .border(theme.red)
        .background(theme.red_bg)
        .render(area, buf, theme);

    let (top, center, bottom) = split_zones(inner);
    render_top(&data, top, buf, theme, model.now, model);
    render_center(&data, model, center, buf, theme);
    render_bottom(model, bottom, buf, theme);
}

/// `Vertical[ Length(3) top, Min(0) center, Length(2) bottom ]` over
/// the interior — the center gets what remains after both bars.
fn split_zones(inner: Rect) -> (Rect, Rect, Rect) {
    let top_height = 3.min(inner.height);
    let bottom_height = 2.min(inner.height - top_height);
    let center_height = inner.height - top_height - bottom_height;
    let top = Rect::new(inner.x, inner.y, inner.width, top_height);
    let center = Rect::new(inner.x, inner.y + top_height, inner.width, center_height);
    let bottom = Rect::new(
        inner.x,
        inner.y + top_height + center_height,
        inner.width,
        bottom_height,
    );
    (top, center, bottom)
}

/// The top bar (item 10): `SALAT` left, the blinking `◉ ADHAN` badge
/// centered, the instant right — on the bar's middle row.
fn render_top(
    data: &AdhanAlertData,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    now: DateTime<Tz>,
    model: &UiModel,
) {
    if area.width == 0 || area.height < 2 {
        return;
    }
    let y = area.y + 1;
    // §11.5: a late fire carries `(delayed)` in the overlay's top bar.
    let time = hhmm(data.at, &model.config.display.time_format);
    let time = if data.late {
        format!("{time} (delayed)")
    } else {
        time
    };

    let salat = Line::from(Span::styled("SALAT", Style::new().fg(theme.red)));
    buf.set_line(area.x, y, &salat, area.width);

    let right_x = area.x + area.width.saturating_sub(time.width() as u16);
    let right = Line::from(Span::styled(time, Style::new().fg(theme.text_sub)));
    buf.set_line(right_x, y, &right, area.width);

    let badge_on = Blink::new(blink::ADHAN_BADGE_MS).on(now);
    let badge = if badge_on {
        Line::from(Span::styled("◉ ADHAN", Style::new().fg(theme.red)))
    } else {
        Line::from(Span::styled("  ADHAN", Style::new().fg(theme.red_dim)))
    };
    let badge_x = area.x + (area.width.saturating_sub(badge.width() as u16)) / 2;
    buf.set_line(badge_x, y, &badge, area.width);
}

/// The center block (§16.7 items 5–6 and item 10's order), vertically
/// centered: the labelled rule, the 3-row name (blank-flanked), the
/// instant · location, the Arabic, the transliteration, a rule, and
/// the mosque-reminder note.
fn render_center(
    data: &AdhanAlertData,
    model: &UiModel,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let cfg = &model.config;
    let settings = data.prayer.settings(&cfg.prayers);
    let name = data.prayer.display();
    let instant = hhmm(data.at, &cfg.display.time_format);
    let instant_location = format!("{} · {}", instant, cfg.location.label.to_uppercase());
    let arabic_width = unicode_width::UnicodeWidthStr::width(ARABIC) as u16;
    let show_arabic = arabic_width <= area.width;

    let block = 1 /* labelled rule */        + 1 /* blank */
        + 3 /* name */
        + 1 /* blank */
        + 1 /* instant · location */
        + u16::from(show_arabic)
        + 1 /* transliteration */
        + 1 /* rule */
        + 1; /* reminder */
    let mut y = area.y + (area.height.saturating_sub(block)) / 2;

    y = labelled_rule(area, y, buf, theme.red, RULE_LABEL);
    y += 1; // the blank flanking the name

    let big = bigtext::width(name);
    if big <= area.width && y + 3 <= area.y + area.height {
        bigtext::render(
            Rect::new(area.x + (area.width - big) / 2, y, area.width, 3),
            buf,
            name,
            theme.red,
        );
        y += 3;
    } else {
        y = bold_row(area, y, buf, theme.red, name);
    }

    y += 1; // the second flanking blank
    y = centered_row(area, y, buf, theme.text, &instant_location);
    if show_arabic {
        let line = Line::from(Span::styled(ARABIC, Style::new().fg(theme.text)));
        let x = area.x + (area.width.saturating_sub(arabic_width)) / 2;
        if y < area.y + area.height {
            buf.set_line(x, y, &line, area.width);
        }
        y += 1;
    }
    y = centered_row(area, y, buf, theme.text_sub, TRANSLITERATION);
    y = rule_row(area, y, buf, theme.red);
    let reminder = format!("Mosque reminder in {} min", settings.after_minutes);
    centered_row(area, y, buf, theme.text_sub, &reminder);
}

/// The bottom bar (items 6 and 8): the `Adhan audio:` line and the two
/// key hints. No player or no file → `unavailable`; the visual
/// announcement is independent of playback.
fn render_bottom(model: &UiModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let audio_line = audio_line(model, theme);
    let y = area.y;
    let x = area.x + (area.width.saturating_sub(audio_line.width() as u16)) / 2;
    buf.set_line(x, y, &audio_line, area.width);

    if area.height > 1 {
        let hints = hint_row("SNOOZE 3m", theme);
        let x = area.x + (area.width.saturating_sub(hints.width() as u16)) / 2;
        buf.set_line(x, y + 1, &hints, area.width);
    }
}

/// The bottom bar's audio line: the preset and volume when both the
/// file and the player check out (the Status model already computed
/// the verdicts — rendering never probes a service), `unavailable`
/// otherwise (item 8).
fn audio_line(model: &UiModel, theme: &Theme) -> Line<'static> {
    let status = &model.status;
    let file_ready = status
        .component(ComponentId::AdhanSound)
        .is_some_and(|component| {
            component.health == Health::Ok && component.detail.starts_with("READY")
        });
    let player_ready = status
        .component(ComponentId::AudioSystem)
        .is_some_and(|component| component.health == Health::Ok);
    let text = if file_ready && player_ready {
        format!(
            "{AUDIO_PREFIX}{} · Vol {}%",
            preset_display(&model.config.adhan.preset),
            model.config.adhan.volume
        )
    } else {
        format!("{AUDIO_PREFIX}unavailable (no player found)")
    };
    Line::from(Span::styled(text, Style::new().fg(theme.text_sub)))
}

/// The two key hints on one row, three spaces apart (§16.7 item 5).
fn hint_row(snooze_label: &str, theme: &Theme) -> Line<'static> {
    let dismiss = kbd_hint::line("ENTER", "DISMISS", theme);
    let snooze = kbd_hint::line("S", snooze_label, theme);
    let mut spans: Vec<Span<'static>> = Vec::new();
    for span in dismiss.spans {
        spans.push(Span::styled(span.content.to_string(), span.style));
    }
    spans.push(Span::raw("   "));
    for span in snooze.spans {
        spans.push(Span::styled(span.content.to_string(), span.style));
    }
    Line::from(spans)
}

/// The §16.15 inline variant: the whole cleared viewport, eleven
/// content rows, the drop ladder, the bold-row floor and the single
/// truncated line below it.
pub fn render_inline(
    overlay: Overlay,
    model: &UiModel,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let Some(data) = AdhanAlertData::of(overlay) else {
        return;
    };
    let cfg = &model.config;
    let name = data.prayer.display().to_string();
    let instant = hhmm(data.at, &cfg.display.time_format);
    let location = cfg.location.label.to_uppercase();

    // The overlay owns the whole viewport: full red wash.
    buf.set_style(area, Style::new().bg(theme.red_bg));

    let arabic_width = unicode_width::UnicodeWidthStr::width(ARABIC) as u16;
    let mut rows: Vec<InlineRow> = vec![
        InlineRow::Badge,
        InlineRow::Blank,
        InlineRow::Name,
        InlineRow::Blank,
        InlineRow::Location,
        InlineRow::Blank,
        InlineRow::Rule,
        InlineRow::Hints,
    ];
    // The Arabic line is width-gated (§16.15's diagram: "dropped if too
    // wide"), not a ladder step — it slots in between the name and the
    // location line.
    if arabic_width <= area.width {
        rows.insert(4, InlineRow::Arabic);
    }

    // The ladder (§16.15's table): the two blanks flanking the name,
    // the blank above the rule, the Arabic, the rule, the location
    // line, then the name collapses to a single bold row — floor 3.
    while inline_height(&rows) > area.height {
        match drop_one(&mut rows) {
            LadderStep::Dropped => {}
            LadderStep::Floor => break,
        }
    }

    let block = inline_height(&rows);
    if rows.is_empty() || block > area.height {
        // Below the floor: the single truncated line — the overlay is
        // never skipped for want of space. It stands in for the top
        // bar, so a late fire keeps its `(delayed)` marker (§11.5's
        // "never misled about timing").
        let delayed = if data.late { " (delayed)" } else { "" };
        let line = format!("◉ ADHAN · {name} {instant}{delayed} · [ENTER] dismiss  [S] snooze");
        buf.set_stringn(
            area.x,
            area.y,
            line,
            area.width as usize,
            Style::new().fg(theme.red),
        );
        return;
    }

    let mut y = area.y + (area.height.saturating_sub(block)) / 2;
    for row in &rows {
        match row {
            InlineRow::Badge => {
                let badge_on = Blink::new(blink::ADHAN_BADGE_MS).on(model.now);
                let badge = if badge_on {
                    Line::from(Span::styled("◉ ADHAN", Style::new().fg(theme.red)))
                } else {
                    Line::from(Span::styled("  ADHAN", Style::new().fg(theme.red_dim)))
                };
                buf.set_line(area.x, y, &badge, area.width);
                // The badge row is the inline variant's top bar, so
                // §11.5's `(delayed)` marker lives here too — never on
                // the location row, which is center content.
                let badge_time = if data.late {
                    format!("{instant} (delayed)")
                } else {
                    instant.clone()
                };
                let time = Line::from(Span::styled(badge_time, Style::new().fg(theme.text_sub)));
                let x = area.x + area.width.saturating_sub(time.width() as u16);
                buf.set_line(x, y, &time, area.width);
                y += 1;
            }
            InlineRow::Blank => y += 1,
            InlineRow::Name => {
                let big = bigtext::width(&name);
                if big <= area.width && y + 3 <= area.y + area.height {
                    bigtext::render(
                        Rect::new(area.x + (area.width - big) / 2, y, area.width, 3),
                        buf,
                        &name,
                        theme.red,
                    );
                    y += 3;
                } else {
                    y = bold_row(area, y, buf, theme.red, &name);
                }
            }
            InlineRow::CollapsedName => {
                y = bold_row(area, y, buf, theme.red, &name);
            }
            InlineRow::Arabic => {
                let line = Line::from(Span::styled(ARABIC, Style::new().fg(theme.text)));
                let x = area.x + (area.width.saturating_sub(arabic_width)) / 2;
                if y < area.y + area.height {
                    buf.set_line(x, y, &line, area.width);
                }
                y += 1;
            }
            InlineRow::Location => {
                let text = format!("{instant} · {location}");
                y = centered_row(area, y, buf, theme.text, &text);
            }
            InlineRow::Rule => {
                y = rule_row(area, y, buf, theme.red);
            }
            InlineRow::Hints => {
                let hints = hint_row("SNOOZE 3m", theme);
                if y < area.y + area.height {
                    buf.set_line(area.x, y, &hints, area.width);
                }
                y += 1;
            }
        }
    }
}

/// One content row of the inline layout, tagged for the dropper.
enum InlineRow {
    /// The `◉ ADHAN … 19:21` row.
    Badge,
    /// A spacer row.
    Blank,
    /// The prayer name: 3-row blocks, collapsing to one bold row.
    Name,
    /// The name's collapsed form — one bold row (the ladder's floor).
    CollapsedName,
    /// The Arabic takbīr line.
    Arabic,
    /// `19:21 · SÉTIF, ALGERIA`.
    Location,
    /// A full-width rule.
    Rule,
    /// `[ENTER] DISMISS   [S] SNOOZE 3m`.
    Hints,
}

/// The block's height — the §16.15 ladder's row accounting.
fn inline_height(rows: &[InlineRow]) -> u16 {
    rows.iter()
        .map(|row| match row {
            InlineRow::Name => 3,
            _ => 1,
        })
        .sum()
}

/// One step of §16.15's drop ladder: the blanks (the two flanking the
/// name, then the one above the rule — all three come off before
/// anything else), then the Arabic, the rule, the location line, then
/// the name's collapse. Returns `Floor` when nothing is left to drop.
fn drop_one(rows: &mut Vec<InlineRow>) -> LadderStep {
    let is_blank = |row: &InlineRow| matches!(row, InlineRow::Blank);
    let is_arabic = |row: &InlineRow| matches!(row, InlineRow::Arabic);
    let is_rule = |row: &InlineRow| matches!(row, InlineRow::Rule);
    let is_location = |row: &InlineRow| matches!(row, InlineRow::Location);

    for kind in [
        &is_blank as &dyn Fn(&InlineRow) -> bool,
        &is_arabic,
        &is_rule,
        &is_location,
    ] {
        if let Some(index) = rows.iter().position(kind) {
            rows.remove(index);
            return LadderStep::Dropped;
        }
    }
    // The name's collapse: 3-row blocks → one bold row (−2).
    if let Some(index) = rows.iter().position(|row| matches!(row, InlineRow::Name)) {
        rows[index] = InlineRow::CollapsedName;
        return LadderStep::Dropped;
    }
    LadderStep::Floor
}

/// Whether a ladder step dropped a row or hit the floor.
enum LadderStep {
    Dropped,
    Floor,
}

/// A rule with an inline label: `─── LABEL ───` across `area.width`.
/// Returns the next row's y.
fn labelled_rule(area: Rect, y: u16, buf: &mut Buffer, color: Color, label: &str) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    let width = area.width as usize;
    let label_width = label.chars().count();
    let left = (width.saturating_sub(label_width)) / 2;
    let right = width.saturating_sub(label_width) - left;
    let rule = format!("{}{}{}", "─".repeat(left), label, "─".repeat(right));
    buf.set_stringn(area.x, y, rule, width, Style::new().fg(color));
    y + 1
}

/// A full-width rule. Returns the next row's y.
fn rule_row(area: Rect, y: u16, buf: &mut Buffer, color: Color) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    let rule = "─".repeat(area.width as usize);
    buf.set_stringn(area.x, y, rule, area.width as usize, Style::new().fg(color));
    y + 1
}

/// One centered text row. Returns the next row's y.
fn centered_row(area: Rect, y: u16, buf: &mut Buffer, color: Color, text: &str) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    let width = text.width() as u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    buf.set_stringn(x, y, text, area.width as usize, Style::new().fg(color));
    y + 1
}

/// One centered bold row — the name's collapsed form. Returns the next
/// row's y.
fn bold_row(area: Rect, y: u16, buf: &mut Buffer, color: Color, text: &str) -> u16 {
    if y >= area.y + area.height {
        return y;
    }
    let width = text.width() as u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    buf.set_stringn(
        x,
        y,
        text,
        area.width as usize,
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    );
    y + 1
}

/// The preset's display form (§16.7 item 6's `Abdul Basit`): the kebab
/// case becomes Title Case.
fn preset_display(preset: &str) -> String {
    preset
        .split(['-', '_'])
        .map(|word| {
            let mut characters = word.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// §23 Phase 10 item 8's overlay suite: snapshots at the three full-screen
// sizes and in the inline viewport (§16.15), the whole-viewport clear, the
// top_pad arithmetic, the floors, the §11.5 `(delayed)` wording, the
// §16.7 item 9 Arabic rule and the item 8 unavailable line.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::testing;
    use crate::domain::testing as dt;
    use crate::status::Component;
    use crate::ui::theme::RenderMode;

    /// The §20.3 fixture model — 17:38:23, Maghrib next, both audio
    /// components READY (the fixture's scripted player and owned file),
    /// so the default bottom bar reads the healthy `Abdul Basit` line.
    fn model() -> UiModel {
        let fixture = testing::snapshot_runtime();
        crate::ui::model::build_model(&fixture.runtime)
    }

    /// The §20.3 specimen overlay — Maghrib at 19:21.
    fn overlay(late: bool) -> Overlay {
        Overlay::AdhanAlert {
            prayer: Notifiable::Maghrib,
            at: dt::maghrib_instant().at,
            late,
        }
    }

    fn theme() -> Theme {
        Theme::for_mode(RenderMode::Truecolor)
    }

    /// The full-screen §16.7 overlay at one terminal size.
    fn rendered_full(width: u16, height: u16, late: bool) -> Buffer {
        let model = model();
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        render(overlay(late), &model, buf.area, &mut buf, &theme());
        buf
    }

    /// The §16.15 inline overlay in a `v`-row viewport `width` columns
    /// wide — quick mode's cleared viewport, header row included.
    fn rendered_inline(v: u16, late: bool, width: u16) -> Buffer {
        let model = model();
        let mut buf = Buffer::empty(Rect::new(0, 0, width, v));
        render_inline(overlay(late), &model, buf.area, &mut buf, &theme());
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

    /// §20.3's reviewable form: the buffer filed under `tests/snapshots`
    /// (accept with `INSTA_UPDATE=always`, and read the diff — a
    /// snapshot is a design decision).
    fn snap(name: &str, buf: &Buffer) {
        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
        settings.bind(|| insta::assert_snapshot!(name, text(buf)));
    }

    /// One row's foreground text.
    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_owned())
            .collect()
    }

    /// Whether a row carries no foreground content — only the red wash.
    fn row_is_blank(buf: &Buffer, y: u16) -> bool {
        (0..buf.area.width).all(|x| buf[(x, y)].symbol() == " ")
    }

    /// §16.15: "the overlay owns the whole viewport … the header line is
    /// taken over as well" — every cell of every row carries the red
    /// wash, so nothing of the frame beneath survives.
    fn whole_viewport_is_red(buf: &Buffer) {
        let red_bg = theme().red_bg;
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                assert_eq!(
                    buf[(x, y)].bg,
                    red_bg,
                    "cell ({x},{y}) escaped the red wash"
                );
            }
        }
    }

    /// §16.7 item 10's content, in the 100×30 form.
    #[test]
    fn full_screen_carries_the_designed_content() {
        let text = text(&rendered_full(100, 30, false));
        for word in [
            "SALAT",
            "ADHAN",
            "─── PRAYER TIME ───",
            "19:21 · SÉTIF, ALGERIA",
            ARABIC,
            TRANSLITERATION,
            "Mosque reminder in 15 min",
            "Adhan audio: Abdul Basit · Vol 80%",
            "[ENTER] DISMISS",
            "[S] SNOOZE 3m",
        ] {
            assert!(text.contains(word), "missing {word:?} in:\n{text}");
        }
        assert!(!text.contains("(delayed)"), "an on-time fire: {text}");
    }

    /// The three full-screen sizes, snapshotted (§23 item 8).
    #[test]
    fn full_screen_snapshot_80x24() {
        snap("adhan_alert_80x24", &rendered_full(80, 24, false));
    }

    #[test]
    fn full_screen_snapshot_100x30() {
        snap("adhan_alert_100x30", &rendered_full(100, 30, false));
    }

    #[test]
    fn full_screen_snapshot_120x35() {
        snap("adhan_alert_120x35", &rendered_full(120, 35, false));
    }

    /// §16.7 item 4: the full frame is the `redBg` wash — no header, no
    /// footer, no ground beneath.
    #[test]
    fn the_full_frame_is_the_red_wash() {
        whole_viewport_is_red(&rendered_full(80, 24, false));
        whole_viewport_is_red(&rendered_full(100, 30, false));
        whole_viewport_is_red(&rendered_full(120, 35, false));
    }

    /// §16.15's inline snapshots at the three ladder points §23 names:
    /// v = 14 (full 11-row block), v = 11 (exactly the block), v = 3
    /// (the bold-row floor).
    #[test]
    fn inline_snapshot_v14() {
        snap("overlay_inline_adhan_v14", &rendered_inline(14, false, 80));
    }

    #[test]
    fn inline_snapshot_v11() {
        snap("overlay_inline_adhan_v11", &rendered_inline(11, false, 80));
    }

    #[test]
    fn inline_snapshot_v3() {
        snap("overlay_inline_adhan_v3", &rendered_inline(3, false, 80));
    }

    /// §16.15: the inline overlay clears the whole viewport — the red
    /// wash reaches every cell, the header row included.
    #[test]
    fn the_inline_overlay_clears_the_whole_viewport() {
        whole_viewport_is_red(&rendered_inline(14, false, 80));
    }

    /// `top_pad = (v − block) / 2` by integer division, remainder below
    /// (§16.15): at v = 14 the 11-row block sits one row down with two
    /// below; at v = 11 it fills the viewport exactly, badge first.
    #[test]
    fn top_pad_puts_the_remainder_below() {
        let v14 = rendered_inline(14, false, 80);
        assert!(row_is_blank(&v14, 0), "one row above at v = 14");
        assert!(row_text(&v14, 1).contains("ADHAN"), "the badge row");
        assert!(
            row_is_blank(&v14, 12) && row_is_blank(&v14, 13),
            "two rows below"
        );

        let v11 = rendered_inline(11, false, 80);
        assert!(
            row_text(&v11, 0).contains("ADHAN"),
            "badge at row 0 at v = 11"
        );
        assert!(!row_is_blank(&v11, 10), "the hints fill the last row");
    }

    /// Below the floor (v < 3): the single truncated line — the overlay
    /// is never skipped for want of space (§16.15).
    #[test]
    fn below_the_floor_a_single_truncated_line() {
        let v2 = rendered_inline(2, false, 80);
        let lines = lines(&v2);
        assert!(
            lines[0].contains("◉ ADHAN · MAGHRIB 19:21 · [ENTER] dismiss  [S] snooze"),
            "the stand-in line:\n{}",
            lines[0]
        );
        assert!(row_is_blank(&v2, 1), "nothing else renders");
    }

    /// §16.7 item 9's rule, as a mechanism: the Arabic line is measured
    /// with `unicode-width` and **dropped, never wrapped** when it does
    /// not fit. At 80 columns it fits whole (the Figma line measures
    /// ~21 columns — §23's "at 80 columns" wording assumed otherwise;
    /// the drop itself is exercised at a sub-Arabic width), and at 20
    /// columns no Arabic codepoint survives anywhere in the viewport —
    /// a wrap would have left a fragment.
    #[test]
    fn the_arabic_line_is_dropped_not_wrapped() {
        let wide = text(&rendered_inline(14, false, 80));
        assert!(wide.contains(ARABIC), "whole at 80 columns:\n{wide}");

        let narrow = text(&rendered_inline(14, false, 20));
        assert!(
            !narrow
                .chars()
                .any(|c| ('\u{0600}'..='\u{06FF}').contains(&c)),
            "Arabic survived a 20-column viewport:\n{narrow}"
        );
        // The §16.15 diagram carries no transliteration row inline —
        // the name and the instant carry the meaning there — so the
        // assertion is the drop, not a substitute line. The location
        // line is 22 columns wide, so at 20 it centers at x = 0 and
        // clips after `ALGER` — assert the head that always survives.
        assert!(
            narrow.contains("19:21 · SÉTIF"),
            "the location line survives the drop:\n{narrow}"
        );
    }

    /// §11.5: a live late fire carries `(delayed)` beside the instant —
    /// in the full-screen top bar, in the inline badge row, and in the
    /// below-floor stand-in line, so the user is never misled about
    /// timing at any ladder point.
    #[test]
    fn a_late_fire_carries_the_delayed_wording() {
        let full = text(&rendered_full(80, 24, true));
        assert!(full.contains("19:21 (delayed)"), "the top bar:\n{full}");

        let inline = text(&rendered_inline(14, true, 80));
        assert!(
            inline.contains("19:21 (delayed)"),
            "the badge row:\n{inline}"
        );

        let floor = text(&rendered_inline(2, true, 80));
        assert!(
            floor.contains("19:21 (delayed)"),
            "the stand-in line:\n{floor}"
        );
    }

    /// §16.7 item 8 / §23 item 8: no player (or no file) degrades the
    /// bottom bar to `unavailable` — and the overlay still displays:
    /// the visual announcement is independent of playback (§10.1).
    #[test]
    fn no_player_degrades_the_bottom_bar_not_the_overlay() {
        let mut model = model();
        let mut status = (*model.status).clone();
        status.components = status
            .components
            .iter()
            .map(|component| match component.id {
                ComponentId::AudioSystem => Component::new(
                    ComponentId::AudioSystem,
                    Health::Degraded,
                    "NO AUDIO PLAYER",
                ),
                ComponentId::AdhanSound => {
                    Component::new(ComponentId::AdhanSound, Health::Degraded, "NO ADHAN FILE")
                }
                _ => component.clone(),
            })
            .collect();
        model.status = std::sync::Arc::new(status);

        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        render(overlay(false), &model, buf.area, &mut buf, &theme());
        let text = text(&buf);
        assert!(
            text.contains("Adhan audio: unavailable (no player found)"),
            "the degraded bottom bar:\n{text}"
        );
        assert!(text.contains("PRAYER TIME"), "the overlay held:\n{text}");
    }
}
