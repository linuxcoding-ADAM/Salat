//! The `Clock` trait — the only source of "now" (§6.6).
//!
//! No code outside [`SystemClock`] may call `Utc::now()`, `Local::now()`
//! or `SystemTime::now()`; `clippy.toml` enforces it via
//! `disallowed-methods`. This is the single most important testability
//! decision in the plan (§6.6): it turns "does the state machine handle
//! the second Maghrib begins" from a flaky integration test into a
//! two-line unit test that advances [`MockClock`].

use std::sync::{Mutex, MutexGuard, PoisonError};

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
}

/// The real clock. The one place in the codebase where the wall clock
/// is read legally — see the `#[allow]` below and `clippy.toml`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    // `Utc::now` is disallowed everywhere else by clippy.toml; this is
    // the single sanctioned read (§6.6).
    #[allow(clippy::disallowed_methods)]
    fn now_utc(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A settable, advanceable clock for tests (§6.6, §20.1). Thread-safe:
/// tests share one clock across the scheduler and its assertions, and
/// no `sleep` is ever needed — timing tests *advance* the clock.
#[derive(Debug)]
pub struct MockClock {
    now: Mutex<DateTime<Utc>>,
}

impl MockClock {
    /// A clock pinned to one instant.
    pub fn at(utc: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(utc),
        }
    }

    /// Jump to an instant.
    pub fn set(&self, utc: DateTime<Utc>) {
        *self.lock() = utc;
    }

    /// Advance (or, with a negative duration, step back) by an exact
    /// amount — the test-side counterpart of waiting.
    pub fn advance(&self, delta: chrono::Duration) {
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
}
