# Contributing to salat

> This guide starts small and grows with the project — the full
> contributor documentation (style, commit conventions, release process)
> lands with Phase 15 (§23). What exists today is the one routine the
> API layer depends on.

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
