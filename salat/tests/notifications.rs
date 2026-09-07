//! §23 Phase 9: the notification path end-to-end — the scheduler firing
//! through [`salat::notify::NotifySink`] into a
//! [`salat::notify::RecordingNotifier`], so every gate and specimen is
//! asserted as sent (or not sent) requests, not as composed values. The
//! gates live in `plan_events` (§11.3), so each case here runs the whole
//! loop: the plan, the tick, the catch-up judgement, the ledger, and
//! only then the sink.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use salat::app::clock::{Clock, MockClock};
use salat::cache::Store;
use salat::config::Config;
use salat::domain::{DayMeta, HijriDate, PrayerDay, PrayerWindow};
use salat::notify::{
    NotificationRequest, Notifier, NotifyError, NotifySink, NotifyTimeout, RecordingNotifier,
    Urgency,
};
use salat::scheduler::{Scheduler, SchedulerDeps, SchedulerHandle, WindowSource};
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

/// The §20.3 fixture window's dates: Sep 4/5/6 2026.
const YESTERDAY: NaiveDate = date(2026, 9, 4);
const TODAY: NaiveDate = date(2026, 9, 5);
const TOMORROW: NaiveDate = date(2026, 9, 6);

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
/// order; the scheduler-facing fields (date, zone, times) are what the
/// plan reads, the rest is well-formed filler.
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

/// The §20.3 window: Sep 4/5/6 2026 — today is Sep 5: Fajr 05:09 ·
/// Dhuhr 12:47 · Asr 16:23 · Maghrib 19:21 · Isha 20:47, so with the
/// default offsets the evening under test is Maghrib's Pre-Adhan at
/// 19:16, Adhan at 19:21, reminder at 19:36.
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

// ---- seams ------------------------------------------------------------------

/// A window source frozen on the fixture window — no test here crosses
/// midnight or needs a refetch, so `refresh` and `rollover` are no-ops.
struct FrozenWindowSource {
    window: Mutex<Option<Arc<PrayerWindow>>>,
}

#[async_trait]
impl WindowSource for FrozenWindowSource {
    async fn window(&self) -> Option<Arc<PrayerWindow>> {
        self.window
            .lock()
            .expect("the window source is not poisoned")
            .clone()
    }

    async fn refresh(&self) {}

    async fn rollover(&self) {}
}

// ---- harness ----------------------------------------------------------------

/// One beat of real time for the current-thread runtime to run the
/// parked runner — a yield, never sleep-for-timing.
async fn beat() {
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
}

/// A spawned scheduler whose sink is the production `NotifySink` wired
/// to a `RecordingNotifier` — the exact §23 Phase 9 wiring, with the
/// transport swapped for the test double.
struct Harness {
    clock: Arc<MockClock>,
    notifier: Arc<RecordingNotifier>,
    tick_tx: mpsc::Sender<()>,
    _cfg_tx: watch::Sender<Arc<Config>>,
    _svc: broadcast::Receiver<salat::scheduler::ServiceEvent>,
    _handle: SchedulerHandle,
}

impl Harness {
    /// The default harness: a healthy notifier that records everything.
    async fn start(start: DateTime<Utc>, cfg: Config) -> Harness {
        let notifier = Arc::new(RecordingNotifier::new());
        Self::start_with(start, cfg, notifier).await
    }

    /// The §12.3 failure-path variant: the caller owns the notifier
    /// (e.g. [`RecordingNotifier::failing`]).
    async fn start_with(
        start: DateTime<Utc>,
        cfg: Config,
        notifier: Arc<RecordingNotifier>,
    ) -> Harness {
        let store = Arc::new(Store::open_in_memory().await.expect("an in-memory store"));
        let clock = Arc::new(MockClock::at(start));
        let (cfg_tx, cfg_rx) = watch::channel(Arc::new(cfg));
        let sink = Arc::new(NotifySink::new(
            Arc::clone(&notifier) as Arc<dyn Notifier>,
            cfg_rx.clone(),
        ));
        let (events_tx, svc) = broadcast::channel(64);
        let (tick_tx, tick_rx) = mpsc::channel(16);
        let handle = Scheduler::spawn_on_ticker(
            SchedulerDeps {
                clock: clock.clone(),
                store,
                scope: "default".to_owned(),
                sink,
                windows: Arc::new(FrozenWindowSource {
                    window: Mutex::new(Some(Arc::new(initial_window()))),
                }),
                config: cfg_rx,
                events: events_tx,
            },
            tick_rx,
        )
        .await;
        Harness {
            clock,
            notifier,
            tick_tx,
            _cfg_tx: cfg_tx,
            _svc: svc,
            _handle: handle,
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

    /// The single sent request, when exactly one was.
    fn one_sent(&self) -> NotificationRequest {
        let sent = self.notifier.sent();
        assert_eq!(sent.len(), 1, "exactly one notification: {sent:?}");
        sent.into_iter().next().expect("len checked above")
    }
}

// ---- §23 Phase 9 item 8: the timing -----------------------------------------

/// §23's own case: a 15-minute `before` on a 19:21 prayer sends at
/// 19:06 — the notification fires the moment the plan's instant
/// arrives, and its body names the prayer's own wall clock.
#[tokio::test]
async fn a_fifteen_minute_before_sends_at_19_06() {
    let mut cfg = Config::default();
    cfg.prayers.maghrib.before_minutes = 15;
    let harness = Harness::start(utc(TODAY, 19, 0, 0), cfg).await;

    // 19:00 → 19:06: the re-armed plan's Maghrib Pre-Adhan instant.
    harness
        .clock
        .advance(utc(TODAY, 19, 6, 0) - utc(TODAY, 19, 0, 0));
    harness.tick().await;

    let request = harness.one_sent();
    assert_eq!(request.summary, "Maghrib in 15 minutes");
    assert_eq!(request.body, "19:21 · Sétif, Algeria");
    assert_eq!(request.urgency, Urgency::Normal);
    assert_eq!(request.timeout, NotifyTimeout::Seconds(10));
    assert_eq!(request.category, "event");
}

/// §23's own case: `notify = false` sends nothing for that prayer —
/// the Pre-Adhan is never planned. The reminder is a separate setting
/// and still sends, which is what makes the empty recorder mean
/// "gated", not "broken".
#[tokio::test]
async fn notify_false_sends_no_pre_notify() {
    let mut cfg = Config::default();
    cfg.prayers.maghrib.notify = false;
    let harness = Harness::start(utc(TODAY, 19, 15, 0), cfg).await;

    // 19:15 → 19:16: where the Pre-Adhan would fire.
    harness
        .clock
        .advance(utc(TODAY, 19, 16, 0) - utc(TODAY, 19, 15, 0));
    harness.tick().await;
    assert!(
        harness.notifier.sent().is_empty(),
        "notify = false plans no Pre-Adhan: {:?}",
        harness.notifier.sent()
    );

    // 19:16 → 19:36: the reminder — a separate gate — still sends.
    harness
        .clock
        .advance(utc(TODAY, 19, 36, 0) - utc(TODAY, 19, 16, 0));
    harness.tick().await;
    assert_eq!(
        harness.notifier.sent().len(),
        1,
        "the reminder is gated by mosque_reminder, not notify"
    );
}

/// §23's own case: `notifications.enabled = false` silences all five
/// pre-notifies. Every prayer's Pre-Adhan instant arrives on time, tick
/// by tick, and the recorder stays empty — the master switch unplanned
/// them at plan time.
#[tokio::test]
async fn notifications_enabled_false_silences_all_five() {
    let mut cfg = Config::default();
    cfg.notifications.enabled = false;
    let harness = Harness::start(utc(TODAY, 4, 50, 0), cfg).await;

    // Each step lands exactly on one prayer's Pre-Adhan instant with the
    // default offsets: Fajr 05:09−15, Dhuhr 12:47−10, Asr 16:23−15,
    // Maghrib 19:21−5, Isha 20:47−15.
    for (hour, minute) in [(4, 54), (12, 37), (16, 8), (19, 16), (20, 32)] {
        let now = harness.clock.now_utc();
        harness.clock.advance(utc(TODAY, hour, minute, 0) - now);
        harness.tick().await;
        assert!(
            harness.notifier.sent().is_empty(),
            "nothing may be sent at {hour:02}:{minute:02}: {:?}",
            harness.notifier.sent()
        );
    }
}

/// §23's own case: a delayed fire's body starts with `(delayed)`. A
/// start 90 s past the Pre-Adhan is inside its two-minute grace, so the
/// startup catch-up fires it flagged late — and the prefix is the flag
/// made visible.
#[tokio::test]
async fn a_delayed_fire_prefixes_the_body() {
    let harness = Harness::start(utc(TODAY, 19, 17, 30), Config::default()).await;

    // The catch-up ran before `start` returned — no tick needed.
    let request = harness.one_sent();
    assert_eq!(request.summary, "Maghrib in 5 minutes");
    assert!(
        request.body.starts_with("(delayed) 19:21 · Sétif, Algeria"),
        "the late flag prefixes the body: {}",
        request.body
    );
}

/// §23's own case: a `NotifyError` is logged and does not abort the
/// scheduler. Both of Maghrib's notifications fail to send, and the
/// runner keeps ticking through both — the second recorded request is
/// the proof the first failure killed nothing.
#[tokio::test]
async fn a_notify_error_does_not_abort_the_scheduler() {
    let notifier = Arc::new(RecordingNotifier::failing(NotifyError::SendFailed {
        detail: "daemon wedged".to_string(),
    }));
    let harness = Harness::start_with(utc(TODAY, 19, 15, 0), Config::default(), notifier).await;

    // 19:15 → 19:16: the Pre-Adhan — send #1 fails.
    harness
        .clock
        .advance(utc(TODAY, 19, 16, 0) - utc(TODAY, 19, 15, 0));
    harness.tick().await;
    assert_eq!(harness.notifier.sent().len(), 1);

    // 19:16 → 19:36: past the Adhan (no notification — §12.1) to the
    // reminder. Send #2 is attempted, which is the whole assertion.
    harness
        .clock
        .advance(utc(TODAY, 19, 36, 0) - utc(TODAY, 19, 16, 0));
    harness.tick().await;
    let sent = harness.notifier.sent();
    assert_eq!(
        sent.len(),
        2,
        "the scheduler survived the first failure: {sent:?}"
    );
    assert_eq!(sent[1].summary, "Time to head out");
}

/// §23's own case: bodies contain no unescaped markup. The location
/// label is the only user-controlled string in a body, and `A&B
/// <Sétif>` arrives as entities — no daemon's HTML subset is fed.
#[tokio::test]
async fn bodies_carry_no_unescaped_markup() {
    let mut cfg = Config::default();
    cfg.location.label = "A&B <Sétif>".to_string();
    let harness = Harness::start(utc(TODAY, 19, 15, 0), cfg).await;

    harness
        .clock
        .advance(utc(TODAY, 19, 16, 0) - utc(TODAY, 19, 15, 0));
    harness.tick().await;

    let request = harness.one_sent();
    assert_eq!(request.body, "19:21 · A&amp;B &lt;Sétif&gt;");
    assert!(!request.body.contains('<') && !request.body.contains('>'));
}

// ---- §12.1: the two specimens and the non-notifying kinds --------------------

/// The reminder specimen, verbatim: `Time to head out` / `Maghrib was
/// 15 minutes ago · Iqama soon`, critical urgency, sticky — the user
/// dismisses it, no timeout does.
#[tokio::test]
async fn the_mosque_reminder_composes_the_specimen() {
    let harness = Harness::start(utc(TODAY, 19, 35, 0), Config::default()).await;

    // 19:35 → 19:36: the reminder's instant. The earlier Pre-Adhan
    // (19:16) and Adhan (19:21) are outside their graces at the 19:35
    // start, so this is the day's only send.
    harness
        .clock
        .advance(utc(TODAY, 19, 36, 0) - utc(TODAY, 19, 35, 0));
    harness.tick().await;

    let request = harness.one_sent();
    assert_eq!(request.summary, "Time to head out");
    assert_eq!(request.body, "Maghrib was 15 minutes ago · Iqama soon");
    assert_eq!(request.urgency, Urgency::Critical);
    assert_eq!(request.timeout, NotifyTimeout::Never);
}

/// §12.1: the Adhan sends no desktop notification — its medium is the
/// overlay plus audio (Phase 10), and a third notification would be
/// noise. The Adhan itself fires; the recorder stays empty.
#[tokio::test]
async fn the_adhan_sends_no_notification() {
    // 19:20: the Pre-Adhan (19:16) is already four minutes past —
    // outside its grace, so it is ledgered missed, not sent.
    let harness = Harness::start(utc(TODAY, 19, 20, 0), Config::default()).await;

    // 19:20 → 19:21: the Adhan fires on time through the sink.
    harness
        .clock
        .advance(utc(TODAY, 19, 21, 0) - utc(TODAY, 19, 20, 0));
    harness.tick().await;

    assert!(
        harness.notifier.sent().is_empty(),
        "the Adhan's medium is the overlay, not the desktop: {:?}",
        harness.notifier.sent()
    );
}

/// `mosque_reminder = false` gates the reminder at plan time: the
/// Pre-Adhan still sends — the control that proves the loop is alive —
/// while the reminder's instant passes silently.
#[tokio::test]
async fn mosque_reminder_false_sends_no_reminder() {
    let mut cfg = Config::default();
    cfg.notifications.mosque_reminder = false;
    let harness = Harness::start(utc(TODAY, 19, 15, 0), cfg).await;

    // 19:15 → 19:16: the Pre-Adhan sends — the loop works.
    harness
        .clock
        .advance(utc(TODAY, 19, 16, 0) - utc(TODAY, 19, 15, 0));
    harness.tick().await;
    assert_eq!(harness.notifier.sent().len(), 1);

    // 19:16 → 19:36: the reminder's instant — nothing.
    harness
        .clock
        .advance(utc(TODAY, 19, 36, 0) - utc(TODAY, 19, 16, 0));
    harness.tick().await;
    assert_eq!(
        harness.notifier.sent().len(),
        1,
        "mosque_reminder = false plans no reminder"
    );
}
