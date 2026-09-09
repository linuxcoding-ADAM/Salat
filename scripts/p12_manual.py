#!/usr/bin/env python3
"""Phase 12's §23 item-9 manual checks, driving the real binary under
a real pty (pyte renders the screen, so assertions read the frame the
user would see, not the escape stream).

Scenarios (run one at a time):
  a_screen   — boot the full app (online, the p11 precedent — a cold
               boot cannot reach the loop without a window), press `4`,
               read the STATUS screen: SERVICES with the ten components,
               the live clock, RECENT EVENTS; then `r` paints
               REFRESHING… into the crumb while the tick arm re-probes,
               and the screen settles back to the live clock.
  b_agree    — the §15.8 agreement row: in the SAME isolated footprint,
               `salat status` (the text surface) and the STATUS screen
               (the TUI surface) report the same ten components with
               the same marks — one SystemStatus, two renderings.
  c_broken   — break the audio subsystem (PATH without any player) and
               watch both surfaces report it honestly (Degraded /
               NO AUDIO PLAYER), and the exit code stay 0 — a warning,
               not a failure.
"""

import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

import fcntl  # noqa: E402
import pyte  # noqa: E402

REPO = "/home/adammila/Projects/Rust_Projects/salat"
SALAT = f"{REPO}/target/debug/salat"
OFFLINE = {
    "HTTPS_PROXY": "http://127.0.0.1:9",
    "HTTP_PROXY": "http://127.0.0.1:9",
    "ALL_PROXY": "http://127.0.0.1:9",
}


class Session:
    """A command on a fresh pty with a steerable window size."""

    def __init__(self, argv, rows, cols, env=None):
        self.pid, self.master = pty.fork()
        if self.pid == 0:  # child
            try:
                os.environ["TERM"] = "xterm-truecolor"
                if env:
                    for key, value in env.items():
                        os.environ[key] = value
                os.execv(argv[0], argv)
            finally:
                os._exit(127)
        fcntl.ioctl(
            self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0)
        )
        self.stream = bytearray()
        self.exit_code = None

    def pump(self, seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if ready:
                try:
                    data = os.read(self.master, 65536)
                except OSError:  # child closed the slave
                    break
                if not data:
                    break
                self.stream.extend(data)

    def wait_for(self, marker, timeout):
        needle = marker.encode() if isinstance(marker, str) else marker
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if needle in self.stream:
                return True
            self.pump(0.1)
        return needle in self.stream

    def send(self, data):
        os.write(self.master, data)

    def wait_exit(self, timeout):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump(0.05)
            pid, status = os.waitpid(self.pid, os.WNOHANG)
            if pid == self.pid:
                if os.WIFEXITED(status):
                    self.exit_code = os.WEXITSTATUS(status)
                elif os.WIFSIGNALED(status):
                    self.exit_code = 128 + os.WTERMSIG(status)
                return self.exit_code
        return None

    def force_exit(self):
        if self.exit_code is None:
            try:
                os.kill(self.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.wait_exit(5)
            try:
                os.waitpid(self.pid, os.WNOHANG)
            except ChildProcessError:
                pass


def isolated_env(state, extra=None):
    env = {
        "HOME": state,
        "XDG_CONFIG_HOME": state + "/config",
        "XDG_DATA_HOME": state + "/data",
        "XDG_STATE_HOME": state,
        "XDG_RUNTIME_DIR": state + "/run",
        "RUST_LOG": "salat=debug",
    }
    if extra:
        env.update(extra)
    return env


def frame_text(screen):
    """The rendered pyte screen as one block (trailing blanks trimmed
    per row, blank rows kept — the shape the eye sees)."""
    rows = []
    for line in screen.display:
        rows.append(line.rstrip())
    return "\n".join(rows)


def scenario_a_screen():
    """Boot, tab to STATUS, read it; press `r`, watch REFRESHING…."""
    state = tempfile.mkdtemp(prefix="p12_a_")
    env = isolated_env(state)
    session = Session([SALAT, "--full"], 30, 100, env)
    try:
        if not session.wait_for("ALGERIA", 20):
            print("FAIL: the dashboard never rendered")
            return False
        session.send(b"4")
        if not session.wait_for("SYSTEM STATUS", 10):
            print("FAIL: `4` did not reach the STATUS screen")
            return False
        session.pump(1.0)
        screen = pyte.Screen(100, 30)
        stream = pyte.ByteStream(screen)
        stream.feed(bytes(session.stream))
        text = frame_text(screen)

        for needle in [
            "SERVICES",
            "Prayer service",
            "Prayer calculation",
            "Location",
            "Timezone",
            "Notifications",
            "Adhan sound",
            "Audio system",
            "Network",
            "Config file",
            "Prayer database",
            "RECENT EVENTS",
        ]:
            if needle not in text:
                print(f"FAIL: STATUS screen is missing {needle!r}:\n{text}")
                return False

        # `r` re-probe: the flag paints REFRESHING… into the crumb
        # while the tick arm awaits the (offline) refresh.
        session.send(b"r")
        if not session.wait_for(b"REFRESHING", 5):
            print("FAIL: `r` never painted REFRESHING…")
            return False
        session.pump(5.0)  # the tick arm awaits the re-probe
        screen = pyte.Screen(100, 30)
        stream = pyte.ByteStream(screen)
        stream.feed(bytes(session.stream))
        settled = frame_text(screen)
        # The flag clears when the re-probe lands: the crumb shows the
        # live clock again, Network back to ONLINE.
        if "REFRESHING" in settled:
            print("FAIL: the refreshing flag never cleared")
            return False
        if "ONLINE" not in settled:
            print(f"FAIL: the re-probe did not report ONLINE:\n{settled[:600]}")
            return False
        print(f"OK: settled after re-probe:\n{settled[:600]}")
        return True
    finally:
        session.force_exit()


def scenario_b_agree():
    """One SystemStatus, two surfaces: the text block and the STATUS
    screen, from the same footprint, agree component by component."""
    state = tempfile.mkdtemp(prefix="p12_b_")
    env = isolated_env(state)

    # The text surface first (it also warms the cache for the TUI run).
    import tomllib  # noqa: F401  (the config default needs no file)

    text = subprocess.run(
        [SALAT, "status"],
        env={**os.environ, **env},
        capture_output=True,
        timeout=120,
    )
    if text.returncode != 0:
        print(f"FAIL: salat status exited {text.returncode}: {text.stderr!r}")
        return False
    block = text.stdout.decode()
    text_marks = {}
    for line in block.splitlines():
        match = re.match(r"^(●|○|✕) (.+?) — (.+)$", line)
        if match:
            text_marks[match.group(2)] = match.group(1)

    # The TUI surface, same footprint. The text run's refresh left a
    # fresh cache, so the TUI boot serves from it without an HTTP
    # attempt — `Network` honestly `Unknown` until probed. Press `r`
    # so both surfaces have actually probed before comparing.
    session = Session([SALAT, "--full"], 30, 100, env)
    try:
        if not session.wait_for("ALGERIA", 20):
            print("FAIL: the dashboard never rendered")
            return False
        session.send(b"4")
        if not session.wait_for("SERVICES", 10):
            print("FAIL: the STATUS screen never rendered")
            return False
        session.send(b"r")
        if not session.wait_for(b"REFRESHING", 5):
            print("FAIL: `r` never painted REFRESHING…")
            return False
        session.wait_for(b"ONLINE", 30)
        session.pump(1.0)
        screen = pyte.Screen(100, 30)
        stream = pyte.ByteStream(screen)
        stream.feed(bytes(session.stream))
        ui_text = frame_text(screen)

        agreed, disagreed = 0, []
        for label, mark in text_marks.items():
            # The screen's row: the mark then the label on one line.
            pattern = re.compile(re.escape(mark) + r"\s+" + re.escape(label))
            if pattern.search(ui_text):
                agreed += 1
            else:
                disagreed.append((label, mark))
        if disagreed:
            print(f"FAIL: surfaces disagree on {disagreed}:\n{ui_text}\n--- text ---\n{block}")
            return False
        print(f"OK: {agreed} components agree across both surfaces")
        return True
    finally:
        session.force_exit()


def scenario_c_broken():
    """Break the audio subsystem (no player on PATH): both surfaces
    report it honestly, and the exit stays 0 — a warning, not a
    failure."""
    state = tempfile.mkdtemp(prefix="p12_c_")
    empty = os.path.join(state, "empty-path")
    os.makedirs(empty, exist_ok=True)
    env = isolated_env(state, {"PATH": empty})

    # The text surface: Audio system is Degraded (✕ never appears — it
    # is Degraded, not Down), exit 0.
    text = subprocess.run(
        [SALAT, "status"],
        env={**os.environ, **env},
        capture_output=True,
        timeout=120,
    )
    block = text.stdout.decode()
    if text.returncode != 0:
        print(f"FAIL: exit {text.returncode} with a merely-broken subsystem")
        return False
    audio = [line for line in block.splitlines() if "Audio system" in line]
    if not audio or "NO AUDIO PLAYER" not in audio[0]:
        print(f"FAIL: the audio row is not honest: {audio}")
        return False

    # The TUI surface reports the same.
    session = Session([SALAT, "--full"], 30, 100, env)
    try:
        if not session.wait_for("ALGERIA", 20):
            print("FAIL: the dashboard never rendered")
            return False
        session.send(b"4")
        if not session.wait_for("SERVICES", 10):
            print("FAIL: the STATUS screen never rendered")
            return False
        session.pump(1.0)
        screen = pyte.Screen(100, 30)
        stream = pyte.ByteStream(screen)
        stream.feed(bytes(session.stream))
        ui_text = frame_text(screen)
        audio_ui = [line for line in ui_text.splitlines() if "Audio system" in line]
        if not audio_ui or "NO AUDIO PLAYER" not in audio_ui[0]:
            print(f"FAIL: the screen's audio row is not honest: {audio_ui}")
            return False
        print(f"OK: broken audio reported on both surfaces, exit 0: {audio[0]}")
        return True
    finally:
        session.force_exit()


SCENARIOS = {
    "a_screen": scenario_a_screen,
    "b_agree": scenario_b_agree,
    "c_broken": scenario_c_broken,
}

if __name__ == "__main__":
    names = sys.argv[1:] or list(SCENARIOS)
    ok = True
    for name in names:
        print(f"=== {name} ===")
        ok = SCENARIOS[name]() and ok
    sys.exit(0 if ok else 1)
