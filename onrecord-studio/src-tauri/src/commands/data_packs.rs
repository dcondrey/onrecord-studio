use crate::commands::validation::{required_text, validate_text};
use crate::error::AppError;
use crate::packs::{import, pinned_pack_keys};
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{DataPackImportOutcome, DataPackSummary};
use std::path::PathBuf;
use tauri::State;

const MAX_SELECTED_FILES: usize = 1_000;

fn pack_directory(path: &str) -> Result<PathBuf, AppError> {
    Ok(PathBuf::from(required_text(
        path,
        "Data pack folder",
        4096,
    )?))
}

#[tauri::command]
pub async fn inspect_data_pack(path: String) -> Result<DataPackSummary, AppError> {
    import::inspect(pack_directory(&path)?, pinned_pack_keys()).await
}

#[tauri::command]
pub async fn import_data_pack_files(
    path: String,
    files: Vec<String>,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<DataPackImportOutcome>, AppError> {
    if files.len() > MAX_SELECTED_FILES {
        return Err(AppError::DataParsing(format!(
            "Choose at most {MAX_SELECTED_FILES} files per import"
        )));
    }
    for file in &files {
        validate_text(file, "Data pack file", 4096)?;
    }
    import::import_files(
        &repository,
        pack_directory(&path)?,
        files,
        pinned_pack_keys(),
    )
    .await
}
