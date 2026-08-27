use crate::error::AppError;

pub(crate) async fn spawn<T, F>(
    task_name: &'static str,
    task: F,
) -> Result<Result<T, AppError>, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    // NOTE(runtime): The nested result is intentional: the inner error belongs to the
    // operation, while the outer error means the blocking worker itself panicked or was cancelled.
    spawn_with(task_name, AppError::Internal, task).await
}

pub(crate) async fn spawn_with<T, F, E>(
    task_name: &'static str,
    join_error: E,
    task: F,
) -> Result<Result<T, AppError>, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
    E: FnOnce(String) -> AppError,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|error| join_error(format!("{task_name} task failed: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    // INVARIANT(runtime): the two nesting levels mean different things, and the whole point of
    // the nested Result is that a caller can tell them apart. These two tests make the NOTE above
    // executable rather than aspirational.
    #[tokio::test]
    async fn an_operation_failure_surfaces_as_the_inner_error() {
        let result = spawn("unit", || {
            Err::<(), _>(AppError::DataParsing("bad document".to_string()))
        })
        .await;

        let inner = result.expect("the worker itself ran to completion");
        let error = inner.expect_err("the operation failed");
        assert!(matches!(error, AppError::DataParsing(_)), "got {error:?}");
    }

    #[tokio::test]
    async fn a_panicking_worker_surfaces_as_the_outer_error() {
        let result = spawn("unit", || -> Result<(), AppError> {
            panic!("worker exploded");
        })
        .await;

        let error = result.expect_err("a panic must surface at the outer level");
        assert!(matches!(error, AppError::Internal(_)), "got {error:?}");
        assert!(
            format!("{error}").contains("unit"),
            "the task name should identify which worker died: {error}",
        );
    }

    #[tokio::test]
    async fn success_nests_twice() {
        let result = spawn("unit", || Ok(7_u32)).await;
        assert_eq!(result.expect("worker ran").expect("operation succeeded"), 7);
    }

    // `spawn_with` lets the caller choose the OUTER constructor, which describes a dead worker,
    // not a domain failure. This test exists to make that choice visible at review time.
    #[tokio::test]
    async fn spawn_with_uses_the_supplied_constructor_only_for_a_dead_worker() {
        let result = spawn_with("hash", AppError::Provenance, || -> Result<(), AppError> {
            panic!("hash worker exploded");
        })
        .await;
        let error = result.expect_err("panic surfaces at the outer level");
        assert!(matches!(error, AppError::Provenance(_)), "got {error:?}");

        // The inner error is untouched by the constructor choice.
        let result = spawn_with("hash", AppError::Provenance, || {
            Err::<(), _>(AppError::Storage("disk gone".to_string()))
        })
        .await;
        let inner = result.expect("worker ran").expect_err("operation failed");
        assert!(matches!(inner, AppError::Storage(_)), "got {inner:?}");
    }
}
