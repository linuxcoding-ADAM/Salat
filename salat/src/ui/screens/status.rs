//! The Status screen (§16.7) — the `[4] STATUS` tab. Phase 6 ships
//! the title-only placeholder body; Phase 12 replaces it with the
//! component table and event log.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::theme::Theme;

use crate::ui::shell::StatusState;

/// Render the status body (Phase 12).
pub fn render(
    _model: &UiModel,
    _state: &StatusState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    _breakpoint: Breakpoint,
) {
    super::title_only("STATUS", area, buf, theme);
}
