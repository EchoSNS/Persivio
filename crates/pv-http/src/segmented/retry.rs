//! Retry timing: exponential backoff with jitter, honoring `Retry-After`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{HeaderMap, RETRY_AFTER};

/// How a worker retries a failing segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Consecutive failures without progress before giving up.
    pub max_retries: u32,
    /// First delay.
    pub base: Duration,
    /// Longest delay (also caps server-requested `Retry-After`).
    pub cap: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base: Duration::from_millis(500),
            cap: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// Delay before retry number `attempt` (0-based): `min(cap, base·2^attempt)` scaled
    /// by a random factor in `[0.5, 1.5)`, so many workers that failed together do not
    /// all hammer the server again at the same instant. A server's `Retry-After` wins.
    #[must_use]
    pub fn delay(&self, attempt: u32, retry_after: Option<Duration>, salt: u64) -> Duration {
        if let Some(server) = retry_after {
            return server.min(self.cap);
        }
        let exp = self
            .base
            .saturating_mul(1u32 << attempt.min(16))
            .min(self.cap);
        // jitter in [0.5, 1.5): 512..1536 / 1024
        let factor = 512 + (jitter_seed(salt) % 1024);
        let nanos = exp.as_nanos() * u128::from(factor) / 1024;
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }
}

/// Cheap per-call randomness (no `rand` dependency): clock nanoseconds mixed with a
/// caller salt through the `SplitMix64` finalizer. Not cryptographic; only spreads retries.
fn jitter_seed(salt: u64) -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let mut z = u64::from(now) ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Parses `Retry-After` in its delta-seconds form (the HTTP-date form is ignored).
#[must_use]
pub fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;

    #[test]
    fn backoff_grows_with_jitter_and_caps() {
        let p = RetryPolicy {
            max_retries: 5,
            base: Duration::from_millis(100),
            cap: Duration::from_secs(1),
        };
        for attempt in 0..6 {
            let nominal = Duration::from_millis(100 * (1 << attempt)).min(p.cap);
            for salt in 0..50 {
                let d = p.delay(attempt, None, salt);
                assert!(
                    d >= nominal / 2 && d < nominal * 3 / 2,
                    "attempt {attempt}: {d:?}"
                );
            }
        }
        assert!(
            p.delay(40, None, 1) < Duration::from_millis(1500),
            "huge attempts don't overflow"
        );
    }

    #[test]
    fn retry_after_wins_but_is_capped() {
        let p = RetryPolicy::default();
        assert_eq!(p.delay(3, Some(Duration::ZERO), 0), Duration::ZERO);
        assert_eq!(p.delay(0, Some(Duration::from_secs(3600)), 0), p.cap);
        let mut h = HeaderMap::new();
        assert_eq!(retry_after(&h), None);
        h.insert(RETRY_AFTER, HeaderValue::from_static(" 7 "));
        assert_eq!(retry_after(&h), Some(Duration::from_secs(7)));
        h.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        assert_eq!(retry_after(&h), None);
    }
}
