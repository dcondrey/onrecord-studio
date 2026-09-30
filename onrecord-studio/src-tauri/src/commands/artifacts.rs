//! IPC commands for drafted follow-up requests and appeals.
//!
//! Each command takes a finding id, loads the finding from storage, and delegates to
//! [`crate::analysis::artifacts`] to produce a `DraftedArtifact`. Neither command sends anything
//! anywhere — this API produces drafts, period.

use crate::{
    analysis::artifacts::{
        appeal::draft_appeal, followup_request::draft_followup,
        statutory_appeal::draft_for_request, ArtifactError, DraftAvailability, DraftedArtifact,
    },
    commands::profile::disclosure_context,
    error::AppError,
    profile::ProfileStore,
    storage::{corpus::CorpusIndex, sqlite::DuckDbRepository},
};
use std::sync::Arc;
use tauri::State;

/// Default jurisdiction used when the caller does not specify one. CPRA is the only jurisdiction
/// with templates today; extending the templates table is how a new jurisdiction is added.
const DEFAULT_JURISDICTION: &str = "US-CA";

fn map_artifact_error(error: ArtifactError) -> AppError {
    match error {
        ArtifactError::NoApplicableTemplate {
            kind,
            finding_kind,
        } => AppError::Analysis(format!(
            "No {kind:?} template applies to a finding of kind {finding_kind:?}. Add a template for this kind in the artifact templates registry to enable this action."
        )),
        ArtifactError::MissingField(field) => AppError::Analysis(format!(
            "Finding is missing a field the draft needs: {field}"
        )),
        other @ (ArtifactError::NotALegalClaim { .. } | ArtifactError::AppealNotSupported(_)) => {
            AppError::Analysis(other.to_string())
        }
    }
}

#[tauri::command]
pub async fn draft_followup_from_finding(
    finding_id: String,
    jurisdiction: Option<String>,
    repository: State<'_, DuckDbRepository>,
    profile_store: State<'_, Arc<ProfileStore>>,
    corpus: State<'_, CorpusIndex>,
) -> Result<DraftedArtifact, AppError> {
    let jurisdiction = jurisdiction.unwrap_or_else(|| DEFAULT_JURISDICTION.to_string());
    let finding = repository
        .get_finding_by_id(finding_id.clone())
        .await?
        .ok_or_else(|| AppError::Analysis(format!("No finding found with id {finding_id}")))?;
    let disclosure = disclosure_context(&profile_store, &repository, &corpus).await?;
    draft_followup(&finding, &jurisdiction, &disclosure).map_err(map_artifact_error)
}

#[tauri::command]
pub async fn draft_appeal_from_finding(
    finding_id: String,
    jurisdiction: Option<String>,
    repository: State<'_, DuckDbRepository>,
    profile_store: State<'_, Arc<ProfileStore>>,
    corpus: State<'_, CorpusIndex>,
) -> Result<DraftedArtifact, AppError> {
    let jurisdiction = jurisdiction.unwrap_or_else(|| DEFAULT_JURISDICTION.to_string());
    let finding = repository
        .get_finding_by_id(finding_id.clone())
        .await?
        .ok_or_else(|| AppError::Analysis(format!("No finding found with id {finding_id}")))?;
    let disclosure = disclosure_context(&profile_store, &repository, &corpus).await?;
    draft_appeal(&finding, &jurisdiction, &disclosure).map_err(map_artifact_error)
}

/// Which drafts the UI may offer for a finding, decided from the finding's stored kind.
#[tauri::command]
pub async fn draft_availability_for_finding(
    finding_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<DraftAvailability, AppError> {
    let finding = repository
        .get_finding_by_id(finding_id.clone())
        .await?
        .ok_or_else(|| AppError::Analysis(format!("No finding found with id {finding_id}")))?;
    Ok(DraftAvailability::for_kind(finding.kind))
}

/// Drafts an appeal for a request the statute evaluator counts as past its statutory deadline.
/// Refuses when the determination is a proxy or the extension record is defective.
#[tauri::command]
pub async fn draft_appeal_from_statutory_deadline(
    portal_key: String,
    request_id: u64,
    jurisdiction: Option<String>,
    repository: State<'_, DuckDbRepository>,
    profile_store: State<'_, Arc<ProfileStore>>,
    corpus: State<'_, CorpusIndex>,
) -> Result<DraftedArtifact, AppError> {
    let jurisdiction = jurisdiction.unwrap_or_else(|| DEFAULT_JURISDICTION.to_string());
    let disclosure = disclosure_context(&profile_store, &repository, &corpus).await?;
    draft_for_request(
        &repository,
        &portal_key,
        request_id,
        &jurisdiction,
        &disclosure,
    )
    .await
}

#[cfg(test)]
mod tests {
    // The wrappers are thin adapters over `storage::findings::get_finding_by_id` and
    // `analysis::artifacts::{draft_followup, draft_appeal}`. Those are covered by their own
    // exhaustive unit tests. A duplicate test here would restate the implementation. See:
    // - `storage::findings::tests` for the round-trip storage semantics.
    // - `analysis::artifacts::followup_request::tests` for the followup-drafting invariants.
    // - `analysis::artifacts::appeal::tests` for the appeal-drafting invariants.
}
