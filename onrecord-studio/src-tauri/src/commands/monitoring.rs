use crate::{error::AppError, storage::sqlite::DuckDbRepository, types::monitoring::*};
use tauri::State;

#[tauri::command]
pub async fn get_monitoring_status(
    topic: String,
    case_id: Option<String>,
    repository: State<'_, DuckDbRepository>,
) -> Result<MonitoringStatus, AppError> {
    repository.monitoring_status(topic, case_id).await
}

#[tauri::command]
pub async fn refresh_monitoring_now(
    topic: String,
    case_id: Option<String>,
    repository: State<'_, DuckDbRepository>,
) -> Result<MonitoringStatus, AppError> {
    repository.refresh_monitoring().await?;
    repository.monitoring_status(topic, case_id).await
}

#[tauri::command]
pub async fn save_monitoring_coverage(
    expectation: CoverageExpectation,
    repository: State<'_, DuckDbRepository>,
) -> Result<(), AppError> {
    repository.save_coverage_expectation(expectation).await
}

#[tauri::command]
pub async fn save_monitoring_observation(
    observation: ServiceObservation,
    repository: State<'_, DuckDbRepository>,
) -> Result<(), AppError> {
    repository.save_service_observation(observation).await
}
