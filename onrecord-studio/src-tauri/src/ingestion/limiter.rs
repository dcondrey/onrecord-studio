//! Independent, dynamically configurable token buckets for portal traffic.

use crate::error::AppError;
use crate::types::RateLimitPolicy;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio::time::Instant;

const MINIMUM_REFILL_TOKENS_PER_SECOND: f64 = 0.001;
const MAXIMUM_REFILL_TOKENS_PER_SECOND: f64 = 100.0;
const MAXIMUM_ACQUIRE_SLEEP: Duration = Duration::from_secs(60);

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    capacity: f64,
    refill_tokens_per_second: f64,
    last_refill: Instant,
}

impl Bucket {
    fn new(policy: RateLimitPolicy) -> Self {
        Self {
            tokens: policy.burst_capacity as f64,
            capacity: policy.burst_capacity as f64,
            refill_tokens_per_second: policy.refill_tokens_per_second,
            last_refill: Instant::now(),
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_tokens_per_second).min(self.capacity);
        self.last_refill = now;
    }

    fn reconfigure(&mut self, policy: RateLimitPolicy, now: Instant) {
        // Refill under the old policy first, then clamp. Raising the burst capacity must not
        // mint an immediate batch of tokens that the portal never earned.
        self.refill(now);
        self.capacity = policy.burst_capacity as f64;
        self.tokens = self.tokens.min(self.capacity);
        self.refill_tokens_per_second = policy.refill_tokens_per_second;
    }
}

#[derive(Clone)]
pub struct RateLimiter {
    buckets: Arc<RwLock<HashMap<String, Arc<Mutex<Bucket>>>>>,
    maximum_portals: usize,
}

impl RateLimiter {
    pub fn new(maximum_portals: usize) -> Result<Self, AppError> {
        if maximum_portals == 0 {
            return Err(AppError::RateLimiter(
                "Maximum portal count must be greater than zero".to_string(),
            ));
        }
        Ok(Self {
            buckets: Arc::new(RwLock::new(HashMap::new())),
            maximum_portals,
        })
    }

    pub async fn register_portal(
        &self,
        portal_key: &str,
        policy: RateLimitPolicy,
    ) -> Result<(), AppError> {
        validate_portal_key(portal_key)?;
        validate_policy(policy)?;
        let mut buckets = self.buckets.write().await;
        if let Some(bucket) = buckets.get(portal_key).cloned() {
            drop(buckets);
            bucket.lock().await.reconfigure(policy, Instant::now());
            return Ok(());
        }
        if buckets.len() >= self.maximum_portals {
            return Err(AppError::PortalCapacity(format!(
                "At most {} independently throttled portals may be active",
                self.maximum_portals
            )));
        }
        buckets.insert(
            portal_key.to_string(),
            Arc::new(Mutex::new(Bucket::new(policy))),
        );
        Ok(())
    }

    /// Consumes one token, waiting until one has been earned.
    ///
    /// INVARIANT(ingestion): Each iteration's sleep is clamped to `MAXIMUM_ACQUIRE_SLEEP`, but the
    /// TOTAL wait is unbounded and there is no fairness between waiters. At the minimum legal
    /// refill rate a single token takes ~1000s across many iterations, and a waiter can be
    /// repeatedly beaten to the token by a newcomer. Callers needing a deadline must impose it.
    ///
    /// HTTP-level adaptive pacing is host-specific and lives in `HostGovernor`: it records 429
    /// strikes, honors bounded Retry-After values, and advances that host's next-allowed instant.
    /// This portal-level bucket remains the operator-configured outer ceiling.
    pub async fn acquire(&self, portal_key: &str) -> Result<(), AppError> {
        let bucket = self.bucket(portal_key).await?;
        loop {
            let wait = {
                let mut bucket = bucket.lock().await;
                bucket.refill(Instant::now());
                if bucket.tokens >= 1.0 {
                    bucket.tokens -= 1.0;
                    return Ok(());
                }
                let wait_seconds = ((1.0 - bucket.tokens) / bucket.refill_tokens_per_second)
                    .clamp(0.001, MAXIMUM_ACQUIRE_SLEEP.as_secs_f64());
                Duration::from_secs_f64(wait_seconds)
            };
            // The bucket mutex is released above, before this sleep, so a waiting caller never
            // blocks another portal or a concurrent reader of the same bucket.
            tokio::time::sleep(wait).await;
        }
    }

    pub async fn available_tokens(&self, portal_key: &str) -> Result<f64, AppError> {
        let bucket = self.bucket(portal_key).await?;
        let mut bucket = bucket.lock().await;
        bucket.refill(Instant::now());
        Ok(bucket.tokens)
    }

    pub async fn unregister_portal(&self, portal_key: &str) -> bool {
        self.buckets.write().await.remove(portal_key).is_some()
    }

    pub async fn registered_portal_count(&self) -> usize {
        self.buckets.read().await.len()
    }

    async fn bucket(&self, portal_key: &str) -> Result<Arc<Mutex<Bucket>>, AppError> {
        self.buckets
            .read()
            .await
            .get(portal_key)
            .cloned()
            .ok_or_else(|| {
                AppError::RateLimiter(format!(
                    "Portal '{portal_key}' has no registered token bucket"
                ))
            })
    }
}

fn validate_portal_key(portal_key: &str) -> Result<(), AppError> {
    if portal_key.is_empty()
        || portal_key.len() > 128
        || !portal_key
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || value == b'-' || value == b'_')
    {
        return Err(AppError::RateLimiter(format!(
            "Portal key '{portal_key}' is invalid"
        )));
    }
    Ok(())
}

fn validate_policy(policy: RateLimitPolicy) -> Result<(), AppError> {
    if policy.burst_capacity == 0 || policy.burst_capacity > 100 {
        return Err(AppError::RateLimiter(
            "Burst capacity must be between 1 and 100".to_string(),
        ));
    }
    if !policy.refill_tokens_per_second.is_finite()
        || policy.refill_tokens_per_second < MINIMUM_REFILL_TOKENS_PER_SECOND
        || policy.refill_tokens_per_second > MAXIMUM_REFILL_TOKENS_PER_SECOND
    {
        return Err(AppError::RateLimiter(format!(
            "Refill rate must be finite and between {MINIMUM_REFILL_TOKENS_PER_SECOND} and {MAXIMUM_REFILL_TOKENS_PER_SECOND} tokens per second"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn enforces_portal_capacity_without_cross_bucket_state() {
        let limiter = RateLimiter::new(2).unwrap();
        limiter
            .register_portal("one", RateLimitPolicy::default())
            .await
            .unwrap();
        limiter
            .register_portal("two", RateLimitPolicy::default())
            .await
            .unwrap();
        assert!(limiter
            .register_portal("three", RateLimitPolicy::default())
            .await
            .is_err());
        assert_eq!(limiter.registered_portal_count().await, 2);
    }

    #[tokio::test]
    async fn reconfigures_an_existing_bucket_without_consuming_capacity() {
        let limiter = RateLimiter::new(1).unwrap();
        limiter
            .register_portal(
                "portal",
                RateLimitPolicy {
                    burst_capacity: 5,
                    refill_tokens_per_second: 1.0,
                },
            )
            .await
            .unwrap();
        limiter
            .register_portal(
                "portal",
                RateLimitPolicy {
                    burst_capacity: 2,
                    refill_tokens_per_second: 2.0,
                },
            )
            .await
            .unwrap();

        assert_eq!(limiter.registered_portal_count().await, 1);
        assert!(limiter.available_tokens("portal").await.unwrap() <= 2.0);
    }

    #[test]
    fn rejects_refill_rates_that_could_overflow_sleep_durations() {
        assert!(validate_policy(RateLimitPolicy {
            burst_capacity: 1,
            refill_tokens_per_second: f64::MIN_POSITIVE,
        })
        .is_err());
        assert!(validate_policy(RateLimitPolicy {
            burst_capacity: 1,
            refill_tokens_per_second: f64::NAN,
        })
        .is_err());
    }
}

#[cfg(test)]
mod bucket_invariant_tests {
    use super::*;

    fn policy(burst_capacity: u32, refill_tokens_per_second: f64) -> RateLimitPolicy {
        RateLimitPolicy {
            burst_capacity,
            refill_tokens_per_second,
        }
    }

    // INVARIANT(ingestion): acquire consumes exactly one token per call, and the bucket is the
    // only thing standing between this app and a portal it is asked to be polite to.
    #[tokio::test]
    async fn acquire_consumes_one_token_per_call() {
        let limiter = RateLimiter::new(4).expect("limiter");
        limiter
            .register_portal("portal", policy(3, 0.001))
            .await
            .expect("register");

        // A very slow refill means anything we observe is consumption, not replenishment.
        let before = limiter.available_tokens("portal").await.expect("tokens");
        limiter.acquire("portal").await.expect("first token");
        let after_one = limiter.available_tokens("portal").await.expect("tokens");
        limiter.acquire("portal").await.expect("second token");
        let after_two = limiter.available_tokens("portal").await.expect("tokens");

        assert!(
            (before - after_one - 1.0).abs() < 0.01,
            "{before} -> {after_one}"
        );
        assert!(
            (after_one - after_two - 1.0).abs() < 0.01,
            "{after_one} -> {after_two}"
        );
    }

    // SECURITY(ingestion): raising burst capacity must not mint tokens the portal never earned.
    // This is the invariant the refill-then-clamp ordering in `reconfigure` exists to protect, and
    // it is the direction the pre-existing test does not exercise.
    #[tokio::test]
    async fn raising_burst_capacity_does_not_mint_unearned_tokens() {
        let limiter = RateLimiter::new(4).expect("limiter");
        limiter
            .register_portal("portal", policy(2, 0.001))
            .await
            .expect("register");
        limiter.acquire("portal").await.expect("spend one");
        limiter.acquire("portal").await.expect("spend two");

        let spent = limiter.available_tokens("portal").await.expect("tokens");
        assert!(spent < 1.0, "expected a drained bucket, got {spent}");

        // Re-registering with a larger burst reconfigures in place.
        limiter
            .register_portal("portal", policy(100, 0.001))
            .await
            .expect("reconfigure");

        let after = limiter.available_tokens("portal").await.expect("tokens");
        assert!(
            after < 1.0,
            "raising burst capacity minted {after} tokens that were never earned",
        );
        assert_eq!(limiter.registered_portal_count().await, 1);
    }

    // LIMIT(ingestion): unregister followed by register builds a FRESH bucket at full burst, so
    // the consumed-token history is erased. Pinned as current behavior because it is the one way
    // a caller can reset a portal's budget without waiting for refill.
    #[tokio::test]
    async fn re_registering_after_unregister_restores_a_full_budget() {
        let limiter = RateLimiter::new(4).expect("limiter");
        limiter
            .register_portal("portal", policy(2, 0.001))
            .await
            .expect("register");
        limiter.acquire("portal").await.expect("spend one");
        limiter.acquire("portal").await.expect("spend two");
        assert!(limiter.available_tokens("portal").await.expect("tokens") < 1.0);

        assert!(limiter.unregister_portal("portal").await);
        assert!(
            !limiter.unregister_portal("portal").await,
            "second removal is a no-op"
        );
        assert_eq!(limiter.registered_portal_count().await, 0);

        limiter
            .register_portal("portal", policy(2, 0.001))
            .await
            .expect("re-register");
        let restored = limiter.available_tokens("portal").await.expect("tokens");
        assert!(
            restored >= 1.9,
            "expected a fresh full bucket, got {restored}"
        );
    }

    // Buckets must not share a token pool: draining one portal cannot throttle another.
    #[tokio::test]
    async fn portals_do_not_share_a_token_pool() {
        let limiter = RateLimiter::new(4).expect("limiter");
        limiter
            .register_portal("busy", policy(2, 0.001))
            .await
            .expect("register busy");
        limiter
            .register_portal("idle", policy(2, 0.001))
            .await
            .expect("register idle");

        limiter.acquire("busy").await.expect("spend one");
        limiter.acquire("busy").await.expect("spend two");

        assert!(limiter.available_tokens("busy").await.expect("tokens") < 1.0);
        assert!(limiter.available_tokens("idle").await.expect("tokens") >= 1.9);
    }

    // An unregistered portal must fail rather than be created implicitly, otherwise a typo in a
    // portal key would silently create an unlimited second budget.
    #[tokio::test]
    async fn unknown_portals_are_refused_rather_than_created() {
        let limiter = RateLimiter::new(4).expect("limiter");
        assert!(limiter.acquire("never-registered").await.is_err());
        assert!(limiter.available_tokens("never-registered").await.is_err());
        assert_eq!(limiter.registered_portal_count().await, 0);
    }

    #[test]
    fn a_limiter_must_admit_at_least_one_portal() {
        assert!(RateLimiter::new(0).is_err());
        assert!(RateLimiter::new(1).is_ok());
    }

    // `validate_policy` is what keeps `Duration::from_secs_f64` in the acquire loop from
    // panicking, so its rejections are a panic-safety boundary, not input hygiene.
    #[test]
    fn policy_validation_rejects_values_that_would_break_the_acquire_loop() {
        assert!(validate_policy(policy(0, 1.0)).is_err(), "zero burst");
        assert!(validate_policy(policy(1, 0.0)).is_err(), "zero refill");
        assert!(validate_policy(policy(1, -1.0)).is_err(), "negative refill");
        assert!(validate_policy(policy(1, f64::NAN)).is_err(), "NaN refill");
        assert!(
            validate_policy(policy(1, f64::INFINITY)).is_err(),
            "infinite refill"
        );
        assert!(validate_policy(policy(1, 1.0)).is_ok());
    }

    // LIMIT(ingestion): key validation runs on register only. Pinned so the asymmetry is a known
    // property rather than a surprise, since lookup failure is what makes it currently harmless.
    #[tokio::test]
    async fn portal_key_validation_runs_on_register_only() {
        let limiter = RateLimiter::new(4).expect("limiter");
        assert!(limiter.register_portal("", policy(1, 1.0)).await.is_err());
        assert!(limiter
            .register_portal(&"k".repeat(4096), policy(1, 1.0))
            .await
            .is_err());
        // The same invalid key on a read path errors for "not registered", not for being invalid.
        assert!(limiter.available_tokens("").await.is_err());
    }
}
