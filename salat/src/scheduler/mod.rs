//! Scheduler handle + commands (§11).
//!
//! The scheduler is the side-effect engine behind [`plan_events`]: a
//! runner task that fires each planned event **exactly once** at the
//! right wall-clock second — surviving redraws, resizes, navigation,
//! configuration changes, day boundaries, DST, suspend/resume, NTP
//! steps and late starts (§11.1) — entirely independent of the render
//! loop. The runner lives in [`runner`]; the §11.5 catch-up judgement
//! in [`catchup`].

mod catchup;
mod runner;

pub use catchup::{CatchupDecision, decide, grace_for};
pub use runner::{Scheduler, SchedulerDeps, SchedulerHandle};

use async_trait::async_trait;

use crate::config::Config;
use crate::domain::{EventKind, Notifiable, ScheduledEvent};

/// §A10: re-fire a snoozed Adhan three minutes later.
pub const ADHAN_SNOOZE: chrono::Duration = chrono::Duration::minutes(3);

/// §A10: re-fire a snoozed mosque reminder five minutes later.
pub const MOSQUE_SNOOZE: chrono::Duration = chrono::Duration::minutes(5);

/// The side-effect seam (§23 Phase 8): what actually happens when an
/// event fires. Phase 8's production handler is the recording stub —
/// [`TracingSink`] — which logs; Phase 9 (notifications) and Phase 10
/// (audio, overlays) replace it with the real handlers.
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn on_event(&self, event: ScheduledEvent, late: bool);
}

/// The Phase 8 handler: every fired event is logged with the §11.5
/// `late` flag. Phases 9–10 swap in notify-rust and audio.
pub struct TracingSink;

#[async_trait]
impl EventSink for TracingSink {
    async fn on_event(&self, event: ScheduledEvent, late: bool) {
        tracing::info!(kind = ?event.kind, prayer = ?event.prayer, late, "scheduler event fired");
    }
}

/// The scheduler's read on the prayer window — the service layer's
/// seam against the app layer (§5.1: the scheduler depends on the
/// domain, never on the TUI runtime). `window` returning `None` means
/// the owning runtime is gone and the runner exits.
#[async_trait]
pub trait WindowSource: Send + Sync {
    /// The currently served window.
    async fn window(&self) -> Option<std::sync::Arc<crate::domain::PrayerWindow>>;

    /// Revalidate and re-serve — the horizon top-up (§11.3).
    async fn refresh(&self);

    /// The rollover refresh (§11.6): refetch, roll the window, reset
    /// completions.
    async fn rollover(&self);
}

/// What the scheduler publishes to the UI's `svc_rx` select arm
/// (§15.2) — a reason to repaint, never an overlay trigger (§4.3:
/// overlays are reactions to the state machine, not event messages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceEvent {
    /// An event fired; `late` carries the §11.5 flag.
    Fired { event: ScheduledEvent, late: bool },
    /// An event was judged missed — already in the event log.
    Missed { event: ScheduledEvent },
}

/// Runtime commands into the runner (§11.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Re-plan from the current window and configuration (a config
    /// change, a SIGHUP republish, a `Handle::replan`).
    Refresh,
    /// Snooze one shot (§11.7): re-fire `source`'s alarm after its
    /// snooze window. Runtime-only, never persisted.
    Snooze { source: ScheduledEvent },
    /// Stop the runner.
    Shutdown,
}

/// The category of an event-log row for one kind (§17.2's five
/// categories).
pub(crate) fn log_category(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Adhan | EventKind::AdhanSnooze => "adhan",
        EventKind::PreNotify | EventKind::MosqueReminder | EventKind::ReminderSnooze => {
            "notification"
        }
        EventKind::DayRollover => "system",
    }
}

/// The prayer names the event log uses — title case, the §11.5
/// specimens' own (`Maghrib Adhan missed`), not the dashboard's
/// uppercase table labels.
pub(crate) fn log_prayer(prayer: Notifiable) -> &'static str {
    match prayer {
        Notifiable::Fajr => "Fajr",
        Notifiable::Dhuhr => "Dhuhr",
        Notifiable::Asr => "Asr",
        Notifiable::Maghrib => "Maghrib",
        Notifiable::Isha => "Isha",
    }
}

/// The event-log row for a **fired** event — the label/detail wording
/// follows the §17.2 and §11.5 specimens (`Pre-Adhan notification` /
/// `MAGHRIB · 5 min window`, `Maghrib Adhan fired (delayed)`).
///
/// Returns `(label, detail)`; the caller stamps the row at the event's
/// own `fire_at` (§11.5's `18:21 │ Maghrib Adhan …` rows are stamped
/// with the scheduled time, not the discovery time).
pub(crate) fn fired_row(
    event: &ScheduledEvent,
    late: bool,
    cfg: &Config,
) -> (String, Option<String>) {
    let prayer = event.prayer.map(log_prayer).unwrap_or_default();
    let upper = event.prayer.map(|p| p.display()).unwrap_or_default();
    let settings = |p: Option<Notifiable>| {
        p.map(|p| p.settings(&cfg.prayers))
            .map(|s| (s.before_minutes, s.after_minutes))
            .unwrap_or((0, 0))
    };

    let (label, detail) = match event.kind {
        EventKind::PreNotify => {
            let (before, _) = settings(event.prayer);
            (
                "Pre-Adhan notification".to_string(),
                Some(format!("{upper} · {before} min window")),
            )
        }
        EventKind::Adhan => (format!("{prayer} Adhan fired"), None),
        EventKind::MosqueReminder => {
            let (_, after) = settings(event.prayer);
            (
                "Mosque reminder".to_string(),
                Some(format!("{upper} · Iqama ~{after} min")),
            )
        }
        EventKind::DayRollover => ("Day rollover".to_string(), None),
        EventKind::AdhanSnooze => (format!("{prayer} Adhan snooze fired"), None),
        EventKind::ReminderSnooze => (format!("{prayer} Mosque reminder snooze fired"), None),
    };

    // §11.5: a catch-up fire inside its grace window carries the
    // `(delayed)` prefix — the specimen's exact wording.
    let label = if late {
        format!("{label} (delayed)")
    } else {
        label
    };
    (label, detail)
}

/// The event-log row for a **missed** event (§11.5): the specimen's
/// `Maghrib Adhan missed` / `app was not running`.
pub(crate) fn missed_row(event: &ScheduledEvent) -> (String, &'static str) {
    let kind_word = match event.kind {
        EventKind::PreNotify => "Pre-Adhan notification",
        EventKind::Adhan => "Adhan",
        EventKind::MosqueReminder => "Mosque reminder",
        // Only the three ledgered kinds have a grace window to miss.
        kind => {
            debug_assert!(
                false,
                "missed_row is only called for grace-carrying kinds, got {kind:?}"
            );
            "event"
        }
    };
    let prayer = event.prayer.map(log_prayer).unwrap_or_default();
    (
        format!("{prayer} {kind_word} missed"),
        "app was not running",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::testing;

    /// The §17.2 specimen verbatim: a fired PreNotify row is
    /// `Pre-Adhan notification` / `MAGHRIB · 5 min window`.
    #[test]
    fn a_fired_pre_notify_matches_the_specimen() {
        let window = testing::fixture_window();
        let cfg = testing::default_config();
        let event = crate::domain::plan_events(&window, &cfg, testing::snapshot_now())
            .into_iter()
            .find(|event| event.kind == EventKind::PreNotify)
            .expect("the plan holds a PreNotify");
        let (label, detail) = fired_row(&event, false, &cfg);
        assert_eq!(label, "Pre-Adhan notification");
        assert_eq!(detail.as_deref(), Some("MAGHRIB · 5 min window"));
    }

    /// The §11.5 specimens: `Maghrib Adhan fired (delayed)` and
    /// `Maghrib Adhan missed` / `app was not running`.
    #[test]
    fn delayed_and_missed_rows_match_the_specimens() {
        let window = testing::fixture_window();
        let cfg = testing::default_config();
        let event = crate::domain::plan_events(&window, &cfg, testing::snapshot_now())
            .into_iter()
            .find(|event| {
                event.kind == EventKind::Adhan && event.prayer == Some(Notifiable::Maghrib)
            })
            .expect("the plan holds Maghrib's Adhan");

        let (label, detail) = fired_row(&event, true, &cfg);
        assert_eq!(label, "Maghrib Adhan fired (delayed)");
        assert_eq!(detail, None);

        let (label, detail) = missed_row(&event);
        assert_eq!(label, "Maghrib Adhan missed");
        assert_eq!(detail, "app was not running");
    }
}
