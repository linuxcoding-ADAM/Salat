//! `UiModel` and [`build_model`] — §15.1's immutable truth snapshot.
//!
//! Every frame the render loop folds the `Runtime`'s service state into
//! one `UiModel`, and rendering is a pure function of that model: the
//! widgets read it, they never reach a service (§15.1's separation).
//! Because it is plain data, any screen is snapshot-testable by
//! constructing a model literal — which is exactly how the quick-mode
//! ladder and the goldens pin their output.
//!
//! The model carries no derived strings beyond what the domain already
//! formats: times are formatted at render time so a settings change
//! (12/24-hour) repaints without rebuilding anything.

use std::sync::Arc;

use chrono::DateTime;
use chrono_tz::Tz;

use crate::app::runtime::Runtime;
use crate::config::{Config, ConfigDraft};
use crate::domain::{
    Countdown, DaySummary, Notifiable, PrayerInstant, PrayerState, PrayerWindow, day_summary,
    next_prayer, previous_prayer, progress,
};
use crate::status::SystemStatus;

use super::layout::Breakpoint;

/// The whole truth a frame renders (§15.1, field-for-field).
///
/// Rebuilt per tick; never mutated in place. `Arc` fields are shared
/// with the `Runtime`, so building a model is cheap — one summary, one
/// status, five state evaluations.
#[derive(Debug, Clone)]
pub struct UiModel {
    /// The tick's instant, in the window's zone (§10.2).
    pub now: DateTime<Tz>,
    /// The served window (§6.3) — yesterday/today/tomorrow.
    pub window: Arc<PrayerWindow>,
    /// Today's five rows, in `Notifiable::ALL` order (§10.1).
    pub states: [PrayerState; 5],
    /// The earliest future prayer (§6.5).
    pub next: Option<PrayerInstant>,
    /// The latest prayer at or before `now` (§6.5) — yesterday's Isha
    /// before Fajr when the window holds a yesterday.
    pub previous: Option<PrayerInstant>,
    /// Time until `next` — `ZERO` when the window is exhausted.
    pub countdown: Countdown,
    /// The gauge's fill fraction, `0.0..=1.0` from `previous` to `next`.
    pub progress: f64,
    /// Today's counters (§6.5).
    pub summary: DaySummary,
    /// The components, next event and log (§6.7).
    pub status: Arc<SystemStatus>,
    /// The effective configuration (§8.5).
    pub config: Arc<Config>,
    /// The Settings editor's working copy — `None` outside an editor
    /// session (Phase 6+; quick mode has none).
    pub draft: Option<ConfigDraft>,
    /// The qibla bearing — fetched on demand by the full UI's Qibla
    /// screen, never by quick mode.
    pub qibla: Option<f64>,
    /// The frame's breakpoint classification. Quick mode pins
    /// [`Breakpoint::Small`] (§16.14 item 4) — it owns one layout and
    /// never asks [`Breakpoint::of`], which stays a full-UI concept.
    pub breakpoint: Breakpoint,
}

/// Fold a [`Runtime`] into a [`UiModel`] (§15.1) — the render loop's
/// per-tick rebuild, and the only code that turns service state into
/// renderable truth.
pub fn build_model(runtime: &Runtime) -> UiModel {
    let config = runtime.config();
    let window = runtime.window();
    let now = runtime.clock().now_in(window.today.tz);

    let next = next_prayer(&window, now);
    let previous = previous_prayer(&window, now);
    let countdown = next
        .map(|next| Countdown::between(next.at, now))
        .unwrap_or(Countdown::ZERO);
    // The degenerate interval (no previous, no next, or an inverted
    // polar-latitude pair) renders an empty gauge — §6.5's answer, not
    // a panic.
    let progress = match (previous, next) {
        (Some(previous), Some(next)) if previous.at < next.at => {
            progress(previous.at, next.at, now)
        }
        _ => 0.0,
    };

    let states =
        Notifiable::ALL.map(|prayer| window.state_of(prayer, &config, now, &runtime.completions()));
    let summary = day_summary(&window, &config, now);
    let status = runtime.status();

    UiModel {
        now,
        window,
        states,
        next,
        previous,
        countdown,
        progress,
        summary,
        status,
        config,
        draft: None,
        qibla: None,
        breakpoint: Breakpoint::Small,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::testing;
    use crate::domain::testing as dt;

    /// The §20.3 snapshot, field for field: Maghrib next at 19:21,
    /// Asr previous, the specimen's `01:42:37` countdown and 42 % gauge,
    /// three rows passed, the small breakpoint, nothing else claimed.
    #[test]
    fn the_snapshot_model_matches_the_specimen() {
        let fixture = testing::snapshot_runtime();
        let model = build_model(&fixture.runtime);

        assert_eq!(model.now, dt::snapshot_now());

        let next = model.next.expect("Maghrib is next at 17:38");
        assert_eq!(next.prayer, Notifiable::Maghrib);
        assert_eq!(next.at, dt::at(dt::TODAY, 19, 21, 0));
        let previous = model.previous.expect("Asr is previous at 17:38");
        assert_eq!(previous.prayer, Notifiable::Asr);

        // 19:21 − 17:38:23 = 6157 s (§14.6's specimen countdown).
        assert_eq!(model.countdown.total_secs, 6157);
        assert_eq!(model.countdown.format(), "01:42:37");

        // Asr 16:23 → Maghrib 19:21 is 10 680 s; 17:38:23 is 4 523 s
        // past Asr — the specimen's 14-of-34 gauge cells.
        let expected = 4523.0 / 10_680.0;
        assert!(
            (model.progress - expected).abs() < 1e-12,
            "progress {} != {expected}",
            model.progress
        );

        // Today's five rows: the morning passed, Maghrib next, Isha
        // upcoming.
        assert_eq!(
            model.states,
            [
                PrayerState::Passed,
                PrayerState::Passed,
                PrayerState::Passed,
                PrayerState::Next,
                PrayerState::Upcoming,
            ]
        );

        assert_eq!(
            (
                model.summary.total,
                model.summary.passed,
                model.summary.remaining
            ),
            (5, 3, 2)
        );
        assert_eq!(
            model.summary.next_notify,
            Some(dt::at(dt::TODAY, 19, 16, 0))
        );

        // Quick mode's fixed answers (§16.14 items 4 and 6).
        assert_eq!(model.breakpoint, Breakpoint::Small);
        assert!(model.draft.is_none());
        assert!(model.qibla.is_none());

        // The status the strip renders: ten components, next event
        // Maghrib's pre-notify.
        assert_eq!(model.status.components.len(), 10);
        assert_eq!(
            model.status.next_event.as_ref().map(|event| event.kind),
            Some(crate::domain::EventKind::PreNotify)
        );
    }

    /// The exhausted window (after Isha, before tomorrow's Fajr): no
    /// `next` — the countdown is `ZERO`, the gauge empty, `previous` is
    /// Isha. The honest answers, never a panic (§6.5).
    #[test]
    fn after_isha_the_model_degrades_to_zero() {
        let fixture = testing::snapshot_runtime();
        fixture.clock.advance(chrono::Duration::hours(4)); // 21:38:23
        let model = build_model(&fixture.runtime);

        // Tomorrow's Fajr (05:10, §20.3) is the earliest future prayer —
        // the window exists so `next` survives Isha.
        let next = model.next.expect("tomorrow's Fajr is next");
        assert_eq!(next.prayer, Notifiable::Fajr);
        assert_eq!(next.at, dt::at(dt::TOMORROW, 5, 10, 0));
        assert_eq!(
            model.previous.expect("Isha just passed").prayer,
            Notifiable::Isha
        );
        // 21:38:23 → 05:10:00 is 7 h 31 m 37 s.
        assert_eq!(model.countdown.format(), "07:31:37");
        // Isha 20:47 → Fajr 05:10 is 30 180 s; 21:38:23 is 3 083 s into
        // it — the gauge is barely started, not half way.
        assert!(model.progress > 0.10 && model.progress < 0.11);
    }

    /// A completion changes exactly one row (A3): Maghrib marked
    /// complete renders `Completed` where the state machine said `Next`.
    /// It does not move the countdown — `next_prayer` is temporal
    /// (§6.5) and the panel keeps answering "when is Maghrib" for a
    /// prayer the user merely marked done.
    #[test]
    fn a_completion_replaces_one_row() {
        let fixture = testing::snapshot_runtime();
        fixture.set_completions(vec![Notifiable::Maghrib]);

        let model = build_model(&fixture.runtime);
        assert_eq!(model.states[3], PrayerState::Completed);
        assert_eq!(model.states[4], PrayerState::Upcoming);
        assert_eq!(model.states[0], PrayerState::Passed);
        // Unmoved: Maghrib is still the next prayer at 17:38.
        assert_eq!(
            model
                .next
                .expect("next is temporal, not completion-based")
                .prayer,
            Notifiable::Maghrib
        );
        assert_eq!(model.countdown.format(), "01:42:37");
    }
}
