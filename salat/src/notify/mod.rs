//! The notification contract (§12.5) and the scheduler's wiring into
//! it: [`Notifier`] with a [`desktop::DesktopNotifier`] production
//! implementation, [`RecordingNotifier`] for tests, the §12.1 body
//! composition, and [`NotifySink`] — the `EventSink` that turns a
//! fired `PreNotify`/`MosqueReminder` into a desktop notification.
//!
//! Gating is **not** here: `plan_events` already refuses to plan a
//! `PreNotify` without `notify`/`notifications.enabled`, or a
//! `MosqueReminder` without `mosque_reminder`/`notifications.enabled`
//! (§11.3), so an event reaching the sink has passed every gate. The
//! sink's only judgements are §12.1's — which kinds notify at all —
//! and §12.2/§12.3's — never block, never retry, degrade loudly.

pub mod desktop;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Duration;
use thiserror::Error;
use tokio::sync::watch;

use crate::config::Config;
use crate::domain::{EventKind, ScheduledEvent};
use crate::scheduler::{EventSink, log_prayer};
use crate::status::model::{Component, ComponentId, Health};

pub use desktop::DesktopNotifier;

/// §12.1's `app_name` — never a path or a binary name.
pub const APP_NAME: &str = "Salat";

/// §12.1's icon: a stock freedesktop name, so no asset ships.
pub const ICON: &str = "appointment-soon";

/// §12.1's `hint(Category("event"))`.
pub const CATEGORY_EVENT: &str = "event";

/// §12.1's two urgencies. Adhan's "critical and loud" is audio's
/// business (§13) — only the reminder is a critical *notification*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Urgency {
    Normal,
    Critical,
}

/// §12.1's timeout column: the pre-notify expires after 10 seconds,
/// the reminder is sticky (never expires — the user dismisses it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyTimeout {
    Seconds(u32),
    Never,
}

/// One notification to show (§12.5). Named `…Request` so it can never
/// be confused with `notify_rust::Notification`, which is the
/// transport's own builder — this is the transport-independent request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationRequest {
    pub summary: String,
    pub body: String,
    pub urgency: Urgency,
    pub timeout: NotifyTimeout,
    pub category: &'static str,
}

/// What can go wrong (§12.3): unavailable means the probe said there is
/// nothing to talk to (skip sending), failed means the send itself
/// errored or timed out (log, do not retry — a prayer notice has no
/// value late).
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum NotifyError {
    #[error("notifications unavailable: {detail}")]
    Unavailable { detail: String },
    #[error("sending failed: {detail}")]
    SendFailed { detail: String },
}

/// The §12.5 contract. `send` must never block its caller for long
/// (§12.2); `report` is the Status component (§12.3's table, §6.7).
#[async_trait]
pub trait Notifier: Send + Sync {
    async fn send(&self, request: NotificationRequest) -> Result<(), NotifyError>;
    fn report(&self) -> Component;
}

/// §12.3's `notifications.enabled = false` row: intentionally silent,
/// reported `Ok`/`DISABLED`. Constructed instead of probing — there is
/// nothing to talk to.
pub struct NoopNotifier;

#[async_trait]
impl Notifier for NoopNotifier {
    async fn send(&self, _request: NotificationRequest) -> Result<(), NotifyError> {
        Ok(())
    }

    fn report(&self) -> Component {
        Component::new(ComponentId::Notifications, Health::Ok, "DISABLED")
    }
}

/// The §12.5 test double: records every request, optionally fails with
/// a fixed error so the failure path is an assertion, not an
/// observation.
#[derive(Debug, Default)]
pub struct RecordingNotifier {
    requests: Mutex<Vec<NotificationRequest>>,
    fail_with: Option<NotifyError>,
}

impl RecordingNotifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// A notifier whose every `send` returns this error.
    pub fn failing(error: NotifyError) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            fail_with: Some(error),
        }
    }

    /// Every request that reached `send`, in order.
    pub fn sent(&self) -> Vec<NotificationRequest> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }
}

#[async_trait]
impl Notifier for RecordingNotifier {
    async fn send(&self, request: NotificationRequest) -> Result<(), NotifyError> {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request);
        }
        match &self.fail_with {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    fn report(&self) -> Component {
        Component::new(
            ComponentId::Notifications,
            Health::Unknown,
            "RECORDING (test double)",
        )
    }
}

/// §12.4: some notification daemons render a limited HTML subset, and
/// the location label is the only user-controlled string that reaches
/// a body — so `&`, `<` and `>` are escaped. No remote string ever does.
fn escape_markup(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Compose one fired event's notification (§12.1), or `None` when the
/// kind does not notify. Only `PreNotify` and `MosqueReminder` do —
/// Adhan's medium is the full-screen overlay plus audio, and a third
/// desktop notification would be noise. `late` is §11.5's flag: a
/// delayed fire prefixes the body with `(delayed)`.
pub(crate) fn compose(
    event: &ScheduledEvent,
    late: bool,
    cfg: &Config,
) -> Option<NotificationRequest> {
    // DayRollover and the snoozes carry no prayer; Adhan is handled
    // below by simply not matching.
    let prayer = event.prayer?;
    let settings = prayer.settings(&cfg.prayers);

    let (summary, body, urgency, timeout) = match event.kind {
        EventKind::PreNotify => {
            // The event fires at t − before, so the prayer's own
            // wall clock is one addition away — the specimen's
            // `19:21 · Sétif, Algeria`.
            let at = event.fire_at + Duration::minutes(i64::from(settings.before_minutes));
            (
                format!(
                    "{} in {} minutes",
                    log_prayer(prayer),
                    settings.before_minutes
                ),
                format!(
                    "{} · {}",
                    at.format("%H:%M"),
                    escape_markup(&cfg.location.label)
                ),
                Urgency::Normal,
                NotifyTimeout::Seconds(10),
            )
        }
        EventKind::MosqueReminder => (
            "Time to head out".to_string(),
            format!(
                "{} was {} minutes ago · Iqama soon",
                log_prayer(prayer),
                settings.after_minutes
            ),
            Urgency::Critical,
            NotifyTimeout::Never,
        ),
        // §12.1: Adhan gets no desktop notification; DayRollover and
        // the snooze re-fires are internal events.
        EventKind::Adhan
        | EventKind::DayRollover
        | EventKind::AdhanSnooze
        | EventKind::ReminderSnooze => return None,
    };

    let body = if late {
        format!("(delayed) {body}")
    } else {
        body
    };

    Some(NotificationRequest {
        summary,
        body,
        urgency,
        timeout,
        category: CATEGORY_EVENT,
    })
}

/// The production `EventSink` (§23 Phase 9's wiring): a fired
/// `PreNotify`/`MosqueReminder` becomes a desktop notification through
/// the [`Notifier`]. A send failure is logged and swallowed (§12.3) —
/// a prayer notice has no value late, and nothing may stall the
/// scheduler's tick.
pub struct NotifySink {
    notifier: Arc<dyn Notifier>,
    /// The live configuration — the prayer offsets and the location
    /// label the body is composed from. Gating happened at plan time.
    config: watch::Receiver<Arc<Config>>,
}

impl NotifySink {
    pub fn new(notifier: Arc<dyn Notifier>, config: watch::Receiver<Arc<Config>>) -> Self {
        Self { notifier, config }
    }
}

#[async_trait]
impl EventSink for NotifySink {
    async fn on_event(&self, event: ScheduledEvent, late: bool) {
        let cfg = self.config.borrow().clone();
        let Some(request) = compose(&event, late, &cfg) else {
            return;
        };
        match self.notifier.send(request).await {
            Ok(()) => tracing::info!(
                target: "salat::notify",
                kind = ?event.kind,
                prayer = ?event.prayer,
                late,
                "notification sent"
            ),
            // §12.3: log, do not retry. The event-log row for the fire
            // itself is the runner's and already landed.
            Err(error) => tracing::warn!(
                target: "salat::notify",
                %error,
                kind = ?event.kind,
                prayer = ?event.prayer,
                "notification failed — not retried"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::testing::{default_config, fixture_window, snapshot_now};
    use crate::domain::{EventKind, Notifiable, plan_events};

    /// The §12.1 pre-notify specimen, verbatim: a Maghrib at 19:21 with
    /// the default `before = 5` sends `Maghrib in 5 minutes` /
    /// `19:21 · Sétif, Algeria`, normal urgency, 10 s timeout, the
    /// `event` category hint.
    #[test]
    fn a_pre_notify_composes_the_specimen() {
        let cfg = default_config();
        let event = plan_events(&fixture_window(), &cfg, snapshot_now())
            .into_iter()
            .find(|event| {
                event.kind == EventKind::PreNotify && event.prayer == Some(Notifiable::Maghrib)
            })
            .expect("the plan holds Maghrib's pre-notify");

        let request = compose(&event, false, &cfg).expect("PreNotify composes");
        assert_eq!(request.summary, "Maghrib in 5 minutes");
        assert_eq!(request.body, "19:21 · Sétif, Algeria");
        assert_eq!(request.urgency, Urgency::Normal);
        assert_eq!(request.timeout, NotifyTimeout::Seconds(10));
        assert_eq!(request.category, "event");
    }

    /// The §12.1 reminder specimen, verbatim: `Time to head out` /
    /// `Maghrib was 15 minutes ago · Iqama soon`, critical urgency,
    /// sticky (never expires).
    #[test]
    fn a_mosque_reminder_composes_the_specimen() {
        let cfg = default_config();
        let event = plan_events(&fixture_window(), &cfg, snapshot_now())
            .into_iter()
            .find(|event| {
                event.kind == EventKind::MosqueReminder
                    && event.prayer == Some(Notifiable::Maghrib)
                    && event.date == crate::domain::testing::TODAY
            })
            .expect("the plan holds today's Maghrib reminder");

        let request = compose(&event, false, &cfg).expect("MosqueReminder composes");
        assert_eq!(request.summary, "Time to head out");
        assert_eq!(request.body, "Maghrib was 15 minutes ago · Iqama soon");
        assert_eq!(request.urgency, Urgency::Critical);
        assert_eq!(request.timeout, NotifyTimeout::Never);
    }

    /// §12.1: Adhan sends no desktop notification — its medium is the
    /// overlay plus audio — and neither do the internal kinds.
    #[test]
    fn adhan_and_internal_kinds_send_nothing() {
        let cfg = default_config();
        let plan = plan_events(&fixture_window(), &cfg, snapshot_now());
        for event in &plan {
            if matches!(
                event.kind,
                EventKind::Adhan
                    | EventKind::DayRollover
                    | EventKind::AdhanSnooze
                    | EventKind::ReminderSnooze
            ) {
                assert_eq!(compose(event, false, &cfg), None, "{:?}", event.kind);
            }
        }
    }

    /// §11.5's flag reaches the body: a delayed fire's body starts with
    /// `(delayed)` (§23 Phase 9's own assertion).
    #[test]
    fn a_delayed_fire_prefixes_the_body() {
        let cfg = default_config();
        let mut event = plan_events(&fixture_window(), &cfg, snapshot_now())
            .into_iter()
            .find(|event| {
                event.kind == EventKind::PreNotify && event.prayer == Some(Notifiable::Maghrib)
            })
            .expect("the plan holds Maghrib's pre-notify");
        assert!(
            compose(&event, true, &cfg)
                .unwrap()
                .body
                .starts_with("(delayed) ")
        );
        // The reminder too, not just the pre-notify.
        event.kind = EventKind::MosqueReminder;
        let request = compose(&event, true, &cfg).expect("MosqueReminder composes");
        assert!(request.body.starts_with("(delayed) "));
        assert!(request.body.ends_with("Iqama soon"));
    }

    /// §12.4: the location label is the only user-controlled string in
    /// a body, and it is escaped — a label like `A&B <Sétif>` cannot
    /// smuggle markup into a daemon that renders an HTML subset.
    #[test]
    fn the_location_label_is_escaped() {
        let mut cfg = default_config();
        cfg.location.label = "A&B <Sétif>".to_string();
        let event = plan_events(&fixture_window(), &cfg, snapshot_now())
            .into_iter()
            .find(|event| {
                event.kind == EventKind::PreNotify && event.prayer == Some(Notifiable::Maghrib)
            })
            .expect("the plan holds Maghrib's pre-notify");

        let request = compose(&event, false, &cfg).expect("PreNotify composes");
        assert_eq!(request.body, "19:21 · A&amp;B &lt;Sétif&gt;");
        // §12.4's other half: no unescaped markup character survives —
        // every `&` in the body starts an entity, and no raw `<`/`>`
        // is left.
        assert_eq!(
            request.body.matches('&').count(),
            request.body.matches("&amp;").count()
                + request.body.matches("&lt;").count()
                + request.body.matches("&gt;").count()
        );
        assert!(!request.body.contains('<') && !request.body.contains('>'));
    }

    /// The sink hands the composed request to the notifier — §12.5's
    /// "an assertion rather than an observation" — and a failing
    /// notifier is logged and swallowed (§12.3): the call returns, so
    /// the scheduler's tick never notices.
    #[tokio::test]
    async fn the_sink_sends_and_survives_a_failing_notifier() {
        let cfg = Arc::new(default_config());
        let (tx, rx) = watch::channel(Arc::clone(&cfg));
        let event = plan_events(&fixture_window(), &cfg, snapshot_now())
            .into_iter()
            .find(|event| {
                event.kind == EventKind::PreNotify && event.prayer == Some(Notifiable::Maghrib)
            })
            .expect("the plan holds Maghrib's pre-notify");

        // The happy path: one request, the composed specimen.
        let recorder = Arc::new(RecordingNotifier::new());
        NotifySink::new(Arc::clone(&recorder) as Arc<dyn Notifier>, rx.clone())
            .on_event(event, false)
            .await;
        let sent = recorder.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].summary, "Maghrib in 5 minutes");

        // §12.3's last row: a send error is not the scheduler's
        // problem — the sink returns normally and the request is
        // still recorded (the failure is the notifier's, not a skip).
        let failing = Arc::new(RecordingNotifier::failing(NotifyError::SendFailed {
            detail: "daemon wedged".to_string(),
        }));
        NotifySink::new(Arc::clone(&failing) as Arc<dyn Notifier>, rx)
            .on_event(event, true)
            .await;
        assert_eq!(failing.sent().len(), 1);
        assert!(failing.sent()[0].body.starts_with("(delayed) "));
        drop(tx);
    }
}
