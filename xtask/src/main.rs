//! Workspace task runner (§19): `gen-cities` · `screenshots` ·
//! `check-fixtures`.
//!
//! Each subcommand arrives with the phase that needs it (`gen-cities`
//! with the location phase, `screenshots` with Phase 15); until then
//! they answer honestly with a non-zero exit rather than pretending to
//! work. `check-fixtures` arrived with Phase 2 — it automates the
//! monthly AlAdhan drift check (§7.8, §23 Phase 2 item 9).

#![deny(clippy::unwrap_used, clippy::expect_used)]
#![deny(clippy::todo, clippy::unimplemented, clippy::dbg_macro)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::Value;

const USAGE: &str = "\
usage: cargo xtask <task>

tasks:
  gen-cities      regenerate salat/assets/cities.bin from GeoNames (§9.2)
  screenshots     regenerate docs/screenshots/ at each breakpoint (§22.4)
  check-fixtures  re-capture the AlAdhan endpoints live and compare the
                  response shape against the committed fixtures (§7.8) —
                  the monthly drift check; needs network + curl
";

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("gen-cities") => match gen_cities() {
            Ok(report) => {
                println!("xtask: {report}");
                ExitCode::SUCCESS
            }
            Err(report) => {
                eprintln!("xtask: {report}");
                ExitCode::from(1)
            }
        },
        Some("screenshots") => {
            eprintln!("xtask: screenshots is not implemented yet (arrives in Phase 15)");
            ExitCode::from(1)
        }
        Some("check-fixtures") => match check_fixtures() {
            Ok(()) => {
                println!("xtask: all 5 live endpoints still match the committed fixtures (§7.8)");
                ExitCode::SUCCESS
            }
            Err(report) => {
                eprintln!("xtask: {report}");
                ExitCode::from(1)
            }
        },
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// The canonical §7.2 request that captured the fixtures — the default
/// configuration (Sétif, method 12, `Africa/Algiers`, zero tune).
const CANONICAL_QUERY: &str = "latitude=36.19&longitude=5.41&method=12&school=0&latitudeAdjustmentMethod=3&midnightMode=0&timezonestring=Africa/Algiers&tune=0,0,0,0,0,0,0,0,0&iso8601=true";

// ─── gen-cities (§9.2) ───────────────────────────────────────────────────────

/// One city as it comes out of the GeoNames dump, before encoding.
struct CityRecord {
    name: String,
    cc: String,
    country: String,
    /// Degrees, multiplied by 1e6 and rounded at encode time.
    lat: f64,
    lng: f64,
    tz: String,
    population: u32,
}

/// Regenerate `salat/assets/cities.bin` from GeoNames `cities15000.txt`
/// (§9.2): population ≥ 15,000, country names joined from
/// `countryInfo.txt`, one tz table, everything little-endian. The source
/// is a local path passed as the argument, or a live download when none
/// is — regeneration must not depend on remembering where a dump was
/// saved.
fn gen_cities() -> Result<String, String> {
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../salat/assets/cities.bin");
    let (cities_source, countries_source) = sources()?;

    let countries = read_country_names(&countries_source)?;
    let mut cities = read_cities(&cities_source, &countries)?;
    // Deterministic output regardless of the dump's own row order: sort
    // by name then country code, so the same snapshot always encodes to
    // the same bytes.
    cities.sort_by(|a, b| (&a.name, &a.cc).cmp(&(&b.name, &b.cc)));

    let bytes = encode(&cities);
    // The self-check: the encoder's own accounting must equal the byte
    // vector it produced, or the format and the writer have drifted
    // apart.
    let expected_len = encoded_len(&cities);
    if bytes.len() != expected_len {
        return Err(format!(
            "internal: encoded {} bytes but accounted for {expected_len}",
            bytes.len()
        ));
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create assets/: {e}"))?;
    }
    std::fs::write(&out, &bytes).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    Ok(format!(
        "{} cities · {} zones · {} bytes written to {} (§9.2)",
        cities.len(),
        distinct_zones(&cities),
        bytes.len(),
        out.display()
    ))
}

/// Where to read the two GeoNames files from: explicit paths when given
/// (`gen-cities <cities15000.txt> <countryInfo.txt>`), otherwise a live
/// download into a temp directory. The dump is CC-BY 4.0 — see
/// LICENSE-THIRD-PARTY.
fn sources() -> Result<(PathBuf, PathBuf), String> {
    let mut args = std::env::args().skip(2);
    if let (Some(cities), Some(countries)) = (args.next(), args.next()) {
        return Ok((PathBuf::from(cities), PathBuf::from(countries)));
    }
    let dir = std::env::temp_dir().join("salat-gen-cities");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let zip = dir.join("cities15000.zip");
    fetch(
        "https://download.geonames.org/export/dump/cities15000.zip",
        &zip,
    )?;
    // `-p` streams the one member to stdout; the archive is never
    // extracted next to anything it could clobber.
    let txt = dir.join("cities15000.txt");
    let unzip = Command::new("unzip")
        .args([
            "-p",
            zip.to_str().ok_or("a non-UTF-8 temp path")?,
            "cities15000.txt",
        ])
        .output()
        .map_err(|e| format!("cannot run unzip (is it installed?): {e}"))?;
    if !unzip.status.success() {
        return Err(format!(
            "unzip failed: {}",
            String::from_utf8_lossy(&unzip.stderr).trim()
        ));
    }
    std::fs::write(&txt, &unzip.stdout)
        .map_err(|e| format!("cannot write {}: {e}", txt.display()))?;
    let countries = dir.join("countryInfo.txt");
    fetch(
        "https://download.geonames.org/export/dump/countryInfo.txt",
        &countries,
    )?;
    Ok((txt, countries))
}

/// Download `url` to `to`, following redirects, with a bounded wait.
fn fetch(url: &str, to: &Path) -> Result<(), String> {
    let output = Command::new("curl")
        .args(["-sSL", "--max-time", "120", "-o"])
        .arg(to)
        .arg(url)
        .output()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "curl {url} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// ISO code → country name, from `countryInfo.txt`'s columns 0 and 4.
fn read_country_names(path: &Path) -> Result<std::collections::BTreeMap<String, String>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut names = std::collections::BTreeMap::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() > 4 {
            names.insert(fields[0].to_owned(), fields[4].to_owned());
        }
    }
    Ok(names)
}

/// The §9.2 city rows: name (1), latitude (4), longitude (5), country
/// code (8), population (14), timezone (17) — keeping only population
/// ≥ 15,000 rows that carry a timezone and a known country.
fn read_cities(
    path: &Path,
    countries: &std::collections::BTreeMap<String, String>,
) -> Result<Vec<CityRecord>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut cities = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 18 {
            continue;
        }
        let population: u32 = match fields[14].parse() {
            Ok(population) => population,
            Err(_) => continue,
        };
        if population < 15_000 || fields[17].is_empty() {
            continue;
        }
        let Some(country) = countries.get(fields[8]) else {
            continue;
        };
        let (Ok(lat), Ok(lng)) = (fields[4].parse::<f64>(), fields[5].parse::<f64>()) else {
            continue;
        };
        // The format's fixed-point and one-byte lengths must both hold:
        // skip (not truncate) any row that would break the parser.
        if !(-90.0..=90.0).contains(&lat)
            || !(-180.0..=180.0).contains(&lng)
            || fields[1].len() > u8::MAX as usize
            || country.len() > u8::MAX as usize
            || fields[17].len() > u8::MAX as usize
        {
            continue;
        }
        cities.push(CityRecord {
            name: fields[1].to_owned(),
            cc: fields[8].to_owned(),
            country: country.clone(),
            lat,
            lng,
            tz: fields[17].to_owned(),
            population,
        });
    }
    if cities.is_empty() {
        return Err(
            "no cities survived the population filter — the source is not a cities15000 dump"
                .to_owned(),
        );
    }
    Ok(cities)
}

/// The distinct zone count, for the report.
fn distinct_zones(cities: &[CityRecord]) -> usize {
    let zones: BTreeSet<&str> = cities.iter().map(|city| city.tz.as_str()).collect();
    zones.len()
}

/// The §9.2 encoding: `SLTC` · u16 version · u32 city_count · u16
/// tz_count, then the tz table, then the city table — all integers
/// little-endian, the salat parser's only consumer.
fn encode(cities: &[CityRecord]) -> Vec<u8> {
    let zones: Vec<&str> = {
        let set: BTreeSet<&str> = cities.iter().map(|city| city.tz.as_str()).collect();
        set.into_iter().collect()
    };
    let zone_index: std::collections::BTreeMap<&str, u16> = zones
        .iter()
        .enumerate()
        .map(|(index, zone)| (*zone, index as u16))
        .collect();

    let mut bytes = Vec::with_capacity(encoded_len(cities));
    bytes.extend_from_slice(b"SLTC");
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&(cities.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(zones.len() as u16).to_le_bytes());
    for zone in &zones {
        bytes.push(zone.len() as u8);
        bytes.extend_from_slice(zone.as_bytes());
    }
    for city in cities {
        bytes.push(city.name.len() as u8);
        bytes.extend_from_slice(city.name.as_bytes());
        bytes.push(city.cc.len() as u8);
        bytes.extend_from_slice(city.cc.as_bytes());
        bytes.push(city.country.len() as u8);
        bytes.extend_from_slice(city.country.as_bytes());
        bytes.extend_from_slice(&e6(city.lat).to_le_bytes());
        bytes.extend_from_slice(&e6(city.lng).to_le_bytes());
        bytes.extend_from_slice(&zone_index[city.tz.as_str()].to_le_bytes());
        bytes.extend_from_slice(&city.population.to_le_bytes());
    }
    bytes
}

/// The format version this encoder writes; bump only with a reader that
/// understands the new shape (§8.4's rule, applied to the index too).
const VERSION: u16 = 1;

/// Degrees into the i32 fixed-point the format carries.
fn e6(degrees: f64) -> i32 {
    (degrees * 1e6).round() as i32
}

/// The byte count `encode` will produce — the encoder's self-check and
/// the capacity hint share one arithmetic truth.
fn encoded_len(cities: &[CityRecord]) -> usize {
    let zones: BTreeSet<&str> = cities.iter().map(|city| city.tz.as_str()).collect();
    let mut len = 4 + 2 + 4 + 2;
    for zone in zones {
        len += 1 + zone.len();
    }
    for city in cities {
        len += 1 + city.name.len() + 1 + city.cc.len() + 1 + city.country.len() + 4 + 4 + 2 + 4;
    }
    len
}

/// §7.8 / §23 Phase 2 item 9, automated: one live `curl` per recorded
/// endpoint, compared against the committed fixture. `code` and `status`
/// must match exactly; `data` is compared by **shape** (field names,
/// types, array element shapes) because its values legitimately drift by
/// minutes as the year advances — that is not a schema change. Any
/// difference reported here is: update the fixture and the DTOs in the
/// same commit, and record the new capture date in the fixtures README.
///
/// Two fixtures are not re-checked:
/// - `malformed_truncated.json` — a synthetic corruption (the calendar
///   capture cut at byte 20000), not a live response.
/// - `error_latitude_503.json` — the API's answer to `latitude=999` is
///   not stable: it was a fabricated 503 at capture time (2026-09-05)
///   and a 200 with fabricated times hours later. That instability is
///   exactly why §7.4 rejects out-of-range coordinates client-side; the
///   fixture stays as recorded evidence, checked by the DTO tests.
fn check_fixtures() -> Result<(), String> {
    let base = "https://api.aladhan.com";
    let checks: [(&str, String); 5] = [
        (
            "calendar_ok.json",
            format!("{base}/v1/calendar/2026/09?{CANONICAL_QUERY}"),
        ),
        (
            "timings_ok.json",
            format!("{base}/v1/timings/05-09-2026?{CANONICAL_QUERY}"),
        ),
        ("qibla.json", format!("{base}/v1/qibla/36.19/5.41")),
        ("methods.json", format!("{base}/v1/methods")),
        (
            "error_geocode_400.json",
            // The recorded capture: a bogus city on the endpoint salat
            // itself never calls — kept as evidence of the polymorphic
            // `data` field (§7.3).
            format!("{base}/v1/timingsByCity?city=zzzznotacity&country=zzzz"),
        ),
    ];

    let fixtures = fixture_dir();
    let mut drifts = Vec::new();
    for (name, url) in &checks {
        match check_one(&fixtures.join(name), url) {
            Ok(()) => println!("  ok  {name}"),
            Err(problem) => drifts.push(format!("  drift  {name}: {problem}")),
        }
    }

    if drifts.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "schema drift detected (§7.8) — update the fixtures and the DTOs in the same commit:\n{}",
            drifts.join("\n")
        ))
    }
}

fn check_one(fixture: &Path, url: &str) -> Result<(), String> {
    let committed = std::fs::read_to_string(fixture)
        .map_err(|e| format!("cannot read the committed fixture: {e}"))?;
    let expected: Value = serde_json::from_str(&committed)
        .map_err(|e| format!("the committed fixture is not valid JSON: {e}"))?;
    let live = curl(url)?;
    let actual: Value =
        serde_json::from_str(&live).map_err(|e| format!("the live response is not JSON: {e}"))?;
    drift("root", &expected, &actual).map_or(Ok(()), Err)
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../salat/tests/fixtures/aladhan")
}

/// Fetch a URL with `curl`. No `-f`: the error fixtures are HTTP 400/503
/// by design, and their bodies are what gets compared.
fn curl(url: &str) -> Result<String, String> {
    let output = Command::new("curl")
        .args(["-sS", "--max-time", "20", url])
        .output()
        .map_err(|e| format!("cannot run curl (is it installed?): {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "curl exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("the response is not UTF-8: {e}"))
}

/// Compare two §7.3 envelopes: `code` and `status` by value, `data` by
/// shape. Returns the first difference as a human-readable path, or
/// `None` when the shapes agree.
fn drift(path: &str, expected: &Value, actual: &Value) -> Option<String> {
    let (expected, actual) = (envelope_of(expected), envelope_of(actual));
    let (Some((e_code, e_status, e_data)), Some((a_code, a_status, a_data))) = (expected, actual)
    else {
        return Some(format!(
            "{path}: not a {{code, status, data}} envelope on both sides"
        ));
    };
    if e_code != a_code {
        return Some(format!("{path}.code: expected {e_code}, got {a_code}"));
    }
    if e_status != a_status {
        return Some(format!(
            "{path}.status: expected \"{e_status}\", got \"{a_status}\""
        ));
    }
    shape_drift(&format!("{path}.data"), e_data, a_data)
}

/// The `{code, status, data}` triple of a §7.3 envelope, or `None` when
/// the value is not one.
fn envelope_of(value: &Value) -> Option<(&Value, &Value, &Value)> {
    let object = value.as_object()?;
    Some((
        object.get("code")?,
        object.get("status")?,
        object.get("data")?,
    ))
}

/// Structural comparison: same field names, same types, same array
/// element shapes. Values are ignored — they drift by design.
fn shape_drift(path: &str, expected: &Value, actual: &Value) -> Option<String> {
    use Value::{Array, Bool, Null, Number, Object, String as JsonString};

    match (expected, actual) {
        (Null, Null)
        | (Bool(_), Bool(_))
        | (Number(_), Number(_))
        | (JsonString(_), JsonString(_)) => None,
        (Array(expected_items), Array(actual_items)) => {
            if expected_items.len() != actual_items.len() {
                return Some(format!(
                    "{path}: array length changed from {} to {}",
                    expected_items.len(),
                    actual_items.len()
                ));
            }
            let expected_shapes: BTreeSet<String> = expected_items.iter().map(shape).collect();
            let actual_shapes: BTreeSet<String> = actual_items.iter().map(shape).collect();
            if expected_shapes != actual_shapes {
                return Some(format!("{path}: array element shape changed"));
            }
            None
        }
        (Object(expected_map), Object(actual_map)) => {
            for key in expected_map.keys() {
                if !actual_map.contains_key(key) {
                    return Some(format!(
                        "{path}.{key}: field disappeared from the live response"
                    ));
                }
            }
            for key in actual_map.keys() {
                if !expected_map.contains_key(key) {
                    return Some(format!("{path}.{key}: new field in the live response"));
                }
            }
            for (key, value) in expected_map {
                if let Some(found) = shape_drift(&format!("{path}.{key}"), value, &actual_map[key])
                {
                    return Some(found);
                }
            }
            None
        }
        (expected, actual) => Some(format!(
            "{path}: type changed from {} to {}",
            kind(expected),
            kind(actual)
        )),
    }
}

/// A value's shape as a comparable string: types for leaves, sorted
/// `key:type` pairs for objects, the set of distinct element shapes for
/// arrays (a calendar month's days all share one shape).
fn shape(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(_) => "bool".to_owned(),
        Value::Number(_) => "number".to_owned(),
        Value::String(_) => "string".to_owned(),
        Value::Array(items) => {
            let shapes: BTreeSet<String> = items.iter().map(shape).collect();
            format!("[{}]", shapes.into_iter().collect::<Vec<_>>().join("|"))
        }
        Value::Object(map) => {
            let mut fields: Vec<String> = map
                .iter()
                .map(|(key, value)| format!("{key}:{}", shape(value)))
                .collect();
            fields.sort();
            format!("{{{}}}", fields.join(","))
        }
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}
