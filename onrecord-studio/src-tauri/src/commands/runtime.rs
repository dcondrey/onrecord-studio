//! Read-only native runtime capability reporting.

use crate::error::AppError;
use crate::parsers::{probe_ocr_capability, OcrCapability};

#[tauri::command]
pub async fn get_ocr_capability() -> Result<OcrCapability, AppError> {
    tokio::task::spawn_blocking(probe_ocr_capability)
        .await
        .map_err(|error| AppError::Internal(format!("OCR capability probe failed: {error}")))
}
