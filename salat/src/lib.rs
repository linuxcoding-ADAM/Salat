//! Salat — prayer times and Adhan for the Linux terminal (§1 of
//! `docs/ARCHITECTURE.md`).
//!
//! One binary, one library crate, no client/server split (§4.1). This
//! crate re-exports its public surface; every module below the re-exports
//! is private until its phase makes it part of the surface.

// §18.3: unwrap/expect are forbidden outside tests (tests carry their own
// evidence and are allowed; `clippy.toml` sets `allow-unwrap-in-tests`).
// The one production allowance is the `OnceLock` in `app`, documented
// there.
#![deny(clippy::unwrap_used, clippy::expect_used)]
// No placeholders anywhere in the tree (core rule 12).
#![deny(clippy::todo, clippy::unimplemented, clippy::dbg_macro)]

pub mod app;
pub mod cli;
pub mod error;

pub mod config;

pub mod api;
mod audio;
mod cache;
pub mod domain;
mod location;
mod notify;
mod scheduler;
mod status;
mod ui;

pub use app::clock::{Clock, MockClock, SystemClock};
pub use domain::{
    ADHAN_DISPLAY_SECS, Countdown, DayMeta, DaySummary, EventKind, HijriDate, Notifiable, Prayer,
    PrayerDay, PrayerInstant, PrayerState, PrayerWindow, ScheduledEvent, day_summary, next_prayer,
    plan_events, previous_prayer, progress, state_of,
};
pub use error::{ErrorAction, ExitCode, SalatError};
