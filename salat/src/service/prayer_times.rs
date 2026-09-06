//! `PrayerTimeService` — the §7.6 window resolution: cache first,
//! network only for a month the cache cannot answer, zero HTTP when the
//! cache is warm.
//!
//! The trigger set (boot, local-midnight rollover, a config change that
//! alters the cache key) all lands on the same code path —
//! [`PrayerTimeService::refresh`] — because every trigger needs exactly
//! the same work: make the months around today complete, then serve
//! yesterday/today/tomorrow out of SQLite (§17.3: instant start, full
//! offline correctness for fetched dates).
//!
//! The one fatal case is the §17.3 honest error: a month that is
//! neither cached nor fetchable means the requested day cannot be
//! served, and that is `E301` — a guess is never substituted.

use std::sync::{Arc, RwLock};

use chrono::{Datelike, Days, NaiveDate};
use chrono_tz::Tz;

use crate::api::params::Params;
use crate::api::{AlAdhanClient, ApiError, Revalidated};
use crate::app::clock::Clock;
use crate::cache::{CacheError, Store, cache_key};
use crate::domain::PrayerWindow;
use crate::error::SalatError;

/// What the prayer-times service can fail with (§18.1: one typed enum
/// per service), mapped to the user-facing taxonomy by
/// [`ServiceError::to_salat_error`].
#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    /// The cache layer failed → `E601`.
    #[error(transparent)]
    Cache(#[from] CacheError),
    /// A request the service could not even attempt to satisfy from
    /// cache failed → the §17.3 honest error, `E301`. Carries the
    /// source `ApiError` for the log; the user sees "prayer times
    /// unavailable", which is exactly the truth.
    #[error("no cached month and the fetch failed: {source}")]
    Unavailable {
        /// The month that could not be served, as `(year, month)`.
        month: (i32, u32),
        /// Why the fetch failed — logged, not shown.
        source: ApiError,
    },
    /// An API failure the caller may still want in its raw form
    /// (e.g. a revalidation on the refresh path) → the `ApiError`
    /// mapping decides the code.
    #[error(transparent)]
    Api(#[from] ApiError),
}

/// The §7.6 refresh outcome: the window now serving, and whether the
/// revalidation reached the API. `revalidated == false` means the
/// window is stored data served while the last network attempt
/// failed — the caller surfaces that honestly (§17.3's `· cached`
/// marker, `OFFLINE (using cache)`) rather than guessing freshness.
#[derive(Debug)]
pub struct Refreshed {
    /// The window the refresh resolved to.
    pub window: Arc<PrayerWindow>,
    /// Whether the §7.5 revalidation could talk to the API at all
    /// (`Fresh` or `NotModified`). A failed attempt degrades to the
    /// cache without failing the refresh — prayer times are immutable
    /// — but it is still a fact the caller must not hide.
    pub revalidated: bool,
}

impl ServiceError {
    /// The §18.1 mapping: exactly one place decides presentation.
    pub fn to_salat_error(&self) -> SalatError {
        match self {
            ServiceError::Cache(error) => error.to_salat_error(),
            ServiceError::Unavailable { .. } => SalatError::PrayerTimesUnavailable,
            ServiceError::Api(error) => error.to_salat_error(),
        }
    }
}

/// The §7.6 window service. Cheap to share through an `Arc`; the memo
/// holds one window so the render loop's every-frame `window()` call
/// costs a lock, not a query.
pub struct PrayerTimeService {
    client: Arc<dyn AlAdhanClient>,
    store: Arc<Store>,
    clock: Arc<dyn Clock>,
    /// The current parameters — replaced whole by [`set_params`] on a
    /// config change (§7.6's third trigger).
    params: RwLock<Params>,
    /// `(date, window)` — the last served window. `None` until the
    /// first [`window`](Self::window) call, and after every
    /// [`set_params`].
    memo: RwLock<Option<(NaiveDate, Arc<PrayerWindow>)>>,
}

impl PrayerTimeService {
    /// Wire the service (§4.2: `app::Runtime` is the only caller).
    pub fn new(
        client: Arc<dyn AlAdhanClient>,
        store: Arc<Store>,
        params: Params,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            client,
            store,
            clock,
            params: RwLock::new(params),
            memo: RwLock::new(None),
        }
    }

    /// A config change (§7.6): the new parameters take effect on the
    /// next window. When the change alters the cache key the old rows
    /// simply stop being read (§17.3); when it does not (e.g. an Adhan
    /// volume), the cache still serves.
    pub fn set_params(&self, params: Params) {
        if let Ok(mut current) = self.params.write() {
            *current = params;
        }
        if let Ok(mut memo) = self.memo.write() {
            *memo = None;
        }
    }

    /// The window for a date (§7.6): ensure the months it spans are
    /// complete, then load yesterday/today/tomorrow from the cache.
    ///
    /// A warm cache performs **zero** HTTP calls — the property the
    /// Phase 4 tests assert against the mock client's counters.
    /// `yesterday` is `None` in the degraded fresh-cache case (§6.3):
    /// §7.6 deliberately does not fetch the previous month.
    pub async fn window(&self, today: NaiveDate) -> Result<Arc<PrayerWindow>, ServiceError> {
        if let Some((date, window)) = self.memo()
            && date == today
        {
            return Ok(window);
        }

        self.ensure_window(today).await?;

        let params = self.params();
        let key = cache_key(&params);
        // §7.6 loads all three days from the cache; yesterday may be
        // absent on a fresh cache (§6.3's degraded case) — §7.6
        // deliberately does not fetch the previous month.
        let yesterday = match today.pred_opt() {
            Some(date) => self.store.load_day(&key, date).await?,
            None => None,
        };
        let today_day = self.store.load_day(&key, today).await?;
        let tomorrow = match today.succ_opt() {
            Some(date) => self.store.load_day(&key, date).await?,
            None => None,
        };
        let (today_day, tomorrow_day) = match (today_day, tomorrow) {
            (Some(today_day), Some(tomorrow_day)) => (today_day, tomorrow_day),
            // ensure_window returned without these being loadable: a
            // month is neither cached nor fetchable — §17.3's honest
            // error.
            _ => {
                return Err(ServiceError::Unavailable {
                    month: (today.year(), today.month()),
                    source: ApiError::Malformed(
                        "the month was neither cached nor fetchable".into(),
                    ),
                });
            }
        };

        let window = Arc::new(PrayerWindow {
            yesterday,
            today: today_day,
            tomorrow: tomorrow_day,
        });
        if let Ok(mut memo) = self.memo.write() {
            *memo = Some((today, Arc::clone(&window)));
        }
        Ok(window)
    }

    /// The rollover/refresh trigger (§7.6): recompute "today" from the
    /// clock, revalidate the current month against the API (§7.5's
    /// explicit path — this is not the hot path, §17.3), and serve the
    /// window. A failed revalidation degrades to the cache rather than
    /// failing the refresh: prayer times are immutable, so the stored
    /// month is not made wrong by staying — but [`Refreshed`] says so,
    /// so the caller can report `· cached` honestly (§17.3).
    pub async fn refresh(&self) -> Result<Refreshed, ServiceError> {
        let today = self.clock.now_in(self.zone()?).date_naive();
        let params = self.params();
        let key = cache_key(&params);

        // The explicit §7.5 revalidation: send the stored ETag, take
        // 304 as "still valid", store anything fresh.
        let (year, month) = (today.year(), today.month());
        let stored_etag = self.store.month_etag(&key, year, month).await?;
        let etag = stored_etag.as_deref().unwrap_or("");
        let mut revalidated = true;
        match self
            .client
            .revalidate_calendar(year, month, &params, etag)
            .await
        {
            Ok(Revalidated::Fresh(days, etag)) => {
                let fetched_at = self.clock.now_in(self.zone()?);
                let mapped = days
                    .iter()
                    .map(crate::api::map::to_prayer_day)
                    .collect::<Result<Vec<_>, _>>()?;
                self.store
                    .upsert_month(&key, &mapped, Some(&etag), fetched_at)
                    .await?;
            }
            Ok(Revalidated::NotModified) => {}
            Err(error) => {
                tracing::warn!(
                    target: "salat::service",
                    "refresh revalidation failed, serving from cache: {error}"
                );
                revalidated = false;
            }
        }

        if let Ok(mut memo) = self.memo.write() {
            *memo = None;
        }
        let window = self.window(today).await?;
        Ok(Refreshed {
            window,
            revalidated,
        })
    }

    /// §7.6: for each month around `today` — the month of `today`, of
    /// `today + 1 day`, and (the prose's prefetch, so tomorrow's Fajr
    /// is always available offline near month end) the month of the
    /// next three days — if the cache holds a complete month for the
    /// current key, continue; else fetch, validate and store all days
    /// atomically.
    ///
    /// Only the months the window itself needs (today's and tomorrow's)
    /// are fatal to fetch; a prefetch month that cannot be fetched is
    /// logged and skipped — the window is servable without it.
    async fn ensure_window(&self, today: NaiveDate) -> Result<(), ServiceError> {
        let params = self.params();
        let key = cache_key(&params);
        let zone = self.zone()?;

        let mut months: Vec<(i32, u32)> = Vec::new();
        for offset in 0..=3u32 {
            let date = today
                .checked_add_days(Days::new(u64::from(offset)))
                .unwrap_or(today);
            let month = (date.year(), date.month());
            if !months.contains(&month) {
                months.push(month);
            }
        }
        // The window needs the months of today and today + 1 (§7.6);
        // anything else in the lookahead is the month-end prefetch,
        // desirable but never fatal.
        let need = [months_of(today), months_of(next_day(today))].concat();

        for &month in &months {
            if self.store.month_complete(&key, month.0, month.1).await? {
                continue; // the cache answers — zero HTTP (§7.6).
            }
            match self.client.calendar(month.0, month.1, &params).await {
                Ok(days) => {
                    let fetched_at = self.clock.now_in(zone);
                    let mapped = days
                        .iter()
                        .map(crate::api::map::to_prayer_day)
                        .collect::<Result<Vec<_>, _>>()?;
                    self.store
                        .upsert_month(&key, &mapped, None, fetched_at)
                        .await?;
                }
                Err(source) => {
                    if need.contains(&month) {
                        return Err(ServiceError::Unavailable { month, source });
                    }
                    tracing::warn!(
                        target: "salat::service",
                        "prefetch of {}-{:02} failed, continuing without it: {source}",
                        month.0,
                        month.1
                    );
                }
            }
        }
        Ok(())
    }

    fn params(&self) -> Params {
        self.params
            .read()
            .map(|params| params.clone())
            // Lock poisoning means a panic elsewhere mid-write; the
            // params are still a coherent value, so serve them.
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    fn memo(&self) -> Option<(NaiveDate, Arc<PrayerWindow>)> {
        self.memo.read().ok().and_then(|memo| memo.clone())
    }

    fn zone(&self) -> Result<Tz, ServiceError> {
        let name = self.params().timezone;
        name.parse().map_err(|_| {
            ServiceError::Api(ApiError::Malformed(format!(
                "configured timezone {name:?} is not an IANA zone"
            )))
        })
    }
}

fn next_day(date: NaiveDate) -> NaiveDate {
    date.succ_opt().unwrap_or(date)
}

/// The months a date window (`date`, `date + 1`) spans — the §7.6
/// "need" set, before the prefetch extension.
fn months_of(date: NaiveDate) -> Vec<(i32, u32)> {
    let mut months = vec![(date.year(), date.month())];
    let next = next_day(date);
    let month = (next.year(), next.month());
    if !months.contains(&month) {
        months.push(month);
    }
    months
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §7.6 need set: one month mid-month, two across a boundary.
    #[test]
    fn months_of_midmonth_and_boundary() {
        let mid = NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();
        assert_eq!(months_of(mid), vec![(2026, 9)]);
        let boundary = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        assert_eq!(months_of(boundary), vec![(2026, 9), (2026, 10)]);
        let year_end = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        assert_eq!(months_of(year_end), vec![(2026, 12), (2027, 1)]);
    }
}
