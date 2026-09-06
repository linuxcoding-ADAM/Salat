//! `Shell`, the navigation stack, the overlay slot and every screen's
//! interactive state (§15.1) — the one mutable thing in the UI, rebuilt
//! nowhere, threaded to input handling and rendering.
//!
//! Stack semantics: `stack[0]` is always a main screen; the four tab
//! keys and their digit shortcuts *replace* the root (tab switching
//! never accumulates main screens), `push` stacks sub-screens and
//! reference screens on top, and `pop` — §23 Phase 6's A6 — never
//! empties the stack. The overlay is not on the stack: it floats above
//! whatever screen is focused and swallows input (§15.4).

use crossterm::event::{Event, KeyEventKind};

use crate::app::boot::BootState;
use crate::app::runtime::Runtime;
use crate::error::SalatError;

use super::input::{self, Outcome};

/// The twelve screens (§3.3). The main four are stack roots; the rest
/// are pushed. `Error` carries the typed error it renders (§16.10);
/// `Confirm`'s message arrives with Phase 11's flows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// `[1] TODAY` — the design's dashboard.
    Dashboard,
    /// `[2] SETTINGS`.
    Settings,
    /// `[3] PRAYERS` — the tab reads "pray", the screen configures
    /// them: prayer-config (§3.3's note).
    PrayerConfig,
    /// `[4] STATUS`.
    Status,
    /// Notifications, pushed from Settings.
    NotifSettings,
    /// Location, pushed from Settings.
    Location,
    /// A yes/no confirmation, pushed by saving flows.
    Confirm,
    /// A typed failure (§16.10) — boot errors and later flows.
    Error(SalatError),
    /// The boot screen (§16.11); never on the post-boot stack, but part
    /// of the enum so `ui::render` dispatches it uniformly.
    Loading,
    /// `?` — the prayer-state reference (§16.12).
    States,
}

impl Screen {
    /// The four tab roots (§3.3) — the footer's main-screen chrome and
    /// the `go_main` switch apply to these.
    pub fn is_main(&self) -> bool {
        matches!(
            self,
            Screen::Dashboard | Screen::Settings | Screen::PrayerConfig | Screen::Status
        )
    }

    /// The design label for a screen's stub body and breadcrumbs.
    pub fn title(&self) -> &'static str {
        match self {
            Screen::Dashboard => "TODAY",
            Screen::Settings => "SETTINGS",
            Screen::PrayerConfig => "PRAYER CONFIG",
            Screen::Status => "STATUS",
            Screen::NotifSettings => "NOTIFICATION SETTINGS",
            Screen::Location => "LOCATION",
            Screen::Confirm => "SAVED",
            Screen::Error(_) => "ERROR",
            Screen::Loading => "INITIALIZING",
            Screen::States => "PRAYER STATE REFERENCE",
        }
    }
}

/// The two full-screen overlays (§3.3) — they suppress the header and
/// footer and swallow all input but their own three keys (§15.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// The full-screen Adhan call.
    AdhanAlert,
    /// The post-Adhan reminder banner.
    MosqueReminder,
}

/// The dashboard's interactive state: which prayer row the cursor is
/// on (the `c` complete action's target, Phase 7).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DashboardState {
    pub cursor: usize,
}

/// The settings list's selected row.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SettingsState {
    pub row: usize,
}

/// The prayer-config editor: which of the five prayer columns is being
/// edited (`tab` 0–4) and which setting row within it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrayerConfigState {
    pub tab: usize,
    pub row: usize,
}

impl Default for PrayerConfigState {
    /// Opens on Maghrib (§16.3) — index 3 in `Notifiable::ALL` order.
    fn default() -> Self {
        Self { tab: 3, row: 0 }
    }
}

/// The notification matrix's focused column (§16.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotifCol {
    #[default]
    Before,
    After,
    Adhan,
    Notif,
}

/// The notification matrix's cursor.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NotifState {
    pub row: usize,
    pub col: NotifCol,
}

/// The location editor's mode (§16.5). The search-results list arrives
/// with Phase 11's `CityMatch` — this Phase 6 shape carries everything
/// else.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LocationMode {
    /// The city search box.
    #[default]
    Search,
    /// Manual coordinate entry.
    Coords,
    /// The automatic default.
    Auto,
}

/// Which coordinate buffer keyboard entry targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoordFocus {
    #[default]
    Lat,
    Lng,
}

/// The location editor's full interactive state (§15.1, minus the
/// Phase 11 results list).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LocationState {
    pub mode: LocationMode,
    pub query: String,
    pub sel: usize,
    pub lat_buf: String,
    pub lng_buf: String,
    pub focus: CoordFocus,
}

/// The status log's scroll position.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StatusState {
    pub scroll: u16,
}

/// The whole interactive UI (§15.1, field-for-field).
pub struct Shell {
    /// Always holds at least one element; `[0]` is always a main
    /// screen. The focused screen is the last.
    pub stack: Vec<Screen>,
    /// The floating overlay, if any — not on the stack.
    pub overlay: Option<Overlay>,
    pub dashboard: DashboardState,
    pub settings: SettingsState,
    pub prayer_cfg: PrayerConfigState,
    pub notif: NotifState,
    pub location: LocationState,
    pub status: StatusState,
    /// The boot that produced the running `Runtime` — `done()` once
    /// the main loop starts.
    pub boot: BootState,
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

impl Shell {
    /// A shell whose stack roots on the dashboard, every cursor at its
    /// documented default, and boot marked complete.
    pub fn new() -> Self {
        Self {
            stack: vec![Screen::Dashboard],
            overlay: None,
            dashboard: DashboardState::default(),
            settings: SettingsState::default(),
            prayer_cfg: PrayerConfigState::default(),
            notif: NotifState::default(),
            location: LocationState::default(),
            status: StatusState::default(),
            boot: BootState::done(),
        }
    }

    /// The focused screen — the stack's last element.
    pub fn screen(&self) -> &Screen {
        // `stack` is never empty: `pop` guards the root.
        self.stack.last().unwrap_or(&Screen::Dashboard)
    }

    /// Push a sub-screen or reference screen (never a main one — tab
    /// switches use [`Shell::go_main`]).
    pub fn push(&mut self, screen: Screen) {
        self.stack.push(screen);
    }

    /// Pop the focused screen — A6: the stack never empties, so Esc on
    /// a bare main screen is a no-op, not an exit (`q` and Ctrl-C are
    /// the exits).
    pub fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    /// Replace the focused screen.
    pub fn replace(&mut self, screen: Screen) {
        if let Some(top) = self.stack.last_mut() {
            *top = screen;
        }
    }

    /// Switch the main tab: truncate to the root and replace it, so
    /// `[D, S, L]` + `3` lands on `[PrayerConfig]`, not
    /// `[D, S, L, PrayerConfig]`.
    pub fn go_main(&mut self, screen: Screen) {
        self.stack.truncate(1);
        self.replace(screen);
    }

    /// Show or clear the floating overlay.
    pub fn set_overlay(&mut self, overlay: Option<Overlay>) {
        self.overlay = overlay;
    }

    /// Route one event through §15.4's resolution order and apply it.
    /// Async because resolution can write: the dashboard's `c` completes
    /// a prayer through the store (A3) before returning.
    pub async fn handle(&mut self, event: Event, runtime: &Runtime) -> Result<Outcome, SalatError> {
        let Event::Key(key) = event else {
            return Ok(Outcome::Ignored);
        };
        // §15.4: only `Press` events are input; `Release` (a Kitty-
        // keyboard protocol feature) is not a keypress.
        if key.kind != KeyEventKind::Press {
            return Ok(Outcome::Ignored);
        }
        Ok(input::resolve(key, self, runtime).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §23 Phase 6's A6: Esc pops but never empties — the shell always
    /// has a screen to render.
    #[test]
    fn pop_never_empties_the_stack() {
        let mut shell = Shell::new();
        shell.push(Screen::States);
        shell.pop();
        assert_eq!(shell.stack.len(), 1);
        shell.pop();
        shell.pop();
        assert_eq!(shell.stack.len(), 1, "Esc on a root is a no-op");
        assert_eq!(*shell.screen(), Screen::Dashboard);
    }

    /// Tab switches replace the root, they do not stack (§3.3's four
    /// main screens).
    #[test]
    fn go_main_replaces_the_root() {
        let mut shell = Shell::new();
        shell.push(Screen::Location);
        shell.go_main(Screen::PrayerConfig);
        assert_eq!(shell.stack, vec![Screen::PrayerConfig]);
    }

    /// The prayer-config editor opens on Maghrib (§16.3).
    #[test]
    fn prayer_config_defaults_to_maghrib() {
        let shell = Shell::new();
        assert_eq!(shell.prayer_cfg.tab, 3);
    }
}
