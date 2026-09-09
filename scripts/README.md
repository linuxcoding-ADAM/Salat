# Manual regression scripts

Drivers for the architecture doc's §23 manual acceptance checks. They run
the real binary under a real pty (pyte renders the screen, so assertions
read the frame a user would see, not the escape stream) and hit the live
AlAdhan API. They are deliberately **not** part of `cargo test` — they need
a network connection and a debug build — so run them by hand when you want
to re-verify a phase's manual items.

## `p11_manual.py` — Phase 11 (§23 item 9)

Four scenarios, run one at a time:

```
python3 scripts/p11_manual.py a_search   # location change by search, save + CONFIRM, times change
python3 scripts/p11_manual.py b_coords   # location by coordinates, then auto-detect; times change
python3 scripts/p11_manual.py c_before   # change before_minutes with an event ~3 min out; live re-arm fires
python3 scripts/p11_manual.py d_watch    # hand-edit the TOML while running; SIGHUP picks it up
```

Prerequisites:

- Python ≥ 3.11 (`tomllib`) and `pyte` (`pip install --user pyte`).
- A debug build at `$REPO/target/debug/salat` (`cargo build`).
- Network access to `api.aladhan.com` (scenario `c_before` probes it live
  to pick a city whose target prayer is still undischarged).

Notes:

- `REPO` at the top of the script is hardcoded to the original author's
  checkout; adjust it if the repo lives elsewhere.
- Each run creates an isolated throwaway state dir (mkdtemp) with its own
  XDG dirs and `HOME`, so it never touches your real `~/.config/salat`.
- `c_before` stages the clock so the scheduler's Pre-Adhan notification
  fires during the run; the other scenarios finish in seconds.

## `p12_manual.py` — Phase 12 (§23 item 9)

Three scenarios, run one at a time:

```
python3 scripts/p12_manual.py a_screen   # STATUS screen: ten components, live clock; `r` → REFRESHING… → settle
python3 scripts/p12_manual.py b_agree    # `salat status` and the STATUS screen agree on all ten components
python3 scripts/p12_manual.py c_broken   # no player on $PATH: both surfaces report NO AUDIO PLAYER, exit 0
```

Same prerequisites as `p11_manual.py` (the TUI scenarios boot online — a
cold boot cannot reach the render loop without a window). Scenario
`b_agree` presses `r` on the TUI before comparing: the text surface's
own refresh probes the network, and a TUI booted on the fresh cache it
left makes no HTTP attempt — `Network` honestly stays `Unknown` until
probed, so both surfaces must have actually probed before the row
compares.
