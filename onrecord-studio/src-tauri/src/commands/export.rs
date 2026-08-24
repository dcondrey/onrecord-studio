//! IPC bridges handling analytical markdown exports.

use std::fs;
use crate::error::AppError;

#[tauri::command]
pub async fn export_case_dataset(
    target_path: String,
    report_content: String,
) -> Result<(), AppError> {
    fs::write(&target_path, report_content)
        .map_err(|e| AppError::Internal(format!("Failed to persist analytical report: {}", e)))
}
