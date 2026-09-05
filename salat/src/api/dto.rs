//! Tolerant deserialization of AlAdhan responses (§7.3, §7.8).
//!
//! Two-stage parse: the wire envelope is `{code, status, data}` where
//! `data` is polymorphic — an array (calendar), an object (timings, qibla)
//! or a **bare string** (every error response) — so it is first captured
//! as raw JSON, and only branched into a typed shape once `code` says
//! which of those it is.
//!
//! Tolerant where the API is loose (unknown fields ignored, per-day `meta`
//! reduced to what mapping needs) and strict where correctness demands it:
//! every timing is parsed as an RFC 3339 timestamp — the `"05:12 (CET)"`
//! suffix form is a **parse failure**, not a value (§7.2) — and
//! `meta.offset` is never deserialized at all, because with a non-zero
//! tune it arrives as strings instead of integers (§7.3).

use chrono::{DateTime, FixedOffset, NaiveDate};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::ApiError;

/// The §7.3 envelope, generic over the success shape of `data`.
#[derive(Debug)]
pub struct ApiEnvelope<T> {
    pub code: u16,
    pub status: String,
    pub data: T,
}

impl ApiEnvelope<serde_json::Value> {
    /// Stage one: parse the envelope with `data` still untyped.
    pub fn from_json(body: &str) -> Result<Self, ApiError> {
        #[derive(Deserialize)]
        struct Wire {
            code: u16,
            status: String,
            data: serde_json::Value,
        }
        let wire: Wire = serde_json::from_str(body)
            .map_err(|e| ApiError::Malformed(format!("response is not valid JSON: {e}")))?;
        Ok(Self {
            code: wire.code,
            status: wire.status,
            data: wire.data,
        })
    }

    /// Stage two: on `code == 200` deserialize `data` into `T`; otherwise
    /// `data` is the API's own error message (§7.3, the bare-string form).
    pub fn into_result<T: DeserializeOwned>(self) -> Result<T, ApiError> {
        if self.code == 200 {
            return serde_json::from_value(self.data).map_err(|e| {
                ApiError::Malformed(format!("success payload has an unexpected shape: {e}"))
            });
        }
        match self.data.as_str() {
            Some(message) => Err(ApiError::Upstream(format!(
                "{} (HTTP {}): {}",
                self.status, self.code, message
            ))),
            None => Err(ApiError::Malformed(format!(
                "error envelope (HTTP {}) without a string message",
                self.code
            ))),
        }
    }
}

/// A calendar day (and, via the `TimingsDto` alias, the single-day shape
/// `/v1/timings` returns — they are the same object).
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarDayDto {
    /// The eleven timings, already parsed to instants.
    pub times: PrayerTimesDto,
    /// The civil date this day is for.
    pub gregorian_date: NaiveDate,
    /// The Hijri date for the same civil day.
    pub hijri: HijriDto,
    /// The per-response metadata, reduced to what mapping needs.
    pub meta: MetaDto,
}

/// `/v1/timings` returns exactly one `CalendarDayDto`.
pub type TimingsDto = CalendarDayDto;

/// The eleven timings of a day, parsed as RFC 3339 instants with the
/// day's real UTC offset. `Midnight` (and `Lastthird`) may fall on the
/// **next** civil day — that is correct, not a bug (§7.2).
#[derive(Debug, Clone, PartialEq)]
pub struct PrayerTimesDto {
    pub imsak: DateTime<FixedOffset>,
    pub fajr: DateTime<FixedOffset>,
    pub sunrise: DateTime<FixedOffset>,
    pub dhuhr: DateTime<FixedOffset>,
    pub asr: DateTime<FixedOffset>,
    pub maghrib: DateTime<FixedOffset>,
    pub sunset: DateTime<FixedOffset>,
    pub isha: DateTime<FixedOffset>,
    pub midnight: DateTime<FixedOffset>,
    pub first_third: DateTime<FixedOffset>,
    pub last_third: DateTime<FixedOffset>,
}

/// The Hijri date of a day, typed (the wire form is all strings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HijriDto {
    /// `DD-MM-YYYY` in the Hijri calendar, as the API wrote it.
    pub date: String,
    pub day: u32,
    /// Month number, `1..=12`.
    pub month: u32,
    /// English month name, e.g. `Rabīʿ al-awwal`.
    pub month_name: String,
    pub year: i32,
    /// Islamic occasions the API marks on this day (may be empty).
    pub holidays: Vec<String>,
}

/// The response `meta`, reduced: timezone, and which method actually
/// produced these times — the echo that lets a caller notice a silent
/// remap (§7.4). `offset` is deliberately absent: with a non-zero tune it
/// arrives as strings, and its information is already baked into the
/// timings (§7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaDto {
    pub timezone: String,
    pub method_id: u32,
    pub method_name: String,
}

// ---- wire structs ---------------------------------------------------------
//
// Everything below mirrors the committed fixtures (§7.8) and nothing else.
// Unknown fields are ignored by design — additive upstream fields must not
// break the parse.

#[derive(Debug, Deserialize)]
struct RawDay {
    timings: RawTimings,
    date: RawDate,
    meta: RawMeta,
}

#[derive(Debug, Deserialize)]
struct RawTimings {
    #[serde(rename = "Imsak")]
    imsak: String,
    #[serde(rename = "Fajr")]
    fajr: String,
    #[serde(rename = "Sunrise")]
    sunrise: String,
    #[serde(rename = "Dhuhr")]
    dhuhr: String,
    #[serde(rename = "Asr")]
    asr: String,
    #[serde(rename = "Sunset")]
    sunset: String,
    #[serde(rename = "Maghrib")]
    maghrib: String,
    #[serde(rename = "Isha")]
    isha: String,
    #[serde(rename = "Midnight")]
    midnight: String,
    #[serde(rename = "Firstthird")]
    first_third: String,
    #[serde(rename = "Lastthird")]
    last_third: String,
}

#[derive(Debug, Deserialize)]
struct RawDate {
    gregorian: RawGregorian,
    hijri: RawHijri,
}

#[derive(Debug, Deserialize)]
struct RawGregorian {
    /// `DD-MM-YYYY`.
    date: String,
}

#[derive(Debug, Deserialize)]
struct RawHijri {
    date: String,
    day: String,
    month: RawHijriMonth,
    year: String,
    #[serde(default)]
    holidays: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawHijriMonth {
    number: u32,
    en: String,
}

#[derive(Debug, Deserialize)]
struct RawMeta {
    timezone: String,
    method: RawMethodMeta,
    // No `offset` field, on purpose — see `MetaDto`.
}

#[derive(Debug, Deserialize)]
struct RawMethodMeta {
    id: u32,
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawQibla {
    direction: f64,
}

// ---- wire → DTO -----------------------------------------------------------

fn timing(field: &str, raw: &str) -> Result<DateTime<FixedOffset>, ApiError> {
    DateTime::parse_from_rfc3339(raw).map_err(|_| {
        ApiError::Malformed(format!(
            "timing {field} is not an ISO-8601 timestamp (found \"{raw}\") — \
             every request must send iso8601=true (§7.2)"
        ))
    })
}

impl TryFrom<&RawDay> for CalendarDayDto {
    type Error = ApiError;

    fn try_from(raw: &RawDay) -> Result<Self, Self::Error> {
        let t = &raw.timings;
        let times = PrayerTimesDto {
            imsak: timing("Imsak", &t.imsak)?,
            fajr: timing("Fajr", &t.fajr)?,
            sunrise: timing("Sunrise", &t.sunrise)?,
            dhuhr: timing("Dhuhr", &t.dhuhr)?,
            asr: timing("Asr", &t.asr)?,
            maghrib: timing("Maghrib", &t.maghrib)?,
            sunset: timing("Sunset", &t.sunset)?,
            isha: timing("Isha", &t.isha)?,
            midnight: timing("Midnight", &t.midnight)?,
            first_third: timing("Firstthird", &t.first_third)?,
            last_third: timing("Lastthird", &t.last_third)?,
        };
        let gregorian_date = NaiveDate::parse_from_str(&raw.date.gregorian.date, "%d-%m-%Y")
            .map_err(|_| {
                ApiError::Malformed(format!(
                    "gregorian date is not DD-MM-YYYY (found \"{}\")",
                    raw.date.gregorian.date
                ))
            })?;
        let hijri = &raw.date.hijri;
        let day = hijri.day.parse::<u32>().map_err(|_| {
            ApiError::Malformed(format!(
                "hijri day is not a number (found \"{}\")",
                hijri.day
            ))
        })?;
        let year = hijri.year.parse::<i32>().map_err(|_| {
            ApiError::Malformed(format!(
                "hijri year is not a number (found \"{}\")",
                hijri.year
            ))
        })?;
        Ok(Self {
            times,
            gregorian_date,
            hijri: HijriDto {
                date: hijri.date.clone(),
                day,
                month: hijri.month.number,
                month_name: hijri.month.en.clone(),
                year,
                holidays: hijri.holidays.clone(),
            },
            meta: MetaDto {
                timezone: raw.meta.timezone.clone(),
                method_id: raw.meta.method.id,
                method_name: raw.meta.method.name.clone(),
            },
        })
    }
}

/// Parse a `/v1/calendar/{year}/{month}` success body.
pub fn parse_calendar(body: &str) -> Result<Vec<CalendarDayDto>, ApiError> {
    let days: Vec<RawDay> = ApiEnvelope::from_json(body)?.into_result()?;
    days.iter().map(CalendarDayDto::try_from).collect()
}

/// Parse a `/v1/timings/{date}` success body.
pub fn parse_timings(body: &str) -> Result<TimingsDto, ApiError> {
    let day: RawDay = ApiEnvelope::from_json(body)?.into_result()?;
    CalendarDayDto::try_from(&day)
}

/// Parse a `/v1/qibla/{lat}/{lng}` success body → the Qibla direction in
/// degrees from true North.
pub fn parse_qibla(body: &str) -> Result<f64, ApiError> {
    let qibla: RawQibla = ApiEnvelope::from_json(body)?.into_result()?;
    Ok(qibla.direction)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/aladhan")
            .join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"))
    }

    /// The §7.8 calendar fixture: 30 days, every timing parsed, the
    /// documented specimens in place, Midnight rolling to the next civil
    /// day (§7.2).
    #[test]
    fn calendar_fixture_parses_all_thirty_days() {
        let days = parse_calendar(&fixture("calendar_ok.json")).unwrap();
        assert_eq!(days.len(), 30);

        let first = &days[0];
        assert_eq!(
            first.gregorian_date,
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()
        );
        assert_eq!(first.times.fajr.to_rfc3339(), "2026-09-01T05:12:00+01:00");
        // §7.2: Midnight is the *next* civil day's 00:38.
        assert_eq!(
            first.times.midnight.to_rfc3339(),
            "2026-09-02T00:38:00+01:00"
        );
        assert_eq!(
            first.times.midnight.date_naive(),
            first.gregorian_date + chrono::Duration::days(1)
        );

        // The last day is the 30th, and the hijri date is typed.
        let last = &days[29];
        assert_eq!(
            last.gregorian_date,
            NaiveDate::from_ymd_opt(2026, 9, 30).unwrap()
        );
        assert_eq!(last.hijri.date, "19-04-1448");
        assert_eq!(last.hijri.day, 19);
        assert_eq!(last.hijri.month, 4);
        assert_eq!(last.hijri.month_name, "Rabīʿ al-thānī");
        assert_eq!(last.hijri.year, 1448);

        // Meta carries the method echo that defends against silent
        // remapping (§7.4).
        assert_eq!(first.meta.method_id, 12);
        assert_eq!(
            first.meta.method_name,
            "Union Organization Islamic de France"
        );
        assert_eq!(first.meta.timezone, "Africa/Algiers");

        // 2026-09-22 carries a holiday in the fixture.
        assert_eq!(
            days[21].hijri.holidays,
            vec!["Urs of Shaykh Abdul Qadir Jilāni ق".to_owned()]
        );
    }

    /// The §7.2 specimen: the timings endpoint returns one day of the
    /// same shape (`TimingsDto` is `CalendarDayDto`).
    #[test]
    fn timings_fixture_parses_one_day() {
        let day = parse_timings(&fixture("timings_ok.json")).unwrap();
        assert_eq!(
            day.gregorian_date,
            NaiveDate::from_ymd_opt(2026, 9, 5).unwrap()
        );
        assert_eq!(day.times.fajr.to_rfc3339(), "2026-09-05T05:16:00+01:00");
        assert_eq!(day.times.midnight.to_rfc3339(), "2026-09-06T00:37:00+01:00");
    }

    /// §7.1: the Qibla direction for Sétif.
    #[test]
    fn qibla_fixture_parses_the_direction() {
        let direction = parse_qibla(&fixture("qibla.json")).unwrap();
        assert!((direction - 106.782_518_604_693_42).abs() < 1e-9);
    }

    /// §7.3: an error envelope's `data` is a bare string — both committed
    /// error fixtures surface as `Upstream` with the API's message.
    #[test]
    fn error_envelopes_surface_as_upstream() {
        let error = parse_calendar(&fixture("error_geocode_400.json")).unwrap_err();
        match &error {
            ApiError::Upstream(message) => {
                assert!(message.contains("Unable to geocode"), "{message}");
                assert!(message.contains("400"), "{message}");
            }
            other => panic!("expected Upstream, got {other:?}"),
        }

        let error = parse_timings(&fixture("error_latitude_503.json")).unwrap_err();
        match &error {
            ApiError::Upstream(message) => {
                assert!(message.contains("Timezone lookup"), "{message}");
            }
            other => panic!("expected Upstream, got {other:?}"),
        }
    }

    /// A truncated body is `Malformed`, not a panic and not `Upstream`.
    #[test]
    fn truncated_body_is_malformed() {
        let error = parse_calendar(&fixture("malformed_truncated.json")).unwrap_err();
        assert!(matches!(error, ApiError::Malformed(_)), "{error:?}");
    }

    /// §7.2: the `"05:12 (CET)"` suffix form must not parse — that is the
    /// whole reason every request sends `iso8601=true`.
    #[test]
    fn the_cet_suffix_form_is_rejected() {
        let body = fixture("timings_ok.json").replace(
            "\"Fajr\":\"2026-09-05T05:16:00+01:00\"",
            "\"Fajr\":\"05:16 (CET)\"",
        );
        let error = parse_timings(&body).unwrap_err();
        match &error {
            ApiError::Malformed(message) => {
                assert!(message.contains("Fajr"), "{message}");
                assert!(message.contains("05:16 (CET)"), "{message}");
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    /// §7.3: `meta.offset` arrives as **strings** when a tune is set —
    /// the parse must not depend on its shape, because it never looks at
    /// it.
    #[test]
    fn string_offsets_do_not_break_the_parse() {
        let body = fixture("timings_ok.json").replace(
            "\"offset\":{\"Imsak\":0,\"Fajr\":0,\"Sunrise\":0,\"Dhuhr\":0,\"Asr\":0,\"Maghrib\":0,\"Sunset\":0,\"Isha\":0,\"Midnight\":0}",
            "\"offset\":{\"Imsak\":\"0\",\"Fajr\":\"0\",\"Sunrise\":\"0\",\"Dhuhr\":\"0\",\"Asr\":\"0\",\"Maghrib\":\"0\",\"Sunset\":\"0\",\"Isha\":\"0\",\"Midnight\":\"0\"}",
        );
        assert!(parse_timings(&body).is_ok());
    }

    /// The pinning test promised by the §7.4 correction note: the config
    /// whitelist must equal the `/v1/methods` fixture's id set exactly, so
    /// future drift fails here instead of producing silent wrong times.
    #[test]
    fn method_whitelist_is_exactly_the_methods_fixture() {
        let body: serde_json::Value =
            serde_json::from_str(&fixture("methods.json")).expect("methods.json is valid JSON");
        let data = body
            .get("data")
            .and_then(|d| d.as_object())
            .expect("methods data is an object keyed by name");
        let mut ids: Vec<u32> = data
            .values()
            .filter_map(|method| {
                method
                    .get("id")
                    .and_then(|id| id.as_u64())
                    .map(|id| id as u32)
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();

        let whitelisted: Vec<u32> = (0..=999)
            .filter(|method| crate::config::validate::is_valid_method(*method))
            .collect();
        assert_eq!(
            ids, whitelisted,
            "is_valid_method and /v1/methods have drifted — see §7.4 correction note"
        );
    }
}
