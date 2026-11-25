# Contributing to Salat

Thank you for helping. This guide covers setting up, the checks a change
must pass, snapshot review, conventions, how CI gates changes, and how a
release is made. The design itself — every module, decision and phase —
is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md), which is the source
of truth for implementation decisions.

Issues and pull requests go to the canonical repository,
<https://github.com/linuxcoding-ADAM/Salat>. Salat supports Linux on
`x86_64` only, so develop and test there.

## Setup

```sh
git clone https://github.com/linuxcoding-ADAM/Salat.git
cd Salat
cargo build
```

`rust-toolchain.toml` pins the Rust toolchain (with `rustfmt` and
`clippy`); `rustup` installs it automatically on the first `cargo`
command, so contributors and CI use the same compiler. The repository is
a cargo workspace with two crates: `salat` (the application) and `xtask`
(maintenance tasks).

Running it locally:

```sh
cargo run                    # quick view
cargo run -- --full          # full TUI
cargo run -- --config /tmp/dev.toml --full   # a separate instance: its own config, database and lock
```

## The checks a change must pass

These are the checks GitLab CI runs on every push; run them before
sending a change:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

- The test suite makes **no real network calls**: the AlAdhan API is
  replaced by recorded fixtures.
- Run the tests as a normal user, not root. Root ignores file
  permissions, so the permission tests (an unreadable Adhan file, a
  read-only configuration directory) only mean something without it.
- The daemon tests use `kill` from `procps`.
- One test is ignored by default because it needs a real pseudo-terminal.

CI also runs `cargo deny check advisories bans licenses sources`
(dependency advisories and licences, configured in `deny.toml`),
`cargo check --workspace --all-targets` against the pinned toolchain,
and `shellcheck install.sh ci/*.sh`. If you change dependencies or shell
scripts, run those too.

## Snapshot tests

Screens and frames are tested with [insta](https://insta.rs) snapshots in
`salat/tests/snapshots/`. When a change alters what a screen draws, the
affected tests fail and show the difference. To accept new output:

```sh
INSTA_UPDATE=always cargo test --all-features
git diff salat/tests/snapshots/
```

Review the snapshot diff like code: every changed line should be one you
meant to change. Then run the tests again without `INSTA_UPDATE`, to
confirm the committed files match what the code draws.

The loading screen shows the crate version, so a version bump changes
its six `loading_stage_*` snapshots; regenerate them as above.

## Manual checks

Some behaviour cannot be automated — a real pseudo-terminal, the live
API, systemd. `scripts/` holds drivers for the architecture document's
manual acceptance checks; see [`scripts/README.md`](scripts/README.md).
The installation and systemd checks are described in
[`docs/INSTALL.md`](docs/INSTALL.md).

## Conventions

- **Formatting and lints:** `rustfmt` and `clippy` with warnings denied,
  as configured in `rustfmt.toml` and `clippy.toml`.
- **Commits:** a short, imperative subject line, optionally prefixed with
  the area — for example `CI: …`, `Docs: …`, `Tests: …`, `Deps: …`, or
  `chore(release): prepare vX.Y.Z` for a release. The body explains why.
- **Documentation:** a change to user-facing behaviour updates the
  matching document in the same change — a configuration key in
  [`docs/CONFIGURATION.md`](docs/CONFIGURATION.md), a command or flag in
  the README, an error in
  [`docs/TROUBLESHOOTING.md`](docs/TROUBLESHOOTING.md) — and adds a line
  under `[Unreleased]` in [`CHANGELOG.md`](CHANGELOG.md).
- **Design decisions** that change the plan are recorded in
  `docs/ARCHITECTURE.md` §28.
- **Known defects** that are not being fixed in the current release are
  recorded, with their evidence, in
  [`docs/KNOWN_ISSUES.md`](docs/KNOWN_ISSUES.md).

## CI and the merge gate

CI runs on GitLab CI/CD (`.gitlab-ci.yml`); GitHub stays the canonical
repository and the home of Releases (`docs/ARCHITECTURE.md` §22, §28).

| Stage | Jobs |
|---|---|
| check | `check` (fmt, clippy, tests), `msrv`, `supply-chain` (cargo-deny), `lint-shell` (shellcheck) |
| build | `build-musl` — the static `x86_64-unknown-linux-musl` release binary |
| package | `package` — verifies that binary and builds the release tarball and `SHA256SUMS` |
| verify | `verify-installer-debian`, `verify-installer-alpine` — the installer tests against those artifacts (manual on branches, automatic on tags) |

GitLab cannot gate GitHub branches, so the gate is a procedure: a
commit is pushed to GitLab first, and only when its pipeline is green is
the same commit pushed to GitHub.

## Releases

The full procedure is `docs/ARCHITECTURE.md` §22.3. In short:

1. Add a `## [X.Y.Z] - YYYY-MM-DD` section to `CHANGELOG.md`.
2. Set `version` in `salat/Cargo.toml`, refresh `Cargo.lock`, and
   regenerate the loading-screen snapshots.
3. Commit `chore(release): prepare vX.Y.Z`, push to GitLab, and after a
   green pipeline push the same commit to GitHub.
4. Create the annotated tag `vX.Y.Z` and push it to GitLab (`v*` tags
   are protected there, so only maintainers can create them). The tag
   pipeline adds `verify-version` (the tag must equal the Cargo version),
   runs every job above, and runs both installer tests automatically.
5. When it is green, push the tag to GitHub, then start the manual
   `publish` job, which creates the GitHub Release with the tarball and
   `SHA256SUMS` and uses the changelog section as the release notes.
6. Verify the published release by running the one-line installer on a
   clean x86_64 system.

A prerelease tag such as `v0.1.0-rc.1` runs the same pipeline plus
`auth-check` (a read-only test of the release token) and never
publishes: the `publish` job only exists for stable `vX.Y.Z` tags.

## The monthly AlAdhan drift check

The tests in `salat/tests/fixtures/aladhan/` parse **recorded** API
responses, so they only stay meaningful while the live API keeps the
shape those recordings captured. Once a month, run:

```
cargo run -p xtask -- check-fixtures
```

It re-captures the five live endpoints (calendar, timings, qibla,
methods, and the geocode-400 error) and compares each response's shape —
field names, types, array element shapes — against the committed
fixture. Prayer times legitimately shift by a minute or two as the year
advances; that is not drift. If the tool reports drift:

1. Update the fixture **and** the DTOs in `salat/src/api/dto.rs` in the
   same commit — a fixture that no longer matches the parser is worse
   than none.
2. If the method list changed, the §7.4 whitelist test
   (`method_whitelist_is_exactly_the_methods_fixture`) will also fail;
   update `is_valid_method` in `salat/src/config/validate.rs` with it.
3. Record the new capture date in
   `salat/tests/fixtures/aladhan/README.md`.

The check needs `curl` and network access; the test suite itself never
performs real network I/O (§23 Phase 2 item 11).

See `salat/tests/fixtures/aladhan/README.md` for the full fixture
inventory, including why `error_latitude_503.json` is outside the
automated check.
