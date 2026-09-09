//! `salat status` — print system status and exit (§14.1).
//!
//! The §15.8 pair's second surface: the same `SystemStatus` the
//! STATUS screen renders, through [`crate::status::render_text`]. One
//! refresh runs first — it is the network probe, so `Network` reports
//! a verified verdict rather than the boot-time `Unknown` — then the
//! status is collected and printed. §14.6's discipline holds: stdout
//! carries the block or nothing, every message goes to stderr, and
//! colour only under the four conditions.
//!
//! Exit code (§23 Phase 12 item 8): `0` when the system is healthy,
//! `1` when a component is `Down` — a degraded component is a warning
//! the block itself already tells, not a failure exit.

use std::io::{IsTerminal as _, Write as _};

use crate::cli::Cli;
use crate::error::{ExitCode, SalatError};
use crate::status::{Health, SystemStatus, render_text};

/// §14.6's four conditions, computed once: stdout a TTY, no
/// `--no-color`, `NO_COLOR` unset — the resolved `color_mode` has no
/// `none` variant left to check (`--no-color`/`NO_COLOR` are that
/// rule's carriers).
fn colored(cli: &Cli) -> bool {
    std::io::stdout().is_terminal() && !cli.no_color && std::env::var_os("NO_COLOR").is_none()
}

pub fn run(cli: &Cli) -> Result<ExitCode, SalatError> {
    let runtime = crate::app::runtime::Runtime::build(cli)?;
    // The probe run: one refresh decides `Network`'s health for real.
    runtime.block_on(runtime.refresh());
    let status = runtime.status();

    let text = render_text(&status, colored(cli));
    let mut stdout = std::io::stdout();
    write!(stdout, "{text}")
        .and_then(|()| stdout.flush())
        .map_err(crate::cli::commands::term_io)?;

    Ok(exit_for(&status))
}

/// The exit rule (§23 Phase 12 item 8): `0` when the system is
/// healthy, `1` when a component is `Down` — a degraded component is
/// a warning the block itself already tells, not a failure exit.
fn exit_for(status: &SystemStatus) -> ExitCode {
    let any_down = status
        .components
        .iter()
        .any(|component| component.health == Health::Down);
    if any_down {
        ExitCode::Runtime
    } else {
        ExitCode::Ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{Component, ComponentId, SystemStatus};

    /// A status whose ten components all carry the given health.
    fn status_with(health: Health) -> SystemStatus {
        SystemStatus {
            components: ComponentId::ALL
                .map(|id| Component::new(id, health, "fixture".to_owned()))
                .to_vec(),
            next_event: None,
            today: crate::domain::DaySummary {
                total: 5,
                passed: 3,
                remaining: 2,
                notifications_sent: 0,
                adhan_plays: 0,
                next_notify: None,
            },
            recent: Vec::new(),
        }
    }

    /// The rule's rows: healthy → 0, `Down` anywhere → 1, `Degraded`
    /// alone → 0 (a warning, not a failure exit).
    #[test]
    fn the_exit_rule_is_zero_healthy_one_down() {
        assert_eq!(exit_for(&status_with(Health::Ok)), ExitCode::Ok);
        assert_eq!(exit_for(&status_with(Health::Unknown)), ExitCode::Ok);
        assert_eq!(exit_for(&status_with(Health::Degraded)), ExitCode::Ok);
        assert_eq!(exit_for(&status_with(Health::Down)), ExitCode::Runtime);
    }
}
