//! Configuration round-trip and persistence tests (§23 Phase 1 item 10):
//! load-or-create, atomic save, the watch channel, and the §8.4 gate.
//!
//! These exercise `ConfigStore` against the real filesystem (in
//! tempdirs); the schema and rule-level behaviour is unit-tested in
//! `src/config/`.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;

use salat::config::{Config, ConfigDraft, ConfigError, ConfigStore};

/// A store rooted at a fresh `<tempdir>/config.toml`.
fn new_store() -> (ConfigStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = ConfigStore::new(dir.path().join("config.toml"));
    (store, dir)
}

/// Write `text` as the store's configuration file.
fn write_config(dir: &tempfile::TempDir, text: &str) {
    let path = dir.path().join("config.toml");
    let mut file = fs::File::create(&path).expect("create config");
    file.write_all(text.as_bytes()).expect("write config");
}

/// §23 item 10.1: save → fresh store → load is the identity.
#[test]
fn save_then_load_round_trips() {
    let (store, dir) = new_store();
    let mut config = Config::default();
    config.adhan.volume = 30;
    config.location.label = "Constantine, Algeria".into();
    config.prayers.maghrib.before_minutes = 7;
    store.save(&config).expect("save");

    let reloaded = ConfigStore::new(dir.path().join("config.toml"));
    let loaded = reloaded.load().expect("load");
    assert_eq!(*loaded, config);
}

/// §23 item 10.2: a missing file is not an error — the exact §8.3
/// commented template is written (and parsed back).
#[test]
fn missing_file_creates_the_exact_defaults() {
    let (store, dir) = new_store();
    let loaded = store.load().expect("load creates the file");
    assert_eq!(*loaded, Config::default());
    let text = fs::read_to_string(dir.path().join("config.toml")).expect("created file");
    assert_eq!(text, salat::config::defaults::default_toml());
    // The parent directory of a nested config is created at 0700 (§8.1).
    let nested = ConfigStore::new(dir.path().join("deep").join("nested").join("c.toml"));
    nested.load().expect("nested load");
    let mode = fs::metadata(dir.path().join("deep"))
        .expect("created dir")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
}

/// §8.1: the configuration file is written at mode 0600.
#[test]
fn saved_file_is_owner_only() {
    let (store, dir) = new_store();
    store.load().expect("load");
    let mode = fs::metadata(dir.path().join("config.toml"))
        .expect("file exists")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

/// §23 item 10.3: an unknown key is a structural error naming the key
/// and its line.
#[test]
fn unknown_key_error_names_the_key() {
    let (store, dir) = new_store();
    write_config(
        &dir,
        "version = 1\n\n[location]\nlabell = \"Sétif, Algeria\"\n",
    );
    let err = store.load().unwrap_err();
    let lines = err.detail_lines().join("\n");
    assert!(lines.contains("labell"), "key not named in: {lines}");
    assert!(
        lines.contains("unknown field"),
        "not a deny_unknown_fields error: {lines}"
    );
    assert!(
        lines.contains("line 4"),
        "the offending line must be named in: {lines}"
    );
}

/// §23 item 10.4: every violation is reported together, including the
/// tune/volume/before_minutes domain rules.
#[test]
fn all_violations_are_reported_together_from_disk() {
    let (store, dir) = new_store();
    write_config(
        &dir,
        "\
version = 1

[calculation.tune]
fajr = 90

[adhan]
volume = 150

[prayers.fajr]
before_minutes = 61
",
    );
    let err = store.load().unwrap_err();
    let ConfigError::Invalid(violations) = &err else {
        panic!("expected semantic violations, got: {err}");
    };
    let fields: Vec<&str> = violations.iter().map(|v| v.field.as_str()).collect();
    assert_eq!(
        fields,
        vec![
            "calculation.tune.fajr",
            "adhan.volume",
            "prayers.fajr.before_minutes",
        ]
    );
}

/// §23 item 10.5: atomic save leaves no `.tmp` — after a successful save
/// and after a failed one.
#[test]
fn no_tmp_files_survive_a_save() {
    let (store, dir) = new_store();
    store.save(&Config::default()).expect("save");
    assert!(!dir.path().join("config.toml.tmp").exists());

    // A failed save (see the read-only test) also leaves no `.tmp`; a
    // validation failure fails before any file is touched.
    let mut invalid = Config::default();
    invalid.adhan.volume = 150;
    let err = store.save(&invalid).unwrap_err();
    assert!(matches!(err, ConfigError::Invalid(_)));
    assert!(!dir.path().join("config.toml.tmp").exists());
}

/// §23 item 10.6: an unwritable directory yields E202 — the §14.6
/// specimen naming the path — and leaves the old file untouched.
#[test]
fn read_only_directory_yields_e202_and_preserves_the_old_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("config.toml");
    let original = salat::config::defaults::default_toml();
    fs::write(&path, original).expect("write original");

    // Read-only for the owner: rename() into the directory fails.
    let mut perms = fs::metadata(dir.path()).expect("dir").permissions();
    perms.set_mode(0o500);
    fs::set_permissions(dir.path(), perms).expect("chmod 0500");

    let store = ConfigStore::new(&path);
    let mut config = Config::default();
    config.adhan.volume = 40;
    let result = store.save(&config);

    // Restore before any assertion can panic and drop the tempdir.
    let mut perms = fs::metadata(dir.path()).expect("dir").permissions();
    perms.set_mode(0o700);
    fs::set_permissions(dir.path(), perms).expect("chmod 0700 back");

    let err = result.expect_err("save into a read-only directory");
    let salat_err = err.into_salat_error(&path);
    assert_eq!(salat_err.code(), "E202");
    assert_eq!(
        salat_err.to_string(),
        format!("cannot write configuration to {}", path.display())
    );
    // The old file is untouched, and no `.tmp` lingers.
    assert_eq!(fs::read_to_string(&path).expect("old file"), original);
    assert!(!dir.path().join("config.toml.tmp").exists());
}

/// §23 item 10.7: `subscribe` sees saves — `has_changed`/`borrow` are
/// enough, no async runtime needed.
#[test]
fn watch_delivers_saves() {
    let (store, _dir) = new_store();
    let mut rx = store.subscribe();
    assert_eq!(**rx.borrow(), Config::default());

    let mut config = Config::default();
    config.adhan.volume = 55;
    store.save(&config).expect("save");
    assert!(rx.has_changed().expect("sender alive"));
    rx.borrow_and_update();
    assert_eq!(**rx.borrow(), config);

    // An external write followed by `load` (the SIGHUP path, §8.5)
    // republishes too — through the same channel, not a poll.
    let (store2, dir2) = new_store();
    let mut rx2 = store2.subscribe();
    rx2.borrow_and_update();
    let mut hand_edited = Config::default();
    hand_edited.display.compact_mode = true;
    write_config(
        &dir2,
        &toml::to_string_pretty(&hand_edited).expect("serialize"),
    );
    store2.load().expect("reload");
    assert!(rx2.has_changed().expect("sender alive"));
    rx2.borrow_and_update();
    assert_eq!(**rx2.borrow(), hand_edited);
}

/// §23 Phase 1: the generated file and `docs/CONFIGURATION.md` are the
/// same template — the documentation cannot drift from the code.
#[test]
fn documentation_carries_the_exact_default_template() {
    let docs = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../docs/CONFIGURATION.md"
    ))
    .expect("docs/CONFIGURATION.md readable");
    assert!(
        docs.contains(salat::config::defaults::default_toml()),
        "docs/CONFIGURATION.md no longer contains the exact default template"
    );
}

/// §8.4: the version gate — newer, absent and non-integer versions all
/// reject naming `version`, the newer case with the exact message.
#[test]
fn version_gating_from_disk() {
    let (store, dir) = new_store();

    write_config(
        &dir,
        "version = 2\n\n[location]\nlabell = \"unknown schema\"\n",
    );
    let err = store.load().unwrap_err();
    assert!(matches!(err, ConfigError::Version(2)));
    assert_eq!(
        err.detail_lines(),
        vec![
            "configuration version 2 was written by a newer salat; \
             this build understands version 1"
                .to_owned()
        ]
    );

    write_config(&dir, "[location]\nlabel = \"x\"\n");
    let err = store.load().unwrap_err();
    assert!(matches!(err, ConfigError::MissingVersion));

    write_config(&dir, "version = \"one\"\n");
    let err = store.load().unwrap_err();
    assert!(matches!(err, ConfigError::VersionNotInteger));
}

/// §23 item 10.8: a draft whose save fails is preserved — the working
/// copy is still exactly what the user had, and the disk still holds the
/// last valid configuration.
#[test]
fn draft_survives_a_failed_save() {
    let (store, dir) = new_store();
    store.load().expect("initial load");
    let on_disk_before = fs::read_to_string(dir.path().join("config.toml")).expect("file");

    let mut draft = ConfigDraft::from(&Config::default());
    draft.working().adhan.volume = 200; // invalid
    // The TUI promotes `working` onto a target before saving.
    let mut target = Config::default();
    draft.apply_to(&mut target);
    let err = store.save(&target).unwrap_err();
    assert!(matches!(err, ConfigError::Invalid(_)));

    // The draft itself is untouched: still dirty, still holding the edit.
    assert!(draft.is_dirty());
    assert_eq!(draft.working_ref().adhan.volume, 200);
    assert_eq!(
        fs::read_to_string(dir.path().join("config.toml")).expect("file"),
        on_disk_before
    );
}
