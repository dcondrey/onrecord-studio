use crate::{error::AppError, storage::sqlite::DuckDbRepository, types::*};
use tauri::State;

#[tauri::command]
pub async fn get_facility_coverage(
    repository: State<'_, DuckDbRepository>,
) -> Result<FacilityCoverageWorkspace, AppError> {
    repository.facility_coverage_workspace().await
}

#[tauri::command]
pub async fn save_facility_coverage_mapping(
    mapping: FacilityCoverageMapping,
    repository: State<'_, DuckDbRepository>,
) -> Result<FacilityCoverageWorkspace, AppError> {
    repository.save_facility_coverage_mapping(mapping).await
}
