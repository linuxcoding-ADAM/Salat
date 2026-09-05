//! `SalatError` — the user-facing error taxonomy (§18.1–18.2).
//!
//! One enum the ERROR screen renders, with the design-mandated code table:
//! fifteen codes whose meanings are final, plus a transient `NotImplemented`
//! variant (E000) that carries the Phase 0 command stubs so the binary
//! answers with a typed error instead of a panic. Message bodies beyond the
//! codes are filled in by the phases that implement each failure; the codes
//! and titles are the contract.

use std::fmt;
use std::process::ExitCode as ProcessExitCode;

/// The actions the ERROR screen offers for an error (§18.2).
///
/// The variant set mirrors the design's button vocabulary exactly; which
/// actions a given error shows is part of the code table below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorAction {
    /// `[R]` — attempt the failed step again.
    Retry,
    /// `[C]` — open the Location screen to set coordinates manually.
    Coordinates,
    /// `[D]` — proceed with default settings instead of the invalid ones.
    UseDefaults,
    /// `[Esc]` — return to the previous screen.
    Back,
}

impl ErrorAction {
    /// Lower-case name used by `SalatError::help` for non-design-specified
    /// help lines.
    pub fn name(self) -> &'static str {
        match self {
            ErrorAction::Retry => "retry",
            ErrorAction::Coordinates => "coordinates",
            ErrorAction::UseDefaults => "use defaults",
            ErrorAction::Back => "back",
        }
    }
}

/// The five process exit codes (§14.5).
///
/// Distinct codes matter for `salat daemon` under systemd, where
/// `Restart=on-failure` should retry `4` but not `3`, and for shell users
/// piping `salat --once` into a status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    /// Success — including a degraded or cache-only result.
    Ok = 0,
    /// Runtime failure: audio, IO, or the database (`E401`–`E403`, `E601`).
    Runtime = 1,
    /// Usage error — Clap's own exit code for a bad argument.
    Usage = 2,
    /// Configuration invalid or unwritable (`E201`, `E202`).
    Config = 3,
    /// Prayer data unavailable: network failed, no cached row (`E301`–`E304`).
    Network = 4,
}

impl From<ExitCode> for ProcessExitCode {
    fn from(code: ExitCode) -> Self {
        // The discriminants are the documented exit codes 0–4 (§14.5), so
        // the cast cannot produce an invalid `ExitCode`.
        ProcessExitCode::from(code as u8)
    }
}

/// The error the ERROR screen renders, and the only error that crosses the
/// process boundary (§18.1 layer 2).
#[derive(Debug, thiserror::Error)]
pub enum SalatError {
    /// `E001` — the city search / lookup failed.
    #[error("location lookup failed")]
    LocationLookupFailed,
    /// `E101` — coordinates outside the valid range.
    #[error("invalid coordinates")]
    InvalidCoordinates,
    /// `E102` — an unknown calculation method string.
    #[error("invalid calculation method")]
    InvalidMethod,
    /// `E103` — no timezone could be determined for the location.
    #[error("timezone not detected")]
    TimezoneUndetermined,
    /// `E201` — the configuration file failed semantic validation.
    #[error("configuration invalid")]
    ConfigInvalid,
    /// `E202` — the configuration file could not be written.
    #[error("could not save settings")]
    ConfigWriteFailed,
    /// `E301` — no prayer times for the requested day (network failed, no
    /// cached row).
    #[error("prayer times unavailable")]
    PrayerTimesUnavailable,
    /// `E302` — api.aladhan.com could not be reached.
    #[error("api unreachable")]
    ApiUnreachable,
    /// `E303` — the API answered with something the client cannot parse.
    #[error("unexpected api response")]
    ApiMalformed,
    /// `E304` — the API answered 429 and retries are exhausted.
    #[error("rate limited")]
    ApiRateLimited,
    /// `E401` — no supported audio player on `$PATH`.
    #[error("no audio player found")]
    AudioNoPlayer,
    /// `E402` — the configured Adhan file does not exist.
    #[error("adhan file missing")]
    AudioNoFile,
    /// `E403` — the player exited non-zero.
    #[error("adhan playback failed")]
    AudioPlaybackFailed,
    /// `E501` — notifications are unavailable (no D-Bus session).
    #[error("notifications unavailable")]
    NotificationsUnavailable,
    /// `E601` — the SQLite cache failed.
    #[error("database error")]
    DatabaseFailed,
    /// `E000` — the command's service arrives in a later phase (§23).
    /// Transient: exists only while phases land one at a time.
    #[error("{command} is not implemented yet")]
    NotImplemented { command: &'static str },
}

impl SalatError {
    /// The design-mandated code, e.g. `"E001"` (§18.2).
    pub fn code(&self) -> &'static str {
        match self {
            SalatError::NotImplemented { .. } => "E000",
            SalatError::LocationLookupFailed => "E001",
            SalatError::InvalidCoordinates => "E101",
            SalatError::InvalidMethod => "E102",
            SalatError::TimezoneUndetermined => "E103",
            SalatError::ConfigInvalid => "E201",
            SalatError::ConfigWriteFailed => "E202",
            SalatError::PrayerTimesUnavailable => "E301",
            SalatError::ApiUnreachable => "E302",
            SalatError::ApiMalformed => "E303",
            SalatError::ApiRateLimited => "E304",
            SalatError::AudioNoPlayer => "E401",
            SalatError::AudioNoFile => "E402",
            SalatError::AudioPlaybackFailed => "E403",
            SalatError::NotificationsUnavailable => "E501",
            SalatError::DatabaseFailed => "E601",
        }
    }

    /// The screen heading, e.g. `"LOCATION LOOKUP FAILED"` (§18.2).
    pub fn title(&self) -> &'static str {
        match self {
            SalatError::NotImplemented { .. } => "NOT IMPLEMENTED",
            SalatError::LocationLookupFailed => "LOCATION LOOKUP FAILED",
            SalatError::InvalidCoordinates => "INVALID COORDINATES",
            SalatError::InvalidMethod => "INVALID CALCULATION METHOD",
            SalatError::TimezoneUndetermined => "TIMEZONE NOT DETECTED",
            SalatError::ConfigInvalid => "CONFIGURATION INVALID",
            SalatError::ConfigWriteFailed => "COULD NOT SAVE SETTINGS",
            SalatError::PrayerTimesUnavailable => "PRAYER TIMES UNAVAILABLE",
            SalatError::ApiUnreachable => "API UNREACHABLE",
            SalatError::ApiMalformed => "UNEXPECTED API RESPONSE",
            SalatError::ApiRateLimited => "RATE LIMITED",
            SalatError::AudioNoPlayer => "NO AUDIO PLAYER FOUND",
            SalatError::AudioNoFile => "ADHAN FILE MISSING",
            SalatError::AudioPlaybackFailed => "ADHAN PLAYBACK FAILED",
            SalatError::NotificationsUnavailable => "NOTIFICATIONS UNAVAILABLE",
            SalatError::DatabaseFailed => "DATABASE ERROR",
        }
    }

    /// The `→` detail lines the ERROR screen prints. `E001` reproduces the
    /// design's example exactly; other bodies arrive with their phases.
    pub fn details(&self) -> Vec<String> {
        match self {
            SalatError::LocationLookupFailed => vec![
                "→ HTTP 503 Service Unavailable".into(),
                "→ retried 3× (1s · 2s · 4s)".into(),
                "→ giving up".into(),
            ],
            SalatError::InvalidCoordinates => {
                vec!["→ coordinates are outside the valid range".into()]
            }
            SalatError::InvalidMethod => {
                vec!["→ the configured calculation method is not recognised".into()]
            }
            SalatError::TimezoneUndetermined => {
                vec!["→ no timezone could be determined for this location".into()]
            }
            SalatError::ConfigInvalid => vec!["→ the configuration failed validation".into()],
            SalatError::ConfigWriteFailed => {
                vec!["→ the configuration file could not be written".into()]
            }
            SalatError::PrayerTimesUnavailable => {
                vec!["→ no cached row covers the requested day".into()]
            }
            SalatError::ApiUnreachable => vec!["→ api.aladhan.com could not be reached".into()],
            SalatError::ApiMalformed => {
                vec!["→ the api response could not be parsed".into()]
            }
            SalatError::ApiRateLimited => vec!["→ the api answered 429".into()],
            SalatError::AudioNoPlayer => {
                vec!["→ no supported player found on $PATH".into()]
            }
            SalatError::AudioNoFile => vec!["→ the configured adhan file is missing".into()],
            SalatError::AudioPlaybackFailed => {
                vec!["→ the audio player exited with an error".into()]
            }
            SalatError::NotificationsUnavailable => {
                vec!["→ no d-bus session bus is available".into()]
            }
            SalatError::DatabaseFailed => vec!["→ the cache database is unusable".into()],
            SalatError::NotImplemented { command } => {
                vec![format!("→ {command} arrives in a later phase (§23)")]
            }
        }
    }

    /// The `help:` line. `E001` reproduces the design's example exactly.
    pub fn help(&self) -> String {
        match self {
            SalatError::LocationLookupFailed => "use [C] coordinates or [R] retry".into(),
            // No design text exists yet for the others; naming the actions
            // from the §18.2 table is the honest minimum.
            other => {
                let actions = other
                    .actions()
                    .iter()
                    .map(|a| a.name())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("actions: {actions}")
            }
        }
    }

    /// Which buttons the ERROR screen shows for this error (§18.2).
    pub fn actions(&self) -> &'static [ErrorAction] {
        match self {
            SalatError::LocationLookupFailed => &[
                ErrorAction::Retry,
                ErrorAction::Coordinates,
                ErrorAction::Back,
            ],
            SalatError::InvalidCoordinates => &[ErrorAction::Coordinates, ErrorAction::Back],
            SalatError::InvalidMethod => &[ErrorAction::Back],
            SalatError::TimezoneUndetermined => &[ErrorAction::Coordinates, ErrorAction::Back],
            SalatError::ConfigInvalid => &[
                ErrorAction::Retry,
                ErrorAction::UseDefaults,
                ErrorAction::Back,
            ],
            SalatError::ConfigWriteFailed => &[ErrorAction::Retry, ErrorAction::Back],
            SalatError::PrayerTimesUnavailable => &[ErrorAction::Retry, ErrorAction::Back],
            SalatError::ApiUnreachable => &[ErrorAction::Retry, ErrorAction::Back],
            SalatError::ApiMalformed => &[ErrorAction::Retry, ErrorAction::Back],
            SalatError::ApiRateLimited => &[ErrorAction::Retry, ErrorAction::Back],
            SalatError::AudioNoPlayer => &[ErrorAction::Back],
            SalatError::AudioNoFile => &[ErrorAction::Back],
            SalatError::AudioPlaybackFailed => &[ErrorAction::Retry, ErrorAction::Back],
            SalatError::NotificationsUnavailable => &[ErrorAction::Back],
            SalatError::DatabaseFailed => &[ErrorAction::Retry, ErrorAction::Back],
            SalatError::NotImplemented { .. } => &[ErrorAction::Back],
        }
    }

    /// The process exit code for this error (§14.5): configuration errors
    /// exit `3`, prayer-data errors exit `4`, everything else exits `1`.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            SalatError::ConfigInvalid | SalatError::ConfigWriteFailed => ExitCode::Config,
            SalatError::PrayerTimesUnavailable
            | SalatError::ApiUnreachable
            | SalatError::ApiMalformed
            | SalatError::ApiRateLimited => ExitCode::Network,
            _ => ExitCode::Runtime,
        }
    }
}

impl From<SalatError> for ExitCode {
    fn from(err: SalatError) -> Self {
        err.exit_code()
    }
}

impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = match self {
            ExitCode::Ok => 0,
            ExitCode::Runtime => 1,
            ExitCode::Usage => 2,
            ExitCode::Config => 3,
            ExitCode::Network => 4,
        };
        write!(f, "{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn every_variant() -> Vec<SalatError> {
        vec![
            SalatError::LocationLookupFailed,
            SalatError::InvalidCoordinates,
            SalatError::InvalidMethod,
            SalatError::TimezoneUndetermined,
            SalatError::ConfigInvalid,
            SalatError::ConfigWriteFailed,
            SalatError::PrayerTimesUnavailable,
            SalatError::ApiUnreachable,
            SalatError::ApiMalformed,
            SalatError::ApiRateLimited,
            SalatError::AudioNoPlayer,
            SalatError::AudioNoFile,
            SalatError::AudioPlaybackFailed,
            SalatError::NotificationsUnavailable,
            SalatError::DatabaseFailed,
            SalatError::NotImplemented { command: "status" },
        ]
    }

    /// Phase 0 item 8: every variant returns a unique, correctly formatted
    /// code — `E` plus exactly three digits.
    #[test]
    fn codes_are_unique_and_well_formed() {
        let mut seen = HashSet::new();
        for err in every_variant() {
            let code = err.code();
            let digits_ok = code.len() == 4
                && code.starts_with('E')
                && code[1..].chars().all(|c| c.is_ascii_digit());
            assert!(digits_ok, "malformed code: {code}");
            assert!(seen.insert(code), "duplicate code: {code}");
            assert!(!err.title().is_empty(), "empty title for {code}");
            assert!(!err.details().is_empty(), "no details for {code}");
            assert!(!err.help().is_empty(), "no help for {code}");
            assert!(!err.actions().is_empty(), "no actions for {code}");
        }
        assert_eq!(seen.len(), 16);
    }

    /// The §14.5 exit-code mapping: E201/E202 → 3, E301–E304 → 4, the rest
    /// (including the transient E000) → 1.
    #[test]
    fn exit_codes_follow_the_documented_mapping() {
        let cases = [
            (SalatError::ConfigInvalid, ExitCode::Config),
            (SalatError::ConfigWriteFailed, ExitCode::Config),
            (SalatError::PrayerTimesUnavailable, ExitCode::Network),
            (SalatError::ApiUnreachable, ExitCode::Network),
            (SalatError::ApiMalformed, ExitCode::Network),
            (SalatError::ApiRateLimited, ExitCode::Network),
            (SalatError::LocationLookupFailed, ExitCode::Runtime),
            (SalatError::AudioNoPlayer, ExitCode::Runtime),
            (SalatError::DatabaseFailed, ExitCode::Runtime),
            (
                SalatError::NotImplemented { command: "full" },
                ExitCode::Runtime,
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.exit_code(), expected, "{}", err.code());
        }
    }

    /// E001 reproduces the design's example exactly (§18.2).
    #[test]
    fn e001_reproduces_the_design_example() {
        let err = SalatError::LocationLookupFailed;
        assert_eq!(err.code(), "E001");
        assert_eq!(err.title(), "LOCATION LOOKUP FAILED");
        assert_eq!(
            err.details(),
            vec![
                "→ HTTP 503 Service Unavailable",
                "→ retried 3× (1s · 2s · 4s)",
                "→ giving up",
            ]
        );
        assert_eq!(err.help(), "use [C] coordinates or [R] retry");
        assert_eq!(
            err.actions(),
            &[
                ErrorAction::Retry,
                ErrorAction::Coordinates,
                ErrorAction::Back
            ]
        );
    }

    /// The numeric values are the documented exit codes themselves.
    #[test]
    fn exit_code_discriminants_match_the_documented_numbers() {
        assert_eq!(ExitCode::Ok as u8, 0);
        assert_eq!(ExitCode::Runtime as u8, 1);
        assert_eq!(ExitCode::Usage as u8, 2);
        assert_eq!(ExitCode::Config as u8, 3);
        assert_eq!(ExitCode::Network as u8, 4);
        // `From<ExitCode> for ProcessExitCode` must compile for every
        // variant — this binding exercises it on the full set.
        for code in [
            ExitCode::Ok,
            ExitCode::Runtime,
            ExitCode::Usage,
            ExitCode::Config,
            ExitCode::Network,
        ] {
            let _process_code = ProcessExitCode::from(code);
        }
    }
}
