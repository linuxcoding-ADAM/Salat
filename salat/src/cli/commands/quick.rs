//! Bare `salat` — the quick live view in an inline viewport (§14.1, §16.14).
//!
//! Two entries, matching §14.3's dispatch:
//! - [`run_live`] — a TTY stdout and no `--once`: the inline viewport that
//!   renders the 14-row layout at 1 Hz until the user quits.
//! - [`run_once`] — `--once`, or any non-TTY stdout: one frame through the
//!   same render path, printed as trimmed lines with no ANSI (§14.6).
//!
//! Implemented in Phase 5 (§23); until then both entries answer with
//! `SalatError::NotImplemented` so the binary never panics.

use crate::error::{ExitCode, SalatError};

pub fn run_live() -> Result<ExitCode, SalatError> {
    Err(SalatError::NotImplemented {
        command: "quick (live view)",
    })
}

pub fn run_once() -> Result<ExitCode, SalatError> {
    Err(SalatError::NotImplemented {
        command: "quick (--once frame)",
    })
}
