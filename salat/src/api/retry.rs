//! Retry, backoff and the rate-limit gate (§7.5).
//!
//! The transport policy in one place: at most four attempts per request
//! (one try plus three retries), a fixed 1s/2s/4s schedule with ±20%
//! jitter so a fleet of clients does not retry in lockstep, a 100 ms
//! minimum gap between request starts (the 12 req/s limit), and the rule
//! for what is retryable at all.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::sync::Mutex;

use super::ApiError;

/// §7.5: one attempt plus three retries — four requests at most.
pub const MAX_ATTEMPTS: u32 = 4;

/// §7.5: the fixed backoff schedule, one entry per retry, ±20% jitter at
/// sleep time. The ERROR screen's `retried 3× (1s · 2s · 4s)` line quotes
/// these values (§18.2).
pub const BACKOFF_SCHEDULE: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

/// §7.5: 12 requests/second, expressed as the minimum gap between request
/// starts.
pub const MIN_REQUEST_GAP: Duration = Duration::from_millis(100);

/// The delay to sleep before the attempt after `attempt` (1-based): 1s
/// after the first, 2s after the second, 4s after the third, `None` once
/// the budget is spent.
pub fn backoff_for(attempt: u32) -> Option<Duration> {
    BACKOFF_SCHEDULE
        .get(attempt.saturating_sub(1) as usize)
        .copied()
}

/// Async sleep, as a trait so the failure tests can record the delays a
/// retry sequence asked for instead of waiting a real 7 seconds.
#[async_trait]
pub trait Delayer: Send + Sync {
    /// Wait for `duration` — the production impl adds jitter.
    async fn delay(&self, duration: Duration);
}

/// The production delayer: sleeps for the requested duration with ±20%
/// jitter.
pub struct StdDelayer;

#[async_trait]
impl Delayer for StdDelayer {
    async fn delay(&self, duration: Duration) {
        tokio::time::sleep(jittered(duration)).await;
    }
}

/// ±20% jitter without a `rand` dependency: FNV-1a over the elapsed
/// nanoseconds since the first call picks a multiplier in `[0.8, 1.2]`.
/// The seed is the **monotonic** clock, not the wall clock — the `Clock`
/// trait (§6.6) owns wall-time reads, and jitter only needs a value that
/// differs call to call. It is not cryptographically random.
fn jittered(duration: Duration) -> Duration {
    static BASE: OnceLock<Instant> = OnceLock::new();
    let seed = BASE.get_or_init(Instant::now).elapsed().as_nanos() as u64;
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in seed.to_le_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // 80..=120, in 1% steps.
    let percent = 80 + hash % 41;
    let millis = duration.as_millis() as u64 * percent / 100;
    Duration::from_millis(millis)
}

/// The §7.5 rate limit as a gate: a `tokio::sync::Mutex<Option<Instant>>`
/// doubles as a one-permit semaphore whose `lock()` is infallible — the
/// reason this module holds no `unwrap` despite the crate-wide ban.
/// Requests queue on the mutex, and each start waits until 100 ms have
/// passed since the previous start.
pub struct RequestGate {
    last_start: Mutex<Option<tokio::time::Instant>>,
}

impl RequestGate {
    pub fn new() -> Self {
        Self {
            last_start: Mutex::new(None),
        }
    }

    /// Wait for the gap, then stamp this request's start time.
    pub async fn enter(&self) {
        let mut last_start = self.last_start.lock().await;
        if let Some(previous) = *last_start {
            let elapsed = previous.elapsed();
            if elapsed < MIN_REQUEST_GAP {
                tokio::time::sleep(MIN_REQUEST_GAP - elapsed).await;
            }
        }
        *last_start = Some(tokio::time::Instant::now());
    }
}

impl Default for RequestGate {
    fn default() -> Self {
        Self::new()
    }
}

/// §7.5: only connect errors, timeouts, HTTP 429 and HTTP 5xx are retried.
/// Any other 4xx would fail identically next time, so it fails fast — and
/// a malformed body is never retried because re-reading it cannot fix it.
pub fn retryable(error: &ApiError) -> bool {
    match error {
        ApiError::Network(_) | ApiError::Timeout(_) | ApiError::RateLimited => true,
        ApiError::Status(code) => (500..=599).contains(code),
        ApiError::Malformed(_) | ApiError::Upstream(_) | ApiError::InvalidParams(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §7.5 / §18.2: the schedule is exactly 1s, 2s, 4s — the values the
    /// ERROR screen's detail line quotes.
    #[test]
    fn backoff_schedule_is_one_two_four_seconds() {
        assert_eq!(BACKOFF_SCHEDULE.len(), 3);
        assert_eq!(BACKOFF_SCHEDULE[0], Duration::from_secs(1));
        assert_eq!(BACKOFF_SCHEDULE[1], Duration::from_secs(2));
        assert_eq!(BACKOFF_SCHEDULE[2], Duration::from_secs(4));
        assert_eq!(backoff_for(1), Some(Duration::from_secs(1)));
        assert_eq!(backoff_for(2), Some(Duration::from_secs(2)));
        assert_eq!(backoff_for(3), Some(Duration::from_secs(4)));
        // Four attempts total: nothing sleeps after the last one.
        assert_eq!(backoff_for(4), None);
        assert_eq!(backoff_for(5), None);
    }

    /// Jitter stays inside ±20% for every sample, at both ends of the
    /// schedule.
    #[test]
    fn jitter_stays_within_twenty_percent() {
        let one_second = Duration::from_secs(1);
        let four_seconds = Duration::from_secs(4);
        for _ in 0..1_000 {
            let delayed = jittered(one_second);
            assert!(delayed >= one_second * 4 / 5, "{delayed:?} is too short");
            assert!(delayed <= one_second * 6 / 5, "{delayed:?} is too long");
            let delayed = jittered(four_seconds);
            assert!(delayed >= four_seconds * 4 / 5, "{delayed:?} is too short");
            assert!(delayed <= four_seconds * 6 / 5, "{delayed:?} is too long");
        }
    }

    /// §7.5: the 100 ms gap holds across consecutive requests (under
    /// paused time, so the test costs nothing real).
    #[tokio::test(start_paused = true)]
    async fn request_gate_enforces_the_minimum_gap() {
        let gate = RequestGate::new();
        let start = tokio::time::Instant::now();

        // The first request starts immediately.
        gate.enter().await;
        assert!(
            start.elapsed() < Duration::from_millis(10),
            "first enter must not wait, waited {:?}",
            start.elapsed()
        );

        // The second waits out the remainder of the 100 ms gap.
        gate.enter().await;
        assert!(
            start.elapsed() >= MIN_REQUEST_GAP,
            "second enter waited only {:?}",
            start.elapsed()
        );

        // And the third, 200 ms after start.
        gate.enter().await;
        assert!(
            start.elapsed() >= MIN_REQUEST_GAP * 2,
            "third enter waited only {:?}",
            start.elapsed()
        );
    }

    /// §7.5: the retry matrix — connect errors, timeouts, 429 and 5xx
    /// retry; other 4xx, malformed bodies and pre-flight validation
    /// rejections never do.
    #[test]
    fn only_transport_failures_are_retryable() {
        for retryable_error in [
            ApiError::Network("connection refused".into()),
            ApiError::Timeout("request timed out".into()),
            ApiError::RateLimited,
            ApiError::Status(500),
            ApiError::Status(503),
        ] {
            assert!(retryable(&retryable_error), "{retryable_error:?}");
        }
        for fatal_error in [
            ApiError::Status(400),
            ApiError::Status(404),
            ApiError::Malformed("truncated body".into()),
            ApiError::Upstream("HTTP 400: bad request".into()),
            ApiError::InvalidParams(vec![]),
        ] {
            assert!(!retryable(&fatal_error), "{fatal_error:?}");
        }
    }
}
