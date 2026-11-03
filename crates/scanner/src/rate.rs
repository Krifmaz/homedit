//! A token-bucket rate limiter for new connections.

use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

/// Limits how many connections the scanner starts per second.
///
/// A classic token bucket: tokens refill at `rate` per second up to `burst`,
/// and each [`acquire`](RateLimiter::acquire) takes one, waiting if the
/// bucket is empty. The bucket starts full, so a scan may open `burst`
/// connections at once before settling to the steady rate.
///
/// Uses [`tokio::time`], so tests can drive it with a paused clock.
#[derive(Debug)]
pub struct RateLimiter {
    rate: f64,
    burst: f64,
    state: Mutex<Bucket>,
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    /// A limiter allowing `per_second` acquisitions per second, with a burst
    /// of one tenth of a second's worth (at least 1). `0` means unlimited.
    pub fn new(per_second: u32) -> Self {
        let rate = f64::from(per_second);
        let burst = (rate / 10.0).max(1.0);
        Self {
            rate,
            burst,
            state: Mutex::new(Bucket {
                tokens: burst,
                last: Instant::now(),
            }),
        }
    }

    /// Whether this limiter lets everything through.
    pub fn is_unlimited(&self) -> bool {
        self.rate <= 0.0
    }

    /// Waits until a token is available and takes it.
    pub async fn acquire(&self) {
        if self.is_unlimited() {
            return;
        }
        loop {
            let wait = {
                // The lock is never held across an await, and a poisoned
                // bucket is still a valid bucket.
                let mut b = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let now = Instant::now();
                let elapsed = now.duration_since(b.last).as_secs_f64();
                b.tokens = (b.tokens + elapsed * self.rate).min(self.burst);
                b.last = now;
                if b.tokens >= 1.0 {
                    b.tokens -= 1.0;
                    return;
                }
                Duration::from_secs_f64((1.0 - b.tokens) / self.rate)
            };
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn holds_the_rate() {
        let limiter = RateLimiter::new(100);
        let start = Instant::now();
        for _ in 0..110 {
            limiter.acquire().await;
        }
        // 10 come from the initial burst, the other 100 take about a second.
        let elapsed = start.elapsed();
        assert!(elapsed >= Duration::from_millis(990), "{elapsed:?}");
        assert!(elapsed <= Duration::from_millis(1100), "{elapsed:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn zero_is_unlimited() {
        let limiter = RateLimiter::new(0);
        let start = Instant::now();
        for _ in 0..10_000 {
            limiter.acquire().await;
        }
        assert_eq!(start.elapsed(), Duration::ZERO);
    }
}
