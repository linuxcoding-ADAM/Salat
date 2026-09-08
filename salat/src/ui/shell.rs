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

use chrono::{DateTime, Duration};
use chrono_tz::Tz;
use crossterm::event::{Event, KeyEventKind};

use crate::app::boot::BootState;
use crate::app::runtime::Runtime;
use crate::domain::{Notifiable, PrayerState};
use crate::error::SalatError;
use crate::scheduler::ServiceEvent;
use crate::ui::model::UiModel;

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
///
/// The payloads are §16.7 item 2's `{prayer, at}` and §16.8 item 2's
/// `{prayer, adhan_at}`: the overlay is raised with the firing
/// event's data and stays self-contained afterwards — rendering needs
/// nothing but the model, and `Enter`/`s`/`Esc` are handled from the
/// payload alone. `blink` is a rate, not a field (§3.2's half-period
/// constants): the badge phase is derived from the clock at render
/// time, so a snapshot with a pinned clock is deterministic. The
/// reminder's payload stores its own fire instant (`t + after`) and
/// the screen recovers the Adhan instant from the live config's
/// offset — no guessed `adhan_at` in the shell (§16.8 item 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    /// The full-screen Adhan call.
    AdhanAlert {
        /// The prayer whose time has begun.
        prayer: Notifiable,
        /// The prayer's instant (the Adhan's own fire time).
        at: DateTime<Tz>,
        /// The §11.5 catch-up flag — the `(delayed)` wording.
        late: bool,
    },
    /// The post-Adhan reminder banner.
    MosqueReminder {
        /// The prayer whose Adhan this reminder follows.
        prayer: Notifiable,
        /// The reminder's own fire instant (`t + after`) — the screen
        /// recovers the Adhan instant from the live config's offset.
        reminder_at: DateTime<Tz>,
        /// §16.8 item 8's catch-up flag.
        late: bool,
    },
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

    /// One scheduler publication (§15.2's `svc_rx` arm). A fired
    /// `Adhan` raises the §16.7 overlay and a fired `MosqueReminder`
    /// the §16.8 one (§16.7 item 3's entry rule: scheduler events and
    /// observer transitions, never keypresses). Everything else is the
    /// Phase 8 behaviour — a repaint — because an event firing changes
    /// no screen's own state, but the frame is rebuilt the moment it
    /// lands. Returns whether the frame needs a rebuild.
    ///
    /// The §11.5 overlay rule holds structurally: the runner never
    /// broadcasts the fires its startup catch-up discharges (they are
    /// a different call site, and it knows the difference), so the
    /// only `Fired` events a live UI ever sees are live fires — `late`
    /// marks the within-grace delayed ones, whose overlays show the
    /// `(delayed)` wording.
    pub fn on_service_event(&mut self, event: ServiceEvent) -> bool {
        tracing::debug!(target: "salat::ui", ?event, "scheduler event");
        if let Some(overlay) = overlay_from(&event) {
            self.set_overlay(Some(overlay));
        }
        true
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

/// The overlay the state machine's own transitions detect (§4.3,
/// §16.7 item 3's second entry path). An observer receives no
/// scheduler broadcast — the daemon owns it — so it watches the
/// model: a prayer whose state newly enters `Adhan` raises the Adhan
/// overlay at the same instant the daemon's alarm fires, and one
/// whose state newly leaves `PostAdhan` (the window whose end is the
/// reminder's fire time, §11.3's `t + after`) raises the mosque
/// reminder. The owner watches too: §10.1's "audio playback and
/// Adhan *state* are independent by design" and §23 Phase 10's
/// "`adhan = false` plays nothing but **still** shows the overlay"
/// both require the visual alarm at the prayer's own instant even
/// when the planner gated the event off — the owner's broadcast and
/// its transition land together there, and [`raise_transition`]
/// keeps them from fighting over the slot. Latest transition wins,
/// matching the owner path's "one overlay at a time".
///
/// The payloads mirror `overlay_from`'s: the fire instant is the
/// boundary the transition crossed, and `late` is always false — an
/// observer is never the §11.5 catch-up path, and a transition seen
/// live is by definition on time.
pub fn overlay_from_transition(
    previous: &[PrayerState; 5],
    current: &[PrayerState; 5],
    model: &UiModel,
) -> Option<Overlay> {
    // Scan back-to-front so the latest prayer's transition wins when
    // two fire in the same tick (a rarity the plan spaces minutes
    // apart, but the tie-break must still be deterministic).
    for index in (0..Notifiable::ALL.len()).rev() {
        let prayer = Notifiable::ALL[index];
        let (was, is) = (previous[index], current[index]);
        if was == is {
            continue;
        }
        let settings = prayer.settings(&model.config.prayers);
        let at = model.window.today.instant(prayer).at;
        if is == PrayerState::Adhan {
            // §10.1: "audio playback and Adhan *state* are independent
            // by design" — the visual alarm never reads the switches,
            // so `adhan = false` mutes the call but not the overlay
            // (§23 Phase 10 item 8). The planner's gate still governs
            // the event, the audio and the notification.
            return Some(Overlay::AdhanAlert {
                prayer,
                at,
                late: false,
            });
        }
        if was == PrayerState::PostAdhan && is == PrayerState::Passed {
            // Unlike the Adhan alarm, the reminder has no
            // independence language: §16.8 item 3's entry is the
            // planner-gated `MosqueReminder` event, so a reminder
            // switched off must not flash its overlay either.
            if !(settings.after_minutes > 0
                && model.config.notifications.mosque_reminder
                && model.config.notifications.enabled)
            {
                continue;
            }
            let reminder_at = at + Duration::minutes(i64::from(settings.after_minutes));
            return Some(Overlay::MosqueReminder {
                prayer,
                reminder_at,
                late: false,
            });
        }
    }
    None
}

/// How a transition-detected overlay enters the slot: a broadcast
/// that already raised the *same* alarm wins — its payload is the
/// richer one, carrying §11.5's `late` flag, which a transition seen
/// live can never know — while a different alarm takes the slot,
/// matching the owner path's "one overlay at a time". Both loops call
/// this, so the owner's tick can never downgrade a late fire's
/// `(delayed)` wording.
pub fn raise_transition(current: Option<Overlay>, raised: Overlay) -> Option<Overlay> {
    match current {
        Some(current) if same_alarm(&current, &raised) => Some(current),
        _ => Some(raised),
    }
}

/// Whether two overlays are the same alarm — same variant, same
/// prayer — regardless of instants and flags.
fn same_alarm(a: &Overlay, b: &Overlay) -> bool {
    match (a, b) {
        (Overlay::AdhanAlert { prayer: left, .. }, Overlay::AdhanAlert { prayer: right, .. }) => {
            left == right
        }
        (
            Overlay::MosqueReminder { prayer: left, .. },
            Overlay::MosqueReminder { prayer: right, .. },
        ) => left == right,
        _ => false,
    }
}

/// The overlay a scheduler publication raises, if any (§16.7 item 3's
/// entry rule: scheduler events and observer transitions, never
/// keypresses). Shared by the full TUI's shell and quick mode's
/// `svc_rx` arm — both route their fires through this one mapping.
/// `Missed` raises nothing (the event log is where a user looks, §11.5).
pub fn overlay_from(event: &ServiceEvent) -> Option<Overlay> {
    let ServiceEvent::Fired { event: fired, late } = event else {
        return None;
    };
    match (fired.kind, fired.prayer) {
        (crate::domain::EventKind::Adhan, Some(prayer)) => Some(Overlay::AdhanAlert {
            prayer,
            at: fired.fire_at,
            late: *late,
        }),
        // §16.8 item 6's data: the reminder fires at `t + after`; the
        // Adhan it follows is the same instant, which the screen
        // recovers as `fire_at - after` from the live config's offsets
        // — no extra state to carry.
        (crate::domain::EventKind::MosqueReminder, Some(prayer)) => Some(Overlay::MosqueReminder {
            prayer,
            reminder_at: fired.fire_at,
            late: *late,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ScheduledEvent;
    use crate::domain::testing as dt;

    /// §16.7 item 3's observer path: the state machine's transitions
    /// are the alarm when no broadcast arrives. A prayer newly in
    /// `Adhan` raises the alert at its own instant; one leaving
    /// `PostAdhan` for `Passed` raises the mosque reminder at
    /// `t + after`; nothing else raises anything, and `late` is always
    /// false (a transition seen live is on time by definition).
    #[test]
    fn overlay_from_transition_detects_observer_alarms() {
        let fixture = crate::app::runtime::testing::snapshot_runtime();
        let model = crate::ui::model::build_model(&fixture.runtime);

        // 19:20: still PreAdhan for Maghrib (before = 5); 19:21 enters
        // Adhan — the transition raises the alert.
        let before = build_states_at(&model, 19, 20);
        let after = build_states_at(&model, 19, 21);
        let raised = overlay_from_transition(&before, &after, &model).expect("the Adhan entry");
        let at = dt::at(dt::TODAY, 19, 21, 0);
        assert_eq!(
            raised,
            Overlay::AdhanAlert {
                prayer: Notifiable::Maghrib,
                at,
                late: false
            }
        );

        // 19:36: Maghrib leaves PostAdhan (15 min after 19:21) — the
        // reminder's own fire time is `t + after`.
        let post = build_states_at(&model, 19, 35);
        let passed = build_states_at(&model, 19, 36);
        let raised = overlay_from_transition(&post, &passed, &model).expect("the reminder entry");
        assert_eq!(
            raised,
            Overlay::MosqueReminder {
                prayer: Notifiable::Maghrib,
                reminder_at: at + Duration::minutes(15),
                late: false
            }
        );

        // No transition, no overlay — same states either side of a tick.
        assert_eq!(overlay_from_transition(&before, &before, &model), None);

        // §10.1's independence rule, as §23 Phase 10 item 8 states it:
        // `adhan = false` plays nothing but still shows the overlay —
        // the visual alarm rides the state machine, which never reads
        // the switches.
        let mut muted = (*model.config).clone();
        muted.adhan.enabled = false;
        let muted = with_config(&model, muted);
        assert_eq!(
            overlay_from_transition(&before, &after, &muted),
            Some(Overlay::AdhanAlert {
                prayer: Notifiable::Maghrib,
                at,
                late: false
            })
        );

        // The reminder has no such independence (§16.8 item 3's entry
        // is the planner-gated event): switched off, its transition
        // raises nothing.
        let mut no_reminder = (*model.config).clone();
        no_reminder.notifications.mosque_reminder = false;
        let no_reminder = with_config(&model, no_reminder);
        assert_eq!(overlay_from_transition(&post, &passed, &no_reminder), None);
    }

    /// `raise_transition`'s rule: the same alarm already in the slot
    /// wins (the broadcast's payload carries the §11.5 `late` flag a
    /// live transition can never know), while a different alarm takes
    /// the slot — one overlay at a time.
    #[test]
    fn raise_transition_keeps_the_broadcasts_payload() {
        let at = dt::at(dt::TODAY, 19, 21, 0);
        let live = Overlay::AdhanAlert {
            prayer: Notifiable::Maghrib,
            at,
            late: true,
        };
        let seen = Overlay::AdhanAlert {
            prayer: Notifiable::Maghrib,
            at,
            late: false,
        };
        // The late fire's overlay is not downgraded by its own
        // transition arriving a tick later.
        assert_eq!(raise_transition(Some(live), seen), Some(live));

        // A different alarm replaces the slot, owner-path style.
        let reminder = Overlay::MosqueReminder {
            prayer: Notifiable::Maghrib,
            reminder_at: at + Duration::minutes(15),
            late: false,
        };
        assert_eq!(raise_transition(Some(live), reminder), Some(reminder));

        // An empty slot takes the transition's raise.
        assert_eq!(raise_transition(None, seen), Some(seen));
    }

    /// The fixture's states at a wall-clock time on the snapshot date,
    /// in `Notifiable::ALL` order. The evaluation is pure in `now`, so
    /// the clock never needs scripting for this.
    fn build_states_at(model: &UiModel, hour: u32, minute: u32) -> [PrayerState; 5] {
        let now = dt::at(dt::TODAY, hour, minute, 0);
        let mut out = [PrayerState::Upcoming; 5];
        for (index, prayer) in Notifiable::ALL.iter().enumerate() {
            out[index] = model.window.state_of(*prayer, &model.config, now, &[]);
        }
        out
    }

    /// The model with its configuration swapped — the transition
    /// detector reads only the window and the config, so this one
    /// change is all the gated cases need.
    fn with_config(model: &UiModel, config: crate::config::Config) -> UiModel {
        let mut swapped = model.clone();
        swapped.config = std::sync::Arc::new(config);
        swapped
    }

    /// §16.7 item 3's mapping, the one both UIs route their fires
    /// through: a fired `Adhan` raises the Adhan overlay, a fired
    /// `MosqueReminder` the mosque one, a `Missed` nothing (§11.5 —
    /// the event log is where a user looks), and the `late` flag rides
    /// the payload both ways.
    #[test]
    fn overlay_from_maps_fires_to_overlays() {
        let at = dt::at(dt::TODAY, 19, 21, 0);
        let adhan = ScheduledEvent {
            kind: crate::domain::EventKind::Adhan,
            prayer: Some(Notifiable::Maghrib),
            date: dt::TODAY,
            fire_at: at,
        };
        assert_eq!(
            overlay_from(&ServiceEvent::Fired {
                event: adhan,
                late: true
            }),
            Some(Overlay::AdhanAlert {
                prayer: Notifiable::Maghrib,
                at,
                late: true
            })
        );

        let reminder = ScheduledEvent {
            kind: crate::domain::EventKind::MosqueReminder,
            prayer: Some(Notifiable::Maghrib),
            date: dt::TODAY,
            fire_at: at,
        };
        assert_eq!(
            overlay_from(&ServiceEvent::Fired {
                event: reminder,
                late: false
            }),
            Some(Overlay::MosqueReminder {
                prayer: Notifiable::Maghrib,
                reminder_at: at,
                late: false
            })
        );

        // A missed event and an event with no prayer raise nothing.
        assert_eq!(overlay_from(&ServiceEvent::Missed { event: adhan }), None);
        let bare = ScheduledEvent {
            kind: crate::domain::EventKind::Adhan,
            prayer: None,
            date: dt::TODAY,
            fire_at: at,
        };
        assert_eq!(
            overlay_from(&ServiceEvent::Fired {
                event: bare,
                late: false
            }),
            None
        );
    }

    /// `on_service_event` raises the overlay and always asks for a
    /// repaint (§15.2).
    #[test]
    fn on_service_event_raises_and_rebuilds() {
        let mut shell = Shell::new();
        let at = dt::at(dt::TODAY, 19, 21, 0);
        let event = ServiceEvent::Fired {
            event: ScheduledEvent {
                kind: crate::domain::EventKind::Adhan,
                prayer: Some(Notifiable::Maghrib),
                date: dt::TODAY,
                fire_at: at,
            },
            late: false,
        };
        assert!(shell.on_service_event(event));
        assert_eq!(
            shell.overlay,
            Some(Overlay::AdhanAlert {
                prayer: Notifiable::Maghrib,
                at,
                late: false
            })
        );
    }

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
