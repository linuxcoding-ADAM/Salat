//! Every design token + 256/ansi degradation (§15.6). The Figma design
//! is the source of truth; this module is its faithful translation.
//!
//! [`Theme`] holds all 27 named values of §3.2 — 24 single tokens plus
//! the `dot1`/`dot2`/`dot3` row. Five of those (`winBg`, `winTitle`,
//! `dot1`–`dot3`) are prototype chrome that no screen in §16 renders;
//! they are declared for completeness and marked as such.
//!
//! The degradation modes are *mappings*, not approximations: the
//! 256-colour table is the §15.6 hand-picked one (its two deliberate
//! collapses are asserted as collapses by the tests), and the ANSI
//! mapping keeps every §15.6 distinctness pair expressible. All
//! rendering metadata for prayer states — glyph, word, colour — lives
//! here, not in the domain (§10's rule).

use chrono::{DateTime, Timelike};
use chrono_tz::Tz;
use ratatui::style::Color;

use crate::config::{ColorMode, DisplayConfig, TimeFormat};
use crate::domain::PrayerState;

/// The blink rates §3.2 fixes, in milliseconds. All phases are derived
/// from the clock, never from process start, so a snapshot with a
/// pinned clock is deterministic.
pub mod blink {
    /// The text cursor in the Location search field.
    pub const CURSOR_MS: i64 = 560;
    /// The `◉ ADHAN` badge on the Adhan overlay.
    pub const ADHAN_BADGE_MS: i64 = 500;
    /// The mosque reminder banner.
    pub const MOSQUE_BANNER_MS: i64 = 800;
    /// The loading screen's in-flight step marker.
    pub const LOADING_MS: i64 = 300;
}

/// A blink phase derived from the clock (§3.2): on for one period, off
/// for the next, so every blinking element is a pure function of `now`
/// and a snapshot never depends on when the process started.
#[derive(Debug, Clone, Copy)]
pub struct Blink {
    /// One half-period, in milliseconds.
    pub period_ms: i64,
}

impl Blink {
    /// A blink with the given half-period.
    pub const fn new(period_ms: i64) -> Self {
        Self { period_ms }
    }

    /// Whether the element is visible at `now`.
    pub fn on(&self, now: DateTime<Tz>) -> bool {
        (now.timestamp_millis() / self.period_ms) % 2 == 0
    }
}

/// The rendering mode after `auto` detection — the config's
/// [`ColorMode`] resolved against the environment (§15.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// §3.2's RGB values verbatim.
    Truecolor,
    /// The §15.6 hand-picked xterm-256 table.
    Palette256,
    /// The 16-colour set.
    Ansi,
}

impl RenderMode {
    /// §15.6's `auto` rule: `$COLORTERM` ∈ {`truecolor`, `24bit`} →
    /// truecolor; `$TERM` contains `256color` → 256; else ANSI.
    fn detect() -> Self {
        if let Ok(colorterm) = std::env::var("COLORTERM")
            && matches!(colorterm.as_str(), "truecolor" | "24bit")
        {
            return Self::Truecolor;
        }
        if let Ok(term) = std::env::var("TERM")
            && term.contains("256color")
        {
            return Self::Palette256;
        }
        Self::Ansi
    }
}

/// The §3.2 palette with all 27 named values, already degraded to the
/// requested mode. Built through [`Theme::from`] — never assembled by
/// hand at a call site, so a mode change is one place.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// The mode this theme's colours were degraded for.
    pub mode: RenderMode,
    /// `#080d18` — the terminal ground.
    pub term_bg: Color,
    /// `#0b1222` — panel ground.
    pub panel_bg: Color,
    /// `#0f1f36` — the next prayer's row ground.
    pub row_active: Color,
    /// `#122440` — the selected row's ground.
    pub row_sel: Color,
    /// `#141428` — prototype window chrome; no screen renders it.
    pub win_bg: Color,
    /// `#1a1a32` — prototype window chrome; no screen renders it.
    pub win_title: Color,
    /// `#1a3355` — the standard panel border.
    pub border: Color,
    /// `#0d1a2a` — the dim border.
    pub border_dim: Color,
    /// `#3d7ab5` — the active panel's border.
    pub border_hi: Color,
    /// `#1e4a30` — the green border (prayer-config panel).
    pub border_grn: Color,
    /// `#c8d4ea` — body text.
    pub text: Color,
    /// `#6b7d94` — secondary text.
    pub text_sub: Color,
    /// `#2d3f52` — the dimmest text.
    pub text_dim: Color,
    /// `#eef2f8` — emphasized text.
    pub text_bright: Color,
    /// `#4ade80` — the "next"/"completed" state colour.
    pub green: Color,
    /// `#091a0f` — a green-tinted ground. Collapses to `panelBg` at 256
    /// (§15.6's deliberate collapse, asserted by the palette tests).
    pub green_bg: Color,
    /// `#1a3d24` — dim green.
    pub green_dim: Color,
    /// `#f87171` — the Adhan/alert colour.
    pub red: Color,
    /// `#190808` — the red wash; the one tint that keeps its hue at 256.
    pub red_bg: Color,
    /// `#3d1515` — dim red.
    pub red_dim: Color,
    /// `#100808` — the ERROR screen's trace inset (§3.2's ground
    /// outside the palette table). At 256/Ansi the mechanical nearest
    /// is the panel ground, so the trace reads as a plain inset — the
    /// `redBg` wash around it keeps the block legible either way.
    pub error_trace: Color,
    /// `#fbbf24` — the "soon"/warning colour.
    pub yellow: Color,
    /// `#1a1200` — a yellow-tinted ground. Collapses to `panelBg` at 256.
    pub yellow_bg: Color,
    /// `#60a5fa` — selection and links.
    pub blue: Color,
    /// `#34d399` — tertiary accent.
    pub cyan: Color,
    /// `#ff5f57` — prototype traffic light; no screen renders it.
    pub dot1: Color,
    /// `#ffbd2e` — prototype traffic light; no screen renders it.
    pub dot2: Color,
    /// `#28c841` — prototype traffic light; no screen renders it.
    pub dot3: Color,
}

const fn rgb(red: u8, green: u8, blue: u8) -> Color {
    Color::Rgb(red, green, blue)
}

const fn idx(index: u8) -> Color {
    Color::Indexed(index)
}

impl Theme {
    /// Resolve the theme from the display configuration (§15.6):
    /// the configured mode, with `auto` detected against the
    /// environment.
    pub fn from(display: &DisplayConfig) -> Self {
        Self::for_mode(match display.color_mode {
            ColorMode::Truecolor => RenderMode::Truecolor,
            ColorMode::TwoHundredFiftySix => RenderMode::Palette256,
            ColorMode::Ansi => RenderMode::Ansi,
            ColorMode::Auto => RenderMode::detect(),
        })
    }

    /// The theme for one resolved mode — the single place the three
    /// palettes are written down.
    pub fn for_mode(mode: RenderMode) -> Self {
        match mode {
            RenderMode::Truecolor => Self::truecolor(),
            RenderMode::Palette256 => Self::palette256(),
            RenderMode::Ansi => Self::ansi(),
        }
    }

    /// §3.2 verbatim.
    fn truecolor() -> Self {
        Self {
            mode: RenderMode::Truecolor,
            term_bg: rgb(0x08, 0x0d, 0x18),
            panel_bg: rgb(0x0b, 0x12, 0x22),
            row_active: rgb(0x0f, 0x1f, 0x36),
            row_sel: rgb(0x12, 0x24, 0x40),
            win_bg: rgb(0x14, 0x14, 0x28),
            win_title: rgb(0x1a, 0x1a, 0x32),
            border: rgb(0x1a, 0x33, 0x55),
            border_dim: rgb(0x0d, 0x1a, 0x2a),
            border_hi: rgb(0x3d, 0x7a, 0xb5),
            border_grn: rgb(0x1e, 0x4a, 0x30),
            text: rgb(0xc8, 0xd4, 0xea),
            text_sub: rgb(0x6b, 0x7d, 0x94),
            text_dim: rgb(0x2d, 0x3f, 0x52),
            text_bright: rgb(0xee, 0xf2, 0xf8),
            green: rgb(0x4a, 0xde, 0x80),
            green_bg: rgb(0x09, 0x1a, 0x0f),
            green_dim: rgb(0x1a, 0x3d, 0x24),
            red: rgb(0xf8, 0x71, 0x71),
            red_bg: rgb(0x19, 0x08, 0x08),
            red_dim: rgb(0x3d, 0x15, 0x15),
            error_trace: rgb(0x10, 0x08, 0x08),
            yellow: rgb(0xfb, 0xbf, 0x24),
            yellow_bg: rgb(0x1a, 0x12, 0x00),
            blue: rgb(0x60, 0xa5, 0xfa),
            cyan: rgb(0x34, 0xd3, 0x99),
            dot1: rgb(0xff, 0x5f, 0x57),
            dot2: rgb(0xff, 0xbd, 0x2e),
            dot3: rgb(0x28, 0xc8, 0x41),
        }
    }

    /// The §15.6 hand-picked xterm-256 table, transcribed exactly —
    /// including the two deliberate collapses of `greenBg` and
    /// `yellowBg` onto `panelBg` (233) and `redBg`'s kept hue (52).
    /// The five prototype chrome tokens are not in §15.6's table; they
    /// get the mechanical nearest of the same rules and are never
    /// rendered.
    fn palette256() -> Self {
        Self {
            mode: RenderMode::Palette256,
            // The ramp, in §15.6's luminance order.
            term_bg: idx(232),     // #080808 — ramp, nearest
            panel_bg: idx(233),    // #121212 — ramp, nearest
            border_dim: idx(234),  // #1c1c1c — ramp, nearest
            row_active: idx(235),  // #262626 — nearest was 234 → prev + 1
            row_sel: idx(236),     // #303030 — nearest was 235 → prev + 1
            border: idx(237),      // #3a3a3a — nearest was 236 → prev + 1
            text_dim: idx(238),    // #444444 — nearest was 237 → prev + 1
            text_sub: idx(244),    // #808080 — ramp, nearest
            text: idx(252),        // #d0d0d0 — ramp, nearest
            text_bright: idx(255), // #eeeeee — ramp, nearest
            // The cube.
            border_hi: idx(67),    // cube (1,2,3)
            blue: idx(75),         // cube (1,3,5)
            green: idx(78),        // cube (1,4,2)
            cyan: idx(43),         // cube (0,4,3) — moved off green's cell
            red: idx(203),         // cube (5,1,1)
            yellow: idx(214),      // cube (5,3,0)
            green_dim: idx(22),    // cube (0,1,0)
            border_grn: idx(28),   // cube (0,2,0)
            red_bg: idx(52),       // cube (1,0,0) — the one kept tint
            red_dim: idx(88),      // cube (2,0,0)
            error_trace: idx(233), // #100808 → ramp, nearest = panelBg
            // The declared collapses.
            green_bg: idx(233),  // → panelBg, deliberate (§15.6)
            yellow_bg: idx(233), // → panelBg, deliberate (§15.6)
            // Prototype chrome — mechanical nearest, never rendered.
            win_bg: idx(233),
            win_title: idx(234),
            dot1: idx(203),
            dot2: idx(214),
            dot3: idx(78),
        }
    }

    /// The 16-colour set (§15.6): grounds lose their tints (the
    /// tinted grounds collapse onto `panelBg`, exactly as at 256) and
    /// the glyph vocabulary carries the state meaning. `redBg` keeps a
    /// red (`Color::Red`, the dark ANSI red) so "the screen turns red"
    /// survives; every §15.6 distinctness pair stays expressible.
    fn ansi() -> Self {
        Self {
            mode: RenderMode::Ansi,
            term_bg: Color::Reset, // the terminal's own ground
            panel_bg: Color::Black,
            row_active: Color::Black,
            row_sel: Color::Black,
            win_bg: Color::Black,
            win_title: Color::DarkGray,
            border: Color::Blue,
            border_dim: Color::Black,
            border_hi: Color::LightBlue,
            border_grn: Color::DarkGray,
            text: Color::Gray,
            text_sub: Color::DarkGray,
            text_dim: Color::Black,
            text_bright: Color::White,
            green: Color::LightGreen,
            green_bg: Color::Black, // → panelBg, deliberate
            green_dim: Color::Green,
            red: Color::LightRed,
            red_bg: Color::Red, // the one kept tint
            red_dim: Color::DarkGray,
            error_trace: Color::Black, // → panelBg, mechanical nearest
            yellow: Color::LightYellow,
            yellow_bg: Color::Black, // → panelBg, deliberate
            blue: Color::LightBlue,
            cyan: Color::LightCyan,
            dot1: Color::LightRed,
            dot2: Color::LightYellow,
            dot3: Color::LightGreen,
        }
    }

    /// The §3.5 badge vocabulary: one glyph, one word and one colour
    /// per state — the non-colour-only encoding §22 requires, so the
    /// meaning survives every degraded mode.
    pub fn state_badge(&self, state: PrayerState) -> (&'static str, &'static str, Color) {
        let colour = match state {
            PrayerState::Passed => self.text_dim,
            PrayerState::Completed => self.green,
            PrayerState::Next => self.green,
            PrayerState::PreAdhan => self.yellow,
            PrayerState::Adhan => self.red,
            PrayerState::PostAdhan => self.red,
            PrayerState::Upcoming => self.text_sub,
        };
        let (glyph, word) = match state {
            PrayerState::Passed => ("✓", "passed"),
            PrayerState::Completed => ("✓", "completed"),
            PrayerState::Next => ("●", "next"),
            PrayerState::PreAdhan => ("◈", "soon"),
            PrayerState::Adhan => ("◉", "adhan"),
            PrayerState::PostAdhan => ("◉", "reminder"),
            PrayerState::Upcoming => ("○", "upcoming"),
        };
        (glyph, word, colour)
    }
}

/// The §14.6 table's name column: the design's `FAJR` label in title
/// case (`Fajr`), as the quick-mode specimen prints it.
pub fn quick_name(prayer: crate::domain::Notifiable) -> &'static str {
    match prayer {
        crate::domain::Notifiable::Fajr => "Fajr",
        crate::domain::Notifiable::Dhuhr => "Dhuhr",
        crate::domain::Notifiable::Asr => "Asr",
        crate::domain::Notifiable::Maghrib => "Maghrib",
        crate::domain::Notifiable::Isha => "Isha",
    }
}

/// `HH:MM` in the configured format — the table's TIME column and the
/// panel's `· 19:21` (§3.5). Rendering metadata lives here per §10's
/// rule, not in the domain.
pub fn hhmm(time: DateTime<Tz>, format: &TimeFormat) -> String {
    let (hour, minute) = (time.hour(), time.minute());
    match format {
        TimeFormat::TwentyFourHour => format!("{hour:02}:{minute:02}"),
        TimeFormat::TwelveHour => {
            let (hour12, meridiem) = match hour % 12 {
                0 => (12, if hour < 12 { "AM" } else { "PM" }),
                hour12 => (hour12, if hour < 12 { "AM" } else { "PM" }),
            };
            format!("{hour12:02}:{minute:02} {meridiem}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    /// §15.6's eight distinctness pairs, in every mode — the property
    /// the hand-picked table exists to preserve. ANSI keeps them too:
    /// that is why `borderGrn` is DarkGray and not Green there.
    #[test]
    fn the_eight_pairs_stay_distinct_in_every_mode() {
        for mode in [
            RenderMode::Truecolor,
            RenderMode::Palette256,
            RenderMode::Ansi,
        ] {
            let theme = Theme::for_mode(mode);
            let pairs = [
                ("border", theme.border, "borderDim", theme.border_dim),
                ("border", theme.border, "borderHi", theme.border_hi),
                ("text", theme.text, "textSub", theme.text_sub),
                ("textSub", theme.text_sub, "textDim", theme.text_dim),
                ("green", theme.green, "cyan", theme.cyan),
                ("redBg", theme.red_bg, "redDim", theme.red_dim),
                ("greenDim", theme.green_dim, "borderGrn", theme.border_grn),
                ("termBg", theme.term_bg, "panelBg", theme.panel_bg),
            ];
            for (a, ca, b, cb) in pairs {
                assert_ne!(ca, cb, "{mode:?}: {a} and {b} collapsed");
            }
        }
    }

    /// §15.6: the ten neutral ramp indices are strictly increasing in
    /// the table's luminance order. The assertion is about the
    /// hand-picked 256 indices; ANSI has no ramp to preserve — its
    /// neutrals collapse onto Black by design (the mapping, and the
    /// §22 non-colour encoding, carry the meaning) — so the pairs test
    /// is what covers that mode.
    #[test]
    fn the_ten_neutrals_form_a_strictly_increasing_ramp() {
        let theme = Theme::for_mode(RenderMode::Palette256);
        let ramp = [
            (theme.term_bg, 232u8),
            (theme.panel_bg, 233),
            (theme.border_dim, 234),
            (theme.row_active, 235),
            (theme.row_sel, 236),
            (theme.border, 237),
            (theme.text_dim, 238),
            (theme.text_sub, 244),
            (theme.text, 252),
            (theme.text_bright, 255),
        ];
        // The tokens are the §15.6 table, in the table's order, with
        // strictly increasing indices — an unfaithful transcription
        // breaks either half.
        let indices: Vec<u8> = ramp
            .iter()
            .map(|(colour, index)| {
                assert_eq!(*colour, Color::Indexed(*index));
                *index
            })
            .collect();
        for pair in indices.windows(2) {
            assert!(
                pair[0] < pair[1],
                "ramp not strictly increasing: {indices:?}"
            );
        }
    }

    /// §15.6's two deliberate collapses, asserted *as* collapses in
    /// both degraded modes — a later "fix" that separates them fails
    /// here and has to be argued for.
    #[test]
    fn the_two_tinted_grounds_collapse_onto_the_panel_ground() {
        for mode in [RenderMode::Palette256, RenderMode::Ansi] {
            let theme = Theme::for_mode(mode);
            assert_eq!(theme.green_bg, theme.panel_bg, "{mode:?}: greenBg");
            assert_eq!(theme.yellow_bg, theme.panel_bg, "{mode:?}: yellowBg");
        }
    }

    /// §15.6: `redBg` is the one tint that keeps its hue at 256 — the
    /// Adhan wash must still read as red.
    #[test]
    fn red_bg_keeps_its_hue_at_256() {
        let theme = Theme::for_mode(RenderMode::Palette256);
        assert_eq!(theme.red_bg, Color::Indexed(52));
        assert_ne!(theme.red_bg, theme.panel_bg);
    }

    /// The 256 table is §15.6's, token for token.
    #[test]
    fn the_256_table_matches_the_specification() {
        let theme = Theme::for_mode(RenderMode::Palette256);
        let expected: [(&str, Color); 23] = [
            ("termBg", idx(232)),
            ("panelBg", idx(233)),
            ("borderDim", idx(234)),
            ("rowActive", idx(235)),
            ("rowSel", idx(236)),
            ("border", idx(237)),
            ("textDim", idx(238)),
            ("textSub", idx(244)),
            ("text", idx(252)),
            ("textBright", idx(255)),
            ("borderHi", idx(67)),
            ("blue", idx(75)),
            ("green", idx(78)),
            ("cyan", idx(43)),
            ("red", idx(203)),
            ("yellow", idx(214)),
            ("greenDim", idx(22)),
            ("borderGrn", idx(28)),
            ("redBg", idx(52)),
            ("redDim", idx(88)),
            ("errorTrace", idx(233)),
            ("greenBg", idx(233)),
            ("yellowBg", idx(233)),
        ];
        let actual: [(&str, Color); 23] = [
            ("termBg", theme.term_bg),
            ("panelBg", theme.panel_bg),
            ("borderDim", theme.border_dim),
            ("rowActive", theme.row_active),
            ("rowSel", theme.row_sel),
            ("border", theme.border),
            ("textDim", theme.text_dim),
            ("textSub", theme.text_sub),
            ("text", theme.text),
            ("textBright", theme.text_bright),
            ("borderHi", theme.border_hi),
            ("blue", theme.blue),
            ("green", theme.green),
            ("cyan", theme.cyan),
            ("red", theme.red),
            ("yellow", theme.yellow),
            ("greenDim", theme.green_dim),
            ("borderGrn", theme.border_grn),
            ("redBg", theme.red_bg),
            ("redDim", theme.red_dim),
            ("errorTrace", theme.error_trace),
            ("greenBg", theme.green_bg),
            ("yellowBg", theme.yellow_bg),
        ];
        assert_eq!(expected, actual);
    }

    /// §3.5's badge vocabulary: the glyph/word half is
    /// mode-independent; only the colour degrades.
    #[test]
    fn the_state_badge_vocabulary_is_the_designs() {
        let theme = Theme::for_mode(RenderMode::Truecolor);
        let cases = [
            (PrayerState::Passed, ("✓", "passed")),
            (PrayerState::Completed, ("✓", "completed")),
            (PrayerState::Next, ("●", "next")),
            (PrayerState::PreAdhan, ("◈", "soon")),
            (PrayerState::Adhan, ("◉", "adhan")),
            (PrayerState::PostAdhan, ("◉", "reminder")),
            (PrayerState::Upcoming, ("○", "upcoming")),
        ];
        for (state, expected) in cases {
            let (glyph, word, _) = theme.state_badge(state);
            assert_eq!((glyph, word), expected);
        }
    }

    /// §3.2's blink phase comes from the clock, not process start:
    /// half-period boundaries flip the phase deterministically.
    #[test]
    fn blink_phase_is_clock_derived() {
        let blink = Blink::new(500);
        let zone: Tz = chrono_tz::Africa::Algiers;
        let base = zone
            .with_ymd_and_hms(2026, 9, 5, 17, 38, 0)
            .single()
            .unwrap();
        assert!(blink.on(base));
        assert!(!blink.on(base + chrono::Duration::milliseconds(500)));
        assert!(blink.on(base + chrono::Duration::milliseconds(1000)));
    }

    /// `hhmm` honours the configured format.
    #[test]
    fn hhmm_formats_both_clocks() {
        let zone: Tz = chrono_tz::Africa::Algiers;
        let time = zone
            .with_ymd_and_hms(2026, 9, 5, 19, 21, 0)
            .single()
            .unwrap();
        assert_eq!(hhmm(time, &TimeFormat::TwentyFourHour), "19:21");
        assert_eq!(hhmm(time, &TimeFormat::TwelveHour), "07:21 PM");
        let midnight = zone.with_ymd_and_hms(2026, 9, 5, 0, 5, 0).single().unwrap();
        assert_eq!(hhmm(midnight, &TimeFormat::TwelveHour), "12:05 AM");
    }
}
