//! Token-bucket speed limiter shared by the connections it governs.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::time::Instant;

pub struct RateLimiter {
    /// Bytes per second; 0 disables limiting.
    rate: AtomicU64,
    state: Mutex<Bucket>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl RateLimiter {
    pub fn new(rate: u64) -> Self {
        RateLimiter { rate: AtomicU64::new(rate), state: Mutex::new(Bucket { tokens: 0.0, last: Instant::now() }) }
    }

    pub fn set_rate(&self, rate: u64) {
        self.rate.store(rate, Ordering::Relaxed);
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Ordering::Relaxed)
    }

    /// Waits until `n` bytes may pass. Callers consume data first and then
    /// wait, so TCP flow control applies the back-pressure to the server.
    pub async fn acquire(&self, n: usize) {
        loop {
            let rate = self.rate();
            if rate == 0 {
                return;
            }
            let wait = {
                let mut b = self.state.lock();
                let now = Instant::now();
                let elapsed = now.duration_since(b.last).as_secs_f64();
                b.last = now;
                // Allow bursts of at most a quarter second of traffic.
                let cap = (rate as f64 * 0.25).max(64.0 * 1024.0);
                b.tokens = (b.tokens + elapsed * rate as f64).min(cap);
                if b.tokens >= 0.0 {
                    // Go into debt; later callers wait it off. This keeps
                    // large chunks from starving under small rates.
                    b.tokens -= n as f64;
                    return;
                }
                Duration::from_secs_f64((-b.tokens / rate as f64).min(1.0))
            };
            tokio::time::sleep(wait).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn limits_throughput() {
        let rl = RateLimiter::new(1_000_000);
        let start = std::time::Instant::now();
        for _ in 0..40 {
            rl.acquire(64 * 1024).await; // 2.6 MB total
        }
        let secs = start.elapsed().as_secs_f64();
        assert!(secs > 2.0 && secs < 3.5, "took {secs}");
    }

    #[tokio::test]
    async fn unlimited_is_instant() {
        let rl = RateLimiter::new(0);
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            rl.acquire(1 << 20).await;
        }
        assert!(start.elapsed().as_millis() < 50);
    }
}
