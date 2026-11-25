# Known issues

Confirmed defects in Salat **v0.1.0** (as published), with the evidence
for each and the fixes that have been proposed.

**Status of this document.** v0.1.0 stays as published. Nothing below
has been fixed. Every proposed fix is marked **NOT IMPLEMENTED**: it is
recorded as technical debt for a future release, with no commitment to
a version or a date. For user-facing symptoms and workarounds, see
[`TROUBLESHOOTING.md`](TROUBLESHOOTING.md) and
[`INSTALL.md`](INSTALL.md#running-salat-as-a-user-service-systemd).

## Summary

| ID | Defect | Affects | Workaround |
|---|---|---|---|
| [KI-1](#ki-1-the-systemd-unit-cannot-write-the-scheduler-lock) | The systemd unit cannot write the scheduler lock, so the daemon fails with `E904` and restarts every 10 s | `salat daemon` under the shipped unit only | Yes, a drop-in |
| [KI-2](#ki-2-the-unit-restarts-after-a-configuration-error) | The unit restarts after a configuration error (exit 3), which a restart cannot fix | the shipped unit only | Fix the configuration, or stop the unit |
| [KI-3](#ki-3-a-missing-readwritepaths-directory-stops-the-unit-before-it-starts) | The unit refuses to start until Salat's three directories exist | the shipped unit only | Run `salat` once first |

Running `salat`, `salat --full` or `salat daemon` **outside** systemd is
not affected by any of these.

## Reproduction environment

All tests below were run on 2026-11-25.

- **System:** Fedora 44, Linux 7.3.3, systemd 259 (259.10-1.fc44), user
  session with a `systemd --user` manager and `XDG_RUNTIME_DIR=/run/user/<uid>`.
- **Binary:** the published v0.1.0 release tarball
  (`salat-0.1.0-x86_64-unknown-linux-musl.tar.gz`, SHA-256
  `51a6d93c8481ffb23aa5de459e842aa81f5d681dc60f48cd7409b11c15ff7691`),
  checked against the release's `SHA256SUMS` before use.
- **Method:** temporary user units created with `systemd-run --user`,
  not the installed unit file. The unit's sandbox directives were passed
  as properties: `NoNewPrivileges=true`, `PrivateTmp=true`,
  `ProtectSystem=strict`, `ProtectHome=read-only`,
  `ProtectKernelTunables=true`, `ProtectControlGroups=true`,
  `RestrictNamespaces=true`, `MemoryMax=128M`, and a `ReadWritePaths=`
  listing the three Salat directories. The command was
  `salat --config <path> daemon`.
- **Isolation:** the tests used throw-away configuration, data and state
  directories (`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`) and a
  `--config` path of their own, which gives the daemon its own scoped
  database and lock file. No Salat unit was installed and no Salat
  process was running. Everything the tests created was removed.
- **Limits of the method:** `ExecStart` in the shipped unit is
  `%h/.local/bin/salat daemon`, with no `--config`. The tests used a
  scoped `--config` to keep the lock file separate from any real one;
  the path of the lock directory (`$XDG_RUNTIME_DIR/salat/`) is the same
  either way. The shipped unit file itself was not started. Test D used
  `RestartSec=2s` instead of the unit's `10s` so that it finished
  quickly.

## KI-1: the systemd unit cannot write the scheduler lock

**Symptom.** The failure was reproduced with a transient `systemd-run`
unit configured with the shipped unit's sandbox directives (see
[Reproduction environment](#reproduction-environment)); the installed
`salat.service` itself was not started. In that setup the daemon exits
immediately:

```
salat: error[E904]: the scheduler lock could not be acquired
→ Read-only file system (os error 30)
```

The shipped unit has `Restart=on-failure` and `RestartSec=10s`, so
systemd would be expected to restart the daemon every 10 seconds and it
would never stay up. That restart cycle is inferred from the unit's
directives; the restart behaviour itself was observed only in test D
(KI-2).

**Cause.** The scheduler lock is
`$XDG_RUNTIME_DIR/salat/scheduler.lock` (`/run/user/<uid>/salat/…`).
Before taking it, the daemon creates the directory and opens the file
for writing (`salat/src/app/lock.rs`, `try_acquire`: `create_dir_all`,
then `OpenOptions … write(true).create(true)`). The unit sets
`ProtectSystem=strict`, which mounts the whole filesystem read-only
except the paths in `ReadWritePaths=`, and that list names only
`%h/.config/salat`, `%h/.local/share/salat` and `%h/.local/state/salat`.
`/run/user/<uid>` is not in it.

**Evidence.**

| Test | Sandbox | Result |
|---|---|---|
| **A** | the shipped unit's sandbox directives, reproduced with `systemd-run` | ❌ `E904 … Read-only file system (os error 30)`, exit status 1, after 4.7 s |
| **B** | A, plus `<runtime dir>/salat` in `ReadWritePaths` | ✅ ran until the test's 12 s time limit, then stopped normally with exit status 0 |
| **C** | A, plus the whole runtime directory (`%t`) in `ReadWritePaths` | ✅ ran until the test's 10 s time limit; no `E90x` or error line in the journal |

**Workaround** (also in `INSTALL.md`): a drop-in that makes the runtime
directory writable, equivalent to test C.

```sh
mkdir -p ~/.config/systemd/user/salat.service.d
printf '[Service]\nReadWritePaths=%%t\n' > ~/.config/systemd/user/salat.service.d/runtime-dir.conf
systemctl --user daemon-reload
```

Note that the drop-in itself was not run as a drop-in file. Test C
applied the same directive as a `systemd-run` property.

**Affected files.**

- `packaging/systemd/salat.service`: `ReadWritePaths=`
- `salat/src/app/lock.rs`: where the lock directory is created and the
  file opened (no defect in the code; it behaves correctly when the
  path is writable)
- `salat/src/config/store.rs`: where the lock directory is resolved from
  `$XDG_RUNTIME_DIR`, with fallbacks to the state and data directories
- `docs/ARCHITECTURE.md` §21.4: states that the three paths are "the
  whole filesystem surface Salat needs" (a dated note records the issue)

**Proposed fixes. NOT IMPLEMENTED.**

1. Add the runtime directory to the unit's `ReadWritePaths=`, as
   `%t/salat` (the directory must exist when the unit starts, see KI-3)
   or as `%t` (always exists, but makes the whole runtime directory
   writable; the more permissive option).
2. Add `RuntimeDirectory=salat` to the unit. systemd then creates
   `%t/salat` before the service starts, removes the existence problem
   of option 1, and makes that directory writable under
   `ProtectSystem=strict`. This needs checking against the daemon's
   `--once` and TUI paths, which use the same directory outside systemd.
3. Add an automated check that runs the daemon under the unit's sandbox
   (for example the transient-unit method above), so that a sandbox
   change cannot silently break the lock again. The current tests do not
   run under systemd.

## KI-2: the unit restarts after a configuration error

**Symptom.** An invalid configuration makes the daemon exit with code 3
(`E201`). The unit has `Restart=on-failure`, so systemd restarts it
anyway, repeatedly.

**Cause.** `Restart=on-failure` restarts on any non-zero exit status.
The unit does not exclude exit status 3, although a restart cannot fix
a configuration error. Earlier documentation stated that the unit did
not restart in this case; that statement was wrong and has been
corrected.

**Evidence.**

| Test | Setup | Result |
|---|---|---|
| **D** | `Restart=on-failure`, `RestartSec=2s`, a configuration with `adhan.volume = 150` | ❌ `E201`, exit status 3, then `Scheduled restart job`; **`NRestarts=2`** after 7 s |

**Workaround.** Fix the configuration (the `E201` lines name each
problem), or stop the unit: `systemctl --user stop salat`. The reason
for a failure is in `journalctl --user -u salat -e`.

**Affected files.**

- `packaging/systemd/salat.service`: `Restart=on-failure`
- `docs/ARCHITECTURE.md` §21.4

**Proposed fix. NOT IMPLEMENTED.** Add `RestartPreventExitStatus=3` to
the unit, so that a configuration error is a final failure.
`E202` (could not save settings) also exits with 3; whether it, or any
other exit code, should be excluded as well is open.

## KI-3: a missing `ReadWritePaths` directory stops the unit before it starts

**Symptom.** If `~/.config/salat`, `~/.local/share/salat` or
`~/.local/state/salat` does not exist, the unit fails before running
Salat at all:

```
Failed to set up mount namespacing: <path>: No such file or directory
Failed at step NAMESPACE spawning …
status=226/NAMESPACE
```

**Cause.** systemd must bind-mount each `ReadWritePaths=` entry, and a
path that does not exist makes that fail. On a fresh installation
nothing has created the directories yet, because the installer installs
only the binary and Salat creates its directories when it first runs.

**Evidence.**

| Test | Setup | Result |
|---|---|---|
| **E** | `ProtectSystem=strict` with a `ReadWritePaths=` entry that does not exist, running `/bin/true` | ❌ `status=226/NAMESPACE`, `Failed to set up mount namespacing … No such file or directory` |

This test did not run Salat. It shows systemd's behaviour for the
directive, which the unit uses.

**Workaround.** Run `salat` once before enabling the unit. This is in
`INSTALL.md`.

**Affected files.**

- `packaging/systemd/salat.service`: `ReadWritePaths=`
- `install.sh`: installs only the binary and creates none of these
  directories

**Proposed fixes. NOT IMPLEMENTED.**

1. Prefix the `ReadWritePaths=` entries with `-` (for example
   `-%h/.config/salat`). systemd then ignores a missing path instead of
   failing. By reasoning (untested), this alone is not enough: the
   directory would still be missing, and with `ProtectHome=read-only` the
   daemon could not create it.
2. Use `ConfigurationDirectory=`, `StateDirectory=` and
   `RuntimeDirectory=` (all `salat`), which make systemd create and
   manage the directories. These resolve to `/etc`, `/var/lib` and
   `/run` for system units; for user units they resolve under the user's
   `XDG_*` directories, which would need to match the paths Salat uses
   (Salat's data directory is `~/.local/share/salat`, which none of
   these directives provides). This is reasoning, not a tested result,
   and needs verifying before it is relied on.
3. Have the installer create the three directories. This changes
   `install.sh`, and does not help a user who installs the binary some
   other way.

## Not tested

Recorded so that nobody reads this document as saying more than it
does:

- The shipped unit file was never started as `salat.service`; the tests
  used `systemd-run` with its sandbox properties (see above).
- None of the proposed fixes has been tried, except the part of KI-1
  covered by tests B and C (the runtime directory made writable).
- `RuntimeDirectory=salat`, `RestartPreventExitStatus=3` and the
  directory-creation options have **not** been tested.
- Behaviour on other distributions or other systemd versions.
