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
        Some("gen-cities") => {
            eprintln!("xtask: gen-cities is not implemented yet (arrives with the location phase)");
            ExitCode::from(1)
        }
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
