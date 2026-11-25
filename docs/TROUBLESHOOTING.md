# Troubleshooting Salat

Symptoms, their causes, and what to do — grounded in what Salat
actually prints. Start with the two diagnostic tools, then find your
symptom below.

## First steps

- **`salat status`** checks ten components (configuration, database,
  network, scheduler, notifications, Adhan, audio and more) and prints
  each as healthy, degraded, down or unknown. It exits `0` when nothing
  is down and `1` when something is. The Status screen (`4` in
  `salat --full`) shows the same report; press `r` there to re-check.
- **The log file** is `~/.local/state/salat/salat.log`
  (`$XDG_STATE_HOME/salat/salat.log`). It logs at `info` by default;
  `-v` raises it to `debug` and `-vv` to `trace`. A non-empty `RUST_LOG`
  overrides both. When `-v` is given and no TUI is running, log lines
  also go to stderr. The file is truncated once it exceeds 5 MB.
- **`salat config`** prints the configuration file, database and lock
  paths Salat is actually using, then the effective configuration.

## Platform support

| Platform | Status |
|---|---|
| Linux on `x86_64` (64-bit Intel/AMD) | **Supported.** Release binaries and the installer target it. |
| Linux on ARM64 (`aarch64`) | Planned for a later release. The installer stops with `aarch64 is not supported yet; supported: x86_64`. |
| Windows, macOS | Not supported. The installer stops with `salat currently supports Linux only`. |
| WSL2 | Not officially supported. It may work, since it runs a Linux kernel, but it is not tested or targeted; desktop notifications and audio depend on your WSL setup. |

## Installation

Installer messages start with `salat:`; fatal ones with
`salat: error:`.

| Message | Cause | Fix |
|---|---|---|
| `refusing to run as root: salat is a per-user tool …` | The installer was run as root (for example with `sudo`). | Run it as your normal user. It installs into your home directory and never needs root. |
| `aarch64 is not supported yet; supported: x86_64` | An ARM machine. | No release binary exists yet; see [Platform support](#platform-support). |
| `need curl or wget to download the release` | Neither downloader is installed. | Install `curl` or `wget` with your package manager. |
| `need sha256sum or shasum: refusing to install anything that cannot be verified` | No checksum tool. | Install `coreutils` (for `sha256sum`) or `perl` (for `shasum`). The installer will not skip verification. |
| `could not determine the latest release; pass --version vX.Y.Z` | GitHub could not be reached to look up the latest release. | Check your connection, or name the release: `sh install.sh --version v0.1.0`. |
| `checksum mismatch for <asset>: refusing to install` | The download does not match the published `SHA256SUMS`. | Nothing was installed. Retry; if it persists, do not install and report it. |
| `cannot write to <dir>; choose another directory with --prefix DIR` | The install directory is not writable. | Use `--prefix` with a directory you own. |
| `refusing to --purge without a terminal to confirm on: nothing was changed.` | `--purge` was run through `curl … \| sh`. | Download `install.sh` and run `sh install.sh --uninstall --purge` directly. |

### `salat: command not found` after installing

The installer puts the binary in `~/.local/bin`, and prints this when
that directory is not on your `PATH`:

```
salat: /home/you/.local/bin is not on your PATH. Add it for this shell and future ones:
salat:   export PATH="/home/you/.local/bin:$PATH"    # add this line to your shell's startup file
```

Add that line to your shell's startup file (`~/.bashrc`, `~/.zshrc`,
…), or for fish run `fish_add_path ~/.local/bin`, then open a new
terminal. The installer never edits these files itself.

## The terminal and the layout

### The dashboard is stacked instead of two columns

The layout follows the terminal size, and each step needs **both**
the width and the height:

| Terminal size | Layout |
|---|---|
| below 80×24 | the "too small" guard (see below) |
| 80×24 or more | compact, stacked layout |
| 100×30 or more | the full two-column dashboard |
| 120×35 or more | the large layout |

A wide but short window — for example 170 columns by 29 rows — is one
row short of 100×30, so it shows the stacked layout. Make the window
taller, use a smaller font, or switch your terminal to fullscreen
(most terminal emulators toggle fullscreen with `F11`; that is a
feature of the terminal, not of Salat). The layout changes as soon as
the terminal is resized.

### `salat needs 80×24 · this terminal is W×H`

The full interface needs at least 80 columns and 24 rows. Resize the
terminal and it continues on its own.

### The quick view fails with `E902` (terminal I/O failed)

Bare `salat` draws inline in your shell and asks the terminal for the
cursor position. A terminal that never answers that query cannot show
the live quick view. Use `salat --once` (one plain frame) or
`salat --full` (the full-screen interface) instead.

### Colours look wrong

With `display.color_mode = "auto"`, Salat uses truecolor when
`$COLORTERM` is `truecolor` or `24bit`, 256 colours when `$TERM`
contains `256color`, and basic ANSI colours otherwise. Set
`color_mode` to `truecolor`, `256` or `ansi` to override it, or use
`--no-color` / `NO_COLOR` to turn colour off.

### Output piped to a file or command has no colours or live view

That is intended. When standard output is not a terminal, bare `salat`
prints one plain frame with no escape codes, exactly like
`salat --once`.

## Prayer times and the network

The only network traffic is to `api.aladhan.com`. Fetched months are
cached, so a day that was fetched once works offline.

| Code | Title | Meaning | Fix |
|---|---|---|---|
| `E301` | PRAYER TIMES UNAVAILABLE | No cached row covers the requested day, and it could not be fetched. | Connect to the network once; the month is then cached. Salat never guesses times. |
| `E302` | API UNREACHABLE | `api.aladhan.com` could not be reached. | Check your connection, DNS or proxy, then retry (`r` on the Status screen). |
| `E303` | UNEXPECTED API RESPONSE | The API's answer could not be parsed. | Retry later. If it persists, the API may have changed; please report it. |
| `E304` | RATE LIMITED | The API answered HTTP 429. | Wait and retry. Salat makes about one request per month per configuration. |

These four exit with code `4` when they end a command.

## Location

| Code | Title | Fix |
|---|---|---|
| `E001` | LOCATION LOOKUP FAILED | The lookup failed after three retries (1 s, 2 s, 4 s). Retry, or enter coordinates instead. |
| `E101` | INVALID COORDINATES | Latitude must be in `-90..=90` and longitude in `-180..=180`. Enter them again on the Location screen. |
| `E102` | INVALID CALCULATION METHOD | Choose a method from the list; see [the method ids](CONFIGURATION.md#calculation). |
| `E103` | TIMEZONE NOT DETECTED | Auto-detection could not determine a timezone. Search for your city or enter coordinates instead. |

## Configuration

| Code | Title | Meaning | Fix |
|---|---|---|---|
| `E201` | CONFIGURATION INVALID | The file has an unknown key, a wrong type, a value out of range, or an unsupported `version`. Every problem is listed together. | Fix each line it names; [CONFIGURATION.md](CONFIGURATION.md) lists every key and range. `salat config --edit` opens the file even when it is invalid. |
| `E202` | COULD NOT SAVE SETTINGS | The configuration could not be written, or the editor could not start or exited with an error. | Check the permissions of `~/.config/salat/`, and that `$VISUAL` or `$EDITOR` names a working editor. |

Both exit with code `3`. On the command line they look like:

```
salat: error[E201]: configuration invalid
→ adhan.volume: must be between 0 and 100 (found 150)
```

A hand edit does not reach a running Salat until it receives `SIGHUP`
or restarts; see [Editing and saving](CONFIGURATION.md#5-editing-and-saving).

## Adhan audio

Salat ships no recitation audio. Run `salat test-adhan` to check the
setup; it plays the configured file once.

| Code / status | Meaning | Fix |
|---|---|---|
| `E401` NO AUDIO PLAYER FOUND | None of `mpv`, `paplay`, `ffplay` or `aplay` is on `$PATH`. | Install one of them. |
| `E402` ADHAN FILE MISSING, or `NO ADHAN FILE` on the Status screen | The file does not exist or cannot be used. | Put an MP3 at `~/.local/share/salat/adhan/<preset>.mp3` (default preset: `abdul-basit`), or set `[adhan] file` to an absolute path. The file must be a readable regular file of at most 50 MB. |
| `E403` ADHAN PLAYBACK FAILED | The player exited with an error. | Run the player by hand on the same file to see its error. |

## Notifications

| Code | Meaning | Fix |
|---|---|---|
| `E501` NOTIFICATIONS UNAVAILABLE | No D-Bus session bus is available. | Desktop notifications need a desktop session. Run Salat from inside your session; for the systemd unit, it must be a **user** unit (as shipped). |

## The scheduler, the daemon and systemd

### `E903` — scheduler already running

```
salat: error[E903]: a scheduler is already running (pid 1234)
→ held by pid 1234 since …
→ stop the running salat (or its systemd unit) first
```

Only one process fires notifications and the Adhan at a time. A second
`salat daemon` exits with `E903`, naming the process that holds the
schedule. A TUI started while the daemon runs does not fail: it shows
everything but fires nothing, and its Status screen reports
`RUNNING (daemon)`. To use the TUI's own scheduler, stop the daemon
first (`systemctl --user stop salat`).

### `E904` — scheduler lock failed

The lock file (`$XDG_RUNTIME_DIR/salat/scheduler.lock`, see
`salat config`) could not be created or opened. The message after `→`
gives the operating-system reason.

### Known issue in v0.1.0: the systemd unit fails with E904

The shipped unit `packaging/systemd/salat.service` uses
`ProtectSystem=strict`, which makes the filesystem read-only apart from
the paths in `ReadWritePaths`. Those list Salat's three directories in
your home, but not the runtime directory where the scheduler lock lives
(`/run/user/<uid>/salat`). Under the unit, the daemon therefore fails:

```
salat: error[E904]: the scheduler lock could not be acquired
→ Read-only file system (os error 30)
```

and `Restart=on-failure` restarts it every 10 seconds. Running
`salat daemon` or the TUI outside systemd is not affected.

The full evidence is in [Known issues](KNOWN_ISSUES.md). As a temporary
workaround, add the runtime directory with a drop-in:

```sh
systemctl --user edit salat
```

and enter:

```ini
[Service]
ReadWritePaths=%t
```

then `systemctl --user restart salat`. (`%t` is your runtime
directory, `/run/user/<uid>`.)

### The unit fails with `status=226/NAMESPACE`

The journal shows `Failed to set up mount namespacing: … No such file
or directory`. One of the directories the unit makes writable —
`~/.config/salat`, `~/.local/share/salat` or `~/.local/state/salat` —
does not exist yet, and systemd will not start the unit without it.
Run `salat` once (it creates them), then
`systemctl --user restart salat`.

### The unit restarts in a loop

`Restart=on-failure` restarts the daemon 10 seconds after **any**
failed exit, including a configuration error. Look at the reason with:

```sh
journalctl --user -u salat -e
```

Fix what it reports — for a configuration error, the `E201` lines name
each problem — then `systemctl --user restart salat`.

### Notifications or the Adhan do not happen with the daemon

- Check `systemctl --user status salat` and the journal (above).
- Make sure the `salat` binary is at `~/.local/bin/salat`, the path the
  unit runs.
- User units start at login. For boot without a login, see
  [Starting at boot](INSTALL.md#starting-at-boot).

## Database

| Code | Meaning | Fix |
|---|---|---|
| `E601` DATABASE ERROR | The cache database is unusable. | A corrupt database is renamed aside as `salat.db.corrupt-<timestamp>` and recreated automatically at the next start. Deleting `~/.local/share/salat/salat.db` is also safe: the cache is re-fetched; only completion marks and the event history are lost. |

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success. |
| `1` | A runtime error, including a scheduler conflict (`E903`). |
| `2` | A command-line usage error. |
| `3` | A configuration error (`E201`, `E202`). |
| `4` | Prayer data unavailable or the API failed (`E301`–`E304`). |

## Reporting a problem

Include `salat --version`, the output of `salat status`, your Linux
distribution, and the relevant lines of
`~/.local/state/salat/salat.log` (run with `-v` for more detail).
Remove your coordinates from what you share if you prefer: the
configuration records your location.
