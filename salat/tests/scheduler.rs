//! §23 Phase 8: the scheduler runner end-to-end — `spawn_on_ticker`
//! driven by a manual tick channel, `MockClock` advanced by the test,
//! an in-memory store, and recording seams for the sink and the window
//! source. No network, no wall clock, no sleep-for-timing: every case
//! in §11 that the unit tests judged in isolation is here as one whole
//! running loop — fire-once, the §11.4 ledger across a restart, the
//! §11.5 catch-up graces, §11.3's jump detection, §11.6's config
//! re-arm, §11.7's snooze, and the day rollover.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use salat::cache::Store;
use salat::config::Config;
use salat::domain::{
    DayMeta, EventKind, HijriDate, Notifiable, PrayerDay, PrayerWindow, ScheduledEvent,
};
use salat::scheduler::{
    EventSink, Scheduler, SchedulerDeps, SchedulerHandle, ServiceEvent, WindowSource,
};
use tokio::sync::{broadcast, mpsc, watch};

/// Africa/Algiers — the default config's zone, UTC+1 with no DST in
/// 2026, so every fixture instant below is unambiguous.
const ZONE: Tz = chrono_tz::Africa::Algiers;

const fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    match NaiveDate::from_ymd_opt(year, month, day) {
        Some(date) => date,
        None => panic!("the fixture dates are valid"),
    }
}

/// The §20.3 fixture window's dates: Sep 4/5/6 2026, plus Sep 7 for the
/// post-rollover window.
const YESTERDAY: NaiveDate = date(2026, 9, 4);
const TODAY: NaiveDate = date(2026, 9, 5);
const TOMORROW: NaiveDate = date(2026, 9, 6);
const AFTER_TOMORROW: NaiveDate = date(2026, 9, 7);

/// A wall-clock instant in the fixture zone.
fn at(day: NaiveDate, hour: u32, minute: u32, second: u32) -> DateTime<Tz> {
    ZONE.with_ymd_and_hms(day.year(), day.month(), day.day(), hour, minute, second)
        .single()
        .unwrap_or_else(|| panic!("Africa/Algiers has no DST — {day} {hour:02}:{minute:02} exists"))
}

/// The same wall-clock reading in UTC, for `MockClock::at`.
fn utc(day: NaiveDate, hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
    at(day, hour, minute, second).with_timezone(&Utc)
}

// ---- fixtures ---------------------------------------------------------------

/// One fixture day: the six prayer times in [`salat::domain::Prayer`]
/// order. Only the scheduler-facing fields (date, zone, times) matter
/// here; the rest are well-formed filler.
fn fixture_day(day: NaiveDate, times: [(u32, u32); 6], hijri_day: u32) -> PrayerDay {
    let next = day.succ_opt().expect("every fixture date has a successor");
    PrayerDay {
        date: day,
        tz: ZONE,
        times: times.map(|(hour, minute)| at(day, hour, minute, 0)),
        midnight: at(next, 0, 37, 0),
        last_third: at(next, 2, 47, 0),
        hijri: HijriDate {
            day: hijri_day,
            month_number: 3,
            month_en: "Rabīʿ al-awwal".to_owned(),
            month_ar: "رَبيع الأوّل".to_owned(),
            year: 1448,
        },
        meta: DayMeta {
            method_id: 12,
            method_name: "Union Organization Islamic de France".to_owned(),
            school: 0,
            lat: 36.19,
            lng: 5.41,
            tz_name: "Africa/Algiers".to_owned(),
        },
    }
}

/// The §20.3 window: Sep 4/5/6 2026 — Fajr 05:09 · Dhuhr 12:47 · Asr
/// 16:23 · Maghrib 19:21 · Isha 20:47 today, drifting a minute per day.
fn initial_window() -> PrayerWindow {
    PrayerWindow {
        yesterday: Some(fixture_day(
            YESTERDAY,
            [(5, 8), (6, 43), (12, 46), (16, 24), (19, 22), (20, 48)],
            22,
        )),
        today: fixture_day(
            TODAY,
            [(5, 9), (6, 44), (12, 47), (16, 23), (19, 21), (20, 47)],
            23,
        ),
        tomorrow: fixture_day(
            TOMORROW,
            [(5, 10), (6, 45), (12, 47), (16, 22), (19, 19), (20, 45)],
            24,
        ),
    }
}

/// What the runtime serves after the Sep 5→6 rollover: the old
/// tomorrow becomes today, and Sep 7 joins the horizon.
fn rolled_window() -> PrayerWindow {
    PrayerWindow {
        yesterday: Some(fixture_day(
            TODAY,
            [(5, 9), (6, 44), (12, 47), (16, 23), (19, 21), (20, 47)],
            23,
        )),
        today: fixture_day(
            TOMORROW,
            [(5, 10), (6, 45), (12, 47), (16, 22), (19, 19), (20, 45)],
            24,
        ),
        tomorrow: fixture_day(
            AFTER_TOMORROW,
            [(5, 11), (6, 46), (12, 48), (16, 23), (19, 20), (20, 46)],
            25,
        ),
    }
}

// ---- seams ------------------------------------------------------------------

/// The Phase 8 recording stub: every side effect lands in a vector the
/// assertions read, in firing order.
#[derive(Default)]
struct RecordingSink {
    fired: Mutex<Vec<(ScheduledEvent, bool)>>,
}

#[async_trait]
impl EventSink for RecordingSink {
    async fn on_event(&self, event: ScheduledEvent, late: bool) {
        self.fired
            .lock()
            .expect("the recording sink is not poisoned")
            .push((event, late));
    }
}

impl RecordingSink {
    fn fired(&self) -> Vec<(ScheduledEvent, bool)> {
        self.fired
            .lock()
            .expect("the recording sink is not poisoned")
            .clone()
    }

    fn count(&self, kind: EventKind, prayer: Option<Notifiable>) -> usize {
        self.fired()
            .iter()
            .filter(|(event, _)| event.kind == kind && event.prayer == prayer)
            .count()
    }
}

/// A window source with counters: `refresh` is the §11.3 horizon top-up
/// seam, `rollover` is §11.6's refetch — the tests prove both ran by
/// reading the counters, and prove the replan by what fires next.
struct MockWindowSource {
    window: Mutex<Option<Arc<PrayerWindow>>>,
    /// Installed by the test, served by the next `refresh()`.
    after_refresh: Mutex<Option<Arc<PrayerWindow>>>,
    /// Installed by the test, served by the next `rollover()`.
    after_rollover: Mutex<Option<Arc<PrayerWindow>>>,
    refreshes: AtomicUsize,
    rollovers: AtomicUsize,
}

impl MockWindowSource {
    fn new(window: PrayerWindow) -> Self {
        Self {
            window: Mutex::new(Some(Arc::new(window))),
            after_refresh: Mutex::new(None),
            after_rollover: Mutex::new(None),
            refreshes: AtomicUsize::new(0),
            rollovers: AtomicUsize::new(0),
        }
    }

    /// Stage the window a horizon top-up will serve.
    fn on_refresh_serve(&self, window: PrayerWindow) {
        *self
            .after_refresh
            .lock()
            .expect("the window source is not poisoned") = Some(Arc::new(window));
    }

    /// Stage the window a rollover refetch will serve.
    fn on_rollover_serve(&self, window: PrayerWindow) {
        *self
            .after_rollover
            .lock()
            .expect("the window source is not poisoned") = Some(Arc::new(window));
    }

    fn refreshes(&self) -> usize {
        self.refreshes.load(Ordering::SeqCst)
    }

    fn rollovers(&self) -> usize {
        self.rollovers.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl WindowSource for MockWindowSource {
    async fn window(&self) -> Option<Arc<PrayerWindow>> {
        self.window
            .lock()
            .expect("the window source is not poisoned")
            .clone()
    }

    async fn refresh(&self) {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        let fresh = self
            .after_refresh
            .lock()
            .expect("the window source is not poisoned")
            .take();
        if let Some(fresh) = fresh {
            *self
                .window
                .lock()
                .expect("the window source is not poisoned") = Some(fresh);
        }
    }

    async fn rollover(&self) {
        self.rollovers.fetch_add(1, Ordering::SeqCst);
        let fresh = self
            .after_rollover
            .lock()
            .expect("the window source is not poisoned")
            .take();
        if let Some(fresh) = fresh {
            *self
                .window
                .lock()
                .expect("the window source is not poisoned") = Some(fresh);
        }
    }
}

// ---- harness ----------------------------------------------------------------

/// One beat of real time for the current-thread runtime to run the
/// parked runner — the tests never sleep *for* timing, only to yield
/// the executor between a clock advance and its assertions.
async fn beat() {
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
}

/// A spawned scheduler plus everything the assertions read back.
struct Harness {
    clock: Arc<salat::app::clock::MockClock>,
    store: Arc<Store>,
    windows: Arc<MockWindowSource>,
    cfg_tx: watch::Sender<Arc<Config>>,
    sink: Arc<RecordingSink>,
    svc: broadcast::Receiver<ServiceEvent>,
    tick_tx: mpsc::Sender<()>,
    handle: SchedulerHandle,
}

impl Harness {
    /// A scheduler over a fresh store and window, clock pinned at
    /// `start`. The §11.5 startup catch-up has already run when this
    /// returns — `spawn_on_ticker` completes it before yielding the
    /// handle.
    async fn start(start: DateTime<Utc>) -> Harness {
        let store = Arc::new(Store::open_in_memory().await.expect("an in-memory store"));
        Harness::start_on(
            store,
            Arc::new(salat::app::clock::MockClock::at(start)),
            MockWindowSource::new(initial_window()),
        )
        .await
    }

    /// The "same machine, maybe a second process" variant: the caller
    /// owns the store and the clock, so a restart can share the ledger.
    async fn start_on(
        store: Arc<Store>,
        clock: Arc<salat::app::clock::MockClock>,
        windows: MockWindowSource,
    ) -> Harness {
        let windows = Arc::new(windows);
        let (cfg_tx, cfg_rx) = watch::channel(Arc::new(Config::default()));
        let (events_tx, svc) = broadcast::channel(64);
        let sink = Arc::new(RecordingSink::default());
        let (tick_tx, tick_rx) = mpsc::channel(16);
        let handle = Scheduler::spawn_on_ticker(
            SchedulerDeps {
                clock: clock.clone(),
                store: store.clone(),
                scope: "default".to_owned(),
                sink: sink.clone(),
                windows: windows.clone(),
                config: cfg_rx,
                events: events_tx,
            },
            tick_rx,
        )
        .await;
        Harness {
            clock,
            store,
            windows,
            cfg_tx,
            sink,
            svc,
            tick_tx,
            handle,
        }
    }

    /// Drive one tick and give the runner a beat to reconcile.
    async fn tick(&self) {
        self.tick_tx
            .send(())
            .await
            .expect("the runner holds the tick channel");
        beat().await;
    }
}

/// Every service publication since the last drain.
fn drain(svc: &mut broadcast::Receiver<ServiceEvent>) -> Vec<ServiceEvent> {
    let mut events = Vec::new();
    while let Ok(event) = svc.try_recv() {
        events.push(event);
    }
    events
}

/// Whether the drained publications report this event missed.
fn has_missed(events: &[ServiceEvent], kind: EventKind, prayer: Option<Notifiable>) -> bool {
    events.iter().any(|event| match event {
        ServiceEvent::Missed { event } => event.kind == kind && event.prayer == prayer,
        _ => false,
    })
}

/// The event-log rows of one civil date — the §11.5 specimens are
/// asserted against the store, not the broadcast.
async fn logged_on(store: &Store, day: NaiveDate) -> Vec<salat::cache::LoggedEvent> {
    store
        .events_today(ZONE, at(day, 23, 59, 59))
        .await
        .expect("the event log answers")
}

// ---- §11.1: fire exactly once -----------------------------------------------

/// The core §11.1 property: an event fires exactly once at the right
/// wall-clock second, and replayed ticks — redraws, resizes, whatever
/// else wakes the loop — fire nothing.
#[tokio::test]
async fn events_fire_exactly_once_across_replayed_ticks() {
    let harness = Harness::start(utc(TODAY, 19, 15, 0)).await;

    // Maghrib's Pre-Adhan at 19:16 — on the second, so not late.
    harness.clock.advance(chrono::Duration::minutes(1));
    harness.tick().await;
    harness.tick().await;
    harness.tick().await;
    assert_eq!(
        harness
            .sink
            .count(EventKind::PreNotify, Some(Notifiable::Maghrib)),
        1,
        "replayed ticks fire nothing"
    );

    // The Adhan at 19:21 — same story.
    harness.clock.advance(chrono::Duration::minutes(5));
    harness.tick().await;
    harness.tick().await;
    assert_eq!(
        harness
            .sink
            .count(EventKind::Adhan, Some(Notifiable::Maghrib)),
        1
    );

    let fired = harness.sink.fired();
    assert_eq!(fired.len(), 2, "exactly two side effects: {fired:?}");
    assert!(
        fired.iter().all(|(_, late)| !late),
        "live ticks fire on time: {fired:?}"
    );
}

// ---- §11.3: jump detection --------------------------------------------------

/// A six-minute wall-only jump (a suspend): the Adhan, 30 s past its
/// instant and inside its five-minute grace, fires once flagged late;
/// the Pre-Adhan, 5½ minutes past and outside its two-minute grace,
/// never fires — a missed row instead (§11.5).
#[tokio::test]
async fn a_short_wall_jump_fires_within_grace_and_misses_the_rest() {
    let mut harness = Harness::start(utc(TODAY, 19, 15, 30)).await;

    harness
        .clock
        .advance_wall_only(chrono::Duration::minutes(6));
    harness.tick().await;

    let fired = harness.sink.fired();
    assert_eq!(fired.len(), 1, "only the in-grace Adhan fires: {fired:?}");
    assert_eq!(fired[0].0.kind, EventKind::Adhan);
    assert_eq!(fired[0].0.prayer, Some(Notifiable::Maghrib));
    assert!(
        fired[0].1,
        "a catch-up fire inside grace carries the late flag"
    );

    let svc = drain(&mut harness.svc);
    assert!(has_missed(
        &svc,
        EventKind::PreNotify,
        Some(Notifiable::Maghrib)
    ));

    let log = logged_on(&harness.store, TODAY).await;
    assert!(
        log.iter()
            .any(|event| event.label == "Maghrib Adhan fired (delayed)"),
        "the §11.5 specimen wording: {log:?}"
    );
    assert!(log.iter().any(
        |event| event.label == "Maghrib Pre-Adhan notification missed"
            && event.detail.as_deref() == Some("app was not running")
    ));
}

/// A six-hour wall-only jump (a suspend through the night): everything
/// past its grace is ledgered and reported missed, nothing fires but
/// the rollover — idempotent by nature, so it refetches and rolls.
#[tokio::test]
async fn a_six_hour_jump_misses_everything_past_grace_and_rolls_the_day() {
    let mut harness = Harness::start(utc(TODAY, 19, 15, 0)).await;

    harness.clock.advance(chrono::Duration::minutes(1));
    harness.tick().await; // the Pre-Adhan fires on time

    harness.clock.advance_wall_only(chrono::Duration::hours(6));
    harness.tick().await;

    let fired = harness.sink.fired();
    assert_eq!(
        fired.len(),
        2,
        "the Pre-Adhan and the rollover only: {fired:?}"
    );
    assert_eq!(fired[0].0.kind, EventKind::PreNotify);
    assert_eq!(fired[1].0.kind, EventKind::DayRollover);
    assert_eq!(
        harness.windows.rollovers(),
        1,
        "the rollover refetched the window"
    );

    let svc = drain(&mut harness.svc);
    for (kind, prayer) in [
        (EventKind::Adhan, Some(Notifiable::Maghrib)),
        (EventKind::MosqueReminder, Some(Notifiable::Maghrib)),
        (EventKind::Adhan, Some(Notifiable::Isha)),
    ] {
        assert!(
            has_missed(&svc, kind, prayer),
            "{kind:?} {prayer:?} must be reported missed"
        );
    }
}

/// A backward clock step replans the Adhan into the future again — but
/// the §11.4 ledger holds it, so passing its instant a second time
/// fires nothing (§11.1: exactly once, whatever the clock does).
#[tokio::test]
async fn a_backward_jump_never_refires() {
    let mut harness = Harness::start(utc(TODAY, 19, 15, 0)).await;

    harness.clock.advance(chrono::Duration::minutes(1));
    harness.tick().await;
    harness.clock.advance(chrono::Duration::minutes(5));
    harness.tick().await;
    assert_eq!(
        harness
            .sink
            .count(EventKind::Adhan, Some(Notifiable::Maghrib)),
        1
    );

    // Two wall hours back, monotonic counter still — a clock step.
    harness
        .clock
        .advance_wall_only(chrono::Duration::minutes(-120));
    harness.tick().await;

    // And forward again, through the Adhan's instant a second time.
    harness
        .clock
        .advance(chrono::Duration::minutes(120) + chrono::Duration::seconds(30));
    harness.tick().await;

    assert_eq!(
        harness
            .sink
            .count(EventKind::Adhan, Some(Notifiable::Maghrib)),
        1,
        "the ledger holds the Adhan across the replan"
    );
    assert_eq!(
        harness
            .sink
            .count(EventKind::PreNotify, Some(Notifiable::Maghrib)),
        1
    );
    let svc = drain(&mut harness.svc);
    assert!(
        !has_missed(&svc, EventKind::PreNotify, Some(Notifiable::Maghrib)),
        "the re-discovered Pre-Adhan is already ledgered — the conflict path \
         skips it silently, neither re-firing nor reporting it missed"
    );
}

// ---- §11.6: the config re-arm and the rollover ------------------------------

/// A configuration publication re-arms the plan: Adhan playback off
/// globally removes the Maghrib Adhan, while the mosque reminder — a
/// separate setting — still fires.
#[tokio::test]
async fn a_config_change_re_arms_the_plan() {
    let harness = Harness::start(utc(TODAY, 19, 15, 0)).await;

    harness.clock.advance(chrono::Duration::minutes(1));
    harness.tick().await; // the Pre-Adhan fires

    let mut cfg = Config::default();
    cfg.adhan.enabled = false;
    harness
        .cfg_tx
        .send(Arc::new(cfg))
        .expect("the runner holds the watch receiver");
    // A tick at an unchanged clock gives the config arm its turn before
    // the decisive advance — whichever arm the select picks first, the
    // re-arm is settled by the next one.
    harness.tick().await;

    harness.clock.advance(chrono::Duration::minutes(5));
    harness.tick().await;
    assert_eq!(
        harness
            .sink
            .count(EventKind::Adhan, Some(Notifiable::Maghrib)),
        0,
        "the re-armed plan holds no Adhan"
    );

    harness.clock.advance(chrono::Duration::minutes(15));
    harness.tick().await;
    assert_eq!(
        harness
            .sink
            .count(EventKind::MosqueReminder, Some(Notifiable::Maghrib)),
        1,
        "the reminder is a separate setting and survives"
    );
}

/// The DayRollover at local midnight: it fires, refetches the window
/// through the rollover seam, and the next day's events come from the
/// rolled window — proven by Sep 6's Maghrib Pre-Adhan firing, dated
/// Sep 6.
#[tokio::test]
async fn the_day_rollover_refetches_and_replans() {
    let harness = Harness::start(utc(TODAY, 23, 59, 0)).await;
    harness.windows.on_rollover_serve(rolled_window());

    harness.tick().await;

    harness.clock.advance(chrono::Duration::seconds(90));
    harness.tick().await;
    assert_eq!(harness.sink.count(EventKind::DayRollover, None), 1);
    assert_eq!(
        harness.windows.rollovers(),
        1,
        "the rollover refetched the window"
    );

    // One more tick adopts the rolled window (the `window_changed`
    // check compares against the plan's source).
    harness.tick().await;

    // Sep 6's Maghrib Pre-Adhan is at 19:19 − 5 = 19:14; from 00:00:30
    // that is 19 h 13 m 30 s.
    harness
        .clock
        .advance(chrono::Duration::seconds(19 * 3600 + 13 * 60 + 30));
    harness.tick().await;

    let pre = harness
        .sink
        .fired()
        .into_iter()
        .find(|(event, _)| event.kind == EventKind::PreNotify)
        .expect("Sep 6's Pre-Adhan fired from the rolled window");
    assert_eq!(pre.0.prayer, Some(Notifiable::Maghrib));
    assert_eq!(
        pre.0.date, TOMORROW,
        "dated Sep 6 — the rolled window's day"
    );
}

/// The §11.3 horizon top-up: when the served window is going stale and
/// the plan's last event (Sep 6 Isha's reminder) is under six hours
/// out, the runner refetches through the refresh seam and replans from
/// the fresh window — proven by Sep 7's Fajr Pre-Adhan firing, dated
/// Sep 7, with the top-up attempted exactly once.
#[tokio::test]
async fn the_horizon_top_up_refetches_before_the_plan_runs_out() {
    let harness = Harness::start(utc(TOMORROW, 15, 30, 0)).await;
    // The stale window serves Sep 4/5/6; the top-up's refetch serves
    // the rolled Sep 5/6/7 window.
    harness.windows.on_refresh_serve(rolled_window());

    harness.tick().await;
    assert_eq!(
        harness.windows.refreshes(),
        1,
        "the plan's last event (Sep 6 Isha's reminder, 21:15) is under six hours out"
    );

    // Through the night: Sep 6's evening prayers pass unattended, the
    // Sep 7 rollover refetches again, and the fresh plan's first event
    // — Sep 7's Fajr Pre-Adhan at 05:11 − 15 (the Fajr default) — fires
    // on the second.
    harness
        .clock
        .advance(chrono::Duration::seconds(13 * 3600 + 26 * 60));
    harness.tick().await;
    harness.tick().await;

    assert_eq!(
        harness.windows.refreshes(),
        1,
        "at most one attempt per civil date"
    );
    let pre = harness
        .sink
        .fired()
        .into_iter()
        .find(|(event, _)| event.kind == EventKind::PreNotify)
        .expect("Sep 7's Pre-Adhan fired from the topped-up plan");
    assert_eq!(pre.0.prayer, Some(Notifiable::Fajr));
    assert_eq!(
        pre.0.date, AFTER_TOMORROW,
        "dated Sep 7 — the refreshed window's new day"
    );
    assert!(!pre.1, "fired on the second");
}

// ---- §11.4: the ledger across a restart -------------------------------------

/// A restart (a second scheduler over the same ledger, at the same
/// instant) re-fires nothing and re-reports nothing missed: the
/// INSERT-then-side-effect row is the sole idempotency mechanism.
#[tokio::test]
async fn the_ledger_survives_a_restart() {
    let store = Arc::new(Store::open_in_memory().await.expect("an in-memory store"));
    let clock = Arc::new(salat::app::clock::MockClock::at(utc(TODAY, 19, 15, 0)));
    let first = Harness::start_on(
        store.clone(),
        clock.clone(),
        MockWindowSource::new(initial_window()),
    )
    .await;

    clock.advance(chrono::Duration::minutes(1));
    first.tick().await;
    clock.advance(chrono::Duration::minutes(5));
    first.tick().await;
    assert_eq!(
        first
            .sink
            .count(EventKind::Adhan, Some(Notifiable::Maghrib)),
        1
    );

    first.handle.shutdown().await;

    // The "restart": same ledger, same window, same instant.
    clock.advance(chrono::Duration::seconds(30));
    let mut second = Harness::start_on(store, clock, MockWindowSource::new(initial_window())).await;
    second.tick().await;

    assert!(
        second.sink.fired().is_empty(),
        "a restart re-fires nothing: {:?}",
        second.sink.fired()
    );
    let svc = drain(&mut second.svc);
    assert!(
        !has_missed(&svc, EventKind::Adhan, Some(Notifiable::Maghrib)),
        "the ledger already holds the Adhan — it is not re-reported as missed"
    );
}

// ---- §11.7: snooze ----------------------------------------------------------

/// A snoozed Adhan re-fires once, three minutes later (§A10), never
/// re-fires on replayed ticks, displaces nothing (the mosque reminder
/// still fires), and leaves the fired-events ledger for the source
/// Adhan untouched — one row for the Adhan, one for the snooze.
#[tokio::test]
async fn a_snooze_fires_once_and_never_enters_the_ledger() {
    let harness = Harness::start(utc(TODAY, 19, 15, 0)).await;

    harness.clock.advance(chrono::Duration::minutes(1));
    harness.tick().await;
    harness.clock.advance(chrono::Duration::minutes(5));
    harness.tick().await;

    let adhan = harness
        .sink
        .fired()
        .into_iter()
        .find(|(event, _)| event.kind == EventKind::Adhan)
        .expect("the Adhan fired")
        .0;
    harness.handle.snooze(adhan).await;
    // A tick at an unchanged clock settles the command before the
    // clock moves — the snooze instant is computed from `now`.
    harness.tick().await;

    harness.clock.advance(chrono::Duration::minutes(3));
    harness.tick().await;
    harness.tick().await;
    assert_eq!(
        harness
            .sink
            .count(EventKind::AdhanSnooze, Some(Notifiable::Maghrib)),
        1,
        "one in-memory re-fire, never a second"
    );

    harness.clock.advance(chrono::Duration::minutes(12));
    harness.tick().await;
    assert_eq!(
        harness
            .sink
            .count(EventKind::MosqueReminder, Some(Notifiable::Maghrib)),
        1,
        "the snooze displaces nothing"
    );

    let log = logged_on(&harness.store, TODAY).await;
    assert_eq!(
        log.iter()
            .filter(|event| event.label == "Maghrib Adhan snooze fired")
            .count(),
        1
    );
    assert_eq!(
        log.iter()
            .filter(|event| event.label == "Maghrib Adhan fired")
            .count(),
        1,
        "the source Adhan keeps its single ledger row"
    );
}

// ---- §11.5: the startup catch-up --------------------------------------------

/// A late start three minutes past the Adhan: inside its five-minute
/// grace, so it fires once — flagged `(delayed)` — before any tick.
#[tokio::test]
async fn a_short_late_start_fires_the_adhan_delayed() {
    let harness = Harness::start(utc(TODAY, 19, 24, 0)).await;

    let fired = harness.sink.fired();
    assert_eq!(fired.len(), 1, "only the in-grace Adhan fires: {fired:?}");
    assert_eq!(fired[0].0.kind, EventKind::Adhan);
    assert_eq!(fired[0].0.prayer, Some(Notifiable::Maghrib));
    assert!(fired[0].1, "three minutes late is a delayed fire");

    let log = logged_on(&harness.store, TODAY).await;
    assert!(
        log.iter()
            .any(|event| event.label == "Maghrib Adhan fired (delayed)"),
        "the §11.5 specimen wording: {log:?}"
    );
}

/// A late start forty minutes past the Adhan: outside every grace, so
/// nothing fires — the ledger row lands and the miss is reported.
#[tokio::test]
async fn a_late_start_past_grace_is_missed_not_fired() {
    let mut harness = Harness::start(utc(TODAY, 20, 1, 0)).await;

    assert!(
        harness.sink.fired().is_empty(),
        "nothing fires: {:?}",
        harness.sink.fired()
    );

    let svc = drain(&mut harness.svc);
    assert!(has_missed(
        &svc,
        EventKind::Adhan,
        Some(Notifiable::Maghrib)
    ));

    let log = logged_on(&harness.store, TODAY).await;
    assert!(log.iter().any(|event| event.label == "Maghrib Adhan missed"
        && event.detail.as_deref() == Some("app was not running")));
}
