//! `render(frame, &UiModel, &Shell)` — the single UI entry point (§15).
//!
//! The tree is the §19 layout: `Shell` (input state), `input`
//! (resolution), `screens` (one module per §3.3 screen), `widgets`
//! (§15.5 inventory), and the `theme`/`layout`/`model` foundations.
//!
//! `render` applies the frame-wide rules once — the ground colour, the
//! §16.13 too-small guard, and §3.3's chrome suppression (the overlays
//! and the loading screen own every cell) — then dispatches to the
//! focused screen.

pub mod input;
pub mod layout;
pub mod model;
pub mod screens;
pub mod shell;
pub mod theme;
pub mod widgets;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::theme::Theme;
use crate::ui::widgets::too_small;

use shell::{Screen, Shell};

/// Render one frame (§15.1): a pure function of the model and the
/// shell. Chrome (header + footer) wraps every screen except the
/// overlays and the loading screen; below 80×24 the guard is the
/// frame (§16.13).
pub fn render(frame: &mut Frame, model: &UiModel, shell: &Shell) {
    let area = frame.area();
    let theme = Theme::from(&model.config.display);
    let buf = frame.buffer_mut();

    // The app ground (the design's `termBg`) — every screen draws over
    // it, and the chrome rows are it.
    buf.set_style(area, Style::new().bg(theme.term_bg));

    // §16.13: nothing renders below 80×24 but the guard.
    let breakpoint = Breakpoint::of(area);
    if breakpoint == Breakpoint::TooSmall {
        too_small::render(area, buf);
        return;
    }

    // §3.3: the overlays suppress header and footer and own the frame.
    if let Some(overlay) = shell.overlay {
        screens::render_overlay(overlay, model, shell, area, buf, &theme);
        return;
    }

    // §16.11 item 4: the loading screen, too, owns every cell.
    if matches!(shell.screen(), Screen::Loading) {
        screens::loading::render(area, buf, &theme, &shell.boot, model.now, breakpoint);
        return;
    }

    // Chrome: header 2 rows, body, footer 2 rows (§3.4).
    let header = Rect::new(area.x, area.y, area.width, 2);
    let footer = Rect::new(area.x, area.bottom().saturating_sub(2), area.width, 2);
    let body = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(4),
    );
    widgets::chrome::render_header(model, header, buf, &theme, breakpoint);
    widgets::chrome::render_footer(shell.screen(), footer, buf, &theme, breakpoint);
    screens::render(shell.screen(), model, shell, body, buf, &theme, breakpoint);
}
