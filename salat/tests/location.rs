//! §23 Phase 11 item 8's location rows, through the public API and
//! against the real embedded asset — the parser's shape tests live in
//! `src/location/cities.rs`; these prove the *data*: the diacritic-fold
//! a keyboard can actually type, the zone derivation COORDINATES mode
//! commits, and the >200 km boundary AUTO-DETECT's policy keys on.

use salat::location::cities;
use salat::location::system_timezone;

/// §23: `Setif` — typed without the accent — finds `Sétif`, with every
/// item-5 field the config needs to commit a location.
#[test]
fn setif_finds_setif_with_every_field() {
    let index = cities::load().expect("the committed cities.bin parses");
    let setif = index
        .search("Setif")
        .first()
        .expect("Sétif is in the index")
        .clone();
    assert_eq!(setif.name, "Sétif");
    assert_eq!(setif.country, "Algeria");
    assert_eq!(setif.cc, "DZ");
    assert_eq!(setif.tz, "Africa/Algiers");
    assert_eq!(setif.label(), "Sétif, Algeria");
    assert!((setif.lat - 36.19).abs() < 0.05, "lat: {}", setif.lat);
    assert!((setif.lng - 5.41).abs() < 0.05, "lng: {}", setif.lng);
    assert!(setif.population > 100_000, "a real city, not a hamlet");
}

/// The design's result list is three rows: search never returns more,
/// and a broad prefix still ranks by population (Algiers first).
#[test]
fn search_caps_at_three_and_ranks_by_population() {
    let index = cities::load().expect("the index parses");
    let algiers = index.search("Algiers");
    assert!(
        algiers.len() <= 3,
        "the design shows three rows, got {}",
        algiers.len()
    );
    assert_eq!(algiers[0].name, "Algiers", "the capital outranks the rest");
    assert_eq!(algiers[0].tz, "Africa/Algiers");

    // A fragment many cities share is capped, not exhaustive.
    let broad = index.search("san");
    assert!(
        broad.len() <= 3,
        "a broad fragment is capped at three, got {}",
        broad.len()
    );
}

/// §9.3's zone derivation: the nearest city to Sétif's own coordinates
/// is Sétif itself, single-digit kilometres away — the `(nearest: Sétif,
/// 3 km)` line the COORDINATES mode shows before commit.
#[test]
fn nearest_at_setif_is_setif() {
    let index = cities::load().expect("the index parses");
    let (city, km) = index
        .nearest(36.19, 5.41)
        .expect("the index is never empty");
    assert_eq!(city.name, "Sétif");
    assert_eq!(city.tz, "Africa/Algiers");
    assert!(km < 10.0, "distance was {km} km");
}

/// The >200 km fallback's data: mid-ocean, `nearest` still answers (the
/// policy is the caller's — §9.3 falls back to the system zone), but
/// the distance it reports is far past the threshold the policy keys
/// on. The caller never has to guess whether it is in fallback
/// territory.
#[test]
fn mid_ocean_nearest_is_far_past_the_200km_threshold() {
    let index = cities::load().expect("the index parses");
    let (city, km) = index.nearest(0.0, -160.0).expect("nearest always answers");
    assert!(
        km > 200.0,
        "mid-Pacific is >200 km from any city, was {km} km to {}",
        city.name
    );
}

/// §9.3's AUTO-DETECT proposal: the largest city in the detected zone.
#[test]
fn largest_in_the_algeria_zone_is_algiers() {
    let index = cities::load().expect("the index parses");
    let algiers = index
        .largest_in_zone("Africa/Algiers")
        .expect("the zone has cities");
    assert_eq!(algiers.name, "Algiers");
    // A zone with no cities is an honest None, not a guess.
    assert!(index.largest_in_zone("Mars/Olympus_Mons").is_none());
}

/// §9.3's chain resolves on this machine — the same test-machine rule
/// the unit suite applies, asserted through the public entry point the
/// LOCATION screen calls on entry to AUTO-DETECT.
#[test]
fn the_system_timezone_chain_resolves_here() {
    let tz = system_timezone().expect("this machine's zone is detectable");
    // A real IANA id, not a sentinel — `Area/Location`.
    assert!(tz.name().contains('/'), "got {}", tz.name());
}
