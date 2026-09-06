//! The Prayer Configuration screen (§16.5) — the `[3] PRAYERS` tab.
//! Phase 6 ships the title-only placeholder body; Phase 11 replaces it
//! with the five-column editor.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::ui::layout::Breakpoint;
use crate::ui::model::UiModel;
use crate::ui::theme::Theme;

use crate::ui::shell::PrayerConfigState;

/// Render the prayer-config body (Phase 11).
pub fn render(
    _model: &UiModel,
    _state: &PrayerConfigState,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    _breakpoint: Breakpoint,
) {
    super::title_only("PRAYER CONFIG", area, buf, theme);
}
