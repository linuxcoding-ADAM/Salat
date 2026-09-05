//! DTO → domain mapping (§7.6): one calendar day as it came off the wire
//! becomes one pure `PrayerDay`.
//!
//! This is the only place the api and domain types touch. The mapping is
//! total except for two echoes the API could produce that have no domain
//! meaning: a timezone name chrono-tz does not know, and a school string
//! that is neither `STANDARD` nor `HANAFI`. Both are `Malformed` — the
//! day is not trustworthy, so nothing is invented to stand in for it.

use chrono_tz::Tz;

use crate::domain::{DayMeta, HijriDate, PrayerDay};

use super::ApiError;
use super::dto::CalendarDayDto;

/// Map one parsed calendar day to its domain form (§7.6).
///
/// The timings are converted from the wire's fixed offset into the
/// location's zone — an instant-preserving conversion, not a wall-clock
/// copy — because every derived value below the domain assumes the day's
/// own `tz`. `midnight` and `last_third` may land on the next civil day
/// (§7.2); that is the API's answer and is carried through untouched.
pub fn to_prayer_day(dto: &CalendarDayDto) -> Result<PrayerDay, ApiError> {
    let tz: Tz = dto.meta.timezone.parse().map_err(|_| {
        ApiError::Malformed(format!(
            "timezone \"{}\" is not a zone chrono-tz knows (§7.6)",
            dto.meta.timezone
        ))
    })?;
    let school = match dto.meta.school.as_str() {
        "STANDARD" => 0,
        "HANAFI" => 1,
        other => {
            return Err(ApiError::Malformed(format!(
                "school {other:?} is neither STANDARD nor HANAFI (§7.6)"
            )));
        }
    };
    // Indexed by `Prayer` order (§6.2): Fajr, Sunrise, Dhuhr, Asr,
    // Maghrib, Isha. Imsak and Sunset ride along nowhere — the domain
    // never needed them.
    let times = [
        dto.times.fajr.with_timezone(&tz),
        dto.times.sunrise.with_timezone(&tz),
        dto.times.dhuhr.with_timezone(&tz),
        dto.times.asr.with_timezone(&tz),
        dto.times.maghrib.with_timezone(&tz),
        dto.times.isha.with_timezone(&tz),
    ];
    Ok(PrayerDay {
        date: dto.gregorian_date,
        tz,
        times,
        midnight: dto.times.midnight.with_timezone(&tz),
        last_third: dto.times.last_third.with_timezone(&tz),
        hijri: HijriDate {
            day: dto.hijri.day,
            month_number: dto.hijri.month,
            month_en: dto.hijri.month_name.clone(),
            month_ar: dto.hijri.month_name_ar.clone(),
            year: dto.hijri.year,
        },
        meta: DayMeta {
            method_id: dto.meta.method_id,
            method_name: dto.meta.method_name.clone(),
            school,
            lat: dto.meta.latitude,
            lng: dto.meta.longitude,
            tz_name: dto.meta.timezone.clone(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Notifiable, Prayer};
    use chrono::NaiveDate;

    fn calendar_body() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/aladhan")
            .join("calendar_ok.json");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture calendar_ok.json: {e}"))
    }

    fn calendar_fixture() -> Vec<CalendarDayDto> {
        super::super::dto::parse_calendar(&calendar_body()).expect("the calendar fixture parses")
    }

    /// The calendar fixture with a string edit applied — the error tests
    /// rewrite one wire field and re-parse.
    fn edited_calendar(from: &str, to: &str) -> Vec<CalendarDayDto> {
        let body = calendar_body().replace(from, to);
        super::super::dto::parse_calendar(&body).expect("the edit keeps it parseable")
    }

    /// Days[4] — 2026-09-05 — maps field for field, with midnight and the
    /// last third on the next civil day (§7.2).
    #[test]
    fn the_sep_5_day_maps_to_a_prayer_day() {
        let days = calendar_fixture();
        let day = to_prayer_day(&days[4]).expect("a well-formed day maps");
        assert_eq!(day.date, NaiveDate::from_ymd_opt(2026, 9, 5).unwrap());
        assert_eq!(day.tz, chrono_tz::Africa::Algiers);
        assert_eq!(
            day.time(Prayer::Fajr).to_rfc3339(),
            "2026-09-05T05:16:00+01:00"
        );
        assert_eq!(
            day.time(Prayer::Sunrise).to_rfc3339(),
            "2026-09-05T06:13:00+01:00"
        );
        assert_eq!(
            day.time(Prayer::Dhuhr).to_rfc3339(),
            "2026-09-05T12:37:00+01:00"
        );
        assert_eq!(
            day.time(Prayer::Asr).to_rfc3339(),
            "2026-09-05T16:14:00+01:00"
        );
        assert_eq!(
            day.time(Prayer::Maghrib).to_rfc3339(),
            "2026-09-05T19:01:00+01:00"
        );
        assert_eq!(
            day.time(Prayer::Isha).to_rfc3339(),
            "2026-09-05T19:57:00+01:00"
        );
        assert_eq!(day.midnight.to_rfc3339(), "2026-09-06T00:37:00+01:00");
        assert_eq!(day.last_third.to_rfc3339(), "2026-09-06T02:29:00+01:00");

        assert_eq!(
            day.hijri,
            HijriDate {
                day: 23,
                month_number: 3,
                month_en: "Rabīʿ al-awwal".to_owned(),
                month_ar: "رَبيع الأوّل".to_owned(),
                year: 1448,
            }
        );
        assert_eq!(
            day.meta,
            DayMeta {
                method_id: 12,
                method_name: "Union Organization Islamic de France".to_owned(),
                school: 0,
                lat: 36.19,
                lng: 5.41,
                tz_name: "Africa/Algiers".to_owned(),
            }
        );
        // The instant tags carry the day's civil date (§6.3).
        assert_eq!(day.instant(Notifiable::Maghrib).date, day.date);
    }

    /// The offset→zone conversion is instant-preserving: the wire's
    /// `+01:00` reading and the mapped `Africa/Algiers` reading are the
    /// same moment.
    #[test]
    fn the_conversion_preserves_instants() {
        let days = calendar_fixture();
        let dto = &days[4];
        let day = to_prayer_day(dto).expect("a well-formed day maps");
        let wire = &dto.times;
        assert_eq!(day.time(Prayer::Fajr).timestamp(), wire.fajr.timestamp());
        assert_eq!(day.time(Prayer::Isha).timestamp(), wire.isha.timestamp());
        assert_eq!(day.midnight.timestamp(), wire.midnight.timestamp());
    }

    /// A whole month maps: thirty days, every date one day later than the
    /// last, none rejected.
    #[test]
    fn the_whole_month_maps_in_order() {
        let days = calendar_fixture();
        let mapped: Vec<PrayerDay> = days
            .iter()
            .map(to_prayer_day)
            .collect::<Result<_, _>>()
            .expect("every fixture day maps");
        assert_eq!(mapped.len(), 30);
        for pair in mapped.windows(2) {
            assert!(
                pair[0].date.succ_opt() == Some(pair[1].date),
                "dates must be consecutive: {} then {}",
                pair[0].date,
                pair[1].date
            );
        }
    }

    /// `HANAFI` maps to the domain's `1`.
    #[test]
    fn hanafi_maps_to_one() {
        let days = edited_calendar("\"school\":\"STANDARD\"", "\"school\":\"HANAFI\"");
        let day = to_prayer_day(&days[4]).expect("HANAFI is a school the domain knows");
        assert_eq!(day.meta.school, 1);
    }

    /// A school string the domain does not know is `Malformed` — nothing
    /// is guessed.
    #[test]
    fn an_unknown_school_is_malformed() {
        let days = edited_calendar("\"school\":\"STANDARD\"", "\"school\":\"SHAFI\"");
        let error = to_prayer_day(&days[4]).unwrap_err();
        match &error {
            ApiError::Malformed(message) => {
                assert!(message.contains("school"), "{message}");
                assert!(message.contains("SHAFI"), "{message}");
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    /// A timezone chrono-tz does not know is `Malformed` — the day is not
    /// trustworthy, so it is not mapped.
    #[test]
    fn an_unknown_timezone_is_malformed() {
        let days = edited_calendar(
            "\"timezone\":\"Africa/Algiers\"",
            "\"timezone\":\"Mars/Olympus\"",
        );
        let error = to_prayer_day(&days[4]).unwrap_err();
        match &error {
            ApiError::Malformed(message) => {
                assert!(message.contains("Mars/Olympus"), "{message}");
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }
}
