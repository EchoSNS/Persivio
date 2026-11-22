//! Global and per-task byte-rate limits on top of `governor` (a GCRA token bucket).

use std::num::NonZeroU32;
use std::sync::{Arc, PoisonError, RwLock};

use async_trait::async_trait;
use governor::{DefaultDirectRateLimiter, Quota, RateLimiter as Governor};
use pv_core::RateLimiter;

/// One byte-rate bucket.
///
/// Design (each point fixes a measured inaccuracy):
/// - The bucket holds 1/4 s of traffic, so a waiter that wakes up late (Windows timers
///   tick every ~15.6 ms; macOS coalesces timers) finds the capacity it missed still
///   there, instead of it overflowing a full bucket and being lost.
/// - Requests are paid in installments of 1/8 of the bucket. A request as big as the
///   whole bucket could only pass when the bucket is *completely full*, and every late
///   wake-up would waste capacity (that cost ~20 % of the rate).
/// - The bucket starts **empty** (drained on creation), so the headroom never turns into
///   an initial burst above the limit.
#[derive(Debug)]
pub struct ByteLimiter {
    bucket: DefaultDirectRateLimiter,
    installment: NonZeroU32,
}

impl ByteLimiter {
    /// A limiter for `bytes_per_second` (clamped to `u32::MAX`, i.e. ~4 GiB/s).
    #[must_use]
    pub fn new(bytes_per_second: u64) -> Self {
        let rate = u32::try_from(bytes_per_second).unwrap_or(u32::MAX).max(1);
        let rate = NonZeroU32::new(rate).unwrap_or(NonZeroU32::MIN);
        let burst = NonZeroU32::new((rate.get() / 4).max(1)).unwrap_or(NonZeroU32::MIN);
        let installment = NonZeroU32::new((burst.get() / 8).max(1)).unwrap_or(NonZeroU32::MIN);
        let bucket = Governor::direct(Quota::per_second(rate).allow_burst(burst));
        // Start empty: consume the initial full bucket.
        let _ = bucket.check_n(burst);
        Self {
            bucket,
            installment,
        }
    }

    /// Waits until `bytes` may pass.
    pub async fn acquire(&self, bytes: u64) {
        let mut left = bytes;
        while left > 0 {
            let step = u32::try_from(left)
                .unwrap_or(u32::MAX)
                .min(self.installment.get());
            if let Some(n) = NonZeroU32::new(step) {
                // Cannot fail: an installment never exceeds the bucket capacity.
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
        // 100 KB at 200 KB/s = 0.5 s (the bucket starts empty: no initial burst).
        for _ in 0..(100_000 / 10_000) {
            limiter.acquire(10_000).await;
        }
        limiter.acquire(0).await;
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(470), "{elapsed:?}");
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
        task.acquire(30_000).await; // 30 KB at 100 KB/s; the bucket starts empty
        assert!(
            t.elapsed() >= Duration::from_millis(280),
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
            t.elapsed() >= Duration::from_millis(280),
            "{:?}",
            t.elapsed()
        );
    }
}
