//! `prayer_days` — the offline cache proper (§17.2, §17.3): `cache_key`,
//! `upsert_month`, `load_day`, month completeness, and the stored ETag.
//!
//! A row is immutable and never expires (§17.3): AlAdhan's answer for
//! `(location, calculation, date)` is a deterministic astronomical
//! computation, so there is no TTL, no staleness heuristic and no
//! revalidation on the hot path. Invalidation happens by key change —
//! old rows simply stop being read — and by [`Store::vacuum`]
//! (§17.3, in `mod.rs`).
//!
//! The storage format is owned here, in private serde structs with
//! RFC 3339 strings — the domain stays free of serde derives (§6), and
//! the on-disk shape stays exactly what §17.2 documents: one `timings`
//! JSON object of eight instants, one `hijri` JSON object, one `meta`
//! JSON object.

use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use crate::api::params::Params;
use crate::domain::{DayMeta, HijriDate, PrayerDay};

use super::{CacheError, Store};

impl Store {
    /// Store a whole fetched month atomically (§7.6): one transaction,
    /// every day of the response, all under one `cache_key` and one
    /// `etag`. Re-storing a month already present replaces it — rows are
    /// immutable in the §17.3 sense (nothing ever *changes* a day's
    /// times in place), but a refetch may carry a fresher `fetched_at`
    /// or `etag`, and the replacement writes the same instants.
    pub async fn upsert_month(
        &self,
        cache_key: &str,
        days: &[PrayerDay],
        etag: Option<&str>,
        fetched_at: DateTime<Tz>,
    ) -> Result<(), CacheError> {
        let mut tx = self.pool().begin().await?;
        for day in days {
            let timings = StoredTimings::from_day(day).to_json()?;
            let hijri = StoredHijri::from_day(day).to_json()?;
            let meta = StoredMeta::from_day(day).to_json()?;
            sqlx::query(
                "INSERT OR REPLACE INTO prayer_days \
                 (cache_key, date, tz, timings, hijri, meta, fetched_at, etag) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .bind(cache_key)
            .bind(day.date.to_string())
            .bind(day.tz.name())
            .bind(timings)
            .bind(hijri)
            .bind(meta)
            .bind(fetched_at.to_rfc3339())
            .bind(etag)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// One cached day, or `None` when this key has no row for the date
    /// (§17.3: the cache cannot answer for a date never fetched — that
    /// is the caller's `E301`, decided above this layer).
    pub async fn load_day(
        &self,
        cache_key: &str,
        date: NaiveDate,
    ) -> Result<Option<PrayerDay>, CacheError> {
        let row = sqlx::query_as::<_, StoredRow>(
            "SELECT date, tz, timings, hijri, meta FROM prayer_days \
             WHERE cache_key = ?1 AND date = ?2",
        )
        .bind(cache_key)
        .bind(date.to_string())
        .fetch_optional(self.pool())
        .await?;
        row.map(StoredRow::into_day).transpose()
    }

    /// Does this key hold every day of the given month? — the
    /// [`crate::service::PrayerTimeService`] completeness check (§7.6):
    /// a complete month means zero HTTP.
    pub async fn month_complete(
        &self,
        cache_key: &str,
        year: i32,
        month: u32,
    ) -> Result<bool, CacheError> {
        let first = first_of_month(year, month);
        let next = next_first_of_month(year, month);
        let expected: i64 = days_in_month(year, month);
        let stored: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM prayer_days \
             WHERE cache_key = ?1 AND date >= ?2 AND date < ?3",
        )
        .bind(cache_key)
        .bind(first.to_string())
        .bind(next.to_string())
        .fetch_one(self.pool())
        .await?;
        Ok(stored == expected)
    }

    /// The stored ETag for a month under this key, if any — the input to
    /// an explicit revalidation (§7.5). Any row of the month answers,
    /// because one response filled them all.
    pub async fn month_etag(
        &self,
        cache_key: &str,
        year: i32,
        month: u32,
    ) -> Result<Option<String>, CacheError> {
        let first = first_of_month(year, month);
        let next = next_first_of_month(year, month);
        let etag: Option<Option<String>> = sqlx::query_scalar(
            "SELECT etag FROM prayer_days \
             WHERE cache_key = ?1 AND date >= ?2 AND date < ?3 AND etag IS NOT NULL \
             LIMIT 1",
        )
        .bind(cache_key)
        .bind(first.to_string())
        .bind(next.to_string())
        .fetch_optional(self.pool())
        .await?;
        Ok(etag.flatten())
    }
}

/// The §17.2 cache key: the readable canonical string itself, built with
/// one `format!`, in exactly the documented field order — latitude,
/// longitude, method, school, latitude-adjustment rule, midnight mode,
/// timezone, tune vector — coordinates fixed at four decimal places so
/// `36.19` and `36.1900` cannot produce two rows.
///
/// Not a hash; nothing in this layer hashes anything (§17.2). Any change
/// to a field's order, name or formatting is a cache-key change and
/// therefore a migration.
pub fn cache_key(params: &Params) -> String {
    let t = &params.tune;
    let tune = [
        t.imsak, t.fajr, t.sunrise, t.dhuhr, t.asr, t.maghrib, t.sunset, t.isha, t.midnight,
    ]
    .iter()
    .map(|minutes| minutes.to_string())
    .collect::<Vec<_>>()
    .join("-");
    format!(
        "{:.4},{:.4},m{},s{},l{},mm{},tz={},tune={}",
        params.latitude,
        params.longitude,
        params.method,
        params.school,
        params.latitude_adjustment_method,
        params.midnight_mode,
        params.timezone,
        tune
    )
}

/// The first of a month, or `NaiveDate::MIN` for an impossible one —
/// `year`/`month` always come from chrono's own `Datelike` on a real
/// date, so the fallback is unreachable bookkeeping, not a path that
/// could silently corrupt a window.
fn first_of_month(year: i32, month: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(NaiveDate::MIN)
}

fn next_first_of_month(year: i32, month: u32) -> NaiveDate {
    if month == 12 {
        first_of_month(year + 1, 1)
    } else {
        first_of_month(year, month + 1)
    }
}

fn days_in_month(year: i32, month: u32) -> i64 {
    let first = first_of_month(year, month);
    let next = next_first_of_month(year, month);
    (next - first).num_days()
}

// ---- storage shapes (§17.2) ------------------------------------------------
//
// Private on purpose: the JSON column shapes are this file's business,
// and changing one is a migration decision, not a public API decision.

/// One row as it sits in `prayer_days`.
#[derive(sqlx::FromRow)]
struct StoredRow {
    date: String,
    tz: String,
    timings: String,
    hijri: String,
    meta: String,
}

/// `timings` — the six prayers plus midnight and last third, RFC 3339
/// with each instant's real UTC offset.
#[derive(Serialize, Deserialize)]
struct StoredTimings {
    fajr: String,
    sunrise: String,
    dhuhr: String,
    asr: String,
    maghrib: String,
    isha: String,
    midnight: String,
    last_third: String,
}

/// `hijri` — exactly the §17.2 fields.
#[derive(Serialize, Deserialize)]
struct StoredHijri {
    day: u32,
    month_number: u32,
    month_en: String,
    month_ar: String,
    year: i32,
}

/// `meta` — exactly the §17.2 fields (the IANA name appears both here
/// and in the `tz` column; the column is the authority on load).
#[derive(Serialize, Deserialize)]
struct StoredMeta {
    method_id: u32,
    method_name: String,
    school: u8,
    lat: f64,
    lng: f64,
    tz: String,
}

trait StoredJson {
    fn to_json(&self) -> Result<String, CacheError>;
}

impl<T: Serialize> StoredJson for T {
    fn to_json(&self) -> Result<String, CacheError> {
        serde_json::to_string(self)
            .map_err(|e| CacheError::Serde(format!("could not encode a stored row: {e}")))
    }
}

fn parse_rfc3339(
    field: &str,
    raw: &str,
) -> Result<chrono::DateTime<chrono::FixedOffset>, CacheError> {
    chrono::DateTime::parse_from_rfc3339(raw).map_err(|_| {
        CacheError::Serde(format!(
            "stored timing {field} is not RFC 3339 (found \"{raw}\")"
        ))
    })
}

impl StoredTimings {
    fn from_day(day: &PrayerDay) -> Self {
        // Indexed by `Prayer` order (§6.2): Fajr, Sunrise, Dhuhr, Asr,
        // Maghrib, Isha.
        Self {
            fajr: day.times[0].to_rfc3339(),
            sunrise: day.times[1].to_rfc3339(),
            dhuhr: day.times[2].to_rfc3339(),
            asr: day.times[3].to_rfc3339(),
            maghrib: day.times[4].to_rfc3339(),
            isha: day.times[5].to_rfc3339(),
            midnight: day.midnight.to_rfc3339(),
            last_third: day.last_third.to_rfc3339(),
        }
    }

    fn parse(&self, tz: Tz) -> Result<[DateTime<Tz>; 6], CacheError> {
        let into = |field: &str, raw: &str| -> Result<DateTime<Tz>, CacheError> {
            Ok(parse_rfc3339(field, raw)?.with_timezone(&tz))
        };
        Ok([
            into("fajr", &self.fajr)?,
            into("sunrise", &self.sunrise)?,
            into("dhuhr", &self.dhuhr)?,
            into("asr", &self.asr)?,
            into("maghrib", &self.maghrib)?,
            into("isha", &self.isha)?,
        ])
    }
}

impl StoredHijri {
    fn from_day(day: &PrayerDay) -> Self {
        Self {
            day: day.hijri.day,
            month_number: day.hijri.month_number,
            month_en: day.hijri.month_en.clone(),
            month_ar: day.hijri.month_ar.clone(),
            year: day.hijri.year,
        }
    }
}

impl StoredMeta {
    fn from_day(day: &PrayerDay) -> Self {
        Self {
            method_id: day.meta.method_id,
            method_name: day.meta.method_name.clone(),
            school: day.meta.school,
            lat: day.meta.lat,
            lng: day.meta.lng,
            tz: day.meta.tz_name.clone(),
        }
    }
}

impl StoredRow {
    fn into_day(self) -> Result<PrayerDay, CacheError> {
        let date = NaiveDate::parse_from_str(&self.date, "%Y-%m-%d").map_err(|_| {
            CacheError::Serde(format!(
                "stored date is not YYYY-MM-DD (found \"{}\")",
                self.date
            ))
        })?;
        let tz: Tz = self.tz.parse().map_err(|_| {
            CacheError::Serde(format!(
                "stored timezone is not an IANA zone (found \"{}\")",
                self.tz
            ))
        })?;
        let timings: StoredTimings = serde_json::from_str(&self.timings)
            .map_err(|e| CacheError::Serde(format!("stored timings JSON is unreadable: {e}")))?;
        let hijri: StoredHijri = serde_json::from_str(&self.hijri)
            .map_err(|e| CacheError::Serde(format!("stored hijri JSON is unreadable: {e}")))?;
        let meta: StoredMeta = serde_json::from_str(&self.meta)
            .map_err(|e| CacheError::Serde(format!("stored meta JSON is unreadable: {e}")))?;
        Ok(PrayerDay {
            date,
            tz,
            times: timings.parse(tz)?,
            midnight: parse_rfc3339("midnight", &timings.midnight)?.with_timezone(&tz),
            last_third: parse_rfc3339("last_third", &timings.last_third)?.with_timezone(&tz),
            hijri: HijriDate {
                day: hijri.day,
                month_number: hijri.month_number,
                month_en: hijri.month_en,
                month_ar: hijri.month_ar,
                year: hijri.year,
            },
            meta: DayMeta {
                method_id: meta.method_id,
                method_name: meta.method_name,
                school: meta.school,
                lat: meta.lat,
                lng: meta.lng,
                tz_name: meta.tz,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// The §17.2 specimen, byte for byte: the default config's canonical
    /// string. A `format!` change here is a cache-key change and must be
    /// an explicit decision, so it is pinned.
    #[test]
    fn the_default_config_yields_the_specimen_key() {
        let params = Params::from(&Config::default());
        assert_eq!(
            cache_key(&params),
            "36.1900,5.4100,m12,s0,l3,mm0,tz=Africa/Algiers,tune=0-0-0-0-0-0-0-0-0"
        );
    }

    /// Four decimal places make `36.19` and `36.1900` one location, while
    /// a real difference (`5.41` → `5.42`) is a different row — the exact
    /// reason the doc fixes the precision (§17.2).
    #[test]
    fn coordinate_precision_collapses_and_separates() {
        let mut params = Params::from(&Config::default());
        params.latitude = 36.1900;
        assert_eq!(
            cache_key(&params),
            cache_key(&Params::from(&Config::default()))
        );
        params.longitude = 5.42;
        assert_ne!(
            cache_key(&params),
            cache_key(&Params::from(&Config::default()))
        );
    }

    /// Every calculation input appears in the key, so any config change
    /// reads a different cache (§17.3 invalidation).
    #[test]
    fn every_calculation_input_participates() {
        let base = cache_key(&Params::from(&Config::default()));
        let mut params = Params::from(&Config::default());
        params.method = 3;
        assert_ne!(cache_key(&params), base);
        let mut params = Params::from(&Config::default());
        params.school = 1;
        assert_ne!(cache_key(&params), base);
        let mut params = Params::from(&Config::default());
        params.tune.fajr = 1;
        assert_ne!(cache_key(&params), base);
        // …and tune is dash-separated here, comma-separated on the wire.
        let mut params = Params::from(&Config::default());
        params.tune = crate::config::Tune {
            imsak: 1,
            fajr: 2,
            sunrise: 3,
            dhuhr: 4,
            asr: 5,
            maghrib: 6,
            sunset: 7,
            isha: 8,
            midnight: 9,
        };
        assert!(cache_key(&params).ends_with("tune=1-2-3-4-5-6-7-8-9"));
    }

    /// Month arithmetic helpers agree with chrono on leap Februaries.
    #[test]
    fn month_bounds_handle_february() {
        assert_eq!(days_in_month(2028, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(next_first_of_month(2026, 12), first_of_month(2027, 1));
    }
}
