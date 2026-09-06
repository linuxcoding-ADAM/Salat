//! The pre-prayer reminder overlay (§16.9) — the mosque banner. Phase
//! 6 ships the title-only placeholder body (no header, no footer,
//! §3.3); Phase 9 replaces it with the real layout.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::ui::theme::Theme;

/// The design title the Phase 6 placeholder shows.
pub const TITLE: &str = "MOSQUE REMINDER";

/// Render the overlay body (Phase 9).
pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme) {
    super::title_only(TITLE, area, buf, theme);
}
