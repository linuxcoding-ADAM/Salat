//! `AlAdhanClient` — the sole prayer provider (§7).
//!
//! One trait, two implementations: [`HttpClient`] talks to
//! `https://api.aladhan.com` under the full §7.5 transport policy
//! (timeouts, retry with 1s/2s/4s backoff, rate gate, redirect leash,
//! body cap, ETag revalidation), and [`MockClient`] records and replays
//! for the layers above. Every request passes §7.4 client-side validation
//! **before** any bytes go on the wire.

mod map;

pub mod dto;
pub mod params;
pub mod retry;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};

use crate::error::SalatError;
use dto::{ApiEnvelope, CalendarDayDto, TimingsDto, parse_calendar, parse_qibla, parse_timings};
use params::{InvalidParam, ParamKind, Params, validate_coordinates};
use retry::{Delayer, MAX_ATTEMPTS, RequestGate, StdDelayer, backoff_for, retryable};

/// §7.1: the one host this build will ever ask for prayer times.
pub const ALADHAN_BASE: &str = "https://api.aladhan.com";

/// §7.5: bodies are capped at 2 MiB — a legitimate calendar month is
/// ~44 KB, so anything bigger is a runaway response, not memory pressure.
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// §7.5: 10 s to establish the connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// §7.5: 20 s for the whole request, connection included.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);

/// What the api layer can fail with, mapped to the user-facing taxonomy
/// by [`ApiError::to_salat_error`].
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// Connection-level failure: DNS, refused, TLS (retryable, §7.5).
    #[error("connection failed: {0}")]
    Network(String),
    /// The 10 s/20 s budget expired (retryable, §7.5).
    #[error("request timed out: {0}")]
    Timeout(String),
    /// HTTP 5xx (retryable) or an unclassified non-success status.
    #[error("HTTP {0}")]
    Status(u16),
    /// HTTP 429 (retryable until the three retries are spent, then E304).
    #[error("rate limited (HTTP 429)")]
    RateLimited,
    /// The body could not be parsed (§7.3) — never retried.
    #[error("malformed response: {0}")]
    Malformed(String),
    /// The API answered a typed error envelope (§7.3, `data` as string).
    #[error("the API rejected the request: {0}")]
    Upstream(String),
    /// Rejected client-side before any request (§7.4): coordinates →
    /// E101, method → E102, anything else → E201.
    #[error("invalid parameters: {0:?}")]
    InvalidParams(Vec<InvalidParam>),
}

impl ApiError {
    /// The §18.1 mapping: transport failures → E301/E302, parse failures
    /// → E303, an exhausted 429 → E304, and §7.4 rejections the parameter
    /// codes E101/E102/E201.
    pub fn to_salat_error(&self) -> SalatError {
        match self {
            ApiError::Network(_) | ApiError::Timeout(_) => SalatError::ApiUnreachable,
            ApiError::Status(_) | ApiError::Upstream(_) => SalatError::PrayerTimesUnavailable,
            ApiError::RateLimited => SalatError::ApiRateLimited,
            ApiError::Malformed(_) => SalatError::ApiMalformed,
            ApiError::InvalidParams(problems) => {
                if problems.is_empty() {
                    SalatError::ConfigInvalid {
                        problems: Vec::new(),
                    }
                } else if problems
                    .iter()
                    .any(|problem| problem.kind == ParamKind::Coordinates)
                {
                    SalatError::InvalidCoordinates
                } else if problems
                    .iter()
                    .any(|problem| problem.kind == ParamKind::Method)
                {
                    SalatError::InvalidMethod
                } else {
                    SalatError::ConfigInvalid {
                        problems: problems
                            .iter()
                            .map(|problem| problem.message.clone())
                            .collect(),
                    }
                }
            }
        }
    }
}

/// The outcome of an ETag revalidation (§7.5): fresh data with its new
/// ETag, or "the month you already have is still valid".
#[derive(Debug, PartialEq)]
pub enum Revalidated {
    /// HTTP 200: the days, and the ETag to store for next time (empty
    /// when the response carried none).
    Fresh(Vec<CalendarDayDto>, String),
    /// HTTP 304: nothing changed.
    NotModified,
}

/// The prayer provider surface (§7): three endpoints, plus ETag
/// revalidation for explicit refreshes.
#[async_trait]
pub trait AlAdhanClient: Send + Sync {
    /// `GET /v1/calendar/{year}/{month}` — a whole month in one request.
    async fn calendar(
        &self,
        year: i32,
        month: u32,
        p: &Params,
    ) -> Result<Vec<CalendarDayDto>, ApiError>;

    /// `GET /v1/timings/{DD-MM-YYYY}` — a single day.
    async fn timings(&self, date: NaiveDate, p: &Params) -> Result<TimingsDto, ApiError>;

    /// `GET /v1/qibla/{lat}/{lng}` — the Qibla direction in degrees from
    /// true North.
    async fn qibla(&self, lat: f64, lng: f64) -> Result<f64, ApiError>;

    /// §7.5 conditional revalidation: sends `If-None-Match: {etag}` and
    /// reports 304 as [`Revalidated::NotModified`] instead of an error.
    /// An empty `etag` skips the header (an unconditional fetch that
    /// still returns the new ETag alongside the data). The caller owns
    /// ETag storage.
    async fn revalidate_calendar(
        &self,
        year: i32,
        month: u32,
        p: &Params,
        etag: &str,
    ) -> Result<Revalidated, ApiError>;
}

/// What a completed HTTP exchange looks like.
enum Executed {
    /// HTTP 200: the body and its ETag, if any.
    Ok { body: String, etag: Option<String> },
    /// HTTP 304: the stored data is still valid.
    NotModified,
}

/// The production client: HTTPS, rustls, the §7.5 transport policy.
pub struct HttpClient {
    http: reqwest::Client,
    base: reqwest::Url,
    gate: RequestGate,
    delayer: Arc<dyn Delayer>,
}

impl HttpClient {
    /// The §7.5 defaults against `https://api.aladhan.com`.
    pub fn new() -> Result<Self, ApiError> {
        Self::with_transport(
            ALADHAN_BASE,
            CONNECT_TIMEOUT,
            TOTAL_TIMEOUT,
            Arc::new(StdDelayer),
        )
    }

    /// Full control of the transport knobs — the seam the failure tests
    /// use to point at a wiremock server, shrink the total timeout so a
    /// timeout costs milliseconds, and inject a recording `Delayer` so the
    /// 1s/2s/4s pattern is asserted, not slept. Production code always
    /// goes through [`HttpClient::new`].
    pub fn with_transport(
        base_url: &str,
        connect: Duration,
        total: Duration,
        delayer: Arc<dyn Delayer>,
    ) -> Result<Self, ApiError> {
        let base = reqwest::Url::parse(base_url)
            .map_err(|e| ApiError::Malformed(format!("invalid base URL: {e}")))?;
        let host = base
            .host_str()
            .ok_or_else(|| ApiError::Malformed("base URL has no host".into()))?
            .to_owned();
        // §7.5: follow at most two redirects, and only within the API's
        // own host. `stop()` returns the redirect response itself —
        // reqwest's error type is not publicly constructible — which then
        // surfaces as a non-success status.
        let redirects = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 2 {
                attempt.stop()
            } else if attempt.url().host_str() == Some(host.as_str()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        });
        let http = reqwest::Client::builder()
            .user_agent(format!(
                "salat/{} (+https://github.com/linuxcoding-ADAM/salat)",
                env!("CARGO_PKG_VERSION")
            ))
            .connect_timeout(connect)
            .timeout(total)
            .redirect(redirects)
            .build()
            .map_err(|e| ApiError::Malformed(format!("could not build HTTP client: {e}")))?;
        Ok(Self {
            http,
            base,
            gate: RequestGate::new(),
            delayer,
        })
    }

    /// `path` (leading `/`) under the base, with the query pairs
    /// URL-encoded.
    fn url_for(&self, path: &str, pairs: &[(&'static str, String)]) -> reqwest::Url {
        let mut url = self.base.clone();
        url.set_path(path);
        {
            let mut query = url.query_pairs_mut();
            for (key, value) in pairs {
                query.append_pair(key, value);
            }
        }
        url
    }

    /// One request under the full §7.5 policy: rate gate, timeouts,
    /// classification, and up to three retries on transport failures with
    /// the 1s/2s/4s schedule (jittered by the delayer).
    async fn execute(&self, url: reqwest::Url, etag: Option<&str>) -> Result<Executed, ApiError> {
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            self.gate.enter().await;
            let mut request = self.http.get(url.clone());
            if let Some(etag) = etag {
                request = request.header(reqwest::header::IF_NONE_MATCH, etag);
            }
            let outcome = match request.send().await {
                Ok(response) => self.classify(response).await,
                Err(error) => Err(if error.is_timeout() {
                    ApiError::Timeout(error.to_string())
                } else {
                    ApiError::Network(error.to_string())
                }),
            };
            match outcome {
                Ok(executed) => return Ok(executed),
                Err(error) => {
                    if attempt < MAX_ATTEMPTS && retryable(&error) {
                        if let Some(delay) = backoff_for(attempt) {
                            self.delayer.delay(delay).await;
                        }
                        continue;
                    }
                    return Err(error);
                }
            }
        }
    }

    /// Turn a response into [`Executed`], or the `ApiError` the retry
    /// loop reasons about.
    async fn classify(&self, response: reqwest::Response) -> Result<Executed, ApiError> {
        let status = response.status().as_u16();
        match status {
            200 => {
                let etag = header_etag(&response);
                let body = read_capped(response).await?;
                Ok(Executed::Ok { body, etag })
            }
            304 => Ok(Executed::NotModified),
            429 => Err(ApiError::RateLimited),
            code if (500..=599).contains(&code) => Err(ApiError::Status(code)),
            code => {
                // Other 4xx: the body is often an error envelope whose
                // message is worth keeping (§7.3); keep the bare status
                // otherwise. Not retryable either way.
                match read_capped(response).await {
                    Ok(body) => {
                        if let Ok(envelope) = ApiEnvelope::from_json(&body)
                            && envelope.code != 200
                            && let Some(message) = envelope.data.as_str()
                        {
                            return Err(ApiError::Upstream(format!(
                                "HTTP {code} ({}): {message}",
                                envelope.status
                            )));
                        }
                        Err(ApiError::Status(code))
                    }
                    Err(_) => Err(ApiError::Status(code)),
                }
            }
        }
    }
}

/// §7.5: read the body in chunks, refusing anything over the 2 MiB cap.
async fn read_capped(mut response: reqwest::Response) -> Result<String, ApiError> {
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(ApiError::Malformed(format!(
                "response body exceeds the {MAX_BODY_BYTES} byte cap"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body)
        .map_err(|_| ApiError::Malformed("response body is not valid UTF-8".into()))
}

fn header_etag(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn unexpected_304() -> ApiError {
    ApiError::Malformed("HTTP 304 without an If-None-Match header".into())
}

#[async_trait]
impl AlAdhanClient for HttpClient {
    async fn calendar(
        &self,
        year: i32,
        month: u32,
        p: &Params,
    ) -> Result<Vec<CalendarDayDto>, ApiError> {
        p.validate().map_err(ApiError::InvalidParams)?;
        let url = self.url_for(&format!("/v1/calendar/{year}/{month}"), &p.query_pairs());
        match self.execute(url, None).await? {
            Executed::Ok { body, .. } => parse_calendar(&body),
            Executed::NotModified => Err(unexpected_304()),
        }
    }

    async fn timings(&self, date: NaiveDate, p: &Params) -> Result<TimingsDto, ApiError> {
        p.validate().map_err(ApiError::InvalidParams)?;
        // §7.1: the timings path takes DD-MM-YYYY.
        let path = format!(
            "/v1/timings/{:02}-{:02}-{}",
            date.day(),
            date.month(),
            date.year()
        );
        let url = self.url_for(&path, &p.query_pairs());
        match self.execute(url, None).await? {
            Executed::Ok { body, .. } => parse_timings(&body),
            Executed::NotModified => Err(unexpected_304()),
        }
    }

    async fn qibla(&self, lat: f64, lng: f64) -> Result<f64, ApiError> {
        validate_coordinates(lat, lng).map_err(ApiError::InvalidParams)?;
        let url = self.url_for(&format!("/v1/qibla/{lat}/{lng}"), &[]);
        match self.execute(url, None).await? {
            Executed::Ok { body, .. } => parse_qibla(&body),
            Executed::NotModified => Err(unexpected_304()),
        }
    }

    async fn revalidate_calendar(
        &self,
        year: i32,
        month: u32,
        p: &Params,
        etag: &str,
    ) -> Result<Revalidated, ApiError> {
        p.validate().map_err(ApiError::InvalidParams)?;
        let url = self.url_for(&format!("/v1/calendar/{year}/{month}"), &p.query_pairs());
        let conditional = if etag.is_empty() { None } else { Some(etag) };
        match self.execute(url, conditional).await? {
            Executed::Ok { body, etag } => {
                let days = parse_calendar(&body)?;
                Ok(Revalidated::Fresh(days, etag.unwrap_or_default()))
            }
            Executed::NotModified => Ok(Revalidated::NotModified),
        }
    }
}

/// In-memory double for the layers above: Phase 4's scheduler tests assert
/// against it with zero HTTP. Each method has a queue of scripted results
/// and a call counter; popping an empty queue is a `Malformed` error
/// naming the endpoint, so a test setup bug fails loudly instead of
/// hanging.
#[derive(Default)]
pub struct MockClient {
    calendar: std::sync::Mutex<std::collections::VecDeque<Result<Vec<CalendarDayDto>, ApiError>>>,
    timings: std::sync::Mutex<std::collections::VecDeque<Result<TimingsDto, ApiError>>>,
    qibla: std::sync::Mutex<std::collections::VecDeque<Result<f64, ApiError>>>,
    revalidations: std::sync::Mutex<std::collections::VecDeque<Result<Revalidated, ApiError>>>,
    calendar_calls: AtomicUsize,
    timings_calls: AtomicUsize,
    qibla_calls: AtomicUsize,
    revalidation_calls: AtomicUsize,
}

// Lock poisoning only happens after a panic elsewhere in the test; the
// queue's contents are still sound to read, so recover the inner value
// instead of failing.
fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl MockClient {
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the next `calendar` response.
    pub fn push_calendar(&self, result: Result<Vec<CalendarDayDto>, ApiError>) {
        lock(&self.calendar).push_back(result);
    }

    /// Script the next `timings` response.
    pub fn push_timings(&self, result: Result<TimingsDto, ApiError>) {
        lock(&self.timings).push_back(result);
    }

    /// Script the next `qibla` response.
    pub fn push_qibla(&self, result: Result<f64, ApiError>) {
        lock(&self.qibla).push_back(result);
    }

    /// Script the next `revalidate_calendar` outcome.
    pub fn push_revalidation(&self, result: Result<Revalidated, ApiError>) {
        lock(&self.revalidations).push_back(result);
    }

    pub fn calendar_calls(&self) -> usize {
        self.calendar_calls.load(Ordering::Relaxed)
    }

    pub fn timings_calls(&self) -> usize {
        self.timings_calls.load(Ordering::Relaxed)
    }

    pub fn qibla_calls(&self) -> usize {
        self.qibla_calls.load(Ordering::Relaxed)
    }

    pub fn revalidation_calls(&self) -> usize {
        self.revalidation_calls.load(Ordering::Relaxed)
    }

    fn pop<T>(queue: &std::sync::Mutex<std::collections::VecDeque<T>>) -> Option<T> {
        lock(queue).pop_front()
    }

    fn exhausted(endpoint: &str) -> ApiError {
        ApiError::Malformed(format!(
            "the mock {endpoint} queue is empty — the test scripted too few responses"
        ))
    }
}

#[async_trait]
impl AlAdhanClient for MockClient {
    async fn calendar(
        &self,
        _year: i32,
        _month: u32,
        _p: &Params,
    ) -> Result<Vec<CalendarDayDto>, ApiError> {
        self.calendar_calls.fetch_add(1, Ordering::Relaxed);
        Self::pop(&self.calendar).ok_or_else(|| Self::exhausted("calendar"))?
    }

    async fn timings(&self, _date: NaiveDate, _p: &Params) -> Result<TimingsDto, ApiError> {
        self.timings_calls.fetch_add(1, Ordering::Relaxed);
        Self::pop(&self.timings).ok_or_else(|| Self::exhausted("timings"))?
    }

    async fn qibla(&self, _lat: f64, _lng: f64) -> Result<f64, ApiError> {
        self.qibla_calls.fetch_add(1, Ordering::Relaxed);
        Self::pop(&self.qibla).ok_or_else(|| Self::exhausted("qibla"))?
    }

    async fn revalidate_calendar(
        &self,
        _year: i32,
        _month: u32,
        _p: &Params,
        _etag: &str,
    ) -> Result<Revalidated, ApiError> {
        self.revalidation_calls.fetch_add(1, Ordering::Relaxed);
        Self::pop(&self.revalidations).ok_or_else(|| Self::exhausted("revalidate_calendar"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SalatError;

    /// §18.1: every transport/parse failure lands on its design-mandated
    /// code.
    #[test]
    fn api_errors_map_to_the_user_facing_codes() {
        let cases = [
            (ApiError::Network("connection refused".into()), "E302"),
            (ApiError::Timeout("20 s elapsed".into()), "E302"),
            (ApiError::Status(503), "E301"),
            (
                ApiError::Upstream("HTTP 400 (BAD_REQUEST): …".into()),
                "E301",
            ),
            (ApiError::RateLimited, "E304"),
            (ApiError::Malformed("truncated body".into()), "E303"),
        ];
        for (error, code) in cases {
            assert_eq!(error.to_salat_error().code(), code, "{error:?}");
        }
    }

    /// §7.4 rejections map by rule family: coordinates → E101, method →
    /// E102, everything else → E201 carrying every message.
    #[test]
    fn invalid_params_map_by_kind() {
        let coordinates = ApiError::InvalidParams(vec![InvalidParam {
            kind: ParamKind::Coordinates,
            message: "latitude must be between -90 and 90 (found 999)".into(),
        }]);
        assert_eq!(
            coordinates.to_salat_error().code(),
            "E101",
            "{coordinates:?}"
        );

        let method = ApiError::InvalidParams(vec![InvalidParam {
            kind: ParamKind::Method,
            message: "method 777 is not a recognised AlAdhan method id".into(),
        }]);
        assert_eq!(method.to_salat_error().code(), "E102", "{method:?}");

        let config = ApiError::InvalidParams(vec![InvalidParam {
            kind: ParamKind::Config,
            message: "school must be 0 (Shafi) or 1 (Hanafi) (found 2)".into(),
        }]);
        match config.to_salat_error() {
            SalatError::ConfigInvalid { problems } => assert_eq!(
                problems,
                vec!["school must be 0 (Shafi) or 1 (Hanafi) (found 2)".to_owned()]
            ),
            other => panic!("expected ConfigInvalid, got {other:?}"),
        }
    }

    /// §7.5: the production constructor parses the base URL and builds the
    /// client without panicking; a bogus base is a `Malformed` error.
    #[test]
    fn constructor_validates_the_base_url() {
        assert!(HttpClient::new().is_ok());
        let error = match HttpClient::with_transport(
            "not a url",
            CONNECT_TIMEOUT,
            TOTAL_TIMEOUT,
            Arc::new(StdDelayer),
        ) {
            Ok(_) => panic!("a bogus base URL must not build a client"),
            Err(error) => error,
        };
        assert!(matches!(error, ApiError::Malformed(_)), "{error:?}");
    }
}
