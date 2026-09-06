//! Grace windows and the missed decision (§11.5).
//!
//! One judgement for every path that can find an event due: the live
//! 1 Hz tick, the startup catch-up, the post-jump catch-up. `decide`
//! is the single place the §11.4 rule lives — the ledger insert is the
//! dedup, and it precedes the side effect — so a restart, a replayed
//! tick and a clock jump all funnel through the same atomic check.

use chrono::{DateTime, Duration};
use chrono_tz::Tz;

use crate::cache::{CacheError, Store};
use crate::domain::{EventKind, ScheduledEvent};

/// §11.5: a pre-notify may still fire up to two minutes late.
pub const PRE_NOTIFY_GRACE: Duration = Duration::minutes(2);

/// §11.5: an Adhan may still play up to five minutes late.
pub const ADHAN_GRACE: Duration = Duration::minutes(5);

/// §11.5: a mosque reminder may still fire up to five minutes late.
pub const MOSQUE_REMINDER_GRACE: Duration = Duration::minutes(5);

/// How far past `fire_at` a fire may be and still count as on time
/// (§11.5's `late` flag). The live tick is 1 Hz, so an on-time fire is
/// at most a second late; anything older — a startup or post-jump
/// catch-up — reports `(delayed)`.
pub const DELAYED_AFTER: Duration = Duration::seconds(2);

/// The grace window of one kind, or `None` for the kinds without one:
/// rollovers are idempotent by nature and snoozes are one-shot (§11.7),
/// so neither can be "missed".
pub fn grace_for(kind: EventKind) -> Option<Duration> {
    match kind {
        EventKind::PreNotify => Some(PRE_NOTIFY_GRACE),
        EventKind::Adhan => Some(ADHAN_GRACE),
        EventKind::MosqueReminder => Some(MOSQUE_REMINDER_GRACE),
        EventKind::DayRollover | EventKind::AdhanSnooze | EventKind::ReminderSnooze => None,
    }
}

/// The outcome of judging one event against one instant (§23 Phase 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatchupDecision {
    /// The caller owes the side effect; `late` is §11.5's flag.
    Fire { late: bool },
    /// Nothing fires. `missed` marks the case where this call just
    /// discovered the event past its grace window — the caller owes a
    /// `Missed` event-log row and broadcast. A plain `Skip` means the
    /// ledger already held the event: it fired before, in this process
    /// or a previous one, and must never fire again.
    Skip { missed: bool },
    /// Not due yet — `fire_at > now`.
    Pending,
}

/// Whether a due event is old enough to report as `(delayed)`.
fn late_for(event: &ScheduledEvent, now: DateTime<Tz>) -> bool {
    now - event.fire_at > DELAYED_AFTER
}

/// Judge one event at one instant (§11.5).
///
/// The ledger is the sole idempotency mechanism: `try_mark_fired`'s
/// INSERT … ON CONFLICT is atomic, so whichever path asks first wins
/// the side effect and every other path — a replayed tick, a restarted
/// process, a second scheduler in observer mode that lost the lock —
/// sees the conflict and skips.
///
/// Past-grace events are still ledgered (with `late = 1`) so a later
/// tick cannot fire them, and reported as [`CatchupDecision::Skip`]
/// with `missed = true` when the row lands.
pub async fn decide(
    scope: &str,
    event: &ScheduledEvent,
    now: DateTime<Tz>,
    store: &Store,
) -> Result<CatchupDecision, CacheError> {
    if event.fire_at > now {
        return Ok(CatchupDecision::Pending);
    }

    // Kinds without a ledger row fire whenever they are due — a
    // re-fired rollover merely refreshes, and a snooze is one-shot.
    let Some(grace) = grace_for(event.kind) else {
        return Ok(CatchupDecision::Fire {
            late: late_for(event, now),
        });
    };

    // Past grace: too late to be useful. Ledger it (a `late` row, so a
    // replaying tick cannot fire it) and let the caller record the
    // miss. A conflict here means it already fired once — a plain skip.
    if now - event.fire_at > grace {
        let landed = store.try_mark_fired(scope, event, now, true).await?;
        return Ok(CatchupDecision::Skip { missed: landed });
    }

    // Within grace: the §11.5 catch-up fire — once, flagged late.
    let late = late_for(event, now);
    let landed = store.try_mark_fired(scope, event, now, late).await?;
    Ok(if landed {
        CatchupDecision::Fire { late }
    } else {
        CatchupDecision::Skip { missed: false }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::testing;
    use crate::domain::{Notifiable, plan_events};

    /// The fixture's Maghrib Adhan (19:21 today), judged from various
    /// distances — every branch of `decide`.
    #[tokio::test]
    async fn decide_judges_every_distance_correctly() {
        let store = Store::open_in_memory().await.expect("an in-memory store");
        let window = testing::fixture_window();
        let cfg = testing::default_config();
        let event = plan_events(&window, &cfg, testing::snapshot_now())
            .into_iter()
            .find(|event| {
                event.kind == EventKind::Adhan && event.prayer == Some(Notifiable::Maghrib)
            })
            .expect("the plan holds Maghrib's Adhan");

        // Before the instant: pending, and no ledger row.
        let before = event.fire_at - Duration::minutes(1);
        assert_eq!(
            decide("default", &event, before, &store)
                .await
                .expect("decide"),
            CatchupDecision::Pending
        );

        // Exactly on time: fires, not late.
        assert_eq!(
            decide("default", &event, event.fire_at, &store)
                .await
                .expect("decide"),
            CatchupDecision::Fire { late: false }
        );

        // A replayed judgement: the ledger holds it — plain skip.
        assert_eq!(
            decide("default", &event, event.fire_at, &store)
                .await
                .expect("decide"),
            CatchupDecision::Skip { missed: false }
        );

        // Four minutes late: inside the five-minute Adhan grace, fires
        // with the §11.5 late flag.
        let late = event.fire_at + Duration::minutes(4);
        let store2 = Store::open_in_memory().await.expect("a second store");
        assert_eq!(
            decide("default", &event, late, &store2)
                .await
                .expect("decide"),
            CatchupDecision::Fire { late: true }
        );

        // Six minutes late: past grace — ledgered and missed.
        let store3 = Store::open_in_memory().await.expect("a third store");
        assert_eq!(
            decide(
                "default",
                &event,
                event.fire_at + Duration::minutes(6),
                &store3
            )
            .await
            .expect("decide"),
            CatchupDecision::Skip { missed: true }
        );
        // …and once ledgered-as-missed it stays a plain skip.
        assert_eq!(
            decide(
                "default",
                &event,
                event.fire_at + Duration::minutes(7),
                &store3
            )
            .await
            .expect("decide"),
            CatchupDecision::Skip { missed: false }
        );
    }

    /// A different scope is a different ledger (§8.1): the same event
    /// fires once per `--config` scope, never once globally.
    #[tokio::test]
    async fn scopes_do_not_dedup_each_other() {
        let store = Store::open_in_memory().await.expect("an in-memory store");
        let window = testing::fixture_window();
        let cfg = testing::default_config();
        let event = plan_events(&window, &cfg, testing::snapshot_now())
            .into_iter()
            .find(|event| event.kind == EventKind::Adhan)
            .expect("the plan holds an Adhan");

        assert_eq!(
            decide("default", &event, event.fire_at, &store)
                .await
                .expect("decide"),
            CatchupDecision::Fire { late: false }
        );
        assert_eq!(
            decide("scoped-1234", &event, event.fire_at, &store)
                .await
                .expect("decide"),
            CatchupDecision::Fire { late: false }
        );
    }

    /// The grace table is §11.5's: 2 / 5 / 5 minutes, none for the
    /// idempotent and one-shot kinds.
    #[test]
    fn the_grace_table_matches_the_section() {
        assert_eq!(grace_for(EventKind::PreNotify), Some(PRE_NOTIFY_GRACE));
        assert_eq!(PRE_NOTIFY_GRACE, Duration::minutes(2));
        assert_eq!(grace_for(EventKind::Adhan), Some(ADHAN_GRACE));
        assert_eq!(ADHAN_GRACE, Duration::minutes(5));
        assert_eq!(
            grace_for(EventKind::MosqueReminder),
            Some(MOSQUE_REMINDER_GRACE)
        );
        assert_eq!(MOSQUE_REMINDER_GRACE, Duration::minutes(5));
        assert_eq!(grace_for(EventKind::DayRollover), None);
        assert_eq!(grace_for(EventKind::AdhanSnooze), None);
        assert_eq!(grace_for(EventKind::ReminderSnooze), None);
    }
}
