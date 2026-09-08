//! `salat test-adhan` — play the configured Adhan and exit (§14.1).
//!
//! §13.5's ladder, as the command's own three numbered steps: resolve
//! the file (missing → print the expected path and the one-line
//! instruction, exit 1); resolve the player (none → print the four
//! supported players, exit 1); play at the configured volume and
//! await exit or `Ctrl-C` — both kill the child, exit 0.
//!
//! Exit 1 here is the design's own choice for this command, not the
//! `SalatError` ladder: a missing file or player is an explanation,
//! not a crash, so the failures print their message on stdout rather
//! than rendering an error block.

use std::time::Duration;

use crate::audio::{self, AudioPlayer as _, Player, players::PlayerKind};
use crate::cli::Cli;
use crate::error::{ExitCode, SalatError};

/// §13.5's exit for "explained, not played".
const EXPLAINED_FAILURE: ExitCode = ExitCode::Runtime;

/// One second between `is_playing` polls while awaiting the playback.
const POLL: Duration = Duration::from_secs(1);

pub fn run(cli: &Cli) -> Result<ExitCode, SalatError> {
    // The configuration only — no window, no scheduler: §13.5 is a
    // play-and-exit command, and booting the full runtime for it
    // would fire nothing but cost a network round-trip. The config
    // path resolution is the same `--config` handling every command
    // uses (§8.1).
    let runtime = crate::app::runtime::Runtime::build(cli)?;
    let config = runtime.config();
    runtime.block_on(async move {
        let player = Player::probed();

        // Step 1: the file. Missing → the expected path and the
        // one-line instruction, exit 1 (§13.4's wording).
        let data_dir = audio::system_data_dir().unwrap_or_default();
        let file = audio::resolve_path(&config.adhan, &data_dir);
        if audio::validate(&file).is_err() {
            println!("No Adhan file found.");
            println!("Expected: {}", file.display());
            println!("Place an MP3 there (or set adhan.file in the config).");
            return Ok(EXPLAINED_FAILURE);
        }

        // Step 2: the player. None → the four supported players, exit 1.
        let Some(kind) = player.kind() else {
            println!("No supported audio player found.");
            println!("Supported players, in priority order:");
            for candidate in PlayerKind::ALL {
                println!("  {} — {}", candidate.program(), player_note(candidate));
            }
            println!("Install one of these and ensure it is on $PATH.");
            return Ok(EXPLAINED_FAILURE);
        };

        // Step 3: play at the configured volume and await the end or
        // `Ctrl-C` (§13.5's print, verbatim shape).
        let volume = config.adhan.volume;
        println!(
            "Playing {} via {} at {}% — press Ctrl-C to stop",
            file.display(),
            kind.program(),
            volume
        );
        player.play(&file, volume).await?;
        // Await the child's own exit or `Ctrl-C` (§13.5 step 4): both
        // stop the child through the bounded §13.3 ladder, and a
        // Ctrl-C exits 0, not by signal. Registering the handler is
        // what makes the process catch it at all — without one the
        // default disposition would kill Salat mid-playback and leave
        // the reaping to the terminal's group-wide signal.
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        loop {
            tokio::select! {
                _ = &mut ctrl_c => break,
                _ = tokio::time::sleep(POLL) => {
                    if !player.is_playing() {
                        break;
                    }
                }
            }
        }
        // On a natural exit this is the documented no-op; after a
        // Ctrl-C it is the SIGTERM → grace → SIGKILL ladder. Either
        // way no Adhan outlives the command.
        player.stop().await?;
        Ok(ExitCode::Ok)
    })
}

/// The one-line note per player in the "none found" list — §13.2's
/// own caveats, so the message teaches instead of just naming.
const fn player_note(kind: PlayerKind) -> &'static str {
    match kind {
        PlayerKind::Mpv => "plays MP3/OGG/WAV with volume control",
        PlayerKind::Paplay => "PulseAudio/PipeWire playback with volume control",
        PlayerKind::Ffplay => "FFmpeg playback with volume control",
        PlayerKind::Aplay => "WAV only, no volume control",
    }
}
