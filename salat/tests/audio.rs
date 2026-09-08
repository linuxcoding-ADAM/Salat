//! `salat test-adhan` against a stub `mpv` (§13.5, §23 Phase 10
//! item 8): the exact argv reaches the player, §13.5's print goes to
//! stdout with exit 0, and a `Ctrl-C` mid-playback kills the child —
//! the §13.3 stop ladder observed from outside the process. The stub
//! is a shell script on an isolated `$PATH`, so nothing here depends
//! on (or touches) the host's real players, and the Adhan file is stub
//! bytes in an isolated `$XDG_DATA_HOME` — the same hermeticity the
//! cli.rs harness applies (§5.2, §8.1).

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The stub `mpv`: logs its argv, records its pid, then either exits
/// immediately (the argv test — §13.5's natural end) or lingers until
/// killed (the Ctrl-C test). The `TERM` trap is the proof of §13.3's
/// first rung: a SIGTERM is delivered and *handled* (the log line is
/// written by the trap, after the signal) — a bare SIGKILL first
/// policy could never leave that line.
const STUB: &str = r#"#!/bin/sh
printf '%s\n' "$@" >> "$STUB_LOG"
echo $$ > "$STUB_PID"
trap 'echo TERM >> "$STUB_LOG"; exit 0' TERM
if [ -n "$STUB_LINGER" ]; then
    while :; do sleep 10; done
fi
exit 0
"#;

/// One `salat test-adhan` setup: fresh XDG directories, a stub Adhan
/// file at the §13.4 default path, a stub `mpv` as the only thing on
/// `$PATH`, and the stub's log/pid files placed where the parent (this
/// test) can read them. The environment is per-child — the test's own
/// process environment is never mutated.
struct Harness {
    dir: tempfile::TempDir,
    log: PathBuf,
    pid: PathBuf,
    adhan: PathBuf,
    linger: bool,
}

impl Harness {
    fn new(linger: bool) -> Self {
        let dir = tempfile::tempdir().expect("the harness tempdir");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("the bin directory");

        let stub = bin.join("mpv");
        std::fs::write(&stub, STUB).expect("the stub script");
        set_executable(&stub);

        let adhan_dir = dir.path().join("data").join("salat").join("adhan");
        std::fs::create_dir_all(&adhan_dir).expect("the adhan directory");
        let adhan = adhan_dir.join("abdul-basit.mp3");
        std::fs::write(&adhan, b"ID3 stub adhan bytes").expect("the stub adhan file");

        let log = dir.path().join("argv.log");
        let pid = dir.path().join("stub.pid");
        Self {
            dir,
            log,
            pid,
            adhan,
            linger,
        }
    }

    /// A `salat test-adhan` child, ready to spawn. Built per call: the
    /// environment derives from the tempdir paths only, so every spawn
    /// from one harness is identical.
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_salat"));
        command
            .arg("test-adhan")
            .env("XDG_STATE_HOME", self.dir.path().join("state"))
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"))
            .env("XDG_RUNTIME_DIR", self.dir.path().join("run"))
            .env("PATH", self.dir.path().join("bin"))
            .env("STUB_LOG", &self.log)
            .env("STUB_PID", &self.pid)
            .env_remove("DBUS_SESSION_BUS_ADDRESS");
        if self.linger {
            command.env("STUB_LINGER", "1");
        }
        command
    }
}

fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let mut permissions = std::fs::metadata(path)
        .expect("the stub exists")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("the stub becomes executable");
}

/// §13.2's exact mpv argv, one argument per line in the stub's log:
/// `--no-video --really-quiet --volume=80 <file>` — plus §13.5's print
/// on stdout and exit 0 when the player ends on its own.
#[test]
fn test_adhan_passes_the_exact_argv_and_exits_zero() {
    let harness = Harness::new(false);
    let output = harness
        .command()
        .output()
        .expect("salat test-adhan runs to completion");
    assert_eq!(output.status.code(), Some(0), "the natural end exits 0");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.contains(&format!(
            "Playing {} via mpv at 80% — press Ctrl-C to stop",
            harness.adhan.display()
        )),
        "§13.5's print:\n{stdout}"
    );

    let log = std::fs::read_to_string(&harness.log).expect("the stub logged its argv");
    let argv: Vec<&str> = log.lines().collect();
    assert_eq!(
        argv,
        vec![
            "--no-video",
            "--really-quiet",
            "--volume=80",
            harness.adhan.to_str().expect("a UTF-8 tempdir path"),
        ],
        "the exact §13.2 argv"
    );
}

/// §13.5 step 4: `Ctrl-C` stops the playback through the bounded §13.3
/// ladder and exits 0 — the child is killed and reaped, never orphaned.
#[test]
fn ctrl_c_mid_playback_kills_the_child_and_exits_zero() {
    let harness = Harness::new(true);
    let mut child = harness
        .command()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("salat test-adhan starts");
    let pid = child.id();

    // Read stdout until §13.5's print — proof the playback began.
    // Rust's stdout is line-buffered, so the print flushes on its
    // newline and this read cannot miss it.
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut seen = Vec::new();
    let mut byte = [0u8; 1];
    while !window_contains(&seen, b"Playing") {
        stdout
            .read_exact(&mut byte)
            .expect("salat prints the Playing line");
        seen.push(byte[0]);
    }

    // The stub records its own pid once spawned.
    let stub_pid = read_pid_when_ready(&harness.pid);

    // Ctrl-C to Salat itself — the handler §13.5 step 4 registers.
    let status = Command::new("/bin/kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .expect("/bin/kill signals the child");
    assert!(status.success(), "the SIGINT was delivered");

    let output = child.wait_with_output().expect("salat exits");
    assert_eq!(
        output.status.code(),
        Some(0),
        "Ctrl-C exits 0, not by signal"
    );

    // §13.3's first rung was SIGTERM, not SIGKILL: the stub's trap
    // wrote `TERM` after handling the signal. The argv lines above it
    // are the spawn's; this line only a graceful kill can produce.
    let log = std::fs::read_to_string(&harness.log).expect("the stub logged its end");
    assert!(
        log.lines().any(|line| line == "TERM"),
        "the stop ladder began with SIGTERM:\n{log}"
    );

    // The stub child is gone: Salat reaped it, and no Adhan outlives
    // the command (§13.3). A survivor would have been reparented and
    // stayed visible under /proc.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Path::new("/proc").join(stub_pid.to_string()).exists() {
        assert!(
            Instant::now() < deadline,
            "the stub player (pid {stub_pid}) outlived salat"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Whether the window contains the needle.
fn window_contains(window: &[u8], needle: &[u8]) -> bool {
    window.windows(needle.len()).any(|w| w == needle)
}

/// The stub's pid, polled until the stub has written it.
fn read_pid_when_ready(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && let Ok(pid) = text.trim().parse::<u32>()
        {
            return pid;
        }
        assert!(Instant::now() < deadline, "the stub never recorded its pid");
        std::thread::sleep(Duration::from_millis(50));
    }
}
