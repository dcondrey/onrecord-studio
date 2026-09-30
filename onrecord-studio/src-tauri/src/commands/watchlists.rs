use crate::{error::AppError, storage::sqlite::DuckDbRepository, types::*};
use tauri::State;

#[tauri::command]
pub async fn list_watches(
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<WatchSummary>, AppError> {
    repository.list_watches().await
}

#[tauri::command]
pub async fn save_watch(
    watch: Watch,
    repository: State<'_, DuckDbRepository>,
) -> Result<Watch, AppError> {
    repository.save_watch(watch).await
}

#[tauri::command]
pub async fn delete_watch(
    watch_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<(), AppError> {
    repository.delete_watch(watch_id).await
}

/// Checks one watch, or every enabled watch when `watch_id` is absent. Reads the local corpus and
/// records changes; nothing is fetched from a source.
#[tauri::command]
pub async fn check_watches(
    watch_id: Option<String>,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<WatchCheck>, AppError> {
    repository.check_watches(watch_id).await
}

#[tauri::command]
pub async fn list_watch_changes(
    watch_id: Option<String>,
    limit: u32,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<WatchChange>, AppError> {
    repository.list_watch_changes(watch_id, limit).await
}
