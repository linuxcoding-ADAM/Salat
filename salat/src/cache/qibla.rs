//! `qibla` — one direction per location (§17.1): the Qibla bearing
//! never changes, so it is fetched once per configuration and cached
//! forever, avoiding a request at every launch.

use chrono::DateTime;
use chrono_tz::Tz;

use super::{CacheError, Store};

impl Store {
    /// Store (or replace) the Qibla direction for a `cache_key`.
    pub async fn store_qibla(
        &self,
        cache_key: &str,
        direction: f64,
        fetched_at: DateTime<Tz>,
    ) -> Result<(), CacheError> {
        sqlx::query(
            "INSERT OR REPLACE INTO qibla (cache_key, direction, fetched_at) \
             VALUES (?1, ?2, ?3)",
        )
        .bind(cache_key)
        .bind(direction)
        .bind(fetched_at.to_rfc3339())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// The cached Qibla direction for a `cache_key`, in degrees from
    /// true North — `None` when this location has never been fetched.
    pub async fn load_qibla(&self, cache_key: &str) -> Result<Option<f64>, CacheError> {
        let direction: Option<f64> =
            sqlx::query_scalar("SELECT direction FROM qibla WHERE cache_key = ?1")
                .bind(cache_key)
                .fetch_optional(self.pool())
                .await?;
        Ok(direction)
    }
}
