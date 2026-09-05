//! `salat status` — print system status and exit (§14.1).
//!
//! Implemented in Phase 12 (§23); until then the command answers with
//! `SalatError::NotImplemented` so the binary never panics.

use crate::error::{ExitCode, SalatError};

pub fn run() -> Result<ExitCode, SalatError> {
    Err(SalatError::NotImplemented { command: "status" })
}
