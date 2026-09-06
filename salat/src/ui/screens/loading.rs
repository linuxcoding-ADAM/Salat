//! The LOADING screen (§16.11) — the boot screen `--full` shows while
//! [`crate::app::boot`] walks §15.7's ladder. It owns the whole frame:
//! no header, no footer (§3.3).
//!
//! Every frame is a pure function of the latest `BootState` and the
//! clock: the active step's `▶` blinks at 300 ms, its label grows one
//! to three dots, and the bar tracks `stage / 5`. A failing stage
//! turns its glyph `✕` red, the panel border red — then, after a
//! 400 ms beat the caller holds, the ERROR screen replaces this one.

use chrono::DateTime;
use chrono_tz::Tz;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget as _;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::app::boot::{SHORT_STEPS, STEPS};
use crate::ui::layout::Breakpoint;
use crate::ui::theme::{Blink, Theme, blink};
use crate::ui::widgets::bar;
use crate::ui::widgets::panel::Panel;

/// The panel's inner height: brand 3, steps 5, progress 2 (§16.11
/// item 2's `Vertical[Length(3), Length(5), Length(2)]`).
const INNER_HEIGHT: u16 = 10;

/// Render the loading screen over the whole frame.
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    state: &crate::app::boot::BootState,
    now: DateTime<Tz>,
    breakpoint: Breakpoint,
) {
    let small = breakpoint == Breakpoint::Small;
    // §16.11 item 8: `min(52, w − 4)` at 80×24; the design's
    // `clamp(55%, …)` otherwise, capped where the labels stop growing.
    let panel_width = if small {
        52.min(area.width.saturating_sub(4))
    } else {
        ((area.width as u32 * 55 / 100) as u16).clamp(36, 56)
    };
    let panel_width = panel_width.max(4).min(area.width);
    let panel_height = INNER_HEIGHT + 2;
    let panel_x = area.x + (area.width.saturating_sub(panel_width)) / 2;
    let panel_y = area.y + (area.height.saturating_sub(panel_height)) / 2;
    let rect = Rect::new(panel_x, panel_y, panel_width, panel_height.min(area.height));

    let failed = state.failed();
    let panel = Panel::new().title("INITIALIZING");
    let panel = if failed {
        panel.border(theme.red)
    } else {
        panel
    };
    let inner = panel.render(rect, buf, theme);
    if inner.width < 4 || inner.height < INNER_HEIGHT {
        return;
    }

    // Brand: `SALAT` green centered, `v0.1.0 · Rust · Ratatui ·
    // Crossterm` dim beneath (rows 0–1, row 2 blank).
    centered(
        inner.x,
        inner.y,
        inner.width,
        buf,
        Line::from(Span::styled(
            "SALAT",
            Style::new().fg(theme.green).add_modifier(Modifier::BOLD),
        )),
    );
    centered(
        inner.x,
        inner.y + 1,
        inner.width,
        buf,
        Line::from(Span::styled(
            concat!(
                "v",
                env!("CARGO_PKG_VERSION"),
                " · Rust · Ratatui · Crossterm"
            ),
            Style::new().fg(theme.text_dim),
        )),
    );

    // The five steps (rows 3–7).
    let blink = Blink::new(blink::LOADING_MS);
    for (index, label) in STEPS.iter().enumerate() {
        let label = if small { SHORT_STEPS[index] } else { label };
        let row = inner.y + 3 + index as u16;
        let (glyph, glyph_style, text, text_style) =
            step_visual(index, state, &blink, now, label, theme);
        let mut spans = vec![
            Span::styled(glyph, glyph_style),
            Span::raw("  "),
            Span::styled(text, text_style),
        ];
        // The active label's animated dots: one to three, cycling at
        // the blink cadence.
        if index as u8 == state.stage && !failed {
            let dots = 1 + (now.timestamp_subsec_millis() / 300) % 3;
            spans.push(Span::styled(
                ".".repeat(dots as usize),
                Style::new().fg(theme.text_sub),
            ));
        }
        Line::from(spans).render(Rect::new(inner.x + 1, row, inner.width - 1, 1), buf);
    }

    // Progress (rows 8–9): the 38-cell bar (30 at 80×24), then
    // `{pct}% · {location}`.
    let pct = (state.stage as f64 / STEPS.len() as f64).clamp(0.0, 1.0);
    let bar_width = if small { 30 } else { 38 }.min(inner.width - 2);
    bar::render(
        Rect::new(
            inner.x + 1 + (inner.width.saturating_sub(bar_width + 2)) / 2,
            inner.y + 8,
            inner.width,
            1,
        ),
        buf,
        theme,
        pct,
        bar_width,
        None,
    );
    // §16.11 item 6: the location label once step 2 (Resolving
    // location) has completed — `starting…` before that.
    let location = if state.stage >= 2 {
        state.location.as_deref().unwrap_or("starting…")
    } else {
        "starting…"
    };
    centered(
        inner.x,
        inner.y + 9,
        inner.width,
        buf,
        Line::from(Span::styled(
            format!("{}% · {location}", (pct * 100.0).round() as u16),
            Style::new().fg(theme.text_dim),
        )),
    );
}

/// One step's `(glyph, glyph style, label, label style)` — done ✓
/// green, active ▶ yellow blinking, failed ✕ red, future ○ dim.
fn step_visual<'a>(
    index: usize,
    state: &crate::app::boot::BootState,
    blink: &Blink,
    now: DateTime<Tz>,
    label: &'a str,
    theme: &Theme,
) -> (&'static str, Style, &'a str, Style) {
    if index < state.stage as usize {
        (
            "✓",
            Style::new().fg(theme.green),
            label,
            Style::new().fg(theme.text),
        )
    } else if index as u8 == state.stage {
        if state.failed() {
            (
                "✕",
                Style::new().fg(theme.red),
                label,
                Style::new().fg(theme.red),
            )
        } else {
            // The blinking marker: `▶` on the beat, blank off it.
            let glyph = if blink.on(now) { "▶" } else { " " };
            (
                glyph,
                Style::new().fg(theme.yellow),
                label,
                Style::new().fg(theme.text_sub),
            )
        }
    } else {
        (
            "○",
            Style::new().fg(theme.text_dim).add_modifier(Modifier::DIM),
            label,
            Style::new().fg(theme.text_dim).add_modifier(Modifier::DIM),
        )
    }
}

/// Render one centered line at `y`.
fn centered(x: u16, y: u16, width: u16, buf: &mut Buffer, line: Line) {
    let line_width = line.width() as u16;
    let start = x + width.saturating_sub(line_width) / 2;
    line.render(Rect::new(start, y, width, 1), buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::boot::BootState;
    use crate::error::SalatError;

    fn theme() -> Theme {
        Theme::for_mode(crate::ui::theme::RenderMode::Truecolor)
    }

    fn render_frame(state: &BootState, width: u16, height: u16, breakpoint: Breakpoint) -> Buffer {
        let now =
            chrono::TimeZone::with_ymd_and_hms(&chrono_tz::Africa::Algiers, 2026, 9, 5, 17, 38, 23)
                .unwrap();
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        render(buf.area, &mut buf, &theme(), state, now, breakpoint);
        buf
    }

    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_owned())
            .collect()
    }

    /// §23 Phase 6's snapshots: the five stages' frames at 80×24 —
    /// step glyphs and the progress label advance together.
    #[test]
    fn the_five_stages_snapshot() {
        for stage in 0u8..=5 {
            let state = BootState {
                stage,
                error: None,
                location: (stage >= 2).then(|| "Sétif, Algeria".to_owned()),
            };
            let buf = render_frame(&state, 80, 24, Breakpoint::Small);
            let lines = (0..buf.area.height)
                .map(|y| row(&buf, y))
                .collect::<Vec<_>>()
                .join("\n");
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
            settings.bind(|| insta::assert_snapshot!(format!("loading_stage_{stage}"), lines));
        }
    }

    /// Stage 2: two steps done, the third active, the location label
    /// now shown instead of `starting…`.
    #[test]
    fn stage_two_shows_done_active_and_the_location() {
        let state = BootState {
            stage: 2,
            error: None,
            location: Some("Sétif, Algeria".to_owned()),
        };
        let buf = render_frame(&state, 80, 24, Breakpoint::Small);
        let text = (0..buf.area.height)
            .map(|y| row(&buf, y))
            .collect::<String>();
        assert!(text.contains("✓"), "done steps carry the check");
        assert!(text.contains("Prayer times"), "the short label at Small");
        assert!(text.contains("○"), "future steps are dimmed circles");
        assert!(
            text.contains("40% · Sétif, Algeria"),
            "stage 2/5 with the location: {text}"
        );
    }

    /// Before stage 2 the label reads `starting…`.
    #[test]
    fn before_location_the_label_reads_starting() {
        let state = BootState {
            stage: 0,
            error: None,
            location: None,
        };
        let buf = render_frame(&state, 80, 24, Breakpoint::Small);
        let text = (0..buf.area.height)
            .map(|y| row(&buf, y))
            .collect::<String>();
        assert!(text.contains("0% · starting…"), "{text}");
    }

    /// §16.11 item 5: the failing stage renders `✕`.
    #[test]
    fn a_failed_stage_shows_the_cross() {
        let state = BootState {
            stage: 2,
            error: Some(SalatError::PrayerTimesUnavailable),
            location: Some("Sétif, Algeria".to_owned()),
        };
        let buf = render_frame(&state, 80, 24, Breakpoint::Small);
        let text = (0..buf.area.height)
            .map(|y| row(&buf, y))
            .collect::<String>();
        assert!(text.contains('✕'), "{text}");
        // Everything before it still shows its ✓ — glyph, two spaces,
        // then the label (the row layout every step shares).
        assert!(
            text.contains("✓  Loading configuration") || {
                // At Small the label is short — assert the short form too.
                text.contains("✓  Configuration")
            }
        );
    }

    /// Above Small the full labels show.
    #[test]
    fn medium_terminals_show_the_full_labels() {
        let state = BootState::starting();
        let buf = render_frame(&state, 100, 30, Breakpoint::Medium);
        let text = (0..buf.area.height)
            .map(|y| row(&buf, y))
            .collect::<String>();
        assert!(text.contains("Loading configuration"), "{text}");
    }
}
