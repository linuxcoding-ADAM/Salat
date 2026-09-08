//! The player probe chain and argv construction (§13.2).
//!
//! Four players, one priority order, one argv shape each — and one
//! security property that outranks everything else (§21.5): the file
//! path is a single argv element, never a shell string, so a path
//! containing spaces, quotes or `;` cannot become a command. All
//! spawning funnels through [`tokio::process::Command`], which takes an
//! argv by construction.
//!
//! `aplay` is last because it plays uncompressed WAV only and cannot
//! set volume — an honest partial capability (the Status screen says
//! `READY (aplay · no volume control)`), not a silent failure.

use std::path::Path;

use tokio::process::Command;

/// The four supported players, in §13.2's probe priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerKind {
    /// `mpv` — MP3/OGG/WAV, native 0–100 volume, most common.
    Mpv,
    /// `paplay` — PulseAudio/PipeWire, volume scaled ×655 to 0–65536.
    Paplay,
    /// `ffplay` — FFmpeg, native 0–100 volume.
    Ffplay,
    /// `aplay` — WAV only, no volume control.
    Aplay,
}

impl PlayerKind {
    /// Every player, in probe priority order (§13.2's table rows).
    pub const ALL: [PlayerKind; 4] = [Self::Mpv, Self::Paplay, Self::Ffplay, Self::Aplay];

    /// The binary's name on `$PATH`.
    pub const fn program(self) -> &'static str {
        match self {
            Self::Mpv => "mpv",
            Self::Paplay => "paplay",
            Self::Ffplay => "ffplay",
            Self::Aplay => "aplay",
        }
    }

    /// Whether this player can set the volume at all — `aplay` cannot
    /// (§13.2's documented limitation).
    pub const fn has_volume(self) -> bool {
        !matches!(self, Self::Aplay)
    }

    /// Build the spawn for `file` at `volume` (§13.2's exact argv).
    /// No shell, ever: the path is one argv element however hostile it
    /// looks.
    fn spawn_command(self, file: &Path, volume: u8) -> Command {
        let mut command = Command::new(self.program());
        match self {
            Self::Mpv => {
                command
                    .arg("--no-video")
                    .arg("--really-quiet")
                    .arg(format!("--volume={volume}"))
                    .arg(file);
            }
            Self::Paplay => {
                // 0–100 scaled into PulseAudio's 0–65536 range.
                command
                    .arg("--volume")
                    .arg((u32::from(volume) * 655).to_string())
                    .arg(file);
            }
            Self::Ffplay => {
                command
                    .arg("-nodisp")
                    .arg("-autoexit")
                    .arg("-loglevel")
                    .arg("quiet")
                    .arg("-volume")
                    .arg(volume.to_string())
                    .arg(file);
            }
            Self::Aplay => {
                // No volume flag exists — the configured volume is
                // documented as ignored, not approximated.
                command.arg("-q").arg(file);
            }
        }
        command
    }
}

/// Where a probe looks for binaries — the environment's `$PATH` in
/// production, a test-supplied directory in tests. The split keeps
/// [`probe_path`] pure over its inputs while production resolves the
/// variable once.
pub trait PathEnv {
    /// The `$PATH` value, as `os`-shaped search directories.
    fn search_path(&self) -> Vec<std::path::PathBuf>;
}

/// The production `$PATH` reader.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPathEnv;

impl PathEnv for SystemPathEnv {
    fn search_path(&self) -> Vec<std::path::PathBuf> {
        std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default()
    }
}

/// The §13.2 probe: walk the priority order and keep the first binary
/// that exists and is executable on the search path. Pure over its
/// inputs (a filesystem walk, but no clock and no process).
pub fn probe_path(env: &dyn PathEnv) -> Option<PlayerKind> {
    let search = env.search_path();
    PlayerKind::ALL.into_iter().find(|kind| {
        search
            .iter()
            .any(|dir| is_executable_file(&dir.join(kind.program())))
    })
}

/// Exists, is a regular file (or a symlink to one), and carries an
/// executable bit — the same honesty the Adhan file validation applies
/// (§13.4), minus the readability of contents.
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    metadata.permissions().mode() & 0o111 != 0
}

/// The §13.3 `PlayHandle`'s child side: spawn, and how to stop.
///
/// Kept separate from [`super::Player`] so the argv construction and
/// the probe stay testable without spawning anything: the unit tests
/// assert the argv exactly, and only the integration tests run a real
/// child.
pub(super) fn spawn(
    kind: PlayerKind,
    file: &Path,
    volume: u8,
) -> std::io::Result<tokio::process::Child> {
    kind.spawn_command(file, volume)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::PathBuf;

    /// The argv each player builds, exactly §13.2's table (the §23
    /// Phase 10's own first test): flags in order, the volume scaled
    /// only for `paplay`, the file as one single argument.
    #[test]
    fn the_argv_per_player_is_exact() {
        fn argv(kind: PlayerKind, file: &Path, volume: u8) -> Vec<String> {
            let command = kind.spawn_command(file, volume);
            command
                .as_std()
                .get_args()
                .map(|arg| arg.to_string_lossy().to_string())
                .collect()
        }

        let file = Path::new("/data/salat/adhan/abdul-basit.mp3");
        assert_eq!(
            argv(PlayerKind::Mpv, file, 80),
            vec![
                "--no-video",
                "--really-quiet",
                "--volume=80",
                "/data/salat/adhan/abdul-basit.mp3",
            ]
        );
        // paplay's ×655: 80 → 52 400, 100 → 65 500, 1 → 655.
        assert_eq!(
            argv(PlayerKind::Paplay, file, 80),
            vec!["--volume", "52400", "/data/salat/adhan/abdul-basit.mp3"]
        );
        assert_eq!(
            argv(PlayerKind::Paplay, file, 100),
            vec!["--volume", "65500", "/data/salat/adhan/abdul-basit.mp3"]
        );
        assert_eq!(
            argv(PlayerKind::Paplay, file, 1),
            vec!["--volume", "655", "/data/salat/adhan/abdul-basit.mp3"]
        );
        assert_eq!(
            argv(PlayerKind::Ffplay, file, 80),
            vec![
                "-nodisp",
                "-autoexit",
                "-loglevel",
                "quiet",
                "-volume",
                "80",
                "/data/salat/adhan/abdul-basit.mp3",
            ]
        );
        // aplay: quiet flag and the file; no volume element at all.
        assert_eq!(
            argv(PlayerKind::Aplay, file, 80),
            vec!["-q", "/data/salat/adhan/abdul-basit.mp3"]
        );
        assert!(!PlayerKind::Aplay.has_volume());
    }

    /// The program name is the binary's own — and the argv above never
    /// contains a shell metacharacter-bearing string built from the
    /// path: a hostile filename stays one argument.
    #[test]
    fn a_hostile_path_stays_one_argument() {
        let hostile = Path::new("/tmp/evil; rm -rf ~.mp3");
        let command = PlayerKind::Mpv.spawn_command(hostile, 50);
        let args: Vec<String> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        // The whole hostile string is the last argv element, in one
        // piece — there is no parsing, so no injection surface.
        assert_eq!(
            args.last().map(String::as_str),
            Some("/tmp/evil; rm -rf ~.mp3")
        );
        assert_eq!(args.len(), 4);
    }

    /// A fixed search path: every entry exists as given.
    struct FixedEnv(Vec<PathBuf>);

    impl PathEnv for FixedEnv {
        fn search_path(&self) -> Vec<PathBuf> {
            self.0.clone()
        }
    }

    /// The probe takes the first player in priority order that has an
    /// executable file on the search path — `mpv` beats `paplay` beats
    /// `ffplay` beats `aplay`, and none beats `Some`-nothing.
    #[test]
    fn the_probe_keeps_the_first_present_player() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).expect("mkdir");

        let empty = FixedEnv(vec![bin.clone()]);
        assert_eq!(probe_path(&empty), None, "nothing on PATH");

        let make = |name: &str| {
            let path = bin.join(name);
            std::fs::write(&path, b"#!/bin/sh\n").expect("write");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        };

        // Only aplay: last resort still wins when alone.
        make("aplay");
        assert_eq!(
            probe_path(&FixedEnv(vec![bin.clone()])),
            Some(PlayerKind::Aplay)
        );

        // ffplay joins: it outranks aplay.
        make("ffplay");
        assert_eq!(
            probe_path(&FixedEnv(vec![bin.clone()])),
            Some(PlayerKind::Ffplay)
        );

        make("paplay");
        assert_eq!(
            probe_path(&FixedEnv(vec![bin.clone()])),
            Some(PlayerKind::Paplay)
        );

        make("mpv");
        assert_eq!(
            probe_path(&FixedEnv(vec![bin.clone()])),
            Some(PlayerKind::Mpv)
        );
    }

    /// A non-executable file is not a player: the probe skips it rather
    /// than spawning something that will fail at exec time.
    #[test]
    fn a_non_executable_file_is_not_a_player() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("mpv"), b"not executable").expect("write");
        assert_eq!(probe_path(&FixedEnv(vec![dir.path().to_path_buf()])), None);
    }

    /// The production env reads `$PATH` — the split matches
    /// `std::env::split_paths` on a colon-separated value.
    #[test]
    fn the_system_env_splits_path() {
        let env = SystemPathEnv;
        // Whatever the machine's PATH, the split is well-formed and the
        // entries are absolute (a sane invariant; not asserted for
        // empty PATH, which the harness may produce).
        for dir in env.search_path() {
            assert!(dir.is_absolute() || dir.as_os_str().is_empty(), "{dir:?}");
        }
    }
}
