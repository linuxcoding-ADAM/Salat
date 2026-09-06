//! §23 Phase 4 row: the whole cache and persistence layer against
//! in-memory (and, where reopen/corruption demand a real file,
//! temp-directory) SQLite — no network, no wall clock.
//!
//! Every case in the phase's item-8 list is here: the month round-trip,
//! cache-key invalidation, the fired-events idempotency ledger, the
//! event log's ordering and today filter, completion persistence across
//! a reopen, vacuum's exact delete set, corruption recovery, and the
//! §7.6 property that matters most — **zero HTTP on a warm cache**,
//! asserted against the mock client's call counters.

use std::path::Path;
use std::sync::Arc;

use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use salat::api::params::Params;
use salat::api::{AlAdhanClient, ApiError, MockClient, Revalidated};
use salat::app::clock::MockClock;
use salat::cache::{Store, cache_key};
use salat::config::Config;
use salat::domain::{DayMeta, EventKind, HijriDate, Notifiable, PrayerDay, ScheduledEvent};
use salat::{PrayerTimeService, ServiceError};

/// Africa/Algiers — the default config's zone, UTC+1 with no DST in 2026.
const ZONE: Tz = chrono_tz::Africa::Algiers;

fn params() -> Params {
    Params::from(&Config::default())
}

/// A wall-clock instant in the location's zone.
fn at(date: NaiveDate, h: u32, m: u32) -> DateTime<Tz> {
    ZONE.with_ymd_and_hms(date.year(), date.month(), date.day(), h, m, 0)
        .single()
        .unwrap_or_else(|| panic!("a real instant in {ZONE}: {date} {h:02}:{m:02}"))
}

fn utc(date: NaiveDate, h: u32, m: u32) -> DateTime<Utc> {
    at(date, h, m).with_timezone(&Utc)
}

// ---- fixtures ---------------------------------------------------------------

/// The committed §7.8 September-2026 calendar fixture, mapped to domain
/// days — a real month, thirty days, exactly as the API wrote it.
fn fixture_month() -> Vec<PrayerDay> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/aladhan")
        .join("calendar_ok.json");
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture calendar_ok.json: {e}"));
    let dtos = salat::api::dto::parse_calendar(&body).expect("the fixture parses");
    dtos.iter()
        .map(salat::api::map::to_prayer_day)
        .collect::<Result<_, _>>()
        .expect("every fixture day maps")
}

/// A synthetic complete month of domain days — for seeding dates the
/// fixture does not cover (October 2026, ancient dates for vacuum).
/// The times are plausible and strictly ordered; only the dates matter
/// to the layers under test.
fn synth_month(year: i32, month: u32) -> Vec<PrayerDay> {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("a real month");
    let next_month = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .expect("a real following month");
    (0..)
        .map(|offset| first + Duration::days(offset))
        .take_while(|date| date < &next_month)
        .map(|date| {
            let wall = |h, m| {
                ZONE.with_ymd_and_hms(date.year(), date.month(), date.day(), h, m, 0)
                    .single()
                    .unwrap_or_else(|| panic!("a real instant on {date}"))
            };
            let next = |h, m| {
                ZONE.with_ymd_and_hms(
                    date.succ_opt().unwrap_or(date).year(),
                    date.succ_opt().unwrap_or(date).month(),
                    date.succ_opt().unwrap_or(date).day(),
                    h,
                    m,
                    0,
                )
                .single()
                .unwrap_or_else(|| panic!("a real instant after {date}"))
            };
            PrayerDay {
                date,
                tz: ZONE,
                times: [
                    wall(5, 15),
                    wall(6, 15),
                    wall(12, 30),
                    wall(16, 0),
                    wall(19, 0),
                    wall(19, 45),
                ],
                midnight: next(0, 30),
                last_third: next(2, 30),
                hijri: HijriDate {
                    day: 1,
                    month_number: 1,
                    month_en: "Muḥarram".into(),
                    month_ar: "مُحَرَّم".into(),
                    year: 1448,
                },
                meta: DayMeta {
                    method_id: 12,
                    method_name: "Union Organization Islamic de France".into(),
                    school: 0,
                    lat: 36.19,
                    lng: 5.41,
                    tz_name: "Africa/Algiers".into(),
                },
            }
        })
        .collect()
}

fn adhan(prayer: Notifiable, date: NaiveDate, at: DateTime<Tz>) -> ScheduledEvent {
    ScheduledEvent {
        kind: EventKind::Adhan,
        prayer: Some(prayer),
        date,
        fire_at: at,
    }
}

// ---- item 1: the month round-trip -------------------------------------------

/// Store the whole fixture month once, load every day back, and get the
/// identical domain values — the offline cache's entire promise.
#[tokio::test]
async fn a_month_round_trips_exactly() {
    let store = Store::open_in_memory().await.expect("in-memory store");
    let month = fixture_month();
    let now = at(NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(), 12, 0);

    store
        .upsert_month(&cache_key(&params()), &month, Some("\"etag-1\""), now)
        .await
        .expect("upsert the month");

    let key = cache_key(&params());
    for day in &month {
        let loaded = store
            .load_day(&key, day.date)
            .await
            .expect("load one day")
            .unwrap_or_else(|| panic!("day {} is cached", day.date));
        assert_eq!(loaded, *day, "the round-trip preserves {}", day.date);
    }

    // A date the month does not cover is an honest miss.
    let october = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
    assert!(store.load_day(&key, october).await.expect("miss").is_none());

    // Completeness and the stored ETag, both per month.
    assert!(store.month_complete(&key, 2026, 9).await.expect("complete"));
    assert!(
        !store
            .month_complete(&key, 2026, 10)
            .await
            .expect("incomplete")
    );
    assert_eq!(
        store.month_etag(&key, 2026, 9).await.expect("etag"),
        Some("\"etag-1\"".to_owned())
    );
}

// ---- item 2: config change → key change → miss ------------------------------

/// §17.3 invalidation: a different calculation input is a different key,
/// and the old key's rows are simply not read.
#[tokio::test]
async fn a_config_change_reads_a_different_cache() {
    let store = Store::open_in_memory().await.expect("in-memory store");
    let now = at(NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(), 12, 0);
    let base = params();

    store
        .upsert_month(&cache_key(&base), &fixture_month(), None, now)
        .await
        .expect("upsert under the base key");

    let mut changed = base.clone();
    changed.method = 3;
    assert_ne!(cache_key(&changed), cache_key(&base));

    let miss = store
        .load_day(
            &cache_key(&changed),
            NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
        )
        .await
        .expect("a miss, not an error");
    assert!(miss.is_none(), "the new key sees none of the old rows");
}

// ---- item 3: the idempotency ledger ------------------------------------------

/// §11.4: the first claim lands, the second is refused — the guarantee
/// that a restart can never double-fire an audible event.
#[tokio::test]
async fn an_event_fires_exactly_once() {
    let store = Store::open_in_memory().await.expect("in-memory store");
    let date = NaiveDate::from_ymd_opt(2026, 9, 5).unwrap();
    let event = adhan(Notifiable::Maghrib, date, at(date, 19, 1));

    let first = store
        .try_mark_fired("default", &event, at(date, 19, 1), false)
        .await
        .expect("the first insert");
    assert!(first, "the first caller owes the side effect");

    let second = store
        .try_mark_fired("default", &event, at(date, 19, 1), true)
        .await
        .expect("the conflicting insert");
    assert!(!second, "the second caller must skip it");

    // The same prayer under a different scope (§8.1: `--config`) fires
    // again — scopes do not share a ledger.
    let other_scope = store
        .try_mark_fired("travel", &event, at(date, 19, 1), false)
        .await
        .expect("a different scope's row");
    assert!(other_scope, "scopes are independent");

    // DayRollover is never ledgered (§11.3): every rollover "fires".
    let rollover = ScheduledEvent {
        kind: EventKind::DayRollover,
        prayer: None,
        date,
        fire_at: at(date, 0, 0),
    };
    assert!(
        store
            .try_mark_fired("default", &rollover, at(date, 0, 0), false)
            .await
            .expect("rollovers are idempotent by nature")
    );
}

// ---- item 4: the event log ----------------------------------------------------

/// §6.7: newest first, and only today — "today" in the location's zone,
/// never the host's.
#[tokio::test]
async fn the_event_log_lists_today_newest_first() {
    let store = Store::open_in_memory().await.expect("in-memory store");
    let today = NaiveDate::from_ymd_opt(2026, 9, 5).unwrap();
    let yesterday = NaiveDate::from_ymd_opt(2026, 9, 4).unwrap();

    store
        .append_event(
            at(today, 5, 10),
            "notification",
            "Pre-Adhan notification",
            Some("FAJR · 5 min window"),
            Some("fajr"),
        )
        .await
        .expect("append");
    store
        .append_event(
            at(today, 12, 37),
            "adhan",
            "Adhan playback",
            Some("DHUHR"),
            Some("dhuhr"),
        )
        .await
        .expect("append");
    store
        .append_event(
            at(yesterday, 19, 1),
            "adhan",
            "Yesterday's Adhan",
            None,
            Some("maghrib"),
        )
        .await
        .expect("append");

    let events = store
        .events_today(ZONE, at(today, 20, 0))
        .await
        .expect("today's events");
    assert_eq!(
        events
            .iter()
            .map(|event| event.label.clone())
            .collect::<Vec<_>>(),
        vec![
            "Adhan playback".to_owned(),
            "Pre-Adhan notification".to_owned()
        ],
        "newest first, and yesterday is excluded"
    );
    assert_eq!(events[0].prayer.as_deref(), Some("dhuhr"));
    assert_eq!(events[1].detail.as_deref(), Some("FAJR · 5 min window"));

    // An unknown category is refused rather than stored.
    let refused = store
        .append_event(at(today, 21, 0), "gossip", "Not a category", None, None)
        .await;
    assert!(refused.is_err(), "the schema's categories are a closed set");
}

// ---- item 5: completions survive a reopen ------------------------------------

/// §16.6: `Completed` is a database row, so it survives the process —
/// proven by a second `Store` opening the same file.
#[tokio::test]
async fn completions_survive_a_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("salat.db");
    let date = NaiveDate::from_ymd_opt(2026, 9, 5).unwrap();
    let now = at(date, 12, 0);

    let store = Store::open(&db).await.expect("open the file store");
    assert!(
        store
            .toggle_completion(date, Notifiable::Isha, now)
            .await
            .expect("toggle Isha")
    );
    assert!(
        store
            .toggle_completion(date, Notifiable::Fajr, now)
            .await
            .expect("toggle Fajr")
    );

    // A second toggle untoggles — the user's `[c]` is honest.
    assert!(
        !store
            .toggle_completion(date, Notifiable::Isha, now)
            .await
            .expect("toggle Isha back")
    );
    assert!(
        store
            .toggle_completion(date, Notifiable::Isha, now)
            .await
            .expect("toggle Isha on again")
    );

    let reopened = Store::open(&db).await.expect("reopen the same file");
    let completions = reopened
        .completions_for(date)
        .await
        .expect("read completions");
    assert_eq!(
        completions,
        vec![Notifiable::Fajr, Notifiable::Isha],
        "both rows survived, in canonical order"
    );
}

// ---- item 6: vacuum removes only what it should ------------------------------

/// §17.3: the 60-day rule on every table's own date column, the 7-day
/// grace for a superseded key, and nothing else.
#[tokio::test]
async fn vacuum_removes_only_the_expired_and_the_stale() {
    let store = Store::open_in_memory().await.expect("in-memory store");
    let key = cache_key(&params());
    let now = at(NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(), 12, 0);
    let ancient = NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(); // > 60 days back

    // prayer_days: current-key recent (keep), current-key ancient
    // (expire), other-key fresh (grace), other-key week-stale (expire).
    store
        .upsert_month(&key, &synth_month(2026, 9), None, now)
        .await
        .expect("seed current month");
    store
        .upsert_month(&key, &synth_month(2026, 4)[..1], None, now)
        .await
        .expect("seed an ancient current-key row");
    let other_key = format!("{other},m9", other = cache_key(&params()));
    store
        .upsert_month(&other_key, &synth_month(2026, 9)[..1], None, now)
        .await
        .expect("seed a fresh other-key row");
    store
        .upsert_month(
            &other_key,
            &synth_month(2026, 8)[..1],
            None,
            now - Duration::days(8),
        )
        .await
        .expect("seed a stale other-key row");

    // fired_events and prayer_completions: one ancient, one recent each.
    let recent_event = adhan(
        Notifiable::Fajr,
        NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
        now,
    );
    let ancient_event = adhan(Notifiable::Fajr, ancient, at(ancient, 5, 15));
    for event in [&recent_event, &ancient_event] {
        store
            .try_mark_fired("default", event, now, false)
            .await
            .expect("seed fired_events");
    }
    store
        .toggle_completion(
            NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
            Notifiable::Dhuhr,
            now,
        )
        .await
        .expect("seed a recent completion");
    store
        .toggle_completion(ancient, Notifiable::Dhuhr, at(ancient, 12, 30))
        .await
        .expect("seed an ancient completion");

    // event_log: one ancient, one recent.
    store
        .append_event(now, "state", "recent", None, None)
        .await
        .expect("seed event_log");
    store
        .append_event(now - Duration::days(61), "state", "ancient", None, None)
        .await
        .expect("seed event_log");

    // qibla: current-key fresh (keep), two distinct superseded keys — one
    // 90-days-old (expire on the 60-day rule), one week-stale (expire on
    // the 7-day rule). Distinct keys, because `qibla`'s primary key is
    // the key and a repeat would replace, not add.
    store
        .store_qibla(&key, 106.78, now)
        .await
        .expect("seed qibla");
    let old_key_a = format!("{key}#a");
    let old_key_b = format!("{key}#b");
    store
        .store_qibla(&old_key_a, 100.0, now - Duration::days(90))
        .await
        .expect("seed qibla");
    store
        .store_qibla(&old_key_b, 101.0, now - Duration::days(8))
        .await
        .expect("seed qibla");

    // Expected deletions: 1 ancient prayer_day + 1 stale-key prayer_day +
    // 1 fired_event + 1 completion + 1 event_log row + 2 qibla rows.
    let removed = store.vacuum(now, &key).await.expect("the boot vacuum");
    assert_eq!(removed, 7, "exactly the expired and the stale");

    // What survives is exactly what should.
    assert!(
        store
            .load_day(&key, NaiveDate::from_ymd_opt(2026, 9, 1).unwrap())
            .await
            .expect("survivor")
            .is_some(),
        "current-key recent rows stay"
    );
    assert!(
        store
            .load_day(&other_key, NaiveDate::from_ymd_opt(2026, 9, 1).unwrap())
            .await
            .expect("survivor")
            .is_some(),
        "a fresh other-key row is inside its 7-day grace"
    );
    assert!(
        store
            .load_day(&key, NaiveDate::from_ymd_opt(2026, 4, 1).unwrap())
            .await
            .expect("gone")
            .is_none(),
        "ancient rows are gone"
    );
    assert_eq!(
        store
            .events_today(ZONE, now)
            .await
            .expect("event_log survivors")
            .len(),
        1,
        "only the recent event_log row stays"
    );
    assert_eq!(
        store
            .completions_for(NaiveDate::from_ymd_opt(2026, 9, 5).unwrap())
            .await
            .expect("completion survivors")
            .len(),
        1
    );
    assert_eq!(
        store.load_qibla(&key).await.expect("qibla survivor"),
        Some(106.78),
        "the current key's fresh qibla stays"
    );
    assert!(
        store
            .load_qibla(&old_key_a)
            .await
            .expect("qibla gone")
            .is_none(),
        "the 90-day-old qibla row is gone"
    );
    assert!(
        store
            .load_qibla(&old_key_b)
            .await
            .expect("qibla gone")
            .is_none(),
        "the week-stale qibla row is gone"
    );
}

// ---- item 7: corruption recovery ----------------------------------------------

/// §17.4: a garbage file at the database path never crashes the app —
/// it is renamed aside as evidence and a fresh database takes its place.
#[tokio::test]
async fn a_corrupt_database_is_quarantined_and_recreated() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("salat.db");
    std::fs::write(&db, b"definitely not a sqlite database").expect("write garbage");

    // The store opens, migrates and answers queries.
    let store = Store::open(&db).await.expect("recovery, not a crash");
    let key = cache_key(&params());
    let now = at(NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(), 12, 0);
    store
        .upsert_month(&key, &fixture_month(), None, now)
        .await
        .expect("the fresh database is fully usable");

    // The damaged file survives as `salat.db.corrupt-*` evidence.
    let evidence = std::fs::read_dir(dir.path()).expect("list the directory");
    let names: Vec<String> = evidence
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("salat.db.corrupt-")),
        "the damaged file was renamed aside: {names:?}"
    );
}

// ---- item 8: §7.6 — zero HTTP on a warm cache ---------------------------------

/// The property the whole design orbits: with the months the window
/// needs already cached, serving it performs zero HTTP — proven by the
/// mock client's counters, not by absence of error.
#[tokio::test]
async fn a_warm_cache_serves_the_window_with_zero_http() {
    let store = Store::open_in_memory().await.expect("in-memory store");
    let key = cache_key(&params());
    let now = at(NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(), 12, 0);

    // Warm the cache directly, as a previous run's fetch would have.
    store
        .upsert_month(&key, &fixture_month(), None, now)
        .await
        .expect("warm September");

    let client = Arc::new(MockClient::new()); // nothing scripted: any call would fail loudly
    let service = PrayerTimeService::new(
        Arc::clone(&client) as Arc<dyn AlAdhanClient>,
        Arc::new(store),
        params(),
        Arc::new(MockClock::at(utc(
            NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
            12,
            0,
        ))),
    );

    let window = service
        .window(NaiveDate::from_ymd_opt(2026, 9, 14).unwrap())
        .await
        .expect("the warm cache answers");
    assert_eq!(
        window.today.date,
        NaiveDate::from_ymd_opt(2026, 9, 14).unwrap()
    );
    assert_eq!(
        window.tomorrow.date,
        NaiveDate::from_ymd_opt(2026, 9, 15).unwrap()
    );
    assert_eq!(
        window.yesterday.as_ref().expect("yesterday is cached").date,
        NaiveDate::from_ymd_opt(2026, 9, 13).unwrap()
    );

    assert_eq!(client.calendar_calls(), 0, "no calendar fetch");
    assert_eq!(client.timings_calls(), 0, "no timings fetch");
    assert_eq!(client.qibla_calls(), 0, "no qibla fetch");
    assert_eq!(client.revalidation_calls(), 0, "no revalidation");
}

/// A cold cache fetches exactly once for the month it needs, and the
/// memo holds the window steady after that.
#[tokio::test]
async fn a_cold_cache_fetches_once_and_then_memoizes() {
    let store = Store::open_in_memory().await.expect("in-memory store");

    let client = Arc::new(MockClient::new());
    let dtos = {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/aladhan")
            .join("calendar_ok.json");
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture calendar_ok.json: {e}"));
        salat::api::dto::parse_calendar(&body).expect("the fixture parses")
    };
    client.push_calendar(Ok(dtos));

    let service = PrayerTimeService::new(
        Arc::clone(&client) as Arc<dyn AlAdhanClient>,
        Arc::new(store),
        params(),
        Arc::new(MockClock::at(utc(
            NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
            12,
            0,
        ))),
    );

    let window = service
        .window(NaiveDate::from_ymd_opt(2026, 9, 14).unwrap())
        .await
        .expect("the fetch serves the window");
    assert_eq!(client.calendar_calls(), 1, "one fetch for one month");
    assert_eq!(
        window.today,
        fixture_month()[13],
        "the window carries the fetched month's real times"
    );

    let again = service
        .window(NaiveDate::from_ymd_opt(2026, 9, 14).unwrap())
        .await
        .expect("the memo serves the repeat");
    assert_eq!(client.calendar_calls(), 1, "the memo costs no HTTP");
    assert_eq!(again.today.date, window.today.date);
}

/// §17.3's honest error: offline with nothing cached for the day is
/// `E301` — no guess is substituted.
#[tokio::test]
async fn offline_and_uncached_is_the_honest_e301() {
    let store = Store::open_in_memory().await.expect("in-memory store");
    let client = Arc::new(MockClient::new());
    client.push_calendar(Err(ApiError::Network("connection refused".into())));

    let service = PrayerTimeService::new(
        Arc::clone(&client) as Arc<dyn AlAdhanClient>,
        Arc::new(store),
        params(),
        Arc::new(MockClock::at(utc(
            NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
            12,
            0,
        ))),
    );

    let error = service
        .window(NaiveDate::from_ymd_opt(2026, 9, 14).unwrap())
        .await
        .expect_err("nothing cached, nothing fetchable");
    assert!(
        matches!(error, ServiceError::Unavailable { .. }),
        "{error:?}"
    );
    assert_eq!(error.to_salat_error().code(), "E301");
}

/// A warm month boundary: September 30 needs October too, and with both
/// months cached the window still costs zero HTTP — tomorrow's Fajr is
/// always available offline (§7.6's prefetch rationale).
#[tokio::test]
async fn a_month_boundary_needs_both_months_cached() {
    let store = Store::open_in_memory().await.expect("in-memory store");
    let key = cache_key(&params());
    let now = at(NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(), 12, 0);

    store
        .upsert_month(&key, &fixture_month(), None, now)
        .await
        .expect("warm September");
    store
        .upsert_month(&key, &synth_month(2026, 10), None, now)
        .await
        .expect("warm October");

    let client = Arc::new(MockClient::new()); // nothing scripted: any call would fail loudly
    let service = PrayerTimeService::new(
        Arc::clone(&client) as Arc<dyn AlAdhanClient>,
        Arc::new(store),
        params(),
        Arc::new(MockClock::at(utc(
            NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
            12,
            0,
        ))),
    );

    let window = service
        .window(NaiveDate::from_ymd_opt(2026, 9, 30).unwrap())
        .await
        .expect("both months are cached");
    assert_eq!(client.calendar_calls(), 0, "the boundary costs no HTTP");
    assert_eq!(
        window.today.date,
        NaiveDate::from_ymd_opt(2026, 9, 30).unwrap()
    );
    assert_eq!(
        window.tomorrow.date,
        NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
    );
    assert_eq!(
        window.yesterday.as_ref().expect("yesterday is cached").date,
        NaiveDate::from_ymd_opt(2026, 9, 29).unwrap()
    );
}

/// The §7.5 refresh path: a stored ETag plus a 304 keeps the month and
/// clears the memo, still without an unconditional refetch.
#[tokio::test]
async fn refresh_with_a_304_keeps_the_cached_month() {
    let store = Store::open_in_memory().await.expect("in-memory store");
    let key = cache_key(&params());
    let now = at(NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(), 12, 0);
    store
        .upsert_month(&key, &fixture_month(), Some("\"etag-1\""), now)
        .await
        .expect("warm September with an ETag");

    let client = Arc::new(MockClient::new());
    client.push_revalidation(Ok(Revalidated::NotModified));

    let clock = MockClock::at(utc(NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(), 17, 0));
    let service = PrayerTimeService::new(
        Arc::clone(&client) as Arc<dyn AlAdhanClient>,
        Arc::new(store),
        params(),
        Arc::new(clock),
    );

    let window = service.refresh().await.expect("refresh serves the window");
    assert_eq!(client.revalidation_calls(), 1);
    assert_eq!(client.calendar_calls(), 0, "a 304 never refetches");
    assert_eq!(
        window.today.date,
        NaiveDate::from_ymd_opt(2026, 9, 5).unwrap()
    );
}
