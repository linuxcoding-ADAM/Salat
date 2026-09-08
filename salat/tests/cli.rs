//! CLI acceptance tests (§23 Phase 0 item 8, §20.4): the command surface,
//! the exit codes, and the stdout-or-nothing rule. Each test isolates the
//! binary's whole XDG footprint in tempdirs so a run never touches the
//! developer's real directories (§5.2, §8.1).

use std::io::Read as _;

use assert_cmd::Command;

/// A `salat` invocation with every XDG directory pointed at fresh
/// tempdirs: state (logging), config, data and runtime (§8.1).
fn salat() -> (Command, tempfile::TempDir) {
    let state = tempfile::tempdir().expect("tempdir for XDG_STATE_HOME");
    let mut cmd = Command::cargo_bin("salat").unwrap();
    cmd.env("XDG_STATE_HOME", state.path())
        .env("XDG_CONFIG_HOME", state.path().join("config"))
        .env("XDG_DATA_HOME", state.path().join("data"))
        .env("XDG_RUNTIME_DIR", state.path().join("run"));
    (cmd, state)
}

/// `--help` lists the documented surface (§14.1).
#[test]
fn help_lists_the_documented_surface() {
    let (mut cmd, _state) = salat();
    let output = cmd.arg("--help").assert().success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    for needle in [
        "--full",
        "--once",
        "--config <PATH>",
        "--no-color",
        "-v, --verbose",
        "status",
        "config",
        "test-adhan",
        "daemon",
    ] {
        assert!(stdout.contains(needle), "--help is missing {needle:?}");
    }
    // §14.2: the attribution follows the help text.
    assert!(stdout.contains("Prayer times by AlAdhan"));
}

/// `--version` equals `CARGO_PKG_VERSION`.
#[test]
fn version_matches_package_version() {
    let (mut cmd, _state) = salat();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(format!("salat {}\n", env!("CARGO_PKG_VERSION")));
}

/// A usage error exits `2` — Clap's own code (§14.5).
#[test]
fn unknown_flag_exits_two() {
    let (mut cmd, _state) = salat();
    cmd.arg("--definitely-not-a-flag")
        .assert()
        .failure()
        .code(2);
}

/// Point the HTTP stack at a dead local port so api.aladhan.com is
/// deterministically unreachable — the exit table's failure rows
/// without depending on (or touching) any network.
fn offline(cmd: &mut Command) -> &mut Command {
    cmd.env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
}

/// Every still-unimplemented command path answers with the typed
/// not-implemented error — exit `1` (§14.5 runtime), the `error[E000]`
/// line on stderr, and an empty stdout: a frame or nothing (§14.6).
/// Quick's bare and `--once` paths are real since Phase 5 and have
/// their own exit-table tests below; `config` is real since Phase 1;
/// `--full` is real since Phase 6 — it reads the controlling terminal
/// directly, so it cannot run under a piped test harness and is
/// covered by the pty checks instead; `test-adhan` is real since
/// Phase 10 — its own ladder (§13.5) is tested in `tests/audio.rs`.
#[test]
fn every_command_path_reports_not_implemented() {
    let cases: &[&[&str]] = &[&["status"], &["daemon"], &["daemon", "--once"]];
    for case in cases {
        let (mut cmd, _state) = salat();
        cmd.args(*case)
            .assert()
            .failure()
            .code(1)
            .stderr(predicates::str::contains("error[E000]"))
            .stdout(predicates::str::is_empty());
    }
}

/// §13.5's first ladder step: no Adhan file → the explanation on
/// stdout (not an `error[E000]` block — a missing file is an
/// explanation, not a crash), the expected path, and exit 1.
#[test]
fn test_adhan_missing_file_explains_the_expected_path() {
    let (mut cmd, state) = salat();
    cmd.env_remove("DBUS_SESSION_BUS_ADDRESS")
        .arg("test-adhan")
        .assert()
        .failure()
        .code(1)
        .stdout(predicates::str::contains("No Adhan file found."))
        .stdout(predicates::str::contains(
            state
                .path()
                .join("data/salat/adhan/abdul-basit.mp3")
                .display()
                .to_string(),
        ))
        .stdout(predicates::str::contains(
            "Place an MP3 there (or set adhan.file in the config).",
        ));
}

/// §13.5's second step: a valid file but no player on `$PATH` → the
/// four supported players, each with its caveat, and exit 1. The
/// `PATH` points at an empty directory, so the probe's verdict is
/// hermetic — no dependence on the host's installed players.
#[test]
fn test_adhan_without_a_player_lists_the_four_supported() {
    let (mut cmd, state) = salat();
    let adhan_dir = state.path().join("data/salat/adhan");
    std::fs::create_dir_all(&adhan_dir).expect("the adhan directory");
    std::fs::write(adhan_dir.join("abdul-basit.mp3"), b"ID3 stub adhan bytes")
        .expect("the stub adhan file");
    let empty_path = state.path().join("empty-path");
    std::fs::create_dir_all(&empty_path).expect("the empty PATH directory");
    cmd.env("PATH", &empty_path)
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .arg("test-adhan")
        .assert()
        .failure()
        .code(1)
        .stdout(predicates::str::contains(
            "No supported audio player found.",
        ))
        .stdout(predicates::str::contains("mpv"))
        .stdout(predicates::str::contains("paplay"))
        .stdout(predicates::str::contains("ffplay"))
        .stdout(predicates::str::contains("aplay"))
        .stdout(predicates::str::contains(
            "Install one of these and ensure it is on $PATH.",
        ));
}

/// §14.6's "API failure, no cache" row: `E301`, exit `4`, and nothing
/// on stdout — the frame-or-nothing rule's empty case. The piped
/// (non-TTY) stdout routes here per §14.3.
#[test]
fn piped_once_without_cache_exits_four_with_nothing() {
    let (mut cmd, _state) = salat();
    offline(&mut cmd);
    cmd.arg("--once")
        .assert()
        .failure()
        .code(4)
        .stderr(predicates::str::contains("error[E301]"))
        .stdout(predicates::str::is_empty());
}

/// §14.6 on a pipe: no ANSI escape bytes anywhere — stdout carries a
/// frame or nothing (here, a real frame), diagnostics stay plain. The
/// frame itself is the cached one from the warm-cache test below; its
/// byte-level shape is the golden suite's business (§14.6).
#[tokio::test]
async fn piped_output_contains_no_ansi() {
    let (mut cmd, _state) = salat_with_warm_cache().await;
    offline(&mut cmd);
    let output = cmd.arg("--once").assert().success().get_output().clone();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(!stdout.is_empty(), "stdout must carry a frame or nothing");
    assert!(
        stdout.contains("· cached"),
        "the cached frame carries its marker:\n{stdout}"
    );
    assert!(!stdout.contains('\u{1b}'), "ANSI escape in piped stdout");
    assert!(!stderr.contains('\u{1b}'), "ANSI escape in diagnostics");
    assert!(
        stderr.contains("warning[E302]: api.aladhan.com unreachable, using cached times"),
        "the E302 warning on stderr:\n{stderr}"
    );
}

/// `-v -v` must not break the run: the typed error still prints, and
/// the log file is created under the isolated `XDG_STATE_HOME` (§18.4).
#[test]
fn verbose_run_logs_to_state_dir() {
    let (mut cmd, state) = salat();
    offline(&mut cmd);
    cmd.args(["-v", "-v"])
        .assert()
        .failure()
        .code(4)
        .stderr(predicates::str::contains("error[E301]"));
    let log = state.path().join("salat").join("salat.log");
    assert!(log.is_file(), "log file missing at {}", log.display());
    let mut content = String::new();
    std::fs::File::open(&log)
        .and_then(|mut f| f.read_to_string(&mut content))
        .expect("log file readable");
    assert!(!content.trim().is_empty(), "log file is empty");
}

// ---------------------------------------------------------------------------
// `salat --once` with a warm cache (§14.6's exit table, Phase 5)
// ---------------------------------------------------------------------------

/// A `salat` invocation like [`salat`], but with the prayer database
/// pre-warmed around today (in the default config's zone) — as a
/// previous run's fetch would have left it — so the once path serves
/// from cache while the API is unreachable.
async fn salat_with_warm_cache() -> (Command, tempfile::TempDir) {
    use chrono::{Datelike, Duration};
    use salat::api::params::Params;
    use salat::app::clock::{Clock, SystemClock};
    use salat::cache::{Store, cache_key};

    let state = tempfile::tempdir().expect("tempdir for XDG_STATE_HOME");
    let config = salat::config::Config::default();
    let zone: chrono_tz::Tz = config
        .location
        .timezone
        .parse()
        .expect("the default config names a real zone");
    // The Clock trait is the only source of "now" (§6.6) — even here,
    // seeding a database, the system clock is asked through it.
    let now = SystemClock.now_in(zone);
    let today = now.date_naive();

    let db = state.path().join("data").join("salat").join("salat.db");
    std::fs::create_dir_all(db.parent().expect("the data dir has a parent"))
        .expect("create the data dir");
    let store = Store::open(&db).await.expect("open the prayer database");
    let key = cache_key(&Params::from_parts(&config.location, &config.calculation));

    // Yesterday, today and tomorrow — the window the run needs — may
    // straddle month boundaries, so every distinct month around today
    // is seeded.
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

    let mut cmd = Command::cargo_bin("salat").unwrap();
    cmd.env("XDG_STATE_HOME", state.path())
        .env("XDG_CONFIG_HOME", state.path().join("config"))
        .env("XDG_DATA_HOME", state.path().join("data"))
        .env("XDG_RUNTIME_DIR", state.path().join("run"));
    (cmd, state)
}

/// A synthetic complete month of domain days — the dates are real, the
/// times plausible and strictly ordered; only the window's coverage
/// matters to the once path.
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

// ---------------------------------------------------------------------------
// `salat config` (§8.1, §14.1, §23 Phase 1)
// ---------------------------------------------------------------------------

/// `salat config` prints the four resolution lines with the §8.1 default
/// scope, then the effective TOML, and creates the commented default file
/// on first run (§8.3).
#[test]
fn config_prints_resolution_and_creates_the_commented_default() {
    let (mut cmd, state) = salat();
    let output = cmd.arg("config").assert().success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    assert!(
        stdout.contains(&format!("{:<10} default\n", "scope:")),
        "missing default scope line in:\n{stdout}"
    );
    let config_path = state
        .path()
        .join("config")
        .join("salat")
        .join("config.toml");
    assert!(
        stdout.contains(&format!("{:<10} {}\n", "config:", config_path.display())),
        "missing config line in:\n{stdout}"
    );
    assert!(
        stdout.contains("database:") && stdout.contains("lock:"),
        "missing resource lines in:\n{stdout}"
    );
    // The effective TOML follows the blank line after the resolution.
    assert!(
        stdout.contains("Sétif, Algeria"),
        "no effective TOML in:\n{stdout}"
    );

    // The created file is the fully-commented §8.3 template, byte for byte.
    let written = std::fs::read_to_string(&config_path).expect("default config created");
    assert_eq!(written, salat::config::defaults::default_toml());
}

/// `--config` scopes the database and lock resources to the file (§8.1):
/// the printed scope key and suffixed paths match the library's own
/// computation.
#[test]
fn config_flag_scopes_the_resources() {
    let dir = tempfile::tempdir().expect("tempdir for the alternate config");
    let mecca = dir.path().join("mecca.toml");
    let (mut cmd, _state) = salat();
    cmd.arg("--config")
        .arg(&mecca)
        .arg("config")
        .assert()
        .success();

    let scope = salat::config::scope_for(&mecca);
    // The digest depends on the tempdir's real path; only the `/home/u/`
    // specimen itself hashes to 75b6780d (unit-tested in the library).
    // Here the shape is what matters: sanitized stem + exactly 8 hex.
    let (stem, digest) = scope.rsplit_once('-').expect("stem and digest");
    assert_eq!(stem, "mecca");
    assert_eq!(digest.len(), 8);
    assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    // The file was created by the first invocation, so it canonicalizes.
    let canonical = mecca.canonicalize().expect("mecca.toml exists after run");
    let expected_config_line = format!("{:<10} {}\n", "config:", canonical.display());
    let (mut cmd2, state2) = salat();
    let output = cmd2
        .arg("--config")
        .arg(&mecca)
        .arg("config")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&output.get_output().stdout).into_owned();
    assert!(
        stdout.contains(&format!("{:<10} {scope}\n", "scope:")),
        "{stdout}"
    );
    assert!(stdout.contains(&expected_config_line), "{stdout}");
    // The suffixed database/lock paths follow §8.1's table — resolved
    // against THIS invocation's isolated data/run directories.
    let expected_db = state2
        .path()
        .join("data")
        .join("salat")
        .join(format!("salat-{scope}.db"));
    let expected_lock = state2
        .path()
        .join("run")
        .join("salat")
        .join(format!("scheduler-{scope}.lock"));
    assert!(
        stdout.contains(&format!("{:<10} {}\n", "database:", expected_db.display())),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("{:<10} {}\n", "lock:", expected_lock.display())),
        "{stdout}"
    );
}

/// An invalid configuration exits `3` with `error[E201]`, the offending
/// field named on stderr, and an empty stdout (§14.5, §14.6, §8.4).
#[test]
fn invalid_config_exits_three_with_the_field_named() {
    let (mut cmd, state) = salat();
    let config_dir = state.path().join("config").join("salat");
    std::fs::create_dir_all(&config_dir).expect("config dir");
    std::fs::write(
        config_dir.join("config.toml"),
        "version = 1\n\n[adhan]\nvolume = 150\n",
    )
    .expect("invalid config");
    cmd.arg("config")
        .assert()
        .failure()
        .code(3)
        .stderr(predicates::str::contains("error[E201]"))
        .stderr(predicates::str::contains("adhan.volume"))
        .stdout(predicates::str::is_empty());
}

/// `config --edit` with a well-behaved `$EDITOR` succeeds and validates
/// the edited file by loading it.
#[test]
fn config_edit_runs_the_editor() {
    let (mut cmd, _state) = salat();
    cmd.env("VISUAL", "true")
        .arg("config")
        .arg("--edit")
        .assert()
        .success()
        .stdout(predicates::str::contains("scope:"));
}

/// A failing `$EDITOR` is an E202: exit `3`, the §14.6 specimen on
/// stderr, nothing on stdout.
#[test]
fn config_edit_with_failing_editor_exits_three() {
    let (mut cmd, _state) = salat();
    cmd.env("VISUAL", "false")
        .arg("config")
        .arg("--edit")
        .assert()
        .failure()
        .code(3)
        .stderr(predicates::str::contains("error[E202]"))
        .stderr(predicates::str::contains("cannot write configuration to"))
        .stdout(predicates::str::is_empty());
}
