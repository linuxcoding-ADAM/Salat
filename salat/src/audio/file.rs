//! Preset/path resolution and validation (§13.4).
//!
//! `cfg.adhan.file` non-empty wins; otherwise the preset resolves to
//! `$XDG_DATA_HOME/salat/adhan/{preset}.mp3`. Whatever comes out is
//! validated the same way: absolute, existing, a regular file (not a
//! device, FIFO, or symlink to one), readable, and ≤ 50 MB. An absent
//! or invalid file is [`AudioError::NoFile`] — never a panic, never a
//! fallback to a bundled recording (§2.3: none ships).

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::error::SalatError;

/// §13.4's upper bound — a real Adhan is a few MB; 50 leaves room for
/// high-bitrate recitations while keeping a misconfigured path pointed
/// at, say, an ISO from wrecking memory-mapped reads.
const MAX_BYTES: u64 = 50 * 1024 * 1024;

/// What file resolution can fail with — mapped onto the design's
/// `E402` at the `SalatError` boundary.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AudioError {
    /// §13.4's absent/invalid file → the Status row `NO ADHAN FILE`.
    #[error("no usable adhan file")]
    NoFile,
    /// §13.2's absent player → `E401` at the CLI; the player keeps a
    /// `NO AUDIO PLAYER` status row.
    #[error("no audio player found")]
    NoPlayer,
    /// §13.3's failed spawn or non-zero exit (E403's family).
    #[error("playback failed: {detail}")]
    PlaybackFailed { detail: String },
}

impl From<AudioError> for SalatError {
    fn from(error: AudioError) -> Self {
        match error {
            AudioError::NoPlayer => SalatError::AudioNoPlayer,
            AudioError::NoFile => SalatError::AudioNoFile,
            AudioError::PlaybackFailed { .. } => SalatError::AudioPlaybackFailed,
        }
    }
}

/// Resolve the Adhan file for a configuration (§13.4):
/// the explicit path when set, else `$XDG_DATA_HOME/salat/adhan/{preset}.mp3`.
/// Resolution is pure — validation is a separate step.
pub fn resolve_path(config: &crate::config::AdhanConfig, data_dir: &Path) -> PathBuf {
    if config.file.as_os_str().is_empty() {
        data_dir
            .join("adhan")
            .join(format!("{}.mp3", config.preset))
    } else {
        config.file.clone()
    }
}

/// Validate one candidate path (§13.4's five checks). Returns the path
/// back on success — the caller holds the exact same value it
/// validated.
///
/// The checks, in order: the path is absolute; it exists (via symlink
/// traversal — the target is judged, so a symlink to a regular file is
/// fine and one to a device or FIFO is not); it is a regular file; it
/// is readable; and it is ≤ 50 MB.
pub fn validate(path: &Path) -> Result<&Path, AudioError> {
    if !path.is_absolute() {
        return Err(AudioError::NoFile);
    }
    // `metadata` follows symlinks; `symlink_metadata` would judge the
    // link itself. §13.4's "symlink to one [a device/FIFO]" wording
    // judges the target.
    let Ok(metadata) = std::fs::metadata(path) else {
        return Err(AudioError::NoFile);
    };
    if !metadata.is_file() {
        return Err(AudioError::NoFile);
    }
    if metadata.len() > MAX_BYTES {
        return Err(AudioError::NoFile);
    }
    if !path.is_file() || std::fs::File::open(path).is_err() {
        return Err(AudioError::NoFile);
    }
    Ok(path)
}

/// The data directory the preset resolves under, in production:
/// `$XDG_DATA_HOME/salat`, via `directories` (the same resolver the
/// config store's §8.1 paths use).
pub fn system_data_dir() -> Result<PathBuf, SalatError> {
    let project = directories::ProjectDirs::from("", "", "salat").ok_or(SalatError::AudioNoFile)?;
    Ok(project.data_dir().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AdhanConfig;

    fn adhan(file: PathBuf, preset: &str) -> AdhanConfig {
        AdhanConfig {
            enabled: true,
            preset: preset.to_string(),
            file,
            volume: 80,
        }
    }

    /// §13.4's resolution: the explicit file wins, the preset lands
    /// under `$XDG_DATA_HOME/salat/adhan/{preset}.mp3`.
    #[test]
    fn resolution_prefers_the_explicit_file() {
        let data = Path::new("/home/adam/.local/share/salat");
        assert_eq!(
            resolve_path(&adhan(PathBuf::new(), "abdul-basit"), data),
            PathBuf::from("/home/adam/.local/share/salat/adhan/abdul-basit.mp3")
        );
        assert_eq!(
            resolve_path(
                &adhan(PathBuf::from("/opt/adhan/custom.mp3"), "abdul-basit"),
                data
            ),
            PathBuf::from("/opt/adhan/custom.mp3")
        );
    }

    /// §13.4's validation, happy path: a real readable file under 50 MB
    /// validates to itself.
    #[test]
    fn a_regular_readable_file_validates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("adhan.mp3");
        std::fs::write(&file, b"ID3...").expect("write");
        assert_eq!(validate(&file), Ok(file.as_path()));
    }

    /// Every rejection path is the one error, `NoFile` (E402 at the
    /// boundary): a relative path, a missing file, a directory, a FIFO,
    /// a symlink to a device, an unreadable file, an oversized file.
    #[test]
    fn every_invalid_candidate_is_no_file() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Relative — even if it exists.
        let cwd_file = dir.path().join("rel.mp3");
        std::fs::write(&cwd_file, b"x").expect("write");
        let relative = Path::new("rel.mp3");
        assert!(validate(relative).is_err());

        // Missing (absolute).
        assert!(validate(&dir.path().join("absent.mp3")).is_err());

        // A directory.
        assert!(validate(dir.path()).is_err());

        // A FIFO.
        let fifo = dir.path().join("fifo");
        let _ = std::process::Command::new("mkfifo").arg(&fifo).status();
        if fifo.exists() {
            assert_eq!(validate(&fifo), Err(AudioError::NoFile));
        }

        // A symlink to a device.
        let link = dir.path().join("zero.mp3");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/dev/zero", &link).expect("symlink");
        assert_eq!(validate(&link), Err(AudioError::NoFile));

        // Oversized: 50 MB + 1. A sparse file reaches the length
        // without the bytes on disk, which is exactly `metadata().len`.
        let big = dir.path().join("big.mp3");
        let file = std::fs::File::create(&big).expect("create");
        file.set_len(MAX_BYTES + 1).expect("set_len");
        drop(file);
        assert_eq!(validate(&big), Err(AudioError::NoFile));

        // Unreadable: strip the read bits (best effort — running as
        // root defeats it, so the assert is conditional).
        let secret = dir.path().join("secret.mp3");
        std::fs::write(&secret, b"x").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&secret)
                .expect("metadata")
                .permissions()
                .mode();
            std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(mode & 0o222))
                .expect("chmod");
            if nix_like_root() {
                // CI runs the suite as a non-root user precisely so this
                // branch is not taken there.
                eprintln!(
                    "note: running as root; the unreadable-file case needs a non-root user and was not checked"
                );
            } else {
                assert_eq!(validate(&secret), Err(AudioError::NoFile));
            }
        }
    }

    /// A symlink to a regular file is fine — §13.4 judges the target.
    #[test]
    #[cfg(unix)]
    fn a_symlink_to_a_regular_file_validates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("real.mp3");
        std::fs::write(&target, b"ID3").expect("write");
        let link = dir.path().join("link.mp3");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        assert_eq!(validate(&link), Ok(link.as_path()));
    }

    /// Exactly 50 MB is valid — the bound is inclusive.
    #[test]
    fn fifty_megabytes_exactly_is_valid() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fifty = dir.path().join("fifty.mp3");
        let handle = std::fs::File::create(&fifty).expect("create");
        handle.set_len(MAX_BYTES).expect("set_len");
        drop(handle);
        assert_eq!(validate(&fifty), Ok(fifty.as_path()));
    }

    /// The error boundary: each `AudioError` maps to the design's
    /// E401/E402/E403 rows.
    #[test]
    fn audio_errors_map_to_salat_errors() {
        assert_eq!(
            SalatError::from(AudioError::NoPlayer),
            SalatError::AudioNoPlayer
        );
        assert_eq!(
            SalatError::from(AudioError::NoFile),
            SalatError::AudioNoFile
        );
        assert_eq!(
            SalatError::from(AudioError::PlaybackFailed {
                detail: "exit code 1".into()
            }),
            SalatError::AudioPlaybackFailed
        );
    }

    /// `id -u` == 0 means permission checks lie (root reads anything).
    /// Whether the tests run as root, where permission bits cannot
    /// make a file unreadable. `id -u` prints the uid followed by a
    /// newline (`0\n` for root), so the output is trimmed before it is
    /// compared — the untrimmed two-byte output never matched.
    fn nix_like_root() -> bool {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim() == "0")
            .unwrap_or(false)
    }
}
