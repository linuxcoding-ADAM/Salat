//! `prayer_completions` — the A3 row the user's `[c]` writes: the
//! `Completed` state must survive a restart within the day, so it is a
//! database row, not an in-memory flag (§16.6, §17.1).
//!
//! Rows are cleared implicitly: the day-rollover replan stops reading
//! yesterday's date (§11.3), and the boot vacuum removes them after
//! 60 days.

use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;

use crate::domain::Notifiable;

use super::{CacheError, Store, prayer_from_key, prayer_key};

impl Store {
    /// Toggle a prayer's completion for a date, returning the **new**
    /// state (`true` = completed). An un-completion deletes the row —
    /// the honest record of "not marked".
    pub async fn toggle_completion(
        &self,
        date: NaiveDate,
        prayer: Notifiable,
        completed_at: DateTime<Tz>,
    ) -> Result<bool, CacheError> {
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO prayer_completions (date, prayer, completed_at) \
             VALUES (?1, ?2, ?3)",
        )
        .bind(date.to_string())
        .bind(prayer_key(prayer))
        .bind(completed_at.to_rfc3339())
        .execute(self.pool())
        .await?;
        if inserted.rows_affected() == 1 {
            return Ok(true);
        }
        sqlx::query("DELETE FROM prayer_completions WHERE date = ?1 AND prayer = ?2")
            .bind(date.to_string())
            .bind(prayer_key(prayer))
            .execute(self.pool())
            .await?;
        Ok(false)
    }

    /// The prayers completed on a date, in canonical order — the input
    /// to `state_of`'s `completions` argument (§10.1).
    ///
    /// A prayer spelling this version does not know is skipped with a
    /// log line, not an error: a newer Salat writing a new prayer must
    /// not make an older one fail to render today's rows.
    pub async fn completions_for(&self, date: NaiveDate) -> Result<Vec<Notifiable>, CacheError> {
        let keys: Vec<String> =
            sqlx::query_scalar("SELECT prayer FROM prayer_completions WHERE date = ?1")
                .bind(date.to_string())
                .fetch_all(self.pool())
                .await?;
        let mut prayers: Vec<Notifiable> = keys
            .iter()
            .filter_map(|key| match prayer_from_key(key) {
                Some(prayer) => Some(prayer),
                None => {
                    tracing::warn!(
                        target: "salat::cache",
                        "ignoring stored completion with unknown prayer {key:?}"
                    );
                    None
                }
            })
            .collect();
        prayers.sort();
        Ok(prayers)
    }
}
