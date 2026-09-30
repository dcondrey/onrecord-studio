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
    fn first_attempt_is_the_base_delay() {
        let base = Duration::from_millis(500);
        let cap = Duration::from_secs(15);
        assert_eq!(exponential_delay(0, base, cap), base);
    }

    #[test]
    fn doubles_on_the_next_attempt() {
        let base = Duration::from_millis(500);
        let cap = Duration::from_secs(15);
        assert_eq!(
            exponential_delay(1, base, cap),
            Duration::from_millis(1_000)
        );
        assert_eq!(
            exponential_delay(2, base, cap),
            Duration::from_millis(2_000)
        );
    }

    #[test]
    fn saturates_at_the_cap_and_stays_there() {
        let base = Duration::from_secs(1);
        let cap = Duration::from_secs(30);
        // 2^5 = 32s, already past the 30s cap.
        assert_eq!(exponential_delay(5, base, cap), cap);
        // A very large attempt count must not overflow or exceed the cap.
        assert_eq!(exponential_delay(1_000, base, cap), cap);
    }
}
