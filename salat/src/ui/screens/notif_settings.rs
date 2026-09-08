//! The Notification Settings screen (§16.5) — all five prayers'
//! before/after minutes and Adhan/notify toggles in one dense grid,
//! with a timeline preview showing what the numbers mean.
//!
//! Every value renders from `pending_config()` (§8.5): the draft's
//! working copy while an edit session is in flight, the effective
//! config otherwise. The cursor is `NotifState { row, col }` — a row
//! per prayer, a column per field — and `←→` edits the focused cell
//! in place through the shared draft, which is what "values apply
//! immediately after save" previews here.
//!
//! The design's small size drops the ADHAN and NOTIF columns (§16.5
//! item 9): 20 body rows budget 1 crumb + 8 grid + 7 timeline + 4
//! buttons, and those toggles remain reachable on the PRAYERS screen.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::prelude::Widget as _;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::domain::prayer::Notifiable;
use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::shell::{NotifCol, NotifState};
use crate::ui::theme::Theme;
use crate::ui::widgets::crumb;
use crate::ui::widgets::hr;
use crate::ui::widgets::panel::Panel;
use crate::ui::widgets::stepper::Stepper;

use super::settings::render_buttons;

/// Render the notification matrix (§16.5): crumb, the PER-PRAYER
/// TIMING grid, the TIMELINE PREVIEW, the SAVE/CANCEL row.
pub fn render(
    model: &UiModel,
    state: &NotifState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    breakpoint: Breakpoint,
) {
    let small = breakpoint == Breakpoint::Small;
    let config = model.pending_config();

    let [crumb_area, grid, timeline, buttons] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(11),
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
        "SETTINGS › NOTIFICATIONS",
        Some("↑↓ row · TAB col · ←→ adjust"),
    );

    render_grid(config, state, grid, buf, theme, small);
    render_timeline(config, state, timeline, buf, theme, small);
    render_buttons(buttons, buf, theme);
}

/// The grid panel: a header row, a rule, one row per prayer. The
/// focused cell is the `Stepper` (minutes) or the ON/OFF toggle the
/// design colours; the focused row carries the active ground and the
/// green left bar the design's `borderLeft` becomes in a terminal.
fn render_grid(
    config: &crate::config::Config,
    state: &NotifState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    small: bool,
) {
    if area.height < 3 {
        return;
    }
    let inner = Panel::new()
        .title("PER-PRAYER TIMING")
        .render(area, buf, theme);
    if inner.height < 2 {
        return;
    }

    // The design's columns: `100px 1fr 1fr 58px 58px`, the last two
    // dropped at small — laid out over the panel's interior, one
    // column in from the left border (the design's 6px row padding).
    let [prayer_col, before_col, after_col, adhan_col, notif_col] =
        columns(inner.x + 1, inner.width.saturating_sub(2), small);

    // Header (textDim, the design's 0.72em letterspaced label), each
    // label centred over its own column — a header must align with
    // the grid below it, so it renders per column, not as one line.
    header_label(
        if small { "BEFORE" } else { "BEFORE ADHAN" },
        before_col,
        inner,
        buf,
        theme,
    );
    header_label(
        if small { "AFTER" } else { "AFTER ADHAN" },
        after_col,
        inner,
        buf,
        theme,
    );
    if !small {
        header_label("ADHAN", adhan_col, inner, buf, theme);
        header_label("NOTIF", notif_col, inner, buf, theme);
    }
    Line::from(Span::styled("   PRAYER", Style::new().fg(theme.text_dim)))
        .render(Rect::new(prayer_col.x, inner.y, prayer_col.width, 1), buf);
    // The rule under it, the design's `HR my={2}`.
    if inner.height >= 2 {
        hr::render(
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
            buf,
            theme,
            Some(theme.border_dim),
        );
    }
    if inner.height < 3 {
        return;
    }

    // One row per prayer, the cursor row carrying the design's
    // `rowActive` ground, green name, and `▶` marker.
    let rows_area = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(2),
    );
    for (index, prayer) in Notifiable::ALL.iter().enumerate() {
        let y = rows_area.y + index as u16;
        if y >= rows_area.bottom() {
            break;
        }
        let selected = index == state.row;
        let settings = prayer.settings(&config.prayers);
        let ground = if selected {
            Style::new().bg(theme.row_active)
        } else {
            Style::new()
        };
        // The ground stays inside the panel — the border columns are
        // the panel's, not the row's.
        let in_width = rows_area.width.saturating_sub(2);
        buf.set_style(Rect::new(rows_area.x + 1, y, in_width, 1), ground);

        // PRAYER — `▶ FAJR` green and bold on the cursor row, the
        // plain name in textSub otherwise; the `▌` bar is the
        // design's 2px green left border.
        let name_style = if selected {
            Style::new().fg(theme.green).patch(ground)
        } else {
            Style::new().fg(theme.text_sub).patch(ground)
        };
        let marker = if selected { "▌▶ " } else { "   " };
        Line::from(vec![
            Span::styled(marker, Style::new().fg(theme.green).patch(ground)),
            Span::styled(prayer.display(), name_style),
        ])
        .render(Rect::new(prayer_col.x + 1, y, prayer_col.width, 1), buf);

        // BEFORE/AFTER — steppers; the focused one carries rowSel.
        render_cell(
            Stepper::new(&format!("{} min", settings.before_minutes))
                .focused(selected && state.col == NotifCol::Before)
                .at_min(settings.before_minutes == 0)
                .at_max(settings.before_minutes == 60),
            before_col,
            y,
            buf,
            theme,
        );
        render_cell(
            Stepper::new(&format!("{} min", settings.after_minutes))
                .focused(selected && state.col == NotifCol::After)
                .at_min(settings.after_minutes == 0)
                .at_max(settings.after_minutes == 60),
            after_col,
            y,
            buf,
            theme,
        );

        // ADHAN/NOTIF — ON green, OFF textDim (the design's exact
        // vocabulary), dropped at small.
        if !small {
            toggle(settings.adhan, adhan_col, y, buf, theme, ground);
            toggle(settings.notify, notif_col, y, buf, theme, ground);
        }
    }
}

/// The five column rects — `[prayer, before, after, adhan, notif]`,
/// positioned at `x` across `width`. The last two are zero-width at
/// small (the design drops them).
fn columns(x: u16, width: u16, small: bool) -> [Rect; 5] {
    let [prayer, before, after, adhan, notif] = if small {
        Layout::horizontal([
            Constraint::Length(10),
            Constraint::Min(0),
            Constraint::Min(0),
            Constraint::Length(0),
            Constraint::Length(0),
        ])
        .areas(Rect::new(x, 0, width, 1))
    } else {
        Layout::horizontal([
            Constraint::Length(13),
            Constraint::Min(0),
            Constraint::Min(0),
            Constraint::Length(7),
            Constraint::Length(7),
        ])
        .areas(Rect::new(x, 0, width, 1))
    };
    [prayer, before, after, adhan, notif]
}

/// One header label, centred over its column on the grid's first
/// inner row.
fn header_label(text: &str, col: Rect, inner: Rect, buf: &mut Buffer, theme: &Theme) {
    if col.width == 0 {
        return;
    }
    let style = Style::new().fg(theme.text_dim);
    let pad = col.width.saturating_sub(text.len() as u16) / 2;
    Line::from(Span::styled(
        format!("{}{text}", " ".repeat(pad as usize)),
        style,
    ))
    .render(Rect::new(col.x, inner.y, col.width, 1), buf);
}

/// One stepper cell — `Stepper` centres itself; the wrapper exists so
/// the two call sites stay one line each.
fn render_cell(stepper: Stepper<'_>, col: Rect, y: u16, buf: &mut Buffer, theme: &Theme) {
    if col.width == 0 {
        return;
    }
    stepper.render(Rect::new(col.x, y, col.width, 1), buf, theme);
}

/// One ON/OFF toggle, centred in its column.
fn toggle(on: bool, col: Rect, y: u16, buf: &mut Buffer, theme: &Theme, ground: Style) {
    if col.width == 0 {
        return;
    }
    let color = if on { theme.green } else { theme.text_dim };
    let text = if on { "ON" } else { "OFF" };
    let pad = col.width.saturating_sub(2) / 2;
    Line::from(Span::styled(
        format!("{}{text}", " ".repeat(pad as usize)),
        Style::new().fg(color).patch(ground),
    ))
    .render(Rect::new(col.x, y, col.width, 1), buf);
}

/// The timeline preview: one row per prayer, `◆ ─5m─ ◉ adhan ─15m─
/// ◉ mosque` — the relative offsets being edited, the numbers the
/// user is about to save made visible as the distances they are
/// (§16.5 item 8: these do not depend on the API, so there are no
/// absolute labels to lose). The cursor row is full strength; the
/// others dim to the design's 0.35 opacity, `textDim` in a terminal.
fn render_timeline(
    config: &crate::config::Config,
    state: &NotifState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    small: bool,
) {
    if area.height < 3 {
        return;
    }
    let inner = Panel::new()
        .title("TIMELINE PREVIEW")
        .render(area, buf, theme);

    for (index, prayer) in Notifiable::ALL.iter().enumerate() {
        let y = inner.y + index as u16;
        if y >= inner.bottom() {
            break;
        }
        let selected = index == state.row;
        let settings = prayer.settings(&config.prayers);
        let strength = |color: Color| {
            if selected { color } else { theme.text_dim }
        };
        let name = if small {
            prayer.short()
        } else {
            prayer.display()
        };
        Line::from(vec![
            Span::styled(
                format!("{name:<width$}", width = 8),
                Style::new().fg(strength(theme.text_sub)),
            ),
            Span::styled("◆", Style::new().fg(strength(theme.yellow))),
            Span::styled(
                format!("─{}m─", settings.before_minutes),
                Style::new().fg(theme.text_dim),
            ),
            Span::styled("◉ adhan", Style::new().fg(strength(theme.red))),
            Span::styled(
                format!("─{}m─", settings.after_minutes),
                Style::new().fg(theme.text_dim),
            ),
            Span::styled("◉ mosque", Style::new().fg(strength(theme.red))),
        ])
        .render(Rect::new(inner.x, y, inner.width, 1), buf);
    }
}

/// §16.5 item 7's column cycle — `Tab` forward, `Shift-Tab` back,
/// over the four columns.
pub(crate) fn cycle_col(col: NotifCol, forward: bool) -> NotifCol {
    use NotifCol as N;
    let order = [N::Before, N::After, N::Adhan, N::Notif];
    let at = order
        .iter()
        .position(|candidate| *candidate == col)
        .unwrap_or_default();
    let next = if forward {
        (at + 1) % order.len()
    } else {
        (at + order.len() - 1) % order.len()
    };
    order[next]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::testing;

    fn rendered(width: u16, height: u16, state: &NotifState) -> Vec<String> {
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

    /// §16.5's snapshot: the screen at all three sizes.
    #[test]
    fn the_notif_screen_snapshots() {
        let state = NotifState::default();
        for (width, height, name) in [(80, 24, "80x24"), (100, 30, "100x30"), (120, 35, "120x35")] {
            let text = rendered(width, height, &state).join("\n");
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
            settings.bind(|| insta::assert_snapshot!(format!("notif_{name}"), text));
        }
    }

    /// At 80×24 the ADHAN and NOTIF columns are gone (§16.5 item 9);
    /// at 100×30 all five columns render.
    #[test]
    fn small_drops_the_toggle_columns() {
        let state = NotifState::default();
        let small = rendered(80, 24, &state).join("\n");
        assert!(small.contains("BEFORE"), "small header:\n{small}");
        // The toggle *columns* are gone — the header row holds no
        // ADHAN/NOTIF labels (the crumb's NOTIFICATIONS always
        // carries the substring).
        assert!(
            !small
                .lines()
                .any(|line| line.contains("  ADHAN  ") || line.contains("  NOTIF")),
            "small drops the toggles:\n{small}"
        );

        let medium = rendered(100, 30, &state).join("\n");
        assert!(medium.contains("BEFORE ADHAN"), "medium header:\n{medium}");
        assert!(
            medium.lines().any(|line| line.contains("NOTIF")),
            "medium keeps the toggles:\n{medium}"
        );
    }

    /// All five prayers render in both the grid and the timeline.
    #[test]
    fn all_five_prayers_render() {
        let state = NotifState::default();
        let text = rendered(100, 30, &state).join("\n");
        for prayer in Notifiable::ALL {
            assert!(
                text.contains(prayer.display()),
                "missing {}:\n{text}",
                prayer.display()
            );
        }
        assert!(text.contains("◆"), "timeline markers:\n{text}");
        assert!(text.contains("◉ adhan"), "timeline adhan:\n{text}");
        assert!(text.contains("◉ mosque"), "timeline mosque:\n{text}");
    }

    /// `Tab` cycles the four columns in order (§16.5 item 7).
    #[test]
    fn tab_cycles_the_columns() {
        use NotifCol as N;
        assert_eq!(cycle_col(N::Before, true), N::After);
        assert_eq!(cycle_col(N::After, true), N::Adhan);
        assert_eq!(cycle_col(N::Adhan, true), N::Notif);
        assert_eq!(cycle_col(N::Notif, true), N::Before, "wraps");
        assert_eq!(cycle_col(N::Before, false), N::Notif, "wraps back");
    }
}
