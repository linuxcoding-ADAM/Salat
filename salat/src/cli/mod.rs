//! The command-line surface: `Cli`, `Command` and `dispatch` (§14).
//!
//! A root flag plus an optional subcommand — not `salat quick` /
//! `salat full` — because the brief mandates that bare `salat` and
//! `salat --full` both work, and Clap resolves `None` subcommand to the
//! default action naturally (§14.2).

pub mod commands;

use std::io::IsTerminal as _;
use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};

use crate::error::{ExitCode, SalatError};

/// Shown after `--help`: the data attribution the brief requires.
pub const ATTRIBUTION: &str = "Prayer times by AlAdhan — https://aladhan.com";

/// The parsed command line (§14.2). `Clone` because the full TUI's
/// boot thread takes its own copy (§16.11).
#[derive(Debug, Clone, Parser)]
#[command(
    name = "salat",
    version,
    about = "Prayer times and Adhan for the Linux terminal",
    long_about = "Prayer times and Adhan for the Linux terminal — a keyboard-first, \
terminal-native assistant.\n\n\
Run bare `salat` for the quick live view, or `salat --full` for the complete \
interactive interface.",
    after_help = ATTRIBUTION,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Launch the full interactive TUI
    #[arg(long)]
    pub full: bool,

    /// Print one frame and exit (no live loop)
    #[arg(long)]
    pub once: bool,

    /// Alternate config file
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Disable color (also honours NO_COLOR)
    #[arg(long)]
    pub no_color: bool,

    /// Raise log level (repeatable)
    #[arg(short, long, action = ArgAction::Count)]
    pub verbose: u8,
}

/// The four subcommands (§14.2). Everything else is a root flag.
#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Print system status and exit
    Status,
    /// Print the resolved scope, config, database and lock paths, then the
    /// effective config
    Config {
        /// Open $EDITOR, validate on save
        #[arg(long)]
        edit: bool,
    },
    /// Play the configured Adhan and exit
    TestAdhan,
    /// Run the scheduler headlessly (for systemd)
    Daemon {
        /// Run one catch-up pass and exit (cron-style invocation)
        #[arg(long)]
        once: bool,
    },
}

/// Route the parsed command line to its command function (§14.3).
///
/// Non-TTY stdout auto-selects the `--once` path with ANSI stripped, so
/// `salat > file`, `salat | grep` and `watch salat` all behave — one
/// `IsTerminal` check that prevents the most common terminal-app bug report.
/// An explicit `--once` forces the same path regardless of the TTY state,
/// per §14.1's definition of the flag.
pub fn dispatch(cli: Cli) -> Result<ExitCode, SalatError> {
    let stdout_is_tty = std::io::stdout().is_terminal();
    let once = cli.once || !stdout_is_tty;
    match &cli.command {
        Some(Command::Status) => commands::status::run(),
        Some(Command::Config { edit }) => commands::config::run(*edit, cli.config.as_deref()),
        Some(Command::TestAdhan) => commands::test_adhan::run(&cli),
        Some(Command::Daemon { once }) => commands::daemon::run(*once),
        None => {
            if cli.full {
                commands::full::run(&cli)
            } else if once {
                commands::quick::run_once(&cli)
            } else {
                commands::quick::run_live(&cli)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// The command surface parses and advertises every documented entry
    /// point (§14.1).
    #[test]
    fn help_lists_the_documented_surface() {
        let help = Cli::command().render_help().to_string();
        for needle in [
            "--full",
            "--once",
            "--config <PATH>",
            "--no-color",
            "-v, --verbose",
            "status",
            "config",
            "test-adhan",
            "daemon",
        ] {
            assert!(help.contains(needle), "help is missing {needle:?}:\n{help}");
        }
    }
}
