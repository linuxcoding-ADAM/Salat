//! CLI acceptance tests (§23 Phase 0 item 8, §20.4): the command surface,
//! the exit codes, and the stdout-or-nothing rule. Each test isolates
//! `XDG_STATE_HOME` in a tempdir so the binary's logging never touches the
//! developer's real state directory (§5.2).

use std::io::Read as _;

use assert_cmd::Command;

/// A `salat` invocation with `XDG_STATE_HOME` pointed at a fresh tempdir.
fn salat() -> (Command, tempfile::TempDir) {
    let state = tempfile::tempdir().expect("tempdir for XDG_STATE_HOME");
    let mut cmd = Command::cargo_bin("salat").unwrap();
    cmd.env("XDG_STATE_HOME", state.path());
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

/// Every command path answers with the typed not-implemented error —
/// exit `1` (§14.5 runtime), the `error[E000]` line on stderr, and an
/// empty stdout: a frame or nothing (§14.6). Includes the piped (non-TTY)
/// bare invocation, which §14.3 routes to the `--once` path.
#[test]
fn every_command_path_reports_not_implemented() {
    let cases: &[&[&str]] = &[
        &[], // quick live — captured stdout is not a TTY, so this takes the once path
        &["--full"],
        &["--once"],
        &["status"],
        &["config"],
        &["config", "--edit"],
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
