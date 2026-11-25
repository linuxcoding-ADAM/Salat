# Salat — Architecture & Execution Plan

**Version 1.0 · v0.1.0 target · Linux-native terminal prayer time & Adhan assistant**

Prepared as an implementation-ready specification for a coding model. No application code is included; every decision below is stated so it can be implemented without further architectural guessing.

**Source of truth for all UI:** the finalized Figma Make design `Linux-Terminal-Application-UI` (file `ElXdSDMYyLt4XArjTmcC6t`), inspected via the Figma MCP server and read in full from its `src/App.tsx` (1,862 lines) plus `src/index.css` and the designer's own polish brief at `src/imports/pasted_text/salat-ui-polish.md`. Where this plan and the design disagree, the design wins; where the design is silent or physically impossible in a terminal, the ambiguity is named explicitly in §3.6 with a recommended smallest-practical resolution.

**Source of truth for prayer data:** the live AlAdhan API, probed directly during planning. Every claim in §7 marked ✅ was verified against `api.aladhan.com` on 2026-09-05, not inferred from documentation.

---

## 1. Executive summary

Salat is a single statically-linked Rust binary that answers one question instantly — *when is the next prayer, and how long do I have* — and, when asked, opens a full keyboard-driven terminal application for configuring and monitoring prayer times, notifications and Adhan playback.

The architecture rests on seven decisions that shape everything else.

**1. Two entry modes, one rendering vocabulary.** `salat` renders a compact live view into an *inline* Ratatui viewport (no alternate screen, no screen clearing, scrollback preserved); `salat --full` enters the alternate screen and runs the complete interactive TUI. Both draw the same widgets from the same immutable UI snapshot, so the quick view is not a second implementation — it is the 80×24 dashboard's two primary panels with the chrome removed.

**2. Scheduling is wall-clock driven and completely independent of rendering.** A dedicated Tokio task owns a *plan* of absolute instants (`DateTime<Tz>`) and compares them against the wall clock on a 1 Hz tick. It never sleeps until a target — that is what breaks across laptop suspend, because `tokio::time::sleep` is monotonic and does not advance while the machine is asleep. Redraws, resizes, screen changes and config saves cannot cause a missed or duplicated event, because none of them are inputs to the scheduler. Fired events are recorded in SQLite keyed by `(date, prayer, kind)` so a restart cannot re-fire them.

**3. Time is injected, never read ambiently.** A `Clock` trait (`SystemClock` in production, `MockClock` in tests) is the only source of "now" in the entire codebase. This is what makes the state machine, countdown, day-rollover, DST and catch-up logic deterministically testable rather than flaky.

**4. AlAdhan is called in ISO-8601 mode, one month at a time.** `GET /v1/calendar/{year}/{month}?…&iso8601=true` returns a whole month of offset-aware timestamps in ~44 KB. That is ~12 requests per year per configuration instead of ~365, it makes the parser trivial (`DateTime<FixedOffset>` straight from the string), and it makes offline operation *correct rather than approximate* — a cached day is the API's own answer for that day, not a local recalculation. No local prayer calculation engine exists anywhere in the codebase, as required.

**5. Configuration lives in TOML; only genuinely durable data lives in SQLite.** The design itself settles this: the SAVED screen displays `~/.config/salat/config.toml`, and the Status screen lists "Config file" and "Prayer database" as two separate components. Config is a human-editable TOML file written atomically; SQLite holds the prayer-day cache, the event log the Status screen displays, the fired-event ledger and user prayer completions. Five tables, no ORM abstraction layer.

**6. Audio is delegated to a Linux player process, not built in.** Salat probes `mpv → paplay → ffplay → aplay`, spawns the winner with an argv (never a shell), and kills it on dismiss. This keeps ALSA/PipeWire out of the link graph, which is precisely what allows the release binary to be a fully static `*-unknown-linux-musl` artifact with no runtime library dependencies at all.

**7. City search is offline.** AlAdhan cannot geocode — `timingsByCity` returns real timings but placeholder coordinates (`8.8888888`, `7.7777777`) ✅ — so the design's `[ SEARCH CITY ]` results list cannot be backed by it. Salat embeds a compact 25,000-city index (GeoNames `cities15000`: name, country, lat, lng, IANA timezone) and searches it locally. This adds no network service, no privacy exposure and no failure mode, and it gives `[ AUTO-DETECT ]` a real answer too (system timezone → its principal city).

Distribution is a hand-written 150-line POSIX `install.sh` that detects the architecture, downloads the matching release tarball plus `SHA256SUMS` from GitHub Releases, **verifies the checksum before installing**, and places the binary in `~/.local/bin` — never as root, never with `sudo`, never executing downloaded content.

Development proceeds through **16 phases**, each of which leaves the repository buildable, testable and demonstrably working, with its own automated tests, manual checklist, exact commands and definition of done. Testing is not a phase; it is a gate on every phase.

---

## 2. Product scope

### 2.1 In scope for v1

| Capability | Detail |
|---|---|
| Quick view | `salat` — current time, next prayer, live countdown, today's five prayer times, essential status |
| Full TUI | `salat --full` — 12 screens from the Figma design, keyboard-only operation |
| Prayer data | AlAdhan API, monthly calendar fetch, offline from cache |
| Prayer states | 7 states: Upcoming, Next, Entering Adhan Window (pre-adhan), Adhan, Mosque Reminder (post-adhan), Passed, Completed |
| Per-prayer config | Independent before-minutes, after-minutes, Adhan on/off, notifications on/off, sound preset |
| Notifications | Desktop notifications via D-Bus, pre-Adhan and mosque reminder |
| Adhan playback | External Linux player, volume control, test playback, stop on dismiss |
| Scheduling | Wall-clock scheduler surviving suspend, clock changes, day rollover, config changes, restarts |
| Location | Offline city search, manual coordinates, timezone-based auto-detect |
| Settings | Exactly the fields the Figma design shows, with draft/save/cancel semantics |
| Status | The 10 components and event log the Figma Status screen shows |
| Persistence | TOML config, SQLite cache + event log + fired ledger + completions |
| Background service | `salat daemon` + shipped systemd **user** unit |
| Distribution | `curl … install.sh \| sh`, precompiled static musl binaries, x86_64 (aarch64 deferred past v0.1.0 — §28 D8) |

### 2.2 Explicitly out of scope for v1

Local prayer calculation of any kind. A second prayer provider or API fallback. Any web frontend, browser, or HTTP server. Qibla compass visualisation beyond the single degree figure the design prints. Hijri date computation (AlAdhan supplies it). Multiple themes beyond "Midnight Blue". Languages beyond English. A daemon IPC protocol or client/server split. Prayer tracking analytics, streaks or history views beyond today's event log. Packaging for distro repositories (AUR, Flatpak, .deb) — the install script is the v1 channel. Windows or macOS support.

### 2.3 Non-goals that are deliberate, not oversights

- **No `--json` API surface.** `salat --once` prints the plain-text quick view; a stable machine-readable contract is a v1.1 commitment we should not make before the domain model has settled.
- **No shipped Adhan audio.** Recitations carry licensing constraints that a permissively-licensed project should not absorb silently. `sound_preset` resolves to `$XDG_DATA_HOME/salat/adhan/<preset>.mp3`; the Status screen reports `Audio system: NO ADHAN FILE` when absent, and `salat test-adhan` explains exactly where to place a file. Documented in README, not hidden.

### 2.4 Target user and environment

A Linux user comfortable in a terminal, on any modern distribution, running a graphical session (for notifications) or a bare TTY (where notifications degrade gracefully and the TUI still works). Terminal emulator with 256-color minimum, truecolor recommended, UTF-8 mandatory. Minimum terminal 80×24.

---

## 3. Figma design analysis

### 3.1 What was inspected

The Figma Make file is a React prototype. Its `src/App.tsx` is the specification: it contains a `Screen` union of 12 screens, a `PrayerState` union of 7 states, a `TermSize` union of 3 breakpoints, an exact color constant table, an explicit typography scale, and every screen implemented with real content and real per-breakpoint branching.

Three artifacts were read completely:

- `src/App.tsx` — 1,862 lines, all 12 screens, all responsive branches, all keyboard handling.
- `src/index.css` — confirms **JetBrains Mono** as the type family and a blink keyframe as intentional.
- `src/imports/pasted_text/salat-ui-polish.md` — the designer's brief, which independently confirms the constraints: implementable with `Block, Paragraph, Table, List, Gauge, Tabs, Layout, Span, Line`; 80×24 minimum with *secondary information collapsing rather than everything shrinking*; the next prayer as the single focal point; the 7 states distinguishable **without relying on color alone**; keyboard model `↑↓` navigate, `←→` change value, `Enter` select/open, `Esc` back/cancel; and an explicit prohibition on gradients, glassmorphism, oversized illustrations, rounded cards and browser patterns.

Everything below is extracted from those files, not invented.

### 3.2 Design tokens (implement verbatim)

**Palette.** 25 declarations carrying 27 named values — the `dot1/2/3` row holds three. These become `ui::theme::Theme` constants as `Color::Rgb`. §15.6 counts them precisely and gives the 256-color and ANSI degradations.

| Token | Hex | Role |
|---|---|---|
| `termBg` | `#080d18` | terminal body background |
| `panelBg` | `#0b1222` | panel interior |
| `rowActive` | `#0f1f36` | selected / next-prayer row |
| `rowSel` | `#122440` | focused numeric cell |
| `winBg` | `#141428` | window chrome (prototype only) |
| `winTitle` | `#1a1a32` | window titlebar (prototype only) |
| `border` | `#1a3355` | default panel border |
| `borderDim` | `#0d1a2a` | row separators, dim borders |
| `borderHi` | `#3d7ab5` | active panel border, selection rail |
| `borderGrn` | `#1e4a30` | prayer-config panel border |
| `text` | `#c8d4ea` | primary body text |
| `textSub` | `#6b7d94` | secondary / labels |
| `textDim` | `#2d3f52` | passed / disabled |
| `textBright` | `#eef2f8` | emphasised value, countdown |
| `green` | `#4ade80` | positive, next prayer, live clock |
| `greenBg` | `#091a0f` | completed row, SAVED panel |
| `greenDim` | `#1a3d24` | completed border |
| `red` | `#f87171` | Adhan, errors |
| `redBg` | `#190808` | Adhan screen, error panel |
| `redDim` | `#3d1515` | post-adhan border |
| `yellow` | `#fbbf24` | pre-Adhan window, warnings |
| `yellowBg` | `#1a1200` | pre-adhan row |
| `blue` | `#60a5fa` | informational (Adhan-played events) |
| `cyan` | `#34d399` | actionable value (`[ PLAY ]`) |
| `dot1/2/3` | `#ff5f57` `#ffbd2e` `#28c841` | prototype window buttons — **not implemented** |

Additional grounds outside `C`: root `#04060e`, header/footer `#080c15`, error-trace block `#100808`.

**Typography scale.** The design defines four sizes with an explicit comment block: `0.72em` meta (breadcrumbs, timestamps, footnotes), `0.82em` small (table data, secondary labels), `0.88em` body (setting labels, descriptions, fields), `1.00em` base (prayer names, panel titles). Plus display sizes `1.6em`, `2.0em`, `2.2em`, `2.4em`, `2.8em`, `3.0em` used only for the countdown and the Adhan prayer name.

**A terminal has one font size.** This is the single largest translation problem in the design and §3.6-A1 resolves it. The scale maps onto Ratatui as *weight and color*, not size:

| Design size | Ratatui treatment |
|---|---|
| `0.72em` meta | `textSub`/`textDim`, `letterSpacing .14em` → single spaces between letters where it is a short label, otherwise plain |
| `0.82em` small | `text` or `textSub`, no modifier |
| `0.88em` body | `text` |
| `1.00em` base | `text`/`textBright` + `Modifier::BOLD` |
| `≥1.6em` display | dedicated multi-row block-glyph renderer (§3.6-A1) |

**Glyph vocabulary.** Borders `╭ ─ ╮ │ ╰ ╯` (→ `BorderType::Rounded`); gauge `█` filled / `░` empty; state marks `✓ ● ◈ ◉ ○ ▶ ◆`; stepper arrows `◂ ▸`; selection prefixes `› ` and `▶ `; separators `│` and `·`.

**Blink rates.** The design blinks four things at distinct rates: default `560 ms` (text cursor), Adhan badge `500 ms`, mosque reminder banner `800 ms`, loading step marker `300 ms`. Implement as a single `Blink { period }` helper driven by the render tick; the 1 Hz tick is too coarse, so the TUI's tick interval must be **250 ms** with the clock display only changing on second boundaries (§15.4).

**The blink phase comes from the clock, never from process start.** `Blink::on(&self, now: DateTime<Tz>) -> bool` returns `(now.timestamp_millis() / period_ms) % 2 == 0`, where `now` is the `Clock::now()` of §5.2 — the same trait that is the only source of time anywhere in the application. The helper holds no mutable state, no counter and no instant of its own. Two consequences follow and both are wanted: under `MockClock` the phase is a pure function of the fixture, so a snapshot taken at `17:38:23.000` renders identical blink states on every machine forever (§20.3); and two Salat processes on one host blink in lockstep instead of drifting apart, because neither phase depends on when its process started.

Sampling that function on a 250 ms tick preserves the *average* rate exactly — the mean hold is within 1 ms of the ideal for all four periods — but it quantises each individual toggle to the first tick at or after its true instant, so the hold lengths are not uniform unless the period is a multiple of the tick:

| Rate | Hold lengths observed on the 250 ms tick | Uniform? |
|---|---|---|
| `500 ms` Adhan badge | always 2 ticks | yes — the only exact one |
| `560 ms` text cursor | 2 ticks 75 % of the time, 3 ticks 25 % | near enough to read as steady |
| `800 ms` mosque banner | 3 ticks 80 %, 4 ticks 20 % | near enough to read as steady |
| `300 ms` loading marker | 1 tick 80 %, 2 ticks 20 % | visibly uneven, and accepted |

The loading marker's stutter is the one place the tick shows. It is accepted rather than fixed: the design intent there is "fast blink", the average rate is correct, and 250 ms is the tick floor fixed by §15.4. Do not shorten the tick interval to chase it, and do not round the four periods to multiples of 250 ms — the rates are design tokens and are implemented verbatim.

### 3.3 Screen inventory

| # | `Screen` value | Kind | Reached by |
|---|---|---|---|
| 1 | `dashboard` | main | `1`, `Esc` from anywhere |
| 2 | `settings` | main | `2` |
| 3 | `prayer-config` | main | `3` |
| 4 | `status` | main | `4` |
| 5 | `notif-settings` | sub | Settings → Enable notifications / Per-prayer timing |
| 6 | `location` | sub | Settings → Location |
| 7 | `confirm` | sub | any screen's `[S] SAVE` |
| 8 | `error` | sub | any failed operation |
| 9 | `loading` | boot | process start |
| 10 | `adhan-alert` | overlay | scheduler, prayer instant |
| 11 | `mosque-reminder` | overlay | scheduler, prayer + N minutes |
| 12 | `states` | reference | see §3.6-A7 |

`isOverlay = ["adhan-alert", "mosque-reminder"]` — both suppress header and footer. Every other screen keeps them.

### 3.4 Chrome specification

**Header** (2 rows: content + `borderBottom`): left `SALAT` bold + `v0.1.0` (version hidden at 80×24); right `SÉTIF, ALGERIA │ 05 SEP 2026 │ 19:21:03` where the location and date are hidden at 80×24 and the clock is `green`.

**Footer** (2 rows: `borderTop` + content). On main screens: `[1] TODAY  [2] SETTINGS  [3] PRAYERS  [4] STATUS` — the *active* tab renders as green background with dark text, and the word labels disappear at 80×24 leaving `[1] [2] [3] [4]` — then right-aligned `[Q] QUIT`. On sub-screens: `↑↓ move · ←→ adjust · ENTER select · ESC back · S save`, with `S save` dropped at 80×24.

Note the label/screen mismatch: the tab reads **PRAYERS** but the screen is `prayer-config`, and **TODAY** maps to `dashboard`. Use the design's labels in the UI and the design's identifiers in code.

### 3.5 Per-screen extraction (data that must survive into the implementation)

Mock data in the design is *specification of shape*, not of values:

- `PRAYERS`: FAJR 05:12 passed · DHUHR 12:43 passed · ASR 16:18 passed · MAGHRIB 19:21 **next** · ISHA 20:47 upcoming. **Five rows. Sunrise appears in no table at any breakpoint.**
- `PCONFIGS`: FAJR 15/20 · DHUHR 10/30 · ASR 15/20 · MAGHRIB 5/15 · ISHA 15/30, all `adhan: true, notif: true`. These become the shipped defaults (§8.3).
- Breakpoints: `large 120×35`, `medium 100×30`, `small 80×24` — and the pixel dimensions (936×683, 780×585, 624×468) divide to an exact **7.8 × 19.5 px cell**, confirming the design was authored on a real character grid.
- Countdown format `HH:MM:SS` zero-padded, e.g. `01:42:37`.
- Progress gauge: `Asr 16:18 ─── 62% ─── Maghrib 19:21`, i.e. elapsed fraction between the *previous* and *next* prayer.
- Config path shown to the user: `~/.config/salat/config.toml`.
- Retry policy shown to the user: `retried 3× (1s · 2s · 4s)`.
- Error identity shown to the user: `error[E001]: geocoding failed` — the design mandates a **numbered error taxonomy**, which §18 formalises.
- Startup sequence shown to the user: **Loading configuration → Resolving location → Calculating prayer times → Initializing audio → Starting prayer service** — the design mandates a **five-stage boot state machine**, which §15.7 formalises.
- Stepper semantics differ by screen: Notification Settings steps by **±1** via `←→`; Prayer Configuration steps by **±5** via `◂ 5` / `5 ▸` buttons. Both clamp to `0..=60`.
- `Badge` map (the non-color-only encoding the polish brief demands): `passed ✓ "passed" textDim` · `completed ✓ "completed" green` · `next ● "next" green` · `pre-adhan ◈ "soon" yellow` · `adhan ◉ "adhan" red` · `post-adhan ◉ "reminder" red` · `upcoming ○ "upcoming" textSub`.
- Status screen components, in order: Prayer service, Prayer calculation, Location, Timezone, Notifications, Adhan sound, Audio system, Network, Config file, Prayer database. **Ten, and they define the status domain exactly** (§15.8).
- Status "TODAY" statistics: Total prayers, Passed, Remaining, Notifications sent, Adhan plays, Next notify.
- Event log rows carry `time │ label` + a dim sub-line, colored by category: yellow = notification, blue = Adhan played, dim = state change.

### 3.6 Ambiguities and their resolutions

The brief requires each ambiguity to be named with the smallest practical implementation that preserves the design. Ten were found.

**A1 — Display type sizes cannot exist in a terminal.** The countdown is `2.4em`–`2.8em` and the Adhan prayer name is `3em`.
*Resolution:* implement `ui::widgets::bigtext`, a 3-row half-block glyph renderer covering `0-9`, `:`, and A–Z uppercase (the only characters needed). Use it for the countdown at 100×30 and 120×35, and for `MAGHRIB` on the Adhan overlay at every size. At 80×24 the row budget (§4.4) does not permit 3 rows for the countdown, so render it single-row `BOLD` + `textBright` — which is exactly the "secondary information collapses" rule the polish brief states, applied to the primary element's *treatment* rather than its presence. Rejected: a figlet-style 6-row renderer (too tall for any budget); scaling via terminal escape sequences (not portable).

**A2 — Sunrise is required in the domain but appears in no table.** The brief mandates Sunrise; the design's table has five rows at all sizes.
*Resolution:* Sunrise is a first-class `PrayerTime` in the domain (it bounds the Fajr window and is needed for the Status screen and future features) but is **not notifiable and not rendered as a table row**. It appears only in the `TONIGHT`/tomorrow context and in `salat status` output. Do not add a sixth row.

**A3 — `completed` is user-set but no key is shown.** The States screen says "User marked as completed", and no footer on any screen shows a completion key.
*Resolution:* make the dashboard's TODAY table focusable with `↑↓` (a cursor only — no layout change) and bind `c` to toggle completion on the selected prayer, persisted in `prayer_completions`. This adds one keybinding and zero pixels. The alternative — ship the state unreachable — was rejected because the design documents it as a real state with a real appearance. Footer space at 80×24 does not permit advertising it; the States reference screen documents it (see A7).

**A4 — `[ SEARCH CITY ]` has no backing endpoint.** AlAdhan has no geocode-only endpoint, and `timingsByCity` returns placeholder coordinates ✅.
*Resolution:* embed GeoNames `cities15000` (~25,000 cities ≥ 15,000 population) as a packed binary asset with offline substring search. This produces exactly the design's 3-result list with real coordinates and real IANA timezones, works offline, and eliminates the byCity endpoint entirely so the API layer only ever uses the well-behaved coordinate endpoints. Attribution (CC-BY 4.0) in README and `--help`. Rejected: Nominatim/OSM (new external service, usage policy, privacy); coordinates-only (contradicts the design).

**A5 — `[ AUTO-DETECT ]` has no defined provider.**
*Resolution:* resolve the IANA zone from the system (`/etc/localtime` symlink → `$TZ` → `timedatectl show -p Timezone`), then select the highest-population city in that zone from the embedded index. Present it as a *proposal* the user confirms with `Enter`, with the honest sub-label `detected from system timezone · refine with COORDINATES`. Zero network, zero privacy exposure. Rejected for v1: GeoClue2 (frequently absent headless, needs an agent), IP geolocation (third-party service, privacy).

**A6 — `Escape` behaviour contradicts itself.** The prototype's root handler sends `Escape` to `dashboard` from any screen, but the Location screen shows the breadcrumb `SETTINGS › LOCATION` and the footer says `ESC back`.
*Resolution:* `Esc` pops a navigation stack (`location → settings → dashboard`). The breadcrumb and footer express intent; the flat handler is prototype convenience. Overlays are dismissed independently and restore whatever was beneath.

**A7 — The `states` screen has no reachable entry point.** It is navigable only via the prototype's screen picker, and the footer has no fifth tab.
*Resolution:* bind it to `?` as a reference screen from any main screen, with the breadcrumb `PRAYER STATES` and `ESC back`. It is the natural home for documenting the `c` binding from A3. No footer slot is consumed at 80×24.

**A8 — Quick mode is not illustrated.** The design shows only the full TUI.
*Resolution:* derive it from the 80×24 dashboard: the NEXT PRAYER panel and the TODAY table, one header line, no footer, no tab bar, rendered into an inline viewport. Nothing invented (§15.3). **§16.14 specifies it completely** — the row budget and its arithmetic, the compression that fits §16.1's 18 rows into 13 viewport rows, the degradation ladder below 15 terminal rows, and the tick, rollover and config-watch behaviour. §16.15 does the same for the two overlays inside that viewport.

**A9 — Single-valued enumerations.** `Language → English` and `Theme → Midnight Blue` are the only values shown.
*Resolution:* model both as enums with exactly one variant in v1. The setting rows render and are selectable (matching the design) but `←→` cycles a one-element list, i.e. it is a no-op. Do not build an i18n framework or a theme loader. The Arabic `اللَّهُ أَكْبَرُ · اللَّهُ أَكْبَرُ` on the Adhan screen is a fixed literal, not localised content.

**A10 — Inconsistent snooze durations and an unexplained sub-label.** Adhan snooze is `3m`, mosque reminder snooze is `5m`; the Calculation method row carries the sub-label `MWL fallback`.
*Resolution:* implement both snooze values exactly as shown (they are defensible: an Adhan re-alert should be tighter than a departure reminder) and expose them as constants, not settings. Interpret `MWL fallback` as informational text meaning *method 3 (Muslim World League) is used if the configured method is rejected by the API*; implement it as literal sub-text plus the actual fallback behaviour in §7.7.

**One addition not in the design.** Below 80×24 the design specifies nothing. Implement a single centered panel: `TERMINAL TOO SMALL` / `salat needs 80×24 · this terminal is 74×20` / `resize to continue`. It is unavoidable and minimal.

---

## 4. Architecture overview

### 4.1 Shape

Salat is one process, one binary, one library crate. It is not a client/server system: there is no IPC, no socket protocol, no daemon handshake. The *same* scheduler code runs inside the TUI process and inside `salat daemon`; which one owns scheduling at any moment is decided by a lock file, not by a protocol (§4.3).

```
                         ┌──────────────────────────────────────┐
   terminal ───────────► │  cli::  parse → dispatch             │
                         └───────┬──────────────────────────────┘
                                 │ one of: quick · full · config · status
                                 │         test-adhan · daemon
                         ┌───────▼──────────────────────────────┐
                         │  app::Runtime   (owns every service) │
                         └───┬───────────────┬──────────────┬───┘
             watch<Config>   │        mpsc   │              │  1 Hz + 250 ms
                             │  ServiceEvent │              │
        ┌────────────────────▼───┐   ┌───────▼──────────┐  ┌▼──────────────┐
        │ scheduler::Runner      │   │ ui::Shell        │  │ status::       │
        │  plan of instants      │   │  screens+widgets │  │  Collector    │
        └──┬────────┬────────┬───┘   └──────────────────┘  └───────────────┘
           │        │        │
    ┌──────▼──┐ ┌───▼─────┐ ┌▼────────┐
    │ notify::│ │ audio:: │ │ cache:: │
    │ Desktop │ │ Player  │ │ EventLog│
    └─────────┘ └─────────┘ └─────────┘

        ┌──────────────────────────────────────────────────┐
        │ domain::  pure functions — no I/O, no ambient now │
        └──────────────────────────────────────────────────┘
              ▲                    ▲                  ▲
     ┌────────┴──────┐   ┌─────────┴────────┐  ┌──────┴──────────┐
     │ api::AlAdhan  │   │ cache::PrayerDays│  │ config::Store   │
     │  reqwest      │   │  sqlx/SQLite     │  │  TOML + watch   │
     └───────────────┘   └──────────────────┘  └─────────────────┘
```

### 4.2 Layering rule (enforced by review, and by module visibility)

1. **`domain`** — pure. No `reqwest`, no `sqlx`, no `std::time::SystemTime`, no `Local::now()`. Takes a `&dyn Clock` and plain data; returns plain data. Every rule about states, next-prayer selection, countdowns, windows and schedules lives here and nowhere else.
2. **Services** (`api`, `cache`, `config`, `location`, `notify`, `audio`, `scheduler`, `status`) — own exactly one external concern each, expose a narrow trait, and depend on `domain` but never on `ui` or `cli`.
3. **`ui`** — renders an immutable snapshot. Contains no business rules. Cannot mutate config directly; it edits a *draft* and asks `config::Store` to commit.
4. **`cli`** — argument parsing and dispatch only. Each subcommand is a thin function that builds a `Runtime` and calls one service.
5. **`app::Runtime`** — the only place where services are constructed and wired. This is what prevents the "giant application state object": the runtime holds `Arc<dyn Service>` handles; the UI holds a `UiModel` snapshot; neither holds the other's internals.

### 4.3 Foreground / daemon coordination

Notifications and Adhan must fire when no terminal is open, so a background service is necessary. But two schedulers must never double-fire.

- `salat daemon` acquires an exclusive advisory lock on `$XDG_RUNTIME_DIR/salat/scheduler.lock` (fallback `/tmp/salat-$UID/scheduler.lock`) and runs scheduling headlessly.
- `salat` and `salat --full` **try** the same lock. If they get it, they schedule. If they don't, they run in *observer* mode: they still render live state and countdowns, but they do not fire notifications or play Adhan, and the Status screen reports `Prayer service: RUNNING (daemon)` instead of `RUNNING (foreground)`.
- The Adhan and Mosque overlays are UI reactions to scheduled instants, computed from the *state machine* rather than from an event message, so an observer TUI still shows them at the right second — it simply doesn't own the audio or the notification.

This is a 30-line coordination mechanism with no protocol, no versioning, and no failure mode worse than "notifications come from the other process".

### 4.4 Terminal row budget (the constraint that drives every layout)

| Region | Rows |
|---|---|
| Header (content + rule) | 2 |
| Footer (rule + content) | 2 |
| **Body at 80×24** | **20** |
| Body at 100×30 | 26 |
| Body at 120×35 | 31 |

The 80×24 dashboard therefore has 20 rows to spend, and the verified allocation is:

```
NEXT PRAYER panel      6   (border 1 · "● MAGHRIB · 19:21" 1 · countdown 1
                            · "REMAINING" 1 · gauge 1 · border 1)
TODAY panel           11   (border 1 · head 1 · rule 1 · 5 prayers
                            · rule 1 · "Hijri … Qibla …" 1 · border 1)
service strip          1
slack                  2
                     ────
                      20   ✓
```

This is why A1 forces a single-row countdown at 80×24: a 3-row countdown makes the total exactly 20 with zero slack, which breaks the moment a status line wraps.

### 4.5 Breakpoint classification

```
large   if  w >= 120 && h >= 35
medium  if  w >= 100 && h >= 30
small   if  w >=  80 && h >= 24
too_small otherwise
```

`Breakpoint` is computed once per frame from `frame.area()` and threaded to every screen. Layout is recomputed every frame from the current area, which makes resize handling free — there is no cached geometry to invalidate.

---

## 5. Technology stack and dependency decisions

### 5.1 Runtime dependencies

| Crate | Version | Features | Where used | Why this and not the alternative |
|---|---|---|---|---|
| `ratatui` | 0.30 | default (`all-widgets, crossterm, layout-cache, macros, underline-color`) | `ui::` | Mandated. 0.30 is split into `ratatui-core`/`ratatui-widgets`/`ratatui-crossterm`; depending on the facade with default features pulls a consistent set and exposes `ratatui::crossterm`. |
| `crossterm` | 0.29 | `event-stream` | `ui::input`, terminal setup | Mandated, and a direct dependency is required because `EventStream` sits behind `event-stream`, which `ratatui`'s default set does not enable. **Must match the version `ratatui-crossterm` resolves** (it accepts `^0.28` and `^0.29`) or two crossterms end up linked and terminal state corrupts. |
| `tokio` | 1.53 | `rt-multi-thread, macros, time, sync, signal, process, fs, io-util` | everywhere async | Mandated. `process` for the audio player, `signal` for SIGINT/SIGTERM/SIGHUP, `sync` for `watch`/`mpsc`. |
| `reqwest` | 0.13 | `default-features = false`, `json, rustls, gzip, hickory-dns` | `api::` | Mandated. The `rustls` feature (named `rustls-tls` before 0.13 — corrected by the Phase 0 `cargo add --dry-run` sweep) removes OpenSSL, which is what makes a static musl build possible. `hickory-dns` replaces musl's resolver, which mishandles large DNS responses and multi-entry `search` domains. |
| `serde` / `serde_json` | 1.0 | `derive` | `api::dto`, cache blobs | Mandated. |
| `chrono` | 0.4 | `default-features = false`, `clock, std, serde` | `domain::`, everywhere | Mandated. `DateTime<FixedOffset>` from ISO-8601, arithmetic, formatting. |
| `chrono-tz` | 0.10 | `serde` | `domain::`, `location::` | Mandated. IANA zone → `Tz`, DST-correct local conversion. |
| `clap` | 4.6 | `derive, wrap_help` | `cli::` | Mandated. |
| `sqlx` | 0.9 | `default-features = false`, `sqlite, runtime-tokio, tls-none, macros, migrate, chrono` | `cache::` | Mandated. Bundled `libsqlite3-sys` compiles into the binary, so no system SQLite is required — essential for a portable static artifact. `tls-none` because SQLite has no network. |
| `notify-rust` | 4.18 | default | `notify::` | Mandated. Pure-Rust `zbus` backend, so musl-compatible and no `libdbus` linkage. |
| `thiserror` | 2.0 | — | every module's error type | Typed domain errors with zero boilerplate. |
| `color-eyre` | 0.6 | — | `main.rs` **only** | Panic and error reporting at the process boundary, with a hook that restores the terminal before printing. Chosen over `anyhow` because the panic hook is the actual requirement; using both would be two wrapping crates for one job. |
| `tracing` / `tracing-subscriber` | 0.1 / 0.3 | `fmt, env-filter` | everywhere | Structured logging to a **file**, never to stdout while the TUI owns the screen. |
| `directories` | 6.0 | — | `config::`, `cache::` | XDG base directories, correctly, including `$XDG_RUNTIME_DIR`. |
| `toml` | 1.1 | — | `config::` | Config format the design displays. |
| `unicode-width` | 0.2 | — | `ui::` | Centering the Arabic line and any label whose display width ≠ char count. |
| `is-terminal` | 0.4 | — | `cli::` | The §14.3 dispatch needs one `IsTerminal` check on stdout. A dependency rather than `std::io::IsTerminal` because the plan mandates the crate (Phase 0 §23); functionally the two are the same check. |

### 5.2 Development dependencies

| Crate | Purpose |
|---|---|
| `wiremock` | AlAdhan HTTP fixtures: success, 400-with-string-`data`, 503, malformed JSON, 304, timeout |
| `insta` | Golden snapshots of `TestBackend` frames at 80×24, 100×30, 120×35 |
| `assert_cmd` + `predicates` | CLI surface: `--help`, `--version`, `status`, `config`, exit codes |
| `tempfile` | Isolated `XDG_*` roots per test |

### 5.3 Dependencies deliberately rejected

| Rejected | Reason |
|---|---|
| `rodio` / `cpal` | Links ALSA (`libasound2`), which forbids a self-contained static binary and adds a build-time system dependency. An external player also gives free PipeWire/PulseAudio routing and a trivial stop (kill the child). |
| `figment` | Config layering across many sources is not needed: one TOML file plus a handful of CLI overrides. |
| A second HTTP/geocoding service | The embedded city index (A4) removes the need, along with its rate limits, usage policy and privacy exposure. |
| `once_cell` | `std::sync::OnceLock` is sufficient on the pinned toolchain. |
| `cargo-dist` | It would own the installer and CI, but the brief specifies `install.sh` at the repository root behind an exact raw URL. A 150-line auditable script is more transparent for a security-sensitive install path. Named as the migration path if release engineering grows. |
| `anyhow` | Superseded by `color-eyre` at the single boundary that needs it. |

### 5.4 Toolchain

`rust-toolchain.toml` pinning a stable channel with `rustfmt` and `clippy`. Edition 2024. MSRV recorded in `Cargo.toml` and asserted by a CI job. Workspace of two members: `salat` (lib + bin) and `xtask` (regenerates `assets/cities.bin`, captures README screenshots) — `xtask` is never a dependency of `salat`.

> **Verification note.** Feature names for `reqwest` 0.13 and `sqlx` 0.9 were read from crates.io during planning but **must** be re-confirmed by `cargo add --dry-run` in Phase 0 — item 10 of that phase makes the sweep a gate condition and item 12 makes correcting this table part of its DoD — because feature renames across minor versions are the single most common cause of a first-build failure.

---

## 6. Domain model

All of `domain` is pure and takes time as a parameter. This section defines the types precisely enough to implement; field names are normative.

### 6.1 Prayer identity

```
enum Prayer      { Fajr, Sunrise, Dhuhr, Asr, Maghrib, Isha }
enum Notifiable  { Fajr, Dhuhr, Asr, Maghrib, Isha }        // Sunrise excluded — A2
```

`Prayer` carries `fn display(&self) -> &'static str` returning `FAJR` … `ISHA`, `fn short(&self)` returning `FAJ SUN DHU ASR MAG ISH`, and `fn notifiable(&self) -> Option<Notifiable>`. `Notifiable` is the key type for per-prayer configuration, the scheduler, and the config file — which makes it a *type error* to configure a notification for Sunrise.

### 6.2 A day of prayer times

```
struct PrayerDay {
    date:        NaiveDate,          // civil date in `tz`
    tz:          Tz,                 // IANA zone this day was computed in
    times:       [DateTime<Tz>; 6],  // indexed by Prayer
    midnight:    DateTime<Tz>,       // may be on date+1
    last_third:  DateTime<Tz>,       // may be on date+1
    hijri:       HijriDate,          // { day, month_number, month_en, month_ar, year }
    meta:        DayMeta,            // { method_id, method_name, school, lat, lng, tz_name }
}
```

`DayMeta` exists so the Status and Settings screens can display *what the API actually used* rather than what we asked for — the two can differ (§7.7).

### 6.3 The rolling schedule window

Single-day data is insufficient: after Isha, "next prayer" is tomorrow's Fajr, and the design's TONIGHT panel prints `Fajr tomorrow 05:11`.

```
struct PrayerWindow {
    today:     PrayerDay,
    tomorrow:  PrayerDay,
    yesterday: Option<PrayerDay>,   // for "previous prayer" before Fajr
}
```

`PrayerWindow::instants(&self) -> Vec<PrayerInstant>` yields a chronologically sorted list of every notifiable prayer instant across yesterday → tomorrow. This one function is the input to next-prayer selection, gauge computation and schedule planning, so all three agree by construction.

```
struct PrayerInstant { prayer: Notifiable, at: DateTime<Tz>, date: NaiveDate }
```

### 6.4 State

```
enum PrayerState { Upcoming, Next, PreAdhan, Adhan, PostAdhan, Passed, Completed }
```

Names map to the design's `PrayerState` union: `pre-adhan → PreAdhan`, `post-adhan → PostAdhan`. Rendering metadata (`glyph`, `label`, `fg`, `bg`, `border`) lives in `ui::theme`, not in the domain — the domain must not know about colors.

### 6.5 Derived values

```
fn next_prayer(window, now)      -> Option<PrayerInstant>
fn previous_prayer(window, now)  -> Option<PrayerInstant>
fn countdown(target, now)        -> Countdown        // { hours, minutes, seconds, total_secs }
fn progress(prev, next, now)     -> f64             // 0.0..=1.0, the design's 62%
fn state_of(instant, cfg, now, completions) -> PrayerState
fn day_summary(window, now, cfg) -> DaySummary      // total/passed/remaining/next_notify
```

`Countdown::format()` produces `HH:MM:SS` with hours unbounded (`26:03:11` is valid and appears when the app is left open overnight without data for the day after tomorrow — degenerate but correct rather than panicking).

### 6.6 Time source

```
trait Clock: Send + Sync {
    fn now_utc(&self) -> DateTime<Utc>;
    fn now_in(&self, tz: Tz) -> DateTime<Tz> { self.now_utc().with_timezone(&tz) }
}
struct SystemClock;                       // Utc::now()
struct MockClock { now: Mutex<DateTime<Utc>> }  // set(), advance()
```

**No code outside `SystemClock` may call `Utc::now()`, `Local::now()` or `SystemTime::now()`.** Enforce with a clippy `disallowed_methods` entry in `clippy.toml`. This is the single most important testability decision in the plan: it turns "does the state machine handle the second Maghrib begins" from a flaky integration test into a two-line unit test.

### 6.7 Status domain (decoupled, per the brief)

```
enum Health { Ok, Degraded, Down, Unknown }
struct Component { id: ComponentId, label: &'static str, health: Health, detail: String }
struct SystemStatus { components: Vec<Component>, next_event: Option<NextEvent>,
                      today: DaySummary, recent: Vec<LoggedEvent> }
```

`ComponentId` has exactly the ten variants the design lists. Each service implements `fn report(&self) -> Component` for itself; `status::Collector` assembles them. Neither the Status screen nor `salat status` may reach into any service — they consume `SystemStatus` only. Adding a component means adding a variant and a `report`, and both consumers pick it up automatically.

---

## 7. AlAdhan API integration

Everything marked ✅ was verified against `https://api.aladhan.com` during planning.

### 7.1 Endpoints used

| Endpoint | Role |
|---|---|
| `GET /v1/calendar/{year}/{month}` | **Primary.** One month of timings. ~44 KB ✅ |
| `GET /v1/timings/{DD-MM-YYYY}` | Targeted single-day refresh (post-config-change verification, `--refresh`) ✅ |
| `GET /v1/qibla/{lat}/{lng}` | One-shot Qibla degrees, cached indefinitely per location ✅ (`/v1/qibla/36.19/5.41` → `106.78251860469342`) |
| `GET /v1/methods` | Populate and validate the calculation-method list ✅ |

Not used: `timingsByCity` / `timingsByAddress` (placeholder coordinates — A4), `calendar?annual=true` (528,646 bytes ✅ — too large for the benefit), `/v1/nextPrayer` (next-prayer selection is local, as the brief requires).

### 7.2 The canonical request

```
GET /v1/calendar/{year}/{month}
      ?latitude={lat}&longitude={lng}
      &method={id}&school={0|1}
      &latitudeAdjustmentMethod={1|2|3}&midnightMode={0|1}
      &timezonestring={IANA}
      &tune={imsak,fajr,sunrise,dhuhr,asr,maghrib,sunset,isha,midnight}
      &iso8601=true
      [&shafaq={general|ahmer|abyad}]     // only when method == 15
```

**`iso8601=true` is mandatory on every request.** Verified consequences ✅:

- Times arrive as `2026-09-05T05:16:00+01:00` instead of `05:16`, so parsing is `DateTime::parse_from_rfc3339`, and the day's UTC offset is authoritative per date — DST is handled by the API and we never assemble a timestamp by hand.
- `Midnight` correctly rolls to the next civil day: `2026-09-06T00:37:00+01:00`.
- It sidesteps a real parser trap: **without** `iso8601`, calendar responses append a zone suffix (`"05:12 (CET)"`) while single-day `timings` responses do not ✅. Two formats from one API, avoided entirely.

### 7.3 Response handling

Success shape: `{ code: 200, status: "OK", data: [ { timings: {...}, date: {...}, meta: {...} }, … ] }`.

**`data` is polymorphic across success and failure** ✅: an object/array on success, a **bare string** on error — `{"code":400,"status":"BAD_REQUEST","data":"Unable to geocode address: zzzznotacity, zzzz"}`. Therefore deserialize in two stages: parse into an `ApiEnvelope { code: u16, status: String, data: serde_json::Value }`, branch on `code`, and only then deserialize `data` into the typed DTO. An `#[serde(untagged)]` enum would also work but produces uninterpretable errors when both arms fail.

**Do not deserialize `meta.offset`.** With `tune` set, it echoes mixed types in one object — integer `0` alongside string `"3"` ✅ — so a typed struct fails. Skip the field; we already know what we sent.

### 7.4 Validation that must happen client-side

Two verified API behaviours make client-side validation a correctness requirement, not politeness:

- **Invalid latitude returns HTTP 503, not 400** ✅. `latitude=999` yields `"Timezone lookup is temporarily unavailable. Please try again later."` — a user typing a bad coordinate would be shown a fabricated outage. Validate `-90 ≤ lat ≤ 90`, `-180 ≤ lng ≤ 180` before any request, with error `E101`.
- **An invalid method returns HTTP 200 with silently wrong times** ✅. `method=777` succeeds and returns different, incorrect timings. Validate `method ∈ {0..=5, 7..=23, 99}` (the set from `/v1/methods`) before any request, with error `E102`. This is a data-integrity hazard: an unvalidated typo produces plausible prayer times that are wrong all year.
  - *Corrected 2026-09-05 (Phase 2 fixture capture):* the planning-time set `{0..=15, 17..=23, 99}` had drifted from the API. `/v1/methods` now lists no method `6` and does list `16` (Dubai, experimental); verified live that `method=6` is **silently remapped to ISNA** (meta echoes ISNA, not the sent id — precisely this section's hazard), while `method=16` answers with matching Dubai times. The committed `tests/fixtures/aladhan/methods.json` is the evidence, and an api unit test asserts the whitelist equals its id set exactly, so the next drift fails a test instead of shipping wrong times.

Also validate `school ∈ {0,1}`, `latitudeAdjustmentMethod ∈ {1,2,3}`, `midnightMode ∈ {0,1}`, each `tune` element in `-60..=60`, and `timezonestring` parseable by `chrono_tz`.

### 7.5 Transport policy

- HTTPS only; `rustls` with certificate verification on; redirects limited to 2 and rejected if the host is not `api.aladhan.com`.
- Timeouts: 10 s connect, 20 s total. Response body capped at 2 MiB (a legitimate monthly response is ~44 KB; the cap bounds memory against a hostile or broken response).
- `User-Agent: salat/{version} (+https://github.com/linuxcoding-ADAM/Salat)`.
- **Retry exactly as the design promises the user:** 3 attempts with 1 s · 2 s · 4 s backoff, ±20 % jitter. Retry only on connect/timeout/`5xx`/`429`. Never retry `4xx` other than `429`.
- Rate limit is **12 requests/second** ✅ (`ratelimit-limit: 12`, `x-ratelimit-limit-second: 12`). Salat's steady state is ~12 requests/year/config, so a single-permit `Semaphore` plus a 100 ms minimum inter-request gap is sufficient headroom.
- Responses carry `cache-control: public,max-age=3600` and an `etag`, and `If-None-Match` correctly returns **HTTP 304** ✅. Store the ETag; use conditional revalidation on explicit refresh only. Prayer times for a past-or-present civil date do not change, so routine revalidation is pointless.

### 7.6 Fetch strategy

```
ensure_window(today):
    need = { month_of(today), month_of(today + 1 day) }     // month-end needs tomorrow
    for m in need:
        if cache has complete month m for current config key: continue
        fetch calendar(m)  →  validate  →  store all days atomically
    load today / tomorrow / yesterday from cache
```

Triggered at boot (stage 3 of §15.7), at local-midnight rollover, and on any config change that alters the cache key. Prefetching next month whenever `today` is within 3 days of month end guarantees tomorrow's Fajr is always available offline.

### 7.7 Method fallback (`MWL fallback`, A10)

If a calendar request returns `4xx` specifically because of the method parameter, retry once with `method=3` (Muslim World League), surface a `Degraded` health detail `method rejected · using MWL`, and record it in the event log. The Settings screen's `MWL fallback` sub-label then describes real behaviour. Method **12 = "Union Organization Islamic de France"** ✅ is exactly the design's displayed `Union of Islamic Orgs`, so it is the shipped default.

### 7.8 Fixtures

Every probed response is committed to `tests/fixtures/aladhan/` as JSON: `calendar_ok.json`, `timings_ok.json`, `error_geocode_400.json`, `error_latitude_503.json`, `methods.json`, `qibla.json`, `malformed_truncated.json`. Fixtures are recorded evidence, not hand-written guesses, and they are what makes §20's API tests meaningful.

---

## 8. Configuration model

### 8.1 Location and format

`~/.config/salat/config.toml` — the exact path the design's SAVED screen displays. Resolved via `directories::ProjectDirs` so `$XDG_CONFIG_HOME` is honoured. Overridable with `--config <path>` for testing and for users running multiple locations.

File permissions `0600`, directory `0700`. The file records a physical location, which is personal data even though it is not a secret.

**`--config` scopes every shared resource, not just the config file.** "Users running multiple locations" is only coherent if two Salat processes with different `--config` paths do not contend for one lock, one database, or one idempotency ledger — otherwise the second location would be locked out of scheduling, would overwrite the first's cache rows, and would find its own Adhan already marked fired. The scoping key is derived once at startup:

```
scope = "default"                                  // the XDG default path
scope = "<stem>-<8 hex>"                           // any --config path
         │        └── FNV-1a 64-bit of the canonicalized path, low 32 bits
         └── the file stem, lowercased, [a-z0-9_-] only, truncated to 16 chars
```

Two properties are needed and neither requires a dependency. The path is canonicalized first (`std::fs::canonicalize`, falling back to lexical normalization when the file does not yet exist) so `--config ./mecca.toml` and `--config /home/u/mecca.toml` are one instance rather than two. The digest is **FNV-1a**, hand-written in about five lines with the standard 64-bit constants (offset basis `0xcbf29ce484222325`, prime `0x100000001b3`), because it must be stable *forever* — a scope key that changed would give two processes different lock files and let both schedule, which is precisely the duplicate-Adhan failure §4.3 exists to prevent. `std::hash::DefaultHasher` is therefore unusable here: std documents its output as unstable across Rust releases, so rebuilding with a newer toolchain would silently rename the lock. No hashing crate is added; §17.2 already establishes this document's preference for keys you can read, and the stem is there for exactly that reason — `scheduler-mecca-75b6780d.lock` tells a user which config it belongs to, which a bare digest would not. That is the same `/home/u/mecca.toml` used in the table below and in §8.1's `salat config` specimen, and `75b6780d` is its actual FNV-1a low word; every example of this path in the document carries that one value.

The key then scopes exactly three things:

| Resource | Default path | With `--config /home/u/mecca.toml` |
|---|---|---|
| Process lock | `$XDG_RUNTIME_DIR/salat/scheduler.lock` | `$XDG_RUNTIME_DIR/salat/scheduler-<scope>.lock` |
| SQLite database | `$XDG_DATA_HOME/salat/salat.db` | `$XDG_DATA_HOME/salat/salat-<scope>.db` |
| `fired_events` rows | `scope = 'default'` | `scope = '<scope>'` |

The ledger is scoped by a **fourth primary-key column** rather than by a separate file, so `fired_events` becomes `PRIMARY KEY (scope, date, prayer, kind)` (§11.4). A separate database already isolates it, so the column is redundant for `--config` alone — but it is what makes the ledger correct when the *location inside one config* changes, which is the case a separate file cannot cover. The `days` and `qibla` cache tables are already keyed by a `cache_key` that includes the location and method (§17.2), so they need no additional scoping.

**Multiple processes, different config paths.** Each is fully independent: its own lock (so both can schedule), its own database, its own ledger scope. Two locations therefore both fire, which is the intended behaviour — a user tracking Mecca alongside Sétif wants both calls. Each still enforces single-scheduler arbitration *within* its own key, so a daemon and a foreground TUI sharing one `--config` behave exactly as §4.3 describes, with the loser in observer mode. **Multiple processes with the same config path share all three resources**, which is the existing §4.3 case and is unchanged.

One consequence to state plainly: the README's documented `rm ~/.local/share/salat/salat.db` — which §14.4 keeps as a documented `rm` rather than a `salat cache clear` subcommand — deletes the *default* database only. With `--config`, the file to remove is `salat-<scope>.db`.

So that no user ever has to compute a scope by hand, `salat config` (§14.1) prints the resolution before the TOML it already dumps — four lines, then a blank line, then the effective configuration:

```
scope:    mecca-75b6780d
config:   /home/u/mecca.toml
database: /home/u/.local/share/salat/salat-mecca-75b6780d.db
lock:     /run/user/1000/salat/scheduler-mecca-75b6780d.lock

[general]
…
```

Those values are reproducible, not illustrative: `75b6780d` is the low 32 bits of the FNV-1a 64-bit hash of the canonicalized `/home/u/mecca.toml`, and a unit test asserts exactly that pair. Under the default path the same four lines print `scope: default` and the three unsuffixed paths. This is the only place any resolved path is printed. **`salat status` is unaffected** — it renders the ten components of §6.7 and carries no path field; the "Config file" and "Prayer database" components report *health*, not location, exactly as the design's Status screen shows them.

### 8.2 Schema

```toml
version = 1                              # schema version, for future migrations

[location]
label      = "Sétif, Algeria"            # display string, Location screen "City"
country    = "Algeria"
latitude   = 36.1900                     # -90..=90
longitude  = 5.4100                      # -180..=180
timezone   = "Africa/Algiers"            # IANA, must parse via chrono-tz
source     = "city"                      # "city" | "coordinates" | "auto"

[calculation]
method                     = 12          # AlAdhan method id, whitelisted (§7.4)
school                     = 0           # 0 Shafi, 1 Hanafi
latitude_adjustment_method = 3           # 1 MidNight, 2 OneSeventh, 3 AngleBased
midnight_mode              = 0           # 0 Standard, 1 Jafari
shafaq                     = "general"   # only used when method == 15
tune = { imsak = 0, fajr = 0, sunrise = 0, dhuhr = 0, asr = 0,
         maghrib = 0, sunset = 0, isha = 0, midnight = 0 }   # each -60..=60

[notifications]
enabled         = true                   # Settings → Enable notifications
mosque_reminder = true                   # Settings → Mosque reminder

[adhan]
enabled = true                           # Settings → Enable adhan
preset  = "abdul-basit"                  # resolves to $XDG_DATA_HOME/salat/adhan/abdul-basit.mp3
file    = ""                             # absolute path; overrides preset when non-empty
volume  = 80                             # 0..=100

[display]
time_format        = "24h"               # "24h" | "12h"
language           = "en"                # A9: single variant
theme              = "midnight-blue"     # A9: single variant
compact_mode       = false               # Settings → Compact mode
show_seconds       = true                # Settings → Show seconds
countdown_progress = true                # Settings → Countdown progress
color_mode         = "auto"              # "auto" | "truecolor" | "256" | "ansi"

[prayers.fajr]                           # one table per Notifiable prayer
before_minutes = 15                      # 0..=60
after_minutes  = 20                      # 0..=60
adhan          = true
notify         = true
[prayers.dhuhr]   before_minutes = 10  after_minutes = 30  adhan = true  notify = true
[prayers.asr]     before_minutes = 15  after_minutes = 20  adhan = true  notify = true
[prayers.maghrib] before_minutes =  5  after_minutes = 15  adhan = true  notify = true
[prayers.isha]    before_minutes = 15  after_minutes = 30  adhan = true  notify = true
```

Every field above is traceable to a control the Figma design shows, with three exceptions justified explicitly: `version` (migration safety), `color_mode` (terminal capability, not a design choice), and `location.source` (needed to render the Location screen's active mode). Nothing else was added.

### 8.3 Defaults

Defaults are the design's own mock data, so a fresh install renders the screens the designer drew: location Sétif/Algeria, method 12, school 0, `latitudeAdjustmentMethod` 3, all `tune` zero, notifications and Adhan enabled, volume 80, 24h, seconds on, progress on, and the five per-prayer pairs 15/20 · 10/30 · 15/20 · 5/15 · 15/30.

A missing config file is not an error: `Config::default()` is written on first run and stage 1 of the boot sequence reports `Loading configuration ✓ (created default)`.

### 8.4 Validation

Two levels, and the distinction matters:

- **Structural** (`toml` deserialization): unknown keys are *rejected*, not ignored — `#[serde(deny_unknown_fields)]` — so a typo becomes a clear error instead of a silently ignored setting.
- **Semantic** (`Config::validate() -> Result<(), Vec<ConfigError>>`): coordinate ranges, method whitelist, enum domains, `0..=60` clamps, volume `0..=100`, timezone parseability, and — if `adhan.file` is non-empty — that it is an absolute path to an existing regular file.

`validate()` returns **all** violations, not the first, because the error screen should tell a hand-editing user everything that is wrong in one pass. Invalid config at boot → `E201` on the error screen with the offending keys and the option `[R] RETRY` (re-read the file, so the user can fix it in another terminal and continue) or `[D] USE DEFAULTS` (in memory only; nothing is overwritten without consent).

The TUI's editors make invalid states unreachable — steppers clamp, toggles are boolean, method comes from a list — so semantic validation exists mainly to defend against hand-editing.

**The `version` field, and what happens to other values of it.** §8.2's `version = 1` is checked before any other validation, structural or semantic, because a file from a different schema cannot be meaningfully validated against this one:

| Value | Behaviour |
|---|---|
| `1` | Accepted. The only version v1 understands. |
| `> 1` | **Rejected** — `E201`, with the detail line `configuration version 2 was written by a newer salat; this build understands version 1`. Not migrated, not defaulted, not silently downgraded. |
| absent | Rejected by `deny_unknown_fields`' counterpart — `version` is a required field, so a missing one is a structural error naming `version`. |
| `< 1`, non-integer | Rejected — `E201`, same code, detail line naming `version`. |

Rejecting a *newer* file rather than trying to read it is forced by `deny_unknown_fields`: a v2 file almost certainly carries a key v1 does not know, so a downgrade attempt fails structurally anyway, and it would fail with a confusing "unknown field" message instead of an honest "this file is newer than this build". The rejection is also **non-destructive** — `[D] USE DEFAULTS` runs in memory and `ConfigStore::save` is never called on a rejected file, so downgrading a Salat install and running it once cannot destroy the newer config. `[R] RETRY` re-reads, so the user can restore a backup in another terminal and continue.

There is no `version < 1` in existence, so no backward migration is defined and none is needed. The forward rule for any future bump is stated here as a constraint on later work: **a version is not accepted until a migration into it exists.** A release that bumps `version` to 2 must ship, in the same release, a `migrate_1_to_2` step that runs on load, rewrites the file atomically (§8.5), and is covered by a fixture test reading a real v1 file. Bumping the constant without that is a broken release, not a partial one.

### 8.5 Persistence and propagation

```
ConfigStore {
    path: PathBuf,
    current: watch::Sender<Arc<Config>>,      // the single publication point
}
    load()                  -> Result<Config>
    subscribe()             -> watch::Receiver<Arc<Config>>
    save(&Config)           -> Result<()>     // validate → write → publish
```

**Atomic write:** serialize to TOML, write to `config.toml.tmp` *in the same directory*, `fsync`, then `rename` over the target. A crash mid-write can therefore never truncate a working config. Same-directory temp is required because `rename` is only atomic within a filesystem.

**Propagation:** every consumer holds a `watch::Receiver<Arc<Config>>`. On save:
- the scheduler rebuilds its plan from the new per-prayer offsets and re-arms;
- the API cache key is recomputed and, if it changed, `ensure_window` refetches (the design's `Prayer times recalculated.`);
- the UI's next frame reads the new values;
- the audio player picks up the new volume on its next play.

No service polls the config file. No restart is needed for any setting.

**Draft/save semantics, exactly as the design specifies:** the TUI mutates `ConfigDraft` (a clone), the footer offers `[S] SAVE` and `[ESC] CANCEL`, `Esc` discards, `S` calls `ConfigStore::save` and pushes the CONFIRM screen showing the real path and `Prayer times recalculated.`. If `save` fails (read-only filesystem, full disk), the ERROR screen shows `E202` and the draft is *kept* so the user does not lose their edits.

**SIGHUP** re-reads the file from disk and republishes, so `salat daemon` picks up an external edit without a restart.

---

## 9. Location system

Deliberately small: one struct, one embedded dataset, three input modes, zero network calls.

### 9.1 The record

`config.location` (§8.2) *is* the location model. There is no separate location store, no location database, no cache of past locations. One active location, persisted with the rest of the config.

The invariant: **latitude, longitude and IANA timezone are always all present.** Every path that sets a location must supply all three. This is what makes the scheduler and the API client unconditional — neither ever has to ask "do we know the timezone yet".

### 9.2 The embedded city index (resolves A4)

`assets/cities.bin`, generated by `cargo xtask gen-cities` from GeoNames
`cities15000.txt` plus the curated `assets/extra_cities.tsv`, and committed.
The extras file carries hand-verified places the population filter drops
structurally — GeoNames records them with population 0, so no `cities*` dump
can ever include them; each row cites the geonameid it was verified against,
and `gen-cities` retires a row whose id the primary dump gains, so a place is
never embedded twice:

```
header: magic "SLTC" · u16 version · u32 city_count · u16 tz_count
tz table:   tz_count × (u8 len + UTF-8 IANA name)
city table: city_count × { u8 name_len, name, u8 cc_len, cc, u8 country_len,
                           country, i32 lat_e6, i32 lng_e6, u16 tz_index,
                           u32 population }
```

~25,000 cities (population ≥ 15,000), roughly 1.0 MB in `.rodata`, `include_bytes!`-linked and parsed lazily into slices on first search — no allocation of 25,000 owned structs.

Search: case- and diacritic-insensitive (`Setif` must find `Sétif`) substring match on `name` and on `"name, country"`, ranked by (prefix-match first, then population descending), returning the top 3 — which is exactly the design's result count. A linear scan over 25,000 entries is well under a millisecond; no index structure is warranted.

Licensing: GeoNames data is CC-BY 4.0. Attribution appears in README, in `LICENSE-THIRD-PARTY`, and in `salat --help`'s footer.

### 9.3 Three modes, matching the design's three buttons

**`[ SEARCH CITY ]`** — type a query, get up to 3 results as `○ Sétif, Algeria`; `↑↓` selects, `Enter` commits `{label, country, lat, lng, timezone}` from the index. Instant, offline, no failure mode. The design's blinking `█` cursor is a `Blink { 560 ms }` on a `█` after the input text.

**`[ COORDINATES ]`** — two numeric inputs. On `Enter`: validate ranges (`E101` on failure), then derive the timezone by finding the nearest city in the index (great-circle distance) and taking its zone, showing the derivation honestly: `Timezone: Africa/Algiers (nearest: Sétif, 3 km)`. If the nearest city is more than 200 km away, fall back to the system timezone and label it `(system)` — because a mid-ocean coordinate has no defensible zone and guessing silently would be worse than saying so. `label` becomes `36.1900°N · 5.4100°E` unless the user typed one.

**`[ AUTO-DETECT ]`** — resolves A5. In order: read `/etc/localtime`'s symlink target and extract the zone after `zoneinfo/`; else `$TZ`; else `timedatectl show -p Timezone --value`. Then pick the highest-population city in that zone. Presented as a proposal with the sub-label `detected from system timezone · refine with COORDINATES`, confirmed by `Enter`. If no zone can be determined, show `E103` and direct the user to the other two modes.

### 9.4 How the timezone is used

Three distinct jobs, kept distinct:

1. **Sent to the API** as `timezonestring`, so the returned ISO-8601 offsets are computed for the user's zone and are internally consistent with our own conversions.
2. **Interpreting responses:** parse to `DateTime<FixedOffset>`, then `.with_timezone(&tz)` into `DateTime<Tz>`. The FixedOffset carries the correct offset for that specific date, so DST transitions inside a fetched month are already resolved by the API. We never construct a local time from a naive time.
3. **Defining "today":** the civil date, the midnight rollover, and the scheduler's day boundary all use `clock.now_in(location.timezone)` — **not** the host's local time. A user in Berlin configured for Mecca sees Mecca's prayer day. The design's `Africa/Algiers (UTC+1)` footer text is rendered from the configured zone and its offset *at the current instant*, so it updates across DST without special handling.

`meta.timezone` from the API response is compared against the configured zone; a mismatch sets `Timezone` health to `Degraded` with detail `configured Africa/Algiers · API used Europe/Paris`, rather than silently disagreeing.

---

## 10. Prayer state machine

### 10.1 States and their windows

For a notifiable prayer `p` on a given day, with configured `before = cfg.prayers[p].before_minutes` and `after = cfg.prayers[p].after_minutes`, and `t = p`'s instant, `t_next` = the next prayer instant after `t`:

| State | Condition | Design appearance |
|---|---|---|
| `Completed` | user marked it complete (A3) — **highest precedence, overrides all others** | `✓ completed`, green, `greenBg` |
| `Adhan` | `t ≤ now < t + adhan_display` (`adhan_display = 90 s`) | `◉ adhan`, red, `redBg`, blinking `LIVE` |
| `PostAdhan` | `t + adhan_display ≤ now < t + after` (only when `after > 0`) | `◉ reminder`, red, `redDim` border |
| `PreAdhan` | `t − before ≤ now < t` (only when `before > 0`) | `◈ soon`, yellow, `yellowBg` |
| `Next` | `now < t` and `p` is the earliest future prayer | `● next`, green, `rowActive` |
| `Upcoming` | `now < t`, not the next one | `○ upcoming`, `textSub` |
| `Passed` | `now ≥ t + max(after, adhan_display)` | `✓ passed`, `textDim` |

Evaluation order is exactly the table's order, top to bottom, first match wins. Two precedence rules are load-bearing:

- **`PreAdhan` outranks `Next`.** Both are true in the pre-window; the design's yellow "soon" is more informative than green "next", and the design's dashboard shows `Notify before → 5 min · at 19:16` in yellow precisely to signal that transition.
- **`Adhan` and `PostAdhan` outrank `Passed`.** The prayer's instant has passed but the row must stay red until the reminder window closes.

`adhan_display = 90 s` is Salat's constant, not the audio file's length: the visual Adhan state must not depend on whether audio played, how long the recording is, or whether a player exists. Audio playback and Adhan *state* are independent by design.

### 10.2 Transitions

```
Upcoming ─(becomes earliest future)→ Next ─(t−before)→ PreAdhan ─(t)→ Adhan
   ↑                                                                    │
   │                                                            (t+90s) │
   └──────────────── (day rollover, new day's data) ─────────┐          ▼
                                                             │      PostAdhan
Passed ←──(t+after, or t+90s when after==0)──────────────────┴──────────┘
   │
   └──(user presses c)──→ Completed ──(user presses c again)──→ Passed
```

`Completed` is only reachable by user action and is cleared at day rollover (completions are stored per date). A prayer can be marked complete from any state, including `Upcoming` — the design does not forbid it, and a user praying early is a real case.

### 10.3 Purity and testability

```
fn state_of(instant: &PrayerInstant, next: Option<&PrayerInstant>,
            cfg: &PrayerConfig, completions: &CompletionSet,
            now: DateTime<Tz>) -> PrayerState
```

No `self`, no I/O, no ambient clock. Every boundary is a two-line test: set `MockClock` to `t − before − 1s` and assert `Next`; to `t − before` and assert `PreAdhan`; to `t − 1s` assert `PreAdhan`; to `t` assert `Adhan`; to `t + 89s` assert `Adhan`; to `t + 90s` assert `PostAdhan`; to `t + after − 1s` assert `PostAdhan`; to `t + after` assert `Passed`. Plus the `after == 0` and `before == 0` degenerate cases, which must skip the corresponding state entirely rather than producing a zero-width window that flickers.

### 10.4 Why the state machine is not the scheduler

The state machine is a **pure function of time** — it answers "what is true now" and is evaluated on every frame. The scheduler is a **side-effect engine** — it answers "what must fire, once". They share `PrayerWindow` and `Config` but not state, and they can disagree without harm: an observer TUI (§4.3) renders `Adhan` correctly while firing nothing. Conflating them is what would make notification delivery depend on rendering, which the brief explicitly forbids.

---

## 11. Scheduler architecture

This is the component the brief scrutinises hardest, so the reasoning is spelled out.

### 11.1 The requirement, restated precisely

Events must fire once and only once, at the right wall-clock second, regardless of: redraws, resizes, screen navigation, config changes, day boundaries, DST transitions, laptop suspend/resume, manual clock changes, NTP steps, and the app starting *after* an event should already have fired.

### 11.2 Why the obvious design is wrong

The natural implementation is `tokio::time::sleep_until(next_event)`. It is broken here, for a reason worth stating because a coding model will otherwise reach for it: **Tokio's timers are monotonic and do not advance while the machine is suspended.** Sleep for 40 minutes, close the lid for 3 hours, and the timer fires 40 minutes of *awake* time later — around three hours late — with no indication anything went wrong. A `sleep`-based scheduler silently misses every prayer across a suspend.

### 11.3 The design: plan + tick + reconcile

Three parts, each independently testable.

**Planner** (pure, in `domain`):

```
fn plan_events(window: &PrayerWindow, cfg: &Config, now: DateTime<Tz>)
    -> Vec<ScheduledEvent>

struct ScheduledEvent {
    kind:    EventKind,       // PreNotify | Adhan | MosqueReminder | DayRollover
    prayer:  Option<Notifiable>,
    date:    NaiveDate,       // the prayer's civil date — part of the dedup key
    fire_at: DateTime<Tz>,    // absolute wall-clock instant
}
```

For each notifiable prayer in `today` and `tomorrow`: emit `PreNotify` at `t − before` when `before > 0 && notify && notifications.enabled`; `Adhan` at `t` when `adhan && adhan.enabled`; `MosqueReminder` at `t + after` when `after > 0 && mosque_reminder && notifications.enabled`. Plus one `DayRollover` at tomorrow 00:00:00 in the location's zone. Sorted ascending, past events dropped except as §11.5 handles them.

A pure planner means "does a config with `before = 0` produce a pre-notify event" is a unit test with no clock, no I/O and no async.

**Runner** (the Tokio task):

```
loop {
    select! {
        _ = ticker.tick()          => reconcile(),          // every 1 s
        _ = config_rx.changed()    => { replan(); reconcile(); },
        Some(cmd) = cmd_rx.recv()  => handle(cmd),          // Refresh | Snooze | Shutdown
        _ = sigterm.recv()         => break,
    }
}
```

`reconcile()`:
1. `now = clock.now_in(tz)`.
2. **Detect a clock jump:** keep `(last_wall, last_mono)`. If `|Δwall − Δmono| > 2 s`, a suspend/resume or clock step occurred → log it, `replan()`, and route past-due events through catch-up (§11.5).
3. Fire every event with `fire_at ≤ now` that is not already in the fired ledger, oldest first.
4. `DayRollover` triggers `ensure_window` and `replan()`.
5. If the plan's last event is within 6 hours, `replan()` to keep the horizon populated.

**One tick per second is not a cost.** The UI already ticks for its clock and countdown; the daemon's 1 Hz loop is a handful of comparisons and a `DateTime` construction. There is no efficiency argument for the fragile design.

### 11.4 Idempotency

```sql
CREATE TABLE fired_events (
    scope   TEXT NOT NULL,          -- 'default' | '<stem>-<8hex>' scope (§8.1)
    date    TEXT NOT NULL,          -- YYYY-MM-DD, prayer's civil date
    prayer  TEXT NOT NULL,          -- 'fajr' … 'isha' | '' for DayRollover
    kind    TEXT NOT NULL,          -- 'pre_notify' | 'adhan' | 'mosque_reminder'
    fired_at TEXT NOT NULL,         -- RFC3339, actual fire instant
    late    INTEGER NOT NULL,       -- 0 on time, 1 fired via catch-up
    PRIMARY KEY (scope, date, prayer, kind)
);
```

Firing is `INSERT` → on success, perform the side effect; on `UNIQUE` conflict, skip. The insert *precedes* the side effect, so a crash between them loses an event rather than duplicating it — the correct trade for an audible alarm. **This table is why SQLite is genuinely required and not merely convenient:** it is the only way a restart can know what already fired, and it is exactly the "store only what genuinely benefits" the brief asks for.

### 11.5 Late start and catch-up (the brief's explicit question)

When the app starts, or a clock jump is detected, each past-due unfired event is judged by age:

| Event | Grace | Rationale |
|---|---|---|
| `PreNotify` | 2 min | A "5 minutes before" notice delivered 10 minutes late is misinformation. |
| `Adhan` | 5 min | Audio is intrusive; playing the call 40 minutes late is worse than silence. |
| `MosqueReminder` | 5 min | "Head out now" must be roughly now. |

- `now − fire_at ≤ grace` → fire once, recorded with `late = 1`, and the notification body is prefixed `(delayed)` so the user is never misled about timing.
- `now − fire_at > grace` → **do not fire.** Insert into `fired_events` anyway (so it cannot fire later) and write a `Missed` row to the event log, which the Status screen shows as a dim `18:21 │ Maghrib Adhan missed / app was not running`.

This is the concrete answer to "what happens when the app starts after a scheduled event should have fired": recent events fire once and honestly; stale events are recorded as missed and stay silent. Opening a laptop at 3 a.m. never triggers a call to prayer.

**Whether a late fire raises an overlay.** A `late = 1` fire still delivers its notification and, for `Adhan` with `adhan = true`, still plays audio — that is what "fire once" means. But the full-viewport overlay of §16.7 / §16.8 is governed by a narrower rule, because a takeover screen appearing during launch is indistinguishable from a bug:

| Where the late fire came from | Notification + audio | Overlay |
|---|---|---|
| The live tick, while a TUI was already running — e.g. `Ctrl-Z` and resume, or a clock jump caught inside grace | yes, body prefixed `(delayed)` | **yes**, with `(delayed)` in the overlay's top bar |
| `run_catchup` at startup | yes, body prefixed `(delayed)` | **no** |

So: **an overlay is raised only if the TUI was already running when the event fired.** A catch-up fire writes its `fired_events` row, sends the notification, plays audio if configured, and then leaves the UI alone — the dashboard renders that prayer in its post-adhan state and the event log carries `18:21 │ Maghrib Adhan fired (delayed)`, which is where a user looks to find out what they missed. This needs no new state and no new field: it is simply whether the fire originated in `run_catchup` or in the live tick, which the scheduler already knows because they are different call sites. §16.15's five-case table describes the *live* half of this; a catch-up fire adds no sixth case, it just draws nothing.

Over-grace events raise no overlay either, by construction — they do not fire at all.

### 11.6 Change and boundary handling

| Trigger | Behaviour |
|---|---|
| Redraw / resize / navigation | **No effect.** The scheduler has no rendering inputs. |
| Config change | `watch` fires → `replan()` → `reconcile()`. Already-fired events stay fired (the ledger is keyed on identity, not on the plan). Newly-created future events are armed; removed events simply vanish from the plan. |
| Day rollover | The `DayRollover` event fires at 00:00 local, refreshes the window, replans. Completions for the new date start empty. |
| DST transition | Falls out of the design: `fire_at` is a `DateTime<Tz>`, comparison is instant-based, and the API already returned the correct offset for each date. A spring-forward that deletes 02:30 cannot produce a prayer at a nonexistent local time, because we never construct local times ourselves. |
| Suspend / resume | Jump detection (§11.3 step 2) → replan + catch-up. |
| NTP step | Identical to suspend; the same code path handles both. |
| `SIGTERM`/`SIGINT` | Stop audio, flush the event log, release the lock, exit 0. |
| `SIGHUP` | Re-read config from disk, republish, replan. |

### 11.7 Snooze

`Snooze` is a runtime command, never persisted: `[S] SNOOZE 3m` on the Adhan overlay and `[S] SNOOZE 5m` on the mosque reminder insert a one-shot in-memory event at `now + duration` with a synthetic kind (`AdhanSnooze`/`ReminderSnooze`) excluded from the ledger. Snoozes do not survive a restart, which is correct: a snooze is a statement about the next few minutes.

### 11.8 Determinism in tests

The runner takes `Arc<dyn Clock>` and its tick source is injectable (`tokio::time::interval` in production; a manually-driven channel in tests). With `MockClock` a test can advance the clock by 6 hours in one step and assert the exact set of fired events and their `late` flags — no `tokio::time::pause` subtleties, no sleeping, no flakiness.

---

## 12. Notification architecture

### 12.1 Mechanism

`notify-rust` over D-Bus `org.freedesktop.Notifications`. Two notification kinds:

| Kind | Summary | Body | Urgency | Timeout |
|---|---|---|---|---|
| `PreNotify` | `Maghrib in 5 minutes` | `19:21 · Sétif, Algeria` | Normal | 10 s |
| `MosqueReminder` | `Time to head out` | `Maghrib was 15 minutes ago · Iqama soon` | Critical | never (sticky) |

`app_name = "Salat"`, `hint(Category("event"))`, and an `icon` that is a stock freedesktop name (`appointment-soon`) rather than a bundled file, so no asset install is required.

Adhan does **not** send a desktop notification: the design gives it a full-screen terminal overlay plus audio. Sending all three would be noise.

### 12.2 Blocking-call handling

`notify_rust::Notification::show()` is synchronous and does D-Bus round-trips that can block for seconds if the notification daemon is wedged. It is therefore always called inside `tokio::task::spawn_blocking`, with the result awaited under a 5-second timeout. A hung notification daemon must never stall the scheduler tick or the render loop.

### 12.3 Degradation

Capability is probed once at boot (stage 4) by attempting a D-Bus session connection:

| Situation | Health | Detail | Behaviour |
|---|---|---|---|
| Session bus present, daemon responds | `Ok` | `ENABLED` | send normally |
| No `$DBUS_SESSION_BUS_ADDRESS` (SSH, bare TTY) | `Degraded` | `UNAVAILABLE (no D-Bus)` | skip sending; **still log the event**, so the Status screen and event log stay truthful |
| Bus present, no notification daemon | `Degraded` | `NO NOTIFICATION DAEMON` | as above |
| `notifications.enabled = false` | `Ok` | `DISABLED` | intentionally silent |
| Send fails at runtime | `Degraded` | last error text | log, do not retry (a prayer notice has no value late) |

Degradation is never fatal and never blocks: the TUI, scheduler and Adhan audio all work fine on a TTY with no desktop session.

### 12.4 Safety

Notification bodies are assembled exclusively from a fixed enum of prayer names, formatted times and the user's own location label. **No remote string ever reaches a notification body** — API error text is not interpolated into notifications. Some notification daemons render a limited HTML subset, so `&`, `<` and `>` in the user's location label are escaped. This closes the only injection path a prayer-time app plausibly has.

### 12.5 Contract

```
trait Notifier: Send + Sync {
    async fn send(&self, n: NotificationRequest) -> Result<(), NotifyError>;
    fn report(&self) -> Component;
}
struct DesktopNotifier;   // production
struct RecordingNotifier; // tests: records requests in a Mutex<Vec<_>>
```

`RecordingNotifier` is what makes "does a 15-minute pre-notify fire at 19:06 for a 19:21 Maghrib" an assertion rather than an observation.

---

## 13. Adhan / audio architecture

### 13.1 Decision: external player, not an embedded engine

The brief asks for a practical Linux-native implementation rather than a custom audio engine. Two candidates were weighed:

| Option | Verdict |
|---|---|
| `rodio`/`cpal` in-process | **Rejected.** Links `libasound2`, which requires ALSA headers at build time and a matching library at runtime — which forbids a fully static musl binary, the cornerstone of the distribution plan. Also needs manual decoding and its own mixer thread. |
| External player subprocess | **Selected.** Zero build dependencies, zero runtime library dependencies, correct routing through whatever the user actually runs (PipeWire, PulseAudio, bare ALSA), trivial stop semantics (kill the child), and format support inherited from a real media player. |

### 13.2 Player probing

At boot (stage 4), probe `PATH` in priority order and keep the first that exists:

| Player | Command | Volume |
|---|---|---|
| `mpv` | `mpv --no-video --really-quiet --volume=<0-100> <file>` | native 0–100 |
| `paplay` | `paplay --volume=<0-65536> <file>` | `volume * 655` |
| `ffplay` | `ffplay -nodisp -autoexit -loglevel quiet -volume <0-100> <file>` | native 0–100 |
| `aplay` | `aplay -q <file>` | **none** — WAV only, volume ignored, documented limitation |

`mpv` is first because it handles MP3/OGG/WAV, has real volume control, and is the most commonly installed. `aplay` is last because it plays only uncompressed WAV and cannot set volume; when it is the only player available the Status screen reports `Audio system: READY (aplay · no volume control)` — an honest partial capability rather than a silent failure.

Spawned with `tokio::process::Command` and an **argv, never a shell**. The file path is passed as a single argument, so a path containing spaces, quotes or `;` cannot become a command. This is the single most important security property of the audio subsystem (§21.5, `Audio` row).

### 13.3 Contract

```
trait AudioPlayer: Send + Sync {
    async fn play(&self, file: &Path, volume: u8) -> Result<PlayHandle, AudioError>;
    async fn stop(&self) -> Result<(), AudioError>;
    fn is_playing(&self) -> bool;
    fn report(&self) -> Component;
}
```

- **One playback at a time.** `play` stops any current child first. Two overlapping Adhans is never desirable.
- **`stop`** sends `SIGTERM`, waits 500 ms, then `SIGKILL`. Bound because a wedged player must not block `Enter` on the Adhan overlay.
- **Playback is fire-and-forget** with respect to the scheduler: `play` returns once the child is spawned. A `PlayHandle` task awaits exit and reports duration to the event log (the design's `Asr Adhan played / duration 0:32`).
- **Process cleanup:** the child is killed on `Drop`, on `SIGTERM`/`SIGINT`, and on panic (via the `color-eyre` hook), so a crashing Salat never leaves an Adhan playing.

### 13.4 File resolution

```
if !cfg.adhan.file.is_empty()  → that absolute path
else                           → $XDG_DATA_HOME/salat/adhan/{preset}.mp3
```

Validated as: absolute, exists, is a regular file (not a device, FIFO or symlink to one), readable, and ≤ 50 MB. Absent → `AudioError::NoFile`, Status shows `NO ADHAN FILE`, and `salat test-adhan` prints the exact expected path with a one-line instruction. No recording is bundled (§2.3).

### 13.5 `salat test-adhan`

The design's `Test adhan → [ PLAY ]` row and a CLI subcommand share one code path:

1. Resolve the file → if missing, print the expected path and exit 1.
2. Resolve the player → if none, print the four supported players and exit 1.
3. Play at the configured volume, print `Playing <file> via mpv at 80% — press Ctrl-C to stop`.
4. Await exit or `Ctrl-C` (which kills the child), exit 0.

In the TUI, `[ PLAY ]` starts playback and the row's value becomes `[ STOP ]` while active — a two-state control, no new screen.

### 13.6 Independence from the TUI

The player is owned by `Runtime`, not by any screen. It is driven by the scheduler and by explicit user commands. Playback continues across screen changes and resizes, and does not stop when the Adhan overlay is dismissed *unless* the user pressed `Enter` — the design's `[ENTER] DISMISS` on a red screen clearly means "silence this", so dismissal calls `stop()`. `[S] SNOOZE 3m` also stops audio and re-arms.

---

## 14. CLI architecture

### 14.1 Surface (kept minimal, per the brief)

```
salat                      Quick live view (inline viewport)
salat --full               Full interactive TUI
salat --help / -h          Help
salat --version / -V       Version
salat status               Print system status and exit
salat config               Print the resolved scope, config, database and lock
                           paths (§8.1), then the effective config
salat config --edit        Open $EDITOR, validate on save
salat test-adhan           Play the configured Adhan and exit
salat daemon               Run the scheduler headlessly (for systemd)

Global flags:
  --once                   Print one frame and exit (no live loop)
  --config <PATH>          Alternate config file
  --no-color               Disable color (also honours NO_COLOR)
  -v, --verbose            Raise log level (repeatable)
```

### 14.2 Clap structure

```rust
#[derive(Parser)]
#[command(name = "salat", version, about, long_about, after_help = ATTRIBUTION)]
struct Cli {
    #[command(subcommand)] command: Option<Command>,
    #[arg(long)]          full:    bool,
    #[arg(long)]          once:    bool,
    #[arg(long, value_name = "PATH")] config: Option<PathBuf>,
    #[arg(long)]          no_color: bool,
    #[arg(short, long, action = ArgAction::Count)] verbose: u8,
}

#[derive(Subcommand)]
enum Command {
    Status,
    Config { #[arg(long)] edit: bool },
    TestAdhan,
    Daemon { #[arg(long)] once: bool },
}
```

A **root flag plus an optional subcommand**, not `salat quick` / `salat full`. This is required, not stylistic: the brief mandates that bare `salat` and `salat --full` both work, and Clap resolves `None` subcommand to the default action naturally.

### 14.3 Dispatch

```
match (cli.command, cli.full, stdout_is_tty()) {
    (Some(Status), _, _)         => cmd::status(),
    (Some(Config{edit}), _, _)   => cmd::config(edit),
    (Some(TestAdhan), _, _)      => cmd::test_adhan(),
    (Some(Daemon{..}), _, _)     => cmd::daemon(),
    (None, true, _)              => cmd::full_tui(),
    (None, false, true)          => cmd::quick_live(),      // inline viewport
    (None, false, false)         => cmd::quick_once(),      // piped: plain text
}
```

**Non-TTY stdout auto-selects `--once`** with ANSI stripped, so `salat > file`, `salat | grep`, and `watch salat` all behave. This costs one `IsTerminal` check and prevents the most common terminal-app bug report.

### 14.4 Rejected subcommands, with reasons

| Rejected | Reason |
|---|---|
| `salat next` | `salat --once` already prints the next prayer and countdown. Two commands for one answer. |
| `salat times [--date]` | `--once` covers today; arbitrary dates are a v1.1 feature with no design screen. |
| `salat install-service` | The systemd unit ships in the tarball with two documented commands. Wrapping `systemctl --user enable` in a subcommand hides what it does on a security-sensitive path. |
| `salat location set …` | The Location screen is the designed surface; a parallel CLI editor would be a second, undesigned settings UI. |
| `salat cache clear` | `rm ~/.local/share/salat/salat.db` is documented in the README. |

### 14.5 Exit codes

| Code | Meaning |
|---|---|
| `0` | Success — including a degraded or cache-only result, as long as a frame or an answer was produced. |
| `1` | Runtime failure: audio, IO, or the database (`E401`–`E403`, `E601`). |
| `2` | Usage error — Clap's own exit code for a bad argument. |
| `3` | Configuration invalid or unwritable (`E201`, `E202`). |
| `4` | Prayer data unavailable: the network failed and no cached row covers the requested day (`E301`–`E304`). |

Five codes, matching Phase 0's `ExitCode` mapping (§23). Distinct codes matter for `salat daemon` under systemd, where `Restart=on-failure` should retry `4` but not `3`, and for shell users piping `salat --once` into a status bar, where `4` means "show nothing" and `3` means "the user must fix something".

### 14.6 Output discipline, and the `--once` specimen

Human-readable output on stdout; logs and diagnostics on stderr and to `$XDG_STATE_HOME/salat/salat.log`. **`tracing` never writes to stdout while a viewport is active** — its writer is the log file, with stderr added only when `--verbose` is set and no TUI is running. Colors are suppressed when `--no-color`, `NO_COLOR` is set, or stdout is not a TTY.

**One rule governs `--once`: stdout carries the frame or nothing.** Never a partial frame, never a diagnostic, never a warning interleaved with rows. Every message goes to stderr. That is what makes `salat --once` safe to embed in a status bar, a `.zshrc`, or a polybar module: the caller can trust that stdout is either a complete frame or empty.

`--once` renders through the same code path as quick mode (§16.14) — the same `UiModel`, the same widgets, the same 14-row layout — into an offscreen buffer, which is then printed as lines with trailing whitespace trimmed. It does not open a viewport, does not enter raw mode, does not subscribe to config changes, and never acquires the scheduler lock, so it can never fire a notification or an Adhan. When stdout is not a TTY the terminal size is unavailable, so it falls back to **80×24**, which fixes the render width of piped output at 80 columns.

Trimming and that width are not the same claim, and the specimen below shows why. The buffer is 80 columns wide, so every row *renders* at 80; trimming then removes the padding on rows that do not draw to the right edge. Eight of the fourteen rows are structurally full-width — the header, the panel's three borders, the `TODAY` rule, the gauge's row and the status strip all place a glyph in column 80 — and those print at exactly 80. The other six (the `TODAY` heading and the five prayer rows) print at their content width: 5, 24, 24, 24, 36, 26. **The invariant is therefore a maximum, not a uniform width: no line exceeds 80 columns, and no line carries trailing whitespace.** Both halves matter to the golden files — an untrimmed frame would make the goldens sensitive to padding no reader can see, and a frame wider than 80 would wrap in the caller's pipe.

**Specimen.** With `MockClock` at `2026-09-05T17:38:23+01:00`, the §8.3 default configuration, and the fixture window (Fajr 05:09 · Dhuhr 12:47 · Asr 16:23 · Maghrib 19:21 · Isha 20:47), `salat --once | cat` prints exactly this:

```
SALAT                                                                   17:38:23
╭─ NEXT PRAYER ────────────────────────────────────────────────────────────────╮
│ ● MAGHRIB · 19:21                                                            │
│ 01:42:37  REMAINING                                                          │
│ ██████████████░░░░░░░░░░░░░░░░░░░░                                           │
╰──────────────────────────────────────────────────────────────────────────────╯
TODAY
────────────────────────────────────────────────────────────────────────────────
Fajr     05:09  ✓ passed
Dhuhr    12:47  ✓ passed
Asr      16:23  ✓ passed
Maghrib  19:21  ● next · in 01:42:37
Isha     20:47  ○ upcoming
● svc  ● notif  ● adhan  ● audio                        Sétif · 12 Safar 1448 AH
```

Every dimension in it is reproducible: the header is `SALAT` and the clock padded to 80; the panel's top border is `╭─ NEXT PRAYER ` (15 columns) plus 64 `─` plus `╮`; interior rows are `│` + 78 + `│`; the bottom border is `╰` + 78 `─` + `╯`; the table is `Length(9) Length(7) Min(0)` = `9 + 7 + 64`; the strip is a 32-column left group and a 24-column right group with 24 columns between them. The gauge is 34 cells wide (§16.14 item 10) and its fill follows from the interval: Asr 16:23 → Maghrib 19:21 is 10,680 s, 17:38:23 is 4,523 s into it, `4523 / 10680 = 42.35 %`, and `floor(34 × 0.4235) = 14`, giving 14 `█` and 20 `░`. The countdown is `69660 − 63503 = 6157 s = 01:42:37`.

The specimen's *shape* is normative — column positions, glyphs, ordering, row count. The clock time and the five prayer times are fixture values; if the Figma frame shows different minutes, the fixture and the golden file are regenerated together and the shape is unchanged.

**ANSI escape sequences.** Permitted only when all four hold: stdout is a TTY, `--no-color` was not passed, `NO_COLOR` is unset, and the resolved `color_mode` (§15.6) is not `none`. Even then, only SGR colour and attribute sequences are emitted — never cursor movement, never the alternate screen, never raw mode, never a clear. When any of the four fails, the output contains **zero** escape bytes. This is the same rule §14.3 already applies when it auto-selects `--once` for a non-TTY stdout.

**Behaviour and exit code in the four cases the brief names:**

| Case | stdout | stderr | Exit |
|---|---|---|---|
| Success | the 14-line frame | — | `0` |
| Configuration missing | the frame, after `Config::default()` is written (§8.3) | `salat: created default configuration at ~/.config/salat/config.toml` | `0` |
| Configuration missing *and* unwritable | nothing | `salat: error[E202]: cannot write configuration to …` | `3` |
| API failure, cache warm | the frame, with `· cached` on the TODAY head row and a yellow `● svc` | `salat: warning[E302]: api.aladhan.com unreachable, using cached times` | `0` |
| API failure, no cached row for today | nothing | `salat: error[E301]: prayer times unavailable for 2026-09-05` | `4` |

An invalid — as opposed to missing — configuration is `E201` and exits `3` with no stdout. This is consistent with §15.7's rule that only boot steps 1–3 can be fatal, and it needs no new error code: every case above is already in §18.2's taxonomy.

**The single-line summary.** One form serves the two places a frame will not fit: quick mode below its floor (§16.14 item 9), and `--once` on a TTY narrower than 80 columns. The floor rule is shared deliberately, so both surfaces degrade identically:

```
MAGHRIB · 19:21 · in 01:42:37
```

Four fields, fixed order, ` · ` between them: the next prayer's name uppercased as the NEXT PRAYER panel writes it, its `HH:MM`, the literal `in`, and the `HH:MM:SS` countdown. `· cached` is appended when the day was served from cache, mirroring the frame's marker. Nothing else is ever added — no location, no Hijri date, no status dots.

The width is bounded and needs no truncation logic: `MAGHRIB` is the longest of the five names at 7 columns, so the line is at most **29 columns**, or **38** with `· cached`. It therefore fits every terminal that can hold anything at all, which is why this is the floor case rather than a further ladder. The countdown spanning midnight to tomorrow's Fajr changes nothing about the shape.

The exit codes and the stdout-or-nothing rule are unchanged: this line *is* the frame for these two cases, so the table above applies to it verbatim — a summary line on success, empty stdout and `4` when no cached row covers the day.

**Golden-file requirement (Phase 5).** `--once` output is verified by checked-in golden files, not by `insta` snapshots, because `insta` accepts its first snapshot automatically and would therefore ratify whatever the first implementation happened to print (§20.3). `tests/golden/quick_once_{80x24,100x30,120x35}.txt` are compared **byte for byte** against the rendered output under the fixture above, plus `tests/golden/quick_once_80x24_color.txt` asserting the exact SGR sequences for the truecolor path and `tests/golden/quick_once_oneline.txt` holding the 29-column summary above. The goldens are written by hand from this specimen for the 80×24 case; CI fails on any diff; regeneration is an explicit, reviewable `UPDATE_GOLDEN=1 cargo test golden` step and never happens implicitly. Two width assertions run against every golden independent of its bytes, because they are the invariants of the trim rule rather than of one fixture: no line exceeds **that golden's own render width** — 80, 100 or 120 — and no line ends in whitespace. At 80×24 that is `assert!(lines.iter().all(|l| l.width() <= 80))` and `assert!(lines.iter().all(|l| l.trim_end() == *l))`, with `width()` from `unicode-width` so the `·` and `✓` glyphs are measured as they render.

---

## 15. Ratatui / TUI architecture

### 15.1 Separation of concerns (how the monolith is avoided)

Four distinct things, deliberately not merged:

| Concern | Owner | Rule |
|---|---|---|
| **Services and I/O** | `app::Runtime` | Holds `Arc<dyn …>` handles. Knows nothing about screens. |
| **What is true right now** | `UiModel` | An immutable snapshot rebuilt once per tick from domain functions. Contains no `Arc`, no channels, no services. |
| **Where the user is and what is focused** | `ui::Shell` + per-screen state structs | Navigation stack, overlay slot, and one small struct per screen. |
| **Pixels** | `ui::screens::*` + `ui::widgets::*` | Pure functions `(&UiModel, &ScreenState, Rect, &mut Buffer)`. No I/O, no mutation of anything but the buffer. |

```rust
struct UiModel {
    now:        DateTime<Tz>,
    window:     Arc<PrayerWindow>,
    states:     [PrayerState; 5],
    next:       Option<PrayerInstant>,
    previous:   Option<PrayerInstant>,
    countdown:  Countdown,
    progress:   f64,
    summary:    DaySummary,
    status:     Arc<SystemStatus>,
    config:     Arc<Config>,
    draft:      Option<ConfigDraft>,
    qibla:      Option<f64>,
    breakpoint: Breakpoint,
}
```

Rebuilding `UiModel` from scratch each tick is cheap (a handful of comparisons over 10 instants) and eliminates an entire class of stale-derived-state bugs. Because rendering is a pure function of `UiModel`, every screen is snapshot-testable by constructing a `UiModel` literal — no runtime, no network, no clock.

```rust
struct Shell {
    stack:   Vec<Screen>,             // [Dashboard] … [Dashboard, Settings, Location]
    overlay: Option<Overlay>,         // AdhanAlert | MosqueReminder
    dashboard: DashboardState,        // { cursor: usize }
    settings:  SettingsState,         // { row: usize }
    prayer_cfg: PrayerConfigState,    // { tab: usize (default 3 = Maghrib), row: usize }
    notif:     NotifState,            // { row: usize, col: NotifCol }
    location:  LocationState,         // { mode, query, results, sel, lat_buf, lng_buf, focus }
    status:    StatusState,           // { scroll: u16 }
    boot:      BootState,             // { stage, error }
}
```

Nine small structs instead of one god object. Each screen touches only its own.

### 15.2 Event loop

```rust
let mut events   = crossterm::event::EventStream::new();
let mut tick     = tokio::time::interval(Duration::from_millis(250));
let mut cfg_rx   = config.subscribe();

loop {
    let mut dirty = false;
    select! {
        Some(Ok(ev)) = events.next() => dirty = shell.handle(ev, &mut runtime).await?,
        _ = tick.tick()              => { model = build_model(&runtime); dirty = true; }
        Ok(()) = cfg_rx.changed()    => { model.config = cfg_rx.borrow().clone(); dirty = true; }
        Some(ev) = svc_rx.recv()     => dirty = shell.on_service_event(ev),
        _ = sigterm.recv()           => break,
    }
    if dirty && last_draw.elapsed() >= Duration::from_millis(50) {
        terminal.draw(|f| ui::render(f, &model, &shell))?;
        last_draw = Instant::now();
    }
}
```

Five decisions embedded here:

1. **250 ms tick**, not 1000 ms — the design's fastest blink is 300 ms (loading marker) and 500 ms (Adhan badge), which a 1 Hz tick cannot express. The clock and countdown only *change* on second boundaries, so the extra ticks redraw identical content, which the diffing backend collapses to zero writes.
2. **Redraw on demand, capped at 20 Hz.** No unconditional draw-per-loop.
3. **`Event::Resize` needs no handler.** Layout is derived from `frame.area()` every frame; the resize event's only job is marking dirty.
4. **`EventStream`, not a blocking reader thread.** This is why `crossterm/event-stream` is a required feature — a blocking `event::read()` in a spawned thread with a channel would work but reintroduces a second source of terminal truth.
5. **Async is contained.** Only `handle` is async, and only because commands (save config, play Adhan) are. Rendering is entirely synchronous.

### 15.3 Terminal setup, and the quick/full distinction

| | Quick (`salat`) | Full (`salat --full`) |
|---|---|---|
| Viewport | `Viewport::Inline(h)` where `h = min(14, term_height − 1)` | `Viewport::Fullscreen` |
| Alternate screen | no | yes |
| Mouse capture | no | no (keyboard-only by design) |
| Raw mode | yes | yes |
| On exit | leaves the last frame in scrollback | restores the previous screen |
| Chrome | 1 header line, no footer, no tab bar | full header + footer |
| Exit keys | `q`, `Esc`, `Ctrl-C` | `q` (with `Ctrl-C` always) |

`Viewport::Inline` is the correct primitive for the quick mode: it renders in place, keeps scrollback, and — unlike a print-and-clear loop — needs no cursor arithmetic. Terminal state restoration is a `TerminalGuard` whose `Drop` runs on normal exit, on `?` propagation, and inside the `color-eyre` panic hook, so a panic never leaves the terminal in raw mode with a hidden cursor.

The `min(14, term_height − 1)` in that table is the only dimension quick mode gets from this section. Everything the height implies — the 13-row content budget, the compressed panels that fit it, what happens below 15 rows, and how the two overlays render inside 14 rows — is specified in **§16.14** and **§16.15**. Note in particular that quick mode pins `Breakpoint::Small` and does not run §4.5's classifier against the inline area, since a 14-row frame would classify as `too_small`.

### 15.4 Keyboard handling

Resolution order, first match wins:

1. **Global always:** `Ctrl-C` quit, `Ctrl-L` force full redraw.
2. **Overlay, if present:** `Enter` dismiss (+ stop audio), `s` snooze, `Esc` dismiss. Overlays swallow everything else, including `1`–`4` — a call to prayer must not be dismissed by an accidental tab switch.
3. **Screen-specific**, from the focused screen's handler.
4. **Global fallback:** `q` quit, `1`–`4` main screens, `?` states reference, `Esc` pop the stack (A6).

`KeyEventKind` must be filtered to `Press` only — on Windows-adjacent terminals and some multiplexers, crossterm delivers `Release` too, which double-triggers every key. Cheap guard, common bug.

Per the polish brief, the vocabulary is uniform across every screen: `↑↓` move, `←→` adjust value, `Tab` next column (notification settings only), `Enter` select/confirm, `Esc` back/cancel, `s` save, `q` quit. Only the design's own screen-specific extras deviate (`c` complete, `r` retry, `?` states).

### 15.5 Widget inventory

`ui::widgets/` — each a `Widget` or a render function, each independently snapshot-tested:

| Widget | Purpose | Ratatui basis |
|---|---|---|
| `panel` | `╭─ TITLE ────╮` box, `active` → `borderHi` | `Block` + `BorderType::Rounded` + `Padding` |
| `bar` | `████░░░░` gauge with a width parameter | `Paragraph` of `█`/`░` spans (not `Gauge`, which centers a percentage label the design doesn't have) |
| `badge` | `● next` glyph + word + color | `Line` of two `Span`s |
| `prayer_table` | the TODAY table | `Table` with per-breakpoint `Constraint` sets |
| `bigtext` | 3-row block digits for the countdown (A1) | `Paragraph` of 3 `Line`s |
| `stepper` | `◂ 15 min ▸` with a focused cell | `Line` of three `Span`s |
| `kbd_hint` | `[KEY] label`, green key | `Line` of two `Span`s |
| `crumb` | `SETTINGS › LOCATION` + right hint | `Line` with alignment |
| `setting_row` | label / sub / value with a `› ` selection rail | two `Line`s |
| `timeline` | `◆ ─5m─ ◉ ADHAN ─15m─ ◉ MOSQUE` | `Line` of spans |
| `hr` | full-width `─` rule | `Line` repeated to width |
| `status_dot` | `●` / `✕` health mark | `Span` |
| `header` / `footer` | app chrome | `Paragraph` |
| `too_small` | the sub-80×24 guard | `Paragraph` in a centered `Rect` |

Fourteen widgets, each under ~60 lines. This is the "small number of well-defined modules" the brief asks for — the alternative (one big render function per screen) would duplicate the panel and badge logic twelve times.

**`bigtext` geometry, pinned.** A1 puts a 3-row block countdown at 100×30 and above and a 3-row prayer name in the Adhan overlay, and §16.15 and §25 both quote widths derived from it, so the geometry is fixed here rather than left to the implementer.

| Property | Value |
|---|---|
| Glyph width | **3 columns**, every glyph, no proportional widths |
| Glyph height | **3 terminal rows** = 6 half-rows of vertical resolution |
| Inter-character gap | **1 column**, blank |
| Line spacing | **0** — the three rows are contiguous `Line`s in one `Paragraph` |
| Advance per character | **4 columns** (3 + gap) |
| Cell alphabet | `█` (full), `▀` (upper half), `▄` (lower half), and space — nothing else |
| Character set | `0`–`9`, `:`, `A`–`Z`, and space = **38 glyphs** |
| Storage | `const GLYPHS: [[&str; 3]; 38]` — a literal table, no runtime layout, no font parsing |

**Width calculation.** For a string of `n` characters, `width(s) = 3n + (n − 1) = 4n − 1`. The colon is a full-width 3-column glyph like every other; there is no narrow case and no kerning. Both widths quoted elsewhere in this document follow from that one formula and are now reproducible:

- `01:42:37` → `n = 8` → **31 columns.** At 100×30 the NEXT PRAYER panel is `Percentage(45)` of 100 = 45 columns, less 2 border and 2 padding = 41 inner. 31 ≤ 41 ✓. At 120×35 it is 54 columns, 50 inner. 31 ≤ 50 ✓.
- `MAGHRIB` → `n = 7` → **27 columns**, which is why §16.15 can keep the 3-row prayer name in an 80-column inline Adhan overlay.

**Rendering rules.** Each glyph is a 3×3 grid of cells drawn only from the four characters above; a half-block in a cell resolves the 6 half-rows into 3 terminal rows. Glyphs are rendered left to right with a single blank column between them, one `Span` per glyph per row, so a row of the block is one `Line` of `2n − 1` spans. The whole block takes one style — `textBright` for the countdown, the overlay's own foreground for the Adhan name. Five glyphs, to fix the convention unambiguously:

```
 ███     █     ▀▀█     ▄     █▄█
 █ █     █     ▄▄█           █▀█
 ███    ▄█▄    █▄▄     ▀     █ █
  0      1      2      :      M
```

An unmapped character renders the space glyph and emits one `warn!`; `bigtext` never panics and never returns an error, because a malformed countdown must not take down the frame.

**When the terminal is too narrow.** If `4n − 1 > available_width`, `bigtext` does not render at all: the caller falls back to the single-row `BOLD` + `textBright` form that A1 already specifies for 80×24. It never truncates the block, never wraps it to a fourth row, and never drops the gap to squeeze one more glyph in — a half-drawn glyph is unreadable, whereas the single-row form is merely smaller. The check is on the available inner width of the target `Rect`, evaluated per frame, so a terminal dragged narrower degrades on the next tick.

### 15.6 Theme and color degradation

`ui::theme::Theme` holds every token from §3.2 as `Color::Rgb`. `color_mode`:

- `truecolor` — the RGB values verbatim.
- `256` — each token pre-mapped to an xterm-256 index **by hand**, once, as a constant table. Automatic nearest-neighbour quantisation collapses `border`/`borderDim` and `text`/`textSub` into identical indices, destroying the hierarchy; hand-picking keeps them distinct. The table is given below.

**The 256-color table.** §3.2's palette is 25 rows carrying **27 named values** — 24 single tokens plus the `dot1/2/3` row, which holds three. Five of those are prototype chrome that no screen in §16 renders (`winBg`, `winTitle`, `dot1`, `dot2`, `dot3`), leaving **22 tokens** that need an index.

Two mapping rules produce them, and both are stated so the table can be checked rather than trusted:

1. **Neutrals go to the 232–255 grayscale ramp** (`232 + k` → `#(08 + 10k)` in all three channels). The design's grounds and text greys are very dark desaturated blues; the 6×6×6 cube's darkest non-zero level is 95, so every cube candidate is far too bright and too saturated. Assignment is mechanical: sort the ten neutrals by WCAG relative luminance, walk them dark to light, take the nearest ramp index by luminance, and if that index is not strictly greater than the previous assignment, take `previous + 1` instead. This preserves the design's luminance ordering exactly and guarantees distinct indices.
2. **Hued tokens go to the nearest 6×6×6 cube entry of the same hue family** (`idx = 16 + 36r + 6g + b` over levels {0, 95, 135, 175, 215, 255}), preferring hue fidelity over luminance fidelity. Where two design tokens quantise to the same cell, the one whose role is subordinate moves one cube step away along its dominant channel.

| Token | Design | Index | Index hex | Note |
|---|---|---|---|---|
| `termBg` | `#080d18` | **232** | `#080808` | ramp, nearest |
| `panelBg` | `#0b1222` | **233** | `#121212` | ramp, nearest |
| `borderDim` | `#0d1a2a` | **234** | `#1c1c1c` | ramp, nearest |
| `rowActive` | `#0f1f36` | **235** | `#262626` | nearest was 234 → `prev + 1` |
| `rowSel` | `#122440` | **236** | `#303030` | nearest was 235 → `prev + 1` |
| `border` | `#1a3355` | **237** | `#3a3a3a` | nearest was 236 → `prev + 1` |
| `textDim` | `#2d3f52` | **238** | `#444444` | nearest was 237 → `prev + 1` |
| `textSub` | `#6b7d94` | **244** | `#808080` | ramp, nearest |
| `text` | `#c8d4ea` | **252** | `#d0d0d0` | ramp, nearest |
| `textBright` | `#eef2f8` | **255** | `#eeeeee` | ramp, nearest |
| `borderHi` | `#3d7ab5` | **67** | `#5f87af` | cube (1,2,3) |
| `blue` | `#60a5fa` | **75** | `#5fafff` | cube (1,3,5) |
| `green` | `#4ade80` | **78** | `#5fd787` | cube (1,4,2) |
| `cyan` | `#34d399` | **43** | `#00d7af` | cube (0,4,3) — moved off green's cell |
| `red` | `#f87171` | **203** | `#ff5f5f` | cube (5,1,1) |
| `yellow` | `#fbbf24` | **214** | `#ffaf00` | cube (5,3,0) |
| `greenDim` | `#1a3d24` | **22** | `#005f00` | cube (0,1,0) |
| `borderGrn` | `#1e4a30` | **28** | `#008700` | cube (0,2,0) |
| `redBg` | `#190808` | **52** | `#5f0000` | cube (1,0,0) — see below |
| `redDim` | `#3d1515` | **88** | `#870000` | cube (2,0,0) |
| `greenBg` | `#091a0f` | **233** | `#121212` | collapses to `panelBg` — deliberate |
| `yellowBg` | `#1a1200` | **233** | `#121212` | collapses to `panelBg` — deliberate |

**Two consequences worth stating outright.** First, `green` #4ade80 and `cyan` #34d399 quantise to the same cube cell, which is exactly the collapse this mode exists to prevent, so `cyan` is hand-moved to 43 — a teal that stays clearly distinct from 78 at a glance. Second, the near-black tints have no cube representation at all, so **`greenBg` and `yellowBg` collapse to `panelBg`** and the tinted row grounds simply disappear at 256 colors; the meaning is carried by foreground color plus the state glyph, the same non-color-only encoding §22 requires. `redBg` is the one tint that keeps its hue, at 52, because §16.7 washes the entire Adhan overlay with it and "the screen turns red" is the loudest signal in the application — losing it would make the call to prayer look like any other screen. `text` on 52 is 9.2:1, comfortably past §22's floor.

**The distinctness test.** It is a check on an explicit list of pairs, **not** global injectivity — global injectivity would fail on the two deliberate collapses above and is not the property that matters. The test asserts these eight pairs differ, at every color mode that has enough colors to express them:

`border` ≠ `borderDim` · `border` ≠ `borderHi` · `text` ≠ `textSub` · `textSub` ≠ `textDim` · `green` ≠ `cyan` · `redBg` ≠ `redDim` · `greenDim` ≠ `borderGrn` · `termBg` ≠ `panelBg`

A second assertion covers the ordering the first cannot: the ten **neutral** rows — `termBg`, `panelBg`, `borderDim`, `rowActive`, `rowSel`, `border`, `textDim`, `textSub`, `text`, `textBright` — must have strictly increasing indices in the luminance order of the table above. Note that twelve rows fall in the 232–255 range, not ten: `greenBg` and `yellowBg` land there as the declared collapses and are excluded from this assertion, which is why it names the ten rather than filtering on the index range. Together they are what Phase 5 item 8 checks, in both degraded modes.
- `ansi` — the 16-color set: `green`, `red`, `yellow`, `blue`, `cyan`, `white`, `gray`, `dark gray`, and default background. Panels lose their tinted grounds; the glyph-based state encoding (`✓ ● ◈ ◉ ○`) carries the meaning, which is precisely why the polish brief demanded non-color-only encoding.
- `auto` — `truecolor` if `$COLORTERM` ∈ {`truecolor`, `24bit`}; `256` if `$TERM` contains `256color`; else `ansi`.

### 15.7 Boot sequence (the design's LOADING screen is a real state machine)

The five steps are not decoration; they are the startup contract, each with a defined failure:

| # | Step | Work | On failure |
|---|---|---|---|
| 1 | Loading configuration | read + validate TOML; create default if absent | `E201` invalid config → error screen with `[R] RETRY` / `[D] USE DEFAULTS` |
| 2 | Resolving location | validate coordinates, parse timezone | `E101`/`E103` → error screen offering `[C] COORDINATES` (the design's exact button) |
| 3 | Calculating prayer times | open DB, run migrations, `ensure_window` (cache, then network) | cache hit → continue with `Degraded` network; cache miss + offline → `E301` with `[R] RETRY` |
| 4 | Initializing audio | probe players, resolve the Adhan file, probe D-Bus | never fatal — sets `Degraded` health only |
| 5 | Starting prayer service | acquire the lock, plan, spawn the runner, run catch-up | lock held → observer mode (§4.3), still success |

Steps render live with `✓` / blinking `▶` / `○` and the design's progress bar and `{pct}% · Sétif, Algeria` label. **Only steps 1–3 can be fatal.** A machine with no audio player, no D-Bus and a stale cache still reaches the dashboard — the correct priority for an app whose job is telling you the time.

In quick mode the loading screen is skipped (it would flash for 200 ms); failures print a single stderr line and exit non-zero.

### 15.8 Status decoupling

`status::Collector` calls `report()` on each service and assembles `SystemStatus` (§6.7). The Status screen and `salat status` render the same struct through different formatters. Neither knows that "Network" health comes from the last HTTP attempt's outcome or that "Audio system" comes from a `PATH` probe. Adding a component is: add a `ComponentId` variant, implement `report()`, done — both surfaces update.

---

## 16. Screen and component mapping from Figma

Each screen below gives the ten required items: **purpose · state · entry · layout · components · data · keys · loading/error/empty · 80×24 · 100×30 and 120×35.**

§16.1–16.12 are the twelve screens of §3.3. The three subsections after them are not screens in that enum: §16.13 is the sub-80×24 guard and §16.15 is a delta to §16.7 and §16.8, both stated in prose; §16.14 follows the same ten items, with items 9 and 10 adapted because quick mode's layout is driven by terminal *height* rather than by a breakpoint.

### 16.1 Dashboard (`dashboard` — footer tab `[1] TODAY`)

1. **Purpose.** Answer "what's next and how long" at a glance; show today's five times and service health.
2. **State.** `DashboardState { cursor: usize }` — the TODAY row cursor for completion toggling (A3). No other local state; everything else comes from `UiModel`.
3. **Entry.** Default screen at boot; `1` from anywhere; `Esc` from any main screen; bottom of the nav stack.
4. **Layout.** Small: `Vertical[ Length(6) NEXT PRAYER, Min(11) TODAY, Length(1) strip ]`. Medium/large: `Vertical[ Length(1) greeting (large only), Min(0) body, Length(1) strip ]` where body = `Horizontal[ Percentage(45) NEXT PRAYER, Min(0) TODAY ]`; at large the right column is `Vertical[ Min(0) TODAY, Length(4) TONIGHT ]`.
5. **Components.** `panel` ×2–3, `badge`, `bigtext` (medium/large), `bar`, `prayer_table`, `hr`, `status_dot` ×4, `header`, `footer`.
6. **Data.** `next`, `previous`, `countdown`, `progress`, `states[5]`, `window.today.times`, `window.today.hijri`, `qibla`, `window.tomorrow` (fajr + midnight, large only), `config.prayers[next]` (before/after minutes), `status.components` (service, notifications, adhan, audio), `config.location.label`.
7. **Keys.** `↑↓` move the TODAY cursor · `c` toggle completed · `1`–`4` tabs · `?` states · `q` quit. No `←→`, no `Enter` — the dashboard is read-only apart from completion.
8. **Loading/error/empty.** Loading: the boot screen precedes it, so the dashboard never renders empty; a mid-session refresh shows `⋯` in place of the countdown for at most one tick. Network degraded: the strip's `● svc` turns yellow and the TODAY footer appends `· cached`. No data for today at all is unreachable (boot step 3 gates it).
9. **80×24.** Header loses version, location and date; footer shows `[1] [2] [3] [4]` + `[Q]`; NEXT PRAYER stacks above TODAY; countdown is single-row bold (A1); gauge width 34; no greeting, no TONIGHT; the `NOTIF` column collapses to a 3-column table (`90px 52px 1fr` in the design → `Length(9) Length(7) Min(0)`).
10. **100×30 / 120×35.** 100×30: two columns 45/55, 3-row `bigtext` countdown, `SCHEDULED / 19:21` in the panel's top-right, `Notify before → 5 min · at 19:16` (yellow) and `Mosque reminder → +15 min after` below the rule, gauge width 32, 4-column table with `NOTIF` showing `-5m`, status strip with four labelled dots. 120×35 adds: the greeting line `ASSALAMU ALAIKUM · Saturday · 12 Safar 1448 AH`, the TONIGHT panel (`Midnight (est.) 00:04`, `Fajr tomorrow 05:11`), gauge width 40, and the strip's right side `Union of Islamic Orgs · Africa/Algiers (UTC+1)`.

### 16.2 Settings (`settings` — `[2] SETTINGS`)

1. **Purpose.** Single entry point to every configurable value, grouped as the design groups them.
2. **State.** `SettingsState { row: usize }` — a flat index across all rows in all four panels, so `↑↓` crosses panel boundaries without a nested focus model. `UiModel.draft` holds pending edits.
3. **Entry.** `2` from any main screen; `Esc` from Location or Notification Settings (A6).
4. **Layout.** Small: `Vertical[ Length(6) GENERAL, Length(5) NOTIFICATIONS, Length(6) ADHAN, Min(0) DISPLAY ]` with sub-labels suppressed. Medium: same, sub-labels shown. Large: `Horizontal[ Percentage(50) [GENERAL, ADHAN], Percentage(50) [NOTIFICATIONS, DISPLAY] ]`.
5. **Components.** `panel` ×4, `setting_row` ×15, `footer`.
6. **Data.** `draft` (or `config`) for every value; `status` for the `ENABLED`/`DISABLED` rendering of notifications and Adhan.
7. **Keys.** `↑↓` move · `←→` cycle the value in place (time format, volume ±5, presets, booleans) · `Enter` open the sub-screen for actionable rows (Location → `location`; Enable notifications and Per-prayer timing → `notif-settings`; Test adhan → start/stop playback) · `s` save → `confirm` · `Esc` cancel, discarding the draft.
8. **Loading/error/empty.** Never empty. Saving shows `SAVING…` in the footer for the duration of the write. A failed save pushes `error` with `E202` and preserves the draft. Rows whose subsystem is unavailable render their value in `textDim` with the reason as the sub-label (`Volume — no player found`), remaining visible rather than disappearing.
9. **80×24.** Four panels stacked, 20 body rows for 15 rows + 8 border rows = 23 → **the panels must scroll.** Implement a viewport that scrolls to keep `row` visible, with `▲`/`▼` markers in the border when content is clipped. Sub-labels hidden (the design hides them at small), which is what makes 15 rows fit in a scroll region rather than 30.
10. **100×30 / 120×35.** 100×30: all four panels stacked with sub-labels, 26 body rows, no scrolling needed. 120×35: two columns of two panels, all rows visible, sub-labels shown, `↑↓` moves down the left column then continues into the right (a flat index over a two-column visual layout — matching how the design orders the panels).

### 16.3 Prayer Configuration (`prayer-config` — `[3] PRAYERS`)

1. **Purpose.** Configure one prayer at a time with more room than the dense per-prayer grid, including its sound preset and a timeline preview.
2. **State.** `PrayerConfigState { tab: usize, row: usize }` — `tab` defaults to **3 (Maghrib)**, exactly as the design's `sel = 3`.
3. **Entry.** `3` from any main screen.
4. **Layout.** `Vertical[ Length(3) tab bar, Min(0) config panel, Length(6) TIMELINE ]`. The tab bar is `Tabs` with 5 titles; the active tab gets `rowActive` background and a green underline (rendered as `▔▔▔` beneath the label, since a terminal has no border-bottom).
5. **Components.** `Tabs`, `panel` (title `MAGHRIB · 19:21`, green title, `borderGrn` border), `stepper` ×2, `setting_row` ×3, `hr`, `timeline`, `footer`.
6. **Data.** `draft.prayers[tab]`, `window.today.times[tab]` for the panel title, `config.adhan.preset`.
7. **Keys.** `←→` switch tab · `↑↓` move row · `Enter`/`Space` toggle a boolean row · on a stepper row, `←→` adjust by **±5** (A: the design's `◂ 5` / `5 ▸` buttons), clamped `0..=60` · `s` save · `Esc` back.

   *Conflict resolved:* `←→` is both "switch tab" and "adjust value". Resolution: `←→` switches tabs only when `row == 0` (a dedicated first row rendered as the tab bar's focus); otherwise it adjusts the focused row. `Tab`/`Shift-Tab` always switch tabs regardless of row, and the crumb hint reads `←→ tab · ENTER · ESC back` at row 0 and `←→ adjust · TAB tab` elsewhere. This is the smallest change that keeps both design behaviours reachable.
8. **Loading/error/empty.** Never empty (five prayers always exist). If today's times are unavailable the panel title shows `MAGHRIB · --:--` and the timeline renders with `--` markers; the numeric config remains fully editable, because it does not depend on prayer times.
9. **80×24.** Tab bar shows short names `FAJ DHU ASR MAG ISH` (the design's `short` field earns its place here); the timeline panel drops to `Length(4)` with a single-line timeline; the sound-preset row is hidden and reachable only from Settings → Sound preset (documented in the crumb hint as `presets in SETTINGS`).
10. **100×30 / 120×35.** 100×30: full prayer names in tabs, all five rows, 6-row timeline. 120×35: additionally shows the resolved absolute times inline on the stepper rows (`Before Adhan notification   ◂  5 min ▸   → 19:16`) and a second timeline line with the clock labels beneath the markers.

### 16.4 Status (`status` — `[4] STATUS`)

1. **Purpose.** Prove the system is working: 10 component healths, today's counters, the next event, and today's event log.
2. **State.** `StatusState { scroll: u16 }` for the event log.
3. **Entry.** `4` from any main screen.
4. **Layout.** Small: `Vertical[ Min(0) SERVICES, Length(?) RECENT EVENTS ]` with the right column omitted. Medium/large: `Vertical[ Length(1) crumb, Min(0) top, Length(9) RECENT EVENTS ]` where top = `Horizontal[ Min(0) SERVICES, Percentage(42) [NEXT EVENT, TODAY] ]`.
5. **Components.** `crumb` (with a live clock), `panel` ×4, `status_dot` ×10, `badge`, `bigtext`, `bar`, a 2-column stats grid, an event list.
6. **Data.** `status.components` (all ten), `status.next_event`, `status.today` (total/passed/remaining/notifications sent/adhan plays/next notify), `status.recent` (the event log, today only, newest first).
7. **Keys.** `↑↓`/`PgUp`/`PgDn` scroll the event log · `r` force a refresh (re-probe every component and re-fetch prayer times) · `1`–`4` · `q`.
8. **Loading/error/empty.** Empty event log (fresh install, early morning) → `no events yet today` in `textDim`, not a blank panel. A component whose probe is in flight shows `Unknown` with a `○` mark rather than falsely reporting `Ok`. `r` shows `REFRESHING…` in the crumb.
9. **80×24.** The design hides the whole right column and the event sub-lines. Budget: 20 rows = SERVICES (10 rows + 2 border) + RECENT EVENTS (6 rows + 2 border). Ten components fit exactly; the event log becomes a scroll region of ~6 rows with one line per event.
10. **100×30 / 120×35.** 100×30: three panels in two columns plus the 7-row event log with sub-lines; `NEXT EVENT` panel is `active` with a 2-row countdown and gauge width 26. 120×35: same structure with more event rows visible (11), the TODAY grid in two columns of three, and the crumb showing `SYSTEM STATUS` left with the live `HH:MM:SS` right.

### 16.5 Notification Settings (`notif-settings`)

1. **Purpose.** Edit all five prayers' before/after minutes and their Adhan/notify toggles in one dense grid, with a timeline preview.
2. **State.** `NotifState { row: 0..5, col: Before | After | Adhan | Notif }`.
3. **Entry.** Settings → `Enable notifications` or `Per-prayer timing` (both push this screen, per the design's two action rows).
4. **Layout.** `Vertical[ Length(1) crumb, Length(10) PER-PRAYER TIMING, Min(0) TIMELINE PREVIEW ]`.
5. **Components.** `crumb` (hint `↑↓ row · TAB col · ←→ adjust`), `panel` ×2, a `Table` with a header row, `stepper` cells, `timeline` ×5.
6. **Data.** `draft.prayers[*]` for all five; `window.today.times` for the timeline's absolute labels.
7. **Keys.** `↑↓` row (wrapping) · `Tab`/`Shift-Tab` column · `←→` adjust by **±1**, clamped `0..=60` (the design's exact `Math.max(0, Math.min(60, …))`) · `Enter`/`Space` toggle on the Adhan/Notif columns · `s` save → `confirm` · `Esc` back to Settings.
8. **Loading/error/empty.** Never empty. If prayer times are unavailable, the timeline shows relative offsets only (`◆ ─5m─ ◉ adhan ─15m─ ◉ mosque`) without absolute clock labels — the numbers being edited do not depend on the API.
9. **80×24.** 3-column grid (`PRAYER · BEFORE · AFTER`); the Adhan and Notif columns are dropped, matching the design's `76px 1fr 1fr`, and remain editable on the Prayer Configuration screen. Timeline rows lose their absolute times. Budget: crumb 1 + grid (5 + header + 2 border = 8) + timeline (5 + 2 border = 7) = 16 of 20 ✓.
10. **100×30 / 120×35.** 5-column grid (`PRAYER · BEFORE ADHAN · AFTER ADHAN · ADHAN · NOTIF`) per the design's `100px 1fr 1fr 58px 58px`; timeline rows show absolute times and dim non-selected rows to the design's 0.35 opacity — rendered as `textDim` on the non-focused rows, since terminals have no alpha. 120×35 adds a footnote line: `values apply immediately after save · 0 disables that notification`.

### 16.6 Location (`location`)

1. **Purpose.** Show the active location in full and change it by city search, coordinates, or auto-detect.
2. **State.** `LocationState { mode: Search | Coords | Auto, query: String, results: Vec<CityMatch>, sel: usize, lat_buf: String, lng_buf: String, focus: Lat | Lng }`.
3. **Entry.** Settings → `Location` row → `Enter`. Also pushed directly from the ERROR screen's `[C] COORDINATES` button with `mode = Coords`.
4. **Layout.** `Vertical[ Length(1) crumb, Length(8) CURRENT LOCATION, Length(3) mode buttons, Min(0) mode panel ]`.
5. **Components.** `crumb` (`SETTINGS › LOCATION`), `panel` ×2, a `120px 1fr` label/value grid, three `TBtn`-style buttons, a text input with a blinking `█`, a results `List`, two numeric inputs.
6. **Data.** `config.location` (all six fields, lat/lng in green), `config.calculation.method` name; the embedded city index for search; the system timezone for auto-detect.
7. **Keys.** `←→` or `Tab` switch mode · in Search: printable chars append, `Backspace` deletes, `↑↓` select a result, `Enter` commit · in Coords: digits/`.`/`-`, `Tab` switch field, `Enter` validate + commit · in Auto: `Enter` accept the proposal · `Esc` back to Settings without changing anything.
8. **Loading/error/empty.** Search with no matches → `no cities matched "zzz" · try [ COORDINATES ]` in `textSub`. Invalid coordinates → an inline red message under the field (`latitude must be between -90 and 90`), never a screen change, so the user keeps their typing. Auto-detect failure → `E103` inline with a pointer to the other modes. Committing triggers a prayer-time refetch, and the crumb shows `RECALCULATING…` until it completes.
9. **80×24.** The CURRENT LOCATION grid becomes single-column pairs (`City` then value on the next line) only if the width forces it — at 80 columns `Length(12) Min(0)` still fits, so keep two columns. Mode buttons shorten to `[ SEARCH ] [ COORDS ] [ AUTO ]`. Coordinate inputs stack vertically (the design's explicit small-size behaviour). Results list limited to 3.
10. **100×30 / 120×35.** Full `[ SEARCH CITY ] [ COORDINATES ] [ AUTO-DETECT ]` labels; the coordinates panel uses side-by-side `Length(10) Length(22)` fields; results show a right-aligned population/timezone hint (`○ Sétif, Algeria          288k · Africa/Algiers`), which is real information the offline index gives us for free and which the design's plain rows leave room for at these widths.

### 16.7 Adhan Alert (`adhan-alert` — overlay)

1. **Purpose.** Unmissable announcement that a prayer time has begun; dismiss or snooze.
2. **State.** `Overlay::AdhanAlert { prayer, at, blink }` in the shell's overlay slot, not the stack.
3. **Entry.** The scheduler's `Adhan` event, or a locally-detected `PrayerState::Adhan` transition in observer mode. Never by keypress.
4. **Layout.** Full frame, `redBg`, a red border, **no header and no footer** (the design's `isOverlay`). `Vertical[ Length(3) top bar, Min(0) center, Length(2) bottom bar ]`; the center is vertically centered.
5. **Components.** `panel` (full-frame, red), `bigtext` (the prayer name at 3em → 3-row blocks), `hr` with an inline label (`─── PRAYER TIME ───`), `badge` with a 500 ms blink, `kbd_hint` ×2.
6. **Data.** The firing prayer and its instant, `config.location.label`, `config.prayers[p].after_minutes` (for `Mosque reminder in 15 min`), `config.adhan.preset` and `volume` (the bottom bar's `Adhan audio: Abdul Basit · Vol 80%`).
7. **Keys.** `Enter` dismiss + stop audio · `s` snooze 3 min (stop audio, re-arm) · `Esc` dismiss · **everything else is swallowed**, including `1`–`4` and `q`, except `Ctrl-C`.
8. **Loading/error/empty.** No audio player or no file → the bottom bar reads `Adhan audio: unavailable (no player found)` and the overlay still displays; the visual announcement is independent of playback (§10.1). Playback failing mid-way changes nothing visually.
9. **80×24.** The Arabic line `اللَّهُ أَكْبَرُ · اللَّهُ أَكْبَرُ` is measured with `unicode-width`; if its display width exceeds the frame it is **dropped**, not wrapped (A: broken Arabic shaping is worse than absence, and the transliteration `Allahu Akbar · Prayer time has begun` below it carries the meaning). `bigtext` prayer name still renders (3 rows). Budget: 24 rows = top 3 + center 14 + bottom 2 + borders 2 + slack 3 ✓.
10. **100×30 / 120×35.** Full content: the top bar's `SALAT` / blinking `◉ ADHAN` / `19:21`, the labelled rule, 3-row prayer name, `19:21 · SÉTIF, ALGERIA`, the Arabic line, the transliteration, a rule, `Mosque reminder in 15 min`, and the two buttons. At 120×35 add vertical breathing room rather than more content — this screen's power is its emptiness.

### 16.8 Mosque Reminder (`mosque-reminder` — overlay)

1. **Purpose.** Tell the user to leave for the mosque, N minutes after the Adhan.
2. **State.** `Overlay::MosqueReminder { prayer, adhan_at, blink }`.
3. **Entry.** The scheduler's `MosqueReminder` event at `t + after`.
4. **Layout.** A **centered panel**, not a full-frame takeover — the design clamps it to `min(max(340px, 70%), 500px)` ≈ 44–64 columns. `Vertical` centered in the frame, header and footer suppressed.
5. **Components.** `panel` (title `MOSQUE REMINDER`), a blinking (800 ms) banner `◉ ── TIME TO HEAD OUT ── ◉`, `bigtext`-lite (1.6em → bold single row, not 3-row blocks, because the panel is small), `hr` ×2, `kbd_hint` ×2.
6. **Data.** The prayer, its Adhan instant, `after_minutes` (`Adhan was 15 minutes ago · 19:21`), and the literal `Iqama in approximately ~5 minutes` — which is genuinely approximate and must keep the design's `~` and the word "approximately", since Salat has no Iqama data.
7. **Keys.** `Enter` dismiss · `s` snooze 5 min · `Esc` dismiss. Others swallowed except `Ctrl-C`.
8. **Loading/error/empty.** Nothing to load. If it fires via catch-up (§11.5) the sub-line becomes `Adhan was 18 minutes ago · 19:21 (delayed notice)` so the user is not misled about elapsed time.
9. **80×24.** Panel width `min(56, w − 4)`; the two buttons stack on one row as `[ENTER] DISMISS   [S] SNOOZE 5m`; the banner shortens to `◉ TIME TO HEAD OUT ◉`.
10. **100×30 / 120×35.** Panel width clamped to 64 columns and centered — it does **not** grow with the terminal, matching the design's `max 500px`. The extra space is empty, which is the point.

### 16.9 Confirm / Saved (`confirm`)

1. **Purpose.** Confirm that settings were written to disk, and say where.
2. **State.** None (`Screen::Confirm` carries the message it was pushed with).
3. **Entry.** Pushed by a successful `[S] SAVE` on Settings, Notification Settings, Prayer Configuration, or Location.
4. **Layout.** A centered panel ~44 columns wide, `Length(9)` tall, with header and footer retained.
5. **Components.** `panel` (green border, `greenBg`, title `SAVED`), a large `✓`, three text lines, `kbd_hint`.
6. **Data.** The literal config path from `ConfigStore::path()` — rendered as `~/…` with `$HOME` collapsed, exactly as the design shows `~/.config/salat/config.toml`, plus `Prayer times recalculated.` shown only when the save actually changed the API cache key (otherwise `Settings applied.` — a small honesty fix; claiming a recalculation that did not happen is a lie the design didn't intend).
7. **Keys.** `Enter` pop back to the screen beneath · `Esc` same · any other key ignored.
8. **Loading/error/empty.** This screen exists only on success; failures push `error` instead. If the recalculation is still in flight, the third line reads `Recalculating prayer times…` and updates in place when it completes.
9. **80×24.** Panel width `min(48, w − 4)`; the path line is the constraint at 33 characters and fits.
10. **100×30 / 120×35.** Identical, centered. Fixed size — a confirmation does not need more room.

### 16.10 Error (`error`)

1. **Purpose.** Explain a failure in the design's exact idiom — a numbered code, a stack of `→` reasons, and a `help:` line — with recovery actions.
2. **State.** `Screen::Error(SalatError)` — the typed error itself, so rendering is derived, never hand-assembled at the call site.
3. **Entry.** Any operation that fails with a user-visible error: boot steps 1–3, a save failure, a location lookup failure, a refresh failure.
4. **Layout.** A centered panel ~64 columns, `Vertical[ Length(2) title, Min(3) body, Length(6) trace, Length(2) buttons ]`, header and footer retained.
5. **Components.** `panel` (red border, `redBg`, title `ERROR`), a `✕` title line, a wrapped body `Paragraph`, a trace block on `#100808`, `kbd_hint` ×3.
6. **Data.** `err.code()` → `error[E001]`, `err.title()` → `LOCATION LOOKUP FAILED`, `err.details()` → the `→` lines including the design's `retried 3× (1s · 2s · 4s)`, `err.help()` → the `help:` line, and `err.actions()` → which buttons to show.
7. **Keys.** From `err.actions()`: `r` retry (re-run the failed operation) · `c` coordinates (jump to Location in Coords mode) · `d` use defaults (config errors only) · `Esc` back. Only applicable buttons render.
8. **Loading/error/empty.** `r` shows `RETRYING…` with the attempt counter (`attempt 2 of 3`) so the retry policy is visible while it happens, not only afterwards in the trace.
9. **80×24.** Panel width `min(72, w − 4)`; the trace block truncates to 4 lines with `…` and the full text goes to the log file, with the body noting `full detail in ~/.local/state/salat/salat.log`.
10. **100×30 / 120×35.** Full 6-line trace, full body, all applicable buttons on one row. Panel stays clamped at 72 columns — a wider error is not a better error.

### 16.11 Loading (`loading`)

1. **Purpose.** Show the five-stage boot sequence with real progress.
2. **State.** `BootState { stage: 0..5, error: Option<SalatError>, blink }`.
3. **Entry.** Process start in full-TUI mode, before anything else renders.
4. **Layout.** A centered panel ~48 columns, `Vertical[ Length(3) brand, Length(5) steps, Length(2) progress ]`. Header and footer suppressed (nothing is navigable yet).
5. **Components.** `panel` (title `INITIALIZING`), the `SALAT` brand line, the subtitle `v0.1.0 · Rust · Ratatui · Crossterm`, five step lines with `✓` / blinking(300 ms) `▶` / `○`, `bar` width 38, and the `{pct}% · Sétif, Algeria` label.
6. **Data.** The current stage index, and the location label once stage 2 completes (before that the label reads `starting…` — the design shows the location during loading, which is only knowable after step 2).
7. **Keys.** `Ctrl-C` abort. Nothing else — there is nothing to navigate.
8. **Loading/error/empty.** This *is* the loading state. A failing stage turns its `▶` into a red `✕`, the panel border turns red, and after a 400 ms beat the ERROR screen replaces it with the appropriate code (§15.7).
9. **80×24.** Panel width `min(52, w − 4)`; step labels shorten (`Calculating prayer times` → `Prayer times`); bar width 30.
10. **100×30 / 120×35.** Full step labels, bar width 38, and a per-step elapsed time on the right (`Resolving location            0.02s`) — useful when a boot is slow and free to render at this width.

### 16.12 Prayer States reference (`states` — bound to `?`, per A7)

1. **Purpose.** Document all seven states, their glyphs, colors and meanings — the legend for the badge vocabulary, and the discoverability path for the `c` completion binding (A3).
2. **State.** None; static content driven by `UiModel` only for the live sample values.
3. **Entry.** `?` from any main screen (A7). `Esc` returns.
4. **Layout.** `Vertical[ Length(1) crumb, Min(0) seven rows ]`, each row `Length(3)` at medium/large and `Length(2)` at small.
5. **Components.** `crumb` (`PRAYER STATES`), seven state rows each with `badge`, `MAGHRIB · 19:21`, a description, a live visual, and per-state background/border from the design's table.
6. **Data.** The design's seven fixed rows plus live values for the ones that show them: `next → 01:42:37` (the real countdown), `pre-adhan → 12:47`, `adhan → blinking ◉ LIVE`, `post-adhan → +15m`.
7. **Keys.** `↑↓` scroll if clipped · `Esc` back · `1`–`4` jump to a main screen.
8. **Loading/error/empty.** Static; nothing can fail. Without live prayer data the countdown columns show `--:--:--`.
9. **80×24.** Seven 2-row entries = 14 rows + crumb + borders = fits in 20; descriptions truncate to one line with `…`.
10. **100×30 / 120×35.** Seven 3-row entries with full descriptions and the live visual column; at 120×35 add a footer note documenting the keybinding: `press [c] on the dashboard to mark a prayer completed`.

### 16.13 Terminal-too-small guard (addition, not in the design)

Rendered instead of any screen when `w < 80 || h < 24`: a centered panel `TERMINAL TOO SMALL` / `salat needs 80×24 · this terminal is 74×20` / `resize to continue`, using only ASCII and default colors so it renders even in a 20×5 terminal. It reads the live size each frame and disappears the moment the terminal is large enough. Only `q` and `Ctrl-C` are handled.

### 16.14 Quick mode (inline) — the layout for bare `salat`

Quick mode is the default invocation, so its geometry is pinned here rather than left to be derived. This subsection **supersedes the one-sentence sketch in §3.6 A8** and is the authoritative reconciliation of §15.3 (viewport height) with §16.1 (dashboard layout). It introduces no component, no data field and no key that §16.1 does not already have; it only compresses rows.

**Row budget, with arithmetic.** Let `h` be the terminal height in rows and `w` its width in columns.

```
viewport height   v = min(14, h − 1)                      (§15.3)
chrome            1 row — the header line
                  (no footer, no tab bar — §15.3)
content rows      c = v − 1

h = 24  →  v = min(14, 23) = 14  →  c = 13
h = 15  →  v = min(14, 14) = 14  →  c = 13   ← smallest full frame
h = 14  →  v = 13                →  c = 12
h = 10  →  v =  9                →  c =  8   ← lowest frame that renders at all
```

§16.1's small body is `Length(6) + Min(11) + Length(1)` = **18 rows**, which fits §4.4's 20-row full-TUI body with 2 rows of slack. Eighteen does not fit in thirteen, so quick mode renders the same two panels in a compressed form — five rows are removed and nothing is added:

```
NEXT PRAYER panel      5   border 1 · "● MAGHRIB · 19:21" 1
                           · "01:42:37  REMAINING" 1 · gauge 1 · border 1
TODAY block            7   head 1 · rule 1 · 5 prayer rows   (borderless)
service strip          1
                     ────
                      13   = c  ✓
header line            1
                     ────
                      14   = v  ✓
```

The five rows removed relative to §4.4's allocation are: NEXT PRAYER's separate `REMAINING` row, folded onto the countdown row (−1); and TODAY's two borders, its lower rule and its `Hijri … Qibla …` line (−4). `18 − 5 = 13` ✓. The Hijri date moves to the status strip and Qibla is not shown in quick mode — it lives on the full TUI's dashboard, which is the only surface that carries it. No `salat qibla` subcommand exists; §14.4 keeps the CLI surface closed.

**Zero slack is safe here, unlike in §4.4.** §4.4 keeps 2 rows of slack because a wrapped status line would push the layout past the budget. Quick mode never wraps: `Wrap` is not used by any widget in this frame, and every line is hard-truncated (see item 10). With wrapping impossible, the row count is a constant, so exact occupancy is sound.

1. **Purpose.** Answer "what's next and how long" in a terminal the user is already using, leave the answer in scrollback, and cost nothing to run.
2. **State.** A single `QuickView`; no `Screen` enum, no nav stack, no cursor. The shell's overlay slot exists and is used (§16.15). Everything rendered comes from `UiModel`, exactly as in §16.1.
3. **Entry.** Bare `salat` with a TTY stdout. `salat --once`, and any invocation whose stdout is not a TTY, goes to §14.6 instead and never opens a viewport (§14.3).
4. **Layout.** `Vertical[ Length(1) header, Length(5) NEXT PRAYER, Length(7) TODAY, Length(1) strip ]` at `c = 13`, evaluated against the viewport area. **Quick mode pins `Breakpoint::Small` and does not call §4.5's classifier.** This is not a preference: §4.5 tests `h >= 24`, the inline frame is at most 14 rows tall, so classifying the *viewport* would return `too_small` and render §16.13's guard on every run. Quick mode therefore (a) pins `Small`, and (b) evaluates its own floor against the *terminal* size, not the viewport's (item 9). §16.13's guard belongs to the full TUI only.
5. **Components.** `panel` ×1, `badge`, `bar`, `prayer_table`, `hr`, `status_dot` ×4, `header`. No `footer`, no `bigtext` (item 4 pins `Small`, and A1 makes the small countdown single-row), no `crumb`, no `stepper`.
6. **Data.** `next`, `countdown`, `progress`, `states[5]`, `window.today.times`, `window.today.hijri`, `status.components`, `config.location.label`. A strict subset of §16.1 item 6; `qibla`, `window.tomorrow`, `previous` and the per-prayer notify offsets are not read.
7. **Keys.** `q` quit · `Esc` quit · `Ctrl-C` quit · `Ctrl-L` force redraw. Nothing else is handled — no `1`–`4` (there is one screen), no `↑↓`, no `c` (completion toggling needs a cursor the frame has no room to advertise), no `?`. When an overlay is up, §15.4's resolution order gives it `Esc` first, so `Esc` dismisses the overlay and a second `Esc` quits (§16.15).
8. **Loading/error/empty.** No loading screen — §15.7 skips it in quick mode because it would flash for 200 ms. A fatal failure at boot steps 1–3 prints one line to stderr and exits with §14.5's code; no viewport is opened and no partial frame is printed. Non-fatal degradation is shown in place: a failed refresh with a warm cache dims `● svc` to yellow and appends `· cached` to the TODAY head row; missing audio or notifications dim their own dots and change nothing else.
9. **Minimum height, and the degradation ladder.** The full frame needs `c = 13`, i.e. **`h >= 15`**. Below that the frame degrades by dropping rows in a fixed order, one row per step:

   | `h` | `c` | Rows dropped, cumulative |
   |---|---|---|
   | ≥ 15 | 13 | — (full frame) |
   | 14 | 12 | service strip |
   | 13 | 11 | + TODAY's rule |
   | 12 | 10 | + TODAY's head row |
   | 11 | 9 | + one prayer row |
   | 10 | 8 | + a second prayer row |
   | ≤ 9 | ≤ 7 | no viewport is opened |

   The header, the NEXT PRAYER panel and at least three prayer rows therefore always survive. **Which prayer row is dropped** is deterministic: with the five rows indexed `0..4` in list order and `n` the index of the next prayer's row, drop `argmax |i − n|`, ties going to the lower index. With Maghrib next (`n = 3`) the distances are `3 2 1 0 1`, so Fajr goes first and then Dhuhr, leaving Asr · Maghrib · Isha. The next prayer is never the row that is dropped.

   **Below the floor** — `h < 10` or `w < 80` — quick mode does not open a viewport at all: it prints the §14.6 single-line summary to stdout, exits `0`, and never enters raw mode. This is the correct degradation for a terminal too small to hold a frame, and it keeps `salat` usable from a one-line prompt or a status bar. §16.13's `TERMINAL TOO SMALL` panel is not used here; there is no point rendering an error into a viewport that cannot hold the error.

10. **Width, truncation, and wider terminals.** The frame spans the full terminal width and the row budget does not change with `w`; only truncation and the strip's right group are width-dependent. Because item 4 pins `Small`, a 200-column terminal gets the 80-column layout stretched, not the medium or large dashboard — those need 26 and 31 body rows (§4.4) and cannot be expressed in 13. Concretely: the NEXT PRAYER gauge is **fixed at 34 cells** (§16.1's small width) and extra width is left empty; the prayer table keeps the small 3-column geometry `Length(9) Length(7) Min(0)`; the strip's left group is `● svc  ● notif  ● adhan  ● audio` (32 columns: `5 + 2 + 7 + 2 + 7 + 2 + 7`) and its right group is `Sétif · 12 Safar 1448 AH`. **Truncation is by display width, never by wrapping:** every line is measured with `unicode-width` and cut to the viewport width; a right-aligned group that would collide with the left group is dropped whole rather than half-drawn; only the TODAY note column, where the design already shows an ellipsis, ends in `…`. No line may exceed the buffer width — §20.2 already asserts this for every snapshot.

**Tick cadence.** Identical to the full TUI: the §15.2 `select!` loop with a 250 ms tick, the same 20 Hz draw cap, and the same clock-display rule that seconds change only on second boundaries (§15.4). Quick mode does not get a cheaper cadence; the countdown is the whole point of the screen, and 250 ms is what the design's blink rates require (§3.2).

**Day rollover.** The local date is derived from `Clock::now()` on every tick. When it changes, the model reloads `window.today` from the state machine and the next frame renders the new day — no restart, no re-lock, no special case for a session left open overnight. An observer-mode quick session rolls over identically, because the model is computed from the state machine rather than from event messages (§4.3).

**Configuration changes.** Quick mode subscribes to the same `watch::Receiver<Arc<Config>>` as the full TUI and has `cfg_rx.changed()` as a `select!` arm (§15.2), so a `salat --full` session or a SIGHUP elsewhere that changes the method, the location or the time format is reflected in the running inline frame within one tick. Quick mode never *writes* configuration — there is no settings UI and `ConfigStore::save` is never called — so it can never be the process that triggers a replan. `salat --once` renders one frame and exits, and therefore does not subscribe at all.

### 16.15 Overlay behaviour inside the inline viewport

§16.7 and §16.8 specify the Adhan Alert and the Mosque Reminder as full-TUI screens at 80×24 and above. But §4.3 lets bare `salat` win the scheduler lock and fire, and §4.3 further makes both overlays UI reactions to the state machine, so an *observer* quick session shows them too. Both overlays must therefore have a defined presentation in a 14-row inline viewport. This subsection defines it as a delta to §16.7 and §16.8; every rule those subsections state about data, wording, blink rates and audio is unchanged.

**The overlay owns the whole viewport.** It clears the viewport and is drawn from scratch — the dashboard beneath is not drawn, and the header line is taken over as well, which is consistent with §3.3's `isOverlay` suppressing header and footer. The overlay's maximum dimensions are therefore the viewport's: at most 14 rows by the full terminal width. Compositing an overlay *over* a partially visible dashboard is specifically rejected; in 14 rows the leftover rows read as corruption rather than as depth.

Within that cleared viewport the two overlays keep their designed character. Adhan paints the full area `redBg` with a red border, as §16.7 item 4 requires. Mosque draws a centered panel on the normal ground, as §16.8 item 4 requires — "centered in empty space" is exactly what the design shows, and the emptiness is real because the viewport was cleared.

**Adhan Alert, inline.** Eleven rows, centered vertically:

```
◉ ADHAN                                     19:21     1   blink 500 ms · time right
                                                      1
███ ███ ███  (MAGHRIB — 3-row bigtext, 27 cols)       3   §15.5
                                                      1
اللَّهُ أَكْبَرُ · اللَّهُ أَكْبَرُ                                 1   dropped if too wide (§16.7 item 9)
19:21 · SÉTIF, ALGERIA                                1
                                                      1
──────────────────────────────────────────            1
[ENTER] DISMISS   [S] SNOOZE 3m                       1
                                                    ────
                                                     11   in v = 14 → 3 slack
```

`MAGHRIB` is 7 characters, so §15.5's formula gives `4 × 7 − 1 = 27` columns — it fits an 80-column terminal with room to spare, which is why A1's "bigtext for the Adhan prayer name at every size" survives inline. Vertical placement is deterministic: `top_pad = (v − block) / 2` by integer division, remainder below, so at `v = 14` there is one row above and two below.

**Mosque Reminder, inline.** Nine rows, in a panel of `min(56, w − 4)` columns (§16.8 item 9), centered horizontally and vertically by the same rule:

```
╭─ MOSQUE REMINDER ──────────────────────╮   1
│ ◉ ── TIME TO HEAD OUT ── ◉             │   1   blink 800 ms
│ ────────────────────────────────────── │   1
│ MAGHRIB                                │   1   bold single row
│ Adhan was 15 minutes ago · 19:21       │   1
│ Iqama in approximately ~5 minutes      │   1
│ ────────────────────────────────────── │   1
│ [ENTER] DISMISS   [S] SNOOZE 5m        │   1
╰────────────────────────────────────────╯   1
                                           ────
                                             9   in v = 14 → 5 slack
```

**Not enough space.** The viewport height is chosen once, as `v = min(14, h − 1)`; on resize the width follows the terminal and the height is clamped to `min(v, h)`. The drop ladder is re-evaluated against the current height on every frame, so a shrinking terminal degrades rather than clips. Rows come off in a fixed order until the block fits:

| Adhan | rows | Mosque | rows |
|---|---|---|---|
| full | 11 | full | 9 |
| − the two blanks flanking the name | 9 | − the upper rule | 8 |
| − the blank above the rule | 8 | − the prayer-name row | 7 |
| − the Arabic line | 7 | − the lower rule | 6 |
| − the rule | 6 | *floor* | 6 |
| − the location line | 5 | | |
| − name collapses to a single bold row | 3 | | |
| *floor* | 3 | | |

Mosque drops its prayer-name row before its rules because the banner already names the event. Below either floor — a viewport of 1 or 2 rows, which quick mode's own floor makes unreachable in practice — the overlay becomes a single truncated line: `◉ ADHAN · MAGHRIB 19:21 · [ENTER] dismiss  [S] snooze`, or `◉ TIME TO HEAD OUT · MAGHRIB · Iqama ~5 min · [ENTER] dismiss  [S] snooze`. The overlay is never skipped for want of space; a missed call to prayer is a worse failure than an ugly one.

**Keys.** §15.4's resolution order is unchanged and settles every case: level 1 is `Ctrl-C` and `Ctrl-L`, level 2 is the overlay. So while an overlay is up, `Enter` dismisses it and stops audio, `s` snoozes — **3 minutes for Adhan, 5 for Mosque** (§16.7 item 7, §16.8 item 7) — `Esc` dismisses it, and everything else is swallowed, including `q` and `1`–`4`. Only `Ctrl-C` passes through.

This has one consequence worth stating outright, because §15.3 lists `Esc` among quick mode's exit keys: **with an overlay up, `Esc` dismisses the overlay and does not quit.** A second `Esc`, with no overlay present, quits. `q` while an overlay is up does nothing at all — the user must acknowledge the call to prayer first, which is the design's intent.

**Dismissal is manual.** `Enter`, `Esc`, or `s` (which hides it and re-arms at `+3` / `+5` minutes). Neither §16.7 nor §16.8 specifies an auto-dismiss timer, so none is introduced: the overlay persists until the user acts, including across a day rollover. Audio lifetime follows §13.6 exactly and is *not* symmetric across the three keys: `Enter` stops playback, because `[ENTER] DISMISS` on a red screen means "silence this"; `s` stops it and re-arms; `Esc` removes the overlay and lets the file play to its end. Nothing here changes that — the inline viewport reuses the same handler.

**When the event fires and the user is not in the full TUI.** Five cases, exhaustively:

| Situation | Overlay | Notification + audio |
|---|---|---|
| `salat` (quick), holds the lock | inline, per this subsection | yes — it is the scheduler |
| `salat` (quick), observer mode | inline, per this subsection | no — the daemon owns them (§4.3) |
| `salat --full` | §16.7 / §16.8 unchanged | per lock ownership, as above |
| `salat daemon` only, no terminal open | none — there is nothing to draw on | yes |
| `salat --once` | never — it renders one frame and exits | never — it does not take the lock |

Nothing is queued for a later TUI. When a session starts after an Adhan has already fired, the dashboard simply renders that prayer in its post-adhan state and the event log carries the row; see §11.5 for the late-fire rule that governs whether a delayed fire raises an overlay at all.

---

## 17. Persistence and caching

### 17.1 What is stored where, and why

| Data | Store | Justification |
|---|---|---|
| Configuration | `~/.config/salat/config.toml` | The design displays this path; users expect a hand-editable dotfile; XDG-conventional. |
| Prayer days | SQLite | Enables offline correctness and cuts requests ~30× (§7.6). |
| Fired events | SQLite | The **only** way a restart knows what already fired (§11.4). |
| Event log | SQLite | The Status screen's `RECENT EVENTS · TODAY` panel requires it. |
| Prayer completions | SQLite | The `Completed` state must survive a restart within the day (A3). |
| Qibla direction | SQLite | One value per location, never changes; avoids a request per launch. |
| **Not stored** | — | Window sizes, cursor positions, last screen, search history, statistics beyond today, logs (those go to a rotating file). |

`$XDG_DATA_HOME/salat/salat.db` (typically `~/.local/share/salat/salat.db`), mode `0600`.

### 17.2 Schema (five tables — the design's minimum, no more)

```sql
-- migrations/0001_init.sql
CREATE TABLE prayer_days (
    cache_key  TEXT NOT NULL,        -- hash of the location+calculation tuple
    date       TEXT NOT NULL,        -- YYYY-MM-DD, civil date in tz
    tz         TEXT NOT NULL,        -- IANA zone as fetched
    timings    TEXT NOT NULL,        -- JSON: { fajr: RFC3339, …, midnight, last_third }
    hijri      TEXT NOT NULL,        -- JSON: { day, month_number, month_en, month_ar, year }
    meta       TEXT NOT NULL,        -- JSON: { method_id, method_name, school, lat, lng, tz }
    fetched_at TEXT NOT NULL,        -- RFC3339
    etag       TEXT,                 -- for optional revalidation
    PRIMARY KEY (cache_key, date)
);
CREATE INDEX idx_prayer_days_date ON prayer_days(date);

CREATE TABLE fired_events (            -- §11.4
    date TEXT NOT NULL, prayer TEXT NOT NULL, kind TEXT NOT NULL,
    fired_at TEXT NOT NULL, late INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (date, prayer, kind)
);

CREATE TABLE event_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at       TEXT NOT NULL,          -- RFC3339
    category TEXT NOT NULL,          -- 'notification'|'adhan'|'state'|'error'|'system'
    label    TEXT NOT NULL,          -- '  Pre-Adhan notification'
    detail   TEXT,                   -- 'MAGHRIB · 5 min window'
    prayer   TEXT
);
CREATE INDEX idx_event_log_at ON event_log(at DESC);

CREATE TABLE prayer_completions (
    date TEXT NOT NULL, prayer TEXT NOT NULL,
    completed_at TEXT NOT NULL,
    PRIMARY KEY (date, prayer)
);

CREATE TABLE qibla (
    cache_key TEXT PRIMARY KEY,
    direction REAL NOT NULL,
    fetched_at TEXT NOT NULL
);
```

Five tables, no joins across them, no ORM layer, no repository abstraction over `sqlx`. `cache::Store` exposes ten typed methods and nothing more.

**`cache_key`** = the readable canonical string itself. **It is not a hash and nothing in the cache layer hashes anything.** It is built with one `format!` from the resolved settings, in exactly this field order — latitude, longitude, method, school, latitude-adjustment rule, midnight mode, timezone, tune vector — with the two coordinates fixed at four decimal places so that `36.19` and `36.1900` cannot produce two rows for one location:

```
"36.1900,5.4100,m12,s0,l3,mm0,tz=Africa/Algiers,tune=0-0-0-0-0-0-0-0-0"
```

Any change to a field's order, name or formatting is a cache-key change and therefore a migration (§17.4). Storing the string rather than a digest is deliberate: a readable key makes cache bugs inspectable with `sqlite3` and costs nothing at this row count. The contrast with §8.1 is worth stating so the two are not confused — the `--config` scope hashes only because a *filename* cannot carry a seventy-character string, and it keeps a readable stem in front of the digest for the same debuggability reason.

### 17.3 Cache semantics (explicitly defined, per the brief)

**Purpose.** The cache exists so that (a) Salat starts instantly and works fully offline for dates it has already fetched, and (b) AlAdhan receives ~12 requests per year per configuration instead of ~365.

**Immutability and expiry.** A `prayer_days` row is **immutable and never expires**. AlAdhan's answer for `(location, calculation, date)` is a deterministic astronomical computation — it does not change tomorrow. There is therefore no TTL, no staleness heuristic, and no revalidation on the hot path. This is what makes offline operation *correct* rather than approximate, and it is why no local calculation engine is needed as a fallback.

**Invalidation.** Rows are never updated in place. A configuration change produces a new `cache_key`, so the old rows simply stop being read. `cache::vacuum()` deletes rows older than 60 days and rows whose `cache_key` is not the current one and whose `fetched_at` is older than 7 days — run once at boot, bounding the database at a few hundred kilobytes.

**Limitations, stated plainly (and surfaced in the UI).** The cache cannot answer for a date never fetched. If Salat is offline and the user asks for a date outside the cached range, the honest answer is an error (`E301`), not a guess. The dashboard footer appends `· cached` and the Network component reads `OFFLINE (using cache)` whenever the current window was served entirely from cache while the last network attempt failed, so a user always knows whether they are looking at fresh or stored data. Because we prefetch the next month at month-end, the practical offline horizon is 1–2 months, which covers travel and outages.

### 17.4 Connection and migrations

One `SqlitePool` with `max_connections(2)` (the runner writes, the UI reads). `journal_mode = WAL`, `synchronous = NORMAL`, `busy_timeout = 5s`, `foreign_keys = ON`. Migrations via `sqlx::migrate!("./migrations")`, embedded in the binary, run at boot step 3 — so a user upgrading never runs a migration command.

**Corrupt database recovery:** if `migrate` or the first query fails with a corruption error, rename the file to `salat.db.corrupt-<timestamp>`, recreate, log it, and continue. Prayer times are re-fetchable; the only real loss is today's event log. Never crash on a corrupt cache.

---

## 18. Error handling

### 18.1 Strategy

Three layers, each with a distinct job — the point being to avoid the failure mode the brief names: a codebase of generic strings.

1. **Typed domain errors** — one `thiserror` enum per module (`ApiError`, `ConfigError`, `CacheError`, `AudioError`, `NotifyError`, `LocationError`, `SchedulerError`). These are for *code* to match on: `ApiError::RateLimited` triggers a backoff, `ApiError::InvalidMethod` triggers the MWL fallback (§7.7). No module's error type mentions another module's internals.
2. **A user-facing error** — `SalatError`, the enum the ERROR screen renders. Each variant maps to a design-mandated code and knows how to present itself. Module errors convert into it via `From`, and *that conversion* is where a technical cause becomes a human explanation.
3. **The process boundary** — `main.rs` returns `color_eyre::Result<()>`, with a panic hook that restores the terminal before printing. Panics are bugs; every *expected* failure is a typed error long before it gets here.

### 18.2 The error code table (derived from the design's `error[E001]`)

```rust
impl SalatError {
    fn code(&self)    -> &'static str;   // "E001"
    fn title(&self)   -> String;         // "LOCATION LOOKUP FAILED"
    fn details(&self) -> Vec<String>;    // the "→" lines
    fn help(&self)    -> String;         // the "help:" line
    fn actions(&self) -> &[ErrorAction]; // which buttons the screen shows
}
```

| Code | Variant | Title | Actions |
|---|---|---|---|
| `E001` | `LocationLookupFailed` | LOCATION LOOKUP FAILED | Retry, Coordinates, Back |
| `E101` | `InvalidCoordinates` | INVALID COORDINATES | Coordinates, Back |
| `E102` | `InvalidMethod` | INVALID CALCULATION METHOD | Back |
| `E103` | `TimezoneUndetermined` | TIMEZONE NOT DETECTED | Coordinates, Back |
| `E201` | `ConfigInvalid` | CONFIGURATION INVALID | Retry, UseDefaults, Back |
| `E202` | `ConfigWriteFailed` | COULD NOT SAVE SETTINGS | Retry, Back |
| `E301` | `PrayerTimesUnavailable` | PRAYER TIMES UNAVAILABLE | Retry, Back |
| `E302` | `ApiUnreachable` | API UNREACHABLE | Retry, Back |
| `E303` | `ApiMalformed` | UNEXPECTED API RESPONSE | Retry, Back |
| `E304` | `ApiRateLimited` | RATE LIMITED | Retry, Back |
| `E401` | `AudioNoPlayer` | NO AUDIO PLAYER FOUND | Back |
| `E402` | `AudioNoFile` | ADHAN FILE MISSING | Back |
| `E403` | `AudioPlaybackFailed` | ADHAN PLAYBACK FAILED | Retry, Back |
| `E501` | `NotificationsUnavailable` | NOTIFICATIONS UNAVAILABLE | Back |
| `E601` | `DatabaseFailed` | DATABASE ERROR | Retry, Back |

`E001` reproduces the design's example exactly, including its details (`→ HTTP 503 Service Unavailable`, `→ retried 3× (1s · 2s · 4s)`, `→ giving up`) and its help line (`use [C] coordinates or [R] retry`).

### 18.3 Propagation rules

- `domain` returns `Result<T, DomainError>` for the few fallible pure operations (parsing a timezone string, validating a tune array). It never wraps external errors, because it never performs I/O.
- Services return their own typed error. `?` inside a service converts a foreign error into that service's variant via `#[from]`, keeping the source chain intact.
- The boundary from service to UI is exactly one `From<XError> for SalatError` impl per service, and it is the only place error *presentation* is decided. A coding model should not be writing `format!("failed to …")` anywhere else.
- **Non-fatal failures never become errors at all.** Missing audio player, absent D-Bus, unreachable network with a warm cache: these set `Component.health` and write an `event_log` row. The ERROR screen is for things that block the user, not for things that degrade.
- `unwrap`/`expect` are forbidden outside tests, enforced by `#![deny(clippy::unwrap_used, clippy::expect_used)]` in `lib.rs` with a documented allowance for `OnceLock` initialisation.

### 18.4 Logging

`tracing` to `$XDG_STATE_HOME/salat/salat.log`, single file, truncated at 5 MB on startup (a prayer app does not need log rotation infrastructure). Level from `-v` count then `RUST_LOG`. Every error that reaches the ERROR screen is logged at `error!` with its full source chain, which is what makes the design's `full detail in …salat.log` pointer (§16.10) truthful. **Coordinates are logged at `debug!` only, never at `info!`** — the log should not casually record where the user lives.

---

## 19. Project directory structure

```
salat/
├── Cargo.toml                    # workspace: members = ["salat", "xtask"]
├── Cargo.lock                    # committed — reproducible release builds
├── rust-toolchain.toml           # pinned stable channel + rustfmt, clippy
├── clippy.toml                   # disallowed-methods: Utc::now, Local::now, SystemTime::now
├── rustfmt.toml
├── deny.toml                     # cargo-deny: licenses, advisories, bans
├── install.sh                    # THE install script — must live at repo root (raw URL)
├── README.md
├── CHANGELOG.md
├── CONTRIBUTING.md
├── LICENSE                       # MIT OR Apache-2.0
├── LICENSE-THIRD-PARTY           # GeoNames CC-BY 4.0 attribution
│
├── .gitlab-ci.yml                # GitLab CI/CD (§22, §28 D1): fmt · clippy · test · shellcheck · deny · build;
│                                 # tag → cross-build → checksums → verify → manual GitHub Release
├── .github/
│   └── ISSUE_TEMPLATE/{bug_report.md,feature_request.md}   # issues stay on GitHub (canonical)
│
├── docs/
│   ├── ARCHITECTURE.md           # this document
│   ├── CONFIGURATION.md          # every config key, its domain, its default
│   ├── INSTALL.md                # install, upgrade, uninstall, systemd
│   └── screenshots/              # 80x24.png, 100x30.png, 120x35.png, adhan.png
│
├── packaging/
│   └── systemd/salat.service     # systemd **user** unit
│
├── xtask/
│   ├── Cargo.toml
│   └── src/main.rs               # gen-cities · screenshots · check-fixtures
│
└── salat/
    ├── Cargo.toml
    ├── assets/
    │   └── cities.bin            # generated city index (§9.2), committed
    ├── migrations/
    │   └── 0001_init.sql         # the five tables (§17.2)
    ├── tests/
    │   ├── fixtures/aladhan/*.json    # recorded API responses (§7.8)
    │   ├── cli.rs                     # assert_cmd: help, version, status, config, exit codes
    │   ├── api_failures.rs            # wiremock: 200, 400-string-data, 503, 304, malformed, timeout
    │   ├── scheduler.rs               # MockClock: firing, dedup, catch-up, jumps, rollover
    │   ├── config_roundtrip.rs        # write → read → compare; atomicity; bad files
    │   ├── cache.rs                   # in-memory SQLite: store/load, key changes, vacuum, corruption
    │   └── snapshots/                 # insta .snap files for every screen × 3 sizes
    └── src/
        ├── main.rs               # ~60 lines: color-eyre install, Cli::parse, dispatch, exit code
        ├── lib.rs                # module tree, crate lints, pub use of the public surface
        ├── error.rs              # SalatError, codes, titles, details, help, actions (§18.2)
        │
        ├── cli/
        │   ├── mod.rs            # Cli, Command (clap derive) — §14.2
        │   └── commands/         # status.rs · config.rs · test_adhan.rs · daemon.rs
        │                         # quick.rs (inline viewport) · full.rs (alternate screen)
        ├── app/
        │   ├── mod.rs
        │   ├── runtime.rs        # builds and owns every service; the ONLY wiring site
        │   ├── clock.rs          # Clock trait, SystemClock, MockClock — §6.6
        │   ├── boot.rs           # the five-stage sequence — §15.7
        │   ├── lock.rs           # scheduler lock file, foreground/daemon arbitration — §4.3
        │   └── signals.rs        # SIGINT/SIGTERM/SIGHUP wiring
        ├── config/
        │   ├── mod.rs            # Config + all sub-structs (serde) — §8.2
        │   ├── defaults.rs       # the design's mock data as defaults — §8.3
        │   ├── store.rs          # load, atomic save, watch publication — §8.5
        │   ├── validate.rs       # semantic validation, all violations — §8.4
        │   └── draft.rs          # ConfigDraft for the TUI's edit-then-save flow
        ├── domain/
        │   ├── mod.rs
        │   ├── prayer.rs         # Prayer, Notifiable, display/short — §6.1
        │   ├── day.rs            # PrayerDay, PrayerWindow, PrayerInstant — §6.2-6.3
        │   ├── state.rs          # PrayerState + state_of — §10
        │   ├── countdown.rs      # Countdown, progress, formatting — §6.5
        │   ├── schedule.rs       # ScheduledEvent, EventKind, plan_events — §11.3
        │   └── summary.rs        # DaySummary — the Status screen's counters
        ├── api/
        │   ├── mod.rs            # AlAdhanClient trait + impl
        │   ├── dto.rs            # ApiEnvelope + typed payloads (tolerant serde) — §7.3
        │   ├── params.rs         # query construction + client-side validation — §7.4
        │   ├── map.rs            # DTO → domain::PrayerDay
        │   └── retry.rs          # 3× · 1s/2s/4s + jitter, semaphore, timeouts — §7.5
        ├── cache/
        │   ├── mod.rs            # Store: pool, migrations, corruption recovery
        │   ├── prayer_days.rs    # upsert_month, load_day, cache_key, vacuum
        │   ├── fired.rs          # try_mark_fired (INSERT-or-skip) — §11.4
        │   ├── events.rs         # append, today() for the Status screen
        │   ├── completions.rs    # toggle, for_date — A3
        │   └── qibla.rs
        ├── location/
        │   ├── mod.rs            # resolve/validate; the three modes — §9.3
        │   ├── cities.rs         # cities.bin parser + search + nearest — §9.2
        │   └── system_tz.rs      # /etc/localtime → $TZ → timedatectl — §9.3
        ├── scheduler/
        │   ├── mod.rs            # Scheduler handle + commands
        │   ├── runner.rs         # the select! loop, tick, jump detection — §11.3
        │   └── catchup.rs        # grace windows and the missed decision — §11.5
        ├── notify/
        │   ├── mod.rs            # Notifier trait, RecordingNotifier — §12.5
        │   └── desktop.rs        # notify-rust in spawn_blocking, capability probe
        ├── audio/
        │   ├── mod.rs            # AudioPlayer trait, PlayHandle — §13.3
        │   ├── players.rs        # probe chain and argv construction — §13.2
        │   └── file.rs           # preset/path resolution and validation — §13.4
        ├── status/
        │   ├── mod.rs            # Collector — §15.8
        │   └── model.rs          # Health, Component, ComponentId, SystemStatus — §6.7
        └── ui/
            ├── mod.rs            # render(frame, &UiModel, &Shell) — the single entry point
            ├── model.rs          # UiModel + build_model — §15.1
            ├── shell.rs          # Shell, nav stack, overlay slot, per-screen state — §15.1
            ├── input.rs          # key resolution order — §15.4
            ├── theme.rs          # every design token + 256/ansi degradation — §15.6
            ├── layout.rs         # Breakpoint, region splitting, centering helpers — §4.5
            ├── screens/
            │   ├── dashboard.rs        prayer_config.rs   notif_settings.rs
            │   ├── settings.rs         status.rs          location.rs
            │   ├── adhan_alert.rs      mosque_reminder.rs confirm.rs
            │   └── error.rs            loading.rs         states.rs
            └── widgets/
                ├── panel.rs      bar.rs        badge.rs      prayer_table.rs
                ├── bigtext.rs    stepper.rs    kbd_hint.rs   crumb.rs
                ├── setting_row.rs timeline.rs  hr.rs         status_dot.rs
                └── chrome.rs     too_small.rs
```

**Why `main.rs` stays small.** It does four things: install the `color-eyre` hook, parse `Cli`, dispatch to a `cli::commands::*` function, map the returned error to an exit code. Every command function builds a `Runtime` and calls services. There is no global mutable state and no `App` struct with forty fields — the nearest thing, `Shell`, holds nine small per-screen structs and a navigation stack.

---

## 20. Testing strategy

### 20.1 Principles

- **Every phase ships its own tests.** A phase whose tests are deferred is not complete (§24).
- **The domain is where the density goes.** It is pure, so its tests are fast, deterministic and cheap to write — and it holds all the logic that would otherwise be untestable inside async services.
- **No test touches the network, the real clock, the user's config, the user's database, or the user's audio.** `MockClock`, `wiremock`, `tempfile` and `sqlite::memory:` make that absolute rather than aspirational.
- **No `sleep` in any test.** Timing tests advance `MockClock`.

### 20.2 Coverage by area (the brief's named list, each with its mechanism)

| Area | Mechanism | Representative assertions |
|---|---|---|
| Prayer time parsing | Unit + fixtures | ISO-8601 with offset → `DateTime<FixedOffset>`; `Midnight` on the next civil day; the `" (CET)"` suffix form is *rejected* (we always send `iso8601=true`); 30 days parsed from `calendar_ok.json` |
| Timezone handling | Unit, `chrono-tz` | `+01:00` → `Africa/Algiers` round-trip; a real DST date in `Europe/Berlin` (last Sunday of March) yields correct offsets on both sides; the location's zone, not the host's, defines "today" |
| Next prayer calculation | Unit, `MockClock` | before Fajr → Fajr; between Asr and Maghrib → Maghrib; after Isha → **tomorrow's Fajr**; exactly *at* Maghrib → Isha; Sunrise is never returned (A2) |
| Countdown calculation | Unit | `19:21 − 17:38:23` → `01:42:37`; 1 second before → `00:00:01`; 0 → `00:00:00`; crossing midnight → `06:15:00`, not a negative or panic |
| State transitions | Unit, `MockClock` | all eight boundaries of §10.3; `before = 0` skips `PreAdhan`; `after = 0` skips `PostAdhan`; `Completed` overrides everything; `PreAdhan` beats `Next` |
| Scheduler behaviour | Integration, `MockClock` + `RecordingNotifier` | fires once per event; a replay of the same tick fires nothing; a 6-hour jump fires only within-grace events; config change re-arms; day rollover refetches; `SIGHUP` replans |
| Configuration persistence | Integration, `tempfile` | write → read → equal; unknown key rejected; all violations reported at once; atomic write leaves no `.tmp`; a save failure preserves the draft; missing file creates defaults |
| API failures | Integration, `wiremock` | 200 success; 400 with **string** `data`; 503 on an out-of-range latitude; malformed/truncated JSON; connection refused; timeout; 304 from `If-None-Match`; 429 → retries then `E304`; 3 attempts with the exact 1s/2s/4s pattern (asserted against mock call timestamps, not wall time) |
| Location handling | Unit | search finds `Sétif` from `Setif`; coordinate validation rejects 999 **before any request**; nearest-city zone derivation; >200 km falls back to the system zone; `/etc/localtime` parsing from a fixture path |
| Notification timing | Integration | a 15-minute `before` on a 19:21 prayer fires at exactly 19:06; a disabled prayer fires nothing; `notifications.enabled = false` silences all five; a delayed fire carries the `(delayed)` prefix |
| Adhan timing | Integration | fires at exactly `t`; not fired when `adhan = false`; not fired when `adhan.enabled = false`; the visual `Adhan` state appears even when no player exists |
| CLI commands | `assert_cmd` | `--help` lists exactly the documented flags; `--version` matches `CARGO_PKG_VERSION`; `status` exits 0 and prints ten components and no path; `config` prints the four resolution lines of §8.1 in order, and under `--config` the scope suffix matches the FNV-1a of the canonicalized path; bad flag exits 2; invalid config exits 3; non-TTY stdout auto-selects `--once` and emits no ANSI |
| Terminal size behaviour | `TestBackend` + `insta` | every screen at 80×24, 100×30, 120×35; 79×23 renders the too-small guard; **no snapshot contains a truncation artifact or a line wider than the buffer** (asserted programmatically over the buffer, not by eye) |

### 20.3 Snapshot testing specifics

`ratatui::TestBackend` renders into a fixed buffer; `insta` stores the text form. Snapshots are golden files reviewed in PRs, which makes an accidental layout regression visible as a diff rather than as a bug report. Two mechanical assertions run alongside every snapshot: no cell contains a replacement character, and every rendered `Line` fits its `Rect` width (computed with `unicode-width`, which is what catches the Arabic-line problem of §16.7 automatically).

Fixed inputs make snapshots stable: `MockClock` at `2026-09-05T17:38:23+01:00`, the design's exact five prayer times, and the default config. Determinism is the point — with the clock pinned, the countdown, the gauge fill and the blink phase (§3.2) are all pure functions of the fixtures, so any diff means the layout itself changed.

**What a snapshot does not prove.** `insta` writes its first snapshot from whatever the code produced, and once that file is accepted it *becomes* the expected value. A first-run snapshot therefore records the render rather than checking it: if a widget was wrong from the outset, the snapshot enshrines the wrong output and every later run agrees with it. Snapshots are **regression baselines**. They prove the render has not moved since it was last reviewed; they never prove it matches Figma.

Design conformance is established exactly once per screen, by a human placing the rendered frame beside the Figma frame — §24.1 item 10 as the gate, §20.4 as the checklist — and is *protected* from then on by the snapshot. The order matters: compare against Figma first, accept the snapshot second. Accepting a snapshot is consequently a design decision and not a chore. `cargo insta accept` may only be run by a reviewer who has read the diff, and where that diff touches a screen the design specifies, who has re-checked the Figma frame. `cargo insta test --unreferenced=reject` runs in CI so an orphaned snapshot cannot linger unnoticed.

### 20.4 Manual verification (what automation cannot cover)

A checklist in `CONTRIBUTING.md`, executed per phase (§24) and before every release:

- Resize the terminal continuously across all three breakpoints and below 80×24; nothing overlaps, nothing panics, no artifacts remain.
- `TERM=xterm-256color` and `TERM=xterm` (16-color) — verify state distinctness without truecolor.
- With and without a notification daemon (`systemctl --user stop <daemon>`), and over SSH with no `$DBUS_SESSION_BUS_ADDRESS`.
- With each of `mpv`, `paplay`, `ffplay`, `aplay` as the only player on `PATH`, and with none.
- Airplane mode: cold start with an empty cache (expect `E301`), then with a warm cache (expect a working dashboard marked `cached`).
- Suspend the machine across a scheduled event, resume, and verify the catch-up decision matches §11.5.
- `salat` piped to `cat` and redirected to a file — no ANSI, no raw mode left behind.
- Ctrl-C during: the Adhan overlay, a network fetch, a config save. Terminal must be restored every time.

### 20.5 CI gates

`cargo fmt --check` · `cargo clippy --all-targets --all-features -- -D warnings` · `cargo test --all-features` · `shellcheck install.sh` · `cargo deny check` · `cargo build --release --target x86_64-unknown-linux-musl`. All six must pass in the GitLab CI/CD pipeline for every commit before that commit is pushed to GitHub (§22.1); the tag pipeline re-runs them before any release artifact is built (§22.2).

---

## 21. Linux installation architecture

### 21.1 The contract

```
curl -fsSL https://raw.githubusercontent.com/linuxcoding-ADAM/Salat/main/install.sh | sh
```

*(The repository owner, `linuxcoding-ADAM`, appears in `install.sh`'s `REPO` variable, `README.md`, and `docs/INSTALL.md`; §28 D7 lists every place the canonical URL is used.)*

End state: `salat` is an executable on the user's `PATH`. No Rust, no Cargo, no clone, no compiler, no `sudo`. One HTTP round trip for the script, one for the release metadata, two for the assets.

### 21.2 What the script does, in order

| Step | Action | Failure behaviour |
|---|---|---|
| 1 | `set -eu`; `IFS` reset; `umask 022`; define `REPO`, `BIN=salat` | — |
| 2 | Parse `--prefix DIR`, `--version vX.Y.Z`, `--uninstall`, `--help` | unknown flag → usage, exit 2 |
| 3 | Refuse to run as root unless `SALAT_ALLOW_ROOT=1` | message explaining Salat is a per-user tool, exit 1 |
| 4 | `uname -s` must be `Linux` | otherwise: "salat currently supports Linux only", exit 1 |
| 5 | `uname -m` → `x86_64\|amd64` ⇒ `x86_64`; `aarch64\|arm64` → not supported yet (§28 D8) | `aarch64`/`arm64` → `salat: aarch64 is not supported yet; supported: x86_64`, exit 1; any other → list supported arches (`x86_64`), exit 1 |
| 6 | Require `curl` **or** `wget`; require `tar`; require `sha256sum` **or** `shasum` | missing checksum tool → **abort**, never install unverified |
| 7 | Resolve the tag: `--version`, else parse `/releases/latest` | no tag → exit 1 |
| 8 | `TMP=$(mktemp -d)`; `trap 'rm -rf "$TMP"' EXIT INT TERM` | — |
| 9 | Download `salat-<ver>-<arch>-unknown-linux-musl.tar.gz` and `SHA256SUMS` | HTTP failure → print the URL, exit 1 |
| 10 | **Verify the checksum before extracting** | mismatch → print both digests, exit 1 |
| 11 | `tar -tzf` listing check, then extract into `$TMP/x` | any member that is absolute, contains `..`, or is not a regular file/dir → exit 1 |
| 12 | `install -d -m 0755 "$PREFIX"`; `install -m 0755 "$TMP/x/salat" "$PREFIX/salat"` | non-writable prefix → suggest `--prefix`, exit 1 |
| 13 | `"$PREFIX/salat" --version` smoke test | non-zero → remove the installed file, exit 1 |
| 14 | If `$PREFIX` is not on `PATH`, print the exact `export` line for the detected shell | — |
| 15 | Print the installed version, the resolved URL, the verified digest, and `run: salat` | — |

**Install location.** `${SALAT_INSTALL_DIR:-$HOME/.local/bin}` — XDG-adjacent, already on `PATH` in most modern distributions, and writable without privilege. `--prefix` overrides it. The script never writes to `/usr/local/bin`, never invokes `sudo`, never creates a systemd unit, and never modifies a shell rc file: it *prints* the `export` line and lets the user decide. Editing someone's `.zshrc` from a piped shell script is exactly the behaviour that makes people distrust `curl | sh`.

**Latest-release resolution without `jq`.** GitHub's `/releases/latest` HTML page redirects to `…/releases/tag/vX.Y.Z`; the script follows redirects and reads the final URL from `curl -sIL -o /dev/null -w '%{url_effective}'`. This needs no JSON parser and no API token, and therefore no rate-limited authenticated request. Falls back to the `/releases/latest` API JSON with a `grep`/`sed` extraction of `"tag_name"` if the redirect form fails.

**Upgrades.** Re-running the script installs over the existing binary — `install` replaces atomically via a rename, so a running `salat` keeps its open inode and is unaffected. The script prints `upgrading 0.1.0 → 0.2.0` when it detects an existing binary, and exits early with `already at 0.2.0` when the resolved tag matches (unless `--force`).

**Uninstall.** `install.sh --uninstall` removes `$PREFIX/salat`, then *lists* the data it found without deleting it — `~/.config/salat/`, `~/.local/share/salat/`, `~/.local/state/salat/`, and any systemd user unit — with the exact `rm -rf` command to remove each. Silently deleting a user's configuration is not the installer's decision to make. `--uninstall --purge` deletes them after an explicit confirmation.

### 21.3 Why static musl binaries

Two hard requirements make this non-negotiable: the binary must run on any glibc version (Debian oldstable through Arch), and the audio subsystem must not link ALSA or PulseAudio (§13.1). Consequences:

- Target `x86_64-unknown-linux-musl`. (`aarch64-unknown-linux-musl` is deferred past v0.1.0 — §28 D8; nothing in this section is x86_64-specific, so it returns as one more target.)
- TLS is **rustls**, never OpenSSL — no `libssl` to find at runtime.
- DNS is **hickory-dns**, because musl's resolver lacks features glibc's provides and does not read all the configuration users expect.
- SQLite is **bundled** (`libsqlite3-sys/bundled`), compiled into the binary.
- The city index is **embedded** via `include_bytes!`, so there is no data file to ship or lose.
- Result: one file, ~10–14 MB, ~4–5 MB gzipped, zero runtime dependencies beyond libc-free syscalls, an optional D-Bus session, and an optional audio player binary.

### 21.4 systemd (opt-in only)

`packaging/systemd/salat.service` ships inside the tarball and is documented, never installed automatically:

```ini
[Unit]
Description=Salat prayer time notification service
After=graphical-session.target

[Service]
Type=simple
ExecStart=%h/.local/bin/salat daemon
Restart=on-failure
RestartSec=10s
# hardening
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=%h/.config/salat %h/.local/share/salat %h/.local/state/salat
ProtectKernelTunables=true
ProtectControlGroups=true
RestrictNamespaces=true
MemoryMax=128M

[Install]
WantedBy=default.target
```

A **user** unit (`systemctl --user`), not a system unit — it needs the session D-Bus for notifications and the session audio device, both of which a system unit cannot reach without invasive configuration. `ProtectHome=read-only` plus three explicit `ReadWritePaths` is the whole filesystem surface Salat needs. Enabling it is two documented commands; Salat is fully functional without it, because the foreground TUI runs the same scheduler (§4.3).

**Known issue in v0.1.0 (verified 2026-11-25, unresolved).** The three `ReadWritePaths` are not the whole surface: the §4.3 scheduler lock lives in `$XDG_RUNTIME_DIR/salat/` (`/run/user/<uid>`), which `ProtectSystem=strict` makes read-only. Under the unit's sandbox, `salat daemon` fails with `E904 … Read-only file system (os error 30)` and exits 1, and `Restart=on-failure` restarts it every 10 s. The same sandbox with the runtime directory added to `ReadWritePaths` runs normally. `Restart=on-failure` also restarts after a configuration error (exit 3). Both need a unit change, which has not been made: v0.1.0 stays as published (see `docs/KNOWN_ISSUES.md`, which records the evidence and the proposed fixes as not implemented). Meanwhile `docs/INSTALL.md` and `docs/TROUBLESHOOTING.md` give users a `ReadWritePaths=%t` drop-in.

### 21.5 Security posture

| Surface | Control |
|---|---|
| Script transport | HTTPS only, `curl -fsSL` (`-f` so an HTML error page is never piped into `sh`) |
| Asset integrity | SHA-256 verified **before** extraction, against `SHA256SUMS` published as a release asset; abort if no checksum tool exists |
| Archive extraction | Member listing validated for absolute paths, `..`, symlinks, devices; extracted into a `mktemp -d` that a trap always removes |
| Privileges | Never root, never `sudo`, never setuid, never file capabilities; refuses root by default |
| API traffic | HTTPS + rustls with full certificate verification; redirects restricted to `api.aladhan.com`; 2 MiB response cap; 10 s timeout |
| Audio | `Command::new(player).args([...])` — argv only, never a shell string; the configured path is validated as an existing regular file, so no argument injection through the config |
| Notifications | Body text is built from local data only; any string of external origin is HTML-escaped, since the D-Bus spec permits limited markup |
| Config & data | `0600` files, `0700` directories, atomic same-directory `rename`, no world-readable temp files; coordinates logged at `debug!` only |
| Supply chain | `Cargo.lock` committed, `cargo deny check` and `cargo audit` in CI, CI container images pinned by digest (GitLab CI/CD, §22), and the release credential a GitHub token limited to this repository's contents (write) and nothing else, readable only by the manual publish job on protected tags (§22.2) |
| Privacy | No telemetry, no analytics, no crash reporting. `README.md` discloses plainly that coordinates, method and school are sent to `api.aladhan.com` and nothing else leaves the machine |

The one residual trust decision is the `curl | sh` pattern itself. It is mitigated, not eliminated: the script is short, readable, and served from the same repository the user is trusting for the binary; the README shows the two-step alternative (`curl -O`, read, `sh install.sh`) first for readers who prefer it; and every URL and digest the script uses is printed as it runs.

---

## 22. CI/CD and release architecture (GitLab CI/CD)

> **Provider.** CI/CD runs on **GitLab CI/CD** in the project `gitlab.com/linuxcoding-ADAM/salat`. **GitHub Actions is not used.** **GitHub (`github.com/linuxcoding-ADAM/Salat`) remains the canonical repository**: the source of truth for code, tags, issues, the installer's raw URL (§21.1), and the Releases the installer downloads from. Only the provider changed. The jobs, gates, tarball layout, asset names and release procedure below are the ones this section always specified, expressed for GitLab. The reasons and constraints are recorded in §28.

### 22.1 Check jobs — every push and merge request

`.gitlab-ci.yml`, check stage. The outline below lists the jobs; it is not literal GitLab syntax:

```yaml
workflow:          # pipelines run for branch pushes, merge requests and tags
  rules:
    - if: $CI_COMMIT_TAG
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
    - if: $CI_COMMIT_BRANCH
default:
  interruptible: true   # with auto-cancel, a newer pipeline on the same ref cancels this one
# jobs
#   check:         fmt --check · clippy --all-targets --all-features -D warnings · cargo test --all-features
#   build-musl:    cargo build --release --target x86_64-unknown-linux-musl (via cross, Docker-in-Docker service)
#   lint-shell:    shellcheck install.sh ci/*.sh
#   supply-chain:  cargo deny check advisories bans licenses sources
#   msrv:          cargo check with the rust-toolchain.toml pinned version
# then, on the build-musl artifact (nothing downstream recompiles):
#   package:                  verify the binary, build the tarball and SHA256SUMS
#   verify-installer-debian,
#   verify-installer-alpine:  the installer tests (manual on branches, automatic on tags)
```

GitLab's `cache:` (keyed on `Cargo.lock`) replaces `Swatinem/rust-cache`. As implemented it holds only the cargo registry and git downloads, not `target/`: after the `check` job `target/` measured 4.9 GB, which costs more to archive and restore on shared runners than recompiling saves. All five check jobs must pass.

**The merge gate.** GitLab cannot enforce required checks on GitHub branches, so the gate is a fixed procedure rather than a branch-protection setting. Every commit is pushed to the `gitlab` remote first. Only when its pipeline is green is the **same commit** pushed to `origin` (GitHub). The two remotes stay separate: `origin` is GitHub, `gitlab` is GitLab, and `origin` never carries a second push URL (§28 D2).

### 22.2 Release jobs — triggered by a version tag

Tag pipelines run for two tag forms, both validated against `Cargo.toml`:

| Tag form | Example | Pipeline runs | Publishes a GitHub Release |
|---|---|---|---|
| Stable `vMAJOR.MINOR.PATCH` | `v0.1.0` | every job below | only when the manual `publish` job is started by hand |
| SemVer prerelease `vMAJOR.MINOR.PATCH-PRE` | `v0.1.0-rc.1` | every job below except `publish` | **never**: the job is not created for prerelease tags |

```yaml
# jobs in tag pipelines (names as implemented in .gitlab-ci.yml)
#   verify-version:   assert Cargo.toml version == tag (fail fast; build-musl waits for it)
#   auth-check:       prerelease protected tags only, MANUAL: a read-only check that the release token works
#   token-isolation / release-guard / release-scripts-test:  keep the release token away from ordinary jobs (§28 D5)
#   check jobs:       the five §22.1 jobs, re-run
#   build-musl:       x86_64-unknown-linux-musl (aarch64 deferred — §28 D8)
#   package:          verify the binary, build the tarball and SHA256SUMS
#   verify-installer-debian / -alpine:  the installer tests of Phase 14 against this pipeline's own artifacts
#   publish:          MANUAL, stable protected tags only, after every validation job: create the GitHub Release
```

The original plan named these jobs `verify`, `build`, `checksums` and `verify-artifacts`; §28 D4 still uses the name `verify`.

**`verify-version`** exists because the single most common release mistake is a tag that disagrees with `Cargo.toml`, producing a `v0.2.0` release whose binary reports `0.1.0`. It reads the version with `cargo metadata` and requires `${CI_COMMIT_TAG#v}` to equal it **exactly**. It accepts any version Cargo accepts, including a SemVer prerelease suffix, so `v0.1.0-rc.1` passes only when `Cargo.toml` says `0.1.0-rc.1`. It fails in ten seconds rather than after the cross-compile.

**`build-musl`** builds the release binary with `cross` (musl cross-linking without hand-managed sysroots), stripped by `[profile.release] strip = true`. `package` (below) then assembles:

```
salat-<version>-<target>.tar.gz
  └── salat-<version>-<target>/
        ├── salat                (mode 0755)
        ├── README.md
        ├── LICENSE
        ├── CHANGELOG.md
        └── salat.service
```

The tarball has one top-level directory (no tarbombs), and `install.sh` reads `*/salat` from it.

**`package`** takes the binary `build-musl` produced (as an artifact, never rebuilt), verifies it (architecture, static linking, stripped, `--version`, smoke tests), assembles the tarball above, and writes `SHA256SUMS` in `sha256sum -c` format. The tarball and `SHA256SUMS` are kept as pipeline artifacts.

**`verify-installer-debian`** and **`verify-installer-alpine`** run Phase 14's installer tests (clean containers, tampered checksum, malicious tar member) against those pipeline artifacts. A prerelease tag therefore exercises build, test, packaging, checksums and verification end to end without publishing anything (§28 D4).

**`auth-check`** exists only for prerelease tags on protected refs and is `when: manual`. When a maintainer presses Run it makes one read-only GitHub API call with the release token and requires it to be accepted with write access to the repository, so a broken credential is found before a stable release, without publishing anything. It needs the token because testing the credential is its purpose; what changed is that it no longer receives it automatically.

**`publish`**: exists only in pipelines for stable tags matching `^v[0-9]+\.[0-9]+\.[0-9]+$`, runs only on protected tags, and is `when: manual`. Publication is always a deliberate human action, never a side effect of pushing a tag. It runs `gh release create` against the canonical GitHub repository. It uploads the x86_64 tarball plus `SHA256SUMS` (one tarball while aarch64 is deferred — §28 D8), uses release notes extracted from the `CHANGELOG.md` section matching the tag, and passes `--verify-tag`, so the tag must already exist on GitHub. It `needs` every validation job (`verify-version`, `check`, `msrv`, `supply-chain`, `lint-shell`, `token-isolation`, `release-scripts-test`, `release-guard`, `package` and both installer jobs), so it cannot be started while any of them has failed (a manual job whose needed job failed is `Unplayable`). It asks for confirmation, belongs to the `release` resource group so two releases cannot run at once, and before creating anything checks, read-only, that the token has push access, that the tag exists on GitHub and points at the commit this pipeline verified, and that no release for the tag exists. Its only credential is a dedicated, least-privilege GitHub token (§28 D5), received only after the manual start and never printed.

Asset naming, fixed and parseable by the installer:

```
salat-0.1.0-x86_64-unknown-linux-musl.tar.gz
SHA256SUMS
```

The naming scheme already covers other targets: when aarch64 returns (§28 D8) it adds `salat-<version>-aarch64-unknown-linux-musl.tar.gz` and one more `SHA256SUMS` line, and nothing else changes.

### 22.3 Versioning and the release procedure

SemVer, `0.y.z` until the config schema is stable. Pre-1.0, a breaking config change bumps the minor and must ship a migration or a documented default.

1. Update `CHANGELOG.md` (Keep a Changelog format) under a new `## [0.2.0] - YYYY-MM-DD` heading.
2. Bump `version` in `salat/Cargo.toml`; `cargo check` to refresh the lockfile.
3. Commit `chore(release): 0.2.0`, push to `gitlab`, wait for a green pipeline, then push the same commit to `origin` (§22.1).
4. `git tag -a v0.2.0 -m 'salat 0.2.0' && git push gitlab v0.2.0`.
5. The tag pipeline verifies the version, builds the x86_64 target (§28 D8), packages the tarball and `SHA256SUMS`, and runs the installer tests on them. When it is green, `git push origin v0.2.0`, then start the manual `publish` job, which creates the GitHub Release.
6. Verify the published artifacts by running the real install path on a clean x86_64 container (§28 D8).

A prerelease (`v0.1.0-rc.1`) follows steps 1–5 up to the green tag pipeline and stops. It produces no GitHub Release.

**End-to-end path, stated once:** tag `v0.2.0` → pushed to `gitlab` → `verify-version` confirms `Cargo.toml` says `0.2.0` → a `cross` build produces the static x86_64 musl binary → it is tarred with docs and the unit file → its digest is written to `SHA256SUMS` → the artifacts pass the installer tests → the tag is pushed to `origin` → the manual `publish` job runs `gh release create v0.2.0` and uploads two assets to GitHub Releases → `install.sh` resolves `/releases/latest` to `v0.2.0`, downloads the tarball for the detected arch plus `SHA256SUMS`, verifies, extracts, installs to `~/.local/bin/salat`, smoke-tests `--version` → the user runs `salat`.

### 22.4 Developer setup

```bash
git clone https://github.com/linuxcoding-ADAM/Salat && cd Salat
cargo run              # quick mode
cargo run -- --full    # full TUI
cargo test
cargo fmt && cargo clippy --all-targets -- -D warnings
cargo run -p xtask -- gen-cities   # regenerate assets/cities.bin
cargo build --release
```

`rust-toolchain.toml` pins the channel so contributors and CI agree. `INSTA_UPDATE=always cargo test` updates snapshots; the diff is reviewed like code. `docs/screenshots/` is regenerated by `xtask screenshots`, which drives the TUI at each breakpoint under a pseudo-terminal and captures the output — keeping the README's images honest as the UI changes.

---

## 23. Phased implementation plan

Sixteen phases. Every phase leaves the repository buildable, testable and — from Phase 5 onward — runnable with visible value. No phase depends on more than two unfinished systems; each is defined against interfaces that already exist when it starts.

Each phase specifies the twelve required items: **goal · scope · files · interfaces · data models · dependencies · implementation order · automated tests · manual tests · commands · expected results · definition of done.**

---

### Phase 0 — Foundation and CLI skeleton

1. **Goal.** A real repository that builds, lints, tests and responds to `--help`/`--version`, with the module tree in place as empty modules so no later phase has to restructure.
2. **Scope.** Workspace, toolchain pinning, lint config, the full `src/` module tree with `//!` docs and no logic, `Cli` parsing, dispatch, exit codes, logging, `README` skeleton, CI. **Out of scope:** every service body.
3. **Files.** `Cargo.toml` (workspace), `salat/Cargo.toml`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `deny.toml`, `.gitignore`, `LICENSE` ×2, `README.md`, `CHANGELOG.md`, `src/main.rs`, `src/lib.rs`, `src/error.rs`, `src/cli/mod.rs`, `src/cli/commands/{quick,full,status,config,test_adhan,daemon}.rs`, every other `mod.rs` from §19 as a stub, `.github/workflows/ci.yml`.
4. **Interfaces.** `Cli`/`Command` as §14.2. `fn dispatch(cli: Cli) -> Result<ExitCode, SalatError>`. Each command function is `todo!()`-free — it returns `Err(SalatError::NotImplemented)` so the binary never panics.
5. **Data models.** `SalatError` with its `code/title/details/help/actions` methods and the §18.2 variant list (bodies may be minimal but the codes are final). `ExitCode` mapping: 0 ok, 1 runtime, 2 usage, 3 config, 4 network.
6. **Dependencies.** `clap`, `thiserror`, `color-eyre`, `tracing`, `tracing-subscriber`, `directories`, `is-terminal`. Nothing async yet.
7. **Order.** workspace → lint config → `error.rs` → `cli/mod.rs` → `main.rs` → stub command files → module tree stubs → logging init → CI workflow.
8. **Automated tests.** `tests/cli.rs`: `--help` contains `--full`, `--once`, `config`, `status`, `test-adhan`; `--version` equals `CARGO_PKG_VERSION`; an unknown flag exits 2; `-v -v` sets the expected filter. Unit: every `SalatError` variant returns a unique, correctly formatted code.
9. **Manual tests.** `cargo run -- --help` reads well; `cargo run` prints the not-implemented message on stderr and exits non-zero; `cargo run | cat` emits no ANSI.
10. **Commands.** `cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and — before the first of those — **`cargo add --dry-run <crate> --features <list>` for every crate in §5.1 and §5.2**, one invocation per crate, comparing the resolved version and feature set it prints against the table. This is a gate item, not advice: §5.1's feature names were read from crates.io during planning, and a feature renamed across a minor version is the single most common cause of a first-build failure. Any mismatch is recorded in the PR and §5.1 is corrected in the same commit — the pinned versions are the plan's, so a divergence is a documentation bug to fix, never a licence to change the stack.
11. **Expected results.** All four commands succeed, and `--dry-run` resolves every crate at the pinned version with every listed feature accepted and no feature warnings. `--help` output is final-quality. No `unwrap`, no `todo!`, no warnings.
12. **DoD.** CI green on a real PR; help text reviewed against §14; the module tree matches §19 file-for-file; the `cargo add --dry-run` sweep is complete and §5.1 reflects what it resolved.

---

### Phase 1 — Configuration

1. **Goal.** Load, validate, create, atomically save and hot-propagate `config.toml` — the substrate every later phase configures itself from.
2. **Scope.** The full §8.2 schema, §8.3 defaults, §8.4 validation, atomic save, `watch` channel, `ConfigDraft`, and `salat config` becoming real.
3. **Files.** `config/{mod,defaults,store,validate,draft}.rs`, `cli/commands/config.rs`, `tests/config_roundtrip.rs`, `docs/CONFIGURATION.md`.
4. **Interfaces.** `ConfigStore::load() -> Result<Arc<Config>, ConfigError>`, `::save(&Config)`, `::subscribe() -> watch::Receiver<Arc<Config>>`, `::path() -> &Path`, `Config::validate() -> Result<(), Vec<Violation>>`, `ConfigDraft::{from, apply_to, is_dirty}`.
5. **Data models.** Every struct and enum in §8.2 with `Serialize`/`Deserialize`/`deny_unknown_fields`, `PrayerSettings`, `Violation { field, message }`.
6. **Dependencies.** `serde`, `toml`, `tokio` (`sync`, `fs`), `tempfile` (dev).
7. **Order.** structs → defaults → deserialize test → validation → load-or-create → atomic save → watch → draft → `salat config`.
8. **Automated tests.** round-trip equality; missing file creates the exact §8.3 defaults; unknown key → error naming the key; **all** violations reported together; `tune` of wrong length rejected; `volume > 100` rejected; `before_minutes > 60` rejected; atomic save leaves no `.tmp`; a read-only directory yields `E202` and leaves the old file untouched; file mode is `0600`.
9. **Manual tests.** Delete the config, run `salat config`, inspect the generated TOML for readability and correct comments; hand-corrupt it and confirm the error names the line.
10. **Commands.** `cargo test config`, `cargo run -- config`, `cargo run -- config --edit`.
11. **Expected results.** The generated file is human-readable, fully commented, and matches `docs/CONFIGURATION.md` exactly.
12. **DoD.** Every §8.2 key is loadable, validated, defaulted and documented; save is atomic and verified by test; `watch` delivers a change to a subscriber in a test.

---

### Phase 2 — AlAdhan client

1. **Goal.** A tested HTTP client for the three endpoints Salat uses, with the full failure matrix handled and every §7 quirk defended against.
2. **Scope.** Request construction, client-side parameter validation, tolerant deserialization, retry/backoff, timeouts, concurrency limit, ETag support, method whitelist.
3. **Files.** `api/{mod,dto,params,map,retry}.rs`, `tests/api_failures.rs`, `tests/fixtures/aladhan/*.json`.
4. **Interfaces.** `#[async_trait] trait AlAdhanClient { async fn calendar(&self, year:i32, month:u32, p:&Params) -> Result<Vec<PrayerDayDto>, ApiError>; async fn timings(&self, date:NaiveDate, p:&Params) -> ...; async fn qibla(&self, lat:f64, lng:f64) -> Result<f64, ApiError>; }` plus `HttpClient` and `MockClient`.
5. **Data models.** `Params`, `ApiEnvelope<T>`, `CalendarDayDto`, `TimingsDto`, `MetaDto`, `HijriDto`, `ApiError` (`Network`, `Timeout`, `Status`, `RateLimited`, `Malformed`, `Upstream(String)`, `InvalidParams`).
6. **Dependencies.** `reqwest` (rustls, gzip, hickory), `serde_json`, `chrono`, `async-trait`, `wiremock` (dev). Depends on Phase 1 only for `CalculationConfig`.
7. **Order.** `Params` + validation → DTOs against committed fixtures → mapping → the client → retry → ETag → the failure tests.
8. **Automated tests.** The full §20.2 API row: 200, `data`-as-string 400, latitude-999 503 (**and** the client rejecting it before any request), method-777 rejected client-side, malformed JSON, timeout, connection refused, 304, 429 → three retries → `E304`, backoff pattern 1s/2s/4s asserted from mock timestamps, `iso8601=true` present on every request, `tune` serialized as nine comma-separated integers.
9. **Manual tests.** One live `curl` per endpoint compared against the committed fixtures to confirm the schema has not drifted (documented in `CONTRIBUTING.md` as a monthly check).
10. **Commands.** `cargo test api`, `cargo test --test api_failures`.
11. **Expected results.** All failure modes produce typed errors, never panics; no test performs real network I/O.
12. **DoD.** Every §7.5 row implemented and tested; the method whitelist prevents the silent-wrong-times hazard; fixtures committed with a note recording when they were captured.

---

### Phase 3 — Prayer domain and Clock

1. **Goal.** All time logic, pure and exhaustively tested, with no I/O anywhere in the module.
2. **Scope.** `Prayer`, `Notifiable`, `PrayerDay`, `PrayerWindow`, `PrayerInstant`, `PrayerState` + `state_of`, `Countdown`, `plan_events`, `DaySummary`, `Clock`.
3. **Files.** `domain/{mod,prayer,day,state,countdown,schedule,summary}.rs`, `app/clock.rs`.
4. **Interfaces.** `next_prayer(&PrayerWindow, DateTime<Tz>) -> Option<PrayerInstant>`, `state_of(&PrayerWindow, Notifiable, &Config, DateTime<Tz>, &[Notifiable]) -> PrayerState`, `Countdown::between`, `progress(prev, next, now) -> f64`, `plan_events(&PrayerWindow, &Config, DateTime<Tz>) -> Vec<ScheduledEvent>`, `trait Clock`.
5. **Data models.** Exactly §6.1–§6.6 plus `ScheduledEvent { kind, prayer, date, fire_at }` and `EventKind { PreNotify, Adhan, MosqueReminder, DayRollover }`.
6. **Dependencies.** `chrono`, `chrono-tz`. Phase 1 for `Config`. **No** async, no reqwest, no sqlx — enforced by keeping `domain` free of those imports.
7. **Order.** enums → `PrayerDay`/`PrayerWindow` → `next_prayer` → `Countdown`/`progress` → `state_of` → `plan_events` → `DaySummary` → `Clock` + clippy `disallowed_methods`.
8. **Automated tests.** The five domain rows of §20.2 in full: every `next_prayer` case including exact-boundary and after-Isha; countdown boundaries; all eight state transitions; `before = 0` and `after = 0` degenerate cases; precedence (`Completed` > `Adhan` > `PreAdhan` > `Next` > `Passed`); DST in `Europe/Berlin` on the real transition dates; `plan_events` emitting exactly the expected set for the default config; disabled prayers producing no events; a rollover event at local midnight.
9. **Manual tests.** None — this phase has no user surface. (Its correctness is the reason the tests are exhaustive.)
10. **Commands.** `cargo test domain`, `cargo clippy -- -D warnings` (verifying `Utc::now` is rejected).
11. **Expected results.** ≥40 domain tests, all passing in well under a second. Any use of a system clock outside `SystemClock` fails clippy.
12. **DoD.** The state machine table of §10.1 is reproduced by tests row for row; `MockClock` drives every time-dependent assertion; no `domain` function performs I/O.

---

### Phase 4 — Cache and persistence

1. **Goal.** The SQLite layer: migrations, the five tables, and the "network or cache" resolution that makes offline operation correct.
2. **Scope.** Pool setup, migrations, corruption recovery, `prayer_days`, `fired_events`, `event_log`, `prayer_completions`, `qibla`, `cache_key`, `vacuum`, and `PrayerTimeService::ensure_window`.
3. **Files.** `cache/{mod,prayer_days,fired,events,completions,qibla}.rs`, `migrations/0001_init.sql`, `src/service/prayer_times.rs`, `tests/cache.rs`.
4. **Interfaces.** `Store::open(&Path) -> Result<Store, CacheError>`, `::upsert_month`, `::load_day`, `::try_mark_fired -> Result<bool>`, `::append_event`, `::events_today`, `::toggle_completion`, `::completions_for`, `::vacuum`; `PrayerTimeService::window(NaiveDate) -> Result<Arc<PrayerWindow>, ServiceError>` and `::refresh()`.
5. **Data models.** The §17.2 schema and `cache_key` as the readable canonical string of §17.2.
6. **Dependencies.** `sqlx` (sqlite, bundled, runtime-tokio, migrate, chrono). Builds on Phases 1–3.
7. **Order.** pool + PRAGMAs → migration → `prayer_days` round-trip → `cache_key` → `ensure_window` (cache first, then network) → `fired_events` → `event_log` → completions → qibla → vacuum → corruption recovery.
8. **Automated tests.** In-memory SQLite throughout: store then load a month; a config change changes the key and misses; `try_mark_fired` returns `true` once then `false`; the event log orders newest-first and filters to today; completions survive reopening; `vacuum` removes only what it should; a deliberately corrupted file is renamed and recreated; `ensure_window` with a warm cache performs **zero** HTTP calls (asserted against a `MockClient` call counter).
9. **Manual tests.** Run twice with the network disabled after the first success and confirm the second run needs no network; inspect the DB with `sqlite3` and confirm `cache_key` is readable.
10. **Commands.** `cargo test cache`, `cargo test --test cache`, `sqlite3 ~/.local/share/salat/salat.db '.schema'`.
11. **Expected results.** Second launch is instant and offline-capable; DB under 200 KB after a year of use.
12. **DoD.** Migrations run automatically at open; all five tables tested; the cache's purpose and limits documented in `docs/CONFIGURATION.md`; corruption never crashes.

---

### Phase 5 — Quick mode, theme and widget foundation

1. **Goal.** `salat` — the default command — becomes genuinely useful: today's times, now, next prayer, live countdown, in the design's visual language.
2. **Scope.** `Theme` with all 27 named values (§3.2) and three degradation modes, the first eight widgets, `Breakpoint`, `UiModel` + `build_model`, and the inline-viewport quick renderer with both live and `--once` output.
3. **Files.** `ui/{theme,layout,model}.rs`, `ui/widgets/{panel,bar,badge,prayer_table,bigtext,kbd_hint,hr,status_dot}.rs`, `cli/commands/quick.rs`, `app/{runtime,signals}.rs`, `tests/snapshots/quick_*.snap`, `tests/golden/quick_once_{80x24,100x30,120x35}.txt`, `tests/golden/quick_once_80x24_color.txt`, `tests/golden/quick_once_oneline.txt`.
4. **Interfaces.** `Theme::from(&DisplayConfig)`, `Breakpoint::of(Rect)`, `build_model(&Runtime) -> UiModel`, `render_quick(&UiModel, Rect, &mut Buffer)`, `Runtime::build(&Cli) -> Result<Runtime, SalatError>`.
5. **Data models.** `Theme`, `ColorMode`, `Breakpoint`, `UiModel` (§15.1).
6. **Dependencies.** `ratatui`, `crossterm` (event-stream), `unicode-width`, `insta` (dev). Builds on Phases 1–4.
7. **Order.** theme → color degradation → `panel`/`hr`/`badge` → `prayer_table` → `bar` → `bigtext` → `build_model` → inline viewport → `--once` → terminal guard + panic hook → snapshots.
8. **Automated tests.** `TestBackend` snapshots of the quick view at 80×24, 100×30, 120×35 from a fixed `MockClock` and the design's times; every line fits its width (mechanical assertion); `--once` output contains no ANSI when not a TTY. Then the four groups this phase's specifications each require:
   - **Goldens (§14.6).** All five golden files compared byte for byte; the 80×24 one is the §14.6 specimen verbatim. A diff fails; regeneration is only ever `UPDATE_GOLDEN=1`.
   - **Palette (§15.6).** The eight-pair distinctness list asserted in both degraded modes, plus the ten neutral ramp indices strictly increasing in luminance order. The two deliberate collapses (`greenBg`, `yellowBg` → `panelBg`) are asserted *as* collapses, so a later "fix" that separates them fails a test and has to be argued for.
   - **Bigtext (§15.5).** `width(s) == 4 * n - 1` over the whole 38-glyph set; `"01:42:37"` is 31 columns and `"MAGHRIB"` is 27; an unmapped character renders the space glyph and warns rather than panicking; too narrow yields no bigtext at all rather than a truncated row.
   - **Quick-mode ladder (§16.14).** The row budget `v = min(14, h - 1)` at `h` = 24, 15, 14, 12, 10 and 9; each rung drops exactly the specified element; the prayer-row drop order is `argmax |i - n|` with ties to the lower index; `h < 10` or `w < 80` opens no viewport and prints the §14.6 summary; `Breakpoint::Small` is pinned without calling `Breakpoint::of`.
9. **Manual tests.** `salat` in Alacritty, GNOME Terminal, Kitty and `tmux`; `Ctrl-C` mid-countdown leaves the terminal clean and the frame in scrollback; `TERM=xterm`; `salat > out.txt`; resize during the live loop.
10. **Commands.** `cargo run`, `cargo run -- --once`, `cargo run -- --once | cat`, `cargo test ui::`, `cargo test golden`, `TERM=xterm cargo run`, and `stty rows 12 && cargo run` plus `stty rows 8` to walk the degradation ladder by hand.
11. **Expected results.** The countdown ticks each second; the frame matches the design's small-size dashboard; exit is always clean.
12. **DoD.** Quick mode is the finished feature for the default invocation; three snapshots and five goldens committed; every rung of the §16.14 ladder exercised; terminal restoration verified after `Ctrl-C`, a panic, and normal exit.

---

### Phase 6 — TUI shell

1. **Goal.** The full-screen application frame: event loop, navigation, header/footer chrome, the boot sequence, and the too-small guard — with placeholder screen bodies.
2. **Scope.** `Shell`, the nav stack, the overlay slot, `input.rs` resolution order, the `select!` loop, `Viewport::Fullscreen`, `boot.rs`'s five stages, `chrome.rs`, `too_small.rs`. Screen bodies render only their title.
3. **Files.** `ui/{mod,shell,input}.rs`, `ui/widgets/{chrome,too_small,crumb}.rs`, `ui/screens/{loading,states}.rs` + ten title-only stubs, `app/boot.rs`, `cli/commands/full.rs`.
4. **Interfaces.** `Shell::new`, `::handle(Event, &mut Runtime) -> Result<bool>`, `::push/pop/replace`, `::overlay`, `ui::render(&mut Frame, &UiModel, &Shell)`, `boot::run(&mut Runtime, impl FnMut(BootState))`.
5. **Data models.** `Screen`, `Overlay`, `Shell` with its nine state structs (§15.1), `BootState`.
6. **Dependencies.** Phase 5's theme and widgets. `tokio` `signal`.
7. **Order.** terminal guard → event loop → `Shell` + stack → `input.rs` → chrome → too-small guard → boot state machine → the LOADING screen → the STATES screen (it is static, so it proves the pipeline end-to-end).
8. **Automated tests.** `1`–`4` set the expected screen; `Esc` pops and never empties the stack; `?` pushes STATES and `Esc` returns; an overlay swallows `1`; `KeyEventKind::Release` is ignored; snapshots of LOADING at all five stages and of the too-small guard at 79×23 and 40×10.
9. **Manual tests.** Resize continuously below and above 80×24; the guard appears and disappears live; the boot sequence is visible and its steps tick; `Ctrl-C` during boot restores the terminal.
10. **Commands.** `cargo run -- --full`, `cargo test ui::shell`, `cargo test ui::input`.
11. **Expected results.** All twelve screens are reachable by their documented keys; the header and footer match the design; no flicker at 250 ms.
12. **DoD.** Navigation matches §15.4 exactly; the boot sequence's five stages each report real progress; nothing renders below 80×24 except the guard.

---

### Phase 7 — Dashboard

1. **Goal.** The primary screen, pixel-faithful to the design at all three breakpoints.
2. **Scope.** NEXT PRAYER, TODAY, TONIGHT, the greeting line, the status strip, the completion cursor, and the qibla value.
3. **Files.** `ui/screens/dashboard.rs`, `ui/widgets/timeline.rs`, `tests/snapshots/dashboard_{80x24,100x30,120x35}.snap`.
4. **Interfaces.** `dashboard::render(&UiModel, &DashboardState, Rect, &mut Buffer)`, `dashboard::handle(KeyEvent, &mut DashboardState, &mut Runtime) -> Result<bool>`.
5. **Data models.** `DashboardState { cursor }`; reuses `UiModel` unchanged.
6. **Dependencies.** Phases 3–6. The qibla value comes from Phase 4's cache.
7. **Order.** 80×24 layout and row-budget verification → 100×30 two-column → 120×35 with greeting and TONIGHT → the `c` completion toggle → the status strip → snapshots.
8. **Automated tests.** Three snapshots; a snapshot in each `PrayerState` (Next, PreAdhan, Adhan, Passed, Completed) verifying glyph and color; `c` toggles and persists via Phase 4; the 80×24 table has three columns and the ≥100 table has four; every line fits.
9. **Manual tests.** Compare each breakpoint side by side with the Figma render; toggle completion and restart to confirm persistence; watch a real prayer transition (or shift `MockClock` via a debug flag) and confirm badge and color changes.
10. **Commands.** `cargo run -- --full`, `cargo test dashboard`, `INSTA_FORCE_PASS=0 cargo insta review`.
11. **Expected results.** Visually indistinguishable from the design within terminal constraints; 20 body rows at 80×24 with 2 to spare.
12. **DoD.** Three snapshots committed and reviewed against Figma; all five states render distinctly **without color** (verified in the ANSI snapshot); completion round-trips through SQLite.

---

### Phase 8 — Scheduler

1. **Goal.** Events fire at the right wall-clock moment, exactly once, surviving redraws, resizes, config changes, day changes, suspend/resume and restarts — independent of the render loop.
2. **Scope.** The runner, the 1 Hz reconcile tick, monotonic-vs-wall jump detection, the `fired_events` ledger, catch-up with grace windows, replanning on config change and rollover, `SIGHUP`, the lock file and observer mode, and event-log writes. Handlers are the recording stubs from Phase 3's tests.
3. **Files.** `scheduler/{mod,runner,catchup}.rs`, `app/lock.rs`, `tests/scheduler.rs`.
4. **Interfaces.** `Scheduler::spawn(deps) -> SchedulerHandle`, `Handle::{replan, shutdown, subscribe}`, `trait EventSink { async fn on_event(&self, ScheduledEvent, late: bool); }`, `catchup::decide(&ScheduledEvent, now, &Store) -> CatchupDecision`.
5. **Data models.** `ScheduledEvent` (Phase 3), `CatchupDecision { Fire{late}, Skip, Pending }`, `LockFile { pid, started_at }`.
6. **Dependencies.** Phases 1, 3, 4. `tokio` `time`/`sync`/`signal`.
7. **Order.** plan → the tick loop → the ledger → dedup → grace/catch-up → jump detection → config-change replan → rollover replan → `SIGHUP` → the lock file → observer mode.
8. **Automated tests.** The full §20.2 scheduler row with `MockClock`: fires once; replayed ticks fire nothing; a 6-hour forward jump fires only in-grace events and marks the rest missed; a backward jump does not re-fire; a config change re-arms; day rollover refetches and replans; the ledger survives a simulated restart; two schedulers cannot both hold the lock.
9. **Manual tests.** Set a prayer's `before_minutes` so an event is 2 minutes out and watch it fire while sitting on the SETTINGS screen (proving independence from the render loop); `systemctl suspend` across an event and verify the §11.5 decision; start the app 3 minutes after a missed event and confirm one `(delayed)` fire; start 40 minutes after and confirm silence with a `Missed` log row.
10. **Commands.** `cargo test scheduler`, `cargo test --test scheduler`, `cargo run -- --full` with a near-term event.
11. **Expected results.** Zero duplicate fires across any test or manual scenario; suspend/resume behaves per §11.5.
12. **DoD.** Every §11 guarantee has a test; the ledger is the sole idempotency mechanism; the scheduler runs identically in the TUI and in `salat daemon`.

---

### Phase 9 — Notifications

1. **Goal.** Real desktop notifications at the configured offsets, degrading cleanly with no D-Bus.
2. **Scope.** `notify-rust` in `spawn_blocking`, the capability probe, per-prayer gating, body composition, urgency and timeout, the event-log entry, and the Status component report.
3. **Files.** `notify/{mod,desktop}.rs`, `tests/notifications.rs`.
4. **Interfaces.** `#[async_trait] trait Notifier { async fn send(&self, Notification) -> Result<(), NotifyError>; fn report(&self) -> Component; }`; `DesktopNotifier`, `RecordingNotifier` (test), `NoopNotifier`.
5. **Data models.** `Notification { summary, body, urgency, timeout, category }`.
6. **Dependencies.** `notify-rust` (default features off, `d`). Phases 1, 4, 8.
7. **Order.** the probe → `send` → body composition per event kind → gating on `notifications.enabled` and `prayers.<p>.notify` → wire into the scheduler sink → event log → Status report → graceful no-D-Bus path.
8. **Automated tests.** `RecordingNotifier` asserts: a 15-minute `before` on a 19:21 prayer sends at 19:06; `notify = false` sends nothing; `notifications.enabled = false` silences all five; a delayed fire's body starts with `(delayed)`; a `NotifyError` is logged and does **not** abort the scheduler; bodies contain no unescaped markup.
9. **Manual tests.** All five prayer notifications on GNOME and KDE; with the notification daemon stopped (expect a `Degraded` Status row, no crash); over SSH with no `$DBUS_SESSION_BUS_ADDRESS`; verify the notification text matches the design's wording.
10. **Commands.** `cargo test notify`, `cargo run -- --full` with a 1-minute offset, `systemctl --user stop <daemon> && cargo run -- --full`.
11. **Expected results.** Notifications appear at the exact minute; absence of D-Bus is reported, never fatal.
12. **DoD.** Timing verified by test and by hand; the no-D-Bus path exercised manually; Status shows the notification component's real health.

---

### Phase 10 — Adhan audio and the alert overlays

1. **Goal.** Audio playback and the two overlays the design specifies, plus `salat test-adhan`.
2. **Scope.** The player probe chain, playback and stop, volume, file/preset resolution, `test-adhan`, the ADHAN ALERT overlay, the MOSQUE REMINDER overlay, snooze, and the Adhan/PostAdhan visual states end to end.
3. **Files.** `audio/{mod,players,file}.rs`, `cli/commands/test_adhan.rs`, `ui/screens/{adhan_alert,mosque_reminder}.rs`, `tests/audio.rs`, `tests/snapshots/overlay_inline_*.snap`.
4. **Interfaces.** `#[async_trait] trait AudioPlayer { async fn play(&self, &Path, u8) -> Result<PlayHandle, AudioError>; fn probe(&self) -> Option<PlayerKind>; fn report(&self) -> Component; }`, `PlayHandle::{stop, is_running}`.
5. **Data models.** `PlayerKind { Mpv, Paplay, Ffplay, Aplay }`, `AudioError`, `Overlay::{AdhanAlert, MosqueReminder}`.
6. **Dependencies.** `tokio` `process`. Phases 6, 8, 9.
7. **Order.** probe → argv per player → play/stop → volume → file resolution and validation → `test-adhan` → the alert overlay → the reminder overlay → snooze → wire the scheduler's `Adhan`/`MosqueReminder` events to both audio and overlay.
8. **Automated tests.** argv construction per player asserted exactly (including `--volume=` scaling and `paplay`'s ×655); an absent player yields `E401` with a helpful message; a missing file yields `E402`; a non-regular-file path is rejected; `adhan = false` plays nothing but **still** shows the overlay; snapshots of both overlays at all three full-screen sizes **and in the inline viewport** (§16.15) — Adhan at `v` = 14, 11 and 3, Mosque at `v` = 14, 9 and 6 — asserting that each clears the whole viewport including the header, that `top_pad = (v - block) / 2` puts the remainder below, and that below each floor a single truncated line is printed; the overlay swallows `1`–`4` and `q` but not `Ctrl-C`; `Esc` dismisses the overlay and a *second* `Esc` quits; `Enter` and `s` stop audio while `Esc` does not (§13.6); the Arabic line is dropped rather than wrapped at 80 columns.
9. **Manual tests.** `salat test-adhan` with each of the four players as the only one on `PATH` and with none; playback stops on `Enter`; snooze re-fires after 3 minutes; `Ctrl-C` during playback kills the child process (verified with `ps`); the overlay appears at the exact prayer minute.
10. **Commands.** `cargo run -- test-adhan`, `PATH=/usr/bin cargo run -- test-adhan`, `cargo test audio`, `cargo test overlay`.
11. **Expected results.** Audio plays on any of the four players; no player is a clear, non-fatal message; no orphaned child processes ever.
12. **DoD.** All four players tested by hand; child cleanup verified on every exit path; both overlays snapshot-tested full-screen **and inline**; the late-fire rule of §11.5 covered — a catch-up fire raises no overlay, a live late fire does and carries `(delayed)`; the visual alert is proven independent of playback success.

---

### Phase 11 — Settings, Location, Notification Settings, Prayer Configuration

1. **Goal.** Every value the design exposes is editable in the TUI, saved atomically, and propagated to the running scheduler without a restart.
2. **Scope.** Four screens, the draft/save/cancel flow, the CONFIRM screen, `stepper` and `setting_row`, city search, coordinate entry, auto-detect, and the propagation path from save to replan.
3. **Files.** `ui/screens/{settings,notif_settings,prayer_config,location,confirm}.rs`, `ui/widgets/{stepper,setting_row}.rs`, `location/{mod,cities,system_tz}.rs`, `xtask/src/main.rs` (`gen-cities`), `assets/cities.bin`, `tests/location.rs`, `tests/settings_flow.rs`.
4. **Interfaces.** `CityIndex::{load, search, nearest}`, `system_timezone() -> Result<Tz, LocationError>`, per-screen `render`/`handle` pairs, `Runtime::save_draft(&ConfigDraft) -> Result<SaveOutcome, SalatError>`.
5. **Data models.** `CityMatch { name, country, cc, lat, lng, tz, population }`, `LocationState`, `SettingsState`, `NotifState`, `PrayerConfigState`, `SaveOutcome { recalculated: bool }`.
6. **Dependencies.** Phases 1, 4, 6, 8. `include_bytes!` for the index.
7. **Order.** `xtask gen-cities` → the index parser → search → `system_tz` → the LOCATION screen → `setting_row`/`stepper` → SETTINGS → NOTIF SETTINGS → PRAYER CONFIG → save → CONFIRM → propagation → the 80×24 scroll region for SETTINGS.
8. **Automated tests.** `Setif` finds `Sétif`; coordinate validation rejects 999 before any request; nearest-city zone derivation; >200 km falls back to the system zone; each screen's `←→` clamps at 0 and 60; `Tab` cycles columns correctly; save produces CONFIRM and a changed config on disk; a location change changes `cache_key` and triggers a refetch; a save failure shows `E202` and preserves the draft; `Esc` discards; snapshots of all four screens at three sizes; the SETTINGS scroll region keeps the cursor visible at 80×24.
9. **Manual tests.** Change the location by search, by coordinates and by auto-detect, and confirm prayer times actually change; change a `before_minutes` while an event is 3 minutes out and confirm the scheduler re-arms without a restart; verify the CONFIRM screen shows the real config path; edit the TOML by hand while the app runs and confirm the `watch` picks it up.
10. **Commands.** `cargo run -p xtask -- gen-cities`, `cargo test location`, `cargo test settings`, `cargo run -- --full`.
11. **Expected results.** Every design-visible setting is editable; `cities.bin` is ≈1.5–2 MB; search feels instant.
12. **DoD.** All four screens match the design at three sizes; save → propagate → replan proven by test; `cities.bin` regeneration documented and its GeoNames attribution present in `LICENSE-THIRD-PARTY`.

---

### Phase 12 — Status screen, `salat status`, and the Error screen

1. **Goal.** The system explains itself: ten component healths, today's counters, the next event, today's event log — in the TUI and on stdout — and every failure renders in the design's error idiom.
2. **Scope.** `status::Collector`, all ten `report()` implementations, the STATUS screen, the `salat status` formatter, and the ERROR screen driven by `SalatError`.
3. **Files.** `status/{mod,model}.rs`, `ui/screens/{status,error}.rs`, `cli/commands/status.rs`, `tests/status.rs`.
4. **Interfaces.** `Collector::collect(&Runtime) -> SystemStatus`, `trait Reportable { fn report(&self) -> Component; }` implemented by every service, `status::render_text(&SystemStatus, bool) -> String`.
5. **Data models.** §6.7 in full; `ErrorAction { Retry, Coordinates, UseDefaults, Back }`.
6. **Dependencies.** Every prior phase — this is the first phase that legitimately reads from all of them, and it only reads.
7. **Order.** `Health`/`Component` → `report()` per service → `Collector` → `DaySummary` wiring → the STATUS screen at three sizes → `r` refresh → the `salat status` text formatter → the ERROR screen → wire every `SalatError` producer to it.
8. **Automated tests.** All ten components appear; an in-flight probe reports `Unknown`, never `Ok`; snapshots at three sizes; an empty event log shows the placeholder; the text formatter's output is stable and ANSI-free when piped; every `SalatError` variant renders a unique code, title, details and help; the design's `E001` example is reproduced verbatim by a snapshot; `salat status` exits 0 when healthy and 1 when a fatal component is down.
9. **Manual tests.** Compare `salat status` and the STATUS screen for agreement; break each subsystem in turn (no network, no D-Bus, no player, corrupt DB) and confirm the reported health and the offered error actions; press `r` and watch the components re-probe.
10. **Commands.** `cargo run -- status`, `cargo run -- status | cat`, `cargo test status`, `cargo test error`.
11. **Expected results.** Status is trustworthy — it never claims health it has not verified.
12. **DoD.** Ten components reporting real state; both surfaces share one `SystemStatus`; every error code from §18.2 reachable and snapshot-tested.

---

### Phase 13 — Daemon and systemd

1. **Goal.** Notifications and Adhan work with no terminal open.
2. **Scope.** `salat daemon`, the lock handshake with a foreground instance, signal handling, log-to-file, the user unit, and `--once` for a cron-style invocation.
3. **Files.** `cli/commands/daemon.rs`, `packaging/systemd/salat.service`, `docs/INSTALL.md` (the systemd section).
4. **Interfaces.** `daemon::run(&mut Runtime, once: bool) -> Result<ExitCode, SalatError>`.
5. **Data models.** Reuses `LockFile` from Phase 8.
6. **Dependencies.** Phases 8–10.
7. **Order.** the command → lock acquisition → the scheduler in the foreground of the process → `SIGTERM` graceful shutdown → `SIGHUP` reload → log to file → the unit file → hardening directives → docs.
8. **Automated tests.** `daemon --once` performs a catch-up pass and exits 0; a second daemon exits with a clear "already running" message; `SIGTERM` shuts down within 2 seconds and flushes the event log; a full TUI launched while the daemon runs enters observer mode and fires nothing.
9. **Manual tests.** `systemctl --user enable --now salat`; verify notifications with no terminal open; `journalctl --user -u salat` shows structured logs; reboot and confirm it starts; run the TUI alongside it and confirm exactly one notification per event; `systemctl --user stop` mid-Adhan kills the audio child.
10. **Commands.** `cargo run -- daemon`, `cargo run -- daemon --once`, `systemctl --user status salat`, `journalctl --user -u salat -f`.
11. **Expected results.** No duplicate notifications in any TUI/daemon combination; memory steady well under the 128 MB cap.
12. **DoD.** The unit file's hardening directives verified to still allow notifications and audio; the daemon/TUI arbitration of §4.3 proven by hand.

---

### Phase 14 — Packaging, release and installation

1. **Goal.** A user with no Rust toolchain installs Salat with one command, on an x86_64 Linux PC (aarch64 deferred — §28 D8).
2. **Scope.** `install.sh` complete, the check jobs of `.gitlab-ci.yml` finalized, its release jobs (§22.2), cross-compilation, tarball assembly, `SHA256SUMS`, and a real end-to-end test of the published path.
3. **Files.** `install.sh`, `.gitlab-ci.yml`, `Cross.toml`, `docs/INSTALL.md`.
4. **Interfaces.** The CLI contract of §21.2 (`--prefix`, `--version`, `--uninstall`, `--purge`, `--force`, `--help`) and the fixed asset names of §22.2.
5. **Data models.** None — this phase is shell and YAML.
6. **Dependencies.** A working binary from Phases 0–13. `cross` in CI.
7. **Order.** local musl build → tarball layout → `install.sh` (arch detect → tag resolve → download → **verify** → validate members → extract → install → smoke test → PATH advice) → `--uninstall` → the release jobs of `.gitlab-ci.yml` → a pre-release tag (`v0.1.0-rc.1`, §28 D4) → verify on clean containers → shellcheck → docs.
8. **Automated tests.** `shellcheck install.sh` clean; a CI job that runs `install.sh` against the just-built release artifacts inside `debian:oldstable` and `alpine` x86_64 containers (§28 D8) and asserts `salat --version`; a job that mutates `SHA256SUMS` and asserts the installer **refuses to install**; a job asserting a malicious tarball member (`../evil`) is rejected.
9. **Manual tests.** The exact `curl … | sh` command on Fedora, Ubuntu LTS, Debian oldstable, Arch and Alpine, on x86_64 (aarch64 deferred — §28 D8); install → upgrade → uninstall; install with `$HOME/.local/bin` absent from `PATH` and follow the printed advice; run as root and confirm refusal; run with `sha256sum` removed from `PATH` and confirm abort.
10. **Commands.** `shellcheck install.sh`, `cargo build --release --target x86_64-unknown-linux-musl`, `sh install.sh --prefix /tmp/p`, `sh install.sh --uninstall`, `git tag v0.1.0-rc.1 && git push gitlab v0.1.0-rc.1`.
11. **Expected results.** A single ~4–5 MB gzipped x86_64 asset; install completes in a few seconds; `salat` is on `PATH`; checksum tampering always aborts.
12. **DoD.** The full §22.3 path executed for a real test tag, up to and excluding publication (a prerelease tag never publishes — §28 D4); x86_64 verified on clean containers (§28 D8); the installer refuses every unverified path; `README.md` shows the one-line install.

---

### Phase 15 — Documentation, polish and v0.1.0

1. **Goal.** A project a stranger can find, understand, install, configure, trust and contribute to — then tag `v0.1.0`.
2. **Scope.** `README`, `CONFIGURATION`, `INSTALL`, `CONTRIBUTING`, `CHANGELOG`, screenshots, licensing and attribution, issue templates, and a final accessibility and performance pass.
3. **Files.** `README.md`, `docs/*.md`, `docs/screenshots/*`, `CONTRIBUTING.md`, `CHANGELOG.md`, `LICENSE-THIRD-PARTY`, `.github/ISSUE_TEMPLATE/*`, `xtask` `screenshots`.
4. **Interfaces.** `cargo run -p xtask -- screenshots` regenerates every image at the three breakpoints.
5. **Data models.** None.
6. **Dependencies.** Everything.
7. **Order.** README (what/why → install → screenshots → usage → configuration → keybindings → troubleshooting → privacy → dev → license) → CONFIGURATION (every key, domain, default) → INSTALL (install/upgrade/uninstall/systemd) → CONTRIBUTING (setup, the manual checklist, snapshot review) → screenshots → attribution → templates → CHANGELOG → tag.
8. **Automated tests.** A CI job asserting every config key in `Config` appears in `docs/CONFIGURATION.md` (a simple reflective test over the TOML template) and that the README's install command matches the repository's real raw URL; a link checker over the docs.
9. **Manual tests.** Read the README as a newcomer on GitHub — dark and light — and confirm the screenshots are legible; follow INSTALL on a fresh machine with no prior knowledge; run the whole §20.4 checklist one final time.
10. **Commands.** `cargo run -p xtask -- screenshots`, `cargo test docs`, then the §22.3 release procedure for `v0.1.0`.
11. **Expected results.** `v0.1.0` published with a verified x86_64 binary (§28 D8) and working one-line installation.
12. **DoD.** Every §20.4 manual item passes; documentation covers every user-facing feature; the privacy disclosure is present; the release is installed and verified from the public URL on x86_64 (§28 D8).

---

## 24. Definition of done for every phase

### 24.1 The universal gate

No phase is complete until **all ten** hold. This applies to every phase without exception, including documentation-only ones.

| # | Gate | How it is checked |
|---|---|---|
| 1 | Builds clean | `cargo build` and `cargo build --release --target x86_64-unknown-linux-musl` |
| 2 | Formatted | `cargo fmt --check` |
| 3 | Lint-clean | `cargo clippy --all-targets --all-features -- -D warnings` |
| 4 | Tests pass | `cargo test --all-features`, including every test from prior phases |
| 5 | New behaviour is tested | The phase's own automated tests exist and fail if the feature is reverted |
| 6 | Manual checklist executed | The phase's manual items, plus the resize sweep once a TUI exists |
| 7 | No forbidden constructs | No `unwrap`/`expect`/`todo!`/`unimplemented!`/`dbg!` outside tests; no direct `Utc::now`/`Local::now` outside `SystemClock` |
| 8 | Documented | Public items have `///` docs; user-visible changes are in `CHANGELOG.md`; new config keys are in `docs/CONFIGURATION.md` |
| 9 | Terminal safety | Every exit path — normal, `?`, panic, `Ctrl-C` — restores the terminal (from Phase 5 on) |
| 10 | Figma side-by-side | Every screen the phase touched is rendered at the phase's breakpoints and compared against its Figma frame by a human; the PR records the frame names checked. A phase that touches no screen records `n/a` and the gate is satisfied. Snapshots do not discharge this item (§20.3) |

### 24.2 The phase-specific acceptance signal

One concrete, observable thing that proves each phase actually landed. If it cannot be demonstrated, the phase is not done regardless of test count.

| Phase | Acceptance signal |
|---|---|
| 0 | `salat --help` reads like finished software; CI is green on a PR |
| 1 | Deleting `config.toml` and running `salat config` regenerates a readable, fully commented file identical to the documented defaults |
| 2 | The API test suite covers all seven failure modes and no test touches the network |
| 3 | The §10.1 state table is reproduced row-for-row by tests; clippy rejects a hand-written `Utc::now()` |
| 4 | A second launch with networking disabled shows correct prayer times |
| 5 | `salat` shows a live countdown in the §16.14 inline layout — the five-row compression of the design's small dashboard, not the small dashboard itself — at the pinned budget of 13 content rows, and `salat --once \| cat` is plain text matching its golden byte for byte |
| 6 | All twelve screens are reachable by their documented keys; 79×23 shows only the guard |
| 7 | The dashboard at 100×30 is indistinguishable from the Figma render side by side |
| 8 | An event fires on time while the user sits on the SETTINGS screen, and does not re-fire on restart |
| 9 | A real desktop notification appears at exactly `prayer − before_minutes` |
| 10 | `salat test-adhan` plays audio; the ADHAN overlay still appears on a machine with no player; and an Adhan firing under bare `salat` takes over the inline viewport exactly as §16.15 specifies |
| 11 | Changing the location in the TUI changes the prayer times without a restart |
| 12 | `salat status` and the STATUS screen report the same ten components, and each one is broken on purpose to confirm honesty |
| 13 | Notifications arrive with no terminal open, and exactly once when the TUI is also running |
| 14 | `curl -fsSL …/install.sh \| sh` on a clean container yields a working `salat`, and a tampered checksum aborts |
| 15 | `v0.1.0` is published and installed from the public URL on x86_64 (§28 D8) |

---

## 25. Risks and technical decisions

### 25.1 Risks, ranked by expected cost

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| 1 | **AlAdhan returns HTTP 200 with wrong times for a bad `method`** (verified §7) | certain if unguarded | silent incorrectness — the worst possible failure for this app | Client-side whitelist of valid method ids and `school`/`latitudeAdjustmentMethod`/`midnightMode` domains, enforced in `api::params` **and** in `config::validate`, with `E102`. Never trust a 200. |
| 2 | **Tokio timers are monotonic and do not advance across suspend** | certain on laptops | a missed Adhan | Never `sleep` to a target. 1 Hz wall-clock reconcile + jump detection + the `fired_events` ledger + grace-window catch-up (§11). Manually tested across a real suspend. |
| 3 | **Duplicate notifications from a TUI and a daemon running together** | high | user-visible annoyance, erodes trust | A lock file: exactly one scheduler owner; the loser runs in observer mode (§4.3). Plus the ledger as a second line of defence. |
| 4 | **DST and day-boundary errors** | moderate | wrong times twice a year | All arithmetic in the location's `Tz`, never the host's; the API resolves the offset per date; explicit DST tests on real `Europe/Berlin` transition dates. |
| 5 | **80×24 layouts overflow** | moderate | unreadable output — a stated requirement violated | Explicit row-budget arithmetic (§4.4) per screen, plus a mechanical snapshot assertion that no line exceeds its `Rect`. The too-small guard below 80×24. |
| 6 | **musl + ALSA/PulseAudio linkage breaks static builds** | high if linked | no portable binary at all | No audio library is linked. External player subprocesses only (§13.1), rustls instead of OpenSSL, bundled SQLite, hickory-dns. |
| 7 | **AlAdhan schema drift** | low per month, certain eventually | a broken release | Tolerant `serde` (the `data`-as-string case is already handled), committed fixtures with capture dates, a documented monthly live-vs-fixture comparison, and `E303` degrading to cache rather than crashing. |
| 8 | **`curl \| sh` distributing a corrupted or wrong asset** | low | a security incident | Checksum verified before extraction, abort if no checksum tool, tar member validation, no root, printed URLs and digests, and a documented two-step alternative. |
| 9 | **`cities.bin` bloats the binary or drifts stale** | moderate | a larger download, wrong timezones for new places | `cities15000` only (~34 k rows, ~1.2 MB) plus the curated `extra_cities.tsv` (a hand-verified handful), regenerated by `xtask gen-cities`, with coordinate entry as the always-correct escape hatch. |
| 10 | **Scope creep into a local calculation engine, more providers, or a plugin system** | moderate | v1 never ships | The brief forbids it and this document does not describe it. §26's order ships a useful binary at Phase 5. |
| 11 | **Snapshot tests become noise and get ignored** | moderate | design drift | Fixed clock and fixed data, so a diff always means a real change; reviewed like code; the mechanical width assertion catches the class of bug humans miss. |
| 12 | **Terminal left in raw mode after a panic** | moderate | a hostile first impression | `TerminalGuard` in `Drop` plus a `color-eyre` panic hook that restores first; tested on all four exit paths. |

### 25.2 Decisions, with the alternative that was rejected

| Decision | Chosen | Rejected alternative | Why |
|---|---|---|---|
| Prayer times | AlAdhan only | a local engine, or a second provider as fallback | Mandated by the brief. Also correct for v1: an immutable cache gives offline operation without duplicating astronomical code we cannot validate. |
| Cache expiry | never expires | a TTL or revalidation | The answer for a given (place, method, date) is deterministic. A TTL would create pointless network dependence and *worse* offline behaviour. |
| Config format | TOML file + SQLite for state | all-SQLite, or all-file | The design displays `~/.config/salat/config.toml`, so it must be a real editable file; event ledgers and logs are relational and must be transactional. |
| Timing model | 1 Hz wall-clock reconcile | timer-per-event | Timers die across suspend and multiply across config changes; reconcile is idempotent, trivially testable and survives everything. |
| Audio | external player subprocess | `rodio`/`cpal`, or a bundled decoder | Keeps musl static builds working, requires no audio library, uses whatever the user's system already plays audio with, and costs ~80 lines. |
| Notifications | `notify-rust` | raw `zbus` | One dependency, the standard spec, and graceful degradation is a `Result` we already handle. |
| Countdown display | 3-row half-block glyphs | a figlet-style font crate, or plain text | The design's `4.5em` type demands scale; hand-rolling the 38-glyph 3×3 table of §15.5 is ~60 lines and needs no dependency; plain text loses the design's most distinctive element. |
| Quick mode | `Viewport::Inline` | print-and-clear, or full alternate screen | Preserves scrollback, needs no cursor arithmetic, and leaves the last frame visible after exit — which is what a quick-glance command should do. |
| UI state | `UiModel` snapshot + nine small screen states | one `App` struct | The brief forbids a giant state object; snapshot-plus-pure-render also makes every screen testable without a runtime. |
| Geocoding | embedded GeoNames index | AlAdhan's `byCity` endpoints | Verified: `timingsByCity` returns placeholder coordinates (`8.888…`), so it cannot supply the location the app must display. Offline search is also strictly better UX. |
| Errors | typed per-module + one `SalatError` for presentation | `anyhow` everywhere | The design shows numbered codes with structured details; that requires a real enum. `anyhow`/`eyre` stays at the process boundary only. |
| Binary distribution | static musl tarballs + `install.sh` | distro packages, Homebrew, `cargo install`, AppImage | Mandated one-line install with no toolchain. Distro packaging is a good follow-up but cannot be the primary path for a new project. |
| systemd | opt-in **user** unit | auto-install, or a system unit | Notifications and audio need the session bus; auto-installing units from a piped script is exactly the behaviour that makes installers untrustworthy. |

### 25.3 Open questions deferred past v1, deliberately

Each is listed so a future contributor knows it was considered and excluded, not overlooked: multi-location profiles; Hijri calendar browsing beyond today; prayer streak history and statistics; a Qibla compass visualisation; per-prayer distinct audio files; iqama countdown timers (Salat has no iqama data source — see §16.8's honesty constraint); localisation beyond English; mouse support; theme customisation beyond the three color modes; Wayland/X11-specific integrations; and a Windows or macOS port.

---

## 26. Recommended implementation order

The phase numbers already encode the order. What matters is *why* it is safe, and where the value lands.

```
0 ──▶ 1 ──▶ 2 ──▶ 3 ──▶ 4 ──▶ 5 ◀── first genuinely useful binary
                              │
                              ├──▶ 6 ──▶ 7 ◀── the design becomes real
                              │           │
                              │           ├──▶ 8 ──▶ 9 ──▶ 10 ◀── the app does its actual job
                              │           │
                              │           └──▶ 11 ──▶ 12
                              │                        │
                              └────────────────────────┴──▶ 13 ──▶ 14 ──▶ 15
```

**Three properties of this order:**

1. **Dependencies only ever point backwards, and never more than two deep.** Phase 7 needs 5 and 6. Phase 9 needs 8. Phase 11 needs 6 and, for propagation, 8. Phase 12 reads from everything but writes to nothing. No phase waits on ten unfinished systems.
2. **Value lands early and keeps landing.** Phase 5 ships the default command in finished form — a user could stop there and have something worth running. Phase 7 makes the design visible. Phase 10 completes the core promise (you are reminded, you hear the Adhan). Phases 11–15 broaden and harden.
3. **The hard, invisible work comes before the visible work that depends on it.** The domain (3) and the scheduler (8) are the two places where correctness is genuinely difficult; both are built against `MockClock` with no UI in the way, so their tests are exhaustive rather than incidental.

**Two sequencing rules worth stating explicitly.**

*Do not build the scheduler before the dashboard.* It is tempting — it is the interesting problem. But Phase 7 gives you a live surface on which scheduler behaviour is observable, and a body of snapshot tests that catch layout regressions from every later phase. Building 8 first means debugging timing without being able to see anything.

*Do not defer packaging to the end as a formality.* Phase 14 is late in the list, but the musl release build is a CI gate from Phase 0, so the portable build never breaks silently — the phase adds the installer and the release workflow, not the discovery that a dependency links OpenSSL.

**Suggested checkpoints.** After Phase 5, share the binary and confirm the quick mode matches expectations before building twelve screens on top of its theme. After Phase 7, compare all three breakpoints against Figma before the remaining screens copy those layout conventions. After Phase 10, live with it for a full day of prayers — that is the only real test of a scheduler.

---

## 27. Final checklist for the coding model

Read this before writing the first line, and again at the start of every phase.

**Non-negotiable constraints**

- Rust only. Ratatui + Crossterm for the TUI, Tokio for async. No web frontend, no browser runtime, no rewrite in another language.
- AlAdhan is the only prayer-time source. **Do not** write a calculation engine, and **do not** add a second provider as a fallback.
- The Figma design is the specification. Implement what it shows. Where it is ambiguous, the resolution is already recorded in §3.6 (A1–A10) — follow it rather than inventing a new answer.
- The scheduler is never driven by the render loop. §11 is the contract.
- Minimum terminal 80×24, with 100×30 and 120×35 as the wider targets. Nothing overlaps, nothing is unreadable, nothing wraps unintentionally.
- No root, no `sudo`, no setuid. Installation goes to `~/.local/bin`.
- Tests ship with every phase, not after.

**Before writing code in any phase**

1. Re-read that phase's twelve items in §23 and its acceptance signal in §24.2.
2. Confirm the interfaces you need already exist from an earlier phase. If one does not, you are building out of order.
3. Write the phase's tests alongside the code, not after it.

**Invariants to preserve in every file you touch**

- No `unwrap`, `expect`, `todo!`, `unimplemented!` or `dbg!` outside `#[cfg(test)]`.
- No `Utc::now()`, `Local::now()` or `SystemTime::now()` outside `SystemClock` — take a `&dyn Clock`.
- No I/O in `domain/`. It stays pure, which is what makes it testable.
- All time arithmetic in the location's timezone, never the host's.
- Every error is a typed variant; presentation happens only in `From<…> for SalatError`.
- Non-fatal degradation (no D-Bus, no player, offline with a warm cache) sets a `Component` health and logs — it never shows the ERROR screen.
- Every widget and screen renders as a pure function of `(&UiModel, &ScreenState, Rect, &mut Buffer)`.
- Wide content scrolls or truncates deliberately; it never overflows its `Rect`.
- Every terminal exit path restores the terminal.

**Where to look things up in this document**

| Question | Section |
|---|---|
| What does this screen look like and how does it behave at each size? | §16 (per screen), §3.2 (colors), §4.4–§4.5 (budgets and breakpoints) |
| What exactly do I send to AlAdhan, and what comes back? | §7 |
| What are the config keys, defaults and validation rules? | §8 |
| When does a prayer change state? | §10 |
| When and how does an event fire, and what happens after a restart or a suspend? | §11 |
| Which audio player, invoked how? | §13 |
| What are the CLI flags and their exit codes? | §14 |
| What is the module for this responsibility? | §19 |
| What must I test, and with what tool? | §20 |
| What does the installer do, in what order? | §21 |
| What does the release pipeline produce? | §22 |

**Do not**

- Do not create a monolithic `main.rs` or a single large application-state struct.
- Do not add dependencies beyond §5 without recording the justification the same way §5 does.
- Do not add database tables beyond the five in §17.2 without a stated need.
- Do not add CLI subcommands beyond §14.
- Do not redesign any screen, rename any label, or change any color that §3 specifies.
- Do not defer tests, documentation, or the manual checklist to a later phase.
- Do not skip a phase's acceptance signal because its unit tests pass.

**When something is genuinely ambiguous and this document does not answer it:** choose the smallest implementation that preserves the design's visible behaviour, write down the decision and its reasoning in `docs/ARCHITECTURE.md` under a new "Implementation decisions" section, and keep going. Recording the choice is what keeps the next contributor — or the next session — from silently deciding differently.

---

## 28. Implementation decisions

Decisions made during implementation that this document did not answer, recorded as §27 requires so a later contributor or session does not decide differently.

### D1 — CI/CD provider: GitLab CI/CD (2026-11-24)

**Decision.** GitLab CI/CD is the CI/CD provider for Salat, in the project `gitlab.com/linuxcoding-ADAM/salat`. GitHub Actions is not used.

**What does not change.** GitHub (`github.com/linuxcoding-ADAM/Salat`) remains the canonical repository. It holds the code, tags, issues, the installer's raw URL (§21.1) and the Releases `install.sh` downloads from. This is not a migration to GitLab. The GitLab project receives the same commits only so its pipelines can run.

**Scope.** Only the CI/CD provider changed. Every phase in §23, every deliverable, every automated and manual test, the §20.5 gates, the §24 definition of done, the tarball layout and asset names of §22.2, and the installer contract of §21 stay as specified. The edits made for this decision are limited to provider-specific text: §19's directory tree, §20.5, §21.5's supply-chain row, §22, and Phase 14's scope, files, order, tests, commands and DoD.

**Why.** GitHub Actions is not available on this project's GitHub account.

**Record of the past.** Phase 0's file list (§23) still names `.github/workflows/ci.yml`, because that is what Phase 0 built. It is history and stays as written.

### D2 — Two remotes, GitLab first

`origin` is GitHub (canonical) and `gitlab` is GitLab (CI/CD). They stay separate remotes; `origin` never gets a second push URL. Every commit is pushed to `gitlab` first. The same commit goes to `origin` only after its pipeline is green (§22.1). GitLab pull mirroring from GitHub is not used: it is a paid GitLab feature, and pushing to `gitlab` first is what makes the pipeline a gate.

### D3 — `.github/workflows/ci.yml` removed (2026-11-25)

The GitHub Actions workflow was kept until the equivalent GitLab pipeline had been validated. GitLab pipeline 3110581870 ran all five §22.1 jobs green on commit `b3c1a88`, and the workflow was then removed with approval. GitLab CI/CD is the only CI/CD configuration in the repository, and §19's tree describes the current state. `.github/` returns in Phase 15 with `ISSUE_TEMPLATE/`, because issues stay on GitHub.

### D4 — Prerelease tags verify, they never publish

The Phase 14 test tag is `v0.1.0-rc.1`, replacing the original `v0.0.1-test`. The original tag could never have passed §22.2's `verify` job against `Cargo.toml`'s `0.1.0`. `verify` still requires the tag to equal the `Cargo.toml` version exactly, and now accepts SemVer prerelease versions. So that tag needs `salat/Cargo.toml` at `0.1.0-rc.1` in the tagged commit, which is set during Phase 14.

A prerelease tag runs build, test, packaging, checksums and artifact verification, and **never publishes a GitHub Release**: the `publish` job is not created for it. Publishing is manual (`when: manual`) and limited to stable `vX.Y.Z` tags on protected tags. Phase 14's acceptance signal (§24.2) is therefore shown against the tag pipeline's own artifacts, including the tampered-checksum refusal. The first install from a published Release URL on both architectures is Phase 15's `v0.1.0` acceptance signal, as already specified.

### D5 — The release credential

The `publish` job authenticates to GitHub with a dedicated, least-privilege token. It is a fine-grained GitHub token limited to `linuxcoding-ADAM/Salat` with **Contents: read and write** and no other permission. It is stored as a GitLab CI/CD variable that is **protected** (only pipelines on protected branches and tags), **masked and hidden**, and **scoped to the `release` environment**. It is used only by `publish` and `auth-check` and is never printed: no shell tracing in those jobs, and no echoing of the environment.

*Scope and approval (2026-11-25).* Before this change the scope was `*`, so every job on `main` or a `v*` tag, including `cargo test`, received the token. The variable is now scoped to `release` and only `auth-check` and `publish` declare `environment: release`; both are `when: manual` on protected tags. Verified on throwaway refs, GitLab only: an ordinary job on a protected tag received the token under scope `*` and did not under scope `release`; a manual `environment: release` job received it only after it was started; the same job could not be started after a failed validation job; and a job with `environment: release` on an unprotected branch did not receive it (observed under the old `*` scope, which shows the protected-variable rule applies independently of the scope). GitLab Free has no protected environments or deployment approvals (Premium and Ultimate only), so the approval is a Maintainer pressing Run on a protected tag; there is no second approver. `token-isolation` asserts in every pipeline that ordinary jobs do not see the token, `release-guard` fails if the CI file stops matching this shape, and `release-scripts-test` exercises the release scripts with a fake `gh`. These catch accidents: a reviewed-and-merged change to `.gitlab-ci.yml` or `ci/` still runs with the token at release time, so those files need a line-by-line review.

### D6 — Designed for GitLab Free

The pipeline is designed for GitLab Free and does not depend on any paid or programme-granted feature. Constraints taken into account:

- **Compute minutes are limited.** Jobs install `cross` and `cargo-deny` from their prebuilt releases rather than compiling them, and use the `Cargo.lock`-keyed cache.
- **aarch64 runners.** *Superseded by D8:* aarch64 is deferred past v0.1.0, so no arm64 runner is used. (When written: arm64 shared runners were listed for the project but had not run a job.)
- **Docker-in-Docker.** `cross` needs a Docker-in-Docker service on the shared Linux runners. This is proven by the first pipeline, not assumed.
- **GitLab for Open Source** may raise these limits. Its eligibility is being investigated, but nothing in this design depends on it.

### D7 — The canonical GitHub URL is `github.com/linuxcoding-ADAM/Salat` (2026-11-25)

The GitHub repository was renamed with a capital S. The canonical URL is `https://github.com/linuxcoding-ADAM/Salat`. It is used for the `origin` remote, `salat/Cargo.toml`'s `repository`, the API User-Agent (§7.5), clone instructions, and the installer's raw URL (§21.1). GitHub redirects the old lowercase URL, but nothing in the repository relies on that redirect.

What deliberately stays lowercase, and is **not** part of this rename:

- the crate, the binary and every command: `salat`;
- every runtime path: `~/.config/salat`, `~/.local/share/salat`, and the rest;
- release asset names: `salat-<version>-<target>.tar.gz` (§22.2);
- the GitLab CI/CD project: `gitlab.com/linuxcoding-ADAM/salat`.

A clone now creates a `Salat/` directory, so clone instructions say `cd Salat`.

### D8 — v0.1.0 targets x86_64 Linux PCs; aarch64 is deferred (2026-11-25)

**Decision.** The v0.1.0 release targets standard Linux desktop PCs with 64-bit Intel/AMD processors (`x86_64`). ARM (`aarch64`) is **deferred**, not dropped: Raspberry Pi, ARM laptops and ARM servers are not targets of this release.

**What the plan originally committed to**, kept here as history: static musl binaries for `x86_64` **and** `aarch64` (§2.1, §21.3); an aarch64 build in the release matrix and an aarch64 tarball (§22.2); verification "for both architectures" (§22.3 step 6); Phase 14's goal, automated tests (Debian and Alpine "for both arches"), manual tests ("on x86_64 and aarch64") and DoD ("both arches verified"); Phase 15's expected results ("two verified binaries") and DoD, plus §24.2's Phase 15 signal ("on both architectures"); and §28 D6's arm64-runner constraint. Each of those places now reads x86_64 and points here.

**Deferred**, until a later release brings aarch64 back:

- the `aarch64-unknown-linux-musl` build and its tarball, `salat-<version>-aarch64-unknown-linux-musl.tar.gz`;
- its line in `SHA256SUMS`;
- the aarch64 installer tests (Debian and Alpine on arm64) and the aarch64 manual tests;
- arm64 CI runners.

**What does not change.** Everything else in Phase 14 stays as specified. That includes all of §21.2, the checksum verification before extraction, the archive member validation, the root refusal, upgrade and `--force`, `--uninstall` and `--purge`, the Debian and Alpine installer tests, the tamper and `../evil` tests, and the release safeguards of §22.2 and D4. No application code is architecture-specific, and §21.3's design choices (musl, rustls, bundled SQLite, no linked audio library, hickory DNS) are architecture-neutral.

**The installer is explicit about it.** §21.2 step 5 maps `aarch64`/`arm64` to its existing unsupported-architecture failure: it prints `salat: aarch64 is not supported yet; supported: x86_64` and exits 1 before downloading anything. An ARM user gets a clear message instead of a download error for an asset that does not exist.

**How aarch64 returns.** The asset naming scheme already covers it. Bringing it back means: one more `build` matrix entry, one more `SHA256SUMS` line, re-mapping `aarch64|arm64` in step 5, and the arm64 installer tests. Each place marked "§28 D8" then goes back to its original wording above.

---

*End of architecture and execution plan.*





