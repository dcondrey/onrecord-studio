//! IPC commands for the autonomous findings inbox.

use crate::{
    analysis::{detectors, finding::Finding, gaps},
    error::AppError,
    storage::{findings::FindingsFilter, sqlite::DuckDbRepository},
};
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshFindingsSummary {
    pub detectors_run: usize,
    pub findings_recorded: usize,
    pub stale_cleared: usize,
}

#[tauri::command]
pub async fn refresh_findings(
    repository: State<'_, DuckDbRepository>,
) -> Result<RefreshFindingsSummary, AppError> {
    let summary = detectors::refresh_all_findings(&repository).await?;
    Ok(RefreshFindingsSummary {
        detectors_run: summary.detectors_run,
        findings_recorded: summary.findings_recorded,
        stale_cleared: summary.stale_cleared,
    })
}

/// Routes the corpus identified but cannot yet test, each with where its acquisition stands.
/// Reading queues nothing; the collection service does that after each ingest.
#[tauri::command]
pub async fn list_evidence_gaps(
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<gaps::EvidenceGapView>, AppError> {
    let (found, errors) = gaps::collect(&repository).await;
    if found.is_empty() {
        if let Some(error) = errors.into_iter().next() {
            return Err(AppError::Storage(error));
        }
    }
    gaps::resolve(&repository, &found, false).await
}

#[tauri::command]
pub async fn list_findings(
    filter: Option<FindingsFilter>,
    limit: Option<usize>,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<Finding>, AppError> {
    let filter = filter.unwrap_or_default();
    let limit = limit.unwrap_or(50);
    repository.list_findings(filter, limit).await
}

/// The number of findings a filter names, for strips that show a count and never the rows.
#[tauri::command]
pub async fn count_findings(
    filter: Option<FindingsFilter>,
    repository: State<'_, DuckDbRepository>,
) -> Result<u64, AppError> {
    repository.count_findings(filter.unwrap_or_default()).await
}

#[tauri::command]
pub async fn dismiss_finding(
    id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<(), AppError> {
    repository.dismiss_finding(id).await
}

/// Undoes a dismissal.
#[tauri::command]
pub async fn restore_finding(
    id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<(), AppError> {
    repository.restore_finding(id).await
}

#[cfg(test)]
mod tests {
    // The Tauri command wrappers are thin adapters over `detectors::refresh_all_findings` and the
    // `DuckDbRepository::list_findings`/`dismiss_finding` methods. Both of those have their own
    // exhaustive unit tests (see `analysis::detectors::tests` and `storage::findings::tests`). A
    // duplicate test here that re-verifies the same behavior would be a restatement of the
    // implementation, which the project's engineering rules discourage; the wrapper does no
    // logic of its own that isn't covered upstream.
}
