//! The `Clock` trait — the only source of "now" (§6.6).
//!
//! No code outside [`SystemClock`] may call `Utc::now()`, `Local::now()`
//! or `SystemTime::now()`; `clippy.toml` enforces it via
//! `disallowed-methods`. This is the single most important testability
//! decision in the plan (§6.6): it turns "does the state machine handle
//! the second Maghrib begins" from a flaky integration test into a
//! two-line unit test that advances [`MockClock`].

use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
/// The application's time source (§6.6).
pub trait Clock: Send + Sync {
    /// The current instant, in UTC.
    fn now_utc(&self) -> DateTime<Utc>;

    /// The current instant in a location's zone — the **only** correct
    /// way to obtain local time, because §10.2 defines "today" by the
    /// configured zone, not the host's.
    fn now_in(&self, tz: Tz) -> DateTime<Tz> {
        self.now_utc().with_timezone(&tz)
    }

    /// Elapsed time on a monotonic source (§11.3). The baseline is
    /// arbitrary but fixed per process; what matters is that the value
    /// never steps with the wall clock — `CLOCK_MONOTONIC` does not
    /// advance while the machine is suspended, which is exactly what
    /// makes `|Δwall − Δmono|` the suspend/NTP-step detector.
    fn now_monotonic(&self) -> Duration;
}

/// The real clock. The one place in the codebase where the wall clock
/// is read legally — see the `#[allow]` below and `clippy.toml`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

/// The `Instant` the first `SystemClock` reading was taken from — the
/// baseline `now_monotonic` measures against.
fn process_start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

impl Clock for SystemClock {
    // `Utc::now` is disallowed everywhere else by clippy.toml; this is
    // the single sanctioned read (§6.6).
    #[allow(clippy::disallowed_methods)]
    fn now_utc(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn now_monotonic(&self) -> Duration {
        Instant::now() - process_start()
    }
}

/// A settable, advanceable clock for tests (§6.6, §20.1). Thread-safe:
/// tests share one clock across the scheduler and its assertions, and
/// no `sleep` is ever needed — timing tests *advance* the clock.
///
/// The wall reading and the monotonic counter are independent (§11.8):
/// `advance` moves both — the honest passage of time — while `set` and
/// `advance_wall_only` move the wall alone, simulating an NTP step or a
/// suspend/resume (during which `CLOCK_MONOTONIC` stands still).
#[derive(Debug)]
pub struct MockClock {
    now: Mutex<DateTime<Utc>>,
    mono: Mutex<Duration>,
}

impl MockClock {
    /// A clock pinned to one instant.
    pub fn at(utc: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(utc),
            mono: Mutex::new(Duration::ZERO),
        }
    }

    /// Jump to an instant — the wall reading only. A set is a clock
    /// step by definition, so the monotonic counter is left untouched.
    pub fn set(&self, utc: DateTime<Utc>) {
        *self.lock() = utc;
    }

    /// Advance (or, with a negative duration, step back) by an exact
    /// amount — the test-side counterpart of waiting. Forward motion is
    /// honest passage and moves the monotonic counter with it; a
    /// negative duration is a wall-clock step back and leaves the
    /// counter alone.
    pub fn advance(&self, delta: chrono::Duration) {
        *self.lock() += delta;
        if delta > chrono::Duration::zero() {
            *self.mono.lock().unwrap_or_else(PoisonError::into_inner) +=
                delta.to_std().unwrap_or(Duration::ZERO);
        }
    }

    /// Move the wall reading while the monotonic counter stands still —
    /// a suspend/resume or an NTP step, the §11.3 jump cases.
    pub fn advance_wall_only(&self, delta: chrono::Duration) {
        *self.lock() += delta;
    }

    /// The current reading. A poisoned lock is recovered, not
    /// propagated: the value inside is still the last honest reading,
    /// and a panicking test thread must not take the clock down with it.
    fn lock(&self) -> MutexGuard<'_, DateTime<Utc>> {
        self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Clock for MockClock {
    fn now_utc(&self) -> DateTime<Utc> {
        *self.lock()
    }

    fn now_monotonic(&self) -> Duration {
        *self.mono.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Offset as _};

    /// The §20.3 snapshot instant in UTC (17:38:23 in Africa/Algiers).
    fn snapshot_utc() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-05T16:38:23Z")
            .expect("the literal is a valid RFC 3339 timestamp")
            .with_timezone(&Utc)
    }

    /// §6.6: `set` and `advance` move the reading exactly.
    #[test]
    fn mock_clock_sets_and_advances() {
        let start = snapshot_utc();
        let clock = MockClock::at(start);
        assert_eq!(clock.now_utc(), start);

        let later = start + chrono::Duration::hours(3);
        clock.set(later);
        assert_eq!(clock.now_utc(), later);

        clock.advance(chrono::Duration::minutes(90));
        assert_eq!(clock.now_utc(), later + chrono::Duration::minutes(90));

        clock.advance(chrono::Duration::minutes(-90));
        assert_eq!(clock.now_utc(), later);
    }

    /// §6.6: `now_in` converts into the location's zone — the snapshot
    /// is 17:38:23 in Africa/Algiers (UTC+1, no DST) and 18:38:23 in
    /// Europe/Berlin (CEST, UTC+2).
    #[test]
    fn now_in_converts_to_the_location_zone() {
        let utc = snapshot_utc();
        let clock = MockClock::at(utc);
        let algiers = clock.now_in(chrono_tz::Africa::Algiers);
        assert_eq!(algiers.to_rfc3339(), "2026-09-05T17:38:23+01:00");
        let berlin = clock.now_in(chrono_tz::Europe::Berlin);
        assert_eq!(berlin.to_rfc3339(), "2026-09-05T18:38:23+02:00");
        // Round-trip: same instant.
        assert_eq!(algiers.with_timezone(&Utc), utc);
        assert_eq!(berlin.with_timezone(&Utc), utc);
    }

    /// The real clock returns a plausible instant and honours `now_in` —
    /// no equality assertion against `Utc::now()` (that would need a
    /// second wall-clock read and a flake); plausibility is enough to
    /// prove the wiring.
    #[test]
    fn system_clock_reads_a_plausible_instant() {
        let clock = SystemClock;
        let now = clock.now_utc();
        assert!(
            now.year() >= 2026,
            "built in 2026 — the clock is behind: {now}"
        );
        let algiers = clock.now_in(chrono_tz::Africa::Algiers);
        assert_eq!(algiers.offset().fix().local_minus_utc(), 3600);
    }

    /// §11.3/§11.8: the two readings are independent — `set` and
    /// `advance_wall_only` step the wall without moving the monotonic
    /// counter (suspend/NTP), while `advance` moves both (honest time).
    #[test]
    fn mock_clock_monotonic_tracks_passage_not_jumps() {
        let start = snapshot_utc();
        let clock = MockClock::at(start);
        assert_eq!(clock.now_monotonic(), std::time::Duration::ZERO);

        clock.advance(chrono::Duration::minutes(90));
        assert_eq!(
            clock.now_monotonic(),
            std::time::Duration::from_secs(90 * 60),
            "honest passage moves the counter"
        );

        // A six-hour wall-only jump (suspend/resume): the wall races
        // ahead, the counter does not — |Δwall − Δmono| = 6 h.
        clock.advance_wall_only(chrono::Duration::hours(6));
        assert_eq!(
            clock.now_monotonic(),
            std::time::Duration::from_secs(90 * 60),
            "a wall-only jump leaves the counter still"
        );
        assert_eq!(
            clock.now_utc(),
            start + chrono::Duration::minutes(90) + chrono::Duration::hours(6)
        );

        // A set is a step: the counter is untouched.
        clock.set(start);
        assert_eq!(
            clock.now_monotonic(),
            std::time::Duration::from_secs(90 * 60)
        );

        // Stepping back is a wall jump, not backwards passage.
        clock.advance(chrono::Duration::minutes(-90));
        assert_eq!(
            clock.now_monotonic(),
            std::time::Duration::from_secs(90 * 60)
        );
    }

    /// The real monotonic reading never runs backwards and stays small
    /// (it is measured from a per-process baseline, not the epoch).
    #[test]
    fn system_clock_monotonic_is_monotonic() {
        let clock = SystemClock;
        let first = clock.now_monotonic();
        let second = clock.now_monotonic();
        assert!(
            second >= first,
            "monotonic ran backwards: {first:?} → {second:?}"
        );
        assert!(
            first < std::time::Duration::from_secs(24 * 60 * 60),
            "the baseline is per-process, not the epoch: {first:?}"
        );
    }
}
