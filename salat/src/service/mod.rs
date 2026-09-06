//! Services — the layer that composes (§4.2).
//!
//! Each service owns one external concern; `PrayerTimeService` owns the
//! one the brief centres on: turning "cache + API + clock" into the
//! [`PrayerWindow`] every consumer renders, with the §7.6 resolution
//! order — cache first, network only for what the cache cannot answer.
//!
//! The module follows §23 Phase 4's explicit file list
//! (`src/service/prayer_times.rs`); §19's tree predates it and never
//! grew a `service/` entry. The layering rule is unchanged: this layer
//! depends on `api`, `cache`, `config` and `domain`, never on `ui` or
//! `cli`, and is wired only in `app::Runtime`.

pub mod prayer_times;

pub use prayer_times::{PrayerTimeService, Refreshed, ServiceError};
