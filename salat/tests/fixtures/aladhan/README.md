# AlAdhan API fixtures (§7.8)

Recorded evidence of what `https://api.aladhan.com` actually returns —
captured with `curl`, never hand-written. The DTO layer's unit tests parse
these files, so a schema drift upstream becomes a test failure here rather
than a runtime surprise.

**Captured 2026-09-05** against the canonical §7.2 request (the default
configuration: Sétif 36.19/5.41, method 12, school 0, angle-based
adjustment, standard midnight, `Africa/Algiers`, zero tune):

```
latitude=36.19&longitude=5.41&method=12&school=0
&latitudeAdjustmentMethod=3&midnightMode=0
&timezonestring=Africa/Algiers&tune=0,0,0,0,0,0,0,0,0&iso8601=true
```

| File | Endpoint / condition |
|---|---|
| `calendar_ok.json` | `GET /v1/calendar/2026/09` — 30 days, ~44 KB |
| `timings_ok.json` | `GET /v1/timings/05-09-2026` — single day |
| `qibla.json` | `GET /v1/qibla/36.19/5.41` → `106.78251860469342` |
| `methods.json` | `GET /v1/methods` — the method whitelist source |
| `error_geocode_400.json` | `timingsByCity` with a bogus city — HTTP 400, **`data` is a bare string** |
| `error_latitude_503.json` | `latitude=999` — HTTP 503 (not 400!), the §7.4 trap |
| `malformed_truncated.json` | `calendar_ok.json` cut at byte 20000 — a truncated body |

`salat` itself never calls `timingsByCity`; the 400 fixture exists purely
as evidence of the polymorphic `data` field (§7.3).

## Monthly drift check (§23 Phase 2 item 9)

Automated: `cargo run -p xtask -- check-fixtures` re-runs each capture
live and compares the response **shape** — field names, types, array
element shapes — against the committed file. Times shift by a minute or
two as the year advances; that is expected and *not* a schema drift. What
must not change is the shape: the field names, the ISO-8601 form of
every timing, `data` being an array/object on success and a string on
error, and `meta`'s layout. If the shape changes, update the fixtures
*and* the DTOs in the same commit, and record the new date above.

Two fixtures are outside the automated check: `malformed_truncated.json`
is a synthetic corruption, and `error_latitude_503.json` records an
unstable behaviour — at capture time (2026-09-05) `latitude=999` answered
a fabricated 503, but hours later the same request answered **HTTP 200
with fabricated times** (Sunrise after 10:00, Sunset 15:05 — nonsense).
An upstream that will not even fail consistently is the strongest
argument for §7.4's client-side coordinate validation; the fixture stays
as evidence of the 503 form, and the DTO tests keep asserting its shape.
