//! Read-only native runtime capability reporting.

use crate::error::AppError;
use crate::licensing::LicenseGate;
use crate::parsers::{probe_ocr_capability, OcrCapability};
use crate::types::LicenseStatus;
use tauri::State;

#[tauri::command]
pub async fn get_license_status(
    license: State<'_, LicenseGate>,
) -> Result<LicenseStatus, AppError> {
    Ok(license.status())
}

#[tauri::command]
pub async fn refresh_license(license: State<'_, LicenseGate>) -> Result<LicenseStatus, AppError> {
    Ok(license.refresh().await)
}

#[tauri::command]
pub async fn get_ocr_capability() -> Result<OcrCapability, AppError> {
    tokio::task::spawn_blocking(probe_ocr_capability)
        .await
        .map_err(|error| AppError::Internal(format!("OCR capability probe failed: {error}")))
}
