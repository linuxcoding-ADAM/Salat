# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `docs/TROUBLESHOOTING.md`: diagnostics, platform support, installer
  messages, terminal size and layout, every error code, the daemon and
  systemd, and exit codes.
- A full contributor guide in `CONTRIBUTING.md`: setup, the checks CI
  runs, snapshot review, conventions, the merge gate and the release
  procedure.

### Fixed

- Documentation: the valid calculation method ids are `0`–`5`, `7`–`23`
  and `99` (`docs/CONFIGURATION.md` listed `0`–`15`, `17`–`23`, `99`).
- Documentation: the README lists all five exit codes, including `2`
  (usage) and `4` (prayer data or API failure).
- Documentation: the systemd setup no longer needs a clone of the
  repository, and no longer claims the unit skips restarting after a
  configuration error.
- Documentation: the v0.1.0 systemd unit cannot write the scheduler lock
  (`E904 … Read-only file system`); `docs/KNOWN_ISSUES.md` records the
  evidence and `docs/INSTALL.md` documents a `ReadWritePaths=%t` drop-in
  as a temporary workaround.

## [0.1.0] - 2026-11-25

The first release: prayer times and the Adhan for the Linux terminal, as a
single static binary for x86_64 Linux.

### Added

- **Quick view.** Bare `salat` prints a compact live view inside the shell
  (today's five times, the next prayer and a countdown) without clearing
  the scrollback. `salat --once` prints one frame and exits, as plain text
  with no escape codes when piped.
- **Full TUI.** `salat --full` opens the interactive interface: the Today
  dashboard (next prayer, countdown, today's times, Hijri date, Qibla
  direction, service health), Settings, per-prayer configuration,
  Notification settings, Location, Status, and the `?` states reference.
  It adapts from 80×24 up to larger terminals, honours `NO_COLOR`, and
  shows a too-small guard below the minimum size.
- **Prayer times from the AlAdhan API.** One request per month in ISO-8601
  mode, with a choice of calculation method, Shafi or Hanafi school,
  latitude-adjustment and midnight modes, and per-prayer tuning from -60
  to +60 minutes. There is no local calculation engine.
- **Offline cache.** A SQLite database (compiled into the binary) keeps the
  prayer-day cache, the event log, the fired-event ledger and completed
  prayers, so cached days work without a network connection.
- **Configuration.** A commented TOML file at
  `~/.config/salat/config.toml`, created with defaults on first run,
  validated with line numbers for unknown keys, and written atomically.
  `salat config` prints the resolved paths and effective configuration;
  `salat config --edit` opens it in `$EDITOR` and validates the result.
- **Offline city search.** An embedded GeoNames index (cities of 15,000
  people or more, plus a short curated list of verified places), searchable
  by name, by coordinates, or by auto-detection from the system timezone.
- **Scheduler.** Compares absolute wall-clock times on a one-second tick,
  detects clock jumps, records every fired event so a restart never fires
  it twice, and catches up missed events that are still inside their grace
  window.
- **Notifications.** Per-prayer reminder windows before and after each
  prayer, desktop notifications through the freedesktop notification
  service, and an optional mosque reminder.
- **Adhan playback.** Plays a user-supplied MP3 through the first available
  player among `mpv`, `paplay`, `ffplay` and `aplay`, started without a
  shell, with full-screen and inline alert overlays, snooze, and a stop that
  always ends the player. `salat test-adhan` checks the setup. No
  recitation audio is shipped.
- **System status.** The Status screen and `salat status` report ten
  components as healthy, degraded, down or unknown, never a false "OK".
  `salat status` exits 0 when healthy and 1 when something is down.
- **Headless daemon.** `salat daemon` runs the scheduler without a terminal,
  reloads its configuration on `SIGHUP`, and shuts down within two seconds
  on `SIGTERM`. `salat daemon --once` runs one catch-up pass for cron-style
  use. Only one process owns the schedule at a time: a second scheduler
  exits with `E903`, and a TUI started beside the daemon only observes.
- **systemd user unit.** `packaging/systemd/salat.service`, sandboxed
  (`NoNewPrivileges`, read-only filesystem apart from Salat's own
  directories, `PrivateTmp`, `MemoryMax=128M`), with restart on failure but
  not after a configuration error.
- **One-command installer.** `install.sh` downloads the release tarball,
  verifies its SHA-256 against `SHA256SUMS` before unpacking, validates
  every archive member before extracting, smoke-tests the binary, and
  installs it to `~/.local/bin` without `sudo`. It also upgrades, and
  uninstalls with `--uninstall` (listing your data rather than deleting it,
  unless you confirm `--purge`).
- **Release binaries.** A statically linked, stripped musl binary for
  `x86_64-unknown-linux-musl`, packaged as
  `salat-0.1.0-x86_64-unknown-linux-musl.tar.gz` with `SHA256SUMS`.
- **CI.** GitLab CI/CD runs formatting, clippy, the test suite, the MSRV
  check, the supply-chain audit, shell lint, the musl release build,
  packaging, and the installer tests on Debian and Alpine. Releases are
  published to GitHub Releases by a manual job for stable tags only.

### Known limitations

- Linux on x86_64 (64-bit Intel/AMD) only. ARM (`aarch64`) is deferred to
  a later release; the installer says so and stops. Windows and macOS are
  not supported.
- Adhan audio is not included: place your own MP3 at
  `~/.local/share/salat/adhan/<preset>.mp3`.

[Unreleased]: https://github.com/linuxcoding-ADAM/Salat/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/linuxcoding-ADAM/Salat/releases/tag/v0.1.0
