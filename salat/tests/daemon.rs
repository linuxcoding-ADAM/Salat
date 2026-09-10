//! `salat daemon` acceptance (§23 Phase 13 item 8): the `--once`
//! catch-up pass, the second-daemon lock handshake, the SIGTERM
//! shutdown budget, and the TUI-beside-daemon arbitration — the last
//! on a real pty, because `salat --full` reads the controlling
//! terminal and cannot run under pipes. All four rows observe the real
//! binary from outside, against a stub `mpv` and a seeded warm cache in
//! isolated XDG directories — the same hermeticity as `tests/audio.rs`
//! and `tests/cli.rs` (§5.2, §8.1).

use std::io::Read as _;
use std::os::fd::FromRawFd as _;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use chrono::{Datelike, Duration as ChronoDuration, TimeZone as _};
use salat::app::clock::{Clock as _, SystemClock};

/// The stub `mpv` (§13.2's spawn target): logs its argv, records its
/// pid, and lingers until killed — so an in-flight Adhan is observable
/// and its death is provable (§13.3's ladder). `$PATH` holds only this
/// stub (plus the copied `sleep` its loop needs), so the probe's
/// verdict is hermetic. The linger sleeps in the *background* and
/// `wait`s: a trapped SIGTERM interrupts the `wait` builtin
/// immediately, where a foreground `sleep` would defer the trap until
/// the sleep finishes — past the ladder's 500 ms grace, and the proof
/// would see the SIGKILL rung instead. (The real `mpv` traps TERM
/// itself, so the production ladder needs no such contortion.)
const STUB_MPV: &str = r#"#!/bin/sh
printf '%s\n' "$@" >> "$STUB_LOG"
echo $$ > "$STUB_PID"
trap 'echo TERM >> "$STUB_LOG"; exit 0' TERM
while :; do sleep 10 & wait $!; done
"#;

/// One daemon setup: fresh XDG directories, a warm cache seeded around
/// today (the default config's zone), a stub Adhan file, and a stub
/// `mpv` as the only thing on `$PATH`. The network points at a dead
/// local port, so nothing depends on (or touches) any network.
struct Harness {
    dir: tempfile::TempDir,
    argv_log: PathBuf,
    stub_pid: PathBuf,
}

impl Harness {
    /// Build the harness and seed the cache with today's Maghrib at
    /// `maghrib_in` from the seeding instant — the one near-term event
    /// the rows time themselves against. The morning prayers sit one to
    /// three hours in the past (their windows are long past §11.5
    /// grace: catch-up rows them as missed, no side effects) and Isha
    /// three hours out, so the plan's only near-term events are
    /// Maghrib's.
    fn new(maghrib_in: ChronoDuration) -> Self {
        let dir = tempfile::tempdir().expect("the harness tempdir");

        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("the bin directory");
        let stub = bin.join("mpv");
        std::fs::write(&stub, STUB_MPV).expect("the stub mpv");
        set_executable(&stub);
        // The stub's linger loop sleeps; the real `sleep` beside it
        // keeps that loop idle instead of busy-spinning on a PATH miss.
        // A failed copy only makes the stub noisy, never wrong.
        let sleep = bin.join("sleep");
        if std::fs::copy("/bin/sleep", &sleep).is_ok() {
            set_executable(&sleep);
        }
        let argv_log = dir.path().join("argv.log");
        let stub_pid = dir.path().join("stub.pid");

        let adhan_dir = dir.path().join("data").join("salat").join("adhan");
        std::fs::create_dir_all(&adhan_dir).expect("the adhan directory");
        std::fs::write(adhan_dir.join("abdul-basit.mp3"), b"ID3 stub adhan bytes")
            .expect("the stub adhan file");

        let db = dir.path().join("data").join("salat").join("salat.db");
        seed_warm_cache(&db, maghrib_in);

        Self {
            dir,
            argv_log,
            stub_pid,
        }
    }

    /// The §4.3 lock file, under the isolated runtime directory.
    fn lock_path(&self) -> PathBuf {
        self.dir
            .path()
            .join("run")
            .join("salat")
            .join("scheduler.lock")
    }

    /// The §18.4 log file — where row 4's TUI writes its observer-mode
    /// line, alongside the daemon's own lines.
    fn log_file(&self) -> PathBuf {
        self.dir
            .path()
            .join("state")
            .join("salat")
            .join("salat.log")
    }

    /// A `salat` child, ready to spawn with the isolated environment.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_salat"));
        command
            .args(args)
            .env("XDG_STATE_HOME", self.dir.path().join("state"))
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"))
            .env("XDG_RUNTIME_DIR", self.dir.path().join("run"))
            .env("PATH", self.dir.path().join("bin"))
            .env("TERM", "xterm-truecolor")
            .env("STUB_LOG", &self.argv_log)
            .env("STUB_PID", &self.stub_pid)
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            // A dead local port: api.aladhan.com is deterministically
            // unreachable, exactly as the cli tests isolate it. With no
            // session bus address and nothing at $XDG_RUNTIME_DIR/bus,
            // a notification send fails fast instead of autolaunching.
            .env("HTTPS_PROXY", "http://127.0.0.1:9")
            .env("HTTP_PROXY", "http://127.0.0.1:9")
            .env("ALL_PROXY", "http://127.0.0.1:9");
        command
    }
}

/// Seed the default scope's database with the months around today —
/// as a previous run's fetch would have left it — so the daemon's boot
/// serves from cache while the API is unreachable, and its §11.5
/// startup catch-up has real events to judge.
fn seed_warm_cache(database: &Path, maghrib_in: ChronoDuration) {
    let config = salat::config::Config::default();
    let zone: chrono_tz::Tz = config
        .location
        .timezone
        .parse()
        .expect("the default config names a real zone");
    // The Clock trait is the only source of "now" (§6.6) — even here,
    // seeding a database, the system clock is asked through it.
    let now = SystemClock.now_in(zone);
    let key = salat::cache::cache_key(&salat::api::params::Params::from_parts(
        &config.location,
        &config.calculation,
    ));

    let seed = tokio::runtime::Runtime::new().expect("the seeding runtime starts");
    // The daemon child builds its own runtime; this one is gone before
    // any child spawns.
    seed.block_on(async move {
        let store = salat::cache::Store::open(database)
            .await
            .expect("open the prayer database");
        let today = now.date_naive();
        let mut months = Vec::new();
        for date in [
            today - ChronoDuration::days(1),
            today,
            today + ChronoDuration::days(1),
        ] {
            let month = (date.year(), date.month());
            if !months.contains(&month) {
                months.push(month);
            }
        }
        for (year, month) in months {
            store
                .upsert_month(
                    &key,
                    &synth_month(zone, year, month, now, maghrib_in),
                    None,
                    now,
                )
                .await
                .expect("seed the month");
        }
    });
}

/// A synthetic complete month of domain days. Today's grid is built
/// around the seeding instant — the morning prayers one to three hours
/// past, Maghrib at `maghrib_in`, Isha three hours out — so the rows
/// control exactly which §11.5 windows are due; every other day keeps
/// the plausible fixed grid so the window stays realistic.
fn synth_month(
    zone: chrono_tz::Tz,
    year: i32,
    month: u32,
    seeded_now: chrono::DateTime<chrono_tz::Tz>,
    maghrib_in: ChronoDuration,
) -> Vec<salat::domain::PrayerDay> {
    use salat::domain::{DayMeta, HijriDate, PrayerDay};

    let today = seeded_now.date_naive();
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1).expect("a real month");
    let next_month = if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .expect("a real following month");

    (0..)
        .map(|offset| first + ChronoDuration::days(offset))
        .take_while(|date| date < &next_month)
        .map(|date| {
            // [fajr, sunrise, dhuhr, asr, maghrib, isha]; today's grid
            // is relative to the seeding instant, ordered like a real
            // day (fajr < sunrise < … < isha).
            let times = if date == today {
                [
                    seeded_now - ChronoDuration::hours(3),
                    seeded_now - ChronoDuration::hours(2),
                    seeded_now - ChronoDuration::minutes(90),
                    seeded_now - ChronoDuration::minutes(30),
                    seeded_now + maghrib_in,
                    seeded_now + ChronoDuration::hours(3),
                ]
            } else {
                let wall = |h, m| {
                    zone.with_ymd_and_hms(date.year(), date.month(), date.day(), h, m, 0)
                        .single()
                        .unwrap_or_else(|| panic!("a real instant on {date}"))
                };
                [
                    wall(5, 15),
                    wall(6, 15),
                    wall(12, 30),
                    wall(16, 0),
                    wall(19, 0),
                    wall(19, 45),
                ]
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
                times,
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

fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = std::fs::metadata(path)
        .expect("the stub exists")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("the stub becomes executable");
}

/// Today's event-log rows — `(label, detail)` pairs, newest first —
/// read directly from the seeded database: the §23 proof surface, what
/// the daemon actually delivered.
fn logged_today(dir: &Path) -> Vec<(String, Option<String>)> {
    let db = dir.join("data").join("salat").join("salat.db");
    let config = salat::config::Config::default();
    let zone: chrono_tz::Tz = config
        .location
        .timezone
        .parse()
        .expect("the default config names a real zone");
    let now = SystemClock.now_in(zone);
    let read = tokio::runtime::Runtime::new().expect("the reading runtime starts");
    read.block_on(async {
        let store = salat::cache::Store::open(&db)
            .await
            .expect("open the seeded database");
        store
            .events_today(zone, now)
            .await
            .expect("read today's event log")
            .into_iter()
            .map(|row| (row.label, row.detail))
            .collect()
    })
}

// ---------------------------------------------------------------------------
// Row 1: `daemon --once` — one catch-up pass, exit 0
// ---------------------------------------------------------------------------

/// §23 Phase 13 item 8, row 1: `daemon --once` performs a catch-up
/// pass — every due event is judged, and the one inside its §11.5
/// grace window (Maghrib's PreNotify at `t − 5`, one minute past due)
/// fires `(delayed)` and lands in the event log — then exits 0. No
/// Adhan plays: the seeded Maghrib is minutes ahead, so nothing due
/// carries audio.
#[test]
fn daemon_once_performs_a_catchup_pass_and_exits_zero() {
    let harness = Harness::new(ChronoDuration::minutes(4));
    let output = harness
        .command(&["daemon", "--once"])
        .output()
        .expect("salat daemon --once runs to completion");
    assert_eq!(
        output.status.code(),
        Some(0),
        "the once pass exits 0\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // The catch-up judged the due events: Maghrib's PreNotify, one
    // minute past due, is inside the 2-minute grace — a `(delayed)`
    // fire, logged with §17.2's specimen detail. (The morning prayers'
    // windows are hours gone: missed rows, no side effects.)
    let rows = logged_today(harness.dir.path());
    assert!(
        rows.iter().any(|(label, detail)| {
            label == "Pre-Adhan notification (delayed)"
                && detail.as_deref() == Some("MAGHRIB · 5 min window")
        }),
        "the in-grace PreNotify fired:\n{rows:?}"
    );
    // No Adhan was due: the stub player never spawned.
    assert!(
        !harness.argv_log.exists(),
        "no Adhan may play on a pass whose only due events are past-due PreNotifies"
    );
}

// ---------------------------------------------------------------------------
// Row 2: a second daemon — the "already running" handshake
// ---------------------------------------------------------------------------

/// §23 Phase 13 item 8, row 2: a second daemon while one runs exits
/// with the clear "already running" message — `E903` naming the holder
/// — exit 1 (runtime, so `Restart=on-failure` may retry it), never a
/// deadlock and never the silent observer fold the TUI makes of the
/// same arbitration.
#[test]
fn a_second_daemon_reports_already_running_and_exits() {
    let harness = Harness::new(ChronoDuration::minutes(4));
    let mut first = harness
        .command(&["daemon"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the first daemon starts");
    // The lock file appearing is the moment the first daemon owns the
    // schedule — the claim writes the holder record before the runner
    // spawns.
    wait_until(15, || harness.lock_path().is_file());
    assert_eq!(
        lock_holder_pid(&harness.lock_path()),
        Some(first.id()),
        "the lock record names the first daemon"
    );

    let output = harness
        .command(&["daemon", "--once"])
        .output()
        .expect("the second daemon runs to completion");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(
        output.status.code(),
        Some(1),
        "the second daemon exits 1 (runtime), not 3 (config)\nstderr:\n{stderr}"
    );
    assert!(stderr.contains("error[E903]"), "the E903 code:\n{stderr}");
    assert!(
        stderr.contains("already running"),
        "the clear message:\n{stderr}"
    );

    // The first daemon is untouched by the collision and still stops
    // cleanly on SIGTERM.
    assert!(stop(&mut first), "the first daemon exits on SIGTERM");
}

// ---------------------------------------------------------------------------
// Row 3: SIGTERM — the two-second budget and the flushed event log
// ---------------------------------------------------------------------------

/// §23 Phase 13 item 8, row 3: `SIGTERM` shuts the daemon down within
/// two seconds of the signal, and the event log's rows all landed —
/// the awaited runner shutdown is the flush, none lost mid-write.
#[test]
fn sigterm_shuts_down_within_two_seconds_and_flushes_the_log() {
    let harness = Harness::new(ChronoDuration::minutes(4));
    let daemon = harness
        .command(&["daemon"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the daemon starts");

    // Wait until the catch-up's delayed fire has actually landed —
    // the row the flush must preserve.
    wait_until(15, || {
        harness.lock_path().is_file()
            && logged_today(harness.dir.path())
                .iter()
                .any(|(label, _)| label == "Pre-Adhan notification (delayed)")
    });

    let delivered = Command::new("/bin/kill")
        .args(["-TERM", &daemon.id().to_string()])
        .status()
        .expect("/bin/kill signals the daemon");
    assert!(delivered.success(), "the SIGTERM was delivered");

    let signaled = Instant::now();
    let output = daemon.wait_with_output().expect("the daemon exits");
    assert_eq!(
        output.status.code(),
        Some(0),
        "SIGTERM is a clean exit 0, not a signal death"
    );
    assert!(
        signaled.elapsed() < Duration::from_secs(2),
        "the shutdown took {} — the §23 budget is 2 s",
        signaled.elapsed().as_secs_f32()
    );

    // The flush: every row the catch-up wrote is still there.
    let rows = logged_today(harness.dir.path());
    assert!(
        rows.iter()
            .any(|(label, _)| label == "Pre-Adhan notification (delayed)"),
        "the catch-up's rows survived the shutdown:\n{rows:?}"
    );
    assert!(!harness.argv_log.exists(), "no Adhan was due in this row");
}

// ---------------------------------------------------------------------------
// Row 4: the TUI beside the daemon — observer mode, exactly one fire
// ---------------------------------------------------------------------------

/// §23 Phase 13 item 8, row 4: a full TUI launched while the daemon
/// runs enters observer mode and fires nothing — exactly one
/// notification per event in the TUI/daemon combination (§23's expected
/// row). The seeded Maghrib Adhan is thirty seconds out, so the proof
/// is time-bounded: the TUI boots first, the daemon then fires the
/// Adhan (one mpv spawn, one event-log row), and the observer
/// contributes nothing.
#[test]
fn a_full_tui_beside_the_daemon_fires_nothing() {
    let harness = Harness::new(ChronoDuration::seconds(30));
    let daemon = harness
        .command(&["daemon"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the daemon starts");
    wait_until(15, || harness.lock_path().is_file());
    assert_eq!(
        lock_holder_pid(&harness.lock_path()),
        Some(daemon.id()),
        "the daemon holds the lock"
    );

    // The TUI on a 120×35 pty — a real terminal, because `--full`
    // reads the controlling terminal directly (§16).
    let mut pty = Pty::open(120, 35);
    let mut command = harness.command(&["--full"]);
    pty.attach(&mut command);
    let mut tui = command.spawn().expect("the TUI starts");

    // Boot completes when the location label paints — the §16.11
    // lesson: wait for the label, never "DASHBOARD". Draining while
    // waiting also keeps the pty buffer from filling.
    let mut seen = Vec::new();
    wait_until(30, || {
        pty.drain(&mut seen);
        window_contains(&seen, b"ALGERIA")
    });
    // §4.3's observer fold, in the TUI's own log: the claim found the
    // daemon's lock and rendered instead of firing.
    let log = std::fs::read_to_string(harness.log_file()).expect("the log file is readable");
    assert!(
        log.contains("observer mode"),
        "the TUI logged its observer fold:\n{log}"
    );

    // The seeded Adhan (thirty seconds from seeding) fires: one spawn,
    // exactly four argv lines (§13.2's argv).
    wait_until(45, || {
        pty.drain(&mut seen);
        harness.argv_log.exists()
    });
    std::thread::sleep(Duration::from_secs(2)); // the duplicate window
    let argv = std::fs::read_to_string(&harness.argv_log).expect("the stub logged its argv");
    assert_eq!(
        argv.lines().count(),
        4,
        "exactly one mpv spawn — the observer TUI fired nothing:\n{argv}"
    );
    let rows = logged_today(harness.dir.path());
    let adhan_fires = rows
        .iter()
        .filter(|(label, _)| label.contains("Adhan fired"))
        .count();
    assert_eq!(adhan_fires, 1, "exactly one Adhan fire:\n{rows:?}");
    // The lock's holder is unchanged — the TUI never re-claimed it.
    assert_eq!(
        lock_holder_pid(&harness.lock_path()),
        Some(daemon.id()),
        "the TUI must not have rewritten the holder record"
    );

    // The observer TUI is still alive (observer mode renders, it does
    // not die) and stops cleanly on SIGTERM.
    assert!(
        tui.try_wait().expect("the TUI is waitable").is_none(),
        "the observer TUI is still running"
    );
    assert!(stop(&mut tui), "the TUI exits on SIGTERM");

    // §21.4's manual row, proven here where it is cheap: stopping the
    // daemon mid-Adhan runs §13.3's ladder — the stub's TERM trap
    // writes its line and the child dies.
    let delivered = Command::new("/bin/kill")
        .args(["-TERM", &daemon.id().to_string()])
        .status()
        .expect("/bin/kill signals the daemon");
    assert!(delivered.success(), "the SIGTERM was delivered");
    let output = daemon.wait_with_output().expect("the daemon exits");
    let log = std::fs::read_to_string(harness.log_file()).unwrap_or_default();
    assert_eq!(
        output.status.code(),
        Some(0),
        "the daemon exits 0 on SIGTERM (not a crash):\nstderr:\n{}\nlog:\n{log}",
        String::from_utf8_lossy(&output.stderr),
    );
    let argv = std::fs::read_to_string(&harness.argv_log).expect("the stub log is readable");
    assert!(
        argv.lines().any(|line| line == "TERM"),
        "the stop ladder began with SIGTERM:\n{argv}\nlog:\n{log}"
    );
    let stub_pid = std::fs::read_to_string(&harness.stub_pid)
        .expect("the stub recorded its pid")
        .trim()
        .parse::<u32>()
        .expect("a numeric pid");
    wait_until(10, || {
        !Path::new("/proc").join(stub_pid.to_string()).exists()
    });
}

/// The pid recorded in the lock file's holder record (`<pid> <rfc3339>`).
fn lock_holder_pid(path: &Path) -> Option<u32> {
    let record = std::fs::read_to_string(path).ok()?;
    record
        .split_whitespace()
        .next()
        .and_then(|pid| pid.parse().ok())
}

// ---------------------------------------------------------------------------
// The pty (row 4's terminal)
// ---------------------------------------------------------------------------

/// A pseudo-terminal: a master the test reads boot output from, and a
/// slave path the TUI child attaches to as its controlling terminal.
/// Sized 120×35 so the full TUI renders at its native breakpoint —
/// the same mechanism `scripts/p11_manual.py`'s `pty.fork` uses,
/// without leaving Rust.
struct Pty {
    master: std::fs::File,
    slave: std::ffi::CString,
}

impl Pty {
    /// Open the pty and set its window size. The master is
    /// non-blocking so `drain` never parks the test.
    fn open(cols: u16, rows: u16) -> Self {
        unsafe {
            let master_fd = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK);
            assert!(master_fd >= 0, "posix_openpt failed: {master_fd}");
            assert_eq!(libc::grantpt(master_fd), 0, "grantpt failed");
            assert_eq!(libc::unlockpt(master_fd), 0, "unlockpt failed");
            let mut name = [0u8; 64];
            assert_eq!(
                libc::ptsname_r(master_fd, name.as_mut_ptr().cast(), name.len()),
                0,
                "ptsname_r failed"
            );
            let slave = std::ffi::CStr::from_ptr(name.as_ptr().cast()).to_owned();
            let size = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            assert_eq!(
                libc::ioctl(master_fd, libc::TIOCSWINSZ, &size),
                0,
                "TIOCSWINSZ failed"
            );
            Self {
                master: std::fs::File::from_raw_fd(master_fd),
                slave,
            }
        }
    }

    /// Wire `command`'s stdin/stdout/stderr to the pty slave and make
    /// it the child's controlling terminal — crossterm's raw mode and
    /// size query need a real tty. The `pre_exec` body only calls
    /// libc: after `fork`, before `exec`, it is async-signal-safe.
    fn attach(&self, command: &mut Command) {
        let slave = self.slave.clone();
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                // Opened without O_NOCTTY by the fresh session leader:
                // the slave becomes the controlling terminal, exactly
                // `pty.fork`'s rule.
                let fd = libc::open(slave.as_ptr(), libc::O_RDWR);
                if fd < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::dup2(fd, 0) == -1 || libc::dup2(fd, 1) == -1 || libc::dup2(fd, 2) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if fd > 2 {
                    libc::close(fd);
                }
                Ok(())
            });
        }
    }

    /// Read whatever the TUI has painted into `buffer`, without
    /// blocking — every wait loop's read step.
    fn drain(&mut self, buffer: &mut Vec<u8>) {
        let mut chunk = [0u8; 4096];
        match self.master.read(&mut chunk) {
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => panic!("the pty read failed: {error}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Poll `probe` every 50 ms until it holds or `seconds` elapse.
fn wait_until(seconds: u64, mut probe: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !probe() {
        assert!(
            Instant::now() < deadline,
            "the condition never held within {seconds} s"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// SIGTERM a child and report whether it exited within 10 s (any
/// status) — `try_wait` reaps, so no zombie is left behind.
fn stop(child: &mut Child) -> bool {
    let delivered = Command::new("/bin/kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    if !delivered {
        return false;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return false,
        }
    }
    false
}

/// Whether the window contains the needle.
fn window_contains(window: &[u8], needle: &[u8]) -> bool {
    window.windows(needle.len()).any(|w| w == needle)
}
