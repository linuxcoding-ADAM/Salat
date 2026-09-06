//! The status domain (§6.7): the decoupled `SystemStatus` data model
//! in [`model`], and the `Collector` (§15.8) that assembles it in
//! Phase 12.
//!
//! Neither status surface — the Status screen nor `salat status` —
//! may reach into any service; they consume `SystemStatus` only.

pub mod model;

pub use model::{Component, ComponentId, Health, LoggedEvent, NextEvent, SystemStatus};
