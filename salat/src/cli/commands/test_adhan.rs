//! `salat test-adhan` — play the configured Adhan and exit (§14.1).
//!
//! Implemented in Phase 10 (§23); until then the command answers with
//! `SalatError::NotImplemented` so the binary never panics.

use crate::error::{ExitCode, SalatError};

pub fn run() -> Result<ExitCode, SalatError> {
    Err(SalatError::NotImplemented {
        command: "test-adhan",
    })
}
