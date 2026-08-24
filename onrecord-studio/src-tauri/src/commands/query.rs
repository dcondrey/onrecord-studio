//! IPC commands running targeted full-text searches and structural data filters.

use tauri::State;
use crate::error::AppError;
use crate::storage::sqlite::SqliteRepository;

#[tauri::command]
pub async fn query_audit_records(
    search_term: String,
    repo: State<'_, SqliteRepository>,
) -> Result<Vec<u64>, AppError> {
    if search_term.trim().is_empty() {
        return Ok(Vec::new());
    }
    
    let clean_query = crate::storage::fts::format_fts5_query(&search_term);
    repo.full_text_search(&clean_query).await
}
