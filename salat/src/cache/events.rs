//! `event_log` — the append-only record the Status screen's
//! `RECENT EVENTS · TODAY` panel renders (§6.7, §17.1).
//!
//! One row per thing worth telling the user happened: notifications,
//! Adhan playbacks, state changes, errors, system events. Written at
//! fire time, read once per Status render, pruned only by the boot
//! vacuum — never edited, never restructured after the fact.

use chrono::DateTime;
use chrono_tz::Tz;

use super::{CacheError, Store};

/// The §17.2 categories, in the documented order — the only strings the
/// `category` column may hold.
pub const CATEGORIES: [&str; 5] = ["notification", "adhan", "state", "error", "system"];

/// One row of the event log, as the Status screen renders it.
#[derive(Debug, Clone, PartialEq)]
pub struct LoggedEvent {
    /// When it happened.
    pub at: DateTime<Tz>,
    /// One of [`CATEGORIES`].
    pub category: String,
    /// The one-line summary, e.g. `Pre-Adhan notification`.
    pub label: String,
    /// Optional second line, e.g. `MAGHRIB · 5 min window`.
    pub detail: Option<String>,
    /// The prayer involved, lowercase (`fajr` … `isha`), when one is.
    pub prayer: Option<String>,
}

impl Store {
    /// Append one event (§17.2). `category` must be one of
    /// [`CATEGORIES`] — an unknown category is a programming error on
    /// the caller's side and is refused rather than silently stored.
    pub async fn append_event(
        &self,
        at: DateTime<Tz>,
        category: &str,
        label: &str,
        detail: Option<&str>,
        prayer: Option<&str>,
    ) -> Result<(), CacheError> {
        if !CATEGORIES.contains(&category) {
            return Err(CacheError::Serde(format!(
                "unknown event_log category {category:?} (expected one of {CATEGORIES:?})"
            )));
        }
        sqlx::query(
            "INSERT INTO event_log (at, category, label, detail, prayer) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(at.to_rfc3339())
        .bind(category)
        .bind(label)
        .bind(detail)
        .bind(prayer)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Today's events in the location's zone, newest first (§6.7).
    ///
    /// "Today" is decided in the caller's zone — the location's, never
    /// the host's (§20.2) — so the filter runs on the parsed instants in
    /// Rust; SQLite has no chrono-tz. Ties on `at` break by insertion
    /// order, newest first.
    pub async fn events_today(
        &self,
        tz: Tz,
        now: DateTime<Tz>,
    ) -> Result<Vec<LoggedEvent>, CacheError> {
        let rows = sqlx::query_as::<_, (String, String, String, Option<String>, Option<String>)>(
            "SELECT at, category, label, detail, prayer FROM event_log \
             ORDER BY at DESC, id DESC",
        )
        .fetch_all(self.pool())
        .await?;

        let today = now.date_naive();
        let mut events = Vec::new();
        for (at, category, label, detail, prayer) in rows {
            let at = DateTime::parse_from_rfc3339(&at)
                .map_err(|_| {
                    CacheError::Serde(format!(
                        "stored event_log time is not RFC 3339 (found \"{at}\")"
                    ))
                })?
                .with_timezone(&tz);
            if at.date_naive() != today {
                continue;
            }
            events.push(LoggedEvent {
                at,
                category,
                label,
                detail,
                prayer,
            });
        }
        Ok(events)
    }
}
