//! IPC commands for the autonomous findings inbox.

use crate::{
    analysis::{
        detectors,
        finding::Finding as StoredFinding,
        gaps,
        service_leads::{self, LeadFigure, ServiceLeadInput, ServiceLeadsRecorded},
    },
    error::AppError,
    storage::{findings::FindingsFilter, sqlite::DuckDbRepository, vector::VectorIndex},
};
use serde::{Deserialize, Serialize};
use tauri::State;

/// A finding as the inbox reads it: the stored finding, plus a lead's figure on a lookup of one
/// finding by id. Named `Finding` because the IPC contract is typed from a command's result name,
/// and `Finding` in `src/types/findings.ts` carries the optional figure.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    #[serde(flatten)]
    pub finding: StoredFinding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub figure: Option<LeadFigure>,
}

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
    vector_index: State<'_, VectorIndex>,
) -> Result<RefreshFindingsSummary, AppError> {
    let summary =
        detectors::refresh_all_findings_with_embeddings(&repository, Some(&vector_index)).await?;
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
    // Figures go with a lookup of one finding only: a page of up to 500 findings with a figure
    // each would be megabytes over IPC for slots that draw one at a time.
    let lookup = filter.id.clone();
    let findings = repository.list_findings(filter, limit).await?;
    let figure = match (lookup, findings.as_slice()) {
        (Some(id), [found]) if found.id == id => service_leads::figure(&repository, id).await?,
        _ => None,
    };
    Ok(findings
        .into_iter()
        .map(|finding| Finding {
            finding,
            figure: figure.clone(),
        })
        .collect())
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

/// Stores the discovery feed's service-report leads as findings. `complete` is true only when
/// every imported table was checked without error; otherwise earlier leads are left in place.
#[tauri::command]
pub async fn record_service_leads(
    leads: Vec<ServiceLeadInput>,
    complete: bool,
    repository: State<'_, DuckDbRepository>,
) -> Result<ServiceLeadsRecorded, AppError> {
    let run_started = chrono::Utc::now();
    let schema = repository.analytical_schema().await?;
    let findings = service_leads::to_findings(&leads, &schema.datasets, run_started)?;
    service_leads::store(&repository, findings, complete, run_started).await
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
