//! The design's mock data as defaults (§8.3), and the commented
//! `config.toml` written on first run.
//!
//! Defaults are the Figma design's own mock values so a fresh install
//! renders the screens the designer drew. `default_toml()` is the
//! fully-commented file a missing config is created from — its parsed
//! value is pinned to `Config::default()` by a test, so the template and
//! the code cannot drift.

use super::{
    AdhanConfig, CONFIG_VERSION, CalculationConfig, Config, DisplayConfig, LocationConfig,
    LocationSource, NotificationsConfig, PrayerSettings, PrayersConfig,
};

pub(crate) fn label() -> String {
    "Sétif, Algeria".into()
}

pub(crate) fn country() -> String {
    "Algeria".into()
}

pub(crate) fn latitude() -> f64 {
    36.1900
}

pub(crate) fn longitude() -> f64 {
    5.4100
}

pub(crate) fn timezone() -> String {
    "Africa/Algiers".into()
}

pub(crate) fn method() -> u32 {
    12
}

pub(crate) fn school() -> u8 {
    0
}

pub(crate) fn latitude_adjustment_method() -> u8 {
    3
}

pub(crate) fn midnight_mode() -> u8 {
    0
}

pub(crate) fn yes() -> bool {
    true
}

pub(crate) fn preset() -> String {
    "abdul-basit".into()
}

pub(crate) fn volume() -> u8 {
    80
}

pub(crate) fn language() -> String {
    "en".into()
}

pub(crate) fn theme() -> String {
    "midnight-blue".into()
}

/// A prayer's §8.3 pair with adhan and notify enabled.
fn prayer(before: u8, after: u8) -> PrayerSettings {
    PrayerSettings {
        before_minutes: before,
        after_minutes: after,
        adhan: true,
        notify: true,
    }
}

pub(crate) fn prayer_fajr() -> PrayerSettings {
    prayer(15, 20)
}

pub(crate) fn prayer_dhuhr() -> PrayerSettings {
    prayer(10, 30)
}

pub(crate) fn prayer_asr() -> PrayerSettings {
    prayer(15, 20)
}

pub(crate) fn prayer_maghrib() -> PrayerSettings {
    prayer(5, 15)
}

pub(crate) fn prayer_isha() -> PrayerSettings {
    prayer(15, 30)
}

/// The default per-key values inside a partial `[prayers.<name>]` table.
impl Default for PrayerSettings {
    fn default() -> Self {
        prayer(15, 20)
    }
}

impl Default for LocationConfig {
    fn default() -> Self {
        Self {
            label: label(),
            country: country(),
            latitude: latitude(),
            longitude: longitude(),
            timezone: timezone(),
            source: LocationSource::City,
        }
    }
}

impl Default for CalculationConfig {
    fn default() -> Self {
        Self {
            method: method(),
            school: school(),
            latitude_adjustment_method: latitude_adjustment_method(),
            midnight_mode: midnight_mode(),
            shafaq: Default::default(),
            tune: Default::default(),
        }
    }
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            enabled: yes(),
            mosque_reminder: yes(),
        }
    }
}

impl Default for AdhanConfig {
    fn default() -> Self {
        Self {
            enabled: yes(),
            preset: preset(),
            file: Default::default(),
            volume: volume(),
        }
    }
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            time_format: Default::default(),
            language: language(),
            theme: theme(),
            compact_mode: false,
            show_seconds: yes(),
            countdown_progress: yes(),
            color_mode: Default::default(),
        }
    }
}

impl Default for PrayersConfig {
    fn default() -> Self {
        Self {
            fajr: prayer_fajr(),
            dhuhr: prayer_dhuhr(),
            asr: prayer_asr(),
            maghrib: prayer_maghrib(),
            isha: prayer_isha(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            location: LocationConfig::default(),
            calculation: CalculationConfig::default(),
            notifications: NotificationsConfig::default(),
            adhan: AdhanConfig::default(),
            display: DisplayConfig::default(),
            prayers: PrayersConfig::default(),
        }
    }
}

/// The commented file written when no configuration exists (§8.3, §23
/// Phase 1 item 11). Mirrors §8.2's schema exactly; a test pins its
/// parsed value to `Config::default()`.
pub fn default_toml() -> &'static str {
    "\
version = 1                              # schema version, for future migrations

[location]
label      = \"Sétif, Algeria\"          # display string, Location screen \"City\"
country    = \"Algeria\"
latitude   = 36.1900                     # -90..=90
longitude  = 5.4100                      # -180..=180
timezone   = \"Africa/Algiers\"           # IANA, must parse via chrono-tz
source     = \"city\"                     # \"city\" | \"coordinates\" | \"auto\"

[calculation]
method                     = 12          # AlAdhan method id, whitelisted
school                     = 0           # 0 Shafi, 1 Hanafi
latitude_adjustment_method = 3           # 1 MidNight, 2 OneSeventh, 3 AngleBased
midnight_mode              = 0           # 0 Standard, 1 Jafari
shafaq                     = \"general\"   # only used when method == 15

[calculation.tune]                       # each -60..=60, minutes
imsak    = 0
fajr     = 0
sunrise  = 0
dhuhr    = 0
asr      = 0
maghrib  = 0
sunset   = 0
isha     = 0
midnight = 0

[notifications]
enabled         = true                   # Enable notifications
mosque_reminder = true                   # Mosque reminder

[adhan]
enabled = true                           # Enable adhan
preset  = \"abdul-basit\"                  # $XDG_DATA_HOME/salat/adhan/<preset>.mp3
file    = \"\"                             # absolute path; overrides preset when non-empty
volume  = 80                             # 0..=100

[display]
time_format        = \"24h\"               # \"24h\" | \"12h\"
language           = \"en\"                # single variant
theme              = \"midnight-blue\"     # single variant
compact_mode       = false               # Compact mode
show_seconds       = true                # Show seconds
countdown_progress = true                # Countdown progress
color_mode         = \"auto\"              # \"auto\" | \"truecolor\" | \"256\" | \"ansi\"

[prayers.fajr]
before_minutes = 15                      # 0..=60
after_minutes  = 20                      # 0..=60
adhan          = true
notify         = true

[prayers.dhuhr]
before_minutes = 10
after_minutes  = 30
adhan          = true
notify         = true

[prayers.asr]
before_minutes = 15
after_minutes  = 20
adhan          = true
notify         = true

[prayers.maghrib]
before_minutes = 5
after_minutes  = 15
adhan          = true
notify         = true

[prayers.isha]
before_minutes = 15
after_minutes  = 30
adhan          = true
notify         = true
"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The commented template and `Config::default()` are the same
    /// configuration — the pin that keeps the file and the code in sync
    /// (§8.3).
    #[test]
    fn default_toml_parses_to_the_default_config() {
        let parsed: Config = toml::from_str(default_toml()).unwrap();
        assert_eq!(parsed, Config::default());
    }

    /// The §8.3 values themselves, so a change to the design's mock data
    /// is a deliberate, visible act.
    #[test]
    fn defaults_are_the_designs_mock_data() {
        let config = Config::default();
        assert_eq!(config.version, 1);
        assert_eq!(config.location.label, "Sétif, Algeria");
        assert_eq!(config.location.country, "Algeria");
        assert_eq!(config.location.latitude, 36.1900);
        assert_eq!(config.location.longitude, 5.4100);
        assert_eq!(config.location.timezone, "Africa/Algiers");
        assert_eq!(config.calculation.method, 12);
        assert_eq!(config.calculation.school, 0);
        assert_eq!(config.calculation.latitude_adjustment_method, 3);
        assert_eq!(config.calculation.midnight_mode, 0);
        assert!(config.notifications.enabled);
        assert!(config.notifications.mosque_reminder);
        assert!(config.adhan.enabled);
        assert_eq!(config.adhan.preset, "abdul-basit");
        assert_eq!(config.adhan.file, PathBuf::from(""));
        assert_eq!(config.adhan.volume, 80);
        assert_eq!(config.display.language, "en");
        assert_eq!(config.display.theme, "midnight-blue");
        assert!(!config.display.compact_mode);
        assert!(config.display.show_seconds);
        assert!(config.display.countdown_progress);
        assert_eq!(config.prayers.fajr.before_minutes, 15);
        assert_eq!(config.prayers.fajr.after_minutes, 20);
        assert_eq!(config.prayers.dhuhr.before_minutes, 10);
        assert_eq!(config.prayers.dhuhr.after_minutes, 30);
        assert_eq!(config.prayers.asr.before_minutes, 15);
        assert_eq!(config.prayers.asr.after_minutes, 20);
        assert_eq!(config.prayers.maghrib.before_minutes, 5);
        assert_eq!(config.prayers.maghrib.after_minutes, 15);
        assert_eq!(config.prayers.isha.before_minutes, 15);
        assert_eq!(config.prayers.isha.after_minutes, 30);
    }
}
