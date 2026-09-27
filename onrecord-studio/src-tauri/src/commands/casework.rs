use crate::{error::AppError, storage::sqlite::DuckDbRepository, types::casework::*};
use tauri::State;
#[tauri::command]
pub async fn get_casework_state(
    repository: State<'_, DuckDbRepository>,
) -> Result<CaseworkState, AppError> {
    repository.casework_state().await
}
#[tauri::command]
pub async fn create_casework_case(
    report_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<CaseworkCase, AppError> {
    repository.create_casework_case(report_id).await
}
#[tauri::command]
pub async fn save_casework_case(
    case: CaseworkCase,
    expected_revision: u64,
    author: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<CaseworkCase, AppError> {
    repository
        .save_casework_case(case, expected_revision, author)
        .await
}
#[tauri::command]
pub async fn export_casework_packet(
    case_id: String,
    contributor: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<String, AppError> {
    repository
        .export_casework_packet(case_id, contributor)
        .await
}
#[tauri::command]
pub async fn preview_casework_import(
    payload: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<CaseworkImportPreview, AppError> {
    repository.preview_casework_import(payload).await
}
#[tauri::command]
pub async fn import_casework_packet(
    payload: String,
    expected_revision: Option<u64>,
    packet_sha256: String,
    author: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<CaseworkCase, AppError> {
    repository
        .import_casework_packet(payload, expected_revision, packet_sha256, author)
        .await
}
