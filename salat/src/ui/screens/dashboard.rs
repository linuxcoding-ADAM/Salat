//! The Dashboard screen — the design's home (§16.2). Phase 6 ships
//! the title-only placeholder body; Phase 7 replaces it with the real
//! layout (countdown, gauge, table, strip).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::theme::Theme;

use crate::ui::shell::DashboardState;

/// Render the dashboard body (Phase 7).
pub fn render(
    _model: &UiModel,
    _state: &DashboardState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    _breakpoint: Breakpoint,
) {
    super::title_only("TODAY", area, buf, theme);
}
