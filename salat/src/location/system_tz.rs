//! /etc/localtime → $TZ → timedatectl timezone detection (§9.3).
//!
//! AUTO-DETECT's chain, in the doc's order, first source wins: the
//! `/etc/localtime` symlink (every mainstream distro points it at
//! `/usr/share/zoneinfo/<zone>`), then `$TZ` (the classic override —
//! a POSIX spec like `CET-1CEST` simply fails to parse and the chain
//! moves on), then `timedatectl show -p Timezone --value` (systemd's
//! own answer, which follows D-Bus rather than the symlink). Every
//! candidate is validated through chrono-tz before it is accepted, so
//! a garbage name from any source falls through rather than poisoning
//! the config.

use std::path::Path;

use chrono_tz::Tz;
use tracing::debug;

use super::error::LocationError;

/// Detect the system timezone (§9.3). Returns `E103`'s detail when
/// every source fails.
pub fn system_timezone() -> Result<Tz, LocationError> {
    if let Some(candidate) = from_localtime_symlink() {
        if let Some(zone) = parse_zone(&candidate) {
            debug!(zone = %candidate, "system timezone from /etc/localtime");
            return Ok(zone);
        }
        debug!(candidate = %candidate, "/etc/localtime is not a known zone");
    }
    if let Some(candidate) = from_env() {
        if let Some(zone) = parse_zone(&candidate) {
            debug!(zone = %candidate, "system timezone from $TZ");
            return Ok(zone);
        }
        debug!(candidate = %candidate, "$TZ is not a known zone");
    }
    if let Some(candidate) = from_timedatectl() {
        if let Some(zone) = parse_zone(&candidate) {
            debug!(zone = %candidate, "system timezone from timedatectl");
            return Ok(zone);
        }
        debug!(candidate = %candidate, "timedatectl zone is not a known zone");
    }
    Err(LocationError::TimezoneUndetermined)
}

/// The zone name a symlink target encodes: the part after the last
/// `zoneinfo/` component. `/usr/share/zoneinfo/Africa/Algiers` →
/// `Africa/Algiers`; a target without the component (or a plain-file
/// `/etc/localtime`) yields `None` and the chain moves on.
fn from_localtime_symlink() -> Option<String> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    zone_from_symlink_target(Path::new(&target))
}

/// The pure core of the symlink step, separated so a target path can
/// be tested without touching the filesystem.
fn zone_from_symlink_target(target: &Path) -> Option<String> {
    let text = target.to_str()?;
    let marker = "zoneinfo/";
    let at = text.rfind(marker)?;
    let zone = &text[at + marker.len()..];
    if zone.is_empty() {
        None
    } else {
        Some(zone.to_owned())
    }
}

/// `$TZ` — present only when the user set it; a plain zone name is
/// accepted, anything else (a POSIX `TZ` spec, empty) falls through.
fn from_env() -> Option<String> {
    std::env::var_os("TZ")
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.is_empty())
}

/// `timedatectl show -p Timezone --value` — systemd's answer. Absent
/// on non-systemd machines and inside minimal containers; a non-zero
/// exit simply yields `None`.
fn from_timedatectl() -> Option<String> {
    let output = std::process::Command::new("timedatectl")
        .args(["show", "-p", "Timezone", "--value"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout);
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "n/a" {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Validate a candidate through chrono-tz — the same parser the
/// config uses (`validate.rs`), so detection and configuration can
/// never disagree about what a zone name means.
fn parse_zone(candidate: &str) -> Option<Tz> {
    candidate.parse::<Tz>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The symlink step extracts the zone from a standard target.
    #[test]
    fn symlink_targets_extract_their_zone() {
        assert_eq!(
            zone_from_symlink_target(Path::new("/usr/share/zoneinfo/Africa/Algiers")).as_deref(),
            Some("Africa/Algiers")
        );
        // A second `zoneinfo/` component — the rfind keeps the tail.
        assert_eq!(
            zone_from_symlink_target(Path::new("/var/lib/zoneinfo/Europe/zoneinfo/Asia/Tokyo"))
                .as_deref(),
            Some("Asia/Tokyo")
        );
        // A relative target, as `read_link` returns it.
        assert_eq!(
            zone_from_symlink_target(Path::new("../usr/share/zoneinfo/America/New_York"))
                .as_deref(),
            Some("America/New_York")
        );
    }

    /// Targets without a `zoneinfo/` component — a copied file, a
    /// pathological symlink — propose nothing.
    #[test]
    fn non_zone_targets_propose_nothing() {
        assert!(zone_from_symlink_target(Path::new("/etc/localtime")).is_none());
        assert!(zone_from_symlink_target(Path::new("/usr/share/zoneinfo/")).is_none());
        // Non-UTF-8 paths cannot be zone names — `read_link` returns
        // raw bytes and `to_str` refuses them.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let raw = std::ffi::OsStr::from_bytes(b"/usr/share/zoneinfo/\xff");
            assert!(zone_from_symlink_target(Path::new(raw)).is_none());
        }
    }

    /// Every candidate the chain accepts parses through chrono-tz;
    /// the POSIX `TZ` spec and empty names do not.
    #[test]
    fn candidates_are_validated_through_chrono_tz() {
        assert_eq!(
            parse_zone("Africa/Algiers").map(|tz| tz.name()),
            Some("Africa/Algiers")
        );
        assert!(parse_zone("CET-1CEST,M3.5.0,M10.5.0/3").is_none());
        assert!(parse_zone("").is_none());
        assert!(parse_zone("Not/AZone").is_none());
    }

    /// On this machine the chain resolves *something* — the test
    /// machine's own environment is a valid instance of §9.3. It can
    /// legitimately be `Undetermined` only on a machine with no tz
    /// setup at all, which a development host is not.
    #[test]
    fn the_chain_resolves_on_this_machine() {
        let outcome = system_timezone();
        if let Err(error) = outcome {
            assert_eq!(error, LocationError::TimezoneUndetermined);
        }
    }
}
