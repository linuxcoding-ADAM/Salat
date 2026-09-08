//! The `LocationError` contract for the three location modes (§9.3).
//!
//! These carry the *why* of a location failure; the screens map them
//! onto `SalatError`'s stable codes — `E101` invalid coordinates,
//! `E103` an undetermined timezone, `E001` a failed city lookup —
//! keeping user-facing codes fixed while detail stays rich.

/// Why a location operation failed (§9.3, §16.6's E101/E103 paths).
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LocationError {
    /// The timezone could not be determined from any source in the
    /// §9.3 chain: `/etc/localtime`, `$TZ`, `timedatectl`.
    #[error("no timezone could be determined (/etc/localtime, $TZ, timedatectl)")]
    TimezoneUndetermined,
    /// A source produced a name that is not a known IANA zone. The
    /// chain falls through to the next source; this survives for the
    /// log when all of them do.
    #[error("{found:?} is not a known IANA timezone")]
    UnknownZone {
        /// The unrecognised name, verbatim.
        found: String,
    },
    /// The index of cities is unusable — a corrupt build, mapped to
    /// `E001` at the screen boundary.
    #[error("the embedded city index is corrupt: {source}")]
    IndexCorrupt {
        /// The parser's detail.
        source: crate::location::cities::IndexError,
    },
    /// Coordinates outside the valid ranges (§9.3's E101).
    #[error("coordinates out of range: lat {lat}, lng {lng}")]
    CoordinatesOutOfRange {
        /// The latitude that was rejected.
        lat: f64,
        /// The longitude that was rejected.
        lng: f64,
    },
}
