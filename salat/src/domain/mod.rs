//! The prayer-time domain (§6): pure types and pure functions, time as a
//! parameter, no I/O anywhere in the module (§23 Phase 3).
//!
//! The domain holds all the logic that would otherwise be untestable
//! inside async services: next-prayer selection, the state machine
//! (§10), countdowns, event planning (§11.3) and the day summary. Every
//! time-dependent answer takes `now` (or a [`crate::app::clock::Clock`])
//! as an argument — the trait itself lives in `app::clock` (§6.6) only
//! because it is an application service, not domain data.

pub mod countdown;
pub mod day;
pub mod prayer;
pub mod schedule;
pub mod state;
pub mod summary;

pub use countdown::{Countdown, progress};
pub use day::{
    DayMeta, HijriDate, PrayerDay, PrayerInstant, PrayerWindow, next_prayer, previous_prayer,
};
pub use prayer::{Notifiable, Prayer};
pub use schedule::{EventKind, ScheduledEvent, midnight_in, plan_events};
pub use state::{ADHAN_DISPLAY_SECS, PrayerState, state_of};
pub use summary::{DaySummary, day_summary};

/// The §20.3 snapshot inputs, shared by every module's tests: the
/// fixture window (Fajr 05:09 · Dhuhr 12:47 · Asr 16:23 · Maghrib 19:21
/// · Isha 20:47 over 2026-09-04/05/06 in `Africa/Algiers`), `MockClock`
/// pinned at 17:38:23, and the §8.3 default config. With these fixed,
/// every derived value is a pure function of the fixtures.
#[cfg(test)]
pub(crate) mod testing {
    use chrono::{DateTime, NaiveDate, TimeZone, Utc};
    use chrono_tz::Tz;

    use super::super::app::clock::{Clock as _, MockClock};
    use super::*;

    /// The fixture location's zone — Sétif, Algeria (UTC+1, no DST, so
    /// every date arithmetic below is unambiguous).
    pub(crate) const TZ: Tz = chrono_tz::Africa::Algiers;

    const fn fixed_date(year: i32, month: u32, day: u32) -> NaiveDate {
        match NaiveDate::from_ymd_opt(year, month, day) {
            Some(date) => date,
            None => panic!("the fixture dates are valid"),
        }
    }

    pub(crate) const YESTERDAY: NaiveDate = fixed_date(2026, 9, 4);
    pub(crate) const TODAY: NaiveDate = fixed_date(2026, 9, 5);
    pub(crate) const TOMORROW: NaiveDate = fixed_date(2026, 9, 6);

    /// A local wall-clock instant in the fixture zone.
    pub(crate) fn at(date: NaiveDate, hour: u32, minute: u32, second: u32) -> DateTime<Tz> {
        let naive = date
            .and_hms_opt(hour, minute, second)
            .expect("00:00..23:59 is a valid time of day");
        TZ.from_local_datetime(&naive)
            .single()
            .expect("Africa/Algiers has no DST, so local times are never ambiguous")
    }

    /// The same wall-clock reading expressed in UTC (for `MockClock::set`).
    pub(crate) fn utc(date: NaiveDate, hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
        at(date, hour, minute, second).with_timezone(&Utc)
    }

    /// §20.3's pinned instant: 2026-09-05T17:38:23+01:00.
    pub(crate) fn snapshot_now() -> DateTime<Tz> {
        at(TODAY, 17, 38, 23)
    }

    /// One day's shape: the six prayer times, plus midnight and the last
    /// third which may fall on the next civil day (§7.2).
    struct DaySpec {
        date: NaiveDate,
        /// (hour, minute) per prayer, in [`Prayer`] order.
        times: [(u32, u32); 6],
        /// (date, hour, minute) — often `date + 1`.
        midnight: (NaiveDate, u32, u32),
        last_third: (NaiveDate, u32, u32),
        /// The Hijri day this civil date maps to — the recorded
        /// calendar's answer (§7.8), which lags the civil date by 18
        /// days in this month, so it cannot be derived from `date`.
        hijri_day: u32,
    }

    fn day(spec: DaySpec) -> PrayerDay {
        PrayerDay {
            date: spec.date,
            tz: TZ,
            times: spec.times.map(|(h, m)| at(spec.date, h, m, 0)),
            midnight: at(spec.midnight.0, spec.midnight.1, spec.midnight.2, 0),
            last_third: at(spec.last_third.0, spec.last_third.1, spec.last_third.2, 0),
            hijri: HijriDate {
                // The API's real answer for this window: Rabīʿ al-awwal
                // 1448 (the calendar fixture, §7.8 — Sep 1 → 19, so
                // Sep 5 → 23).
                day: spec.hijri_day,
                month_number: 3,
                month_en: "Rabīʿ al-awwal".to_owned(),
                month_ar: "رَبيع الأوّل".to_owned(),
                year: 1448,
            },
            meta: DayMeta {
                method_id: 12,
                method_name: "Union Organization Islamic de France".to_owned(),
                school: 0,
                lat: 36.19,
                lng: 5.41,
                tz_name: "Africa/Algiers".to_owned(),
            },
        }
    }

    /// The §20.3 fixture window over Sep 4/5/6 2026. Times drift a
    /// minute per day, as real ones do.
    pub(crate) fn fixture_window() -> PrayerWindow {
        PrayerWindow {
            yesterday: Some(day(DaySpec {
                date: YESTERDAY,
                times: [(5, 8), (6, 43), (12, 46), (16, 24), (19, 22), (20, 48)],
                midnight: (TODAY, 0, 36),
                last_third: (TODAY, 2, 46),
                hijri_day: 22,
            })),
            today: day(DaySpec {
                date: TODAY,
                times: [(5, 9), (6, 44), (12, 47), (16, 23), (19, 21), (20, 47)],
                midnight: (TOMORROW, 0, 37),
                last_third: (TOMORROW, 2, 47),
                hijri_day: 23,
            }),
            tomorrow: day(DaySpec {
                date: TOMORROW,
                times: [(5, 10), (6, 45), (12, 47), (16, 22), (19, 19), (20, 45)],
                midnight: (
                    NaiveDate::from_ymd_opt(2026, 9, 7).expect("2026-09-07 is a valid date"),
                    0,
                    36,
                ),
                last_third: (
                    NaiveDate::from_ymd_opt(2026, 9, 7).expect("2026-09-07 is a valid date"),
                    2,
                    46,
                ),
                hijri_day: 24,
            }),
        }
    }

    /// The fixture window plus `MockClock` pinned at the snapshot.
    pub(crate) fn fixture() -> (PrayerWindow, MockClock) {
        let clock = MockClock::at(utc(TODAY, 17, 38, 23));
        (fixture_window(), clock)
    }

    /// The §8.3 defaults.
    pub(crate) fn default_config() -> crate::config::Config {
        crate::config::Config::default()
    }

    /// The default `[prayers.maghrib]` — `before = 5`, `after = 15`, both
    /// switches on (§8.3).
    pub(crate) fn default_maghrib_settings() -> crate::config::PrayerSettings {
        default_config().prayers.maghrib
    }

    /// Today's Maghrib as a standalone instant (19:21, §20.3).
    pub(crate) fn maghrib_instant() -> PrayerInstant {
        PrayerInstant {
            prayer: Notifiable::Maghrib,
            at: at(TODAY, 19, 21, 0),
            date: TODAY,
        }
    }

    /// The §23 window-level `state_of` against the default config, with
    /// `now` taken from a clock.
    pub(crate) fn state_of_window(
        window: &PrayerWindow,
        prayer: Notifiable,
        clock: &MockClock,
    ) -> PrayerState {
        window.state_of(prayer, &default_config(), clock.now_in(TZ), &[])
    }
}
