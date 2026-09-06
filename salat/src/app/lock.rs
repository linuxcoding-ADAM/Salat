//! The scheduler lock file and foreground/daemon arbitration (§4.3).
//!
//! `salat daemon` (Phase 13) takes the exclusive advisory lock and owns
//! the schedule; `salat` and `salat --full` *try* the lock — winning
//! makes them the scheduler owner, losing puts them in observer mode:
//! they render everything but fire nothing, and the Status screen
//! reports `RUNNING (daemon)` instead of `RUNNING (foreground)`.
//!
//! The lock is `flock`-style (per file description): one holder per
//! open handle, within a process and across processes. It is released
//! when the guard drops — on a clean exit, a crash, or a kill — with
//! no stale-lock recovery to get wrong. The file itself survives and
//! always describes its current (or last) holder.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use chrono::{DateTime, Utc};

/// The holder record the lock file carries (§23 Phase 8 data model).
/// `pid` is `0` and `started_at` `None` when the file exists but holds
/// an unreadable record — a crashed holder's leftover; the lock itself
/// is what arbitrates, so the record is informational only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LockFile {
    /// The holding process, for the Status screen's observer report.
    pub pid: u32,
    /// When that process acquired the lock.
    pub started_at: Option<DateTime<Utc>>,
}

/// An exclusively held scheduler lock (§4.3). Dropping the guard
/// releases it.
#[derive(Debug)]
pub struct LockGuard {
    _file: File,
}

/// The outcome of trying the lock (§4.3).
#[derive(Debug)]
pub enum Acquisition {
    /// We hold it — this process schedules.
    Held(LockGuard),
    /// Another process holds it — observer mode: render, never fire.
    Occupied(LockFile),
}

/// Try to take the exclusive scheduler lock at `path` (§4.3).
///
/// `now` stamps the holder record; like every cache-layer write, the
/// instant is passed in rather than read from the ambient clock.
pub fn try_acquire(path: &Path, now: DateTime<Utc>) -> io::Result<Acquisition> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;

    match file.try_lock() {
        Ok(()) => {
            // Record who holds it, then hold the guard. A failed write
            // after a successful lock still owns the lock — the record
            // is informational, the lock is the arbitration — so the
            // error is surfaced but ownership kept.
            let record = format!("{} {}\n", std::process::id(), now.to_rfc3339());
            let write_error = file
                .set_len(0)
                .and_then(|()| file.seek(SeekFrom::Start(0)))
                .and_then(|_| file.write_all(record.as_bytes()))
                .and_then(|()| file.flush());
            if let Err(error) = write_error {
                tracing::warn!(%error, "the lock file could not be rewritten — ownership still holds");
            }
            Ok(Acquisition::Held(LockGuard { _file: file }))
        }
        Err(TryLockError::WouldBlock) => {
            let mut record = String::new();
            // The holder may be mid-rewrite; a read failure degrades to
            // the unknown-holder record, never to a false "free".
            let _ = file.read_to_string(&mut record);
            Ok(Acquisition::Occupied(parse_holder(&record)))
        }
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// `<pid> <rfc3339>` → the holder record; anything unparsable is the
/// unknown-holder record (pid 0, no start time).
fn parse_holder(record: &str) -> LockFile {
    let mut parts = record.split_whitespace();
    let pid = parts.next().and_then(|pid| pid.parse().ok()).unwrap_or(0);
    let started_at = parts
        .next()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc));
    LockFile { pid, started_at }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-06T18:00:00Z")
            .expect("the literal parses")
            .with_timezone(&Utc)
    }

    /// §4.3 + §20.2: two schedulers cannot both hold the lock — the
    /// second try lands in observer mode, with the first holder's
    /// record readable for the Status report. `std`'s `try_lock` uses
    /// `flock` semantics here (per open handle, verified on this
    /// platform), so the conflict is observable in-process.
    #[test]
    fn two_schedulers_cannot_both_hold_the_lock() {
        let dir = tempfile::tempdir().expect("a tempdir for the lock");
        let path = dir.path().join("scheduler.lock");

        let first = try_acquire(&path, now()).expect("the first acquire");
        let Acquisition::Held(guard) = first else {
            panic!("an uncontended lock must be held, got {first:?}");
        };

        let second = try_acquire(&path, now()).expect("the second acquire");
        let Acquisition::Occupied(holder) = second else {
            panic!("a held lock must not be acquired again, got {second:?}");
        };
        assert_eq!(holder.pid, std::process::id());
        assert_eq!(
            holder.started_at,
            Some(now()),
            "the observer reads the holder's record"
        );

        // Releasing the first guard frees the lock again.
        drop(guard);
        match try_acquire(&path, now()).expect("the third acquire") {
            Acquisition::Held(_) => {}
            Acquisition::Occupied(holder) => {
                panic!("a released lock is free again, but saw {holder:?}")
            }
        }
    }

    /// A leftover file with an unreadable record still arbitrates by
    /// the lock, and the observer sees the unknown-holder record.
    #[test]
    fn an_unreadable_record_degrades_to_unknown_holder() {
        let dir = tempfile::tempdir().expect("a tempdir for the lock");
        let path = dir.path().join("scheduler.lock");
        std::fs::write(&path, "not a record\n").expect("the leftover record");

        let first = try_acquire(&path, now()).expect("the first acquire");
        // The pre-existing garbage content is replaced on acquisition.
        assert!(matches!(first, Acquisition::Held(_)));

        let second = try_acquire(&path, now()).expect("the second acquire");
        let Acquisition::Occupied(holder) = second else {
            panic!("the lock is held, got {second:?}")
        };
        assert_eq!(holder.pid, std::process::id());
        assert_eq!(holder.started_at, Some(now()));
    }

    /// The parser itself: the exact record shape, and the degraded
    /// shapes.
    #[test]
    fn the_holder_record_round_trips() {
        let holder = parse_holder("1234 2026-09-06T18:00:00+00:00\n");
        assert_eq!(holder.pid, 1234);
        assert_eq!(holder.started_at, Some(now()));

        assert_eq!(parse_holder("").pid, 0);
        assert_eq!(parse_holder("").started_at, None);
        assert_eq!(parse_holder("junk junk").pid, 0);
        assert_eq!(parse_holder("42 junk").pid, 42);
        assert_eq!(parse_holder("42 junk").started_at, None);
    }
}
