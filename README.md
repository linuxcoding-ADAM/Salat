<div align="center">

# Salat

**Prayer times and Adhan for the Linux terminal**

A keyboard-first Ratatui TUI with offline city search, notifications, and a systemd daemon.

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![Rust 1.97+](https://img.shields.io/badge/rust-1.97%2B-orange?logo=rust)](rust-toolchain.toml)
[![Platform: Linux](https://img.shields.io/badge/platform-Linux-informational?logo=linux&logoColor=white)](#install)
[![Built with Ratatui](https://img.shields.io/badge/built%20with-Ratatui-8A2BE2)](https://ratatui.rs)
[![Prayer times: AlAdhan](https://img.shields.io/badge/prayer%20times-AlAdhan-2E8B57)](https://aladhan.com)
[![Status: pre-release](https://img.shields.io/badge/status-pre--release-yellow)](#status-and-roadmap)

`rust` · `ratatui` · `tui` · `terminal` · `terminal-app` · `cli` · `linux` · `systemd` · `prayer-times` · `adhan` · `aladhan` · `islam` · `salat`

</div>

---

Salat is a keyboard-first, terminal-native assistant written in Rust with
[Ratatui](https://ratatui.rs). It answers one question instantly — *when is
the next prayer, and how long do I have?* — and, when you want more, opens a
full interactive interface for configuring prayer times, notifications and
Adhan playback. It is built in the spirit of `btop` and `lazygit`: a midnight
palette, no mouse required, and nothing running that you did not ask for.

## Contents

- [Features](#features)
- [Quick start](#quick-start)
- [Commands](#commands)
- [Using the interface](#using-the-interface)
- [Configuration](#configuration)
- [Adhan audio](#adhan-audio)
- [Running as a user service (systemd)](#running-as-a-user-service-systemd)
- [How it works](#how-it-works)
- [Privacy and network use](#privacy-and-network-use)
- [Files on disk](#files-on-disk)
- [Development](#development)
- [Status and roadmap](#status-and-roadmap)
- [License](#license)

## Features

- **Instant answer.** Bare `salat` prints a compact live view inside your
  shell — today's five times, the next prayer and a countdown — without
  taking over the screen or clearing your scrollback.
- **Full TUI.** `salat --full` opens the complete interface: dashboard,
  settings, per-prayer configuration, notification settings, location,
  system status, and a states reference.
- **Accurate times from AlAdhan.** Times come from the
  [AlAdhan API](https://aladhan.com) in ISO-8601 mode, one month at a time.
  Salat contains no local prayer-calculation engine: what you see is the
  API's answer, and cached days are the API's own answers too.
- **Choice of calculation.** Pick the AlAdhan calculation method, the
  Shafi or Hanafi school, the latitude-adjustment and midnight modes, and
  per-prayer minute tuning from -60 to +60.
- **Offline city search.** Find a city without a network connection. The
  index is embedded in the binary: GeoNames cities with population of
  15,000 or more, plus a small curated list of verified places that
  GeoNames has no population figure for.
- **Notifications and Adhan per prayer.** For each of the five prayers you
  can set a reminder window before and after the time, and choose whether
  it plays the Adhan and whether it sends a desktop notification. A mosque
  reminder can be enabled separately.
- **Reliable scheduling.** The scheduler compares absolute wall-clock times
  on a one-second tick, so suspend and resume, screen redraws and config
  saves cannot cause a missed or duplicated event. Every fired event is
  recorded, so a restart does not fire it again.
- **Catch-up after downtime.** Events that were missed while Salat was not
  running fire on the next pass if they are still inside their grace window.
- **System status.** A status screen and `salat status` command report ten
  components (configuration, database, network, scheduler, notifications,
  Adhan, audio and more), each as healthy, degraded, down or unknown — never
  a false "OK".
- **Headless daemon.** `salat daemon` runs the scheduler without a terminal,
  supervised by a hardened systemd user unit with graceful shutdown.
- **Scriptable output.** Piped or redirected output is plain text with no
  escape codes. `salat --once` and `salat status` are safe in status bars and
  scripts; `salat status` exits 0 when healthy and 1 when something is down.
- **Respects your terminal.** Honours `NO_COLOR`, and adapts to 80×24 up to
  larger terminals.

## Quick start

Salat is pre-release. For now, build it from source (see
[Install](#install)), then run:

```sh
salat                 # quick live view
salat --full          # full interactive interface
```

On the first run Salat writes a fully commented configuration file with
defaults for Sétif, Algeria. Change the location from the Settings screen (`2`), or edit the file
directly, then check the result with `salat config`.

### Install

Requirements:

- Linux on `x86_64` or `aarch64`
- A current stable Rust toolchain. The pinned version in
  `rust-toolchain.toml` is installed automatically by `rustup`.
- Optional: an audio player for Adhan (see [Adhan audio](#adhan-audio))
- Optional: a D-Bus session bus for desktop notifications (present on any
  desktop session)

Build and install into `~/.local/bin`:

```sh
git clone https://github.com/linuxcoding-ADAM/Salat.git
cd Salat
cargo build --release
install -m 0755 target/release/salat ~/.local/bin/salat
```

Make sure `~/.local/bin` is on your `PATH`. A one-command installer with
checksum verification and static release binaries is planned for the first
release; see [`docs/INSTALL.md`](docs/INSTALL.md).

## Commands

| Command | What it does |
|---|---|
| `salat` | Quick live view: today's times, next prayer and countdown. |
| `salat --full` | Full interactive TUI. |
| `salat --once` | Print one frame and exit. Safe for pipes and status bars. |
| `salat status` | Print system status and exit (exit 0 healthy, 1 down). |
| `salat config` | Print the resolved scope, file paths and effective configuration. |
| `salat config --edit` | Open the configuration in `$EDITOR`; the result is validated on save. |
| `salat test-adhan` | Play the configured Adhan and exit. |
| `salat daemon` | Run the scheduler headlessly, for systemd. |
| `salat daemon --once` | Run one catch-up pass and exit, for cron-style use. |

Global flags:

| Flag | Meaning |
|---|---|
| `--config <PATH>` | Use another configuration file. It gets its own database and lock, so it is a fully separate instance. |
| `--no-color` | Disable colour. `NO_COLOR` is also honoured. |
| `-v`, `-vv`, … | Raise the log level. |
| `-h`, `--help` / `-V`, `--version` | Help and version. |

Exit codes: `0` success, `1` a scheduler conflict or a fatal runtime error,
`3` a configuration error. Error codes such as `E903` (scheduler already
running) are printed with a message that says what to do next.

## Using the interface

The full interface has four main screens, switched with the number keys:

| Key | Screen | Shows |
|---|---|---|
| `1` | **Today** (dashboard) | The next prayer with its countdown, today's five times, the Hijri date, the Qibla direction and service health. |
| `2` | **Settings** | Display, location, calculation and notification options. |
| `3` | **Prayers** | Per-prayer reminder windows, Adhan and notification toggles. |
| `4` | **Status** | Ten component health checks, today's counters and the event log. |

Other screens: Location, Notification settings, the Adhan alert and mosque
reminder overlays, the Saved and Error confirmations, and `?` for the
states reference, which explains each prayer state.

Keys used throughout:

| Key | Action |
|---|---|
| `↑` `↓` | Move |
| `←` `→` | Adjust a value (on the Settings and Prayers screens) |
| `Tab` / `Shift-Tab` | Next or previous column or tab |
| `Enter` / `Space` | Select, confirm or toggle |
| `s` | Save (opens a confirmation) |
| `c` | On Today: mark the selected prayer as completed |
| `r` | On Status: re-check every component and re-fetch prayer times |
| `Esc` | Back, or cancel |
| `q` | Quit (`Ctrl-C` always quits) |
| `Ctrl-L` | Force a full redraw |

## Configuration

The configuration is a TOML file at `~/.config/salat/config.toml`, created
with defaults on first run. Every key is optional, and unknown keys are
rejected with the line number, so a typo is reported rather than ignored.

A short example:

```toml
version = 1

[location]
label     = "Sétif, Algeria"
latitude  = 36.1900
longitude = 5.4100
timezone  = "Africa/Algiers"       # any IANA timezone

[calculation]
method = 12                         # an AlAdhan method id
school = 0                          # 0 Shafi, 1 Hanafi

[calculation.tune]
fajr = 0                            # minutes, -60..=60

[prayers.fajr]
before_minutes = 15                 # reminder window before the prayer
after_minutes  = 20
adhan          = true
notify         = true

[adhan]
preset = "abdul-basit"
volume = 80                         # 0..=100
```

The full key reference, with every default and valid range, is in
[`docs/CONFIGURATION.md`](docs/CONFIGURATION.md). Changes made in the
interface take effect on save. You can also send the daemon `SIGHUP` to
reload the file.

### Choosing a calculation method

The `method` key takes an AlAdhan method id. The methods endpoint lists
them, and Salat only accepts ids that the recorded fixtures confirm. The
default, `12`, is the Union of Islamic Organisations of France. Check the
values for your region and mosque before relying on a particular method.

## Adhan audio

Salat ships **no recitation audio**, because recordings carry their own
licences. To use the Adhan, place an MP3 file where Salat looks for it:

```
~/.local/share/salat/adhan/<preset>.mp3
```

With the default preset this is `~/.local/share/salat/adhan/abdul-basit.mp3`.
Alternatively, point `file` in the `[adhan]` section at any absolute path to
an audio file of up to 50 MB.

Salat plays the file through the first available player among `mpv`,
`paplay`, `ffplay` and `aplay`. It starts the player without a shell and
stops it when you dismiss the alert. Run `salat test-adhan` to check the
setup. If no file is found, the Status screen reports `NO ADHAN FILE` and the
command prints the exact path to use.

## Running as a user service (systemd)

Salat works fully as a foreground program, and that is the default. The
systemd user service is for the case where you want **notifications and the
Adhan with no terminal open.**

```sh
mkdir -p ~/.config/systemd/user
cp packaging/systemd/salat.service ~/.config/systemd/user/salat.service
systemctl --user daemon-reload
systemctl --user enable --now salat
```

Then check it with `systemctl --user status salat` and
`journalctl --user -u salat -f`. Reload the configuration with
`systemctl --user kill -s HUP salat`, and stop with
`systemctl --user stop salat`. A stop takes under two seconds and flushes
the event log, releases the scheduler lock and stops any playing Adhan.

The unit is sandboxed: `NoNewPrivileges`, a read-only filesystem apart from
Salat's own three directories, `PrivateTmp`, and `MemoryMax=128M`. It does
not restart after a configuration error, because retrying cannot fix one.

**Only one process owns the schedule at a time.** If the daemon is running,
`salat --full` shows everything but fires nothing, and reports
`RUNNING (daemon)`. If the TUI is running, a second `salat daemon` exits with
`E903`, naming the process that holds the schedule.

For machines without a login session, `loginctl enable-linger "$USER"` lets
user units start at boot. The full guide, including removal, is in
[`docs/INSTALL.md`](docs/INSTALL.md).

## How it works

- **One scheduler, independent of rendering.** A dedicated task owns the
  plan of absolute instants and checks it against the wall clock on a
  one-second tick. Redraws, resizes and saves are not inputs to it.
- **Time is injected.** A single `Clock` abstraction is the only source of
  "now", so the state machine, countdowns, day rollover, DST and catch-up are
  tested deterministically.
- **Monthly API calls.** One request per month returns offset-aware
  timestamps, about 12 requests a year per configuration instead of about
  365.
- **Two stores with clear roles.** The configuration is human-editable TOML,
  written atomically. A SQLite database holds the prayer-day cache, the event
  log, the fired-event ledger and your completions. SQLite is compiled into
  the binary.
- **Audio through a player.** The Adhan is played by an external player
  process. ALSA and PipeWire are not linked into Salat, which is why the
  release build is planned as a fully static musl binary.

The complete design is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md),
which is the source of truth for implementation decisions.

## Privacy and network use

- The only network traffic is to `api.aladhan.com`, to fetch prayer times,
  the Qibla direction and calculation methods. There is no telemetry, no
  analytics and no account.
- City search and the prayer-day cache work offline.
- The configuration and database are created with mode `0600`, and
  directories Salat creates with `0700`.
- Desktop notifications use the standard freedesktop notification service.
  Salat ships no icon or sound files of its own.

## Files on disk

| Purpose | Path |
|---|---|
| Configuration | `$XDG_CONFIG_HOME/salat/config.toml` (default `~/.config/salat/config.toml`) |
| Database | `$XDG_DATA_HOME/salat/salat.db` (default `~/.local/share/salat/salat.db`) |
| Adhan audio | `$XDG_DATA_HOME/salat/adhan/<preset>.mp3` |
| Log | `$XDG_STATE_HOME/salat/salat.log` (default `~/.local/state/salat/salat.log`) |
| Scheduler lock | `$XDG_RUNTIME_DIR/salat/scheduler.lock` |

`salat config` prints the resolved paths for your system. With
`--config <PATH>`, the database and lock are named after that file, so each
configuration stays separate.

## Development

```sh
cargo run                    # quick mode
cargo run -- --full          # full TUI
cargo test                   # the test suite; it makes no real network calls
cargo fmt && cargo clippy --all-targets -- -D warnings
```

Layout:

| Path | Contents |
|---|---|
| `salat/` | The application crate: `api`, `app`, `audio`, `cache`, `cli`, `config`, `domain`, `location`, `notify`, `scheduler`, `service`, `status`, `ui` |
| `salat/assets/` | The embedded city index (`cities.bin`) and the curated extras (`extra_cities.tsv`) |
| `salat/tests/` | Integration tests and recorded AlAdhan fixtures |
| `xtask/` | Maintenance tasks, such as `cargo run -p xtask -- gen-cities` and `cargo run -p xtask -- check-fixtures` |
| `packaging/systemd/` | The user service unit |
| `docs/` | Architecture, configuration and installation guides |

The toolchain is pinned by `rust-toolchain.toml`. The monthly AlAdhan drift
check and contribution conventions are in [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Status and roadmap

Salat is under active development, phase by phase, following
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). The main branch currently
includes:

- prayer times from AlAdhan with the monthly cache and offline operation
- the quick view and the full TUI, at 80×24 and larger
- offline city search and the Location screen
- notifications, the Adhan, and the mosque reminder
- the system status screen and command
- the headless daemon, with the `--once` catch-up and the systemd unit

Still to come: packaging (the one-command installer and static release
binaries), the CI release workflow, and contributor documentation. Changes
are recorded in [`CHANGELOG.md`](CHANGELOG.md).

## License

Dual-licensed under **MIT OR Apache-2.0**, at your option. See
[`LICENSE`](LICENSE). Third-party notices are in
[`LICENSE-THIRD-PARTY`](LICENSE-THIRD-PARTY).
