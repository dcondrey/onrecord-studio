//! IPC command for public-source starter sets.

use crate::{
    error::AppError,
    ingestion::starter_sets::{
        starter_sets_report, validate_registry, StarterSetsReport, STARTER_SETS,
    },
};

/// The starter-set registry. Reads static data only: no network request is made and nothing is
/// recorded. Adding a set's sources is the operator's separate, confirmed action through
/// `start_data_source`.
#[tauri::command]
pub async fn list_starter_sets() -> Result<StarterSetsReport, AppError> {
    validate_registry(STARTER_SETS)?;
    Ok(starter_sets_report())
}
