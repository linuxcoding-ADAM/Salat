//! A day of prayer times and the rolling three-day window (§6.2–§6.3).
//!
//! Single-day data is insufficient: after Isha, "next prayer" is
//! tomorrow's Fajr, and before Fajr "previous prayer" is yesterday's
//! Isha. [`PrayerWindow`] holds the days, and its [`PrayerWindow::instants`]
//! is the *single* chronological input to next-prayer selection, gauge
//! computation and schedule planning — all three agree by construction.

use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;

use super::{Notifiable, Prayer};

/// The Hijri date of a civil day (§6.2) — displayed as the dashboard's
/// greeting line, e.g. `12 Safar 1448 AH`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HijriDate {
    pub day: u32,
    /// `1..=12`.
    pub month_number: u32,
    /// English month name, e.g. `Rabīʿ al-awwal`.
    pub month_en: String,
    /// Arabic month name, e.g. `رَبيع الأوّل`.
    pub month_ar: String,
    pub year: i32,
}

/// What the API *actually used* to compute a day (§6.2, §7.7) — not what
/// we asked for. The Status and Settings screens display this so a silent
/// method remap or school fallback is visible rather than trusted.
#[derive(Debug, Clone, PartialEq)]
pub struct DayMeta {
    pub method_id: u32,
    pub method_name: String,
    /// `0` Shafi, `1` Hanafi — the school the API echoes back.
    pub school: u8,
    pub lat: f64,
    pub lng: f64,
    /// The IANA zone name the API computed this day in.
    pub tz_name: String,
}

/// One civil day of prayer times in one location (§6.2).
#[derive(Debug, Clone, PartialEq)]
pub struct PrayerDay {
    /// Civil date in `tz`.
    pub date: NaiveDate,
    /// The IANA zone this day was computed in.
    pub tz: Tz,
    /// The six prayer instants, indexed by [`Prayer`] order (§6.1).
    pub times: [DateTime<Tz>; 6],
    /// May be on `date + 1` (§7.2).
    pub midnight: DateTime<Tz>,
    /// May be on `date + 1` (§7.2).
    pub last_third: DateTime<Tz>,
    pub hijri: HijriDate,
    pub meta: DayMeta,
}

impl PrayerDay {
    /// The instant of one prayer — `day.times[Prayer::Fajr]` without the
    /// caller having to know the array order.
    pub fn time(&self, prayer: Prayer) -> DateTime<Tz> {
        self.times[prayer as usize]
    }

    /// This day's instant of a notifiable prayer, tagged with its civil
    /// date — the unit every derived value in the domain works with.
    pub fn instant(&self, prayer: Notifiable) -> PrayerInstant {
        PrayerInstant {
            prayer,
            at: self.time(prayer.as_prayer()),
            date: self.date,
        }
    }
}

/// One notifiable prayer at one instant (§6.3). `date` is the prayer's
/// civil date — the day the prayer *belongs to*, which is also the dedup
/// key component (§11.4) — not merely the date part of `at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrayerInstant {
    pub prayer: Notifiable,
    pub at: DateTime<Tz>,
    /// The civil date this prayer belongs to.
    pub date: NaiveDate,
}

/// The rolling schedule window (§6.3): yesterday (for "previous prayer"
/// before Fajr), today, and tomorrow.
#[derive(Debug, Clone, PartialEq)]
pub struct PrayerWindow {
    pub today: PrayerDay,
    pub tomorrow: PrayerDay,
    /// `None` only in the degraded no-yesterday case (a fresh cache at
    /// boot, before the first month fetch completes).
    pub yesterday: Option<PrayerDay>,
}

impl PrayerWindow {
    /// Every notifiable prayer instant across yesterday → tomorrow,
    /// chronologically sorted (§6.3). Sunrise never appears (A2). This is
    /// the input to `next_prayer`, `previous_prayer`, the gauge and the
    /// planner, so they cannot disagree about what the prayers are.
    pub fn instants(&self) -> Vec<PrayerInstant> {
        let mut instants: Vec<PrayerInstant> = Vec::with_capacity(15);
        for day in self.yesterday.iter().chain([&self.today, &self.tomorrow]) {
            instants.extend(Notifiable::ALL.map(|prayer| day.instant(prayer)));
        }
        instants.sort_by_key(|instant| instant.at);
        instants
    }
}

/// The earliest prayer strictly after `now` (§6.5). Exactly *at* an
/// instant does not count — at 19:21:00 the next prayer is Isha, because
/// Maghrib itself is in the Adhan state (§10.1), not a countdown target.
pub fn next_prayer(window: &PrayerWindow, now: DateTime<Tz>) -> Option<PrayerInstant> {
    window
        .instants()
        .into_iter()
        .find(|instant| instant.at > now)
}

/// The latest prayer at or before `now` (§6.5) — yesterday's Isha before
/// Fajr when the window has a yesterday. Exactly *at* an instant counts:
/// at 19:21:00 the previous prayer is Maghrib.
pub fn previous_prayer(window: &PrayerWindow, now: DateTime<Tz>) -> Option<PrayerInstant> {
    window
        .instants()
        .into_iter()
        .rev()
        .find(|instant| instant.at <= now)
}

#[cfg(test)]
mod tests {
    use super::super::Countdown;
    use super::super::testing;
    use super::*;
    use chrono::TimeZone as _;

    /// §20.2 next-prayer: before Fajr → today's Fajr.
    #[test]
    fn before_fajr_next_is_todays_fajr() {
        let window = testing::fixture_window();
        let now = testing::at(testing::TODAY, 4, 0, 0);
        let next = next_prayer(&window, now).unwrap();
        assert_eq!(next.prayer, Notifiable::Fajr);
        assert_eq!(next.date, testing::TODAY);
    }

    /// §20.2 next-prayer: between Asr and Maghrib → Maghrib.
    #[test]
    fn between_asr_and_maghrib_next_is_maghrib() {
        let window = testing::fixture_window();
        let now = testing::snapshot_now();
        let next = next_prayer(&window, now).unwrap();
        assert_eq!(next.prayer, Notifiable::Maghrib);
    }

    /// §20.2 next-prayer: after Isha → **tomorrow's** Fajr (§6.3's whole
    /// reason for existing).
    #[test]
    fn after_isha_next_is_tomorrows_fajr() {
        let window = testing::fixture_window();
        let now = testing::at(testing::TODAY, 23, 30, 0);
        let next = next_prayer(&window, now).unwrap();
        assert_eq!(next.prayer, Notifiable::Fajr);
        assert_eq!(next.date, testing::TOMORROW);
        assert_eq!(next.at, testing::at(testing::TOMORROW, 5, 10, 0));
    }

    /// §20.2 next-prayer: exactly at Maghrib → Isha (an instant is not its
    /// own successor).
    #[test]
    fn exactly_at_maghrib_next_is_isha() {
        let window = testing::fixture_window();
        let now = testing::at(testing::TODAY, 19, 21, 0);
        let next = next_prayer(&window, now).unwrap();
        assert_eq!(next.prayer, Notifiable::Isha);
    }

    /// §20.2 / A2: Sunrise is never returned — at the fixture's sunrise
    /// (06:44) the next prayer is Dhuhr.
    #[test]
    fn sunrise_is_never_next() {
        let window = testing::fixture_window();
        let now = testing::at(testing::TODAY, 6, 44, 0);
        assert_eq!(next_prayer(&window, now).unwrap().prayer, Notifiable::Dhuhr);
        // And it is absent from the window entirely: 15 instants over
        // three days, every one notifiable.
        let instants = window.instants();
        assert_eq!(instants.len(), 15);
        assert!(
            instants
                .iter()
                .all(|i| i.prayer.as_prayer() != Prayer::Sunrise)
        );
    }

    /// §6.3: before Fajr the previous prayer is *yesterday's* Isha.
    #[test]
    fn before_fajr_previous_is_yesterdays_isha() {
        let window = testing::fixture_window();
        let now = testing::at(testing::TODAY, 4, 0, 0);
        let previous = previous_prayer(&window, now).unwrap();
        assert_eq!(previous.prayer, Notifiable::Isha);
        assert_eq!(previous.date, testing::YESTERDAY);

        // Without a yesterday there is no previous prayer before Fajr.
        let mut no_yesterday = window.clone();
        no_yesterday.yesterday = None;
        assert_eq!(previous_prayer(&no_yesterday, now), None);
    }

    /// Exactly at an instant, that instant is the previous prayer.
    #[test]
    fn at_maghrib_previous_is_maghrib() {
        let window = testing::fixture_window();
        let now = testing::at(testing::TODAY, 19, 21, 0);
        let previous = previous_prayer(&window, now).unwrap();
        assert_eq!(previous.prayer, Notifiable::Maghrib);
    }

    /// `instants()` is chronologically sorted across the whole window,
    /// including the yesterday → today and today → tomorrow seams.
    #[test]
    fn instants_are_chronological_across_days() {
        let window = testing::fixture_window();
        let instants = window.instants();
        for pair in instants.windows(2) {
            assert!(pair[0].at <= pair[1].at, "not sorted: {:?}", pair);
        }
        // Without a yesterday: ten instants, still sorted.
        let mut no_yesterday = window;
        no_yesterday.yesterday = None;
        assert_eq!(no_yesterday.instants().len(), 10);
    }

    /// `PrayerDay::time` indexes `times` by [`Prayer`], and `instant`
    /// tags the instant with the day's civil date.
    #[test]
    fn day_time_and_instant_use_the_prayer_index() {
        let day = testing::fixture_window().today;
        assert_eq!(day.time(Prayer::Fajr), testing::at(testing::TODAY, 5, 9, 0));
        assert_eq!(
            day.time(Prayer::Sunrise),
            testing::at(testing::TODAY, 6, 44, 0)
        );
        let instant = day.instant(Notifiable::Isha);
        assert_eq!(instant.at, testing::at(testing::TODAY, 20, 47, 0));
        assert_eq!(instant.date, testing::TODAY);
    }

    /// §20.2 timezone: a `+01:00` fixed-offset instant and the same
    /// instant in `Africa/Algiers` are the same moment — the domain's
    /// conversions are instant-preserving, not field-copies.
    #[test]
    fn fixed_offset_round_trips_through_the_location_zone() {
        let offset = chrono::FixedOffset::east_opt(3600).unwrap();
        let wire = {
            let naive = chrono::NaiveDate::from_ymd_opt(2026, 9, 5)
                .unwrap()
                .and_hms_opt(17, 38, 23)
                .unwrap();
            DateTime::<chrono::FixedOffset>::from_naive_utc_and_offset(
                naive - chrono::Duration::hours(1),
                offset,
            )
        };
        let algiers = testing::TZ.from_utc_datetime(&wire.naive_utc());
        assert_eq!(algiers.to_rfc3339(), "2026-09-05T17:38:23+01:00");
        // …and back.
        let back = algiers.with_timezone(&offset);
        assert_eq!(back, wire);
    }

    /// §20.2 / §10.2: the *location's* zone defines "today". At 00:30 on
    /// Sep 5 in Africa/Algiers the UTC date is still Sep 4 — yet the
    /// window's day is Sep 5 and its Fajr (05:09) is next.
    #[test]
    fn the_location_zone_not_the_host_defines_today() {
        let window = testing::fixture_window();
        let now = testing::at(testing::TODAY, 0, 30, 0); // 2026-09-04T23:30Z
        assert_eq!(
            now.with_timezone(&chrono::Utc).date_naive(),
            testing::YESTERDAY
        );
        let next = next_prayer(&window, now).unwrap();
        assert_eq!(next.date, testing::TODAY);
        assert_eq!(next.prayer, Notifiable::Fajr);
        let previous = previous_prayer(&window, now).unwrap();
        assert_eq!(previous.date, testing::YESTERDAY);
    }

    /// §20.2: a real DST date — Europe/Berlin's last Sunday of March
    /// 2026 (the 29th), with correct offsets on both sides of the jump
    /// and no 02:00 existing in between.
    #[test]
    fn berlin_spring_forward_has_the_right_offsets_on_both_sides() {
        let berlin: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        let before = berlin
            .from_local_datetime(
                &chrono::NaiveDate::from_ymd_opt(2026, 3, 29)
                    .unwrap()
                    .and_hms_opt(1, 59, 0)
                    .unwrap(),
            )
            .single()
            .unwrap();
        assert_eq!(before.to_rfc3339(), "2026-03-29T01:59:00+01:00");

        // 02:00 does not exist on the transition date.
        let skipped = berlin.from_local_datetime(
            &chrono::NaiveDate::from_ymd_opt(2026, 3, 29)
                .unwrap()
                .and_hms_opt(2, 0, 0)
                .unwrap(),
        );
        assert_eq!(skipped, chrono::LocalResult::None);

        let after = berlin
            .from_local_datetime(
                &chrono::NaiveDate::from_ymd_opt(2026, 3, 29)
                    .unwrap()
                    .and_hms_opt(3, 0, 0)
                    .unwrap(),
            )
            .single()
            .unwrap();
        assert_eq!(after.to_rfc3339(), "2026-03-29T03:00:00+02:00");

        // The two readings are one chronological minute apart — the jump
        // eats an hour of wall time, not of instants.
        assert_eq!((after - before).num_minutes(), 1);
    }

    /// §20.2: a window that *spans* the Berlin jump — next_prayer and
    /// the countdown cross DST with no special handling, because every
    /// value below the domain is an instant, never a wall time.
    #[test]
    fn a_window_spanning_the_dst_jump_still_selects_correctly() {
        let berlin: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 3, 29).unwrap();
        let local = |h, m| {
            berlin
                .from_local_datetime(&date.and_hms_opt(h, m, 0).unwrap())
                .single()
                .unwrap()
        };
        // A minimal but well-formed window: yesterday late Isha, today
        // Fajr at 06:00 (already +02:00, after the 02:00 jump).
        let mut window = testing::fixture_window();
        window.today.tz = berlin;
        window.today.times = [
            local(6, 0),  // Fajr
            local(7, 30), // Sunrise
            local(13, 15),
            local(16, 45),
            local(19, 30),
            local(21, 0), // Isha
        ];
        let now = berlin
            .from_local_datetime(&date.and_hms_opt(1, 30, 0).unwrap())
            .single()
            .unwrap(); // +01:00, before the jump
        assert_eq!(now.to_rfc3339(), "2026-03-29T01:30:00+01:00");

        let next = next_prayer(&window, now).unwrap();
        assert_eq!(next.prayer, Notifiable::Fajr);
        assert_eq!(next.at.to_rfc3339(), "2026-03-29T06:00:00+02:00");
        // 01:30 +01:00 (00:30Z) → 06:00 +02:00 (04:00Z) is 3.5 hours of
        // instants across the jump.
        let countdown = Countdown::between(next.at, now);
        assert_eq!(countdown.format(), "03:30:00");
    }

    /// §20.2: the autumn side — Berlin's last Sunday of October 2026
    /// (the 25th), where 03:00 folds back to 02:00 and local times in
    /// the fold hour are ambiguous. The domain never constructs such a
    /// time itself; when the tz database must resolve one, `earliest()`
    /// picks the first occurrence.
    #[test]
    fn berlin_autumn_fold_is_resolved_earliest() {
        let berlin: chrono_tz::Tz = "Europe/Berlin".parse().unwrap();
        let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        let ambiguous = berlin.from_local_datetime(&date.and_hms_opt(2, 30, 0).unwrap());
        match ambiguous {
            chrono::LocalResult::Ambiguous(earliest, latest) => {
                assert_eq!(earliest.to_rfc3339(), "2026-10-25T02:30:00+02:00");
                assert_eq!(latest.to_rfc3339(), "2026-10-25T02:30:00+01:00");
            }
            other => panic!("02:30 on 2026-10-25 in Berlin must be ambiguous, got {other:?}"),
        }
        // On both sides of the fold the offsets are stable and correct.
        let before = berlin
            .from_local_datetime(&date.and_hms_opt(1, 0, 0).unwrap())
            .single()
            .unwrap();
        assert_eq!(before.to_rfc3339(), "2026-10-25T01:00:00+02:00");
        let after = berlin
            .from_local_datetime(&date.and_hms_opt(3, 0, 0).unwrap())
            .single()
            .unwrap();
        assert_eq!(after.to_rfc3339(), "2026-10-25T03:00:00+01:00");
    }
}
