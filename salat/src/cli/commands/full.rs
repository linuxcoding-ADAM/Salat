//! `salat --full` — the complete interactive TUI on the alternate screen
//! (§14.1): the shell, navigation stack and every screen of §16.
//!
//! Implemented in Phase 6 (§23); until then the command answers with
//! `SalatError::NotImplemented` so the binary never panics.

use crate::error::{ExitCode, SalatError};

pub fn run() -> Result<ExitCode, SalatError> {
    Err(SalatError::NotImplemented {
        command: "--full (interactive TUI)",
    })
}
