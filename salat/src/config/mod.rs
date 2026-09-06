//! `Config` and all sub-structs (serde) — the §8.2 schema.
//!
//! Every field carries `#[serde(default)]` so a hand-edited file may
//! contain only the keys its author means to change; unknown keys are
//! still *rejected* (`deny_unknown_fields`), never silently ignored. The
//! one exception is `version`, which has no default: §8.4 makes it a
//! required field checked before any other validation.

pub mod defaults;
pub mod draft;
pub mod store;
pub mod validate;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use draft::ConfigDraft;
pub use store::{ConfigStore, Paths, resolve, scope_for};
pub use validate::Violation;

/// The only configuration schema version this build understands (§8.4).
/// A release that bumps this must ship a migration in the same release.
pub const CONFIG_VERSION: u32 = 1;

/// How the location was obtained — rendered as the Location screen's
/// active mode (§8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LocationSource {
    /// Chosen from the city search.
    #[default]
    City,
    /// Entered as manual coordinates.
    Coordinates,
    /// Detected automatically.
    Auto,
}

/// The twilight clarification used by method 15 (Umm al-Qura) (§8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Shafaq {
    /// `general` — the default AlAdhan behaviour.
    #[default]
    General,
    /// `red` — shafaq ahmar.
    Red,
    /// `white` — shafaq abyad.
    White,
}

/// Clock representation for all rendered times (§8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TimeFormat {
    /// `24h` — the design default.
    #[default]
    #[serde(rename = "24h")]
    TwentyFourHour,
    /// `12h`.
    #[serde(rename = "12h")]
    TwelveHour,
}

/// Which colour palette the renderer may use (§8.2, A11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    /// Detect the terminal's capability at runtime.
    #[default]
    Auto,
    /// 24-bit truecolour.
    Truecolor,
    /// The 256-colour palette.
    #[serde(rename = "256")]
    TwoHundredFiftySix,
    /// The 16-colour ANSI palette.
    Ansi,
}

/// The physical location the times are computed for (§8.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationConfig {
    /// Display string, Location screen "City".
    #[serde(default = "defaults::label")]
    pub label: String,
    #[serde(default = "defaults::country")]
    pub country: String,
    /// `-90..=90`; validated before any request because the API answers
    /// a bad coordinate with a fabricated 503 (§7.4).
    #[serde(default = "defaults::latitude")]
    pub latitude: f64,
    /// `-180..=180` (§7.4).
    #[serde(default = "defaults::longitude")]
    pub longitude: f64,
    /// IANA name, must parse via chrono-tz (§8.4).
    #[serde(default = "defaults::timezone")]
    pub timezone: String,
    #[serde(default)]
    pub source: LocationSource,
}

/// The AlAdhan calculation parameters (§8.2, §7.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalculationConfig {
    /// AlAdhan method id, whitelisted client-side because the API
    /// silently returns wrong times for an unknown id (§7.4).
    #[serde(default = "defaults::method")]
    pub method: u32,
    /// `0` Shafi, `1` Hanafi.
    #[serde(default = "defaults::school")]
    pub school: u8,
    /// `1` MidNight, `2` OneSeventh, `3` AngleBased.
    #[serde(default = "defaults::latitude_adjustment_method")]
    pub latitude_adjustment_method: u8,
    /// `0` Standard, `1` Jafari.
    #[serde(default = "defaults::midnight_mode")]
    pub midnight_mode: u8,
    /// Only used when `method == 15` (§8.2).
    #[serde(default)]
    pub shafaq: Shafaq,
    #[serde(default)]
    pub tune: Tune,
}

/// Per-prayer minute adjustments, each `-60..=60` (§8.2, §7.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Tune {
    #[serde(default)]
    pub imsak: i8,
    #[serde(default)]
    pub fajr: i8,
    #[serde(default)]
    pub sunrise: i8,
    #[serde(default)]
    pub dhuhr: i8,
    #[serde(default)]
    pub asr: i8,
    #[serde(default)]
    pub maghrib: i8,
    #[serde(default)]
    pub sunset: i8,
    #[serde(default)]
    pub isha: i8,
    #[serde(default)]
    pub midnight: i8,
}

/// Desktop notification settings (§8.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationsConfig {
    /// Settings → Enable notifications.
    #[serde(default = "defaults::yes")]
    pub enabled: bool,
    /// Settings → Mosque reminder.
    #[serde(default = "defaults::yes")]
    pub mosque_reminder: bool,
}

/// Adhan playback settings (§8.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdhanConfig {
    /// Settings → Enable adhan.
    #[serde(default = "defaults::yes")]
    pub enabled: bool,
    /// Resolves to `$XDG_DATA_HOME/salat/adhan/<preset>.mp3`.
    #[serde(default = "defaults::preset")]
    pub preset: String,
    /// Absolute path; overrides the preset when non-empty, and must then
    /// be an existing regular file (§8.4). Empty means "use the preset".
    #[serde(default)]
    pub file: PathBuf,
    /// `0..=100`.
    #[serde(default = "defaults::volume")]
    pub volume: u8,
}

/// Display settings (§8.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayConfig {
    #[serde(default)]
    pub time_format: TimeFormat,
    /// A9: single variant.
    #[serde(default = "defaults::language")]
    pub language: String,
    /// A9: single variant.
    #[serde(default = "defaults::theme")]
    pub theme: String,
    /// Settings → Compact mode.
    #[serde(default)]
    pub compact_mode: bool,
    /// Settings → Show seconds.
    #[serde(default = "defaults::yes")]
    pub show_seconds: bool,
    /// Settings → Countdown progress.
    #[serde(default = "defaults::yes")]
    pub countdown_progress: bool,
    #[serde(default)]
    pub color_mode: ColorMode,
}

/// Per-prayer scheduling offsets, one table per Notifiable prayer (§8.2,
/// §6.1).
///
/// A missing `[prayers.<name>]` table takes that prayer's exact §8.3 pair
/// (see [`crate::config::defaults`]); a *partial* table fills missing keys
/// from [`PrayerSettings::default`] (`15`/`20`/`true`/`true`), because a
/// field-level serde default cannot know which prayer it belongs to.
/// A *partial* table fills its missing keys from this struct's `Default`
/// (`15`/`20`/`true`/`true`, defined in [`crate::config::defaults`]) via
/// the container-level serde default below.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PrayerSettings {
    /// `0..=60`.
    pub before_minutes: u8,
    /// `0..=60`.
    pub after_minutes: u8,
    pub adhan: bool,
    pub notify: bool,
}

/// The five Notifiable prayers (§6.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrayersConfig {
    #[serde(default = "defaults::prayer_fajr")]
    pub fajr: PrayerSettings,
    #[serde(default = "defaults::prayer_dhuhr")]
    pub dhuhr: PrayerSettings,
    #[serde(default = "defaults::prayer_asr")]
    pub asr: PrayerSettings,
    #[serde(default = "defaults::prayer_maghrib")]
    pub maghrib: PrayerSettings,
    #[serde(default = "defaults::prayer_isha")]
    pub isha: PrayerSettings,
}

/// The whole `config.toml` (§8.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Schema version. Required: no default, checked before any other
    /// validation (§8.4).
    pub version: u32,
    #[serde(default)]
    pub location: LocationConfig,
    #[serde(default)]
    pub calculation: CalculationConfig,
    #[serde(default)]
    pub notifications: NotificationsConfig,
    #[serde(default)]
    pub adhan: AdhanConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub prayers: PrayersConfig,
}

/// Everything that can go wrong loading or saving a configuration
/// (§8.4–8.5), carrying the detail lines the E201/E202 screens print.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file exists but could not be read.
    #[error("cannot read the configuration file: {0}")]
    Read(std::io::Error),
    /// The TOML did not parse, or a key or structure is unknown — the
    /// `toml` error names the line and the offending key.
    #[error("{0}")]
    Parse(toml::de::Error),
    /// The configuration could not be serialized.
    #[error("cannot serialize the configuration: {0}")]
    Serialize(toml::ser::Error),
    /// `version` is absent from the file.
    #[error("missing field `version`")]
    MissingVersion,
    /// `version` is present but not a positive integer.
    #[error("`version` must be a positive integer")]
    VersionNotInteger,
    /// `version` is a positive integer this build does not understand.
    #[error("configuration version {0}")]
    Version(i64),
    /// Semantic validation failed — all violations, not the first (§8.4).
    /// Constructed explicitly (not `#[from]`): `Vec<Violation>` is not an
    /// error source, only a payload.
    #[error("configuration invalid")]
    Invalid(Vec<Violation>),
    /// The file (or its directory) could not be written.
    #[error("cannot write the configuration file: {0}")]
    Write(std::io::Error),
}

impl ConfigError {
    /// The `→` detail lines for the E201/E202 output, one per problem.
    pub fn detail_lines(&self) -> Vec<String> {
        match self {
            ConfigError::Read(e) => vec![format!("cannot read the configuration file: {e}")],
            ConfigError::Parse(e) => vec![format!("{e}")],
            ConfigError::Serialize(e) => vec![format!("cannot serialize the configuration: {e}")],
            ConfigError::MissingVersion => vec!["missing field `version`".into()],
            ConfigError::VersionNotInteger => {
                vec!["`version` must be a positive integer".into()]
            }
            ConfigError::Version(v) if *v > i64::from(CONFIG_VERSION) => vec![format!(
                "configuration version {v} was written by a newer salat; \
                 this build understands version {CONFIG_VERSION}"
            )],
            ConfigError::Version(v) => vec![format!(
                "`version` must be a positive integer this build understands \
                 ({CONFIG_VERSION}); found {v}"
            )],
            ConfigError::Invalid(violations) => violations.iter().map(|v| v.to_string()).collect(),
            ConfigError::Write(e) => vec![format!("cannot write the configuration file: {e}")],
        }
    }

    /// Map this error onto the process-boundary `SalatError` (§18.1) at
    /// the point where the file path is known. Parse/version/semantic
    /// failures become `E201`, write failures become `E202` (§14.5: both
    /// exit 3).
    pub fn into_salat_error(self, path: &Path) -> crate::error::SalatError {
        match self {
            ConfigError::Write(e) => crate::error::SalatError::ConfigWriteFailed {
                path: path.to_path_buf(),
                cause: e.to_string(),
            },
            ConfigError::Serialize(e) => crate::error::SalatError::ConfigWriteFailed {
                path: path.to_path_buf(),
                cause: e.to_string(),
            },
            other => crate::error::SalatError::ConfigInvalid {
                problems: other.detail_lines(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A partial file fills every missing key with its §8.3 default —
    /// only `version` is required (§8.4).
    #[test]
    fn partial_files_merge_with_defaults() {
        let config: Config = toml::from_str(
            "\
version = 1

[adhan]
volume = 30
",
        )
        .unwrap();
        let expected = Config {
            adhan: AdhanConfig {
                volume: 30,
                ..Config::default().adhan
            },
            ..Config::default()
        };
        assert_eq!(config, expected);
    }

    /// Serialize → deserialize is the identity, and the serialized form
    /// carries every §8.2 table.
    #[test]
    fn serialization_round_trips() {
        let config = Config::default();
        let text = toml::to_string_pretty(&config).unwrap();
        for table in [
            "[location]",
            "[calculation]",
            "[notifications]",
            "[adhan]",
            "[display]",
        ] {
            assert!(text.contains(table), "missing {table} in:\n{text}");
        }
        let reloaded: Config = toml::from_str(&text).unwrap();
        assert_eq!(reloaded, config);
    }

    /// An unknown enum value is a structural error naming the key (§8.4).
    #[test]
    fn unknown_enum_value_names_the_key() {
        let err = toml::from_str::<Config>(
            "\
version = 1

[display]
time_format = \"25h\"
",
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("time_format"), "missing key in: {message}");
        assert!(
            message.contains("24h") && message.contains("12h"),
            "expected variants listed in: {message}"
        );
    }

    /// A partial `[prayers.<name>]` table fills missing keys from
    /// `PrayerSettings::default()` — a field-level serde default cannot
    /// know which prayer it belongs to (see the struct docs).
    #[test]
    fn partial_prayer_tables_fill_from_the_struct_default() {
        let config: Config = toml::from_str(
            "\
version = 1

[prayers.maghrib]
adhan = false
",
        )
        .unwrap();
        assert_eq!(
            config.prayers.maghrib,
            PrayerSettings {
                before_minutes: 15,
                after_minutes: 20,
                adhan: false,
                notify: true,
            }
        );
        // A missing table entirely takes the prayer's exact §8.3 pair.
        assert_eq!(config.prayers.dhuhr.before_minutes, 10);
        assert_eq!(config.prayers.dhuhr.after_minutes, 30);
    }
}
