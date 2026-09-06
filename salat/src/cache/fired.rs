//! `fired_events` — the §11.4 idempotency ledger: the only way a
//! restart knows what already fired.
//!
//! The contract is INSERT-then-side-effect: the row is written *before*
//! the notification or Adhan plays, and a UNIQUE conflict means "someone
//! already fired this" — skip the side effect. A crash between the two
//! therefore loses one event rather than ever duplicating an audible
//! alarm, which is the correct trade (§11.4).

use chrono::DateTime;
use chrono_tz::Tz;

use crate::domain::ScheduledEvent;

use super::{CacheError, Store, kind_key, prayer_key};

impl Store {
    /// Try to mark `event` as fired under `scope` (§8.1: `"default"` or
    /// the `--config` scope). `Ok(true)` means the insert landed — this
    /// process owes the side effect. `Ok(false)` means the ledger
    /// already holds it — skip.
    ///
    /// `late` is §11.5's catch-up flag: the row records whether the
    /// event fired inside its grace window after a sleep or restart.
    pub async fn try_mark_fired(
        &self,
        scope: &str,
        event: &ScheduledEvent,
        fired_at: DateTime<Tz>,
        late: bool,
    ) -> Result<bool, CacheError> {
        // DayRollover is not ledgered (see `kind_key`): report "not
        // already fired" unconditionally, so the runner refreshes the
        // window on every boot — idempotent by nature.
        let Some(kind) = kind_key(event.kind) else {
            return Ok(true);
        };
        let prayer = event
            .prayer
            .map(prayer_key)
            // The schema's NOT NULL placeholder for a prayer-less event;
            // none of the three ledgered kinds is prayer-less today, but the
            // column must still hold something.
            .unwrap_or("");
        let result = sqlx::query(
            "INSERT INTO fired_events (scope, date, prayer, kind, fired_at, late) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT (scope, date, prayer, kind) DO NOTHING",
        )
        .bind(scope)
        .bind(event.date.to_string())
        .bind(prayer)
        .bind(kind)
        .bind(fired_at.to_rfc3339())
        .bind(i64::from(late))
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected() == 1)
    }
}
