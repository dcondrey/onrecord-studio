//! IPC command for topic packs.

use crate::{
    analysis::topic_packs::{build_topic_packs, TopicPacksReport},
    error::AppError,
    storage::sqlite::DuckDbRepository,
};
use tauri::State;

/// Every topic pack with its tagged documents and requests and the findings whose subject is one
/// of them. Reads only; nothing is recorded.
#[tauri::command]
pub async fn list_topic_packs(
    repository: State<'_, DuckDbRepository>,
) -> Result<TopicPacksReport, AppError> {
    build_topic_packs(&repository).await
}
