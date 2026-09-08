//! The select! loop, tick, jump detection (§11.3). Independent of the
//! render loop.
//!
//! The runner is a `tokio` task with three wake sources — the 1 Hz
//! tick (a real interval in production, a manually driven channel in
//! tests, §11.8), runtime commands, and configuration publications —
//! and one `reconcile` that is invoked from all of them. Every
//! event-firing judgement anywhere in the runner funnels through
//! [`catchup::decide`], so the live tick, the startup catch-up and the
//! post-jump catch-up share the §11.4 ledger and the §11.5 grace
//! windows.

use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use chrono_tz::Tz;
use tokio::sync::{broadcast, mpsc, watch};

use crate::app::clock::Clock;
use crate::cache::{Store, prayer_key};
use crate::config::Config;
use crate::domain::{EventKind, PrayerWindow, ScheduledEvent, midnight_in, plan_events};

use super::catchup::{self, CatchupDecision};
use super::{
    ADHAN_SNOOZE, Command, EventSink, MOSQUE_SNOOZE, ServiceEvent, WindowSource, fired_row,
    log_category, missed_row,
};

/// §11.3: wall and monotonic readings that drift apart by more than
/// two seconds between reconcile ticks mean the machine slept (or the
/// clock was stepped) — a jump.
const JUMP_TOLERANCE: Duration = Duration::seconds(2);

/// §11.3: replan when the plan's last event is within six hours, to
/// keep the horizon populated.
const HORIZON: Duration = Duration::hours(6);

/// Everything the runner needs (§11.8). The clock and tick source are
/// injectable; the window source is the seam that keeps the scheduler
/// in the service layer, independent of the app layer and the TUI.
pub struct SchedulerDeps {
    /// The only source of "now" (§6.6).
    pub clock: Arc<dyn Clock>,
    /// The fired-events ledger and event log (§11.4).
    pub store: Arc<Store>,
    /// The ledger scope (§8.1): `"default"` or a `--config` scope.
    pub scope: String,
    /// The side-effect seam — the Phase 8 recording stub, replaced by
    /// notifications (Phase 9) and audio (Phase 10).
    pub sink: Arc<dyn EventSink>,
    /// The prayer window, served by the app layer.
    pub windows: Arc<dyn WindowSource>,
    /// The live configuration — every publication triggers a re-plan
    /// (§11.6).
    pub config: watch::Receiver<Arc<Config>>,
    /// Where [`ServiceEvent`]s are published for the UI's `svc_rx` arm
    /// (§15.2).
    pub events: broadcast::Sender<ServiceEvent>,
}

/// The spawn entry points (§23 Phase 8 interface).
pub struct Scheduler;

impl Scheduler {
    /// Start the scheduler on the production 1 Hz tick.
    ///
    /// The §11.5 startup catch-up runs to completion **before** the
    /// handle is returned, so a caller that subscribes afterwards can
    /// never mistake a catch-up fire for a live one — and catch-up
    /// fires raise no overlays anyway (§11.5).
    pub async fn spawn(deps: SchedulerDeps) -> SchedulerHandle {
        let (tick_tx, tick_rx) = mpsc::channel::<()>(1);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
            // A suspend can queue missed ticks; one catch-up reconcile
            // is enough — do not burst.
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // Consume the interval's immediate first tick: the runner
            // has just reconciled at spawn.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                if tick_tx.send(()).await.is_err() {
                    break;
                }
            }
        });
        Self::spawn_on_ticker(deps, tick_rx).await
    }

    /// Start the scheduler on a caller-driven tick channel — §11.8's
    /// test seam: the tests advance a [`crate::MockClock`] and send one
    /// tick per reconciled instant, no `sleep` anywhere.
    pub async fn spawn_on_ticker(
        deps: SchedulerDeps,
        ticks: mpsc::Receiver<()>,
    ) -> SchedulerHandle {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(16);
        let svc_tx = deps.events.clone();
        let mut runner = Runner::new(deps);
        runner.startup_catchup().await;
        let task = tokio::spawn(runner.run(cmd_rx, ticks));
        SchedulerHandle {
            cmd_tx,
            svc_tx,
            task,
        }
    }
}

/// The caller's grip on a spawned runner (§23 Phase 8 interface).
pub struct SchedulerHandle {
    cmd_tx: mpsc::Sender<Command>,
    svc_tx: broadcast::Sender<ServiceEvent>,
    task: tokio::task::JoinHandle<()>,
}

impl SchedulerHandle {
    /// Ask for a re-plan from the current window and configuration.
    pub async fn replan(&self) {
        let _ = self.cmd_tx.send(Command::Refresh).await;
    }

    /// Queue one snooze (§11.7) — a runtime command, never persisted.
    pub async fn snooze(&self, source: ScheduledEvent) {
        let _ = self.cmd_tx.send(Command::Snooze { source }).await;
    }

    /// An owned queue for §11.7 snoozes — the runtime forwards the
    /// overlay's `s` press without holding its scheduler read guard
    /// across the send's await.
    pub(crate) fn snooze_sender(&self) -> SnoozeSender {
        SnoozeSender {
            cmd_tx: self.cmd_tx.clone(),
        }
    }

    /// Subscribe to the service events the UI's `svc_rx` arm selects
    /// on (§15.2).
    pub fn subscribe(&self) -> broadcast::Receiver<ServiceEvent> {
        self.svc_tx.subscribe()
    }

    /// Stop the runner and wait for it to exit.
    pub async fn shutdown(self) {
        let _ = self.cmd_tx.send(Command::Shutdown).await;
        let _ = self.task.await;
    }
}

/// A cloned snooze queue (see [`SchedulerHandle::snooze_sender`]) —
/// `Runtime::snooze` holds its read guard only for the clone, never
/// across an await.
pub(crate) struct SnoozeSender {
    cmd_tx: mpsc::Sender<Command>,
}

impl SnoozeSender {
    /// Queue one snooze — same command as
    /// [`SchedulerHandle::snooze`], same "latest wins" semantics.
    pub async fn snooze(&self, source: ScheduledEvent) {
        let _ = self.cmd_tx.send(Command::Snooze { source }).await;
    }
}

/// The event loop's private state.
struct Runner {
    deps: SchedulerDeps,
    /// The forward plan (§11.3) — built by [`plan_events`], sorted
    /// ascending, due events drained by each reconcile.
    plan: Vec<ScheduledEvent>,
    /// The §11.7 one-shot snoozes — in-memory only.
    snoozes: Vec<ScheduledEvent>,
    /// The window the current plan was built from; a new `Arc` means a
    /// refresh happened and the plan must be rebuilt.
    planned_from: Option<Arc<PrayerWindow>>,
    /// Set by commands, jumps and rollovers; consumed by the next
    /// reconcile.
    replan_needed: bool,
    /// The jump-detection baselines (§11.3).
    last_wall: Option<DateTime<Utc>>,
    last_mono: Option<std::time::Duration>,
    /// The civil date of the last horizon top-up attempt — bounds an
    /// offline refresh to one try per day instead of a 1 Hz retry.
    last_horizon_attempt: Option<NaiveDate>,
}

/// Whether a due event left the plan (fired, skipped or missed) or
/// stays for the next tick (ledger failure — retry, §11.4).
enum Discharge {
    Done,
    Retry,
}

/// Which call site discharged an event (§11.5): the live tick — the
/// TUI was already running, so an overlay may be raised — or the
/// startup catch-up, which never raises one. The scheduler knows the
/// difference because they are different call sites; no new state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FireOrigin {
    Live,
    Catchup,
}

impl Runner {
    fn new(deps: SchedulerDeps) -> Self {
        Self {
            deps,
            plan: Vec::new(),
            snoozes: Vec::new(),
            planned_from: None,
            replan_needed: false,
            last_wall: None,
            last_mono: None,
            last_horizon_attempt: None,
        }
    }

    /// §11.3's select loop: ticker, commands, configuration.
    async fn run(mut self, mut cmd_rx: mpsc::Receiver<Command>, mut ticks: mpsc::Receiver<()>) {
        // Disarm the config arm once its sender is gone so a dead
        // watch cannot spin the loop.
        let mut config_alive = true;
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    None | Some(Command::Shutdown) => break,
                    Some(Command::Refresh) => {
                        self.replan_needed = true;
                        if !self.reconcile().await {
                            break;
                        }
                    }
                    Some(Command::Snooze { source }) => self.add_snooze(source),
                },
                _ = ticks.recv() => {
                    if !self.reconcile().await {
                        break;
                    }
                },
                changed = self.deps.config.changed(), if config_alive => {
                    if changed.is_err() {
                        config_alive = false;
                    } else {
                        self.deps.config.borrow_and_update();
                        // §11.6: a config change re-arms the plan; the
                        // ledger keys on the event's identity, so what
                        // already fired stays fired.
                        self.replan_needed = true;
                        if !self.reconcile().await {
                            break;
                        }
                    }
                },
            }
        }
    }

    /// One §11.3 reconcile. Returns `false` when the window source is
    /// gone and the runner should exit.
    async fn reconcile(&mut self) -> bool {
        let Some(window) = self.deps.windows.window().await else {
            tracing::info!("the window source is gone — the scheduler runner exits");
            return false;
        };
        let cfg = self.deps.config.borrow().clone();
        let now = self.deps.clock.now_in(zone_of(&cfg, &window));

        // Jump detection (§11.3): keep (last_wall, last_mono); a drift
        // beyond the tolerance means suspend/resume or an NTP step —
        // log it, judge what came due during the gap (the plan still
        // holds it), then replan against the new now.
        let wall = self.deps.clock.now_utc();
        let mono = self.deps.clock.now_monotonic();
        let jumped = match (self.last_wall, self.last_mono) {
            (Some(last_wall), Some(last_mono)) => {
                let dwall = wall - last_wall;
                let dmono = Duration::from_std(mono.saturating_sub(last_mono)).unwrap_or_default();
                (dwall - dmono).abs() > JUMP_TOLERANCE
            }
            _ => false,
        };
        self.last_wall = Some(wall);
        self.last_mono = Some(mono);
        if jumped {
            tracing::info!(
                "clock jump detected (suspend/resume or NTP step) — catching up and replanning"
            );
            self.replan_needed = true;
        }

        // Fire every due event, oldest first (§11.3).
        self.judge_due(now, FireOrigin::Live).await;

        // Replan when the served window changed (a config-change or
        // rollover refresh handed back a new one) or when asked.
        let window_changed = !self
            .planned_from
            .as_ref()
            .is_some_and(|planned| Arc::ptr_eq(planned, &window));
        if self.replan_needed || window_changed {
            self.replan(&window, &cfg, now);
        }

        // Horizon top-up (§11.3): when the plan's last event is within
        // six hours, refresh the window — at most once per civil date,
        // so an offline day cannot turn this into a retry loop.
        let horizon_low = self
            .plan
            .last()
            .is_none_or(|last| last.fire_at - now < HORIZON);
        if horizon_low && self.last_horizon_attempt != Some(now.date_naive()) {
            self.last_horizon_attempt = Some(now.date_naive());
            self.deps.windows.refresh().await;
            if let Some(fresh) = self.deps.windows.window().await {
                self.replan(&fresh, &cfg, now);
            }
        }
        true
    }

    /// Judge and fire every due event of the plan and the snoozes,
    /// oldest first, through the one §11.5 decision.
    async fn judge_due(&mut self, now: DateTime<Tz>, origin: FireOrigin) {
        let mut due: Vec<ScheduledEvent> = self
            .plan
            .iter()
            .chain(self.snoozes.iter())
            .filter(|event| event.fire_at <= now)
            .copied()
            .collect();
        due.sort_by_key(|event| event.fire_at);

        let mut discharged: Vec<ScheduledEvent> = Vec::new();
        for event in due {
            match self.fire_once(&event, now, origin).await {
                Discharge::Done => discharged.push(event),
                Discharge::Retry => tracing::warn!(
                    kind = ?event.kind,
                    "event left undischarged — retried on the next tick"
                ),
            }
        }
        if !discharged.is_empty() {
            self.plan.retain(|event| !discharged.contains(event));
            self.snoozes.retain(|event| !discharged.contains(event));
        }
    }

    /// One event, one decision, every side effect in order (§11.4–§11.5).
    async fn fire_once(
        &mut self,
        event: &ScheduledEvent,
        now: DateTime<Tz>,
        origin: FireOrigin,
    ) -> Discharge {
        let decision = match catchup::decide(&self.deps.scope, event, now, &self.deps.store).await {
            Ok(decision) => decision,
            Err(error) => {
                // The ledger write failed: nothing fired, so nothing is
                // owed yet — keep the event and retry next tick.
                tracing::error!(
                    %error,
                    kind = ?event.kind,
                    "the fired-events ledger failed — the event is retried next tick"
                );
                return Discharge::Retry;
            }
        };

        match decision {
            CatchupDecision::Fire { late } => {
                // Rows are stamped at the event's own time (§11.5's
                // `18:21 │ …` specimens), written before the side
                // effect — alongside the ledger insert inside `decide`.
                let cfg = self.deps.config.borrow().clone();
                let (label, detail) = fired_row(event, late, &cfg);
                if let Err(error) = self
                    .deps
                    .store
                    .append_event(
                        event.fire_at,
                        log_category(event.kind),
                        &label,
                        detail.as_deref(),
                        event.prayer.map(prayer_key),
                    )
                    .await
                {
                    // The ledger row already landed, so a retry could
                    // never re-reach this branch — fire anyway and log
                    // the gap. §11.4's trade: lose one row, never
                    // duplicate an audible alarm.
                    tracing::error!(%error, "the fired event-log row failed after the ledger insert");
                }
                self.deps.sink.on_event(*event, late).await;
                // §11.5's overlay rule: an overlay is raised only if the
                // TUI was already running when the event fired. A
                // catch-up fire writes its ledger row, sends the
                // notification and plays its audio, then leaves the UI
                // alone — the dashboard renders the prayer in its
                // post-adhan state and the event log says `(delayed)`.
                if origin == FireOrigin::Live {
                    let _ = self.deps.events.send(ServiceEvent::Fired {
                        event: *event,
                        late,
                    });
                }
                if event.kind == EventKind::DayRollover {
                    tracing::info!("day rollover — refetching the window and replanning");
                    self.deps.windows.rollover().await;
                    self.replan_needed = true;
                }
                Discharge::Done
            }
            CatchupDecision::Skip { missed } => {
                if missed {
                    let (label, detail) = missed_row(event);
                    if let Err(error) = self
                        .deps
                        .store
                        .append_event(
                            event.fire_at,
                            log_category(event.kind),
                            &label,
                            Some(detail),
                            event.prayer.map(prayer_key),
                        )
                        .await
                    {
                        tracing::error!(%error, "the missed event-log row failed");
                    }
                    let _ = self
                        .deps
                        .events
                        .send(ServiceEvent::Missed { event: *event });
                }
                Discharge::Done
            }
            // Not due at this instant after all — leave it in the plan.
            CatchupDecision::Pending => Discharge::Retry,
        }
    }

    /// §11.7: one in-memory re-fire per snooze, the latest per alarm
    /// wins. Never planned, never ledgered, gone on restart.
    fn add_snooze(&mut self, source: ScheduledEvent) {
        let (kind, duration) = match source.kind {
            EventKind::Adhan => (EventKind::AdhanSnooze, ADHAN_SNOOZE),
            EventKind::MosqueReminder => (EventKind::ReminderSnooze, MOSQUE_SNOOZE),
            // Only the two alarms snooze.
            _ => return,
        };
        let now = self.deps.clock.now_in(source.fire_at.timezone());
        self.snoozes
            .retain(|pending| !(pending.kind == kind && pending.prayer == source.prayer));
        self.snoozes.push(ScheduledEvent {
            kind,
            prayer: source.prayer,
            date: source.date,
            fire_at: now + duration,
        });
        tracing::info!(prayer = ?source.prayer, "snoozed — one in-memory re-fire scheduled");
    }

    /// Rebuild the forward plan from a window and configuration.
    fn replan(&mut self, window: &Arc<PrayerWindow>, cfg: &Config, now: DateTime<Tz>) {
        self.plan = plan_events(window, cfg, now);
        self.planned_from = Some(Arc::clone(window));
        self.replan_needed = false;
    }

    /// The §11.5 startup catch-up: plan from the start of today so
    /// today's already-past events are visible (the forward planner
    /// drops them), judge every due one, keep the forward remainder.
    async fn startup_catchup(&mut self) {
        let Some(window) = self.deps.windows.window().await else {
            tracing::warn!("no window served at scheduler start — the first tick retries");
            return;
        };
        let cfg = self.deps.config.borrow().clone();
        let tz = zone_of(&cfg, &window);
        let now = self.deps.clock.now_in(tz);

        let horizon = midnight_in(tz, now.date_naive()).unwrap_or(now);
        self.plan = plan_events(&window, &cfg, horizon);
        self.planned_from = Some(window);

        // The jump-detection baseline starts now, not at the horizon.
        self.last_wall = Some(self.deps.clock.now_utc());
        self.last_mono = Some(self.deps.clock.now_monotonic());

        self.judge_due(now, FireOrigin::Catchup).await;

        // Everything due was judged; keep only the forward plan.
        self.plan.retain(|event| event.fire_at > now);
    }
}

/// The configured zone defines "now" (§10.2); the window's zone — what
/// the API actually computed in — is the fallback for a config that
/// has not been validated yet (never in practice: boot validates).
fn zone_of(cfg: &Config, window: &PrayerWindow) -> Tz {
    cfg.location.timezone.parse().unwrap_or(window.today.tz)
}
