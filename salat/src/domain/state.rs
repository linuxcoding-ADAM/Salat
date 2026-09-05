//! The prayer state machine (§10) — a pure function of time.
//!
//! The state machine answers "what is true now" and is evaluated on
//! every frame; the scheduler answers "what must fire, once" (§10.4).
//! They share [`PrayerWindow`] and the config but nothing else, and they
//! can disagree without harm. All rendering metadata (glyph, label,
//! colours) lives in `ui::theme`, not here — the domain must not know
//! about colours (§6.4).

use chrono::{DateTime, Duration};
use chrono_tz::Tz;

use crate::config::{Config, PrayerSettings};

use super::day::{PrayerWindow, next_prayer};
use super::prayer::Notifiable;

/// How long the visual Adhan state lasts (§10.1): Salat's constant, not
/// the audio file's length — the visual state must not depend on whether
/// audio played, how long the recording is, or whether a player exists.
pub const ADHAN_DISPLAY_SECS: i64 = 90;

/// A prayer's state at one instant (§6.4, §10.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrayerState {
    /// `now < t`, not the next one.
    Upcoming,
    /// `now < t` and the earliest future prayer.
    Next,
    /// `t − before ≤ now < t` (only when `before > 0`).
    PreAdhan,
    /// `t ≤ now < t + adhan_display`.
    Adhan,
    /// `t + adhan_display ≤ now < t + after` (only when `after > 0`).
    PostAdhan,
    /// `now ≥ t + max(after, adhan_display)`.
    Passed,
    /// The user marked it complete (A3) — highest precedence, reachable
    /// from any state, cleared at day rollover.
    Completed,
}

/// The §10.3 pure core: one instant's state, evaluated top-down in the
/// §10.1 table's order, first match wins.
///
/// * `next` is the earliest future prayer instant at `now` (what
///   [`next_prayer`] returns) — `Next` requires this instant to *be* it.
/// * `completions` are the prayers the user marked complete **for
///   `instant.date`** (A3; the caller stores them per date).
///
/// The two load-bearing precedence rules (§10.1) fall out of the order:
/// `PreAdhan` is tested before `Next`, and `Adhan`/`PostAdhan` before
/// `Passed`. The `before == 0` / `after == 0` degenerate cases skip
/// their state entirely rather than producing a zero-width window that
/// flickers.
pub fn state_of(
    instant: &super::day::PrayerInstant,
    next: Option<&super::day::PrayerInstant>,
    settings: &PrayerSettings,
    completions: &[Notifiable],
    now: DateTime<Tz>,
) -> PrayerState {
    let t = instant.at;
    let before = Duration::minutes(i64::from(settings.before_minutes));
    let after = Duration::minutes(i64::from(settings.after_minutes));
    let adhan_display = Duration::seconds(ADHAN_DISPLAY_SECS);

    // §10.1, top to bottom.
    if completions.contains(&instant.prayer) {
        return PrayerState::Completed;
    }
    if t <= now && now < t + adhan_display {
        return PrayerState::Adhan;
    }
    if settings.after_minutes > 0 && t + adhan_display <= now && now < t + after {
        return PrayerState::PostAdhan;
    }
    if settings.before_minutes > 0 && t - before <= now && now < t {
        return PrayerState::PreAdhan;
    }
    if now < t && next == Some(instant) {
        return PrayerState::Next;
    }
    if now < t {
        return PrayerState::Upcoming;
    }
    // Fall-through: now ≥ t and neither the Adhan nor the PostAdhan
    // window matched, which is exactly `now ≥ t + max(after,
    // adhan_display)` (§10.1) — when `after ≤ 90 s` the PostAdhan window
    // is empty and the fall-through starts at `t + adhan_display`.
    PrayerState::Passed
}

impl PrayerWindow {
    /// The §23 Phase 3 interface: today's row for one notifiable prayer.
    ///
    /// Resolves the prayer's instant on `self.today`, computes the
    /// earliest future prayer at `now`, and delegates to the pure
    /// [`state_of`] core. The PRAYERS table renders today's five rows,
    /// which is exactly what this answers.
    pub fn state_of(
        &self,
        prayer: Notifiable,
        config: &Config,
        now: DateTime<Tz>,
        completions: &[Notifiable],
    ) -> PrayerState {
        let instant = self.today.instant(prayer);
        let next = next_prayer(self, now);
        state_of(
            &instant,
            next.as_ref(),
            prayer.settings(&config.prayers),
            completions,
            now,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing;
    use super::*;
    use crate::app::clock::Clock as _;

    /// The §10.3 recipe, one prayer, all eight boundaries: Maghrib at
    /// 19:21 with the default `before = 5`, `after = 15`.
    #[test]
    fn the_eight_boundaries_of_the_specimen_prayer() {
        let (window, clock) = testing::fixture();
        let instant = window.today.instant(Notifiable::Maghrib);

        let at = |h, m, s| {
            clock.set(testing::utc(testing::TODAY, h, m, s));
            testing::state_of_window(&window, Notifiable::Maghrib, &clock)
        };
        // `next` is "the earliest future prayer at `now`" — so it is
        // recomputed at each boundary, exactly as the caller would.
        let core = |now| {
            let next = next_prayer(&window, now);
            state_of(
                &instant,
                next.as_ref(),
                &testing::default_maghrib_settings(),
                &[],
                now,
            )
        };

        // t − before − 1s → Next … t + after → Passed (§10.3).
        for (h, m, s, expected) in [
            (19, 15, 59, PrayerState::Next),
            (19, 16, 00, PrayerState::PreAdhan),
            (19, 20, 59, PrayerState::PreAdhan),
            (19, 21, 00, PrayerState::Adhan),
            (19, 22, 29, PrayerState::Adhan),     // t + 89 s
            (19, 22, 30, PrayerState::PostAdhan), // t + 90 s
            (19, 35, 59, PrayerState::PostAdhan), // t + after − 1 s
            (19, 36, 00, PrayerState::Passed),    // t + after
        ] {
            let now = testing::at(testing::TODAY, h, m, s);
            let got = core(now);
            assert_eq!(got, expected, "at {h:02}:{m:02}:{s:02}");
            // The window-level §23 interface agrees with the pure core.
            assert_eq!(
                at(h, m, s),
                expected,
                "window interface at {h:02}:{m:02}:{s:02}"
            );
        }
    }

    /// §10.3 / §20.2: `before = 0` skips `PreAdhan` entirely — one second
    /// before the instant the state is still `Next`.
    #[test]
    fn zero_before_skips_pre_adhan() {
        let settings = PrayerSettings {
            before_minutes: 0,
            after_minutes: 15,
            adhan: true,
            notify: true,
        };
        let instant = testing::maghrib_instant();
        let next = Some(instant);
        let one_before = testing::at(testing::TODAY, 19, 20, 59);
        assert_eq!(
            state_of(&instant, next.as_ref(), &settings, &[], one_before),
            PrayerState::Next
        );
    }

    /// §10.3 / §20.2: `after = 0` skips `PostAdhan` entirely — the row
    /// goes straight from `Adhan` to `Passed` at `t + 90 s`.
    #[test]
    fn zero_after_skips_post_adhan() {
        let settings = PrayerSettings {
            before_minutes: 5,
            after_minutes: 0,
            adhan: true,
            notify: true,
        };
        let instant = testing::maghrib_instant();
        let t = instant.at;
        assert_eq!(
            state_of(&instant, None, &settings, &[], t + Duration::seconds(89)),
            PrayerState::Adhan
        );
        assert_eq!(
            state_of(&instant, None, &settings, &[], t + Duration::seconds(90)),
            PrayerState::Passed
        );
    }

    /// `after < adhan_display` (e.g. 30 s): the PostAdhan window is empty
    /// and `Passed` starts at `t + 90 s` — `max(after, adhan_display)`
    /// (§10.1).
    #[test]
    fn an_after_smaller_than_adhan_display_never_post_adhan() {
        let settings = PrayerSettings {
            before_minutes: 0,
            after_minutes: 1, // 1 min < 90 s
            adhan: true,
            notify: true,
        };
        let instant = testing::maghrib_instant();
        let t = instant.at;
        for offset in [90, 91, 120] {
            assert_eq!(
                state_of(
                    &instant,
                    None,
                    &settings,
                    &[],
                    t + Duration::seconds(offset)
                ),
                PrayerState::Passed,
                "at t + {offset} s"
            );
        }
    }

    /// §10.2 / §20.2: `Completed` overrides everything — including
    /// `Adhan` at the instant itself and `Upcoming` before it (a user
    /// praying early, §10.2).
    #[test]
    fn completed_outranks_every_state() {
        let settings = testing::default_maghrib_settings();
        let instant = testing::maghrib_instant();
        let t = instant.at;
        let completed = [Notifiable::Maghrib];
        for (now, would_be) in [
            (t - Duration::minutes(30), PrayerState::Upcoming),
            (t - Duration::minutes(1), PrayerState::PreAdhan),
            (t, PrayerState::Adhan),
            (t + Duration::minutes(10), PrayerState::PostAdhan),
            (t + Duration::minutes(30), PrayerState::Passed),
        ] {
            assert_eq!(
                state_of(&instant, None, &settings, &[], now),
                would_be,
                "precondition at {now}"
            );
            assert_eq!(
                state_of(&instant, None, &settings, &completed, now),
                PrayerState::Completed,
                "completion must override at {now}"
            );
        }
        // And it is per prayer: Isha's completion does not touch Maghrib.
        let other = [Notifiable::Isha];
        assert_eq!(
            state_of(&instant, None, &settings, &other, t),
            PrayerState::Adhan
        );
    }

    /// §10.1's load-bearing rule: `PreAdhan` outranks `Next` — at
    /// `t − 1 s` the instant is both the earliest future prayer and
    /// inside its pre-window, and the design's yellow "soon" wins.
    #[test]
    fn pre_adhan_beats_next() {
        let settings = testing::default_maghrib_settings();
        let instant = testing::maghrib_instant();
        let now = instant.at - Duration::seconds(1);
        // Precondition: the instant IS the next prayer here.
        assert_eq!(
            next_prayer(&testing::fixture_window(), now).unwrap(),
            instant
        );
        assert_eq!(
            state_of(&instant, Some(&instant), &settings, &[], now),
            PrayerState::PreAdhan
        );
    }

    /// §10.1's other load-bearing rule: `Adhan`/`PostAdhan` outrank a
    /// naive "the instant has passed, so `Passed`" reading.
    #[test]
    fn adhan_states_outrank_passed() {
        let settings = testing::default_maghrib_settings();
        let instant = testing::maghrib_instant();
        let t = instant.at;
        assert_eq!(
            state_of(&instant, None, &settings, &[], t + Duration::seconds(89)),
            PrayerState::Adhan
        );
        assert_eq!(
            state_of(&instant, None, &settings, &[], t + Duration::minutes(14)),
            PrayerState::PostAdhan
        );
    }

    /// `Next` requires *being* the earliest future prayer: at the
    /// snapshot, Maghrib is `Next` and Isha is `Upcoming`.
    #[test]
    fn only_the_earliest_future_prayer_is_next() {
        let (window, clock) = testing::fixture();
        let now = clock.now_in(testing::TZ);
        assert_eq!(
            window.state_of(Notifiable::Maghrib, &testing::default_config(), now, &[]),
            PrayerState::Next
        );
        assert_eq!(
            window.state_of(Notifiable::Isha, &testing::default_config(), now, &[]),
            PrayerState::Upcoming
        );
        // The passed rows of the morning are Passed.
        assert_eq!(
            window.state_of(Notifiable::Fajr, &testing::default_config(), now, &[]),
            PrayerState::Passed
        );
    }

    /// The window interface honours per-prayer config: a `before = 0`
    /// Isha is `Next` (not `PreAdhan`) one second before its instant.
    #[test]
    fn the_window_interface_honours_per_prayer_config() {
        let window = testing::fixture_window();
        let mut config = testing::default_config();
        config.prayers.isha.before_minutes = 0;
        let now = testing::at(testing::TODAY, 20, 46, 59);
        assert_eq!(
            window.state_of(Notifiable::Isha, &config, now, &[]),
            PrayerState::Next
        );
        // …and completions flow through it.
        let completed = [Notifiable::Isha];
        assert_eq!(
            window.state_of(Notifiable::Isha, &config, now, &completed),
            PrayerState::Completed
        );
    }
}
