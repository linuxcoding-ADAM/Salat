//! `salat daemon` — run the scheduler headlessly, for the systemd user unit
//! (§14.1, §4.3). With `once`, run a single catch-up pass and exit — the
//! cron-style invocation Phase 13 defines (§23).
//!
//! Implemented in Phase 13 (§23); until then the command answers with
//! `SalatError::NotImplemented` so the binary never panics.

use crate::error::{ExitCode, SalatError};

pub fn run(_once: bool) -> Result<ExitCode, SalatError> {
    Err(SalatError::NotImplemented { command: "daemon" })
}
