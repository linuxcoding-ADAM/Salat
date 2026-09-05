//! DTO → `domain` mapping (§6.2–6.3 ↔ §7.6).
//!
//! Empty by design: `PrayerDay` and its neighbours are Phase 3 types
//! (§23), and Phase 2 depends on Phase 1 only. The mapping lands with
//! them — in the same phase as the type it produces — so this file never
//! holds a half-typed conversion.
