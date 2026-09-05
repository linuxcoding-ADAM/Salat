//! `salat config` — print the resolved scope, config, database and lock
//! paths (§8.1), then the effective configuration (§14.1). With `--edit`,
//! open `$EDITOR` and validate on save.
//!
//! Implemented in Phase 1 (§23); until then the command answers with
//! `SalatError::NotImplemented` so the binary never panics.

use crate::error::{ExitCode, SalatError};

pub fn run(_edit: bool) -> Result<ExitCode, SalatError> {
    Err(SalatError::NotImplemented { command: "config" })
}
