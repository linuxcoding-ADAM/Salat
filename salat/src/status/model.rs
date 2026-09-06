//! `Health`, `Component`, `ComponentId`, `SystemStatus` — the §6.7
//! status domain, decoupled per the brief.
//!
//! This is the data half only. The `Collector` that assembles a
//! `SystemStatus` by calling each service's `report()` is §15.8 /
//! Phase 12; the contract it fills in is here so the renderers that
//! consume `SystemStatus` (the strip in Phase 5, the Status screen and
//! `salat status` later) can be written against it first — neither
//! consumer may reach into any service, they consume this struct only.

use chrono::{DateTime, Utc};
use chrono_tz::Tz;

use crate::domain::prayer::Notifiable;
use crate::domain::schedule::EventKind;
use crate::domain::summary::DaySummary;

/// A component's health, from the design's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Health {
    /// The probe succeeded.
    Ok,
    /// Working, but something the user should know (e.g. serving from
    /// cache after a failed refresh).
    Degraded,
    /// The probe failed.
    Down,
    /// Not probed (or the owning phase is not built yet) — reported
    /// honestly rather than as a false `Ok` (§16.4 item 8).
    #[default]
    Unknown,
}

/// The ten components the design's Status screen lists — exactly the
/// ten, in the design's order (App.tsx `services`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentId {
    /// "Prayer service" — the window service.
    PrayerService,
    /// "Prayer calculation" — the calculation method + its parameters.
    PrayerCalculation,
    /// "Location" — the resolved coordinates/label.
    Location,
    /// "Timezone" — the zone the times are rendered in.
    Timezone,
    /// "Notifications" — the desktop-notification subsystem.
    Notifications,
    /// "Adhan sound" — the Adhan configuration.
    AdhanSound,
    /// "Audio system" — the player behind the Adhan.
    AudioSystem,
    /// "Network" — the last HTTP attempt's outcome.
    Network,
    /// "Config file" — loadability of the on-disk config.
    ConfigFile,
    /// "Prayer database" — the SQLite cache.
    PrayerDatabase,
}

impl ComponentId {
    /// The ten components in the design's Status-screen order — what
    /// `SystemStatus.components` holds and the strip picks from.
    pub const ALL: [ComponentId; 10] = [
        Self::PrayerService,
        Self::PrayerCalculation,
        Self::Location,
        Self::Timezone,
        Self::Notifications,
        Self::AdhanSound,
        Self::AudioSystem,
        Self::Network,
        Self::ConfigFile,
        Self::PrayerDatabase,
    ];

    /// The design's label, verbatim.
    pub const fn label(self) -> &'static str {
        match self {
            Self::PrayerService => "Prayer service",
            Self::PrayerCalculation => "Prayer calculation",
            Self::Location => "Location",
            Self::Timezone => "Timezone",
            Self::Notifications => "Notifications",
            Self::AdhanSound => "Adhan sound",
            Self::AudioSystem => "Audio system",
            Self::Network => "Network",
            Self::ConfigFile => "Config file",
            Self::PrayerDatabase => "Prayer database",
        }
    }
}

/// One component's health (§6.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    /// Which component this is.
    pub id: ComponentId,
    /// The design's label for it.
    pub label: &'static str,
    /// Its health.
    pub health: Health,
    /// The one-line explanation the Status screen shows next to the
    /// mark (e.g. `LOADED`, `UTC+1`, `serving from cache`).
    pub detail: String,
}

impl Component {
    /// Build a component from its id, health and detail — the label
    /// comes from [`ComponentId::label`], so it cannot drift from the
    /// design.
    pub fn new(id: ComponentId, health: Health, detail: impl Into<String>) -> Self {
        Self {
            id,
            label: id.label(),
            health,
            detail: detail.into(),
        }
    }
}

/// The next scheduler firing, as the Status screen's NEXT EVENT panel
/// shows it (§16.4 item 6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextEvent {
    /// What will fire.
    pub kind: EventKind,
    /// Which prayer it belongs to; `None` for the day rollover.
    pub prayer: Option<Notifiable>,
    /// The absolute wall-clock instant it fires at.
    pub fire_at: DateTime<Tz>,
}

/// One entry in the event log (§16.4 item 6): today only, newest
/// first. `at` is the log's wall-clock stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoggedEvent {
    /// When the event happened (UTC — the formatter renders it in the
    /// location's zone).
    pub at: DateTime<Utc>,
    /// What happened.
    pub kind: EventKind,
    /// Which prayer it belonged to; `None` for the rollover.
    pub prayer: Option<Notifiable>,
    /// The sub-line, e.g. `duration 0:32` or `5 min window`.
    pub detail: String,
}

/// The §6.7 aggregate both status surfaces render. Built by the
/// Phase-12 `Collector`; consumed blindly by the renderers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemStatus {
    /// The ten components, in the design's order.
    pub components: Vec<Component>,
    /// The next scheduler firing, if the plan holds one.
    pub next_event: Option<NextEvent>,
    /// Today's counters (§6.5).
    pub today: DaySummary,
    /// Today's event log, newest first.
    pub recent: Vec<LoggedEvent>,
}

impl SystemStatus {
    /// Look one component up by id — what the strip and the Status
    /// screen rows do; never an index.
    pub fn component(&self, id: ComponentId) -> Option<&Component> {
        self.components.iter().find(|component| component.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ten design components, in the design's order, with the
    /// design's labels — the Status screen's SERVICES list is this
    /// array rendered.
    #[test]
    fn the_ten_components_carry_the_designs_labels() {
        let expected = [
            (ComponentId::PrayerService, "Prayer service"),
            (ComponentId::PrayerCalculation, "Prayer calculation"),
            (ComponentId::Location, "Location"),
            (ComponentId::Timezone, "Timezone"),
            (ComponentId::Notifications, "Notifications"),
            (ComponentId::AdhanSound, "Adhan sound"),
            (ComponentId::AudioSystem, "Audio system"),
            (ComponentId::Network, "Network"),
            (ComponentId::ConfigFile, "Config file"),
            (ComponentId::PrayerDatabase, "Prayer database"),
        ];
        for (id, label) in expected {
            assert_eq!(id.label(), label);
        }
        // `ALL` is that same list, in order, complete.
        assert_eq!(ComponentId::ALL.len(), expected.len());
        for (index, (id, _)) in expected.iter().enumerate() {
            assert_eq!(ComponentId::ALL[index], *id);
        }
    }

    /// `Component::new` threads the label from the id — it cannot
    /// disagree with `label()`.
    #[test]
    fn a_components_label_comes_from_its_id() {
        let component = Component::new(ComponentId::PrayerDatabase, Health::Ok, "LOADED");
        assert_eq!(component.label, ComponentId::PrayerDatabase.label());
        assert_eq!(component.detail, "LOADED");
    }

    /// Lookup is by id, not position.
    #[test]
    fn lookup_finds_the_asked_component() {
        let status = SystemStatus {
            components: vec![
                Component::new(ComponentId::PrayerService, Health::Ok, "RUNNING"),
                Component::new(ComponentId::Network, Health::Degraded, "offline, cached"),
            ],
            next_event: None,
            today: DaySummary {
                total: 5,
                passed: 3,
                remaining: 2,
                next_notify: None,
            },
            recent: Vec::new(),
        };
        let network = status.component(ComponentId::Network).unwrap();
        assert_eq!(network.health, Health::Degraded);
        assert!(status.component(ComponentId::AudioSystem).is_none());
    }
}
