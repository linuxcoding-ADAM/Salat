//! `ConfigDraft` (§8.5): the TUI's in-editor working copy, isolated from
//! the effective configuration until the user saves.

use super::Config;

/// An editor session's scratch state: the configuration as loaded (what
/// the store still publishes) and the possibly-modified working copy the
/// TUI's forms mutate.
///
/// The TUI never edits the store's `Arc<Config>` directly — a half-edited
/// form would change prayer times mid-keystroke. It mutates `working`,
/// and [`ConfigDraft::apply_to`] promotes the result only through
/// `ConfigStore::save`, which validates before the file is touched.
pub struct ConfigDraft {
    original: Config,
    working: Config,
}

impl ConfigDraft {
    /// The configuration this draft started from, unmodified.
    pub fn original(&self) -> &Config {
        &self.original
    }

    /// The working copy the TUI's forms mutate.
    pub fn working(&mut self) -> &mut Config {
        &mut self.working
    }

    /// The working copy as it stands, for rendering pending values
    /// without claiming editor rights over the draft.
    pub fn working_ref(&self) -> &Config {
        &self.working
    }

    /// Whether the working copy differs from what was loaded.
    pub fn is_dirty(&self) -> bool {
        self.working != self.original
    }

    /// Promote the working copy onto `target` — the caller then hands
    /// `target` to `ConfigStore::save`, whose validation gate is the
    /// boundary an invalid draft cannot cross.
    pub fn apply_to(&self, target: &mut Config) {
        *target = self.working.clone();
    }
}

impl From<&Config> for ConfigDraft {
    fn from(config: &Config) -> Self {
        Self {
            original: config.clone(),
            working: config.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh draft is not dirty; any edit makes it dirty; applying
    /// copies the working value over an independent target.
    #[test]
    fn dirtiness_tracks_the_working_copy() {
        let config = Config::default();
        let mut draft = ConfigDraft::from(&config);
        assert!(!draft.is_dirty());

        draft.working().adhan.volume = 40;
        assert!(draft.is_dirty());

        let mut target = Config::default();
        draft.apply_to(&mut target);
        assert_eq!(target.adhan.volume, 40);
        // The original stays untouched: the store still publishes it.
        assert_eq!(draft.original().adhan.volume, 80);
    }

    /// `apply_to` is a whole-config replacement, not a merge: reverting a
    /// field in the draft also reverts it in the target.
    #[test]
    fn apply_to_replaces_the_whole_configuration() {
        let mut config = Config::default();
        config.display.show_seconds = false;
        let mut draft = ConfigDraft::from(&config);
        draft.working().display.show_seconds = true;

        let mut target = Config::default();
        target.display.compact_mode = true;
        draft.apply_to(&mut target);
        // compact_mode reverts too — the draft is authoritative.
        assert!(target.display.show_seconds);
        assert!(!target.display.compact_mode);
    }
}
