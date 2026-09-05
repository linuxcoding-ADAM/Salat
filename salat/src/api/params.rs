//! Query construction + client-side validation (§7.2, §7.4).
//!
//! `Params` is the single source of every query string AlAdhan sees. It is
//! built from the effective `Config` (§8) and validated **before any
//! request is sent**, because two verified API behaviours make that a
//! correctness requirement, not politeness: an out-of-range latitude
//! answers a fabricated HTTP 503 (§7.4), and an unknown method id answers
//! HTTP 200 with silently wrong times — `method=6` is even silently
//! remapped to ISNA (§7.4). The config layer already validates all of
//! this on load (§8.4); this second gate exists because `Params` is the
//! last thing between the process and the network.

use crate::config::validate::is_valid_method;
use crate::config::{CalculationConfig, Config, LocationConfig, Shafaq, Tune};

/// Which §7.4 rule a parameter violation trips — it decides the
/// user-facing error: coordinates → `E101`, method → `E102`, everything
/// else → `E201` (a parameter the config schema itself should have
/// caught; seeing it at runtime means a gate is missing somewhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// `latitude`/`longitude` outside their range.
    Coordinates,
    /// `method` not in the `/v1/methods` set.
    Method,
    /// Any other parameter domain violation.
    Config,
}

/// One invalid parameter: its rule family and a message naming the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidParam {
    /// The rule family, for the `E101`/`E102`/`E201` mapping.
    pub kind: ParamKind,
    /// What is wrong, including the offending value.
    pub message: String,
}

impl InvalidParam {
    fn new(kind: ParamKind, message: String) -> Self {
        Self { kind, message }
    }
}

/// The canonical §7.2 request parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    pub latitude: f64,
    pub longitude: f64,
    pub method: u32,
    /// `0` Shafi, `1` Hanafi.
    pub school: u8,
    /// `1` MidNight, `2` OneSeventh, `3` AngleBased.
    pub latitude_adjustment_method: u8,
    /// `0` Standard, `1` Jafari.
    pub midnight_mode: u8,
    /// Sent only when `method == 15` (§7.2); `Shafaq`'s serde names
    /// (`red`/`white`) deliberately differ from the API's
    /// (`ahmer`/`abyad`) — [`Params::shafaq_value`] does the translation.
    pub shafaq: Shafaq,
    /// IANA name, sent as `timezonestring`.
    pub timezone: String,
    pub tune: Tune,
}

/// §7.4 coordinates rule on its own: `qibla` validates lat/lng the same
/// way `Params::validate` does, without needing a full parameter set.
pub fn validate_coordinates(latitude: f64, longitude: f64) -> Result<(), Vec<InvalidParam>> {
    let mut problems = Vec::new();
    if !(-90.0..=90.0).contains(&latitude) {
        problems.push(InvalidParam::new(
            ParamKind::Coordinates,
            format!("latitude must be between -90 and 90 (found {latitude})"),
        ));
    }
    if !(-180.0..=180.0).contains(&longitude) {
        problems.push(InvalidParam::new(
            ParamKind::Coordinates,
            format!("longitude must be between -180 and 180 (found {longitude})"),
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

impl Params {
    /// Build from the two config sections that carry every parameter.
    pub fn from_parts(location: &LocationConfig, calc: &CalculationConfig) -> Self {
        Self {
            latitude: location.latitude,
            longitude: location.longitude,
            method: calc.method,
            school: calc.school,
            latitude_adjustment_method: calc.latitude_adjustment_method,
            midnight_mode: calc.midnight_mode,
            shafaq: calc.shafaq,
            timezone: location.timezone.clone(),
            tune: calc.tune.clone(),
        }
    }

    /// §7.4 client-side validation: every rule, all violations together —
    /// the same shape as `Config::validate` (§8.4).
    pub fn validate(&self) -> Result<(), Vec<InvalidParam>> {
        let mut problems = Vec::new();
        if let Err(coordinate_problems) = validate_coordinates(self.latitude, self.longitude) {
            problems.extend(coordinate_problems);
        }
        if !is_valid_method(self.method) {
            problems.push(InvalidParam::new(
                ParamKind::Method,
                format!(
                    "method {} is not a recognised AlAdhan method id",
                    self.method
                ),
            ));
        }
        if self.school > 1 {
            problems.push(InvalidParam::new(
                ParamKind::Config,
                format!(
                    "school must be 0 (Shafi) or 1 (Hanafi) (found {})",
                    self.school
                ),
            ));
        }
        if !(1..=3).contains(&self.latitude_adjustment_method) {
            problems.push(InvalidParam::new(
                ParamKind::Config,
                format!(
                    "latitudeAdjustmentMethod must be 1, 2 or 3 (found {})",
                    self.latitude_adjustment_method
                ),
            ));
        }
        if self.midnight_mode > 1 {
            problems.push(InvalidParam::new(
                ParamKind::Config,
                format!(
                    "midnightMode must be 0 (Standard) or 1 (Jafari) (found {})",
                    self.midnight_mode
                ),
            ));
        }
        for (name, value) in [
            ("imsak", self.tune.imsak),
            ("fajr", self.tune.fajr),
            ("sunrise", self.tune.sunrise),
            ("dhuhr", self.tune.dhuhr),
            ("asr", self.tune.asr),
            ("maghrib", self.tune.maghrib),
            ("sunset", self.tune.sunset),
            ("isha", self.tune.isha),
            ("midnight", self.tune.midnight),
        ] {
            if !(-60..=60).contains(&value) {
                problems.push(InvalidParam::new(
                    ParamKind::Config,
                    format!("tune.{name} must be between -60 and 60 minutes (found {value})"),
                ));
            }
        }
        if self.timezone.parse::<chrono_tz::Tz>().is_err() {
            problems.push(InvalidParam::new(
                ParamKind::Config,
                format!(
                    "timezonestring must be an IANA timezone name (found \"{}\")",
                    self.timezone
                ),
            ));
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    /// The §7.2 query pairs, in the documented order. `iso8601=true` is
    /// on **every** request — it is what makes every timing an RFC 3339
    /// timestamp with the day's real UTC offset, and what avoids the
    /// `"05:12 (CET)"` suffix trap (§7.2). The caller URL-encodes them.
    pub fn query_pairs(&self) -> Vec<(&'static str, String)> {
        let mut pairs = vec![
            ("latitude", format!("{}", self.latitude)),
            ("longitude", format!("{}", self.longitude)),
            ("method", self.method.to_string()),
            ("school", self.school.to_string()),
            (
                "latitudeAdjustmentMethod",
                self.latitude_adjustment_method.to_string(),
            ),
            ("midnightMode", self.midnight_mode.to_string()),
            ("timezonestring", self.timezone.clone()),
            ("tune", self.tune_string()),
            ("iso8601", "true".to_owned()),
        ];
        // §7.2: shafaq rides along only for Umm al-Qura.
        if let Some(shafaq) = self.shafaq_value() {
            pairs.push(("shafaq", shafaq.to_owned()));
        }
        pairs
    }

    /// `tune` is one comma-separated list of nine integers in the §7.2
    /// order: `imsak,fajr,sunrise,dhuhr,asr,maghrib,sunset,isha,midnight`.
    fn tune_string(&self) -> String {
        let t = &self.tune;
        [
            t.imsak, t.fajr, t.sunrise, t.dhuhr, t.asr, t.maghrib, t.sunset, t.isha, t.midnight,
        ]
        .iter()
        .map(|minutes| minutes.to_string())
        .collect::<Vec<_>>()
        .join(",")
    }

    /// The API's shafaq spelling, or `None` when the method is not 15 —
    /// sending `shafaq` with another method is at best ignored and at
    /// worst another silent-remap hazard, so it is simply omitted.
    fn shafaq_value(&self) -> Option<&'static str> {
        if self.method != 15 {
            return None;
        }
        match self.shafaq {
            Shafaq::General => Some("general"),
            Shafaq::Red => Some("ahmer"),
            Shafaq::White => Some("abyad"),
        }
    }
}

impl From<&Config> for Params {
    fn from(config: &Config) -> Self {
        Self::from_parts(&config.location, &config.calculation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_params() -> Params {
        Params::from(&Config::default())
    }

    /// The §8.3 defaults produce exactly the §7.2 canonical request.
    #[test]
    fn default_config_yields_the_canonical_query() {
        let params = default_params();
        let pairs = params.query_pairs();
        let get = |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.as_str())
                .unwrap_or_default()
        };
        assert_eq!(get("latitude"), "36.19");
        assert_eq!(get("longitude"), "5.41");
        assert_eq!(get("method"), "12");
        assert_eq!(get("school"), "0");
        assert_eq!(get("latitudeAdjustmentMethod"), "3");
        assert_eq!(get("midnightMode"), "0");
        assert_eq!(get("timezonestring"), "Africa/Algiers");
        // §23 Phase 2 item 8: nine comma-separated integers, zeros included.
        assert_eq!(get("tune"), "0,0,0,0,0,0,0,0,0");
        // §7.2: mandatory on every request.
        assert_eq!(get("iso8601"), "true");
        // Method 12 sends no shafaq (§7.2: only when method == 15).
        assert!(pairs.iter().all(|(k, _)| *k != "shafaq"));
        assert_eq!(params.validate(), Ok(()));
    }

    /// Non-zero tune values serialize in the §7.2 order, negatives
    /// included.
    #[test]
    fn tune_serializes_in_the_documented_order() {
        let mut params = default_params();
        params.tune = Tune {
            imsak: -1,
            fajr: 2,
            sunrise: -3,
            dhuhr: 4,
            asr: -5,
            maghrib: 6,
            sunset: -7,
            isha: 8,
            midnight: -9,
        };
        let pairs = params.query_pairs();
        let tune = pairs
            .iter()
            .find(|(k, _)| *k == "tune")
            .map(|(_, v)| v.as_str())
            .unwrap_or_default();
        assert_eq!(tune, "-1,2,-3,4,-5,6,-7,8,-9");
    }

    /// `shafaq` is sent only for method 15, with the API's spelling
    /// (`ahmer`/`abyad`), not the config file's (`red`/`white`).
    #[test]
    fn shafaq_rides_only_with_method_15_in_api_spelling() {
        let mut params = default_params();
        params.method = 15;
        params.shafaq = Shafaq::Red;
        let pairs = params.query_pairs();
        let shafaq = pairs
            .iter()
            .find(|(k, _)| *k == "shafaq")
            .map(|(_, v)| v.as_str())
            .unwrap_or_default();
        assert_eq!(shafaq, "ahmer");

        params.shafaq = Shafaq::White;
        let pairs = params.query_pairs();
        let shafaq = pairs
            .iter()
            .find(|(k, _)| *k == "shafaq")
            .map(|(_, v)| v.as_str())
            .unwrap_or_default();
        assert_eq!(shafaq, "abyad");

        // Not method 15: no shafaq pair at all, whatever the config says.
        params.method = 12;
        assert!(params.query_pairs().iter().all(|(k, _)| *k != "shafaq"));
    }

    /// §7.4: every rule fires, and the two E101/E102 families are
    /// distinguishable for the error mapping.
    #[test]
    fn every_violation_is_reported_with_its_kind() {
        let mut params = default_params();
        params.latitude = 999.0;
        params.longitude = -999.0;
        params.method = 6; // silently remapped to ISNA — must never be sent
        params.school = 2;
        params.latitude_adjustment_method = 0;
        params.midnight_mode = 7;
        params.tune.fajr = 61;
        params.timezone = "Nowhere/Nothing".into();

        let problems = params.validate().unwrap_err();
        let coords = problems
            .iter()
            .filter(|p| p.kind == ParamKind::Coordinates)
            .count();
        let methods = problems
            .iter()
            .filter(|p| p.kind == ParamKind::Method)
            .count();
        let config = problems
            .iter()
            .filter(|p| p.kind == ParamKind::Config)
            .count();
        assert_eq!(coords, 2, "latitude + longitude: {problems:?}");
        assert_eq!(methods, 1, "method: {problems:?}");
        // school, lam, midnight, tune.fajr, timezone.
        assert_eq!(config, 5, "{problems:?}");
    }

    /// `method=777` — the §7.4 silent-wrong-times specimen — is a
    /// `ParamKind::Method` violation.
    #[test]
    fn method_777_is_rejected_as_a_method_problem() {
        let mut params = default_params();
        params.method = 777;
        let problems = params.validate().unwrap_err();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].kind, ParamKind::Method);
        assert!(problems[0].message.contains("777"));
    }

    /// `qibla` shares the §7.4 coordinates rule through
    /// [`validate_coordinates`], without a full `Params`.
    #[test]
    fn coordinate_validation_stands_alone_for_qibla() {
        assert_eq!(validate_coordinates(36.19, 5.41), Ok(()));
        let problems = validate_coordinates(999.0, -999.0).unwrap_err();
        assert_eq!(problems.len(), 2);
        assert!(problems.iter().all(|p| p.kind == ParamKind::Coordinates));
    }
}
