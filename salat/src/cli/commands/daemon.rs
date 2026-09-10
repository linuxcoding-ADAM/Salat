//! `salat daemon` — the scheduler without the terminal (§14.1, §4.3,
//! §23 Phase 13): run the prayer-time engine headlessly so
//! notifications and Adhan happen with no terminal open, under the
//! systemd user unit or by hand.
//!
//! The §4.3 handshake is the whole difference from the TUI: a daemon
//! *must* own the schedule — a second daemon (or a foreground TUI
//! already running) is a fatal `E903` exit with the holder's record,
//! not the silent observer mode the render paths fold the same
//! arbitration into, because a daemon that renders nothing has nothing
//! to fall back to.
//!
//! `--once` is the cron-style invocation: claim, run the §11.5 startup
//! catch-up (which completes inside the claim — every event already
//! due fires through the grace-window judgement before the call
//! returns), stop, exit `0`. No loop, no tick, no signal watch.
//!
//! The long-running loop is the runner's own 1 Hz select (spawned by
//! the claim) plus this thin supervisor: a 1 Hz rollover check, the
//! signal channel (§18.2 — SIGTERM/SIGINT stop, SIGHUP re-reads the
//! configuration), and nothing else. §4.3's promise that the daemon
//! draws nothing holds by construction: no terminal is ever entered.

use std::sync::Arc;
use std::time::Duration;

use crate::app::runtime::{Runtime, SchedulerClaim};
use crate::app::signals;
use crate::cli::Cli;
use crate::error::{ExitCode, SalatError};

/// The supervisor loop's cadence (§4.3's 1 Hz: "a handful of
/// comparisons" — the rollover check and nothing else).
const TICK: Duration = Duration::from_secs(1);

/// `salat daemon [--once]` (§14.1). The spec's §23 signature
/// `daemon::run(&mut Runtime, once)` is realized the way every other
/// command realizes its spec signature — `status::run`, `full::run`:
/// the `Runtime` is built here from the parsed `Cli`, because
/// `Runtime::build` owns the whole boot ladder and hands back the
/// `Arc` the scheduler's window source holds a `Weak` to.
pub fn run(cli: &Cli, once: bool) -> Result<ExitCode, SalatError> {
    let runtime = Runtime::build(cli)?;
    runtime.block_on(daemon(&runtime, once))
}

/// The daemon's body: claim, then either the `--once` pass or the
/// supervised loop.
async fn daemon(runtime: &Arc<Runtime>, once: bool) -> Result<ExitCode, SalatError> {
    match runtime.claim_scheduler().await {
        SchedulerClaim::Owned => tracing::info!(
            target: "salat::daemon",
            "daemon owns the schedule — notifications and adhan run headlessly"
        ),
        // The second-daemon row of the phase's contract: a clear
        // "already running" message (E903's details carry the holder's
        // pid and start time) and a non-zero exit — `Restart=on-failure`
        // may sensibly retry it (exit 1, not the config-error 3).
        SchedulerClaim::Occupied(holder) => {
            return Err(SalatError::SchedulerConflict {
                pid: holder.pid,
                started_at: holder.started_at,
            });
        }
        SchedulerClaim::Unavailable { cause } => {
            return Err(SalatError::SchedulerLockFailed { cause });
        }
    }

    if once {
        // The claim already ran the startup catch-up to completion;
        // stopping the runner flushes every in-flight write and stops
        // any catch-up Adhan it started, then the lock releases.
        runtime.shutdown_scheduler().await;
        tracing::info!(target: "salat::daemon", "catch-up pass complete — exiting");
        return Ok(ExitCode::Ok);
    }

    supervise(runtime).await;

    // §18.2's daemon shutdown: the runner's awaited exit is the event
    // log's flush, the guard's drop releases the §4.3 lock, and §13.3's
    // ladder (SIGTERM → 500 ms → SIGKILL, bounded) stops any playing
    // Adhan — together comfortably inside the 2-second budget.
    runtime.shutdown_scheduler().await;
    tracing::info!(target: "salat::daemon", "daemon stopped cleanly");
    Ok(ExitCode::Ok)
}

/// The long-running supervisor: the rollover check at 1 Hz and the
/// signal channel. The schedule itself — the plan, the 1 Hz firing
/// tick, the horizon top-up — lives in the runner the claim spawned;
/// this loop never touches it.
async fn supervise(runtime: &Arc<Runtime>) {
    let mut signal_rx = signals::forward();
    let mut tick = tokio::time::interval(TICK);
    // A suspend can queue missed ticks; one rollover check is enough.
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Consume the interval's immediate first tick: the claim just
    // reconciled everything.
    tick.tick().await;
    loop {
        tokio::select! {
            signal = signal_rx.recv() => match signal {
                // SIGTERM/SIGINT (§18.2): stop cleanly — the shutdown
                // ladder after the loop covers audio and the lock.
                Some(signal) if signal.is_shutdown() => {
                    tracing::info!(target: "salat::daemon", "received {} — shutting down", signal.name());
                    break;
                }
                // SIGHUP (§8.5): re-read the configuration file. The
                // store's publication wakes the runner's config arm on
                // its own — no replan command is needed here.
                Some(_) => runtime.reload_config().await,
                // The watcher tasks are gone (a signal source could not
                // be watched at startup): nothing further can arrive on
                // this channel, and polling it would spin — stop
                // supervised and let the runner carry on until the
                // process is killed.
                None => break,
            },
            _ = tick.tick() => {
                // The day boundary (§11.6): refetch and roll the
                // window. The runner observes the new window Arc on its
                // next reconcile and replans.
                runtime.refresh_on_rollover().await;
            },
        }
    }
}
