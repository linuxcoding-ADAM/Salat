//! `salat config` — print the resolved scope, config, database and lock
//! paths (§8.1), then the effective configuration (§14.1). With `--edit`,
//! open `$EDITOR` and validate the result by loading it.

use std::path::Path;
use std::process::Command as ProcessCommand;

use crate::config::{ConfigStore, Paths};
use crate::error::{ExitCode, SalatError};

/// The command's entry point (§14.3). `config_path` is the `--config`
/// root flag, which scopes every resource the four printed lines name.
pub fn run(edit: bool, config_path: Option<&Path>) -> Result<ExitCode, SalatError> {
    let paths: Paths = crate::config::store::resolve(config_path)?;
    let store = ConfigStore::new(&paths.config);

    if edit {
        // A missing file is created first so the editor opens something
        // concrete; an *invalid* file is still opened — fixing it by hand
        // is the whole point of `--edit`.
        if !paths.config.exists() {
            store
                .load()
                .map_err(|e| e.into_salat_error(&paths.config))?;
        }
        run_editor(&paths.config)?;
    }

    // After the editor (or straight away), load-or-create and validate.
    let config = store
        .load()
        .map_err(|e| e.into_salat_error(&paths.config))?;

    print_resolution(&paths);
    println!();
    let toml =
        toml::to_string_pretty(config.as_ref()).map_err(|e| SalatError::ConfigWriteFailed {
            path: paths.config.clone(),
            cause: e.to_string(),
        })?;
    print!("{toml}");

    Ok(ExitCode::Ok)
}

/// The four §8.1 lines, labels padded so the paths align.
fn print_resolution(paths: &Paths) {
    println!("{:<10} {}", "scope:", paths.scope);
    println!("{:<10} {}", "config:", paths.config.display());
    println!("{:<10} {}", "database:", paths.database.display());
    println!("{:<10} {}", "lock:", paths.lock.display());
}

/// Open the user's editor on `path` (§14.1). `$VISUAL` wins over
/// `$EDITOR` when both are set — the traditional precedence for
/// full-screen editors — with `vi` as the POSIX fallback.
///
/// A spawn failure or a non-zero exit is an E202-class failure: the file
/// the user asked to edit could not be written. The command string is
/// split on whitespace only — enough for `EDITOR="code -w"` and the
/// like; users with quoting needs can wrap their editor in a script.
fn run_editor(path: &Path) -> Result<(), SalatError> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());
    let mut words = editor.split_whitespace();
    let (program, args): (String, Vec<&str>) = match words.next() {
        Some(program) => (program.into(), words.collect()),
        // An empty $EDITOR means "no preference" — use the fallback.
        None => ("vi".into(), Vec::new()),
    };

    let status = ProcessCommand::new(&program)
        .args(&args)
        .arg(path)
        .status()
        .map_err(|e| SalatError::ConfigWriteFailed {
            path: path.to_path_buf(),
            cause: format!("cannot start the editor {program:?}: {e}"),
        })?;
    if !status.success() {
        return Err(SalatError::ConfigWriteFailed {
            path: path.to_path_buf(),
            cause: format!("the editor {program:?} exited with {status}"),
        });
    }
    Ok(())
}
