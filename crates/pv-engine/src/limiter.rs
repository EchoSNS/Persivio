//! Global and per-task byte-rate limits on top of `governor` (a GCRA token bucket).

use std::num::NonZeroU32;
use std::sync::{Arc, PoisonError, RwLock};

use async_trait::async_trait;
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter as Governor};
use pv_core::RateLimiter;

/// One byte-rate bucket.
///
/// The burst is 1/10 s of traffic: a short initial burst keeps measured rates within
/// ±10 % even for downloads lasting a few seconds. A request bigger than the burst is
/// paid for in burst-sized installments, because `governor` rejects single requests
/// larger than its capacity.
#[derive(Debug)]
pub struct ByteLimiter {
    bucket: DefaultDirectRateLimiter,
    burst: NonZeroU32,
}

impl ByteLimiter {
    /// A limiter for `bytes_per_second` (clamped to `u32::MAX`, i.e. ~4 GiB/s).
    #[must_use]
    pub fn new(bytes_per_second: u64) -> Self {
        let rate = u32::try_from(bytes_per_second).unwrap_or(u32::MAX).max(1);
        let rate = NonZeroU32::new(rate).unwrap_or(NonZeroU32::MIN);
        let burst = NonZeroU32::new((rate.get() / 10).max(1)).unwrap_or(NonZeroU32::MIN);
        Self {
            bucket: Governor::direct(Quota::per_second(rate).allow_burst(burst)),
            burst,
        }
    }

    /// Waits until `bytes` may pass.
    pub async fn acquire(&self, bytes: u64) {
        let mut left = bytes;
        while left > 0 {
            let step = u32::try_from(left)
                .unwrap_or(u32::MAX)
                .min(self.burst.get());
            if let Some(n) = NonZeroU32::new(step) {
                // Cannot fail: `step` never exceeds the burst capacity.
                let _ = self.bucket.until_n_ready(n).await;
            }
            left -= u64::from(step);
        }
    }
}

/// The global limit, changeable at runtime (settings edits). The current bucket is
/// swapped atomically; in-flight waits finish against the bucket they started on.
#[derive(Debug, Default)]
pub struct GlobalLimiter {
    current: RwLock<Option<Arc<ByteLimiter>>>,
}

impl GlobalLimiter {
    /// Sets (or removes, with `None`) the global limit.
    pub fn set(&self, bytes_per_second: Option<u64>) {
        let next = bytes_per_second.map(|bps| Arc::new(ByteLimiter::new(bps)));
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = next;
    }

    fn get(&self) -> Option<Arc<ByteLimiter>> {
        // Clone the Arc and release the lock before any `.await`.
        self.current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// What a running task uses: the global bucket, then its own bucket (if any).
#[derive(Debug)]
pub struct TaskLimiter {
    global: Arc<GlobalLimiter>,
    own: Option<ByteLimiter>,
}

impl TaskLimiter {
    /// Combines the shared global limiter with an optional per-task limit.
    #[must_use]
    pub fn new(global: Arc<GlobalLimiter>, per_task_bps: Option<u64>) -> Self {
        Self {
            global,
            own: per_task_bps.map(ByteLimiter::new),
        }
    }
}

#[async_trait]
impl RateLimiter for TaskLimiter {
    async fn acquire(&self, bytes: u64) {
        if let Some(global) = self.global.get() {
            global.acquire(bytes).await;
        }
        if let Some(own) = &self.own {
            own.acquire(bytes).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[tokio::test]
    async fn rate_is_accurate_and_large_requests_do_not_error() {
        let limiter = ByteLimiter::new(200_000);
        let started = Instant::now();
        // 100 KB in 64 KiB chunks (larger than the 20 KB burst) ≈ 0.4 s after the burst.
        for _ in 0..(100_000 / 10_000) {
            limiter.acquire(10_000).await;
        }
        limiter.acquire(0).await;
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(380), "{elapsed:?}");
        assert!(elapsed < Duration::from_millis(700), "{elapsed:?}");
        let huge = ByteLimiter::new(10_000_000);
        let t = Instant::now();
        huge.acquire(3_000_000).await; // 150 x burst
        assert!(t.elapsed() < Duration::from_millis(600));
    }

    #[tokio::test]
    async fn global_limit_can_be_changed_and_removed() {
        let global = Arc::new(GlobalLimiter::default());
        let task = TaskLimiter::new(global.clone(), None);
        let t = Instant::now();
        task.acquire(5_000_000).await;
        assert!(
            t.elapsed() < Duration::from_millis(50),
            "unlimited by default"
        );
        global.set(Some(100_000));
        let t = Instant::now();
        task.acquire(30_000).await; // 10 KB burst, then 20 KB at 100 KB/s
        assert!(
            t.elapsed() >= Duration::from_millis(150),
            "{:?}",
            t.elapsed()
        );
        global.set(None);
        let t = Instant::now();
        task.acquire(5_000_000).await;
        assert!(t.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn per_task_limit_applies_on_top_of_global() {
        let task = TaskLimiter::new(Arc::default(), Some(100_000));
        let t = Instant::now();
        task.acquire(30_000).await;
        assert!(
            t.elapsed() >= Duration::from_millis(150),
            "{:?}",
            t.elapsed()
        );
    }
}
