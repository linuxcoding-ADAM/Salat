//! §23 Phase 11 item 8's save-and-propagate rows, through the public
//! API: the draft → apply → save → reload identity, the cache-key
//! verdict the CONFIRM screen reports, and the §7.6 consequence — a
//! location change is a cache miss and a refetch, while everything
//! else stays warm.
//!
//! The screen-level wiring (CONFIRM, E202, the kept draft) is unit-
//! tested in `src/ui/`; the E202/E201 store rows in `config_roundtrip`.

use std::path::Path;
use std::sync::Arc;

use chrono::{Datelike, NaiveDate, Utc};
use salat::PrayerTimeService;
use salat::api::params::Params;
use salat::api::{AlAdhanClient, MockClient};
use salat::app::clock::MockClock;
use salat::cache::{Store, cache_key};
use salat::config::{Config, ConfigDraft, ConfigStore};

fn key(config: &Config) -> String {
    cache_key(&Params::from(config))
}

/// §8.5's whole flow on disk: load → draft → edit → apply → save → a
/// fresh store reads the same edits back, and the watch — the channel
/// the running app's config arm listens on — saw the publication.
#[test]
fn the_settings_flow_saves_edits_and_publishes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let store = ConfigStore::new(&path);
    let loaded = store.load().expect("load creates the defaults");
    let mut rx = store.subscribe();
    rx.borrow_and_update();

    // The TUI's session: a draft off the live config, two edits — one
    // from the LOCATION screen, one from a stepper row.
    let mut draft = ConfigDraft::from(loaded.as_ref());
    draft.working().location.label = "Mecca".to_owned();
    draft.working().location.latitude = 21.42;
    draft.working().location.longitude = 39.83;
    draft.working().location.timezone = "Asia/Riyadh".to_owned();
    draft.working().prayers.maghrib.before_minutes = 10;
    assert!(draft.is_dirty());

    // `Runtime::save_draft`'s body: apply onto a clone of live, save.
    let mut target = (*loaded).clone();
    draft.apply_to(&mut target);
    store.save(&target).expect("the edited config is valid");

    // The watch published — no restart, no poll.
    assert!(rx.has_changed().expect("the sender is alive"));
    rx.borrow_and_update();
    assert_eq!(**rx.borrow(), target);

    // A fresh store (the next boot) reads the same edits back.
    let reloaded = ConfigStore::new(&path).load().expect("reload");
    assert_eq!(*reloaded, target);
    assert_eq!(reloaded.location.label, "Mecca");
    assert_eq!(reloaded.prayers.maghrib.before_minutes, 10);
}

/// The CONFIRM screen's honest verdict, decided here: a location or
/// calculation edit moves the API cache key (`Prayer times
/// recalculated.`), a display or audio edit does not (`Settings
/// applied.`).
#[test]
fn only_calculation_inputs_move_the_cache_key() {
    let base = Config::default();

    let mut moved = base.clone();
    moved.location.latitude = 21.42;
    moved.location.longitude = 39.83;
    assert_ne!(key(&moved), key(&base), "a location change refetches");

    let mut method = base.clone();
    method.calculation.method = 3;
    assert_ne!(key(&method), key(&base), "a method change refetches");

    let mut applied = base.clone();
    applied.adhan.volume = 40;
    applied.display.show_seconds = false;
    applied.prayers.maghrib.before_minutes = 10;
    applied.notifications.enabled = false;
    assert_eq!(
        key(&applied),
        key(&base),
        "volume, display, reminders and offsets reuse the cache"
    );
}

/// §7.6 from the save's side: the same params on a warm cache cost zero
/// HTTP; a saved location change is a different key, so the next window
/// read is a miss and exactly one refetch.
#[tokio::test]
async fn a_location_change_is_a_cache_miss_and_a_refetch() {
    let store = Store::open_in_memory().await.expect("in-memory store");
    let client = Arc::new(MockClient::new());
    let dtos = {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/aladhan")
            .join("calendar_ok.json");
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fixture calendar_ok.json: {e}"));
        salat::api::dto::parse_calendar(&body).expect("the fixture parses")
    };
    let date = NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();
    let clock = || {
        Arc::new(MockClock::at(
            chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 9, 5, 12, 0, 0)
                .single()
                .unwrap(),
        ))
    };

    let base = Config::default();
    client.push_calendar(Ok(dtos.clone()));
    let service = PrayerTimeService::new(
        Arc::clone(&client) as Arc<dyn AlAdhanClient>,
        Arc::new(store),
        Params::from(&base),
        clock(),
    );
    service.window(date).await.expect("the first fetch serves");
    assert_eq!(client.calendar_calls(), 1, "one fetch for one month");

    // The save: a location edit promotes new params onto the service —
    // exactly what `Runtime::apply_config` does after `save_draft`.
    let mut draft = ConfigDraft::from(&base);
    draft.working().location.latitude = 21.42;
    draft.working().location.longitude = 39.83;
    let mut saved = base.clone();
    draft.apply_to(&mut saved);
    assert_ne!(key(&saved), key(&base), "the save moved the key");
    service.set_params(Params::from(&saved));

    // A re-read under the new key is a miss: one refetch.
    client.push_calendar(Ok(dtos));
    service
        .window(date)
        .await
        .expect("the refetched month serves");
    assert_eq!(client.calendar_calls(), 2, "the new key refetched once");

    // And the window is the real month's times, not a stale guess.
    let window = service.window(date).await.expect("the memo serves");
    assert_eq!(
        window.today.date, date,
        "the window carries the refetched month"
    );
    assert_eq!(
        window.today.date.day(),
        14,
        "September 14 came from the fixture"
    );
    assert_eq!(client.calendar_calls(), 2, "warm now — the memo holds");
}
