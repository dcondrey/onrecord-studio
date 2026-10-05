//! IPC command for public-source starter sets.

use crate::{
    error::AppError,
    ingestion::starter_sets::{validate_registry, StarterSetsReport, STARTER_SET_REGISTRY_VERSION},
};

/// The starter-set registry. Reads static data only: no network request is made and nothing is
/// recorded. Adding a set's sources is the operator's separate, confirmed action through
/// `start_data_source`.
#[tauri::command]
pub async fn list_starter_sets() -> Result<StarterSetsReport, AppError> {
    let sets = crate::ingestion::starter_sets::all_starter_sets();
    validate_registry(&sets)?;
    Ok(StarterSetsReport {
        registry_version: STARTER_SET_REGISTRY_VERSION,
        sets,
    })
}
