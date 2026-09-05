//! §20.2 API row: the full AlAdhan failure matrix, against a wiremock
//! server on localhost — no test performs real network I/O.
//!
//! Two §7.4 rows are asserted from the **client side**: latitude 999 and
//! method 777 are rejected before any request, proven by the mock server
//! having received zero requests. The 1s/2s/4s backoff is asserted through
//! a recording `Delayer`, not by sleeping.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::NaiveDate;
use salat::api::params::Params;
use salat::api::retry::Delayer;
use salat::api::{AlAdhanClient, ApiError, HttpClient, Revalidated};
use salat::config::Config;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/aladhan")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"))
}

fn params() -> Params {
    Params::from(&Config::default())
}

/// Records every delay the retry loop requests, instead of sleeping.
struct RecordingDelayer(Mutex<Vec<Duration>>);

impl RecordingDelayer {
    fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Vec::new())))
    }

    fn delays(&self) -> Vec<Duration> {
        self.0.lock().unwrap().clone()
    }
}

#[async_trait]
impl Delayer for RecordingDelayer {
    async fn delay(&self, duration: Duration) {
        self.0.lock().unwrap().push(duration);
    }
}

/// Delayer that never sleeps — for tests whose delays are not under
/// assertion.
struct NoopDelayer;

#[async_trait]
impl Delayer for NoopDelayer {
    async fn delay(&self, _duration: Duration) {}
}

fn client(base: &str, delayer: Arc<dyn Delayer>) -> HttpClient {
    HttpClient::with_transport(
        base,
        Duration::from_secs(10),
        Duration::from_secs(20),
        delayer,
    )
    .expect("the wiremock base URL is valid")
}

async fn received_queries(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .expect("wiremock records requests by default")
        .into_iter()
        .map(|request| request.url.query().unwrap_or_default().to_owned())
        .collect()
}

/// 200: the committed calendar fixture parses, and the one request that
/// went out carries `iso8601=true` and the nine-integer `tune`
/// (§7.2 — URL-encoded commas).
#[tokio::test]
async fn calendar_200_parses_with_iso8601_and_tune_on_the_wire() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/calendar/2026/9"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("calendar_ok.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = client(&server.uri(), Arc::new(NoopDelayer));
    let days = client.calendar(2026, 9, &params()).await.unwrap();
    assert_eq!(days.len(), 30);
    assert_eq!(days[0].times.fajr.to_rfc3339(), "2026-09-01T05:12:00+01:00");
    server.verify().await;

    let queries = received_queries(&server).await;
    assert_eq!(queries.len(), 1);
    assert!(
        queries[0].contains("iso8601=true"),
        "iso8601 missing from: {}",
        queries[0]
    );
    assert!(
        queries[0].contains("tune=0%2C0%2C0%2C0%2C0%2C0%2C0%2C0%2C0"),
        "tune is not nine comma-separated integers in: {}",
        queries[0]
    );
}

/// 200 with the qibla fixture: the direction comes through.
#[tokio::test]
async fn qibla_200_returns_the_direction() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/qibla/36.19/5.41"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("qibla.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = client(&server.uri(), Arc::new(NoopDelayer));
    let direction = client.qibla(36.19, 5.41).await.unwrap();
    assert!((direction - 106.782_518_604_693_42).abs() < 1e-9);
    server.verify().await;
}

/// 400 with `data` as a bare string (§7.3): `Upstream`, E301 — and **one**
/// request: other 4xx are never retried.
#[tokio::test]
async fn error_envelope_400_is_upstream_and_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/timings/05-09-2026"))
        .respond_with(ResponseTemplate::new(400).set_body_string(fixture("error_geocode_400.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = client(&server.uri(), Arc::new(NoopDelayer));
    let date = NaiveDate::from_ymd_opt(2026, 9, 5).unwrap();
    let error = client.timings(date, &params()).await.unwrap_err();
    match &error {
        ApiError::Upstream(message) => {
            assert!(message.contains("Unable to geocode"), "{message}")
        }
        other => panic!("expected Upstream, got {other:?}"),
    }
    assert_eq!(error.to_salat_error().code(), "E301");
    server.verify().await;
}

/// §7.4's trap: `latitude=999` answers a fabricated HTTP 503 upstream —
/// the mock serves exactly that, and the client must reject the request
/// before it is ever sent.
#[tokio::test]
async fn latitude_999_is_rejected_before_any_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/calendar/2026/9"))
        .respond_with(
            ResponseTemplate::new(503).set_body_string(fixture("error_latitude_503.json")),
        )
        .mount(&server)
        .await;

    let client = client(&server.uri(), Arc::new(NoopDelayer));
    let mut p = params();
    p.latitude = 999.0;
    let error = client.calendar(2026, 9, &p).await.unwrap_err();
    match &error {
        ApiError::InvalidParams(problems) => {
            assert!(
                problems
                    .iter()
                    .any(|problem| problem.message.contains("latitude"))
            )
        }
        other => panic!("expected InvalidParams, got {other:?}"),
    }
    assert_eq!(error.to_salat_error().code(), "E101");

    // The §20.2 assertion: nothing went on the wire.
    let received = server.received_requests().await.unwrap();
    assert!(
        received.is_empty(),
        "the client sent {} request(s) despite invalid latitude",
        received.len()
    );
}

/// §7.4's other trap: an unknown method answers HTTP 200 with silently
/// wrong times upstream — `method=777` is rejected client-side, proven by
/// zero requests.
#[tokio::test]
async fn method_777_is_rejected_client_side() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/calendar/2026/9"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("calendar_ok.json")))
        .mount(&server)
        .await;

    let client = client(&server.uri(), Arc::new(NoopDelayer));
    let mut p = params();
    p.method = 777;
    let error = client.calendar(2026, 9, &p).await.unwrap_err();
    match &error {
        ApiError::InvalidParams(problems) => {
            assert!(
                problems
                    .iter()
                    .any(|problem| problem.message.contains("777"))
            )
        }
        other => panic!("expected InvalidParams, got {other:?}"),
    }
    assert_eq!(error.to_salat_error().code(), "E102");

    let received = server.received_requests().await.unwrap();
    assert!(
        received.is_empty(),
        "the client sent {} request(s) despite method 777",
        received.len()
    );
}

/// A truncated body is `Malformed`, E303 — and never retried (one
/// request): re-reading cannot fix it.
#[tokio::test]
async fn malformed_body_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/calendar/2026/9"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(fixture("malformed_truncated.json")),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client(&server.uri(), Arc::new(NoopDelayer));
    let error = client.calendar(2026, 9, &params()).await.unwrap_err();
    assert!(matches!(error, ApiError::Malformed(_)), "{error:?}");
    assert_eq!(error.to_salat_error().code(), "E303");
    server.verify().await;
}

/// A response slower than the total timeout is `Timeout`, E302, retried
/// the full three times — four requests, each carrying `iso8601=true`.
#[tokio::test]
async fn timeout_retries_then_reports_e302() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/calendar/2026/9"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(fixture("calendar_ok.json"))
                .set_delay(Duration::from_secs(5)),
        )
        .expect(4)
        .mount(&server)
        .await;

    // §7.5 mandates 10 s/20 s in production; the test seam shrinks the
    // total so a timeout costs 300 ms per attempt instead of 20 s.
    let client = HttpClient::with_transport(
        &server.uri(),
        Duration::from_secs(1),
        Duration::from_millis(300),
        Arc::new(NoopDelayer),
    )
    .unwrap();

    let error = client.calendar(2026, 9, &params()).await.unwrap_err();
    assert!(matches!(error, ApiError::Timeout(_)), "{error:?}");
    assert_eq!(error.to_salat_error().code(), "E302");
    server.verify().await;

    let queries = received_queries(&server).await;
    assert_eq!(queries.len(), 4);
    for query in &queries {
        assert!(
            query.contains("iso8601=true"),
            "iso8601 missing from: {query}"
        );
    }
}

/// A closed port is `Network`, E302 — localhost only, no real network.
#[tokio::test]
async fn connection_refused_is_network() {
    // Reserve a loopback port, then close it: nothing is listening, so
    // connecting is refused. (Dropping a wiremock server is not reliable
    // for this — its task can outlive the handle long enough to answer
    // with a 404.)
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let uri = format!("http://127.0.0.1:{port}");

    let client = client(&uri, Arc::new(NoopDelayer));
    let error = client.calendar(2026, 9, &params()).await.unwrap_err();
    assert!(matches!(error, ApiError::Network(_)), "{error:?}");
    assert_eq!(error.to_salat_error().code(), "E302");
}

/// §7.5 ETag revalidation: an `If-None-Match` that the server honors with
/// 304 is `NotModified`, not an error.
#[tokio::test]
async fn etag_revalidation_304_is_not_modified() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/calendar/2026/9"))
        .and(header("if-none-match", "\"september-2026\""))
        .respond_with(ResponseTemplate::new(304))
        .expect(1)
        .mount(&server)
        .await;

    let client = client(&server.uri(), Arc::new(NoopDelayer));
    let outcome = client
        .revalidate_calendar(2026, 9, &params(), "\"september-2026\"")
        .await
        .unwrap();
    assert_eq!(outcome, Revalidated::NotModified);
    server.verify().await;
}

/// Revalidation without a stored ETag is an unconditional fetch: no
/// `If-None-Match` header on the wire, fresh data and the response's
/// ETag back.
#[tokio::test]
async fn revalidation_without_etag_is_unconditional() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/calendar/2026/9"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(fixture("calendar_ok.json"))
                .insert_header("etag", "\"september-2026\""),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = client(&server.uri(), Arc::new(NoopDelayer));
    let outcome = client
        .revalidate_calendar(2026, 9, &params(), "")
        .await
        .unwrap();
    match outcome {
        Revalidated::Fresh(days, etag) => {
            assert_eq!(days.len(), 30);
            assert_eq!(etag, "\"september-2026\"");
        }
        Revalidated::NotModified => panic!("an unconditional fetch cannot answer 304"),
    }

    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    assert!(
        !received[0].headers.contains_key("if-none-match"),
        "no ETag was stored, so no If-None-Match may be sent"
    );
}

/// §20.2: 429, three retries (four requests total), then E304 — and the
/// backoff pattern is exactly 1s/2s/4s, asserted through the recording
/// delayer rather than the clock.
#[tokio::test]
async fn rate_limited_retries_three_times_then_reports_e304() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/calendar/2026/9"))
        .respond_with(ResponseTemplate::new(429).set_body_string("Too Many Requests"))
        .expect(4)
        .mount(&server)
        .await;

    let delayer = RecordingDelayer::new();
    let client = client(&server.uri(), delayer.clone());
    let error = client.calendar(2026, 9, &params()).await.unwrap_err();
    assert!(matches!(error, ApiError::RateLimited), "{error:?}");
    assert_eq!(error.to_salat_error().code(), "E304");
    server.verify().await;

    // Four requests: the initial attempt plus three retries.
    let queries = received_queries(&server).await;
    assert_eq!(queries.len(), 4);
    for query in &queries {
        assert!(
            query.contains("iso8601=true"),
            "iso8601 missing from: {query}"
        );
    }

    // §7.5: the schedule the ERROR screen quotes — 1s · 2s · 4s.
    assert_eq!(
        delayer.delays(),
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
        ]
    );
}
