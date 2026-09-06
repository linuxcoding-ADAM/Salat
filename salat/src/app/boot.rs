//! The five-stage boot sequence (§15.7) — the state machine the
//! `--full` LOADING screen renders, and the wrapper the command layer
//! calls instead of [`Runtime::build`] directly.
//!
//! Only stages 1–3 (configuration, location, prayer times) can be
//! fatal. Stages 4–5 (audio, prayer service) belong to later phases
//! and complete instantly here — the ladder still walks through them,
//! so the screen's five steps and progress bar are real from Phase 6
//! on. Quick mode never runs this: it has no loading screen.

use crate::app::runtime::Runtime;
use crate::cli::Cli;
use crate::error::SalatError;

/// The five stage labels, §15.7 verbatim (step number = index + 1).
pub const STEPS: [&str; 5] = [
    "Loading configuration",
    "Resolving location",
    "Calculating prayer times",
    "Initializing audio",
    "Starting prayer service",
];

/// The 80×24 short labels (§16.11 item 8).
pub const SHORT_STEPS: [&str; 5] = [
    "Configuration",
    "Location",
    "Prayer times",
    "Audio",
    "Prayer service",
];

/// One boot transition, reported through the callback as each stage
/// *starts*. `stage` is the index of the step now executing (`0..=4`);
/// `BootState::done` carries `5`.
///
/// `location` is the configured location label, published from the
/// stage-2 report onward — §16.11 item 6 shows `starting…` on the
/// loading screen until step 2 (Resolving location) has completed, so
/// earlier reports carry `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootState {
    pub stage: u8,
    pub error: Option<SalatError>,
    pub location: Option<String>,
}

impl BootState {
    /// The first report: configuration loading is about to start.
    pub fn starting() -> Self {
        Self {
            stage: 0,
            error: None,
            location: None,
        }
    }

    /// The final report: all five stages completed.
    pub fn done() -> Self {
        Self {
            stage: 5,
            error: None,
            location: None,
        }
    }

    /// Whether the failing-stage render applies (§16.11 item 5).
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }
}

/// Run the §15.7 boot ladder with a progress callback.
///
/// Each stage reports as it starts; a fatal stage additionally reports
/// a second time carrying the error, then the error is returned — both
/// the callback and the return value see the failure, because the
/// LOADING screen needs to draw the red `✕` beat while the command
/// layer still needs the typed error for the ERROR screen.
pub fn run<F>(cli: &Cli, mut on_state: F) -> Result<Runtime, SalatError>
where
    F: FnMut(BootState),
{
    let mut last = BootState::starting();
    let result = Runtime::build_with(cli, |state| {
        last = state.clone();
        on_state(state);
    });
    match result {
        Ok(runtime) => Ok(runtime),
        Err(error) => {
            let failing = BootState {
                error: Some(error.clone()),
                ..last
            };
            on_state(failing);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    /// The five labels are §15.7's, and the short table's prayer-times
    /// entry is the one §16.11 item 8 names explicitly.
    #[test]
    fn the_step_tables_match_the_sections() {
        assert_eq!(STEPS[2], "Calculating prayer times");
        assert_eq!(SHORT_STEPS[2], "Prayer times");
        assert_eq!(STEPS.len(), 5);
        assert_eq!(SHORT_STEPS.len(), 5);
    }

    /// `done` is the all-clear the shell keeps after boot.
    #[test]
    fn done_is_stage_five_without_error() {
        let state = BootState::done();
        assert_eq!(state.stage, 5);
        assert!(state.error.is_none());
        assert!(!state.failed());
    }

    /// A stage-0 failure (§15.7: configuration is the first fatal
    /// stage) reaches both channels — the callback reports the failing
    /// state so the LOADING screen can draw the red `✕` beat, and the
    /// typed error is returned for the ERROR screen. `resolve` is pure,
    /// so an unparseable `--config` file fails without touching any
    /// environment state.
    #[test]
    fn a_stage_zero_failure_reports_and_returns() {
        let dir = tempfile::tempdir().expect("tempdir for the broken config");
        let path = dir.path().join("broken.toml");
        std::fs::write(&path, "not = = toml\n").expect("write the broken config");
        let cli = crate::cli::Cli::try_parse_from([
            "salat",
            "--config",
            path.to_str().expect("the tempdir path is UTF-8"),
        ])
        .expect("the command line parses");

        let mut states = Vec::new();
        // `Runtime` is not `Debug`, so the failure is matched rather
        // than `expect_err`'d.
        let error = match run(&cli, |state| states.push(state)) {
            Ok(_) => panic!("an unparseable configuration fails boot"),
            Err(error) => error,
        };

        assert!(
            matches!(error, crate::error::SalatError::ConfigInvalid { .. }),
            "stage 0 is the configuration stage: {error:?}"
        );
        let failing = states.last().expect("the failure is reported");
        assert_eq!(failing.stage, 0, "configuration is the first stage");
        assert!(failing.failed());
        assert!(
            matches!(
                failing.error,
                Some(crate::error::SalatError::ConfigInvalid { .. })
            ),
            "the failing report carries the typed error"
        );
    }
}
