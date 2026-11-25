# Salat — Configuration

This document is the user-facing reference for `config.toml`: where the
file lives, what every key means, what validation rejects, and how
changes reach a running instance. It is the companion to the `salat
config` command, which prints exactly the paths and effective values
described here.

---

## 1. Where the file lives

With no `--config` flag, salat reads:

| Resource | Path | Mode |
|---|---|---|
| Configuration | `$XDG_CONFIG_HOME/salat/config.toml` | `0600` |
| Database | `$XDG_DATA_HOME/salat/salat.db` | `0600` |
| Scheduler lock | `$XDG_RUNTIME_DIR/salat/scheduler.lock` | — |

`$XDG_CONFIG_HOME` defaults to `~/.config`, `$XDG_DATA_HOME` to
`~/.local/share`, and `$XDG_RUNTIME_DIR` to `/run/user/<uid>` when a
systemd session exists. When `$XDG_RUNTIME_DIR` is unset, the lock falls
back to the state directory, then the data directory — a stale lock file
is harmless. Directories salat creates are `0700`; directories that
already exist are never modified.

`salat config` prints the resolved values:

```
scope:     default
config:    /home/u/.config/salat/config.toml
database:  /home/u/.local/share/salat/salat.db
lock:      /run/user/1000/salat/scheduler.lock

version = 1

[location]
...
```

The four lines are always printed first (labels padded, paths
canonical), then a blank line, then the effective configuration as TOML.

### The `--config` flag and scoping

`salat --config <path>` uses `<path>` as the configuration file **and
scopes the database and lock to it**, so a second configuration is a
fully separate instance — its cache, its event log, its scheduler:

| Resource | Path under `--config` |
|---|---|
| Scope key | `<stem>-<8 hex>` (see below) |
| Database | `$XDG_DATA_HOME/salat/salat-<scope>.db` |
| Lock | `$XDG_RUNTIME_DIR/salat/scheduler-<scope>.lock` |

The scope key is the configuration file's stem — lowercased, restricted
to `[a-z0-9_-]`, truncated to 16 characters (`config` if nothing
survives) — plus the low 32 bits of the FNV-1a 64-bit hash of the
canonicalized path, as 8 hex digits. Canonicalization makes
`--config ./mecca.toml` and `--config /home/u/mecca.toml` the *same*
instance rather than two. The canonical example from the architecture
document: `/home/u/mecca.toml` → `mecca-75b6780d`.

Without `--config`, the scope is `default` and the resources are
unsuffixed.

## 2. First run

A missing configuration file is not an error. Salat writes the fully
commented default below — the values the UI design ships as its mock
data (Sétif, Algeria) — and carries on. Delete the file at any time to
regenerate it.

```toml
version = 1                              # schema version, for future migrations

[location]
label      = "Sétif, Algeria"          # display string, Location screen "City"
country    = "Algeria"
latitude   = 36.1900                     # -90..=90
longitude  = 5.4100                      # -180..=180
timezone   = "Africa/Algiers"           # IANA, must parse via chrono-tz
source     = "city"                     # "city" | "coordinates" | "auto"

[calculation]
method                     = 12          # AlAdhan method id, whitelisted
school                     = 0           # 0 Shafi, 1 Hanafi
latitude_adjustment_method = 3           # 1 MidNight, 2 OneSeventh, 3 AngleBased
midnight_mode              = 0           # 0 Standard, 1 Jafari
shafaq                     = "general"   # only used when method == 15

[calculation.tune]                       # each -60..=60, minutes
imsak    = 0
fajr     = 0
sunrise  = 0
dhuhr    = 0
asr      = 0
maghrib  = 0
sunset   = 0
isha     = 0
midnight = 0

[notifications]
enabled         = true                   # Enable notifications
mosque_reminder = true                   # Mosque reminder

[adhan]
enabled = true                           # Enable adhan
preset  = "abdul-basit"                  # $XDG_DATA_HOME/salat/adhan/<preset>.mp3
file    = ""                             # absolute path; overrides preset when non-empty
volume  = 80                             # 0..=100

[display]
time_format        = "24h"               # "24h" | "12h"
language           = "en"                # single variant
theme              = "midnight-blue"     # single variant
compact_mode       = false               # Compact mode
show_seconds       = true                # Show seconds
countdown_progress = true                # Countdown progress
color_mode         = "auto"              # "auto" | "truecolor" | "256" | "ansi"

[prayers.fajr]
before_minutes = 15                      # 0..=60
after_minutes  = 20                      # 0..=60
adhan          = true
notify         = true

[prayers.dhuhr]
before_minutes = 10
after_minutes  = 30
adhan          = true
notify         = true

[prayers.asr]
before_minutes = 15
after_minutes  = 20
adhan          = true
notify         = true

[prayers.maghrib]
before_minutes = 5
after_minutes  = 15
adhan          = true
notify         = true

[prayers.isha]
before_minutes = 15
after_minutes  = 30
adhan          = true
notify         = true
```

## 3. Keys, defaults and domains

Every key except `version` may be omitted from the file; a missing key
takes its default. Unknown keys are **rejected**, never silently
ignored — a typo is an error naming the key and its line, not a quietly
unused setting.

### `[location]`

| Key | Type | Default | Domain |
|---|---|---|---|
| `label` | string | `"Sétif, Algeria"` | any display string |
| `country` | string | `"Algeria"` | any |
| `latitude` | float | `36.1900` | `-90..=90` |
| `longitude` | float | `5.4100` | `-180..=180` |
| `timezone` | string | `"Africa/Algiers"` | an IANA name (e.g. `Europe/Paris`) |
| `source` | string | `"city"` | `city` \| `coordinates` \| `auto` |

Latitude and longitude are validated client-side because the AlAdhan API
answers a bad coordinate with a fabricated 503, and an invalid latitude
with silently wrong times.

### `[calculation]`

| Key | Type | Default | Domain |
|---|---|---|---|
| `method` | integer | `12` | an AlAdhan method id (below) |
| `school` | integer | `0` | `0` (Shafi) \| `1` (Hanafi) |
| `latitude_adjustment_method` | integer | `3` | `1` (MidNight) \| `2` (OneSeventh) \| `3` (AngleBased) |
| `midnight_mode` | integer | `0` | `0` (Standard) \| `1` (Jafari) |
| `shafaq` | string | `"general"` | `general` \| `red` \| `white` — only used when `method = 15` |

Valid method ids: `0`–`5`, `7`–`23`, `99` — the set verified against
AlAdhan's own `/v1/methods` list. Anything else is rejected before any
request, because the API answers an unknown id with silently wrong
times rather than an error. `6` is excluded on purpose: AlAdhan accepts
it but answers as ISNA (`2`), so a configuration saying `6` would not
mean what it says.

### `[calculation.tune]`

Nine integer keys — `imsak`, `fajr`, `sunrise`, `dhuhr`, `asr`,
`maghrib`, `sunset`, `isha`, `midnight` — each `-60..=60` minutes,
default `0`. These offset the API's computed times.

### `[notifications]`

| Key | Type | Default |
|---|---|---|
| `enabled` | bool | `true` |
| `mosque_reminder` | bool | `true` |

### `[adhan]`

| Key | Type | Default | Domain |
|---|---|---|---|
| `enabled` | bool | `true` | |
| `preset` | string | `"abdul-basit"` | resolves to `$XDG_DATA_HOME/salat/adhan/<preset>.mp3` |
| `file` | string | `""` | empty = use the preset; non-empty must be an **absolute path to an existing regular file** |
| `volume` | integer | `80` | `0..=100` |

### `[display]`

| Key | Type | Default | Domain |
|---|---|---|---|
| `time_format` | string | `"24h"` | `24h` \| `12h` |
| `language` | string | `"en"` | `en` (only variant in this build) |
| `theme` | string | `"midnight-blue"` | `midnight-blue` (only variant in this build) |
| `compact_mode` | bool | `false` | |
| `show_seconds` | bool | `true` | |
| `countdown_progress` | bool | `true` | |
| `color_mode` | string | `"auto"` | `auto` \| `truecolor` \| `256` \| `ansi` |

### `[prayers.<name>]` — `fajr`, `dhuhr`, `asr`, `maghrib`, `isha`

| Key | Type | Default | Domain |
|---|---|---|---|
| `before_minutes` | integer | per prayer (table below) | `0..=60` |
| `after_minutes` | integer | per prayer | `0..=60` |
| `adhan` | bool | `true` | |
| `notify` | bool | `true` | |

Whole-table defaults, taken when the `[prayers.<name>]` table is absent
entirely:

| Prayer | `before_minutes` | `after_minutes` |
|---|---|---|
| fajr | 15 | 20 |
| dhuhr | 10 | 30 |
| asr | 15 | 20 |
| maghrib | 5 | 15 |
| isha | 15 | 30 |

A **partial** table — some keys present, others not — fills its missing
keys from `15`/`20`/`true`/`true` for every prayer alike, because a
key-level default cannot know which prayer it belongs to. If you write a
`[prayers.maghrib]` table at all, write all four keys.

## 4. Validation

`version` is checked **before anything else**: a file from a different
schema version cannot be meaningfully validated against this one.

| Condition | Result |
|---|---|
| `version` absent | `E201`, error names `version` |
| `version` not a positive integer | `E201`, error names `version` |
| `version` from a newer salat | `E201`: *configuration version 2 was written by a newer salat; this build understands version 1* |
| Unknown key | `E201`, error names the key and its line |
| Wrong type | `E201`, serde's message naming the key |
| Any domain violation (§3) | `E201`, **every** violation reported together, each as `key: what is wrong` |

`E201` and `E202` both exit the process with code `3`. Diagnostics go to
stderr in the shape:

```
salat: error[E201]: configuration invalid
→ adhan.volume: must be between 0 and 100 (found 150)
```

## 5. Editing and saving

- `salat config` prints the resolution lines and the effective TOML,
  creating the file with the commented defaults on first run.
- `salat config --edit` opens the file in `$VISUAL`, `$EDITOR`, or `vi`,
  then validates the result by loading it. A missing file is created
  first; an *invalid* file is still opened — fixing it by hand is the
  point. An editor that cannot start or exits non-zero is an `E202`.
- Saves are **atomic**: salat writes `config.toml.tmp` in the same
  directory, fsyncs, then renames over the target. A crash mid-write can
  never truncate a working configuration, and a failed write never
  leaves a `.tmp` behind or touches the old file.
- Changes saved from the interface reach the running instance
  immediately, with no restart. Salat does **not** watch the file, so a
  hand edit (including `salat config --edit`) is picked up by a running
  TUI or daemon only when it receives `SIGHUP` (for the systemd unit:
  `systemctl --user kill -s HUP salat`) or when it restarts.
- The file is written at mode `0600`: it records a physical location,
  which is personal data even though it is not a secret.

## 6. The database and the offline cache

`salat.db` (and its `salat-<scope>.db` sibling under `--config`) is a
SQLite database holding five tables:

| Table | What it holds | Why |
|---|---|---|
| `prayer_days` | whole fetched months of prayer times | the offline cache — instant start, full offline correctness for fetched dates |
| `fired_events` | which notifications/Adhan already fired | a restart never repeats an audible event |
| `event_log` | the Status screen's "recent events" panel | the user can see what the app did |
| `prayer_completions` | the prayers you marked `[c]` completed | the state survives a restart |
| `qibla` | the Qibla bearing per location | fetched once, never changes |

**Purpose of the cache.** Prayer times are an astronomical computation:
AlAdhan's answer for a given `(location, calculation, date)` never
changes. So a cached day is never stale and never revalidated on the
hot path — once a month is fetched, serving it costs **zero network**.
A fetch happens only for a month the cache does not already hold
completely.

**Limits and lifecycle.**

- Rows never expire and are never rewritten; the cache is bounded
  instead. Once per boot, a vacuum deletes rows older than 60 days
  (each table by its own date column) and rows of a *superseded*
  configuration that have not been read for 7 days.
- Invalidating the cache is implicit: the rows are keyed by the
  canonical cache key (coordinates at four decimals, method, school,
  latitude-adjustment rule, midnight mode, timezone, tune vector), so
  a configuration change simply reads a different key's rows. Old
  rows linger until the vacuum claims them.
- Offline with nothing cached for the requested day is an honest
  `E301` (prayer times unavailable) — a guess is never substituted.
- A corrupt database is never fatal: at boot the damaged file is
  renamed aside as `salat.db.corrupt-<timestamp>` (kept as evidence),
  a fresh one is created in its place, and the months are re-fetched.
- The database file and its WAL/SHM sidecars are mode `0600`: like the
  configuration, they record a physical location.
- Schema migrations run automatically at open; you never run a
  migration command. `_sqlx_migrations` in the schema is migration
  bookkeeping, not application data.

Deleting the database file is always safe — everything in it can be
re-fetched or re-derived, apart from the completion marks and the
event history, which are lost.
