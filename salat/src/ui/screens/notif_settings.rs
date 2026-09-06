//! The Notification Settings screen (§16.5) — pushed from Settings.
//! Phase 6 ships the title-only placeholder body; Phase 11 replaces it
//! with the 5×4 matrix.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::theme::Theme;

use crate::ui::shell::NotifState;

/// Render the notification-matrix body (Phase 11).
pub fn render(
    _model: &UiModel,
    _state: &NotifState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    _breakpoint: Breakpoint,
) {
    super::title_only("NOTIFICATION SETTINGS", area, buf, theme);
}
