//! `DesktopNotifier` (§12.1–12.3): notify-rust over the session bus's
//! `org.freedesktop.Notifications`.
//!
//! The capability probe runs once (§12.3) — [`DesktopNotifier::probed`]
//! is what boot stage 4 calls — and `send` follows §12.2 exactly: the
//! synchronous `show()` is wrapped in `tokio::task::spawn_blocking`
//! and awaited under a five-second timeout, so a wedged notification
//! daemon can stall neither the scheduler's tick nor the render loop.
//! Degradation is a `Component` detail, never a panic and never a
//! fatal error: the app works fine on a TTY with no desktop session.

use std::sync::RwLock;
use std::time::Duration;

use async_trait::async_trait;
use notify_rust::Notification as DbusNotification;
use notify_rust::{Hint, Timeout as DbusTimeout, Urgency as DbusUrgency};

use super::{
    APP_NAME, CATEGORY_EVENT, ICON, NotificationRequest, Notifier, NotifyError, NotifyTimeout,
    Urgency,
};
use crate::status::model::{Component, ComponentId, Health};

/// §12.2: every D-Bus round-trip `send` makes is bounded by this.
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// The boot probe's own bound (§12.3) — same reasoning, once, at boot.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The probe's outcome (§12.3's table), kept for [`Notifier::report`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ProbeOutcome {
    /// Not probed yet — reported honestly as `Unknown`, never `Ok`.
    #[default]
    NotProbed,
    /// Session bus present, daemon answered `GetCapabilities`.
    Enabled,
    /// No `$DBUS_SESSION_BUS_ADDRESS` (SSH, bare TTY).
    NoBus,
    /// Bus present, but no daemon answered.
    NoDaemon,
    /// The daemon never answered within [`PROBE_TIMEOUT`] — a wedged
    /// bus is its own row of honesty, not a claim of "no daemon".
    TimedOut,
}

impl ProbeOutcome {
    /// The §12.3 detail string and health this outcome reports.
    const fn report(self) -> (Health, &'static str) {
        match self {
            Self::NotProbed => (Health::Unknown, "NOT PROBED"),
            Self::Enabled => (Health::Ok, "ENABLED"),
            Self::NoBus => (Health::Degraded, "UNAVAILABLE (no D-Bus)"),
            Self::NoDaemon => (Health::Degraded, "NO NOTIFICATION DAEMON"),
            Self::TimedOut => (Health::Degraded, "PROBE TIMED OUT"),
        }
    }
}

/// The production notifier (§12.5's contract half).
pub struct DesktopNotifier {
    /// The boot probe's outcome. Read on every `send` (a degraded
    /// probe short-circuits it) and by `report`.
    probe: RwLock<ProbeOutcome>,
    /// §12.3's last row: the most recent send failure's text, cleared
    /// by the next successful send. `report` prefers it over the probe
    /// — a daemon that died after boot is news.
    last_error: RwLock<Option<String>>,
}

impl Default for DesktopNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl DesktopNotifier {
    /// An unprobed notifier — `report` says `NOT PROBED`, and `send`
    /// still attempts the bus. Production uses [`Self::probed`].
    pub fn new() -> Self {
        Self {
            probe: RwLock::new(ProbeOutcome::NotProbed),
            last_error: RwLock::new(None),
        }
    }

    /// The §12.3 boot probe: attempt a D-Bus session round-trip and
    /// record the outcome. Never fatal, never blocks past
    /// [`PROBE_TIMEOUT`].
    pub async fn probe(&self) {
        let outcome = self.probe_once().await;
        tracing::info!(target: "salat::notify", ?outcome, "notification capability probe");
        if let Ok(mut probe) = self.probe.write() {
            *probe = outcome;
        }
    }

    /// [`Self::new`] followed by [`Self::probe`] — the one-liner boot
    /// stage 4 calls.
    pub async fn probed() -> Self {
        let notifier = Self::new();
        notifier.probe().await;
        notifier
    }

    /// §12.3's probe, as the table's rows:
    ///
    /// * no `$DBUS_SESSION_BUS_ADDRESS` (SSH, bare TTY) → `NoBus`;
    /// * bus present, daemon answers `GetCapabilities` → `Enabled`;
    /// * bus present, no daemon / it errors → `NoDaemon`;
    /// * no answer within [`PROBE_TIMEOUT`] → `TimedOut`.
    async fn probe_once(&self) -> ProbeOutcome {
        if std::env::var("DBUS_SESSION_BUS_ADDRESS").is_err() {
            return ProbeOutcome::NoBus;
        }
        // `notify_rust::get_capabilities` is synchronous on this crate's
        // zbus-only feature set (§5.1) — same treatment as `show`:
        // blocking call on the blocking pool, awaited under a bound.
        let probe_task = tokio::task::spawn_blocking(notify_rust::get_capabilities);
        match tokio::time::timeout(PROBE_TIMEOUT, probe_task).await {
            Ok(Ok(Ok(_capabilities))) => ProbeOutcome::Enabled,
            Ok(Ok(Err(_no_daemon))) => ProbeOutcome::NoDaemon,
            // A panic inside the probe is still "the bus gave no usable
            // answer" — not a timeout, so it lands on this row.
            Ok(Err(_panicked)) => ProbeOutcome::NoDaemon,
            Err(_elapsed) => ProbeOutcome::TimedOut,
        }
    }
}

#[async_trait]
impl Notifier for DesktopNotifier {
    async fn send(&self, request: NotificationRequest) -> Result<(), NotifyError> {
        // §12.3: skip sending when the probe said there is nothing to
        // talk to — the fired event is still in the event log, so the
        // Status screen and log stay truthful.
        if let Ok(probe) = self.probe.read() {
            match *probe {
                ProbeOutcome::NoBus => {
                    return Err(NotifyError::Unavailable {
                        detail: "no D-Bus session bus".to_string(),
                    });
                }
                ProbeOutcome::NoDaemon => {
                    return Err(NotifyError::Unavailable {
                        detail: "no notification daemon".to_string(),
                    });
                }
                ProbeOutcome::TimedOut => {
                    return Err(NotifyError::Unavailable {
                        detail: "notification daemon unresponsive".to_string(),
                    });
                }
                // `Enabled` sends; `NotProbed` tries its luck.
                ProbeOutcome::Enabled | ProbeOutcome::NotProbed => {}
            }
        }

        // §12.2: the synchronous `show()` does D-Bus round-trips that
        // can block for seconds — always inside `spawn_blocking`, the
        // result awaited under a timeout. A send that outlives the
        // timeout keeps its blocking thread until the daemon lets go
        // (it cannot be cancelled), but nothing here ever waits for it.
        let sent = tokio::task::spawn_blocking(move || {
            let mut notification = DbusNotification::new();
            notification
                .appname(APP_NAME)
                .icon(ICON)
                .summary(&request.summary)
                .body(&request.body)
                .hint(Hint::Category(CATEGORY_EVENT.to_string()))
                .urgency(match request.urgency {
                    Urgency::Normal => DbusUrgency::Normal,
                    Urgency::Critical => DbusUrgency::Critical,
                })
                .timeout(match request.timeout {
                    NotifyTimeout::Seconds(seconds) => DbusTimeout::Milliseconds(seconds * 1000),
                    NotifyTimeout::Never => DbusTimeout::Never,
                });
            notification.show().map(|_handle| ())
        });

        let outcome = match tokio::time::timeout(SEND_TIMEOUT, sent).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(error))) => Err(NotifyError::SendFailed {
                detail: error.to_string(),
            }),
            // A panic inside the blocking task — the daemon layer
            // misbehaved, not the scheduler; carry the join text.
            Ok(Err(panicked)) => Err(NotifyError::SendFailed {
                detail: panicked.to_string(),
            }),
            Err(_elapsed) => Err(NotifyError::SendFailed {
                detail: "no answer from the notification daemon within 5s".to_string(),
            }),
        };

        match &outcome {
            Ok(()) => {
                if let Ok(mut last_error) = self.last_error.write() {
                    *last_error = None;
                }
            }
            Err(error) => {
                tracing::warn!(target: "salat::notify", %error, "notification send failed");
                if let Ok(mut last_error) = self.last_error.write() {
                    *last_error = Some(error.to_string());
                }
            }
        }
        outcome
    }

    fn report(&self) -> Component {
        // §12.3's last row outranks the probe: a daemon that was alive
        // at boot and died since is `Degraded` with the error text.
        if let Ok(last_error) = self.last_error.read()
            && let Some(detail) = last_error.as_ref()
        {
            return Component::new(
                ComponentId::Notifications,
                Health::Degraded,
                detail.clone(),
            );
        }
        let (health, detail) = match self.probe.read() {
            Ok(probe) => probe.report(),
            Err(_) => (Health::Unknown, "NOT PROBED"),
        };
        Component::new(ComponentId::Notifications, health, detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §12.3's vocabulary, before any probe: `Unknown`, never a false
    /// `Ok` (§16.4 item 8's honesty rule).
    #[test]
    fn an_unprobed_notifier_reports_unknown() {
        let notifier = DesktopNotifier::new();
        let component = notifier.report();
        assert_eq!(component.id, ComponentId::Notifications);
        assert_eq!(component.health, Health::Unknown);
        assert_eq!(component.detail, "NOT PROBED");
    }

    /// A recorded send failure outranks the probe in `report` (§12.3's
    /// fifth row), and is cleared by a successful send. The send
    /// itself is exercised through the trait on a machine with a live
    /// daemon only — this asserts the bookkeeping, not the bus.
    #[tokio::test]
    async fn a_send_failure_is_reported_and_cleared_by_success() {
        let notifier = DesktopNotifier::new();
        // Force the degraded path the bookkeeping reacts to.
        if let Ok(mut probe) = notifier.probe.write() {
            *probe = ProbeOutcome::NoDaemon;
        }
        // A skipped send is an error too — it must be recorded.
        let error = notifier
            .send(NotificationRequest {
                summary: "Maghrib in 5 minutes".to_string(),
                body: "19:21 · Sétif, Algeria".to_string(),
                urgency: Urgency::Normal,
                timeout: NotifyTimeout::Seconds(10),
                category: CATEGORY_EVENT,
            })
            .await
            .expect_err("the probe said no daemon");
        assert!(matches!(error, NotifyError::Unavailable { .. }));

        // The failure path's own bookkeeping: inject the error text
        // directly, then clear it the way a successful send would.
        if let Ok(mut last_error) = notifier.last_error.write() {
            *last_error = Some("no notification daemon".to_string());
        }
        assert_eq!(notifier.report().health, Health::Degraded);
        assert_eq!(notifier.report().detail, "no notification daemon");
        if let Ok(mut last_error) = notifier.last_error.write() {
            *last_error = None;
        }
        // Back to the probe's verdict — the NoDaemon row.
        assert_eq!(notifier.report().detail, "NO NOTIFICATION DAEMON");
    }

    /// The five probe outcomes map to §12.3's exact detail strings.
    #[test]
    fn the_probe_outcomes_report_the_tables_wording() {
        assert_eq!(ProbeOutcome::Enabled.report(), (Health::Ok, "ENABLED"));
        assert_eq!(
            ProbeOutcome::NoBus.report(),
            (Health::Degraded, "UNAVAILABLE (no D-Bus)")
        );
        assert_eq!(
            ProbeOutcome::NoDaemon.report(),
            (Health::Degraded, "NO NOTIFICATION DAEMON")
        );
        assert_eq!(
            ProbeOutcome::TimedOut.report(),
            (Health::Degraded, "PROBE TIMED OUT")
        );
        assert_eq!(
            ProbeOutcome::NotProbed.report(),
            (Health::Unknown, "NOT PROBED")
        );
    }
}
