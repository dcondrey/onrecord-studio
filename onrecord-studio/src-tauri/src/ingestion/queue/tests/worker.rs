use super::super::worker::{
    durable_retry_delay_for_error, durable_retry_delay_seconds, join_workers,
};
use super::*;

#[test]
fn durable_retry_backoff_doubles_then_saturates() {
    // The live domain. Every call site is gated by
    // `frontier_attempt < MAXIMUM_DURABLE_TASK_ATTEMPTS`, so only attempts 0..4 reach
    // production and the observable schedule is 1s, 1s, 2s, 4s.
    assert_eq!(MAXIMUM_DURABLE_TASK_ATTEMPTS, 4);
    let live: Vec<u64> = (0..MAXIMUM_DURABLE_TASK_ATTEMPTS)
        .map(durable_retry_delay_seconds)
        .collect();
    assert_eq!(live, vec![1, 1, 2, 4]);

    // Past that gate the pure function keeps doubling and saturates at 30s. The cap is NOT
    // reachable within 0..=5: `attempt.saturating_sub(1).min(5)` first yields a shift of 5 at
    // attempt 6, and 1 << 5 == 32 is the first value `.min(30)` actually clamps. Raising
    // MAXIMUM_DURABLE_TASK_ATTEMPTS is what would put the tail below into service.
    let full: Vec<u64> = (0..=5).map(durable_retry_delay_seconds).collect();
    assert_eq!(full, vec![1, 1, 2, 4, 8, 16]);
    assert_eq!(durable_retry_delay_seconds(6), 30);
    assert_eq!(durable_retry_delay_seconds(u64::MAX), 30);
}

#[test]
fn durable_retry_policy_never_replays_permanent_parser_or_security_failures() {
    for error in [
        AppError::DataParsing("malformed document".to_string()),
        AppError::PortalSecurity("unsafe path".to_string()),
        AppError::Storage("database constraint".to_string()),
        AppError::Internal("blocking worker panicked".to_string()),
    ] {
        assert_eq!(durable_retry_delay_for_error(&error, 1, 7), None);
    }

    for error in [
        AppError::Network("connection reset".to_string()),
        AppError::RateLimiter("temporary throttle".to_string()),
        AppError::PortalCapacity("queue is full".to_string()),
    ] {
        assert!(matches!(
            durable_retry_delay_for_error(&error, 1, 7),
            Some(1 | 2)
        ));
        assert_eq!(
            durable_retry_delay_for_error(&error, MAXIMUM_DURABLE_TASK_ATTEMPTS, 7,),
            None
        );
    }
}

#[test]
fn durable_retry_jitter_spreads_rows_without_lowering_the_backoff_floor() {
    let error = AppError::Network("connection reset".to_string());
    let delays = (1..=32)
        .filter_map(|seed| durable_retry_delay_for_error(&error, 3, seed))
        .collect::<HashSet<_>>();
    assert_eq!(durable_retry_delay_seconds(3), 4);
    assert_eq!(delays, HashSet::from([4, 5, 6]));
}

#[tokio::test]
async fn stopping_workers_does_not_detach_in_flight_blocking_work() {
    let completed = Arc::new(AtomicBool::new(false));
    let worker_completed = Arc::clone(&completed);
    let worker = tokio::spawn(async move {
        tokio::task::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(10));
        })
        .await
        .expect("fixture blocking worker must finish");
        worker_completed.store(true, Ordering::Release);
    });

    join_workers(vec![worker])
        .await
        .expect("worker shutdown must join in-flight work");
    assert!(completed.load(Ordering::Acquire));
}
