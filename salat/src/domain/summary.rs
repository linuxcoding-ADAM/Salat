//! `DaySummary` — the Status screen's day counters (§6.5).
//!
//! The four §6.5 values the *domain* can compute from a window, plus
//! the two §16.4 item-6 counters (notifications sent, Adhan plays) that
//! only the event log knows. The domain fills those with zero; the
//! Phase-12 `Collector` (§15.8) overwrites them from today's rows —
//! the counting itself belongs to the log that recorded the events,
//! not to the pure function that summarises a window.

use chrono::DateTime;
use chrono_tz::Tz;

use crate::config::Config;

use super::day::PrayerWindow;
use super::prayer::Notifiable;
use super::schedule::{EventKind, plan_events};

/// Today's counters (§6.5, §16.4 item 6):
/// `total/passed/remaining/next_notify`, plus the event-log counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaySummary {
    /// Notifiable prayers today — five, unless the API model changes.
    pub total: u32,
    /// Those whose instant is at or before `now`.
    pub passed: u32,
    pub remaining: u32,
    /// Desktop notifications that actually went out today (§16.4 item
    /// 6's "Notifications sent"). Zero from the domain — the
    /// event-log-owning `Collector` fills the real count.
    pub notifications_sent: u32,
    /// Adhan playbacks that fired today ("Adhan plays"). Same deal.
    pub adhan_plays: u32,
    /// When the next user-facing firing (pre-notify, Adhan or mosque
    /// reminder — anything except the rollover, which the user never
    /// sees) is due, from the same event set the planner produces.
    /// `None` when every notification and Adhan is disabled or the
    /// window is exhausted.
    pub next_notify: Option<DateTime<Tz>>,
}

/// Summarise a window's *today* at one instant (§6.5). `next_notify`
/// may point into tomorrow (after Isha, the next firing is tomorrow's
/// Fajr pre-notify) — the window exists precisely so that answer is
/// available.
pub fn day_summary(window: &PrayerWindow, cfg: &Config, now: DateTime<Tz>) -> DaySummary {
    let total = Notifiable::ALL.len() as u32;
    let passed = Notifiable::ALL
        .iter()
        .filter(|&&prayer| window.today.time(prayer.as_prayer()) <= now)
        .count() as u32;
    let next_notify = plan_events(window, cfg, now)
        .into_iter()
        .find(|event| event.kind != EventKind::DayRollover)
        .map(|event| event.fire_at);
    DaySummary {
        total,
        passed,
        remaining: total - passed,
        notifications_sent: 0,
        adhan_plays: 0,
        next_notify,
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing;
    use super::*;

    /// At the snapshot (17:38:23): Fajr, Dhuhr and Asr have passed,
    /// Maghrib and Isha remain, and the next firing is Maghrib's
    /// pre-notify at 19:16.
    #[test]
    fn the_snapshot_summary() {
        let window = testing::fixture_window();
        let summary = day_summary(&window, &testing::default_config(), testing::snapshot_now());
        assert_eq!(summary.total, 5);
        assert_eq!(summary.passed, 3);
        assert_eq!(summary.remaining, 2);
        assert_eq!(
            summary.next_notify,
            Some(testing::at(testing::TODAY, 19, 16, 0))
        );
    }

    /// Late at night, `next_notify` reaches into tomorrow: Fajr's
    /// pre-notify at 04:55.
    #[test]
    fn after_isha_the_next_notify_is_tomorrows_fajr() {
        let window = testing::fixture_window();
        let now = testing::at(testing::TODAY, 23, 30, 0);
        let summary = day_summary(&window, &testing::default_config(), now);
        assert_eq!(summary.passed, 5);
        assert_eq!(summary.remaining, 0);
        assert_eq!(
            summary.next_notify,
            Some(testing::at(testing::TOMORROW, 4, 55, 0))
        );
    }

    /// Just after midnight: nothing has passed yet today, and the
    /// morning's pre-notify is still hours out.
    #[test]
    fn just_after_midnight_nothing_has_passed() {
        let window = testing::fixture_window();
        let now = testing::at(testing::TODAY, 0, 30, 0);
        let summary = day_summary(&window, &testing::default_config(), now);
        assert_eq!(
            (summary.total, summary.passed, summary.remaining),
            (5, 0, 5)
        );
        assert_eq!(
            summary.next_notify,
            Some(testing::at(testing::TODAY, 4, 54, 0))
        );
    }

    /// With every notification and Adhan off there is nothing to notify
    /// — `next_notify` is `None`, but the day counters still count.
    #[test]
    fn a_silent_config_has_no_next_notify() {
        let window = testing::fixture_window();
        let mut cfg = testing::default_config();
        cfg.notifications.enabled = false;
        cfg.adhan.enabled = false;
        let summary = day_summary(&window, &cfg, testing::snapshot_now());
        assert_eq!(summary.next_notify, None);
        assert_eq!(summary.passed, 3);
    }

    /// `next_notify` includes Adhan firings — with notifications off but
    /// Adhan on, the next user-facing firing is the Adhan itself.
    #[test]
    fn next_notify_counts_adhan_firings() {
        let window = testing::fixture_window();
        let mut cfg = testing::default_config();
        cfg.notifications.enabled = false;
        let summary = day_summary(&window, &cfg, testing::snapshot_now());
        assert_eq!(
            summary.next_notify,
            Some(testing::at(testing::TODAY, 19, 21, 0))
        );
    }
}
