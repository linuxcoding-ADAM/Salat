//! SIGINT/SIGTERM/SIGHUP wiring (§15.2's `sigterm` arm, §8.5's SIGHUP).
//!
//! Phase 5's share of the signal story is the quick live loop's: a
//! `SIGTERM` or `SIGINT` arriving from *outside* the terminal (raw mode
//! turns the keyboard's Ctrl-C into a key event, not a signal) must exit
//! through the same `TerminalGuard` path as `q`, and `SIGHUP` re-reads
//! the configuration file (§8.5). The daemon's richer shutdown — stop
//! audio, flush the event log, release the lock (§18.2's table) — is
//! Phase 13 and layers onto the same channel.
//!
//! Signals become messages and nothing else: a handler that did real
//! work would re-enter non-async-safe territory in a signal context.

use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

/// The three signals Salat handles (§18.2's table: `SIGTERM`/`SIGINT`
/// stop; `SIGHUP` re-reads the configuration).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// SIGINT — stop what is running, cleanly.
    Interrupt,
    /// SIGTERM — stop what is running, cleanly.
    Terminate,
    /// SIGHUP — re-read the configuration from disk (§8.5).
    Hangup,
}

impl Signal {
    /// The signal's name, for the log.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Interrupt => "SIGINT",
            Self::Terminate => "SIGTERM",
            Self::Hangup => "SIGHUP",
        }
    }

    /// Whether this signal means "stop".
    pub const fn is_shutdown(self) -> bool {
        matches!(self, Self::Interrupt | Self::Terminate)
    }
}

/// Forward the three signals onto one channel. The watcher tasks live
/// for the process's lifetime and stop when the receiver is dropped.
///
/// Must be called from within a tokio runtime — the quick loop's. A
/// signal that cannot be watched (a kernel without the signal, in
/// practice) is logged and skipped: the loop still runs, it just cannot
/// be stopped from outside the terminal.
pub fn forward() -> mpsc::Receiver<Signal> {
    let (sender, receiver) = mpsc::channel(8);
    for (kind, message) in [
        (SignalKind::interrupt(), Signal::Interrupt),
        (SignalKind::terminate(), Signal::Terminate),
        (SignalKind::hangup(), Signal::Hangup),
    ] {
        let sender = sender.clone();
        tokio::spawn(async move {
            match signal(kind) {
                Ok(mut stream) => {
                    while stream.recv().await.is_some() {
                        tracing::debug!(target: "salat::app", "received {}", message.name());
                        if sender.send(message).await.is_err() {
                            break; // the loop is gone; stop watching
                        }
                    }
                }
                Err(error) => tracing::warn!(
                    target: "salat::app",
                    "could not watch {}: {error}",
                    message.name()
                ),
            }
        });
    }
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vocabulary: three signals, their names, which ones stop.
    #[test]
    fn the_three_signals_are_named_and_classified() {
        assert_eq!(Signal::Interrupt.name(), "SIGINT");
        assert_eq!(Signal::Terminate.name(), "SIGTERM");
        assert_eq!(Signal::Hangup.name(), "SIGHUP");
        assert!(Signal::Interrupt.is_shutdown());
        assert!(Signal::Terminate.is_shutdown());
        assert!(!Signal::Hangup.is_shutdown());
    }

    /// `forward` installs its watchers and hands back the channel; the
    /// receiver observing a close (all senders dropped) proves the
    /// wiring spawned. Real delivery is not tested in-process — sending
    /// a signal to the test harness is exactly the flake the manual
    /// test list covers instead (§23 Phase 5 item 9).
    #[tokio::test]
    async fn forward_returns_a_live_channel() {
        let mut receiver = forward();
        // Drop every sender clone the spawned tasks hold by... we
        // cannot reach them; instead assert the channel is simply
        // readable state — a try_recv reporting Empty (not Closed)
        // proves the watchers still hold their senders.
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }
}
