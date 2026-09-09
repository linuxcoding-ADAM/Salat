//! §23 Phase 12 item 8's rows, through the public API: the Collector's
//! honesty over a hand-built fixture, the STATUS screen's three sizes,
//! the text formatter's pipe discipline, the ERROR screen's §18.2
//! render — and `salat status`'s exit contract (its decision rule is
//! unit-tested in the command; here the *binary* run proves the
//! surface exists and exits cleanly under an isolated XDG footprint).
//!
//! The screens and the collector have their unit tests inside the
//! crate (where the §20.3 fixture lives); these rows pin what the DoD
//! names against the *public* surface: every screen renders at every
//! size, the §18.2 table is reachable through the real ERROR screen,
//! and both status surfaces agree because they read the one struct.

use salat::config::Config;
use salat::domain::{DaySummary, EventKind, Notifiable, PrayerState};
use salat::error::SalatError;
use salat::status::{
    Component, ComponentId, EventCategory, Health, LoggedEvent, NextEvent, SystemStatus,
};
use salat::ui::layout::Breakpoint;
use salat::ui::model::UiModel;
use salat::ui::screens::status as status_screen;
use salat::ui::shell::StatusState;
use salat::ui::theme::{RenderMode, Theme};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// Africa/Algiers — the default config's zone.
const ZONE: chrono_tz::Tz = chrono_tz::Africa::Algiers;

/// A wall-clock instant in the fixture zone.
fn at(hour: u32, minute: u32) -> chrono::DateTime<chrono_tz::Tz> {
    use chrono::TimeZone as _;
    ZONE.with_ymd_and_hms(2026, 9, 5, hour, minute, 0)
        .single()
        .expect("Africa/Algiers has no DST — the fixture instant exists")
}

/// The ten components: the four the collector derives from verified
/// state as `Ok`, the six probes `Unknown` (nothing has probed — the
/// honesty row below pins that this is a claim, never a guess).
fn fixture_status() -> SystemStatus {
    let components = ComponentId::ALL
        .map(|id| match id {
            ComponentId::PrayerCalculation
            | ComponentId::Location
            | ComponentId::Timezone
            | ComponentId::PrayerDatabase => Component::new(id, Health::Ok, "VERIFIED".to_owned()),
            _ => Component::new(id, Health::Unknown, "NOT PROBED".to_owned()),
        })
        .to_vec();
    SystemStatus {
        components,
        next_event: Some(NextEvent {
            kind: EventKind::PreNotify,
            prayer: Some(Notifiable::Maghrib),
            fire_at: at(19, 16),
        }),
        today: DaySummary {
            total: 5,
            passed: 3,
            remaining: 2,
            notifications_sent: 1,
            adhan_plays: 0,
            next_notify: Some(at(19, 16)),
        },
        recent: Vec::new(),
    }
}

/// A `UiModel` carrying the fixture status — the model the render loop
/// builds, with just the fields the STATUS screen reads.
fn fixture_model(status: &SystemStatus) -> UiModel {
    UiModel {
        now: at(17, 38),
        window: fixture_window(),
        states: [PrayerState::Upcoming; 5],
        next: None,
        previous: None,
        countdown: salat::domain::Countdown {
            hours: 0,
            minutes: 0,
            seconds: 0,
            total_secs: 0,
        },
        progress: 0.0,
        summary: status.today,
        status: std::sync::Arc::new(status.clone()),
        config: std::sync::Arc::new(Config::default()),
        draft: None,
        qibla: None,
        breakpoint: Breakpoint::Small,
    }
}

/// The §20.3-shaped window: only the fields the model's types demand;
/// the STATUS screen reads `previous` and the planner's next event
/// through `model.status`, not the window, so a minimal today is
/// enough for these render rows.
fn fixture_window() -> std::sync::Arc<salat::domain::PrayerWindow> {
    use salat::domain::{DayMeta, HijriDate, PrayerDay};
    let day = |hour: u32, minute: u32, hijri: u32| PrayerDay {
        date: chrono::NaiveDate::from_ymd_opt(2026, 9, 5).expect("the fixture date"),
        tz: ZONE,
        times: [
            at(5, 9),
            at(6, 44),
            at(12, 47),
            at(16, 23),
            at(hour, minute),
            at(20, 47),
        ],
        midnight: at(0, 37),
        last_third: at(2, 47),
        hijri: HijriDate {
            day: hijri,
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
    };
    std::sync::Arc::new(salat::domain::PrayerWindow {
        yesterday: None,
        today: day(19, 21, 23),
        tomorrow: day(19, 19, 24),
    })
}

/// The buffer as one string per row — the glyph-by-glyph walk the
/// screen tests use.
fn rendered(status: &SystemStatus, width: u16, height: u16) -> Vec<String> {
    let model = fixture_model(status);
    let state = StatusState::default();
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    status_screen::render(
        &model,
        &state,
        area,
        &mut buf,
        &Theme::for_mode(RenderMode::Truecolor),
        Breakpoint::of(area),
    );
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_owned())
                .collect()
        })
        .collect()
}

/// Item 8's first row: all ten components appear, in the design's
/// order, on the screen at every size and in the text block — the
/// same ten on both surfaces.
#[test]
fn all_ten_components_appear_on_both_surfaces() {
    let status = fixture_status();

    for (width, height) in [(80u16, 24u16), (100, 30), (120, 35)] {
        let text = rendered(&status, width, height).join("\n");
        for component in &status.components {
            assert!(
                text.contains(component.label),
                "{width}×{height} is missing {}",
                component.label
            );
        }
        assert!(text.contains("SERVICES"), "{width}×{height}");
        assert!(text.contains("RECENT EVENTS"), "{width}×{height}");
        assert!(text.contains("no events yet today"), "{width}×{height}");
    }

    let plain = salat::status::render_text(&status, false);
    for (component, line) in status.components.iter().zip(plain.lines()) {
        assert!(line.contains(component.label), "{line}");
    }
}

/// Item 8's honesty row: an unprobed component is `Unknown`, never a
/// claimed `Ok` — and the mark the text surface prints for it is the
/// open circle, not the full one.
#[test]
fn an_unprobed_component_is_unknown_never_ok() {
    let status = fixture_status();
    for component in &status.components {
        match component.id {
            ComponentId::PrayerCalculation
            | ComponentId::Location
            | ComponentId::Timezone
            | ComponentId::PrayerDatabase => {
                assert_eq!(component.health, Health::Ok, "{}", component.label);
            }
            _ => {
                assert_eq!(component.health, Health::Unknown, "{}", component.label);
            }
        }
    }

    // The text surface's mark: `○` for Unknown, `●` for Ok — a probe
    // in flight can never be misread as verified.
    let plain = salat::status::render_text(&status, false);
    let marks: Vec<&str> = plain
        .lines()
        .take(10)
        .map(|l| l.split(' ').next().unwrap_or(""))
        .collect();
    for (mark, component) in marks.iter().zip(status.components.iter()) {
        match component.health {
            Health::Unknown => assert_eq!(*mark, "○", "{}", component.label),
            _ => assert_eq!(*mark, "●", "{}", component.label),
        }
    }
}

/// Item 8's log rows: the placeholder when empty, the rows when not —
/// and TODAY's log-sourced counters ride the same struct.
#[test]
fn the_event_log_renders_rows_or_the_placeholder() {
    let mut status = fixture_status();
    status.recent = vec![
        LoggedEvent {
            at: at(19, 16),
            category: EventCategory::Notification,
            label: "Pre-Adhan notification".to_owned(),
            detail: Some("MAGHRIB · 5 min window".to_owned()),
            prayer: Some(Notifiable::Maghrib),
        },
        LoggedEvent {
            at: at(19, 21),
            category: EventCategory::Adhan,
            label: "Maghrib Adhan fired".to_owned(),
            detail: None,
            prayer: Some(Notifiable::Maghrib),
        },
    ];

    let text = rendered(&status, 100, 30).join("\n");
    assert!(text.contains("Pre-Adhan notification"), "{text}");
    assert!(text.contains("Maghrib Adhan fired"), "{text}");
    assert!(!text.contains("no events yet today"), "{text}");

    let plain = salat::status::render_text(&status, false);
    assert!(plain.contains("Pre-Adhan notification"), "{plain}");
    assert!(!plain.contains("no events yet today"), "{plain}");

    // Empty again: the placeholder, never a blank panel.
    status.recent.clear();
    let empty_text = rendered(&status, 100, 30).join("\n");
    assert!(empty_text.contains("no events yet today"), "{empty_text}");
    let plain = salat::status::render_text(&status, false);
    assert!(plain.contains("no events yet today"), "{plain}");
}

/// Item 8's snapshot row: the STATUS screen at the three sizes —
/// insta-reviewed, the same form the other screens file.
#[test]
fn the_status_screen_snapshots() {
    let status = fixture_status();
    for (width, height) in [(80u16, 24u16), (100, 30), (120, 35)] {
        let text = rendered(&status, width, height).join("\n");
        let mut settings = insta::Settings::clone_current();
        settings.set_snapshot_path(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots"));
        settings.bind(|| insta::assert_snapshot!(format!("status_{width}x{height}"), text));
    }
}

/// The text formatter's §14.6 discipline, at the pipe: zero escape
/// bytes and stable across renders; TODAY's counters and the next
/// event ride the block.
#[test]
fn the_text_block_is_stable_and_ansi_free_when_piped() {
    let status = fixture_status();
    let plain = salat::status::render_text(&status, false);
    assert!(!plain.contains('\x1b'), "escape bytes in piped output");
    assert_eq!(plain, salat::status::render_text(&status, false));
    assert!(plain.contains("total prayers: 5"), "{plain}");
    assert!(plain.contains("notifications sent: 1"), "{plain}");
    assert!(
        plain.contains("next event: Pre-Adhan notification · MAGHRIB at 19:16"),
        "{plain}"
    );
}

/// Item 8's error row: every §18.2 variant renders on the ERROR
/// screen — unique code, title, details, help — and the screen never
/// doubles the `→` prefix `details()` carries. `E001` (the design's
/// specimen) is asserted verbatim by its own row below.
#[test]
fn every_error_variant_renders_on_the_error_screen() {
    use std::collections::HashSet;
    use std::path::PathBuf;

    let variants = vec![
        SalatError::LocationLookupFailed,
        SalatError::InvalidCoordinates,
        SalatError::InvalidMethod,
        SalatError::TimezoneUndetermined,
        SalatError::ConfigInvalid {
            problems: vec!["adhan.volume: must be between 0 and 100".into()],
        },
        SalatError::ConfigWriteFailed {
            path: PathBuf::from("/home/u/.config/salat/config.toml"),
            cause: "permission denied".into(),
        },
        SalatError::PrayerTimesUnavailable,
        SalatError::ApiUnreachable,
        SalatError::ApiMalformed,
        SalatError::ApiRateLimited,
        SalatError::AudioNoPlayer,
        SalatError::AudioNoFile,
        SalatError::AudioPlaybackFailed,
        SalatError::NotificationsUnavailable,
        SalatError::DatabaseFailed,
        SalatError::RuntimeStartFailed {
            cause: "io error".into(),
        },
        SalatError::TerminalIo {
            cause: "inappropriate ioctl for device".into(),
        },
        SalatError::NotImplemented { command: "daemon" },
    ];
    assert_eq!(
        variants.len(),
        18,
        "the §18.2 table plus the transient E000"
    );

    let mut codes = HashSet::new();
    for error in &variants {
        let code = error.code();
        assert!(codes.insert(code), "duplicate code {code}");
        assert!(!error.title().is_empty());
        assert!(!error.details().is_empty());
        assert!(!error.help().is_empty());

        let text = error_text(error, 80, 24);
        assert!(text.contains(&format!("error[{code}]")), "{code}: {text}");
        assert!(text.contains(error.title()), "{code}: {text}");
        assert!(!text.contains("→ →"), "{code} doubled the arrow: {text}");
    }
}

/// Item 8's E001 row: the §18.2 specimen, verbatim through the ERROR
/// screen — code, title, all three detail lines, help, and the [C]
/// COORDINATES button only E001's action set offers.
#[test]
fn e001_is_reproduced_verbatim() {
    let text = error_text(&SalatError::LocationLookupFailed, 80, 24);
    for needle in [
        "error[E001]",
        "LOCATION LOOKUP FAILED",
        "→ HTTP 503 Service Unavailable",
        "→ retried 3× (1s · 2s · 4s)",
        "→ giving up",
        "help: use [C] coordinates or [R] retry",
        "[R] RETRY",
        "[C] COORDINATES",
        "[Q] QUIT",
    ] {
        assert!(text.contains(needle), "missing {needle:?}:\n{text}");
    }
}

/// Item 8's exit row, binary side: `salat status` against a warm
/// cache while the API is unreachable — the refresh degrades to
/// `CACHED`/`OFFLINE (using cache)`, the block still prints on
/// stdout, and the exit is 0: a `Degraded` component is a warning the
/// block itself tells, not a failure exit. The Down→1 half of the
/// rule is pinned by the command's unit test on `exit_for`.
#[tokio::test]
async fn the_status_command_exits_zero_when_degraded() {
    use assert_cmd::Command;
    use chrono::Datelike as _;
    use salat::api::params::Params;
    use salat::app::clock::{Clock, SystemClock};
    use salat::cache::{Store, cache_key};

    // The warm cache, as a previous run's fetch would have left it:
    // yesterday, today and tomorrow seeded around the real clock, in
    // the default config's zone.
    let state = tempfile::tempdir().expect("tempdir for XDG_STATE_HOME");
    let config = Config::default();
    let zone: chrono_tz::Tz = config
        .location
        .timezone
        .parse()
        .expect("the default config names a real zone");
    let now = SystemClock.now_in(zone);
    let today = now.date_naive();

    let db = state.path().join("data").join("salat").join("salat.db");
    std::fs::create_dir_all(db.parent().expect("the data dir has a parent"))
        .expect("create the data dir");
    let store = Store::open(&db).await.expect("open the prayer database");
    let key = cache_key(&Params::from_parts(&config.location, &config.calculation));
    for date in [
        today - chrono::Duration::days(1),
        today,
        today + chrono::Duration::days(1),
    ] {
        store
            .upsert_month(
                &key,
                &synth_month(zone, date.year(), date.month()),
                None,
                now,
            )
            .await
            .expect("seed the month");
    }

    let mut cmd = Command::cargo_bin("salat").unwrap();
    cmd.env("XDG_STATE_HOME", state.path())
        .env("XDG_CONFIG_HOME", state.path().join("config"))
        .env("XDG_DATA_HOME", state.path().join("data"))
        .env("XDG_RUNTIME_DIR", state.path().join("run"))
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .arg("status")
        .assert()
        .success()
        .stdout(predicates::str::contains("Prayer service"))
        .stdout(predicates::str::contains("OFFLINE (using cache)"));
}

/// A synthetic complete month of domain days — the dates are real and
/// the times plausible; only the window's coverage matters here (the
/// same shape `tests/cli.rs` seeds for the once path).
fn synth_month(zone: chrono_tz::Tz, year: i32, month: u32) -> Vec<salat::domain::PrayerDay> {
    use chrono::{Datelike, Duration, NaiveDate, TimeZone};
    use salat::domain::{DayMeta, HijriDate, PrayerDay};

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
                zone.with_ymd_and_hms(date.year(), date.month(), date.day(), h, m, 0)
                    .single()
                    .unwrap_or_else(|| panic!("a real instant on {date}"))
            };
            PrayerDay {
                date,
                tz: zone,
                times: [
                    wall(5, 15),
                    wall(6, 15),
                    wall(12, 30),
                    wall(16, 0),
                    wall(19, 0),
                    wall(19, 45),
                ],
                midnight: wall(0, 30),
                last_third: wall(2, 30),
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

/// The ERROR screen's text at one size — the glyph walk.
fn error_text(error: &SalatError, width: u16, height: u16) -> String {
    use salat::ui::screens::error as error_screen;
    let theme = Theme::for_mode(RenderMode::Truecolor);
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    error_screen::render(error, buf.area, &mut buf, &theme, Breakpoint::Small);
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buf[(x, y)].symbol().to_owned())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// §16.10's 80×24 discipline: the trace truncates to four lines with
/// the cut marked, and the body notes where the full detail went.
#[test]
fn the_error_trace_truncates_at_80x24() {
    let problems: Vec<String> = (1..=8).map(|n| format!("problem {n}")).collect();
    let error = SalatError::ConfigInvalid { problems };
    let text = error_text(&error, 80, 24);
    assert!(text.contains("→ problem 4"), "{text}");
    assert!(!text.contains("→ problem 5"), "{text}");
    assert!(text.contains("salat.log"), "{text}");
}
