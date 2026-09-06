//! `Breakpoint`, region splitting, centering helpers (§4.5).
//!
//! `Breakpoint` is computed once per frame from the frame area and
//! threaded to every screen; layout is recomputed every frame, so
//! resize handling is free — nothing here caches geometry. Quick mode
//! (§16.14) is the one deliberate exception: it pins `Small` and never
//! calls [`Breakpoint::of`], because its inline viewport is at most 14
//! rows and would classify `TooSmall` — a fact the ladder tests assert
//! by construction.
//!
//! Region splitting and centering helpers land with the screens that
//! need them (§16); Phase 5's quick view splits with one fixed
//! constraint list inline in its renderer.

use ratatui::layout::Rect;

/// The §4.5 classifier. Order matters: the tests below are the table,
/// read straight out of the section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakpoint {
    /// `w >= 120 && h >= 35`.
    Large,
    /// `w >= 100 && h >= 30`.
    Medium,
    /// `w >= 80 && h >= 24`.
    Small,
    /// The §16.13 guard: no viewport, the one-line summary.
    TooSmall,
}

impl Breakpoint {
    /// Classify a frame area — §4.5's table, computed once per frame
    /// by the caller and threaded to every screen.
    pub fn of(area: Rect) -> Self {
        Self::of_size(area.width, area.height)
    }

    /// The same table for bare dimensions (the quick-mode floor test
    /// uses terminal size, not a `Rect`, §16.14 item 9).
    pub fn of_size(width: u16, height: u16) -> Self {
        if width >= 120 && height >= 35 {
            Self::Large
        } else if width >= 100 && height >= 30 {
            Self::Medium
        } else if width >= 80 && height >= 24 {
            Self::Small
        } else {
            Self::TooSmall
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §4.5, verbatim: the four thresholds and the boundary itself
    /// (an exact 80×24 is `small`, not `too_small`).
    #[test]
    fn the_four_thresholds() {
        let cases = [
            // (width, height, expected)
            (120, 35, Breakpoint::Large),
            (160, 50, Breakpoint::Large),
            (119, 35, Breakpoint::Medium),
            (120, 34, Breakpoint::Medium),
            (100, 30, Breakpoint::Medium),
            (110, 40, Breakpoint::Medium),
            (99, 30, Breakpoint::Small),
            (100, 29, Breakpoint::Small),
            (80, 24, Breakpoint::Small),
            (90, 30, Breakpoint::Small),
            (79, 24, Breakpoint::TooSmall),
            (80, 23, Breakpoint::TooSmall),
            (0, 0, Breakpoint::TooSmall),
        ];
        for (width, height, expected) in cases {
            assert_eq!(
                Breakpoint::of_size(width, height),
                expected,
                "{width}×{height}"
            );
        }
    }

    /// `of` and `of_size` agree — the `Rect` overload is the same
    /// table.
    #[test]
    fn of_agrees_with_of_size() {
        let area = Rect::new(0, 0, 100, 30);
        assert_eq!(Breakpoint::of(area), Breakpoint::of_size(100, 30));
    }

    /// §16.14's note: a 14-row frame would classify `TooSmall`, which
    /// is exactly why quick mode pins `Small` instead of classifying.
    #[test]
    fn a_quick_viewport_would_be_too_small() {
        assert_eq!(Breakpoint::of_size(80, 14), Breakpoint::TooSmall);
    }
}
