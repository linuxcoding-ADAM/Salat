#!/usr/bin/env python3
"""Phase 11's §23 item-9 manual checks, driving the real binary under
a real pty (pyte renders the screen, so assertions read the frame the
user would see, not the escape stream).

Scenarios (run one at a time):
  a_search   — change location by SEARCH (Mecca), [S] save, CONFIRM
               shows the real config path + "Prayer times
               recalculated.", and the dashboard's times changed.
  b_coords   — change location by COORDINATES (Istanbul), save, times
               change; then AUTO-DETECT accepts the proposal, save,
               times change.
  c_before   — with a prayer ≤ ~55 min out, set its before_minutes to
               one less than the minutes remaining, save ("Settings
               applied." — the cache key did not move), and watch the
               scheduler fire the Pre-Adhan notification in the same
               process: same PID, no restart, an event_log row and a
               fired_events row in salat.db.
  d_watch    — hand-edit the TOML while the app runs and watch the
               dashboard re-render under the new config (24h → 12h).
"""

import datetime
import fcntl
import os
import pty
import re
import select
import signal
import sqlite3
import struct
import sys
import tempfile
import tomllib
import termios
import time

import pyte  # noqa: E402

REPO = "/home/adammila/Projects/Rust_Projects/salat"
SALAT = f"{REPO}/target/debug/salat"


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
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if marker in self.stream:
                return True
            self.pump(0.1)
        return marker in self.stream

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


def isolated_env(state):
    return {
        # HOME under the state dir so CONFIRM's `~`-shortening applies —
        # the realistic case, and the only way a long path fits the
        # 44-wide panel.
        "HOME": state,
        "XDG_CONFIG_HOME": state + "/config",
        "XDG_DATA_HOME": state + "/data",
        "XDG_STATE_HOME": state,
        "XDG_RUNTIME_DIR": state + "/run",
        # The scheduler's own view, into the per-run log file — the
        # manual checks are the last chance to see it decide live.
        "RUST_LOG": "salat=debug",
    }

ROWS, COLS = 35, 120
PRAYERS = ["Fajr", "Dhuhr", "Asr", "Maghrib", "Isha"]
# §8.3's defaults — where each row's BEFORE stepper starts.
DEFAULT_BEFORE = {"Fajr": 15, "Dhuhr": 10, "Asr": 15, "Maghrib": 5, "Isha": 15}


# A complete, app-written config (the Dublin run's save) — the
# template scenario C patches a probed city into, so the boot happens
# already positioned and the target prayer's events are undischarged
# in the §11.6 ledger.
CONFIG_TEMPLATE = """version = 1

[location]
label = "{label}"
country = "{country}"
latitude = {lat:.5f}
longitude = {lng:.5f}
timezone = "{tz}"
source = "city"

[calculation]
method = 12
school = 0
latitude_adjustment_method = 3
midnight_mode = 0
shafaq = "general"

[calculation.tune]
imsak = 0
fajr = 0
sunrise = 0
dhuhr = 0
asr = 0
maghrib = 0
sunset = 0
isha = 0
midnight = 0

[notifications]
enabled = true
mosque_reminder = true

[adhan]
enabled = true
preset = "abdul-basit"
file = ""
volume = 80

[display]
time_format = "24h"
language = "en"
theme = "midnight-blue"
compact_mode = false
show_seconds = true
countdown_progress = true
color_mode = "auto"

[prayers.fajr]
before_minutes = 15
after_minutes = 20
adhan = true
notify = true

[prayers.dhuhr]
before_minutes = 10
after_minutes = 30
adhan = true
notify = true

[prayers.asr]
before_minutes = 15
after_minutes = 20
adhan = true
notify = true

[prayers.maghrib]
before_minutes = 5
after_minutes = 15
adhan = true
notify = true

[prayers.isha]
before_minutes = 15
after_minutes = 30
adhan = true
notify = true
"""


class App:
    """One salat process on a pty, with a pyte screen behind it."""

    def __init__(self, pre_config=None):
        self.state = tempfile.mkdtemp(prefix="salat-p11-")
        print(f"info: state dir {self.state}")
        if pre_config:
            # Boot straight at a chosen location: write the config the
            # store would otherwise create with the defaults.
            path = self.config_path()
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as f:
                f.write(pre_config)
        self.session = Session(
            [SALAT, "--full"],
            rows=ROWS,
            cols=COLS,
            env=isolated_env(self.state),
        )
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.Stream(self.screen)

    @property
    def pid(self):
        return self.session.pid

    def config_path(self):
        return os.path.join(self.state, "config", "salat", "config.toml")

    def db_path(self):
        return os.path.join(self.state, "data", "salat", "salat.db")

    def pump(self, seconds):
        self.session.pump(seconds)

    def feed_pending(self):
        """Feed anything the pty buffered but pyte has not seen yet.

        pyte.Stream has no telltale for "caught up", so track the
        stream length we last fed and feed just the new tail.
        """
        if not hasattr(self, "_fed"):
            self._fed = 0
        data = bytes(self.session.stream[self._fed :])
        if data:
            self.stream.feed(data.decode("utf-8", "replace"))
        self._fed = len(self.session.stream)

    def rows(self):
        self.feed_pending()
        return list(self.screen.display)

    def frame(self):
        return "\n".join(self.rows())

    def wait_for(self, marker, timeout):
        """Wait until `marker` is visible on the RENDERED screen.

        The raw stream cannot be searched: ratatui/crossterm interleave
        ANSI escapes between cell runs, so a title like GENERAL is
        never contiguous in the bytes. pyte reconstructs the screen, so
        the frame is what we poll.
        """
        if isinstance(marker, bytes):
            marker = marker.decode()
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.pump(0.1)
            if marker in self.frame():
                return True
        return marker in self.frame()

    def send(self, keys):
        self.session.send(keys)

    def quit(self):
        return self.session.wait_exit(5)

    def force_exit(self):
        self.session.force_exit()


def check(label, condition, detail=""):
    status = "PASS" if condition else "FAIL"
    suffix = f" — {detail}" if detail else ""
    print(f"{status}: {label}{suffix}")
    return condition


def hhmm_times(frame):
    """Every HH:MM on screen, top-to-bottom — enough to see a location
    change move the times (some may match by chance; the set as a
    whole does not)."""
    return re.findall(r"\b\d{2}:\d{2}\b", frame)


def next_prayer(app):
    """(name, hhmm) from the NEXT PRAYER panel — the panel's times are
    in the location's timezone."""
    rows = app.rows()
    for i, row in enumerate(rows):
        if "● NEXT PRAYER" in row:
            hhmm = re.search(r"(\d{2}:\d{2})\s*[^0-9:]*$", row)
            for above in rows[max(0, i - 3) : i]:
                m = re.search(r"(FAJR|DHUHR|ASR|MAGHRIB|ISHA)", above)
                if m and "SCHEDULED" in above:
                    return m.group(1).title(), (hhmm.group(1) if hhmm else None)
    return None, None


def header_minutes(frame):
    """The header clock in minutes-of-day — it renders in the location's
    timezone, so it is the right clock to subtract a panel time from."""
    for row in frame.split("\n")[:4]:
        if "SALAT" in row:
            m = re.search(r"(\d{2}):(\d{2}):(\d{2})", row)
            if m:
                return int(m.group(1)) * 60 + int(m.group(2))
    return None


def dashboard(app, timeout=45, marker=b"ALGERIA"):
    """Wait for the booted dashboard, dismiss any live overlay (a
    Maghrib that fires mid-run owns the keys until Enter), and return
    (header_label, name, hhmm) from the NEXT PRAYER panel. The default
    marker is the Sétif default's label; a pre-positioned boot passes
    its own."""
    assert app.wait_for(marker, timeout), "boot did not reach the shell"
    app.pump(1.0)
    dismiss_overlays(app)
    name, hhmm = next_prayer(app)
    label = None
    for row in app.rows()[:4]:
        if "ALGERIA" in row:
            label = row.strip()
            break
    return label, name, hhmm


def dismiss_overlays(app):
    """An overlay (Adhan alert, mosque reminder) swallows every key but
    its own three — clear them before driving the shell."""
    for _ in range(4):
        frame = app.frame()
        if "PRAYER TIME" not in frame and "MOSQUE REMINDER" not in frame:
            return
        app.send(b"\r")  # DISMISS
        app.pump(0.6)


def open_location(app):
    """Dashboard → SETTINGS → (Location row) → LOCATION."""
    app.send(b"2")
    if not app.wait_for(b"GENERAL", 5):
        print("---- frame after '2' ----")
        print(app.frame())
        raise AssertionError("SETTINGS did not open")
    app.send(b"\r")
    assert app.wait_for(b"CURRENT LOCATION", 5), "LOCATION did not open"


def save_and_confirm(app, expect_recalculated, back=b"GENERAL"):
    """Press [S], then assert CONFIRM's path line and verdict. CONFIRM
    pops to the screen the save was pressed from — `back` is its
    marker."""
    app.send(b"s")
    assert app.wait_for(b"SETTINGS SAVED", 5), "CONFIRM did not open"
    app.pump(0.5)
    frame = app.frame()
    # The child's HOME is the state dir, so the design's $HOME→~
    # shortening yields exactly this (and fits the 44-wide panel).
    path = app.config_path()
    shortened = "~" + path[len(app.state):] if path.startswith(app.state) else None
    shown = bool(shortened) and shortened in frame
    if not shown and shortened:
        print("---- CONFIRM frame ----")
        print(frame)
    ok = check(
        "CONFIRM shows the real config path",
        path in frame or shown,
        f"looked for {shortened}",
    )
    wanted = (
        "Prayer times recalculated."
        if expect_recalculated
        else "Settings applied."
    )
    ok &= check(f"CONFIRM's verdict is {wanted!r}", wanted in frame)
    app.send(b"\r")  # ENTER CONTINUE → back to the saved-from screen
    assert app.wait_for(back, 5), "ENTER did not leave CONFIRM"
    app.send(b"1")  # back to the dashboard
    app.pump(1.5)
    return ok


def to_dashboard(app):
    """Esc out of whatever screen is up, back to TODAY."""
    for _ in range(6):
        rows = app.rows()
        if any("● NEXT PRAYER" in row or "NEXT PRAYER" in row for row in rows) and not any(
            "CURRENT LOCATION" in row or "PER-PRAYER TIMING" in row or "SETTINGS SAVED" in row
            for row in rows
        ):
            return True
        app.send(b"\x1b")
        app.pump(0.4)
    return False


# ---------------------------------------------------------------- A
def scenario_a():
    app = App()
    try:
        label, name, hhmm = dashboard(app)
        check("boot reaches the dashboard (Sétif)", name is not None and hhmm is not None,
              f"name={name} hhmm={hhmm}")
        before = hhmm_times(app.frame())

        open_location(app)
        # GeoNames carries the city under its official name "Makkah".
        app.send(b"Makkah")
        assert app.wait_for("Makkah, Saudi Arabia", 5), "search did not find Makkah"
        app.send(b"\r")  # commit → back to SETTINGS
        if not app.wait_for(b"GENERAL", 5):
            print(f"---- frame after commit (alive={app.session.exit_code is None}) ----")
            print(app.frame())
            raise AssertionError("commit did not return to SETTINGS")

        save_and_confirm(app, expect_recalculated=True)

        check("the header shows the new location", app.wait_for("MAKKAH", 20))
        after = hhmm_times(app.frame())
        check(
            "prayer times changed with the location (search)",
            set(after) != set(before),
            f"before={sorted(set(before))} after={sorted(set(after))}",
        )
        # The saved config is on disk with the new location.
        with open(app.config_path()) as f:
            body = f.read()
        check("the saved TOML carries Makkah", 'label = "Makkah"' in body)
        check("the process is the same one that booted", app.pid == app.session.pid)
    finally:
        app.force_exit()


# ---------------------------------------------------------------- B
def scenario_b():
    app = App()
    try:
        dashboard(app)
        before = hhmm_times(app.frame())

        # COORDINATES: Istanbul's 41.01 N, 28.98 E.
        open_location(app)
        app.send(b"\x1b[C")  # → COORDINATES
        assert app.wait_for(b"LAT", 5), "COORDINATES did not open"
        check(
            "the panel announces the TAB field switch",
            app.wait_for(b"tab switches field", 5),
        )
        app.send(b"41.01")
        app.send(b"\t")
        app.send(b"28.98")
        app.send(b"\r")  # commit → SETTINGS
        assert app.wait_for(b"GENERAL", 5), "coordinate commit did not return"
        save_and_confirm(app, expect_recalculated=True)
        # A coordinate commit's label is the coordinates themselves.
        check(
            "the header shows the coordinates after the commit",
            app.wait_for("41.01, 28.98", 20),
        )
        after = hhmm_times(app.frame())
        check(
            "prayer times changed with the location (coordinates)",
            set(after) != set(before),
            f"before={sorted(set(before))} after={sorted(set(after))}",
        )

        # AUTO-DETECT: accept the proposal, save, times move again.
        before = after
        open_location(app)
        # The mode persists across visits (it is shell state, not
        # per-visit), so cycle right until AUTO's panel is up.
        saw = False
        for _ in range(3):
            app.send(b"\x1b[C")
            if app.wait_for("enter to accept", 3):
                saw = True
                break
        check("AUTO-DETECT computed a proposal", saw)
        app.send(b"\r")
        assert app.wait_for(b"GENERAL", 5), "auto commit did not return"
        save_and_confirm(app, expect_recalculated=True)
        app.pump(2.0)
        after = hhmm_times(app.frame())
        check(
            "prayer times changed with the location (auto-detect)",
            set(after) != set(before),
            f"before={sorted(set(before))} after={sorted(set(after))}",
        )
    finally:
        app.force_exit()


# ---------------------------------------------------------------- C
# Cities spread across the longitudes, so whatever the hour one of
# them has a prayer in the live-fire window. (name, country, lat, lng)
CANDIDATES = [
    ("Honolulu", "United States", 21.31, -157.86),
    ("Anchorage", "United States", 61.22, -149.90),
    ("Mexico City", "Mexico", 19.43, -99.13),
    ("New York", "United States", 40.71, -74.01),
    ("Sao Paulo", "Brazil", -23.55, -46.63),
    ("Buenos Aires", "Argentina", -34.60, -58.38),
    ("Reykjavik", "Iceland", 64.15, -21.94),
    ("Dublin", "Ireland", 53.35, -6.26),
    ("London", "United Kingdom", 51.51, -0.13),
    ("Lisbon", "Portugal", 38.72, -9.14),
    ("Casablanca", "Morocco", 33.57, -7.59),
    ("Dakar", "Senegal", 14.72, -17.47),
    ("Accra", "Ghana", 5.56, -0.20),
    ("Nairobi", "Kenya", -1.29, 36.82),
    ("Moscow", "Russia", 55.76, 37.62),
    ("Dubai", "United Arab Emirates", 25.20, 55.27),
    ("Karachi", "Pakistan", 24.86, 67.01),
    ("Delhi", "India", 28.61, 77.21),
    ("Jakarta", "Indonesia", -6.21, 106.85),
    ("Singapore", "Singapore", 1.35, 103.82),
    ("Kuala Lumpur", "Malaysia", 3.14, 101.69),
    ("Perth", "Australia", -31.95, 115.86),
    ("Seoul", "South Korea", 37.57, 126.98),
    ("Tokyo", "Japan", 35.68, 139.65),
    ("Darwin", "Australia", -12.46, 130.84),
    ("Adelaide", "Australia", -34.93, 138.60),
    ("Brisbane", "Australia", -27.47, 153.03),
    ("Auckland", "New Zealand", -36.85, 174.76),
    ("Suva", "Fiji", -18.14, 178.44),
]


def probe_candidate(lat, lng):
    """(prayer, remaining_min, tz) for the candidate's next prayer, off
    the same AlAdhan API the app reads (method 12) — selection only;
    the authoritative measurement comes from the dashboard."""
    import json
    import urllib.request
    from zoneinfo import ZoneInfo

    now = datetime.datetime.now(datetime.timezone.utc)
    date = now.strftime("%d-%m-%Y")
    for _ in range(2):  # the city may already be on the next date
        url = (
            f"https://api.aladhan.com/v1/timings/{date}"
            f"?latitude={lat}&longitude={lng}&method=12"
        )
        try:
            with urllib.request.urlopen(url, timeout=10) as r:
                data = json.load(r)["data"]
        except Exception:
            return None
        tz = ZoneInfo(data["meta"]["timezone"])
        local = now.astimezone(tz)
        if local.strftime("%d-%m-%Y") == date:
            break
        date = local.strftime("%d-%m-%Y")
    best = None
    for p in PRAYERS:
        t = data["timings"][p][:5]
        h, m = (int(x) for x in t.split(":"))
        rem = ((h * 60 + m) - (local.hour * 60 + local.minute)) % (24 * 60)
        if 0 < rem < 6 * 60 and (best is None or rem < best[1]):
            best = (p, rem)
    if best:
        return best[0], best[1], data["meta"]["timezone"]
    return None


def pick_city():
    """The candidate whose next prayer is far enough out that the boot
    leaves it undischarged (§11.6: the ledger keys on date/prayer/kind,
    so an event already judged today can never re-fire) and near enough
    that the re-armed event lands inside the stepper's 0..=60."""
    best = None
    for name, country, lat, lng in CANDIDATES:
        probed = probe_candidate(lat, lng)
        if not probed:
            continue
        prayer, rem, tz = probed
        if DEFAULT_BEFORE[prayer] + 5 <= rem <= 50:
            print(f"info: candidate {name}: {prayer} {rem:.0f} min out ({tz})")
            if best is None or abs(rem - 28) < abs(best[5] - 28):
                best = (name, country, lat, lng, prayer, rem)
    return best


def scenario_c():
    # §23 item 9's situation: "an event is 3 minutes out". The event
    # must be undischarged at boot, so boot straight at a probed city
    # whose next prayer is minutes away — booting at the Sétif default
    # at this hour would ledger-miss the whole day (by design, §11.6)
    # and pre-empt the very event the re-arm should fire.
    city = pick_city()
    if not city:
        check(
            "a candidate city has a prayer in the live-fire window",
            False,
            "no candidate with a prayer 20-50 min out — rerun later",
        )
        return
    name, country, lat, lng, prayer, rem = city
    tz = probe_candidate(lat, lng)[2]
    label = name  # GeoNames' label() is the city alone
    print(f"info: prepositioning at {label}, {country} — {prayer} ~{rem:.0f} min out")

    app = App(
        pre_config=CONFIG_TEMPLATE.format(
            label=label, country=country, lat=lat, lng=lng, tz=tz
        )
    )
    try:
        dashboard(app, marker=b"NEXT PRAYER")
        dismiss_overlays(app)
        check("boot reaches the dashboard at the target city", True, label)

        # The NEXT PRAYER panel and the header clock are both in the
        # location's timezone, so their difference is the real minutes
        # remaining — no machine-timezone arithmetic.
        name, hhmm = next_prayer(app)
        header = header_minutes(app.frame())
        check(
            f"the dashboard shows {label}'s next prayer",
            name == prayer and hhmm is not None and header is not None,
            f"name={name} hhmm={hhmm} header={header}",
        )
        if not (name and hhmm and header is not None):
            return
        ph, pm = (int(part) for part in hhmm.split(":"))
        remaining = (ph * 60 + pm - header) % (24 * 60)
        print(f"info: next prayer {name} at {hhmm} ({label}), {remaining} min out")
        if not 3 <= remaining <= 55:
            check(
                "a prayer is 3–55 min out for the live-fire check",
                False,
                f"{name} is {remaining} min out — rerun when a prayer is nearer",
            )
            return

        # §23 item 9's situation verbatim: "an event is 3 minutes out".
        # The re-armed Pre-Adhan lands ~3 min after the measurement,
        # clear of the ~1 min of navigation between measuring and the
        # save that arms it — and inside the 2 min delayed-fire grace.
        target = remaining - 3
        assert 0 <= target <= 60, f"target {target} outside the stepper clamp"
        row_index = PRAYERS.index(name)
        presses = target - DEFAULT_BEFORE[name]

        app.send(b"2")
        assert app.wait_for(b"GENERAL", 5), "SETTINGS did not open"
        app.send(b"\x1b[B" * 5)  # down to PER-PRAYER TIMING
        app.send(b"\r")
        assert app.wait_for(b"PER-PRAYER TIMING", 5), "NOTIF SETTINGS did not open"
        app.send(b"\x1b[B" * row_index)  # the next prayer's row
        if presses > 0:
            app.send(b"\x1b[C" * presses)
        elif presses < 0:
            app.send(b"\x1b[D" * (-presses))

        # Everything from here on must happen in the booted process —
        # a restart would re-run the boot ladder (and its INITIALIZING
        # screen), so the check below reads only this tail.
        tail_mark = len(app.session.stream)
        # BEFORE does not move the API cache key → "Settings applied."
        save_and_confirm(app, expect_recalculated=False, back=b"PER-PRAYER TIMING")

        # The saved TOML carries the new window.
        with open(app.config_path(), "rb") as f:
            saved = tomllib.load(f)
        check(
            f"the TOML carries {name}'s new before_minutes ({target})",
            saved["prayers"][name.lower()]["before_minutes"] == target,
        )

        # Same process, no restart: the event is ~3 min out, the runner
        # ticks at 1 Hz — wait it out, then read the ledger.
        deadline = time.monotonic() + 240
        fired = None
        while time.monotonic() < deadline:
            app.pump(2.0)
            if os.path.exists(app.db_path()):
                conn = sqlite3.connect(f"file:{app.db_path()}?mode=ro", uri=True)
                try:
                    fired = conn.execute(
                        "SELECT label, detail, prayer FROM event_log "
                        "WHERE category = 'notification' AND prayer = ? "
                        "AND label LIKE '%Pre-Adhan%' "
                        "AND label NOT LIKE '%missed%' "
                        "ORDER BY at DESC LIMIT 1",
                        (name.lower(),),
                    ).fetchone()
                finally:
                    conn.close()
                if fired:
                    break
        # The fired row's shape (§11.5): label "Pre-Adhan
        # notification" (+ " (delayed)" when late), the prayer in its
        # own column, and the detail carrying the re-armed window.
        ok = check(
            "the scheduler fired the Pre-Adhan notification without a restart",
            fired is not None
            and "Pre-Adhan notification" in fired[0]
            and fired[2] == name.lower()
            and fired[1] == f"{name.upper()} · {target} min window",
            f"latest notification row: {fired}",
        )
        if ok:
            print(f"info: event_log row: {fired}")
            conn = sqlite3.connect(f"file:{app.db_path()}?mode=ro", uri=True)
            try:
                ledger = conn.execute(
                    "SELECT prayer, kind, late FROM fired_events ORDER BY fired_at DESC LIMIT 1"
                ).fetchone()
            finally:
                conn.close()
            check(
                "the fired_events ledger row landed",
                ledger is not None and ledger[0] == name.lower(),
                f"ledger: {ledger}",
            )
        check("the process never restarted", app.pid == app.session.pid)
        check(
            "no LOADING screen after the save",
            b"INITIALIZING" not in app.session.stream[tail_mark:],
        )
    finally:
        app.force_exit()


# ---------------------------------------------------------------- D
def scenario_d():
    app = App()
    try:
        label, name, hhmm = dashboard(app)
        check("boot reaches the dashboard (Sétif)", name is not None)
        if hhmm and "M" in hhmm:
            return
        check("the clock is 24h before the edit", True, f"next at {hhmm}")

        # Hand-edit the TOML while the app runs. §8.5: "No service
        # polls the config file" — SIGHUP is the external-edit path:
        # it re-reads the file and republishes, the cfg_rx arm applies,
        # the next frame renders 12h.
        path = app.config_path()
        with open(path) as f:
            body = f.read()
        # The store's hand-written default aligns its keys with
        # padding; the app's own save format does not. Either way.
        assert re.search(r'time_format\s*=\s*"24h"', body), body[:200]
        with open(path, "w") as f:
            f.write(
                re.sub(r'time_format\s*=\s*"24h"', 'time_format = "12h"', body)
            )
        os.kill(app.pid, signal.SIGHUP)

        saw_12h = False
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            app.pump(1.0)
            frame = app.frame()
            if re.search(r"\d{1,2}:\d{2} [AP]M", frame):
                saw_12h = True
                break
        check("the running app picked up the hand-edited TOML", saw_12h)
        with open(path) as f:
            check(
                "the app did not rewrite the file",
                re.search(r'time_format\s*=\s*"12h"', f.read()) is not None,
            )
    finally:
        app.force_exit()


SCENARIOS = {
    "a_search": scenario_a,
    "b_coords": scenario_b,
    "c_before": scenario_c,
    "d_watch": scenario_d,
}

if __name__ == "__main__":
    if len(sys.argv) != 2 or sys.argv[1] not in SCENARIOS:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    SCENARIOS[sys.argv[1]]()
