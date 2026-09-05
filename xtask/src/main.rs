//! Workspace task runner (§19): `gen-cities` · `screenshots` ·
//! `check-fixtures`.
//!
//! Phase 0 ships the entry point only — the workspace must contain the
//! member from day one, but each subcommand arrives with the phase that
//! needs it (`gen-cities` with the location phase, `screenshots` with
//! Phase 15, `check-fixtures` with the API phase). Until then they answer
//! honestly with a non-zero exit rather than pretending to work.

use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <task>

tasks:
  gen-cities      regenerate salat/assets/cities.bin from GeoNames (§9.2)
  screenshots     regenerate docs/screenshots/ at each breakpoint (§22.4)
  check-fixtures  validate the recorded AlAdhan fixtures (§7.8)
";

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("gen-cities") => {
            eprintln!("xtask: gen-cities is not implemented yet (arrives with the location phase)");
            ExitCode::from(1)
        }
        Some("screenshots") => {
            eprintln!("xtask: screenshots is not implemented yet (arrives in Phase 15)");
            ExitCode::from(1)
        }
        Some("check-fixtures") => {
            eprintln!("xtask: check-fixtures is not implemented yet (arrives with the API phase)");
            ExitCode::from(1)
        }
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
