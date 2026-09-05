//! Load, atomic save, watch publication (§8.5), and the §8.1 scope keys
//! and resource paths.

use std::fs;
use std::io::Write as _;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::watch;

use super::defaults;
use super::{CONFIG_VERSION, Config, ConfigError};

/// §8.1: the file records a physical location — personal data even though
/// it is not a secret.
const FILE_MODE: u32 = 0o600;
const DIR_MODE: u32 = 0o700;

/// FNV-1a 64-bit constants (§8.1). Hand-written on purpose: the scope key
/// must be stable forever, so a hashing crate — or `DefaultHasher`, whose
/// output std documents as unstable across releases — is unusable here.
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100_0000_01b3;

/// The FNV-1a 64-bit hash of `bytes` (§8.1).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
    })
}

/// The canonical form of `path`: `fs::canonicalize` when the file exists,
/// falling back to lexical absolutization when it does not yet (§8.1), so
/// `--config ./mecca.toml` and `--config /home/u/mecca.toml` are one
/// instance rather than two.
fn canonicalish(path: &Path) -> PathBuf {
    match fs::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(_) => std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()),
    }
}

/// The §8.1 scope key for a `--config` path: the sanitized file stem plus
/// the low 32 bits of the FNV-1a 64-bit hash of the canonicalized path.
///
/// The stem keeps the key readable (`scheduler-mecca-75b6780d.lock` tells
/// a user which config it belongs to); the digest keeps it collision-free.
pub fn scope_for(config_path: &Path) -> String {
    let canonical = canonicalish(config_path);
    let digest = fnv1a_64(canonical.as_os_str().as_bytes()) & 0xFFFF_FFFF;
    let stem: String = canonical
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
        .take(16)
        .collect();
    // A stem made entirely of filtered-out characters (e.g. "Çé.toml")
    // still needs a stable, readable name.
    let stem = if stem.is_empty() {
        "config".to_owned()
    } else {
        stem
    };
    format!("{stem}-{digest:08x}")
}

/// The XDG base directories the §8.1 resources resolve under, already
/// app-specific where the XDG spec makes them so.
#[derive(Debug, Clone)]
struct SystemDirs {
    /// `$XDG_CONFIG_HOME/salat`
    config_dir: PathBuf,
    /// `$XDG_DATA_HOME/salat`
    data_dir: PathBuf,
    /// `$XDG_RUNTIME_DIR` (not app-specific: the lock adds `salat/`).
    runtime_dir: PathBuf,
}

impl SystemDirs {
    /// Resolve from the environment via `directories`. Fails only when no
    /// home directory exists — nothing can be resolved on such a system.
    fn detect() -> Result<Self, crate::error::SalatError> {
        let base = directories::BaseDirs::new();
        let project = directories::ProjectDirs::from("", "", "salat").ok_or_else(|| {
            crate::error::SalatError::ConfigInvalid {
                problems: vec!["no home directory: cannot resolve the XDG paths".into()],
            }
        })?;
        let data_dir = project.data_dir().to_path_buf();
        // §8.1 names $XDG_RUNTIME_DIR for the lock. When it is unset
        // (no systemd session), state- then data-dir are the least-wrong
        // persistent substitutes — a stale lock file is harmless.
        let runtime_dir = base
            .as_ref()
            .and_then(|b| b.runtime_dir())
            .map(PathBuf::from)
            .or_else(|| base.as_ref().and_then(|b| b.state_dir()).map(PathBuf::from))
            .unwrap_or_else(|| data_dir.clone());
        Ok(Self {
            config_dir: project.config_dir().to_path_buf(),
            data_dir,
            runtime_dir,
        })
    }
}

/// The four resolved values `salat config` prints (§8.1): the scope key
/// and the config, database and lock paths it scopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `"default"`, or `<stem>-<8 hex>` for a `--config` path.
    pub scope: String,
    pub config: PathBuf,
    pub database: PathBuf,
    pub lock: PathBuf,
}

/// Resolve the §8.1 paths for an optional `--config` value against
/// injected directories — pure, so the shapes are unit-testable.
fn resolve_in(config_path: Option<&Path>, dirs: &SystemDirs) -> Paths {
    match config_path {
        None => Paths {
            scope: "default".into(),
            config: dirs.config_dir.join("config.toml"),
            database: dirs.data_dir.join("salat.db"),
            lock: dirs.runtime_dir.join("salat").join("scheduler.lock"),
        },
        Some(path) => {
            let scope = scope_for(path);
            Paths {
                database: dirs.data_dir.join(format!("salat-{scope}.db")),
                lock: dirs
                    .runtime_dir
                    .join("salat")
                    .join(format!("scheduler-{scope}.lock")),
                config: canonicalish(path),
                scope,
            }
        }
    }
}

/// Resolve the §8.1 paths from the real environment.
pub fn resolve(config_path: Option<&Path>) -> Result<Paths, crate::error::SalatError> {
    let dirs = SystemDirs::detect()?;
    Ok(resolve_in(config_path, &dirs))
}

/// Parse configuration text: `version` first (§8.4 — a file from a
/// different schema cannot be meaningfully validated against this one),
/// then the structural deserialize, then semantic validation.
fn parse_config(text: &str) -> Result<Config, ConfigError> {
    let table: toml::Table = toml::from_str(text).map_err(ConfigError::Parse)?;
    match table.get("version") {
        Some(toml::Value::Integer(v)) if *v == i64::from(CONFIG_VERSION) => {}
        // Covers both a newer version (the §8.4 message) and `0`/negatives
        // (the "positive integer" message): `detail_lines` picks by value.
        Some(toml::Value::Integer(v)) => return Err(ConfigError::Version(*v)),
        Some(_) => return Err(ConfigError::VersionNotInteger),
        None => return Err(ConfigError::MissingVersion),
    }
    let config: Config = toml::from_str(text).map_err(ConfigError::Parse)?;
    config.validate().map_err(ConfigError::Invalid)?;
    Ok(config)
}

/// The single publication point for the effective configuration (§8.5).
///
/// `load` reads (or creates) the file and republishes; `save` validates,
/// writes atomically and publishes; `subscribe` hands every consumer its
/// `watch::Receiver`. No service polls the file; no restart is needed
/// for any setting.
pub struct ConfigStore {
    path: PathBuf,
    current: watch::Sender<Arc<Config>>,
}

impl ConfigStore {
    /// A store for `path`. No IO happens here; call [`ConfigStore::load`]
    /// before treating a subscription as the effective configuration —
    /// the channel starts at `Config::default()`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let (sender, _) = watch::channel(Arc::new(Config::default()));
        Self {
            path: path.into(),
            current: sender,
        }
    }

    /// The store's configuration file (§8.5 `::path`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the file and publish it. A missing file is not an error
    /// (§8.3): the commented §8.3 default is written and returned. Also
    /// the SIGHUP re-read path (§8.5).
    pub fn load(&self) -> Result<Arc<Config>, ConfigError> {
        let config = match fs::read_to_string(&self.path) {
            Ok(text) => parse_config(&text)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let text = defaults::default_toml();
                // Parse what is about to be written rather than trusting
                // `Config::default()` — the template is the artefact.
                let config = parse_config(text)?;
                self.write_text(text)?;
                config
            }
            Err(e) => return Err(ConfigError::Read(e)),
        };
        let shared = Arc::new(config);
        self.current.send_replace(shared.clone());
        Ok(shared)
    }

    /// Validate, write atomically, publish (§8.5). The validation gate is
    /// `save`'s: an invalid draft never reaches the disk.
    pub fn save(&self, config: &Config) -> Result<(), ConfigError> {
        config.validate().map_err(ConfigError::Invalid)?;
        let text = toml::to_string_pretty(config).map_err(ConfigError::Serialize)?;
        self.write_text(&text)?;
        self.current.send_replace(Arc::new(config.clone()));
        Ok(())
    }

    /// Every consumer's subscription (§8.5).
    pub fn subscribe(&self) -> watch::Receiver<Arc<Config>> {
        self.current.subscribe()
    }

    /// The atomic write (§8.5): serialize, write to `<config>.tmp` *in
    /// the same directory* — `rename` is only atomic within a filesystem
    /// — `fsync`, then `rename` over the target. A crash mid-write can
    /// therefore never truncate a working config, and a failed write
    /// never leaves a `.tmp` behind or touches the old file.
    fn write_text(&self, text: &str) -> Result<(), ConfigError> {
        self.ensure_parent_dir()?;
        let tmp = self.path.with_extension("toml.tmp");
        let write = || -> Result<(), ConfigError> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(FILE_MODE)
                .open(&tmp)
                .map_err(ConfigError::Write)?;
            file.write_all(text.as_bytes())
                .map_err(ConfigError::Write)?;
            file.sync_all().map_err(ConfigError::Write)?;
            drop(file);
            fs::rename(&tmp, &self.path).map_err(ConfigError::Write)?;
            Ok(())
        };
        let result = write();
        if result.is_err() {
            // Never leave the temp file behind, even on failure.
            let _ = fs::remove_file(&tmp);
        }
        result
    }

    /// Create the missing parent-directory chain when needed, every
    /// created directory at §8.1's `0700` (umask would otherwise leave
    /// `create_dir_all`'s 0755). Existing directories are left exactly as
    /// their owner set them — salat never chmods a directory it did not
    /// create.
    fn ensure_parent_dir(&self) -> Result<(), ConfigError> {
        let Some(dir) = self.path.parent() else {
            return Ok(());
        };
        if dir.as_os_str().is_empty() || dir.exists() {
            return Ok(());
        }
        // Record the missing chain, then chmod exactly what was created.
        let mut created: Vec<PathBuf> = Vec::new();
        let mut current = Some(dir);
        while let Some(dir) = current {
            if dir.as_os_str().is_empty() || dir.exists() {
                break;
            }
            created.push(dir.to_path_buf());
            current = dir.parent();
        }
        fs::create_dir_all(dir).map_err(ConfigError::Write)?;
        for dir in &created {
            // Best-effort: some filesystems reject chmod, and the file
            // itself is already 0600.
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(DIR_MODE));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §8.1's canonical example, with its actual FNV-1a low word: the
    /// pair every example in the document carries.
    #[test]
    fn scope_key_matches_the_documented_specimen() {
        assert_eq!(scope_for(Path::new("/home/u/mecca.toml")), "mecca-75b6780d");
    }

    /// The §8.1 property the canonicalization exists for: a relative
    /// path and its absolute form are one instance, not two.
    #[test]
    fn relative_and_absolute_paths_share_a_scope() {
        let cwd = std::env::current_dir().unwrap();
        let relative = scope_for(Path::new("./mecca.toml"));
        let absolute = scope_for(&cwd.join("mecca.toml"));
        assert_eq!(relative, absolute);
    }

    /// The stem is lowercased, restricted to `[a-z0-9_-]` and truncated
    /// to 16 characters (§8.1).
    #[test]
    fn stem_is_sanitized_and_truncated() {
        let scope = scope_for(Path::new("/tmp/My-Locations.File.GZ"));
        // The key is `<stem>-<8 hex>`; the stem itself may contain
        // hyphens, so split from the right. Dots are not in `[a-z0-9_-]`
        // and are dropped, not replaced.
        let (stem, digest) = scope.rsplit_once('-').unwrap();
        assert_eq!(stem, "my-locationsfile");
        assert_eq!(digest.len(), 8);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(stem.chars().count() <= 16);
        assert!(
            stem.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        );
    }

    /// The §8.1 resource table: unsuffixed under the default, suffixed
    /// under `--config`.
    #[test]
    fn resource_paths_follow_the_scope_table() {
        let dirs = SystemDirs {
            config_dir: PathBuf::from("/cfg/salat"),
            data_dir: PathBuf::from("/data/salat"),
            runtime_dir: PathBuf::from("/run/user/1000"),
        };
        let default = resolve_in(None, &dirs);
        assert_eq!(default.scope, "default");
        assert_eq!(default.config, PathBuf::from("/cfg/salat/config.toml"));
        assert_eq!(default.database, PathBuf::from("/data/salat/salat.db"));
        assert_eq!(
            default.lock,
            PathBuf::from("/run/user/1000/salat/scheduler.lock")
        );

        let scoped = resolve_in(Some(Path::new("/home/u/mecca.toml")), &dirs);
        assert_eq!(scoped.scope, "mecca-75b6780d");
        assert_eq!(scoped.config, PathBuf::from("/home/u/mecca.toml"));
        assert_eq!(
            scoped.database,
            PathBuf::from("/data/salat/salat-mecca-75b6780d.db")
        );
        assert_eq!(
            scoped.lock,
            PathBuf::from("/run/user/1000/salat/scheduler-mecca-75b6780d.lock")
        );
    }

    /// `version` is checked before structural validation: a newer file
    /// gets the honest "newer salat" message, not a confusing "unknown
    /// field" (§8.4).
    #[test]
    fn newer_version_is_rejected_before_structure() {
        let text = "\
version = 2

[location]
labell = \"typo from a schema we do not know\"
";
        let err = parse_config(text).unwrap_err();
        assert!(matches!(err, ConfigError::Version(2)));
        assert_eq!(
            err.detail_lines(),
            vec![
                "configuration version 2 was written by a newer salat; \
                 this build understands version 1"
                    .to_owned()
            ]
        );
    }

    /// Version gating for the remaining §8.4 rows: absent, non-integer
    /// and `< 1` all reject naming `version`.
    #[test]
    fn invalid_versions_are_rejected_naming_version() {
        for (text, needle) in [
            ("[location]\nlabel = \"x\"\n", "missing field `version`"),
            ("version = \"one\"\n", "must be a positive integer"),
            ("version = 0\n", "must be a positive integer"),
            ("version = -3\n", "must be a positive integer"),
        ] {
            let err = parse_config(text).unwrap_err();
            let lines = err.detail_lines().join("\n");
            assert!(lines.contains("version"), "version not named in: {lines}");
            assert!(lines.contains(needle), "{needle:?} missing from: {lines}");
        }
    }
}
