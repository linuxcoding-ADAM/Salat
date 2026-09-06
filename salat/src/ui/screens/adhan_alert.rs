//! The Adhan alert overlay (§16.9) — the full-screen call to prayer.
//! Phase 6 ships the title-only placeholder body (no header, no
//! footer, §3.3); Phase 10 replaces it with the real layout and audio.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::ui::theme::Theme;

/// The design title the Phase 6 placeholder shows.
pub const TITLE: &str = "ADHAN";

/// Render the overlay body (Phase 10).
pub fn render(area: Rect, buf: &mut Buffer, theme: &Theme) {
    super::title_only(TITLE, area, buf, theme);
}
