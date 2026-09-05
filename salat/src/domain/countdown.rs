//! Countdown and gauge progress (§6.5).
//!
//! Both are pure arithmetic over instants. The countdown saturates at
//! zero — a passed target renders `00:00:00`, never a negative or a
//! panic — and its hours field is unbounded, because `26:03:11` is the
//! honest answer when the app is left open overnight (§6.5).

use chrono::DateTime;
use chrono_tz::Tz;

/// A rendered countdown: `HH:MM:SS` with hours unbounded (§6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Countdown {
    /// Unbounded — `26` is as valid as `01`.
    pub hours: i64,
    /// `0..=59`.
    pub minutes: i64,
    /// `0..=59`.
    pub seconds: i64,
    /// The whole span in seconds; the other three fields are derived
    /// from it, so they can never disagree.
    pub total_secs: i64,
}

impl Countdown {
    /// `00:00:00` — the value for a target at or before `now`.
    pub const ZERO: Countdown = Countdown {
        hours: 0,
        minutes: 0,
        seconds: 0,
        total_secs: 0,
    };

    /// The time from `now` until `target` (§6.5's `countdown(target,
    /// now)`). Saturates: a target in the past or exactly reached is
    /// zero, never negative.
    pub fn between(target: DateTime<Tz>, now: DateTime<Tz>) -> Self {
        let total_secs = (target - now).num_seconds().max(0);
        Self {
            hours: total_secs / 3600,
            minutes: (total_secs % 3600) / 60,
            seconds: total_secs % 60,
            total_secs,
        }
    }

    /// `HH:MM:SS` — two-digit fields, hours as wide as it must be
    /// (`26:03:11` is valid, §6.5).
    pub fn format(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hours, self.minutes, self.seconds)
    }
}

/// The gauge's fill fraction: how far `now` is from `prev` toward `next`
/// (§6.5) — the design's 62% mid-afternoon fill, clamped to `0.0..=1.0`.
///
/// Degenerate intervals are `0.0`, not a panic or `NaN`: a zero-length
/// (or inverted) prayer interval renders an empty gauge, which is the
/// honest answer for the odd polar-latitude day.
pub fn progress(prev: DateTime<Tz>, next: DateTime<Tz>, now: DateTime<Tz>) -> f64 {
    let span_ms = (next - prev).num_milliseconds();
    if span_ms <= 0 {
        return 0.0;
    }
    let elapsed_ms = (now - prev).num_milliseconds();
    (elapsed_ms as f64 / span_ms as f64).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::super::testing;
    use super::*;
    use chrono::Duration;

    /// §20.2: `19:21 − 17:38:23` → `01:42:37`.
    #[test]
    fn the_specimen_countdown_is_one_hour_forty_two() {
        let target = testing::at(testing::TODAY, 19, 21, 0);
        let now = testing::at(testing::TODAY, 17, 38, 23);
        let countdown = Countdown::between(target, now);
        assert_eq!(countdown.format(), "01:42:37");
        assert_eq!(countdown.total_secs, 6157);
        assert_eq!(
            (countdown.hours, countdown.minutes, countdown.seconds),
            (1, 42, 37)
        );
    }

    /// §20.2: one second before → `00:00:01`; at the target → `00:00:00`.
    #[test]
    fn the_last_second_counts_down_to_one_then_zero() {
        let target = testing::at(testing::TODAY, 19, 21, 0);
        assert_eq!(
            Countdown::between(target, target - Duration::seconds(1)).format(),
            "00:00:01"
        );
        let zero = Countdown::between(target, target);
        assert_eq!(zero.format(), "00:00:00");
        assert_eq!(zero, Countdown::ZERO);
    }

    /// §20.2: a passed target is zero — saturating, never negative,
    /// never a panic.
    #[test]
    fn a_passed_target_saturates_to_zero() {
        let target = testing::at(testing::TODAY, 19, 21, 0);
        let now = testing::at(testing::TODAY, 20, 0, 0);
        assert_eq!(Countdown::between(target, now), Countdown::ZERO);
    }

    /// §20.2: crossing midnight is plain arithmetic — `06:15:00` from
    /// 18:06 to 00:21 the next civil day.
    #[test]
    fn a_countdown_crossing_midnight_is_six_hours_fifteen() {
        let now = testing::at(testing::TODAY, 18, 6, 0);
        let target = testing::at(testing::TOMORROW, 0, 21, 0);
        assert_eq!(Countdown::between(target, now).format(), "06:15:00");
    }

    /// §6.5: hours are unbounded — `26:03:11` renders as-is.
    #[test]
    fn hours_are_unbounded() {
        let now = testing::at(testing::TODAY, 0, 0, 0);
        let target = now + Duration::seconds(26 * 3600 + 3 * 60 + 11);
        assert_eq!(Countdown::between(target, now).format(), "26:03:11");
    }

    /// The four fields are one value in three shapes.
    #[test]
    fn the_fields_agree_with_total_secs() {
        let now = testing::at(testing::TODAY, 12, 0, 0);
        let target = now + Duration::seconds(9373); // 02:35:13
        let countdown = Countdown::between(target, now);
        assert_eq!(
            countdown.total_secs,
            countdown.hours * 3600 + countdown.minutes * 60 + countdown.seconds
        );
    }

    /// §20.3's specimen: Asr 16:23 → Maghrib 19:21 with `now` at
    /// 17:38:23 is 4523/10680 = 42.35%.
    #[test]
    fn the_specimen_progress_is_forty_two_point_three_five() {
        let prev = testing::at(testing::TODAY, 16, 23, 0);
        let next = testing::at(testing::TODAY, 19, 21, 0);
        let now = testing::at(testing::TODAY, 17, 38, 23);
        let fill = progress(prev, next, now);
        assert!((fill - 4523.0 / 10680.0).abs() < 1e-9, "got {fill}");
    }

    /// §6.5: the result is clamped to `0.0..=1.0` on both sides.
    #[test]
    fn progress_clamps_outside_the_interval() {
        let prev = testing::at(testing::TODAY, 16, 23, 0);
        let next = testing::at(testing::TODAY, 19, 21, 0);
        assert_eq!(progress(prev, next, prev - Duration::hours(1)), 0.0);
        assert_eq!(progress(prev, next, prev), 0.0);
        assert_eq!(progress(prev, next, next), 1.0);
        assert_eq!(progress(prev, next, next + Duration::hours(1)), 1.0);
    }

    /// A degenerate (zero-length or inverted) interval is `0.0` — no
    /// `NaN`, no panic.
    #[test]
    fn degenerate_progress_is_zero_not_nan() {
        let t = testing::at(testing::TODAY, 19, 21, 0);
        assert_eq!(progress(t, t, t), 0.0);
        assert_eq!(progress(t, t - Duration::minutes(1), t), 0.0);
    }
}
