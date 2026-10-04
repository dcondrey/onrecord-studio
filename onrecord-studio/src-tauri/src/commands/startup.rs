//! IPC for the startup screen. Both commands depend only on state managed before the window
//! appears, so they answer while the databases are still being prepared.

use crate::error::AppError;
use crate::startup::{StartupState, StartupStatus};
use tauri::State;

const MAXIMUM_ACKNOWLEDGED_IDS: usize = 64;
const MAXIMUM_ID_LENGTH: usize = 64;

#[tauri::command]
pub async fn get_startup_status(
    startup: State<'_, StartupState>,
) -> Result<StartupStatus, AppError> {
    Ok(startup.snapshot())
}

/// Dismisses startup notices so no later launch shows them again.
#[tauri::command]
pub async fn acknowledge_startup_notices(
    ids: Vec<String>,
    startup: State<'_, StartupState>,
) -> Result<StartupStatus, AppError> {
    if ids.len() > MAXIMUM_ACKNOWLEDGED_IDS
        || ids
            .iter()
            .any(|id| id.is_empty() || id.len() > MAXIMUM_ID_LENGTH)
    {
        return Err(AppError::Storage(
            "Startup notice identifiers are malformed.".to_string(),
        ));
    }
    Ok(startup.acknowledge(&ids))
}
