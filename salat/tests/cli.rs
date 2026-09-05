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

/// Every not-yet-implemented command path answers with the typed
/// not-implemented error — exit `1` (§14.5 runtime), the `error[E000]`
/// line on stderr, and an empty stdout: a frame or nothing (§14.6).
/// Includes the piped (non-TTY) bare invocation, which §14.3 routes to
/// the `--once` path. `config` is real since Phase 1 (§23).
#[test]
fn every_command_path_reports_not_implemented() {
    let cases: &[&[&str]] = &[
        &[], // quick live — captured stdout is not a TTY, so this takes the once path
        &["--full"],
        &["--once"],
        &["status"],
        &["test-adhan"],
        &["daemon"],
        &["daemon", "--once"],
    ];
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

/// §14.6: piped output carries no ANSI escape bytes. The captured (pipe)
/// stdout of the once path is empty here, which trivially satisfies the
/// rule; the assertion documents it so Phase 5's golden tests inherit the
/// invariant explicitly.
#[test]
fn piped_output_contains_no_ansi() {
    let (mut cmd, _state) = salat();
    let assert = cmd.arg("--once").assert().failure().code(1);
    let output = assert.get_output();
    let stdout_empty = output.stdout.is_empty();
    let stderr_ansi_free = !String::from_utf8_lossy(&output.stderr).contains('\u{1b}');
    assert!(stdout_empty, "stdout must carry a frame or nothing");
    assert!(stderr_ansi_free, "diagnostics must not use ANSI on a pipe");
}

/// `-v -v` must not break the run: the binary still exits with the typed
/// error, and the log file is created under the isolated `XDG_STATE_HOME`
/// (§18.4).
#[test]
fn verbose_run_logs_to_state_dir() {
    let (mut cmd, state) = salat();
    cmd.args(["-v", "-v"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("error[E000]"));
    let log = state.path().join("salat").join("salat.log");
    assert!(log.is_file(), "log file missing at {}", log.display());
    let mut content = String::new();
    std::fs::File::open(&log)
        .and_then(|mut f| f.read_to_string(&mut content))
        .expect("log file readable");
    assert!(!content.trim().is_empty(), "log file is empty");
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
