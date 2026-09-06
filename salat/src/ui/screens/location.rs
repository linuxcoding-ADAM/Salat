//! The Location screen (§16.3) — pushed from Settings. Phase 6 ships
//! the title-only placeholder body; Phase 11 replaces it with the city
//! search and coordinate entry.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::theme::Theme;

use crate::ui::shell::LocationState;

/// Render the location body (Phase 11).
pub fn render(
    _model: &UiModel,
    _state: &LocationState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    _breakpoint: Breakpoint,
) {
    super::title_only("LOCATION", area, buf, theme);
}
