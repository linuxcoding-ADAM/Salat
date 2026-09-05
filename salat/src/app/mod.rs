//! Application-level wiring. Phase 0 owns logging initialisation (§18.4);
//! `runtime`, `boot`, `lock` and `signals` arrive with their phases.

pub mod boot;
pub mod clock;
pub mod lock;
pub mod runtime;
pub mod signals;

use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::OnceLock;

use tracing_subscriber::fmt::MakeWriter;

/// The log file is truncated at 5 MB on startup — a prayer app does not
/// need log rotation infrastructure (§18.4).
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

/// The log file handle, opened once by [`init_logging`].
///
/// Held in a `OnceLock` for the process lifetime — the documented §18.3
/// allowance for shared state that is initialised exactly once before any
/// reader exists. `std` implements `Write` for `&File`, so the handle can
/// serve every writer clone without mutation or locking.
static LOG_FILE: OnceLock<Option<File>> = OnceLock::new();

/// The writer the file layer of the subscriber logs through. Never stdout —
/// §14.6 forbids tracing on stdout while a viewport is active, and Phase 0
/// has no viewport at all.
#[derive(Clone, Copy)]
struct LogWriter;

impl std::io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match LOG_FILE.get() {
            Some(Some(file)) => (&*file).write(buf),
            // No log file (unwritable state dir, or not yet initialised):
            // discard, so a log call can never fail a command.
            _ => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match LOG_FILE.get() {
            Some(Some(file)) => (&*file).flush(),
            _ => Ok(()),
        }
    }
}

impl<'a> MakeWriter<'a> for LogWriter {
    type Writer = LogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter
    }
}

/// Where the log file lives: `$XDG_STATE_HOME/salat/salat.log`, falling
/// back to `~/.local/state/salat/salat.log` (§18.4, via `directories`).
pub fn log_path() -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let state_dir = base.state_dir()?.join("salat");
    Some(state_dir.join("salat.log"))
}

/// The `EnvFilter` directive for a given `-v` count and `RUST_LOG` value
/// (§18.4): level from `-v` count, then `RUST_LOG`. Pure, so the test
/// suite can assert what `init_logging` will do without touching global
/// state.
pub fn log_directive(verbosity: u8, rust_log: Option<&str>) -> String {
    if let Some(rust_log) = rust_log {
        let trimmed = rust_log.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    match verbosity {
        0 => "info".to_owned(),
        1 => "debug".to_owned(),
        _ => "trace".to_owned(),
    }
}

/// Initialise `tracing` (§18.4): the file at [`log_path`] — created, and
/// truncated first if it already exceeds 5 MB — with a stderr layer added
/// only when `--verbose` was given and no TUI is running (Phase 0 has no
/// TUI).
///
/// Failure to initialise logging is never fatal for a prayer app: every
/// failure path here degrades to a silent no-op and the command continues.
/// This is a deliberate, documented decision — §15.7 makes only boot steps
/// 1–3 fatal, and logging is not one of them.
pub fn init_logging(verbosity: u8) {
    let Some(log_path) = log_path() else {
        return;
    };
    let Some(parent) = log_path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }

    // §18.4: truncate at 5 MB on startup.
    let truncate = std::fs::metadata(&log_path)
        .map(|m| m.len() >= MAX_LOG_BYTES)
        .unwrap_or(false);
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(truncate)
        .append(!truncate)
        .open(&log_path);
    let file = match file {
        Ok(file) => file,
        Err(_) => return, // unwritable state dir: run without logging
    };

    // First initialiser wins; a second call (e.g. in tests) is a no-op.
    if LOG_FILE.set(Some(file)).is_err() {
        return;
    }

    let directive = log_directive(verbosity, std::env::var("RUST_LOG").ok().as_deref());
    let filter = tracing_subscriber::EnvFilter::new(directive);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(LogWriter)
        .with_ansi(false);

    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let result = if verbosity > 0 {
        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_ansi(false);
        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .with(stderr_layer)
            .try_init()
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .try_init()
    };
    // Already initialised (a test binary may have set a global default
    // first): keep logging to the file we just opened and continue.
    let _ = result;

    tracing::info!(target: "salat::app", "salat starting (log: {})", log_path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §18.4: level from `-v` count, then `RUST_LOG`.
    #[test]
    fn log_directive_follows_verbosity_then_rust_log() {
        assert_eq!(log_directive(0, None), "info");
        assert_eq!(log_directive(1, None), "debug");
        assert_eq!(log_directive(2, None), "trace");
        assert_eq!(log_directive(7, None), "trace"); // `-v -v` and beyond
        assert_eq!(log_directive(1, Some("")), "debug"); // blank RUST_LOG ignored
        assert_eq!(log_directive(1, Some("  ")), "debug");
        assert_eq!(log_directive(0, Some("salat=trace")), "salat=trace");
        assert_eq!(log_directive(5, Some("warn")), "warn"); // RUST_LOG wins
    }
}
