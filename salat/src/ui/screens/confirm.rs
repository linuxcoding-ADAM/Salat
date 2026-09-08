//! The confirmation dialog (§16.9): settings were written to disk, and
//! here. A centered green panel that does not grow with the terminal —
//! a confirmation does not need more room.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget as _;

use crate::ui::theme::Theme;
use crate::ui::widgets::kbd_hint;
use crate::ui::widgets::panel::Panel;

/// Render the confirmation body: the ✓, `SETTINGS SAVED`, the real
/// config path with `$HOME` collapsed (the design's
/// `~/.config/salat/config.toml`), and the honest third line —
/// `Prayer times recalculated.` only when the save changed the API
/// cache key, `Settings applied.` otherwise (§16.9 item 6's fix).
pub fn render(path: &str, recalculated: bool, area: Rect, buf: &mut Buffer, theme: &Theme) {
    // §16.9 items 4/9/10: ~44 columns at 100×30 and up — fixed, a
    // confirmation does not grow — and `min(48, w − 4)` at 80×24, where
    // the 33-character path line is the constraint. `Length(9)` tall.
    let width = if area.width < 100 {
        area.width.saturating_sub(4).min(48)
    } else {
        44
    };
    let height = 9u16.min(area.height);
    if width < 10 || height < 9 {
        return; // an impossible terminal; the chrome still renders
    }
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let panel = Rect::new(x, y, width, height);

    let inner = Panel::new()
        .title("SAVED")
        .border(theme.green)
        .title_color(theme.green)
        .background(theme.green_bg)
        .render(panel, buf, theme);

    let mut line = |text: &str, color, row: u16| {
        let width = text.chars().count() as u16;
        let x = inner.x + (inner.width.saturating_sub(width)) / 2;
        Line::from(Span::styled(text, Style::new().fg(color)))
            .render(Rect::new(x, inner.y + row, inner.width, 1), buf);
    };
    // Row offsets are relative to `inner` — `line` adds `inner.y`.
    let center = inner.height / 2;
    // The design's vertical rhythm — ✓, title, path, verdict — centred
    // as a block, the button one row below the panel's middle.
    line("✓", theme.green, center.saturating_sub(2));
    line("SETTINGS SAVED", theme.green, center.saturating_sub(1));
    line(&home_shortened(path), theme.text_dim, center);
    let verdict = if recalculated {
        "Prayer times recalculated."
    } else {
        "Settings applied."
    };
    line(verdict, theme.green_dim, center + 1);
    let button = kbd_hint::line("ENTER", "CONTINUE", theme);
    let total = button.width() as u16;
    if total <= inner.width {
        let x = inner.x + (inner.width - total) / 2;
        button.render(
            Rect::new(x, inner.y + inner.height.saturating_sub(1), total, 1),
            buf,
        );
    }
}

/// The design's path rendering: `$HOME` collapsed to `~`, exactly as
/// `~/.config/salat/config.toml` shows. A path that does not start
/// with `$HOME` renders as-is.
pub(crate) fn home_shortened(path: &str) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home = home.to_string_lossy();
        if path == home {
            return "~".to_owned();
        }
        if let Some(rest) = path
            .strip_prefix(&format!("{home}/"))
            .or_else(|| path.strip_prefix(home.as_ref()))
        {
            return format!("~/{rest}");
        }
    }
    path.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The buffer as one string per row — the same glyph-by-glyph walk
    /// the other screen tests use (empty cells are spaces, so rows
    /// carry trailing spaces out to the buffer's edge).
    fn rendered(width: u16, height: u16, path: &str, recalculated: bool) -> Vec<String> {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        render(
            path,
            recalculated,
            buf.area,
            &mut buf,
            &Theme::for_mode(crate::ui::theme::RenderMode::Truecolor),
        );
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_owned())
                    .collect()
            })
            .collect()
    }

    /// The §16.9 body at the three full-screen sizes: the green SAVED
    /// panel, the ✓, `SETTINGS SAVED`, the path, the verdict, and the
    /// `[ENTER] CONTINUE` button. Fixed 44 columns from 100×30 — a
    /// confirmation does not grow with the terminal.
    #[test]
    fn the_confirm_screen_snapshots() {
        let path = "/home/adammila/.config/salat/config.toml";
        for (width, height) in [(80u16, 24u16), (100, 30), (120, 35)] {
            let text = rendered(width, height, path, true).join("\n");
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
            settings.bind(|| insta::assert_snapshot!(format!("confirm_{width}x{height}"), text));
        }
    }

    /// §16.9 items 6 and 10: the verdict is honest — `Prayer times
    /// recalculated.` only when the cache key changed, `Settings
    /// applied.` otherwise — and the panel stays 44 columns (borders
    /// included) at every size that can hold it.
    #[test]
    fn the_verdict_is_honest_and_the_panel_is_fixed() {
        let path = "/home/adammila/.config/salat/config.toml";
        for width in [80u16, 100, 120] {
            let rows = rendered(width, 30, path, false);
            let text = rows.join("\n");
            assert!(text.contains("Settings applied."), "{width} wide: {text}");
            assert!(!text.contains("recalculated"), "{width} wide: {text}");
            // The top border: `╭` at the panel's left edge, `╮` at its
            // right — 44 columns from 100×30 up (§16.9 item 10), the
            // item 9 clamp `min(48, w − 4)` below that.
            let expected = if width < 100 {
                (width - 4).min(48) as usize
            } else {
                44
            };
            let top = rows
                .iter()
                .find(|row| row.contains('╭'))
                .unwrap_or_else(|| panic!("{width} wide: no panel border in {text}"));
            // `chars().position`, not `str::find` — the box glyphs are
            // three bytes each and this needs glyph columns.
            let left = top.chars().position(|c| c == '╭').unwrap();
            let right = top.chars().position(|c| c == '╮').unwrap();
            assert_eq!(right - left + 1, expected, "{width} wide: {text}");
            // Centered in the row.
            assert_eq!(
                left,
                ((width as usize) - expected) / 2,
                "{width} wide: {text}"
            );
        }

        let text = rendered(100, 30, path, true).join("\n");
        assert!(text.contains("Prayer times recalculated."), "in: {text}");
    }

    /// `$HOME` collapses to `~` (the design's `~/.config/salat/…`), a
    /// foreign path renders as-is, and the bare home directory is `~`.
    #[test]
    fn home_is_shortened_in_the_path_line() {
        let home = std::env::var_os("HOME")
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/home/tester".to_owned());
        assert_eq!(
            home_shortened(&format!("{home}/.config/salat/config.toml")),
            "~/.config/salat/config.toml"
        );
        assert_eq!(home_shortened("/etc/salat.toml"), "/etc/salat.toml");
        assert_eq!(home_shortened(&home), "~");
    }
}
