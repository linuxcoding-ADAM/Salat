//! The pure event planner (§11.3).
//!
//! "Does a config with `before = 0` produce a pre-notify event" is a
//! unit test with no clock, no I/O and no async — because the planner is
//! pure. The *runner* (Phase 8) is the side-effect engine that fires
//! these events; this module only decides what should exist.

use chrono::{DateTime, Duration, NaiveDate, TimeZone};
use chrono_tz::Tz;

use crate::config::Config;

use super::day::PrayerWindow;
use super::prayer::Notifiable;

/// What a [`ScheduledEvent`] makes the app do (§11.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Desktop notification at `t − before`.
    PreNotify,
    /// Adhan playback at `t`.
    Adhan,
    /// "Time to head out" notification at `t + after`.
    MosqueReminder,
    /// The day boundary at local midnight: refetch, replan, clear
    /// completions (§11.3).
    DayRollover,
    /// A user-snoozed Adhan, re-fired once after the snooze window
    /// (§11.7). In-memory and one-shot: never planned, never ledgered.
    AdhanSnooze,
    /// A user-snoozed mosque reminder, same rules as
    /// [`EventKind::AdhanSnooze`] (§11.7).
    ReminderSnooze,
}

impl EventKind {
    /// The snooze durations are the runner's constants (§A10); the kind
    /// only needs to answer whether it is a one-shot snooze event —
    /// those bypass the fired-events ledger (§11.7).
    pub fn is_snooze(self) -> bool {
        matches!(self, EventKind::AdhanSnooze | EventKind::ReminderSnooze)
    }
}

/// One thing the scheduler must fire, once (§11.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledEvent {
    pub kind: EventKind,
    /// `None` for `DayRollover`.
    pub prayer: Option<Notifiable>,
    /// The prayer's civil date — part of the fired-events dedup key
    /// (§11.4), so two adjacent days' Maghrib reminders can never be
    /// mistaken for one another.
    pub date: NaiveDate,
    /// The absolute wall-clock instant to fire at.
    pub fire_at: DateTime<Tz>,
}

/// The events worth firing from a window (§11.3): for each notifiable
/// prayer of `today` **and** `tomorrow` —
///
/// * `PreNotify` at `t − before`, when `before > 0 && notify &&
///   notifications.enabled`,
/// * `Adhan` at `t`, when `adhan && adhan.enabled`,
/// * `MosqueReminder` at `t + after`, when `after > 0 &&
///   mosque_reminder && notifications.enabled`,
///
/// plus one `DayRollover` at tomorrow's local midnight. The result is
/// sorted ascending by `fire_at`.
///
/// Strictly past events (`fire_at < now`) are dropped; an event due at
/// exactly `now` is kept, because the runner fires `fire_at ≤ now`. The
/// §11.5 catch-up grace is the runner's concern — it replans with a
/// past-aware judgement, not this function.
pub fn plan_events(window: &PrayerWindow, cfg: &Config, now: DateTime<Tz>) -> Vec<ScheduledEvent> {
    let mut events = Vec::new();

    for day in [&window.today, &window.tomorrow] {
        for prayer in Notifiable::ALL {
            let settings = prayer.settings(&cfg.prayers);
            let t = day.time(prayer.as_prayer());

            if settings.before_minutes > 0 && settings.notify && cfg.notifications.enabled {
                events.push(ScheduledEvent {
                    kind: EventKind::PreNotify,
                    prayer: Some(prayer),
                    date: day.date,
                    fire_at: t - Duration::minutes(i64::from(settings.before_minutes)),
                });
            }
            if settings.adhan && cfg.adhan.enabled {
                events.push(ScheduledEvent {
                    kind: EventKind::Adhan,
                    prayer: Some(prayer),
                    date: day.date,
                    fire_at: t,
                });
            }
            if settings.after_minutes > 0
                && cfg.notifications.mosque_reminder
                && cfg.notifications.enabled
            {
                events.push(ScheduledEvent {
                    kind: EventKind::MosqueReminder,
                    prayer: Some(prayer),
                    date: day.date,
                    fire_at: t + Duration::minutes(i64::from(settings.after_minutes)),
                });
            }
        }
    }

    // One rollover at tomorrow's local midnight (§11.3).
    if let Some(midnight) = midnight_in(window.tomorrow.tz, window.tomorrow.date) {
        events.push(ScheduledEvent {
            kind: EventKind::DayRollover,
            prayer: None,
            date: window.tomorrow.date,
            fire_at: midnight,
        });
    }

    events.retain(|event| event.fire_at >= now);
    events.sort_by_key(|event| event.fire_at);
    events
}

/// The earliest existing local midnight of `date` in `tz`.
///
/// A few zones perform their DST jump *at* midnight (Cuba, the Azores),
/// so 00:00 can be skipped or — rarer — ambiguous on the transition
/// date. Walk forward hour by hour (a DST jump is one hour; four steps
/// cover every zone in the tz database) and take the first instant that
/// exists, `earliest()` resolving ambiguity. The degenerate "no hour of
/// the night exists" case returns `None` and the caller plans no
/// rollover — better an honest gap than a fabricated instant.
pub fn midnight_in(tz: Tz, date: NaiveDate) -> Option<DateTime<Tz>> {
    for hour in 0..4 {
        let Some(naive) = date.and_hms_opt(hour, 0, 0) else {
            continue;
        };
        if let Some(instant) = tz.from_local_datetime(&naive).earliest() {
            return Some(instant);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::testing;
    use super::*;

    /// §23 Phase 3 item 8: with the default config, the snapshot window
    /// plans **exactly** this set. Today's morning events are past and
    /// dropped; every surviving event of today's Maghrib/Isha and all of
    /// tomorrow's fifteen is listed, plus the rollover.
    #[test]
    fn default_config_plans_exactly_the_expected_set() {
        let window = testing::fixture_window();
        let cfg = testing::default_config();
        let now = testing::snapshot_now();

        let expected: Vec<ScheduledEvent> = {
            let t = testing::TODAY;
            let tm = testing::TOMORROW;
            // (kind, prayer, date, h, m)
            let rows: [(EventKind, Notifiable, NaiveDate, u32, u32); 21] = [
                (EventKind::PreNotify, Notifiable::Maghrib, t, 19, 16),
                (EventKind::Adhan, Notifiable::Maghrib, t, 19, 21),
                (EventKind::MosqueReminder, Notifiable::Maghrib, t, 19, 36),
                (EventKind::PreNotify, Notifiable::Isha, t, 20, 32),
                (EventKind::Adhan, Notifiable::Isha, t, 20, 47),
                (EventKind::MosqueReminder, Notifiable::Isha, t, 21, 17),
                (EventKind::PreNotify, Notifiable::Fajr, tm, 4, 55),
                (EventKind::Adhan, Notifiable::Fajr, tm, 5, 10),
                (EventKind::MosqueReminder, Notifiable::Fajr, tm, 5, 30),
                (EventKind::PreNotify, Notifiable::Dhuhr, tm, 12, 37),
                (EventKind::Adhan, Notifiable::Dhuhr, tm, 12, 47),
                (EventKind::MosqueReminder, Notifiable::Dhuhr, tm, 13, 17),
                (EventKind::PreNotify, Notifiable::Asr, tm, 16, 7),
                (EventKind::Adhan, Notifiable::Asr, tm, 16, 22),
                (EventKind::MosqueReminder, Notifiable::Asr, tm, 16, 42),
                (EventKind::PreNotify, Notifiable::Maghrib, tm, 19, 14),
                (EventKind::Adhan, Notifiable::Maghrib, tm, 19, 19),
                (EventKind::MosqueReminder, Notifiable::Maghrib, tm, 19, 34),
                (EventKind::PreNotify, Notifiable::Isha, tm, 20, 30),
                (EventKind::Adhan, Notifiable::Isha, tm, 20, 45),
                (EventKind::MosqueReminder, Notifiable::Isha, tm, 21, 15),
            ];
            let mut events: Vec<ScheduledEvent> = rows
                .map(|(kind, prayer, date, h, m)| ScheduledEvent {
                    kind,
                    prayer: Some(prayer),
                    date,
                    fire_at: testing::at(date, h, m, 0),
                })
                .to_vec();
            // Rollover at tomorrow's local midnight (§11.3).
            events.push(ScheduledEvent {
                kind: EventKind::DayRollover,
                prayer: None,
                date: tm,
                fire_at: testing::at(tm, 0, 0, 0),
            });
            events.sort_by_key(|event| event.fire_at);
            events
        };

        assert_eq!(plan_events(&window, &cfg, now), expected);
    }

    /// §20.2: strictly past events are dropped — at the snapshot, all of
    /// today's Fajr/Dhuhr/Asr events are gone.
    #[test]
    fn past_events_are_dropped() {
        let window = testing::fixture_window();
        let cfg = testing::default_config();
        let now = testing::snapshot_now();
        let events = plan_events(&window, &cfg, now);
        assert!(events.iter().all(|e| e.fire_at >= now));
        assert!(
            !events
                .iter()
                .any(|e| e.prayer == Some(Notifiable::Fajr) && e.date == testing::TODAY)
        );
        assert!(!events.iter().any(|e| e.kind == EventKind::Adhan
            && e.date == testing::TODAY
            && e.prayer == Some(Notifiable::Asr)));
    }

    /// An event due at exactly `now` survives — the runner fires
    /// `fire_at ≤ now`, so the planner must keep the boundary instant.
    #[test]
    fn an_event_due_exactly_now_survives() {
        let window = testing::fixture_window();
        let cfg = testing::default_config();
        let now = testing::at(testing::TODAY, 19, 21, 0); // Maghrib adhan
        let events = plan_events(&window, &cfg, now);
        assert!(events.iter().any(|e| e.kind == EventKind::Adhan
            && e.prayer == Some(Notifiable::Maghrib)
            && e.date == testing::TODAY));
    }

    /// §11.3: `notify = false` removes only the PreNotify — Adhan and the
    /// MosqueReminder are separate settings.
    #[test]
    fn notify_false_removes_only_pre_notify() {
        let window = testing::fixture_window();
        let mut cfg = testing::default_config();
        cfg.prayers.isha.notify = false;
        let now = testing::snapshot_now();
        let events = plan_events(&window, &cfg, now);
        let isha_today =
            |e: &ScheduledEvent| e.date == testing::TODAY && e.prayer == Some(Notifiable::Isha);
        assert!(
            !events
                .iter()
                .any(|e| isha_today(e) && e.kind == EventKind::PreNotify)
        );
        assert!(
            events
                .iter()
                .any(|e| isha_today(e) && e.kind == EventKind::Adhan)
        );
        assert!(
            events
                .iter()
                .any(|e| isha_today(e) && e.kind == EventKind::MosqueReminder)
        );
    }

    /// §11.3: `adhan = false` removes the Adhan event only.
    #[test]
    fn adhan_false_removes_only_the_adhan_event() {
        let window = testing::fixture_window();
        let mut cfg = testing::default_config();
        cfg.prayers.maghrib.adhan = false;
        let now = testing::snapshot_now();
        let events = plan_events(&window, &cfg, now);
        assert!(!events.iter().any(|e| e.date == testing::TODAY
            && e.prayer == Some(Notifiable::Maghrib)
            && e.kind == EventKind::Adhan));
        assert!(events.iter().any(|e| e.date == testing::TODAY
            && e.prayer == Some(Notifiable::Maghrib)
            && e.kind == EventKind::PreNotify));
    }

    /// §20.2: `notifications.enabled = false` silences every
    /// notification-bearing event; only Adhan (gated by `adhan.enabled`
    /// alone) and the rollover remain.
    #[test]
    fn notifications_disabled_leaves_only_adhan_and_rollover() {
        let window = testing::fixture_window();
        let mut cfg = testing::default_config();
        cfg.notifications.enabled = false;
        let now = testing::snapshot_now();
        let events = plan_events(&window, &cfg, now);
        assert!(
            events
                .iter()
                .all(|e| e.kind == EventKind::Adhan || e.kind == EventKind::DayRollover)
        );
        assert!(events.iter().any(|e| e.kind == EventKind::Adhan));
        assert!(events.iter().any(|e| e.kind == EventKind::DayRollover));
    }

    /// §20.2: `adhan.enabled = false` (the global switch) removes every
    /// Adhan event; notifications remain.
    #[test]
    fn adhan_enabled_false_removes_every_adhan_event() {
        let window = testing::fixture_window();
        let mut cfg = testing::default_config();
        cfg.adhan.enabled = false;
        let now = testing::snapshot_now();
        let events = plan_events(&window, &cfg, now);
        assert!(!events.iter().any(|e| e.kind == EventKind::Adhan));
        assert!(events.iter().any(|e| e.kind == EventKind::PreNotify));
    }

    /// §23 Phase 3 item 8: `before = 0` produces no PreNotify and
    /// `after = 0` no MosqueReminder — no zero-width events.
    #[test]
    fn zero_before_and_after_produce_no_edge_events() {
        let window = testing::fixture_window();
        let mut cfg = testing::default_config();
        cfg.prayers.maghrib.before_minutes = 0;
        cfg.prayers.isha.after_minutes = 0;
        let now = testing::snapshot_now();
        let events = plan_events(&window, &cfg, now);
        assert!(!events.iter().any(|e| e.date == testing::TODAY
            && e.prayer == Some(Notifiable::Maghrib)
            && e.kind == EventKind::PreNotify));
        assert!(!events.iter().any(|e| e.date == testing::TODAY
            && e.prayer == Some(Notifiable::Isha)
            && e.kind == EventKind::MosqueReminder));
    }

    /// `mosque_reminder = false` removes every MosqueReminder.
    #[test]
    fn mosque_reminder_false_removes_every_reminder() {
        let window = testing::fixture_window();
        let mut cfg = testing::default_config();
        cfg.notifications.mosque_reminder = false;
        let now = testing::snapshot_now();
        assert!(
            !plan_events(&window, &cfg, now)
                .iter()
                .any(|e| e.kind == EventKind::MosqueReminder)
        );
    }

    /// §23 Phase 3 item 8: a fully disabled prayer (all four switches
    /// off) produces zero events — and the planner still returns a
    /// rollover, which is not per-prayer.
    #[test]
    fn a_disabled_prayer_produces_no_events() {
        let window = testing::fixture_window();
        let mut cfg = testing::default_config();
        let off = crate::config::PrayerSettings {
            before_minutes: 0,
            after_minutes: 0,
            adhan: false,
            notify: false,
        };
        cfg.prayers.maghrib = off;
        let now = testing::snapshot_now();
        let events = plan_events(&window, &cfg, now);
        assert!(
            events
                .iter()
                .all(|e| e.prayer != Some(Notifiable::Maghrib) || e.kind == EventKind::DayRollover)
        );
    }

    /// §23 Phase 3 item 8: the rollover fires at local midnight of the
    /// window's tomorrow — 2026-09-06T00:00:00+01:00, `prayer: None`,
    /// dated tomorrow.
    #[test]
    fn the_rollover_is_at_local_midnight() {
        let window = testing::fixture_window();
        let cfg = testing::default_config();
        let now = testing::snapshot_now();
        let rollover = plan_events(&window, &cfg, now)
            .into_iter()
            .find(|e| e.kind == EventKind::DayRollover)
            .unwrap();
        assert_eq!(rollover.prayer, None);
        assert_eq!(rollover.date, testing::TOMORROW);
        assert_eq!(rollover.fire_at.to_rfc3339(), "2026-09-06T00:00:00+01:00");
    }

    /// The plan is sorted ascending by `fire_at`.
    #[test]
    fn the_plan_is_sorted() {
        let window = testing::fixture_window();
        let cfg = testing::default_config();
        let events = plan_events(&window, &cfg, testing::snapshot_now());
        for pair in events.windows(2) {
            assert!(pair[0].fire_at <= pair[1].fire_at, "not sorted: {pair:?}");
        }
    }

    /// A zone whose DST jump skips midnight: Cuba springs forward at
    /// 00:00 on the second Sunday of March, so 2026-03-08 00:00 does not
    /// exist in Havana — the rollover lands on the first existing hour.
    #[test]
    fn a_midnight_skipped_by_dst_falls_forward() {
        let havana: Tz = "America/Havana".parse().unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        assert_eq!(
            havana.from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap()),
            chrono::LocalResult::None,
            "precondition: Havana midnight is skipped on 2026-03-08"
        );
        let midnight = midnight_in(havana, date).unwrap();
        assert_eq!(midnight.to_rfc3339(), "2026-03-08T01:00:00-04:00");
    }

    /// Berlin's transition (02:00 → 03:00) never touches midnight: the
    /// rollover is exactly 00:00, on both sides of the switch.
    #[test]
    fn berlin_midnight_is_never_touched_by_dst() {
        let berlin: Tz = "Europe/Berlin".parse().unwrap();
        let before = NaiveDate::from_ymd_opt(2026, 3, 28).unwrap();
        let after = NaiveDate::from_ymd_opt(2026, 3, 29).unwrap();
        assert_eq!(
            midnight_in(berlin, before).unwrap().to_rfc3339(),
            "2026-03-28T00:00:00+01:00"
        );
        assert_eq!(
            midnight_in(berlin, after).unwrap().to_rfc3339(),
            "2026-03-29T00:00:00+01:00"
        );
    }
}
