//! The embedded city index (§9.2): the committed `cities.bin`, parsed
//! lazily into borrowed views — no allocation of 34,000 owned structs.
//!
//! The format is `xtask gen-cities`' output: a 12-byte header —
//! `SLTC` · u16 version · u32 city_count · u16 tz_count — then the tz
//! table, then the city table, every integer little-endian. Both sides
//! of the format live in this repository, and a unit test parses the
//! real embedded bytes, so a mismatch is caught before it can ship.
//!
//! Search is case- and diacritic-insensitive (`Setif` finds `Sétif`),
//! a substring match on the name and on `"name, country"`, ranked
//! prefix-match first then population descending, top 3 — the design's
//! exact result count (§9.3). A linear scan over 34,000 entries folds
//! one character at a time into a reused buffer, so a keystroke costs
//! no per-city allocation and stays well under a millisecond.

use std::sync::OnceLock;

/// The index bytes as `gen-cities` wrote them (§9.2).
const CITIES_BIN: &[u8] = include_bytes!("../../assets/cities.bin");

/// `SLTC` — the format's magic (§9.2).
const MAGIC: &[u8; 4] = b"SLTC";

/// The version this parser understands; bump only with a matching
/// encoder (§8.4's rule, applied to the index too).
const VERSION: u16 = 1;

/// One city as it appears in a result — owned, because results are at
/// most three and they cross into screen state (§23 Phase 11 item 5).
#[derive(Debug, Clone, PartialEq)]
pub struct CityMatch {
    /// The city's own name, diacritics intact (`Sétif`).
    pub name: String,
    /// The country name (`Algeria`).
    pub country: String,
    /// The ISO 3166-1 alpha-2 code (`DZ`).
    pub cc: String,
    /// Latitude in degrees, `-90..=90`.
    pub lat: f64,
    /// Longitude in degrees, `-180..=180`.
    pub lng: f64,
    /// The IANA zone name (`Africa/Algiers`).
    pub tz: String,
    /// GeoNames population, for ranking and the auto-detect pick.
    pub population: u32,
}

impl CityMatch {
    /// The label the config and the design's result rows carry:
    /// `Sétif, Algeria`.
    pub fn label(&self) -> String {
        format!("{}, {}", self.name, self.country)
    }
}

/// A parse failure — the committed asset is corrupt or from a newer
/// format. Unreachable with a binary built from this repository (the
/// encoder and this parser ship together), which is why it carries
/// detail rather than a recovery path: the honest surface is `E001`
/// and the log.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum IndexError {
    /// Not `SLTC`.
    #[error("bad magic — not a cities.bin file")]
    BadMagic,
    /// A version this parser predates.
    #[error("format version {found}, this build understands {VERSION}")]
    BadVersion { found: u16 },
    /// The tables end before the header's counts say they should.
    #[error("truncated at byte {at}")]
    Truncated { at: usize },
    /// A string field is not UTF-8.
    #[error("non-UTF-8 string at byte {at}")]
    NotUtf8 { at: usize },
}

/// The parsed index. Every string borrows from the linked bytes, so the
/// whole structure is `Copy`-cheap views and one parse serves the
/// process (§9.2: parsed lazily on first search).
#[derive(Debug)]
pub struct CityIndex<'a> {
    zones: Vec<&'a str>,
    cities: Vec<RawCity<'a>>,
}

/// One city's borrowed fields, in file order.
#[derive(Debug)]
struct RawCity<'a> {
    name: &'a str,
    cc: &'a str,
    country: &'a str,
    lat_e6: i32,
    lng_e6: i32,
    tz_index: u16,
    population: u32,
}

impl RawCity<'_> {
    /// Latitude in degrees.
    fn lat(&self) -> f64 {
        f64::from(self.lat_e6) / 1e6
    }

    /// Longitude in degrees.
    fn lng(&self) -> f64 {
        f64::from(self.lng_e6) / 1e6
    }

    /// The owned result form, for the at-most-three matches that leave
    /// the index.
    fn to_match(&self, tz: &str) -> CityMatch {
        CityMatch {
            name: self.name.to_owned(),
            country: self.country.to_owned(),
            cc: self.cc.to_owned(),
            lat: self.lat(),
            lng: self.lng(),
            tz: tz.to_owned(),
            population: self.population,
        }
    }
}

/// A little-endian cursor over the asset — the one place the format's
/// arithmetic lives on this side.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], IndexError> {
        let end = self.at + len;
        if end > self.bytes.len() {
            return Err(IndexError::Truncated {
                at: self.bytes.len(),
            });
        }
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, IndexError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, IndexError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, IndexError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn i32(&mut self) -> Result<i32, IndexError> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// A length-prefixed UTF-8 string. Invalid UTF-8 cannot occur in a
    /// file this repository generated, but it is checked rather than
    /// assumed — a lossy conversion would silently corrupt a name.
    fn string(&mut self) -> Result<&'a str, IndexError> {
        let len = self.u8()? as usize;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes).map_err(|_| IndexError::NotUtf8 { at: self.at })
    }
}

impl CityIndex<'_> {
    /// Parse the §9.2 format. The trailing-bytes check is strict: the
    /// encoder writes exactly what the header counts.
    pub fn parse(bytes: &[u8]) -> Result<CityIndex<'_>, IndexError> {
        let mut cursor = Cursor::new(bytes);
        if cursor.take(4)? != MAGIC {
            return Err(IndexError::BadMagic);
        }
        let version = cursor.u16()?;
        if version != VERSION {
            return Err(IndexError::BadVersion { found: version });
        }
        let city_count = cursor.u32()? as usize;
        let tz_count = cursor.u16()? as usize;

        let mut zones = Vec::with_capacity(tz_count);
        for _ in 0..tz_count {
            zones.push(cursor.string()?);
        }
        let mut cities = Vec::with_capacity(city_count);
        for _ in 0..city_count {
            let name = cursor.string()?;
            let cc = cursor.string()?;
            let country = cursor.string()?;
            let lat_e6 = cursor.i32()?;
            let lng_e6 = cursor.i32()?;
            let tz_index = cursor.u16()?;
            let population = cursor.u32()?;
            if tz_index as usize >= zones.len() {
                return Err(IndexError::Truncated { at: cursor.at });
            }
            cities.push(RawCity {
                name,
                cc,
                country,
                lat_e6,
                lng_e6,
                tz_index,
                population,
            });
        }
        if cursor.at != bytes.len() {
            return Err(IndexError::Truncated { at: cursor.at });
        }
        Ok(CityIndex { zones, cities })
    }

    /// Search the index (§9.2): case- and diacritic-insensitive
    /// substring on the name and on `"name, country"`, ranked prefix
    /// first then population descending, top 3.
    pub fn search(&self, query: &str) -> Vec<CityMatch> {
        let mut needle = Vec::new();
        fold_into(query, &mut needle);
        if needle.is_empty() {
            return Vec::new();
        }

        let mut haystack = Vec::with_capacity(128);
        let mut matches: Vec<(bool, u32, &RawCity)> = Vec::new();
        for city in &self.cities {
            // The name alone decides prefix rank; both it and the
            // comma form decide eligibility.
            haystack.clear();
            fold_into(city.name, &mut haystack);
            let name_prefix = haystack.starts_with(&needle);
            let name_contains = name_prefix || contains(&haystack, &needle);
            let eligible = if name_contains {
                true
            } else {
                haystack.push(',');
                haystack.push(' ');
                fold_into(city.country, &mut haystack);
                contains(&haystack, &needle)
            };
            if eligible {
                matches.push((name_prefix, city.population, city));
            }
        }
        matches.sort_by(|a, b| {
            // Prefix beats substring; within a class, population
            // descending; the name breaks ties so one snapshot of the
            // data always produces one order.
            b.0.cmp(&a.0)
                .then(b.1.cmp(&a.1))
                .then(a.2.name.cmp(b.2.name))
        });
        matches
            .into_iter()
            .take(3)
            .map(|(_, _, city)| city.to_match(self.zones[city.tz_index as usize]))
            .collect()
    }

    /// The nearest city by great-circle distance, and the distance in
    /// kilometres (§9.3's COORDINATES mode: the zone derivation and its
    /// `(nearest: Sétif, 3 km)` honesty).
    pub fn nearest(&self, lat: f64, lng: f64) -> Option<(CityMatch, f64)> {
        let mut best: Option<(f64, &RawCity)> = None;
        for city in &self.cities {
            let distance = haversine(lat, lng, city.lat(), city.lng());
            if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                best = Some((distance, city));
            }
        }
        best.map(|(distance, city)| (city.to_match(self.zones[city.tz_index as usize]), distance))
    }

    /// The highest-population city in a zone (§9.3's AUTO-DETECT
    /// proposal).
    pub fn largest_in_zone(&self, zone: &str) -> Option<CityMatch> {
        let mut best: Option<&RawCity> = None;
        for city in &self.cities {
            if self.zones[city.tz_index as usize] != zone {
                continue;
            }
            if best.is_none_or(|best| city.population > best.population) {
                best = Some(city);
            }
        }
        best.map(|city| city.to_match(self.zones[city.tz_index as usize]))
    }
}

/// The embedded index, parsed once on first use (§9.2's laziness).
/// Failing here means the committed asset is corrupt — the encoder and
/// parser ship together, so the caller maps this to `E001` and the log
/// keeps the detail.
pub fn load() -> Result<&'static CityIndex<'static>, IndexError> {
    static INDEX: OnceLock<Result<CityIndex<'static>, IndexError>> = OnceLock::new();
    INDEX
        .get_or_init(|| CityIndex::parse(CITIES_BIN))
        .as_ref()
        .map_err(Clone::clone)
}

/// Whether the folded haystack contains the folded needle.
fn contains(haystack: &[char], needle: &[char]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Fold one string into `out`: lowercased, Latin diacritics stripped.
/// One `char` can fold to a short sequence (`æ` → `ae`, `ß` → `ss`),
/// which is why the output is a `Vec<char>` and not a `char` map.
///
/// The table covers Latin-1 Supplement, Latin Extended-A and the few
/// Latin Extended-B letters GeoNames city names actually carry — every
/// European diacritic in the index. Other scripts pass through
/// lowercased, unchanged: they match themselves exactly, which is the
/// honest behaviour for scripts that have no ASCII fold.
fn fold_into(text: &str, out: &mut Vec<char>) {
    for character in text.chars() {
        match character {
            'Æ' => out.extend(['a', 'e']),
            'æ' => out.extend(['a', 'e']),
            'ß' => out.extend(['s', 's']),
            'Œ' => out.extend(['o', 'e']),
            'œ' => out.extend(['o', 'e']),
            'Đ' | 'đ' => out.push('d'),
            'Ł' | 'ł' => out.push('l'),
            'Ø' | 'ø' => out.push('o'),
            'Þ' | 'þ' => out.push('t'),
            'Ð' | 'ð' => out.push('d'),
            // The Extended-B digraphs — two letters each, like `æ`.
            'Ǆ' | 'ǅ' | 'ǆ' => out.extend(['d', 'z']),
            'Ǉ' | 'ǈ' | 'ǉ' => out.extend(['l', 'j']),
            'Ǌ' | 'ǋ' | 'ǌ' => out.extend(['n', 'j']),
            'Ǳ' | 'ǲ' | 'ǳ' => out.extend(['d', 'z']),
            'Ǣ' | 'ǣ' | 'Ǽ' | 'ǽ' => out.extend(['a', 'e']),
            c if c.is_ascii_uppercase() => out.push(c.to_ascii_lowercase()),
            c @ '\u{00C0}'..='\u{00FF}' => out.extend(c.to_lowercase().map(strip_latin1)),
            c @ '\u{0100}'..='\u{017F}' => out.extend(c.to_lowercase().map(strip_latin_ext_a)),
            c @ '\u{0180}'..='\u{024F}' => out.extend(c.to_lowercase().map(strip_latin_ext_b)),
            c => out.extend(c.to_lowercase()),
        }
    }
}

/// Strip the diacritic of a Latin-1 letter, already lowercase: `é` →
/// `e`. The two non-letters in the range — the multiplication and
/// division signs — have no fold and map to themselves.
fn strip_latin1(c: char) -> char {
    match c {
        'à'..='å' => 'a',
        'è'..='ë' => 'e',
        'ì'..='ï' => 'i',
        'ò'..='ö' => 'o',
        'ù'..='ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'ñ' => 'n',
        'ç' => 'c',
        _ => c,
    }
}

/// Strip the diacritic of a Latin Extended-A letter (the range
/// `ā ă ć č ď ē ...`), already lowercase, mapping to the base letter
/// where one exists and passing through otherwise. Enumerated, not
/// derived — computing it from the codepoint arithmetic would be
/// clever and wrong.
fn strip_latin_ext_a(c: char) -> char {
    match c {
        'ā' | 'ă' | 'ą' => 'a',
        'ć' | 'ĉ' | 'ċ' | 'č' => 'c',
        'ď' => 'd',
        'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => 'g',
        'ĥ' | 'ħ' => 'h',
        'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => 'i',
        'ĵ' => 'j',
        'ķ' => 'k',
        'ĺ' | 'ļ' | 'ľ' => 'l',
        'ń' | 'ņ' | 'ň' => 'n',
        'ō' | 'ŏ' => 'o',
        'ŕ' | 'ŗ' | 'ř' => 'r',
        'ś' | 'ŝ' | 'ş' | 'š' => 's',
        'ţ' | 'ť' => 't',
        'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
        'ŵ' => 'w',
        'ŷ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        _ => c,
    }
}

/// Strip the diacritic of a Latin Extended-B letter (the range
/// `ǎ ǐ Ǒ ǎ ș ț ...`), already lowercase, mapping to the base letter
/// where one exists and passing through otherwise. The same rule as
/// [`strip_latin_ext_a`] — the letters Romanian (`Chișinău`,
/// `Bălți`), Vietnamese and transliteration schemes spell with.
fn strip_latin_ext_b(c: char) -> char {
    match c {
        'ƀ' | 'ƃ' | 'ƅ' => 'b',
        'ƈ' | 'ȼ' => 'c',
        'ƌ' => 'd',
        'ƒ' => 'f',
        'ƙ' => 'k',
        'ƞ' => 'n',
        'ǎ' | 'ǟ' | 'ǡ' | 'ǻ' | 'ȁ' | 'ȃ' | 'ȧ' => 'a',
        'ȇ' | 'ȅ' | 'ȩ' | 'ɇ' => 'e',
        'ǐ' | 'ȉ' | 'ȋ' => 'i',
        'ǒ' | 'ǭ' | 'ǫ' | 'ȍ' | 'ȏ' | 'ȫ' | 'ȭ' | 'ȯ' | 'ȱ' | 'ǿ' => 'o',
        'ǔ' | 'ǖ' | 'ǘ' | 'ǚ' | 'ǜ' | 'ȕ' | 'ȗ' => 'u',
        'ȑ' | 'ȓ' => 'r',
        'ș' => 's',
        'ț' => 't',
        'ǥ' | 'ǧ' | 'ǵ' => 'g',
        'ȟ' => 'h',
        'ǩ' => 'k',
        'ǹ' => 'n',
        'ȳ' => 'y',
        'ȥ' => 'z',
        _ => c,
    }
}

/// Great-circle distance in kilometres (the mean-earth haversine —
/// ±0.3 % is far below the 200 km decision threshold it feeds).
fn haversine(lat1: f64, lng1: f64, lat2: f64, lng2: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6371.0;
    let (lat1, lng1, lat2, lng2) = (
        lat1.to_radians(),
        lng1.to_radians(),
        lat2.to_radians(),
        lng2.to_radians(),
    );
    let dlat = lat2 - lat1;
    let dlng = lng2 - lng1;
    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlng / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The committed asset parses, and its counts are what gen-cities
    /// reported — the two sides of the format can never drift apart
    /// silently.
    #[test]
    fn the_embedded_index_parses() {
        let index = load().expect("the committed cities.bin");
        assert_eq!(index.zones.len(), 322);
        assert_eq!(index.cities.len(), 34_090);
    }

    /// §9.2's own example: `Setif` finds `Sétif` — diacritic- and
    /// case-insensitive, top of the list.
    #[test]
    fn setif_finds_setif() {
        let index = load().expect("the committed cities.bin");
        let results = index.search("Setif");
        let top = results.first().expect("at least one match");
        assert_eq!(top.name, "Sétif");
        assert_eq!(top.country, "Algeria");
        assert_eq!(top.tz, "Africa/Algiers");
    }

    /// The curated extras ride along in the committed asset: Tamokra
    /// (Béjaïa, Algeria) carries population 0 in GeoNames, so no
    /// `cities*` dump can ever include it — it is hand-verified from
    /// the DZ dump (geonameid 2478073) and merged by gen-cities, and
    /// the search returns its exact record.
    #[test]
    fn curated_tamokra_is_searchable_with_exact_coordinates() {
        let index = load().expect("the committed cities.bin");
        let results = index.search("Tamokra");
        let top = results.first().expect("the curated match");
        assert_eq!(top.name, "Tamokra");
        assert_eq!(top.country, "Algeria");
        assert_eq!(top.cc, "DZ");
        assert_eq!(top.tz, "Africa/Algiers");
        assert!(
            (top.lat - 36.39322).abs() < 1e-6,
            "latitude was {}",
            top.lat
        );
        assert!(
            (top.lng - 4.66446).abs() < 1e-6,
            "longitude was {}",
            top.lng
        );
        // And a coordinate entry at Tamokra's own point now derives
        // its nearest city honestly: Tamokra itself, zero kilometres.
        let (city, km) = index.nearest(36.39322, 4.66446).expect("a nearest city");
        assert_eq!(city.name, "Tamokra");
        assert!(km < 1.0, "distance was {km} km");
    }

    /// The comma form matches too — `Setif, Algeria` and a country
    /// fragment both find the city.
    #[test]
    fn the_comma_form_and_country_fragments_match() {
        let index = load().expect("the committed cities.bin");
        assert!(
            index
                .search("setif, algeria")
                .first()
                .is_some_and(|m| m.name == "Sétif")
        );
        assert!(index.search("algeria").len() <= 3);
    }

    /// At most three results, ranked prefix-first then population
    /// descending (§9.2).
    #[test]
    fn results_are_capped_and_ranked() {
        let index = load().expect("the committed cities.bin");
        let results = index.search("san");
        assert!(results.len() <= 3);
        // Every result's folded name starts with "san".
        for result in &results {
            let mut folded = Vec::new();
            fold_into(&result.name, &mut folded);
            assert!(folded.starts_with(&['s', 'a', 'n']), "{}", result.name);
        }
    }

    /// A query with no fold-match finds nothing, and the empty query
    /// finds nothing rather than everything.
    #[test]
    fn no_match_and_empty_query() {
        let index = load().expect("the committed cities.bin");
        assert!(index.search("zzzqqqxxx").is_empty());
        assert!(index.search("").is_empty());
    }

    /// §9.3's derivation: the nearest city to Sétif's own coordinates
    /// is Sétif itself, a few kilometres out at most.
    #[test]
    fn nearest_at_setif_is_setif() {
        let index = load().expect("the committed cities.bin");
        let (city, distance) = index.nearest(36.1900, 5.4100).expect("a nearest city");
        assert_eq!(city.name, "Sétif");
        assert!(distance < 5.0, "distance was {distance} km");
    }

    /// A mid-ocean coordinate still finds *a* nearest city — the
    /// >200 km policy is the caller's, not the index's.
    #[test]
    fn nearest_exists_mid_ocean() {
        let index = load().expect("the committed cities.bin");
        let (city, distance) = index.nearest(0.0, -140.0).expect("a nearest city");
        assert!(distance > 1000.0, "mid-Pacific is far from {city:?}");
    }

    /// Auto-detect's proposal: the largest city in `Africa/Algiers`
    /// is Algiers.
    #[test]
    fn largest_in_zone_picks_algiers() {
        let index = load().expect("the committed cities.bin");
        let algiers = index
            .largest_in_zone("Africa/Algiers")
            .expect("the zone exists");
        assert_eq!(algiers.name, "Algiers");
        // An absent zone proposes nothing.
        assert!(index.largest_in_zone("Not/AZone").is_none());
    }

    /// The parser rejects what it must: bad magic, a future version,
    /// truncation — and accepts a well-formed minimal file.
    #[test]
    fn the_parser_rejects_corruption() {
        assert_eq!(
            CityIndex::parse(&[]).unwrap_err(),
            IndexError::Truncated { at: 0 }
        );
        // Four readable bytes, wrong magic — corruption, not truncation.
        assert_eq!(CityIndex::parse(b"NOTC").unwrap_err(), IndexError::BadMagic);
        // Good magic, no header left.
        assert_eq!(
            CityIndex::parse(b"SLTC").unwrap_err(),
            IndexError::Truncated { at: 4 }
        );
        // Version 2 is a future format.
        let mut future = Vec::new();
        future.extend_from_slice(b"SLTC");
        future.extend_from_slice(&2u16.to_le_bytes());
        future.extend_from_slice(&0u32.to_le_bytes());
        future.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(
            CityIndex::parse(&future).unwrap_err(),
            IndexError::BadVersion { found: 2 }
        );
        // A city count the bytes cannot back.
        let mut short = Vec::new();
        short.extend_from_slice(b"SLTC");
        short.extend_from_slice(&1u16.to_le_bytes());
        short.extend_from_slice(&1u32.to_le_bytes());
        short.extend_from_slice(&0u16.to_le_bytes());
        assert_eq!(
            CityIndex::parse(&short).unwrap_err(),
            IndexError::Truncated { at: 12 }
        );
    }

    /// A hand-built two-city index round-trips through search and
    /// nearest — the format exercised without the 1 MB asset.
    #[test]
    fn a_hand_built_index_round_trips() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"SLTC");
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(b"\x0eEurope/Andorra");
        let cities: [(&str, f64, f64, u32); 2] = [
            ("les Escaldes", 42.50729, 1.53414, 15_853),
            ("Andorra la Vella", 42.50779, 1.52109, 20_430),
        ];
        for (name, lat, lng, pop) in cities {
            bytes.push(name.len() as u8);
            bytes.extend_from_slice(name.as_bytes());
            bytes.push(2);
            bytes.extend_from_slice(b"AD");
            bytes.push(7);
            bytes.extend_from_slice(b"Andorra");
            bytes.extend_from_slice(&((lat * 1e6).round() as i32).to_le_bytes());
            bytes.extend_from_slice(&((lng * 1e6).round() as i32).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&pop.to_le_bytes());
        }
        let index = CityIndex::parse(&bytes).expect("the hand-built index");
        // The capital outranks the parish at equal prefix rank.
        let results = index.search("andorra");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "Andorra la Vella");
        assert_eq!(results[1].name, "les Escaldes");
        // Nearest from the capital's own coordinates is the capital.
        let (city, _) = index.nearest(42.50779, 1.52109).expect("a nearest city");
        assert_eq!(city.name, "Andorra la Vella");
        // Auto-detect in this zone proposes the capital too.
        assert_eq!(
            index
                .largest_in_zone("Europe/Andorra")
                .expect("the zone")
                .name,
            "Andorra la Vella"
        );
    }

    /// The fold: every accented Latin letter folds to its base, the
    /// digraphs expand, and Cyrillic passes through lowercased.
    #[test]
    fn the_fold_is_exact() {
        fn folded(text: &str) -> String {
            let mut out = Vec::new();
            fold_into(text, &mut out);
            out.into_iter().collect()
        }
        assert_eq!(folded("Sétif"), "setif");
        assert_eq!(folded("ZÜRICH"), "zurich");
        assert_eq!(folded("São Paulo"), "sao paulo");
        assert_eq!(folded("Bălți"), "balti");
        assert_eq!(folded("Chișinău"), "chisinau");
        assert_eq!(folded("Ålesund"), "alesund");
        assert_eq!(folded("æøå"), "aeoa");
        assert_eq!(folded("ß"), "ss");
        assert_eq!(folded("Łódź"), "lodz");
        assert_eq!(folded("Москва"), "москва");
        // The non-letters with no fold pass through.
        assert_eq!(folded("×÷"), "×÷");
    }
}
