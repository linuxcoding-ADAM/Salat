//! The §15.7 boot ladder end to end (§23 Phase 6): a warm cache carries
//! the ladder through all five stages with the network unreachable, and
//! every stage report reaches the callback in order — the sequence the
//! `--full` LOADING screen animates.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use chrono::{Datelike, Duration, TimeZone};
use clap::Parser as _;
use salat::app::boot::{self, BootState};
use salat::app::clock::{Clock, SystemClock};
use salat::cache::{Store, cache_key};
use salat::cli::Cli;
use salat::config::Config;
use salat::domain::{DayMeta, HijriDate, PrayerDay};

/// The environment this test swaps in: an isolated XDG home and a dead
/// proxy, so the ladder runs against the seeded cache and nothing else.
struct IsolatedEnv {
    vars: Vec<(&'static str, Option<OsString>)>,
}

impl IsolatedEnv {
    /// Swap the environment. Every variable is restored on drop.
    ///
    /// # Safety
    ///
    /// This binary holds exactly one test, so no other test thread can
    /// read the environment while it is swapped. `SystemDirs::detect`
    /// and the HTTP stack read these variables only inside the swapped
    /// window — during `boot::run` below.
    unsafe fn swap(state: &tempfile::TempDir) -> Self {
        let vars = [
            ("XDG_CONFIG_HOME", Some(state.path().join("config"))),
            ("XDG_DATA_HOME", Some(state.path().join("data"))),
            ("XDG_STATE_HOME", Some(state.path().to_path_buf())),
            ("XDG_RUNTIME_DIR", Some(state.path().join("run"))),
            // A dead local port: api.aladhan.com is deterministically
            // unreachable, exactly as the cli tests isolate it.
            ("HTTPS_PROXY", Some(PathBuf::from("http://127.0.0.1:9"))),
            ("HTTP_PROXY", Some(PathBuf::from("http://127.0.0.1:9"))),
            ("ALL_PROXY", Some(PathBuf::from("http://127.0.0.1:9"))),
        ];
        let saved = vars
            .iter()
            .map(|(name, _)| (*name, std::env::var_os(name)))
            .collect();
        // SAFETY: the single-test guarantee documented on `swap`.
        unsafe {
            for (name, value) in &vars {
                std::env::set_var(name, value.as_ref().expect("every swap sets a value"));
            }
        }
        Self { vars: saved }
    }
}

impl Drop for IsolatedEnv {
    fn drop(&mut self) {
        // SAFETY: same single-thread guarantee as `swap`.
        unsafe {
            for (name, value) in self.vars.drain(..) {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

/// Seed the default scope's database with the months around today —
/// as a previous run's fetch would have left it — so stage 3 serves
/// from the cache while the API is unreachable.
fn seed_warm_cache(database: &Path) {
    let config = Config::default();
    let zone: chrono_tz::Tz = config
        .location
        .timezone
        .parse()
        .expect("the default config names a real zone");
    // The Clock trait is the only source of "now" (§6.6) — even here,
    // seeding a database, the system clock is asked through it.
    let now = SystemClock.now_in(zone);
    let today = now.date_naive();
    let key = cache_key(&salat::api::params::Params::from_parts(
        &config.location,
        &config.calculation,
    ));

    let seed = tokio::runtime::Runtime::new().expect("the seeding runtime starts");
    // `boot::run` builds its own runtime, so this one must be gone
    // before the ladder runs.
    seed.block_on(async {
        let store = Store::open(database)
            .await
            .expect("open the prayer database");
        let mut months = Vec::new();
        for date in [today - Duration::days(1), today, today + Duration::days(1)] {
            let month = (date.year(), date.month());
            if !months.contains(&month) {
                months.push(month);
            }
        }
        for (year, month) in months {
            store
                .upsert_month(&key, &synth_month(zone, year, month), None, now)
                .await
                .expect("seed the month");
        }
    });
}

/// A synthetic complete month of domain days — the dates are real, the
/// times plausible and strictly ordered; only coverage matters to the
/// ladder's first-window fetch.
fn synth_month(zone: chrono_tz::Tz, year: i32, month: u32) -> Vec<PrayerDay> {
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1).expect("a real month");
    let next_month = if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
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
            let next_day = date.succ_opt().expect("a real next day");
            let next = |h, m| {
                zone.with_ymd_and_hms(next_day.year(), next_day.month(), next_day.day(), h, m, 0)
                    .single()
                    .unwrap_or_else(|| panic!("a real instant after {date}"))
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

/// With a warm cache and the network unreachable, the ladder walks all
/// five stages in order (§15.7), every report is error-free, and the
/// location label is published from the stage-2 report onward
/// (§16.11 item 6 — `starting…` before that).
#[test]
fn the_ladder_reports_all_five_stages_with_a_warm_cache() {
    let state = tempfile::tempdir().expect("tempdir for the isolated XDG home");
    // SAFETY: this is the only test in this binary (see `swap`).
    let _env = unsafe { IsolatedEnv::swap(&state) };

    let database = state.path().join("data").join("salat").join("salat.db");
    std::fs::create_dir_all(database.parent().expect("the data dir has a parent"))
        .expect("create the data dir");
    seed_warm_cache(&database);

    let cli = Cli::try_parse_from(["salat"]).expect("the bare command line parses");
    let mut states: Vec<BootState> = Vec::new();
    let runtime = boot::run(&cli, |report| states.push(report))
        .expect("a warm cache carries the ladder through");

    let stages: Vec<u8> = states.iter().map(|state| state.stage).collect();
    assert_eq!(stages, [0, 1, 2, 3, 4], "every stage reports, in order");
    assert!(
        states.iter().all(|state| state.error.is_none()),
        "no stage failed"
    );

    // §16.11 item 6: `starting…` until the location is resolved, the
    // configured label from the stage-2 report on.
    assert!(
        states[0].location.is_none() && states[1].location.is_none(),
        "before stage 2 the label is unknown"
    );
    for state in &states[2..] {
        assert_eq!(
            state.location.as_deref(),
            Some("Sétif, Algeria"),
            "the default configuration's label publishes from stage 2"
        );
    }

    // The runtime the ladder hands back is the wired one: its window
    // holds the same warm days, served without touching the network.
    let zone: chrono_tz::Tz = runtime
        .config()
        .location
        .timezone
        .parse()
        .expect("the running config names a real zone");
    let today = SystemClock.now_in(zone).date_naive();
    let window = runtime.window();
    assert_eq!(window.today.date, today, "the seeded day is today's");
}
