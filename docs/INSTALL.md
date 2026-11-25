# Installing Salat

How to get Salat onto a Linux machine, keep it updated, run it as a
user service, and take it back off again.

## Requirements

- Linux on `x86_64` (64-bit Intel/AMD PCs) — the only supported
  platform. Linux on ARM64 (`aarch64`) is planned; until then the
  installer says so and stops. Windows and macOS are not supported.
  WSL2 is not officially supported (see
  [Platform support](TROUBLESHOOTING.md#platform-support)).
- A tokio-capable kernel — in practice, any Linux from the last decade.
- For the installer: `curl` or `wget`, `tar`, and `sha256sum` or
  `shasum`. The installer refuses to continue without a checksum tool.
- Optional: an audio player for Adhan (one of `mpv`, `paplay`,
  `ffplay`, `aplay` on `$PATH`).
- Optional: a D-Bus session bus for desktop notifications (present on
  any desktop session).

## The one-command install

```sh
curl -fsSL https://raw.githubusercontent.com/linuxcoding-ADAM/Salat/main/install.sh | sh
```

The two-step form, for readers who prefer to inspect first:

```sh
curl -fsSLO https://raw.githubusercontent.com/linuxcoding-ADAM/Salat/main/install.sh
less install.sh
sh install.sh
```

The script finds the latest release on GitHub Releases, downloads its
tarball (`salat-<version>-x86_64-unknown-linux-musl.tar.gz`) and
`SHA256SUMS`, and verifies the tarball's SHA-256 **before** extraction.
It checks every archive member before unpacking, runs the new binary's
`--version` as a smoke test, and only then installs the one
statically-linked binary to `~/.local/bin/salat`. If `~/.local/bin` is
not already on your `PATH`, it prints the `export PATH=...` line for
your shell. It never runs as root, never uses `sudo`, never writes
outside the install directory and your temporary directory, and never
edits a shell rc file.

It prints the tarball's download URL and its verified SHA-256 as it
runs. If something goes wrong — including `salat: command not found`
afterwards — see [Troubleshooting](TROUBLESHOOTING.md#installation).

### Installer options

| Option | Effect |
|---|---|
| `--prefix DIR` | Install into `DIR` instead of `~/.local/bin` (or set `SALAT_INSTALL_DIR`). |
| `--version vX.Y.Z` | Install that release instead of the latest one. |
| `--force` | Reinstall even if that version is already installed. |
| `--uninstall` | Remove the binary and list your data (see [Uninstall](#uninstall)). |
| `--purge` | With `--uninstall`: also delete that data, after you confirm. |
| `--help` | Show the usage text. |

To pass options through the one-line form, use `sh -s --`:

```sh
curl -fsSL https://raw.githubusercontent.com/linuxcoding-ADAM/Salat/main/install.sh | sh -s -- --version v0.1.0
```

### Verifying a download by hand

Each release on
[GitHub Releases](https://github.com/linuxcoding-ADAM/Salat/releases)
carries the tarball and `SHA256SUMS`. To install without the script,
download both into one directory, then:

```sh
sha256sum -c SHA256SUMS
tar -xzf salat-0.1.0-x86_64-unknown-linux-musl.tar.gz
install -m 0755 salat-0.1.0-x86_64-unknown-linux-musl/salat ~/.local/bin/salat
```

The tarball also contains `README.md`, `LICENSE`, `CHANGELOG.md` and
`salat.service`.

## Building from source

To build Salat yourself instead (still Linux x86_64), you need a Rust
toolchain; the version pinned in `rust-toolchain.toml` is installed
automatically by `rustup`:

```sh
git clone https://github.com/linuxcoding-ADAM/Salat.git
cd Salat
cargo build --release
install -m 0755 target/release/salat ~/.local/bin/salat
```

## Upgrades

Re-run the installer. It detects the existing binary, prints
`upgrading 0.1.0 → 0.2.0`, and replaces the file atomically — a
running `salat` keeps its open inode and is unaffected. The systemd
unit, if enabled, picks up the new binary on the next restart
(`systemctl --user restart salat`).

## Uninstall

`sh install.sh --uninstall` removes the binary, then **lists** the
data it found without deleting anything — `~/.config/salat/`,
`~/.local/share/salat/`, `~/.local/state/salat/`, and any systemd user
unit — printing the exact `rm -rf` command for each. Deleting your
configuration is your decision, not the installer's.

`sh install.sh --uninstall --purge` also deletes that data, but only
after you type `yes` at a terminal. Because it needs a terminal, run a
downloaded copy of the script rather than piping it into `sh`; a purge
without a terminal is refused and deletes nothing.

## Running Salat as a user service (systemd)

Salat is fully functional as a foreground terminal program — that is
the default and it runs the same scheduler. The user service is for
the other arrangement: **notifications and Adhan with no terminal
open.** `salat daemon` runs the prayer-time engine headlessly; the
unit file is the supported way to keep it alive.

### 1. Install the binary and run it once

The unit expects `~/.local/bin/salat`, the installer's default
location (or build from source and install it there, as above).

Run `salat` once before enabling the unit. That creates Salat's
configuration, data and state directories, which the unit makes
writable — systemd will not start a unit whose `ReadWritePaths` do
not exist.

### 2. Install the unit

The unit file is `packaging/systemd/salat.service` in the repository
(it is also inside the release tarball). It is documented, never
installed automatically — the installer does not set it up. Download
the copy that belongs to v0.1.0 into your user unit directory:

```sh
mkdir -p ~/.config/systemd/user
curl -fsSL -o ~/.config/systemd/user/salat.service \
  https://raw.githubusercontent.com/linuxcoding-ADAM/Salat/v0.1.0/packaging/systemd/salat.service
```

From a clone of the repository, copy it instead:

```sh
mkdir -p ~/.config/systemd/user
cp packaging/systemd/salat.service ~/.config/systemd/user/salat.service
```

**Known issue in v0.1.0:** the unit does not make the runtime
directory writable, so the daemon cannot create its scheduler lock and
fails with `E904 … Read-only file system`. Add this drop-in before
starting it:

```sh
mkdir -p ~/.config/systemd/user/salat.service.d
printf '[Service]\nReadWritePaths=%%t\n' > ~/.config/systemd/user/salat.service.d/runtime-dir.conf
```

(`%t` is systemd's name for your runtime directory, `/run/user/<uid>`;
see [Known issues](KNOWN_ISSUES.md) and
[Troubleshooting](TROUBLESHOOTING.md#known-issue-in-v010-the-systemd-unit-fails-with-e904).)
Then load the unit:

```sh
systemctl --user daemon-reload
```

### 3. Enable and start

```sh
systemctl --user enable --now salat
```

`enable --now` starts it immediately and arranges for it to start at
login (a **user** unit — it needs your session's D-Bus for
notifications and your session's audio device for Adhan, which is why
a system unit is the wrong tool here).

### 4. Verify

```sh
systemctl --user status salat
journalctl --user -u salat -f
```

The daemon logs structured events (fires, catches-up, reloads) to
journald and to `~/.local/state/salat/salat.log`. Send it a reload to
see a log line without touching anything else:

```sh
systemctl --user kill -s HUP salat    # re-reads ~/.config/salat/config.toml
```

And stop it the graceful way — SIGTERM shuts down within two seconds,
flushes the event log, releases the scheduler lock, and stops any
playing Adhan:

```sh
systemctl --user stop salat
```

### What the unit does

| Directive | Meaning |
|---|---|
| `After=graphical-session.target` | Wait for the session (D-Bus, audio) to exist. |
| `Restart=on-failure`, `RestartSec=10s` | Restart 10 seconds after any failed exit — a crash, but also a configuration error (exit 3), which a restart cannot fix. If the unit keeps restarting, read the reason with `journalctl --user -u salat -e`. |
| `NoNewPrivileges`, `PrivateTmp`, `ProtectSystem=strict` | The process cannot gain privileges, gets a private `/tmp`, and sees the filesystem read-only. |
| `ProtectHome=read-only` + `ReadWritePaths=…` | Home is read-only except Salat's three own directories: `~/.config/salat`, `~/.local/share/salat`, `~/.local/state/salat`. Salat also needs its runtime directory for the scheduler lock, which the v0.1.0 unit omits (see the known issue above). |
| `ProtectKernelTunables`, `ProtectControlGroups`, `RestrictNamespaces` | No kernel or container escapes. |
| `MemoryMax=128M` | The daemon is a 1 Hz loop; if it ever exceeds 128 MB, systemd stops it rather than letting it starve the machine. |

### The daemon and the terminal app together

The §4.3 arbitration covers every combination: exactly one process
owns the schedule at a time, and the others render without firing.

- **TUI while the daemon runs:** `salat --full` enters observer mode —
  everything renders, nothing fires, the Status screen reports
  `RUNNING (daemon)`. One notification per event, from the daemon.
- **Daemon while the TUI runs:** `salat daemon` exits with
  `E903 — scheduler already running`, naming the holding pid. The
  foreground TUI keeps the schedule.
- **Two daemons:** the second exits with the same `E903`.

If you enable the unit and prefer the foreground TUI's scheduler,
stop the unit first — otherwise the TUI will (correctly) render as an
observer.

### Starting at boot

User units start at login, not at boot — that is what makes the
session bus reachable. For a machine that stays logged in, that is
indistinguishable from boot. On distributions with `loginctl
enable-linger $USER`, enabling lingering starts user units at boot
even before login:

```sh
loginctl enable-linger "$USER"
```

Notifications then arrive at the login screen only once a session bus
exists; the daemon itself runs and catches up regardless — missed
events inside their grace windows fire on the next pass
(`salat daemon --once` is the same catch-up as a one-shot cron job,
if you would rather not run the service at all).

### Removing the service

```sh
systemctl --user disable --now salat
rm ~/.config/systemd/user/salat.service
rm -rf ~/.config/systemd/user/salat.service.d    # the drop-in, if you added it
systemctl --user daemon-reload
systemctl --user reset-failed
```
