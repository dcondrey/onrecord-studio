use crate::{
    error::AppError,
    storage::sqlite::DuckDbRepository,
    types::{ReconstructionOptions, ReconstructionReport},
};
use tauri::State;

#[tauri::command]
pub async fn reconstruct_document_context(
    document_id: u64,
    portal_key: String,
    options: Option<ReconstructionOptions>,
    repository: State<'_, DuckDbRepository>,
) -> Result<ReconstructionReport, AppError> {
    let portal = super::validation::portal_key(&portal_key)?;
    repository
        .reconstruct_document(document_id, portal, options.unwrap_or_default())
        .await
}
