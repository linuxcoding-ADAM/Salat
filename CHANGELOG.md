# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Phase 0 — Foundation and CLI skeleton (§23 of `docs/ARCHITECTURE.md`):
  the cargo workspace (`salat` + `xtask`), pinned toolchain, lint and
  supply-chain configuration, the full `src/` module tree as documented
  stubs, `SalatError` with the final error-code table, `Cli` parsing and
  dispatch with the five exit codes, file logging to
  `$XDG_STATE_HOME/salat/salat.log`, and CI. Every command responds with
  an explicit not-implemented error rather than a panic; the services
  themselves arrive phase by phase.
