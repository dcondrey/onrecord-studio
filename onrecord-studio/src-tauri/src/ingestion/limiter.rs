//! Thread-safe Token Bucket Rate Limiter for per-subdomain connection throttling.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

struct Bucket {
    tokens: f64,
    max_tokens: f64,
    refill_rate_per_sec: f64,
    last_refill: Instant,
}

impl Bucket {
    fn new(max_tokens: f64, refill_rate_per_sec: f64) -> Self {
        Self {
            tokens: max_tokens,
            max_tokens,
            refill_rate_per_sec,
            last_refill: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate_per_sec).min(self.max_tokens);
        self.last_refill = now;
    }

    fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Orchestrates traffic thresholds dynamically across separate NextRequest endpoints.
#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<Mutex<HashMap<String, Bucket>>>,
    max_tokens: f64,
    refill_rate_per_sec: f64,
}

impl RateLimiter {
    /// Creates a rate limiter. Default values permit burst speeds of 3 requests, refilling 1 token/sec.
    pub fn new(max_tokens: f64, refill_rate_per_sec: f64) -> Self {
        Self {
            buckets: Arc::new(Mutex::new(HashMap::new())),
            max_tokens,
            refill_rate_per_sec,
        }
    }

    /// Blocks execution thread asynchronously until a connection slot becomes available for the subdomain.
    pub async fn throttle(&self, subdomain: &str) {
        loop {
            let mut buckets = self.buckets.lock().await;
            let bucket = buckets
                .entry(subdomain.to_string())
                .or_insert_with(|| Bucket::new(self.max_tokens, self.refill_rate_per_sec));

            if bucket.try_consume() {
                return;
            }

            // Drop lock explicitly prior to sleeping to allow concurrent check threads to evaluate fields
            drop(buckets);
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}
