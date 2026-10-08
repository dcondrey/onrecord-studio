use crate::commands::validation::{required_text, validate_text};
use crate::error::AppError;
use crate::integrity::is_sha256_hex;
use crate::packs::published::{self, PublishedPackRoot};
use crate::packs::{import, pack_relative, pinned_pack_keys};
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{
    DataPackChangeRows, DataPackImportOutcome, DataPackLocation, DataPackSummary, PublishedPack,
    PublishedPackSelection,
};
use std::path::PathBuf;
use tauri::State;

const MAX_SELECTED_FILES: usize = 1_000;

fn require_inspected(manifest_sha256: &str) -> Result<(), AppError> {
    if !is_sha256_hex(manifest_sha256) {
        return Err(AppError::DataParsing(
            "Inspect the data pack before importing from it".into(),
        ));
    }
    Ok(())
}

fn require_selection_size(count: usize) -> Result<(), AppError> {
    if count > MAX_SELECTED_FILES {
        return Err(AppError::DataParsing(format!(
            "Choose at most {MAX_SELECTED_FILES} files per import"
        )));
    }
    Ok(())
}

#[tauri::command]
pub async fn list_published_packs() -> Result<Vec<PublishedPack>, AppError> {
    Ok(published::published_packs())
}

#[tauri::command]
pub async fn fetch_published_pack_manifest(
    pack: String,
    root: State<'_, PublishedPackRoot>,
) -> Result<DataPackSummary, AppError> {
    let pack = required_text(&pack, "Data pack", 64)?;
    published::fetch_manifest(root.0.clone(), pack, pinned_pack_keys()).await
}

#[tauri::command]
pub async fn fetch_published_pack_files(
    pack: String,
    pack_version: String,
    manifest_sha256: String,
    selection: Vec<PublishedPackSelection>,
    root: State<'_, PublishedPackRoot>,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<DataPackImportOutcome>, AppError> {
    require_inspected(&manifest_sha256)?;
    require_selection_size(selection.len())?;
    for chosen in &selection {
        validate_text(&chosen.table, "Data pack table", 128)?;
    }
    published::fetch_files(
        &repository,
        root.0.clone(),
        required_text(&pack, "Data pack", 64)?,
        required_text(&pack_version, "Data pack version", 64)?,
        &manifest_sha256,
        selection,
        pinned_pack_keys(),
    )
    .await
}

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

/// A page of the change record `path` from the pack at `location`, while its manifest is still
/// the one inspected (`manifest_sha256`).
#[tauri::command]
pub async fn read_data_pack_change_record(
    location: DataPackLocation,
    manifest_sha256: String,
    path: String,
    offset: u64,
    limit: u64,
    root: State<'_, PublishedPackRoot>,
) -> Result<DataPackChangeRows, AppError> {
    require_inspected(&manifest_sha256)?;
    let record = required_text(&path, "Change record", 4096)?.to_owned();
    let (kept, file) = match location {
        DataPackLocation::Folder { path } => {
            let directory = pack_directory(&path)?;
            let file = import::listed_change_record(
                directory.clone(),
                manifest_sha256,
                record.clone(),
                pinned_pack_keys(),
            )
            .await?;
            let relative = pack_relative(&file.path).ok_or_else(|| {
                AppError::DataParsing(format!("{record} is a path outside the pack"))
            })?;
            (directory.join(relative), file)
        }
        DataPackLocation::Published { pack, pack_version } => {
            published::change_record(
                root.0.clone(),
                required_text(&pack, "Data pack", 64)?,
                required_text(&pack_version, "Data pack version", 64)?,
                manifest_sha256,
                record,
                pinned_pack_keys(),
            )
            .await?
        }
    };
    import::read_change_rows(kept, file, offset, limit).await
}

#[tauri::command]
pub async fn import_data_pack_files(
    path: String,
    manifest_sha256: String,
    files: Vec<String>,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<DataPackImportOutcome>, AppError> {
    require_inspected(&manifest_sha256)?;
    require_selection_size(files.len())?;
    for file in &files {
        validate_text(file, "Data pack file", 4096)?;
    }
    import::import_files(
        &repository,
        pack_directory(&path)?,
        &manifest_sha256,
        files,
        pinned_pack_keys(),
    )
    .await
}
