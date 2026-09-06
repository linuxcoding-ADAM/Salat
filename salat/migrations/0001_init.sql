-- 0001_init.sql — the §17.2 schema, five tables, no joins.
--
-- One deviation from the §17.2 inline SQL, resolved by the doc itself:
-- §8.1 and §11.4 scope the fired-events ledger by a fourth primary-key
-- column (so `--config` profiles cannot see each other's firings), which
-- makes fired_events PRIMARY KEY (scope, date, prayer, kind).

CREATE TABLE prayer_days (
    cache_key  TEXT NOT NULL,        -- the readable canonical string itself (§17.2), never a hash
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

CREATE TABLE fired_events (
    scope    TEXT NOT NULL,          -- 'default' or the §8.1 --config scope
    date     TEXT NOT NULL,          -- YYYY-MM-DD, the prayer's civil date
    prayer   TEXT NOT NULL,          -- 'fajr' … 'isha', '' for none
    kind     TEXT NOT NULL,          -- 'pre_notify' | 'adhan' | 'mosque_reminder' (§11.4)
    fired_at TEXT NOT NULL,          -- RFC3339
    late     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (scope, date, prayer, kind)
);

CREATE TABLE event_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at       TEXT NOT NULL,          -- RFC3339
    category TEXT NOT NULL,          -- 'notification'|'adhan'|'state'|'error'|'system'
    label    TEXT NOT NULL,          -- e.g. '  Pre-Adhan notification'
    detail   TEXT,                   -- e.g. 'MAGHRIB · 5 min window'
    prayer   TEXT
);
CREATE INDEX idx_event_log_at ON event_log(at DESC);

CREATE TABLE prayer_completions (
    date         TEXT NOT NULL,      -- YYYY-MM-DD
    prayer       TEXT NOT NULL,      -- 'fajr' … 'isha'
    completed_at TEXT NOT NULL,      -- RFC3339
    PRIMARY KEY (date, prayer)
);

CREATE TABLE qibla (
    cache_key  TEXT PRIMARY KEY,
    direction  REAL NOT NULL,        -- degrees from true North
    fetched_at TEXT NOT NULL         -- RFC3339
);
