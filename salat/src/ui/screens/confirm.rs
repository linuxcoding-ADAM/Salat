//! The confirmation dialog (§16.8). Phase 6 ships the title-only
//! placeholder body; Phase 11 gives it the message it is pushed with.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::ui::layout::Breakpoint;
use crate::ui::theme::Theme;

/// Render the confirmation body (Phase 11).
pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme, _breakpoint: Breakpoint) {
    super::title_only("SAVED", area, buf, theme);
}
