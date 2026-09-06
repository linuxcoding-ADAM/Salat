//! The Settings screen (§16.4). Phase 6 ships the title-only
//! placeholder body; Phase 11 replaces it with the settings list.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::theme::Theme;

use crate::ui::shell::SettingsState;

/// Render the settings body (Phase 11).
pub fn render(
    _model: &UiModel,
    _state: &SettingsState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    _breakpoint: Breakpoint,
) {
    super::title_only("SETTINGS", area, buf, theme);
}
