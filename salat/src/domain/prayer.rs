//! Prayer identity (§6.1): the six prayers the API computes, and the five
//! that can be configured, scheduled and completed.
//!
//! [`Notifiable`] is the load-bearing distinction: Sunrise is computed and
//! displayed in no table (§14.1), so it is *not* part of the per-prayer
//! configuration model — making "configure a Sunrise notification" a type
//! error rather than a runtime lookup that returns nothing.

use crate::config::{PrayerSettings, PrayersConfig};

/// The six computed prayers, in canonical order — the same order the
/// `times` array of [`super::PrayerDay`] is indexed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Prayer {
    Fajr,
    Sunrise,
    Dhuhr,
    Asr,
    Maghrib,
    Isha,
}

/// The five prayers with per-prayer settings, notifications and Adhan
/// (§6.1, A2). Sunrise is excluded by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Notifiable {
    Fajr,
    Dhuhr,
    Asr,
    Maghrib,
    Isha,
}

impl Prayer {
    /// Every prayer in canonical (and `times`-index) order.
    pub const ALL: [Prayer; 6] = [
        Prayer::Fajr,
        Prayer::Sunrise,
        Prayer::Dhuhr,
        Prayer::Asr,
        Prayer::Maghrib,
        Prayer::Isha,
    ];

    /// The design's table label: `FAJR` … `ISHA` (§6.1).
    pub fn display(&self) -> &'static str {
        match self {
            Prayer::Fajr => "FAJR",
            Prayer::Sunrise => "SUNRISE",
            Prayer::Dhuhr => "DHUHR",
            Prayer::Asr => "ASR",
            Prayer::Maghrib => "MAGHRIB",
            Prayer::Isha => "ISHA",
        }
    }

    /// The design's short label: `FAJ SUN DHU ASR MAG ISH` (§6.1).
    pub fn short(&self) -> &'static str {
        match self {
            Prayer::Fajr => "FAJ",
            Prayer::Sunrise => "SUN",
            Prayer::Dhuhr => "DHU",
            Prayer::Asr => "ASR",
            Prayer::Maghrib => "MAG",
            Prayer::Isha => "ISH",
        }
    }

    /// `None` for Sunrise (A2) — the five others map one-to-one.
    pub fn notifiable(&self) -> Option<Notifiable> {
        match self {
            Prayer::Fajr => Some(Notifiable::Fajr),
            Prayer::Sunrise => None,
            Prayer::Dhuhr => Some(Notifiable::Dhuhr),
            Prayer::Asr => Some(Notifiable::Asr),
            Prayer::Maghrib => Some(Notifiable::Maghrib),
            Prayer::Isha => Some(Notifiable::Isha),
        }
    }
}

impl Notifiable {
    /// The five notifiable prayers in canonical order.
    pub const ALL: [Notifiable; 5] = [
        Notifiable::Fajr,
        Notifiable::Dhuhr,
        Notifiable::Asr,
        Notifiable::Maghrib,
        Notifiable::Isha,
    ];

    /// The design's table label, identical to [`Prayer::display`] for the
    /// five prayers that have one.
    pub fn display(&self) -> &'static str {
        self.as_prayer().display()
    }

    /// The design's short label.
    pub fn short(&self) -> &'static str {
        self.as_prayer().short()
    }

    /// The full prayer this notifiable one denotes.
    pub fn as_prayer(&self) -> Prayer {
        match self {
            Notifiable::Fajr => Prayer::Fajr,
            Notifiable::Dhuhr => Prayer::Dhuhr,
            Notifiable::Asr => Prayer::Asr,
            Notifiable::Maghrib => Prayer::Maghrib,
            Notifiable::Isha => Prayer::Isha,
        }
    }

    /// This prayer's slice of `[config.prayers]` (§8.2). The domain reads
    /// the config; it never owns a copy, so a config reload is seen by the
    /// very next state evaluation.
    pub fn settings<'a>(&self, prayers: &'a PrayersConfig) -> &'a PrayerSettings {
        match self {
            Notifiable::Fajr => &prayers.fajr,
            Notifiable::Dhuhr => &prayers.dhuhr,
            Notifiable::Asr => &prayers.asr,
            Notifiable::Maghrib => &prayers.maghrib,
            Notifiable::Isha => &prayers.isha,
        }
    }

    /// The mutable side of [`Notifiable::settings`] — the settings
    /// screens' draft edits go through the same mapping, so the read
    /// and write paths can never disagree about which field is whose.
    pub fn settings_mut<'a>(&self, prayers: &'a mut PrayersConfig) -> &'a mut PrayerSettings {
        match self {
            Notifiable::Fajr => &mut prayers.fajr,
            Notifiable::Dhuhr => &mut prayers.dhuhr,
            Notifiable::Asr => &mut prayers.asr,
            Notifiable::Maghrib => &mut prayers.maghrib,
            Notifiable::Isha => &mut prayers.isha,
        }
    }
}

impl From<Notifiable> for Prayer {
    fn from(notifiable: Notifiable) -> Self {
        notifiable.as_prayer()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §6.1: `display()` is `FAJR` … `ISHA`.
    #[test]
    fn display_names_are_the_design_labels() {
        let labels: [&str; 6] = Prayer::ALL.map(|p| p.display());
        assert_eq!(
            labels,
            ["FAJR", "SUNRISE", "DHUHR", "ASR", "MAGHRIB", "ISHA"]
        );
    }

    /// §6.1: `short()` is `FAJ SUN DHU ASR MAG ISH`.
    #[test]
    fn short_names_are_the_design_labels() {
        let labels: [&str; 6] = Prayer::ALL.map(|p| p.short());
        assert_eq!(labels, ["FAJ", "SUN", "DHU", "ASR", "MAG", "ISH"]);
    }

    /// §6.1 / A2: every prayer except Sunrise is notifiable.
    #[test]
    fn sunrise_is_the_only_non_notifiable_prayer() {
        assert_eq!(Prayer::Sunrise.notifiable(), None);
        let notifiable: Vec<Notifiable> =
            Prayer::ALL.iter().filter_map(|p| p.notifiable()).collect();
        assert_eq!(notifiable, Notifiable::ALL.to_vec());
    }

    /// Notifiable → Prayer → notifiable() round-trips for all five.
    #[test]
    fn notifiable_round_trips_through_prayer() {
        for n in Notifiable::ALL {
            assert_eq!(n.as_prayer().notifiable(), Some(n));
            assert_eq!(Prayer::from(n), n.as_prayer());
            assert_eq!(n.display(), n.as_prayer().display());
            assert_eq!(n.short(), n.as_prayer().short());
        }
    }

    /// `settings()` addresses the per-prayer table by identity — the §8.3
    /// defaults differ per prayer, so a mix-up would be visible here.
    #[test]
    fn settings_selects_the_right_prayer() {
        let prayers = crate::config::Config::default().prayers;
        assert_eq!(Notifiable::Fajr.settings(&prayers).before_minutes, 15);
        assert_eq!(Notifiable::Dhuhr.settings(&prayers).before_minutes, 10);
        assert_eq!(Notifiable::Maghrib.settings(&prayers).after_minutes, 15);
        assert_eq!(Notifiable::Isha.settings(&prayers).after_minutes, 30);
        assert_eq!(Notifiable::Asr.settings(&prayers).after_minutes, 20);
    }
}
