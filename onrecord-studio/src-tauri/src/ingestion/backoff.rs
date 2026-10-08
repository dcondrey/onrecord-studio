//! The capped-exponential-doubling core shared by every retry policy in this module.
//!
//! Callers layer their own policy on top: [`crate::ingestion::client::ApiClient`]'s HTTP retries
//! honor a server's `Retry-After` header as an override, and the durable task queue adds
//! deterministic jitter keyed on a persisted attempt count to spread retried frontier rows across
//! restarts. Neither of those is duplicated here; this is only the doubling math both derive from.

use std::time::Duration;

/// `min(base * 2^attempt, cap)`, saturating rather than panicking as `attempt` grows.
///
/// Exact for every `attempt` when `base <= cap`, which both call sites enforce. The exponent is
/// clamped before shifting so an unbounded `attempt` (a misbehaving caller, not a real retry
/// count) saturates at `cap` instead of overflowing.
pub fn exponential_delay(attempt: u64, base: Duration, cap: Duration) -> Duration {
    let exponent = attempt.min(32) as u32;
    let factor = 1_u32.checked_shl(exponent).unwrap_or(u32::MAX);
    base.saturating_mul(factor).min(cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_from_the_base_and_saturates_at_the_cap() {
        let base = Duration::from_millis(500);
        let cap = Duration::from_secs(15);
        for (attempt, expected_ms) in [
            (0, 500),
            (1, 1_000),
            (2, 2_000),
            (5, 15_000),
            (u64::MAX, 15_000),
        ] {
            assert_eq!(
                exponential_delay(attempt, base, cap),
                Duration::from_millis(expected_ms),
                "attempt {attempt}"
            );
        }
    }
}
