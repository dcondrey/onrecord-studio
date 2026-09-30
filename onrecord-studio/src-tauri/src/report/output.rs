//! Output-path confinement and no-overwrite commit for report files. `commands::export` keeps
//! equivalents private; these are duplicated (they are not cryptography) rather than edited into
//! that file.

use crate::error::AppError;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub async fn validate_output_path(value: &str, output_root: &Path) -> Result<PathBuf, AppError> {
    validate_output_path_with_extension(value, output_root, "html").await
}

pub async fn validate_output_path_with_extension(
    value: &str,
    output_root: &Path,
    required_extension: &str,
) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(value.trim());
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(required_extension))
    {
        return Err(AppError::Export(format!(
            "Output must be a non-empty .{required_extension} file path"
        )));
    }
    if tokio::fs::try_exists(&path)
        .await
        .map_err(|error| AppError::Export(format!("Failed to inspect report path: {error}")))?
    {
        return Err(AppError::Export(format!(
            "Report path '{}' already exists; reports are never overwritten",
            path.display()
        )));
    }
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .ok_or_else(|| AppError::Export("Output must include a parent directory".to_string()))?;
    let parent = tokio::fs::canonicalize(parent).await.map_err(|error| {
        AppError::Export(format!("Report parent directory is inaccessible: {error}"))
    })?;
    let output_root = tokio::fs::canonicalize(output_root)
        .await
        .map_err(|error| {
            AppError::Export(format!("Managed report directory is inaccessible: {error}"))
        })?;
    if !parent.starts_with(&output_root) {
        return Err(AppError::Export(
            "Exports are restricted to the managed report directory".to_string(),
        ));
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Export("Output has no file name".to_string()))?;
    Ok(parent.join(file_name))
}

/// Writes a staging file, syncs it, then hard-links it into place: the link fails if the target
/// appeared meanwhile, so a concurrent export is never overwritten.
pub async fn commit_new_file(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Export("Report output has no parent directory".to_string()))?;
    let temporary = parent.join(format!(".onrecord-report-{}.tmp", Uuid::new_v4()));
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .await
        .map_err(|error| AppError::Export(format!("Failed to create staging file: {error}")))?;
    let written = async {
        file.write_all(bytes).await?;
        file.sync_all().await
    }
    .await;
    drop(file);
    if let Err(error) = written {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(AppError::Export(format!("Failed to write report: {error}")));
    }
    let linked = tokio::fs::hard_link(&temporary, path).await;
    let _ = tokio::fs::remove_file(&temporary).await;
    linked.map_err(|error| AppError::Export(format!("Failed to commit report: {error}")))
}
