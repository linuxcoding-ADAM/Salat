//! Location resolution and validation — the three modes (§9.3).
//!
//! The pieces land in §23's order: the city index first (§9.2), then
//! system timezone detection, then the LOCATION screen that uses both.

pub mod cities;
pub mod error;
pub mod system_tz;

pub use error::LocationError;
pub use system_tz::system_timezone;
