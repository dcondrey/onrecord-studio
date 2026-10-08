use super::super::worker::{
    durable_retry_delay_for_error, durable_retry_delay_seconds, join_workers,
};
use super::*;

#[test]
fn durable_retry_backoff_starts_at_one_second_and_saturates_at_thirty() {
    let live: Vec<u64> = (0..MAXIMUM_DURABLE_TASK_ATTEMPTS)
        .map(durable_retry_delay_seconds)
        .collect();
    assert_eq!(live, vec![1, 1, 2, 4]);
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
