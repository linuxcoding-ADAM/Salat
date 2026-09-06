//! `cache::Store` — the SQLite layer (§17): pool, PRAGMAs, embedded
//! migrations, corruption recovery.
//!
//! Five tables, no joins across them, no ORM layer, no repository
//! abstraction over `sqlx` (§17.2). The typed methods live next to the
//! table they serve, in this module's children; this file owns the
//! connection and its lifecycle only.
//!
//! Nothing in the cache layer reads a clock (§4.2: `Clock` is the one
//! source of time). Every method that needs "now" takes it as a
//! parameter, which is what lets the whole layer be tested against
//! pinned instants.

pub mod completions;
pub mod events;
pub mod fired;
pub mod prayer_days;
pub mod qibla;

pub use events::LoggedEvent;
pub use prayer_days::cache_key;

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Days};
use chrono_tz::Tz;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};

use crate::domain::{EventKind, Notifiable};
use crate::error::SalatError;

/// §17.4: one pool, two connections — the runner writes, the UI reads.
const MAX_CONNECTIONS: u32 = 2;

/// §17.4: how long a blocked writer waits before giving up.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// §17.4 / §18.1: what the cache layer can fail with, mapped to the
/// user-facing taxonomy by [`CacheError::to_salat_error`].
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// The pool could not be opened or migrated (and it was not the
    /// recoverable kind of damage — that is handled inside [`Store::open`]).
    #[error("could not open the cache database: {0}")]
    Open(String),
    /// A query or transaction failed.
    #[error("database query failed: {0}")]
    Query(String),
    /// A stored row could not be decoded back into its domain type —
    /// content corruption, not file corruption.
    #[error("stored row could not be decoded: {0}")]
    Serde(String),
}

impl From<sqlx::Error> for CacheError {
    fn from(error: sqlx::Error) -> Self {
        CacheError::Query(error.to_string())
    }
}

impl CacheError {
    /// The §18.1 mapping: every cache failure is `E601`, whatever the
    /// engine said — the user can only retry or back.
    pub fn to_salat_error(&self) -> SalatError {
        SalatError::DatabaseFailed
    }
}

/// The SQLite store (§17). Cheap to clone through an `Arc`; one per
/// process, wired in `app::Runtime` (§4.2).
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Open (creating if absent) the database at `path`, run the embedded
    /// migrations, and apply the §17.4 connection policy.
    ///
    /// §17.4 corruption recovery: if migrating or the first sanity query
    /// fails with a corruption error, the file is renamed aside, a fresh
    /// database is created in its place, and the incident is logged —
    /// prayer times are re-fetchable, so a corrupt cache is never fatal.
    pub async fn open(path: &Path) -> Result<Self, CacheError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CacheError::Open(format!("{}: {e}", parent.display())))?;
        }

        let store = Self::open_with_recovery(path, MAX_CONNECTIONS).await?;
        restrict_permissions(path);
        Ok(store)
    }

    /// An isolated in-memory database — the seam every `tests/cache.rs`
    /// case runs against, with zero filesystem footprint. A single
    /// connection, because each pooled connection to `:memory:` would be
    /// its own private database; the isolation between parallel tests
    /// comes from each store owning exactly one.
    pub async fn open_in_memory() -> Result<Self, CacheError> {
        Self::open_with_recovery(Path::new(":memory:"), 1).await
    }

    /// Open, migrate, sanity-check — and on corruption, rename aside and
    /// try exactly once more (§17.4).
    async fn open_with_recovery(path: &Path, max_connections: u32) -> Result<Self, CacheError> {
        let mut first_error: Option<sqlx::Error> = None;
        for attempt in 0..2 {
            let options = connect_options(path);
            let pool = SqlitePoolOptions::new()
                .max_connections(max_connections)
                .connect_with(options)
                .await;
            let pool = match pool {
                Ok(pool) => pool,
                Err(error) => {
                    if attempt == 0 && is_corruption(&error) {
                        first_error = Some(error);
                        quarantine(path);
                        continue;
                    }
                    return Err(CacheError::Open(error.to_string()));
                }
            };
            // Boot step 3 (§15.7): migrations run at open, so a user
            // upgrading never runs a migration command (§17.4).
            if let Err(error) = sqlx::migrate!("./migrations").run(&pool).await {
                // Roll the failed attempt back before deciding its fate.
                let error = sqlx::Error::from(error);
                pool.close().await;
                if attempt == 0 && is_corruption(&error) {
                    first_error = Some(error);
                    quarantine(path);
                    continue;
                }
                return Err(CacheError::Open(error.to_string()));
            }
            // The first real query is part of the corruption surface
            // (§17.4: "if migrate **or the first query** fails").
            if let Err(error) = sqlx::query("SELECT COUNT(*) FROM prayer_days")
                .execute(&pool)
                .await
            {
                pool.close().await;
                if attempt == 0 && is_corruption(&error) {
                    first_error = Some(error);
                    quarantine(path);
                    continue;
                }
                return Err(CacheError::Query(error.to_string()));
            }
            if first_error.is_some() {
                tracing::warn!(
                    target: "salat::cache",
                    "cache database was corrupt; the damaged file was renamed aside and \
                     a fresh one created — prayer times will be re-fetched"
                );
            }
            return Ok(Self { pool });
        }
        // Unreachable: the loop returns on its second iteration at the
        // latest, and `quarantine` failing does not continue it.
        Err(CacheError::Open(
            first_error.map(|e| e.to_string()).unwrap_or_default(),
        ))
    }

    /// The pool, for use by this module's `impl Store` blocks only.
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// The §17.3 vacuum, run once at boot: delete every table's rows
    /// older than 60 days (each by its own natural date column), and the
    /// `prayer_days`/`qibla` rows of a superseded `cache_key` whose
    /// `fetched_at` is older than 7 days. Current-key rows of recent
    /// dates are never touched — prayer times are immutable and the
    /// offline horizon depends on them (§17.3).
    ///
    /// Returns how many rows went, so boot can log it in one line.
    pub async fn vacuum(&self, now: DateTime<Tz>, current_key: &str) -> Result<u64, CacheError> {
        let cutoff_date = (now.date_naive() - Days::new(60)).to_string();
        let cutoff_60 = (now - chrono::Duration::days(60)).to_rfc3339();
        let cutoff_7 = (now - chrono::Duration::days(7)).to_rfc3339();

        let mut tx = self.pool().begin().await?;
        let mut removed = 0u64;

        let prayers = sqlx::query(
            "DELETE FROM prayer_days \
             WHERE date < ?1 \
                OR (cache_key <> ?2 AND datetime(fetched_at) < datetime(?3))",
        )
        .bind(&cutoff_date)
        .bind(current_key)
        .bind(&cutoff_7)
        .execute(&mut *tx)
        .await?;
        removed += prayers.rows_affected();

        let fired = sqlx::query("DELETE FROM fired_events WHERE date < ?1")
            .bind(&cutoff_date)
            .execute(&mut *tx)
            .await?;
        removed += fired.rows_affected();

        let completions = sqlx::query("DELETE FROM prayer_completions WHERE date < ?1")
            .bind(&cutoff_date)
            .execute(&mut *tx)
            .await?;
        removed += completions.rows_affected();

        let logged = sqlx::query("DELETE FROM event_log WHERE datetime(at) < datetime(?1)")
            .bind(&cutoff_60)
            .execute(&mut *tx)
            .await?;
        removed += logged.rows_affected();

        let qibla = sqlx::query(
            "DELETE FROM qibla \
             WHERE datetime(fetched_at) < datetime(?1) \
                OR (cache_key <> ?2 AND datetime(fetched_at) < datetime(?3))",
        )
        .bind(&cutoff_60)
        .bind(current_key)
        .bind(&cutoff_7)
        .execute(&mut *tx)
        .await?;
        removed += qibla.rows_affected();

        tx.commit().await?;
        Ok(removed)
    }
}

// ---- §11.4 / §17.2 storage vocabulary --------------------------------------
//
// The lowercase prayer and event-kind strings the tables store. They are
// this layer's wire format: a change to one is a migration decision
// (existing rows keep the old spelling), which is why they live here and
// not as derives on the domain enums.

/// The `prayer` column value of a notifiable prayer.
pub(crate) fn prayer_key(prayer: Notifiable) -> &'static str {
    match prayer {
        Notifiable::Fajr => "fajr",
        Notifiable::Dhuhr => "dhuhr",
        Notifiable::Asr => "asr",
        Notifiable::Maghrib => "maghrib",
        Notifiable::Isha => "isha",
    }
}

/// The prayer a stored `prayer` column names — `None` for anything else
/// (an empty cell, or a spelling a newer version wrote).
pub(crate) fn prayer_from_key(key: &str) -> Option<Notifiable> {
    match key {
        "fajr" => Some(Notifiable::Fajr),
        "dhuhr" => Some(Notifiable::Dhuhr),
        "asr" => Some(Notifiable::Asr),
        "maghrib" => Some(Notifiable::Maghrib),
        "isha" => Some(Notifiable::Isha),
        _ => None,
    }
}

/// The `kind` column value of a user-facing event (§11.4's three kinds).
pub(crate) fn kind_key(kind: EventKind) -> Option<&'static str> {
    match kind {
        EventKind::PreNotify => Some("pre_notify"),
        EventKind::Adhan => Some("adhan"),
        EventKind::MosqueReminder => Some("mosque_reminder"),
        // DayRollover is deliberately absent from the ledger: a re-fired
        // rollover merely refreshes the window, which is idempotent by
        // nature (§11.3), so it needs no cross-restart dedup.
        // Snooze re-fires are likewise harmless (§11.7) and, more to the
        // point, never persist: they are one-shot in-memory events.
        EventKind::DayRollover | EventKind::AdhanSnooze | EventKind::ReminderSnooze => None,
    }
}

/// The §17.4 connection policy: WAL, `synchronous = NORMAL`, a 5 s busy
/// timeout, foreign keys on, create-if-missing.
fn connect_options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(BUSY_TIMEOUT)
        .foreign_keys(true)
}

/// Is an engine error the §17.4 recoverable kind — file corruption, not
/// e.g. a permissions problem or a locked file?
fn is_corruption(error: &sqlx::Error) -> bool {
    let text = error.to_string().to_lowercase();
    text.contains("malformed") || text.contains("not a database") || text.contains("corrupt")
}

/// Rename a damaged database aside as `<path>.corrupt-<timestamp>`
/// (§17.4) and remove its WAL/SHM sidecars, so the fresh file created at
/// the same path cannot inherit a damaged write-ahead log.
///
/// The timestamp is the damaged file's own mtime — the cache layer never
/// reads a clock (see the module docs), and a stale-looking-but-unique
/// suffix is all the name has to guarantee. If the target already
/// exists, a counter is appended.
fn quarantine(path: &Path) {
    let stamp = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|mtime| mtime.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|age| age.as_secs())
        .unwrap_or(0);
    // `salat.db` → `salat.db.corrupt-<timestamp>`: the original file name
    // is preserved whole and the evidence suffix is appended (§17.4).
    let stem = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("salat.db");
    let mut candidate: PathBuf = path.with_file_name(format!("{stem}.corrupt-{stamp}"));
    let mut counter = 1;
    while candidate.exists() {
        candidate.set_file_name(format!(
            "{}-{counter}",
            candidate
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
        ));
        counter += 1;
    }
    if std::fs::rename(path, &candidate).is_ok() {
        tracing::warn!(
            target: "salat::cache",
            "quarantined corrupt database as {}",
            candidate.display()
        );
    }
    for sidecar in ["wal", "shm"] {
        let sidecar = path.with_extension(sidecar);
        if sidecar.exists() {
            // Best effort: a sidecar SQLite cannot reuse is ignored, and
            // a failure to remove it is not worth failing boot over.
            let _ = std::fs::remove_file(sidecar);
        }
    }
}

/// §17.1: the database (and its WAL/SHM sidecars, which hold the same
/// rows) are mode `0600`.
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let restrict = |file: &Path| {
            let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
        };
        restrict(path);
        for sidecar in ["wal", "shm"] {
            let sidecar = path.with_extension(sidecar);
            if sidecar.exists() {
                restrict(&sidecar);
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §17.4 corruption specimen: a file of pure garbage at the
    /// database path is renamed aside, a working store is created in its
    /// place, and the tables are queryable — never a crash.
    #[tokio::test]
    async fn a_corrupt_file_is_quarantined_and_recreated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("salat.db");
        std::fs::write(&db, b"this is definitely not a sqlite database")
            .expect("write the garbage file");

        let store = Store::open(&db).await.expect("recovery yields a store");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM prayer_days")
            .fetch_one(store.pool())
            .await
            .expect("the fresh database answers queries");
        assert_eq!(count, 0);

        // The damaged file was renamed aside, and something corrupt-
        // named remains next to the new database.
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("list the temp dir")
            .map(|entry| {
                entry
                    .expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        let quarantined = names
            .iter()
            .find(|name| name.contains("corrupt"))
            .expect("a corrupt-*.db file survives as evidence");
        assert!(quarantined.starts_with("salat."), "{quarantined}");
    }

    /// A fresh open creates the database and runs migrations; a second
    /// open of the same file is a no-op that still answers queries.
    #[tokio::test]
    async fn opening_twice_is_stable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("salat.db");
        let first = Store::open(&db).await.expect("first open");
        first.pool().close().await;
        let second = Store::open(&db).await.expect("second open");
        // The schema tables only — `_sqlx_migrations` is sqlx's own
        // bookkeeping, not part of the §17.2 five.
        let tables: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' \
               AND name NOT LIKE 'sqlite_%' \
               AND name NOT LIKE '\\_%' ESCAPE '\\'",
        )
        .fetch_one(second.pool())
        .await
        .expect("schema query");
        assert_eq!(tables, 5, "exactly the §17.2 five tables");
    }

    /// §17.1: the database file lands at mode 0600 on unix.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_database_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("salat.db");
        let store = Store::open(&db).await.expect("open");
        store.pool().close().await;
        let mode = std::fs::metadata(&db)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
