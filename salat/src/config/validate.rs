//! Semantic validation reporting all violations together (§8.4).
//!
//! Structural problems (unknown keys, wrong types) are serde's, surfaced
//! as `ConfigError::Parse` during deserialization; this module owns the
//! value-level rules the TUI's own editors make unreachable — it exists
//! to defend against hand-editing, and it tells a hand-editing user
//! *everything* that is wrong in one pass.

use std::fmt;
use std::path::Path;

use chrono_tz::Tz;

use super::{
    AdhanConfig, CalculationConfig, Config, DisplayConfig, LocationConfig, PrayerSettings,
};

/// The AlAdhan method ids this build will send (§7.4): the verified set
/// from `/v1/methods` — recorded in
/// `tests/fixtures/aladhan/methods.json`, and pinned to this function by
/// an `api::dto` test. Anything else either fails or is silently remapped
/// (`method=6` answers as ISNA), so an unknown id is rejected before any
/// request.
pub fn is_valid_method(method: u32) -> bool {
    matches!(method, 0..=5 | 7..=23 | 99)
}

/// One semantic violation: the dotted key that is wrong, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The dotted path of the offending key, e.g. `location.latitude`.
    pub field: String,
    /// What is wrong with it, including the offending value.
    pub message: String,
}

impl Violation {
    fn new(field: &str, message: String) -> Self {
        Self {
            field: field.into(),
            message,
        }
    }
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

impl Config {
    /// Semantic validation (§8.4): `Ok(())` or **all** violations, never
    /// just the first.
    pub fn validate(&self) -> Result<(), Vec<Violation>> {
        let mut violations = Vec::new();
        validate_location(&self.location, &mut violations);
        validate_calculation(&self.calculation, &mut violations);
        validate_adhan(&self.adhan, &mut violations);
        validate_display(&self.display, &mut violations);
        validate_prayer("fajr", &self.prayers.fajr, &mut violations);
        validate_prayer("dhuhr", &self.prayers.dhuhr, &mut violations);
        validate_prayer("asr", &self.prayers.asr, &mut violations);
        validate_prayer("maghrib", &self.prayers.maghrib, &mut violations);
        validate_prayer("isha", &self.prayers.isha, &mut violations);
        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }
}

fn validate_location(location: &LocationConfig, violations: &mut Vec<Violation>) {
    if !(-90.0..=90.0).contains(&location.latitude) {
        violations.push(Violation::new(
            "location.latitude",
            format!("must be between -90 and 90 (found {})", location.latitude),
        ));
    }
    if !(-180.0..=180.0).contains(&location.longitude) {
        violations.push(Violation::new(
            "location.longitude",
            format!(
                "must be between -180 and 180 (found {})",
                location.longitude
            ),
        ));
    }
    if location.timezone.parse::<Tz>().is_err() {
        violations.push(Violation::new(
            "location.timezone",
            format!(
                "must be an IANA timezone name (found \"{}\")",
                location.timezone
            ),
        ));
    }
}

fn validate_calculation(calc: &CalculationConfig, violations: &mut Vec<Violation>) {
    if !is_valid_method(calc.method) {
        violations.push(Violation::new(
            "calculation.method",
            format!(
                "must be a recognised AlAdhan method id (found {})",
                calc.method
            ),
        ));
    }
    if calc.school > 1 {
        violations.push(Violation::new(
            "calculation.school",
            format!("must be 0 (Shafi) or 1 (Hanafi) (found {})", calc.school),
        ));
    }
    if !(1..=3).contains(&calc.latitude_adjustment_method) {
        violations.push(Violation::new(
            "calculation.latitude_adjustment_method",
            format!(
                "must be 1 (MidNight), 2 (OneSeventh) or 3 (AngleBased) (found {})",
                calc.latitude_adjustment_method
            ),
        ));
    }
    if calc.midnight_mode > 1 {
        violations.push(Violation::new(
            "calculation.midnight_mode",
            format!(
                "must be 0 (Standard) or 1 (Jafari) (found {})",
                calc.midnight_mode
            ),
        ));
    }
    for (name, value) in [
        ("imsak", calc.tune.imsak),
        ("fajr", calc.tune.fajr),
        ("sunrise", calc.tune.sunrise),
        ("dhuhr", calc.tune.dhuhr),
        ("asr", calc.tune.asr),
        ("maghrib", calc.tune.maghrib),
        ("sunset", calc.tune.sunset),
        ("isha", calc.tune.isha),
        ("midnight", calc.tune.midnight),
    ] {
        if !(-60..=60).contains(&value) {
            violations.push(Violation::new(
                &format!("calculation.tune.{name}"),
                format!("must be between -60 and 60 minutes (found {value})"),
            ));
        }
    }
}

fn validate_adhan(adhan: &AdhanConfig, violations: &mut Vec<Violation>) {
    if adhan.volume > 100 {
        violations.push(Violation::new(
            "adhan.volume",
            format!("must be between 0 and 100 (found {})", adhan.volume),
        ));
    }
    // §8.4: a non-empty `file` must be an absolute path to an existing
    // regular file; the empty string means "use the preset".
    if !adhan.file.as_os_str().is_empty() {
        let path = Path::new(&adhan.file);
        if !path.is_absolute() {
            violations.push(Violation::new(
                "adhan.file",
                format!(
                    "must be an absolute path when set (found \"{}\")",
                    adhan.file.display()
                ),
            ));
        } else {
            let is_regular_file = std::fs::metadata(path)
                .map(|m| m.is_file())
                .unwrap_or(false);
            if !is_regular_file {
                violations.push(Violation::new(
                    "adhan.file",
                    format!(
                        "must be an existing regular file (\"{}\" not found)",
                        adhan.file.display()
                    ),
                ));
            }
        }
    }
}

fn validate_display(display: &DisplayConfig, violations: &mut Vec<Violation>) {
    // A9: single variants — a hand-edited value the build cannot render
    // is rejected rather than silently ignored.
    if display.language != "en" {
        violations.push(Violation::new(
            "display.language",
            format!(
                "\"en\" is the only supported language (found \"{}\")",
                display.language
            ),
        ));
    }
    if display.theme != "midnight-blue" {
        violations.push(Violation::new(
            "display.theme",
            format!(
                "\"midnight-blue\" is the only supported theme (found \"{}\")",
                display.theme
            ),
        ));
    }
}

fn validate_prayer(name: &str, prayer: &PrayerSettings, violations: &mut Vec<Violation>) {
    if prayer.before_minutes > 60 {
        violations.push(Violation::new(
            &format!("prayers.{name}.before_minutes"),
            format!("must be between 0 and 60 (found {})", prayer.before_minutes),
        ));
    }
    if prayer.after_minutes > 60 {
        violations.push(Violation::new(
            &format!("prayers.{name}.after_minutes"),
            format!("must be between 0 and 60 (found {})", prayer.after_minutes),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The §8.3 defaults validate clean.
    #[test]
    fn defaults_are_valid() {
        assert_eq!(Config::default().validate(), Ok(()));
    }

    /// Every §8.4 rule is reported, together, with the dotted field name
    /// (§8.4: all violations, not the first).
    #[test]
    fn all_violations_are_reported_together() {
        let mut config = Config::default();
        config.location.latitude = 999.0;
        config.location.longitude = -999.0;
        config.location.timezone = "Not/AZone".into();
        config.calculation.method = 6; // silently remapped to ISNA (§7.4)
        config.calculation.school = 2;
        config.calculation.latitude_adjustment_method = 0;
        config.calculation.midnight_mode = 7;
        config.calculation.tune.fajr = 61;
        config.calculation.tune.isha = -61;
        config.adhan.volume = 150;
        config.adhan.file = "relative/path.mp3".into();
        config.display.language = "fr".into();
        config.display.theme = "solarized".into();
        config.prayers.fajr.before_minutes = 61;
        config.prayers.isha.after_minutes = 61;

        let violations = config.validate().unwrap_err();
        let fields: Vec<&str> = violations.iter().map(|v| v.field.as_str()).collect();
        for expected in [
            "location.latitude",
            "location.longitude",
            "location.timezone",
            "calculation.method",
            "calculation.school",
            "calculation.latitude_adjustment_method",
            "calculation.midnight_mode",
            "calculation.tune.fajr",
            "calculation.tune.isha",
            "adhan.volume",
            "adhan.file",
            "display.language",
            "display.theme",
            "prayers.fajr.before_minutes",
            "prayers.isha.after_minutes",
        ] {
            assert!(
                fields.contains(&expected),
                "missing {expected} in {fields:?}"
            );
        }
        // 15 distinct rules were broken; each is reported exactly once.
        assert_eq!(violations.len(), 15);
    }

    /// §7.4's whitelist shape: the contiguous ranges, the 6 gap (silently
    /// remapped to ISNA by the API), and 99.
    #[test]
    fn method_whitelist_matches_the_api() {
        for valid in [0, 5, 15, 16, 17, 20, 23, 99] {
            assert!(is_valid_method(valid), "{valid} must be valid");
        }
        for invalid in [6, 24, 77, 98, 100, 1000, 777] {
            assert!(!is_valid_method(invalid), "{invalid} must be invalid");
        }
    }

    /// A non-empty `adhan.file` that exists and is a regular file passes.
    #[test]
    fn adhan_file_accepts_existing_regular_file() {
        let mut config = Config::default();
        let file = tempfile::tempdir().unwrap().keep().join("adhan.mp3");
        std::fs::write(&file, b"").unwrap();
        config.adhan.file = file.clone();
        assert_eq!(config.validate(), Ok(()));

        // An absolute path that does not exist is a violation.
        config.adhan.file = file.join("missing.mp3");
        let violations = config.validate().unwrap_err();
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, "adhan.file");
        assert!(violations[0].message.contains("existing regular file"));
    }
}
