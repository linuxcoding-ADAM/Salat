# Salat

Prayer times and Adhan for the Linux terminal — a keyboard-first,
terminal-native assistant built with Ratatui, in the spirit of btop and
lazygit. Midnight aesthetic, no mouse required.

```
salat                Quick live view — today's times, next prayer, countdown
salat --full         Full interactive TUI — settings, location, status
salat --once         Print one frame and exit (pipe- and status-bar-safe)
salat status         Print system status and exit
salat config         Print resolved paths, then the effective configuration
salat test-adhan     Play the configured Adhan and exit
salat daemon         Run the scheduler headlessly (for systemd)
```

Prayer times come from the [AlAdhan API](https://aladhan.com) — the sole
provider for v1. City search is offline, backed by an embedded index of
[GeoNames](https://www.geonames.org) cities (population ≥ 15,000),
regenerated with `cargo run -p xtask -- gen-cities`.

## Status

Under active development, phase by phase per `docs/ARCHITECTURE.md`.
See `CHANGELOG.md` for what works today.

## Install

Not yet — the installer ships with the first release. Build from source:

```sh
cargo build --release        # binary: target/release/salat
```

## Configuration

Configuration lives at `~/.config/salat/config.toml` and is created with
defaults on first run. The full key reference will live in
`docs/CONFIGURATION.md`.

## Development

```sh
cargo run                    # quick mode
cargo run -- --full          # full TUI
cargo test
cargo fmt && cargo clippy --all-targets -- -D warnings
```

The toolchain is pinned by `rust-toolchain.toml`; `rustup` handles the rest.
The architecture document (`docs/ARCHITECTURE.md`) is the source of truth
for every implementation decision.

## License

MIT OR Apache-2.0, at your option. See `LICENSE`.
Third-party notices: `LICENSE-THIRD-PARTY`.
