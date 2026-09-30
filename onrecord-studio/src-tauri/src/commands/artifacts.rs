//! IPC commands for drafted follow-up requests and appeals.
//!
//! Each command takes a finding id, loads the finding from storage, and delegates to
//! [`crate::analysis::artifacts`] to produce a `DraftedArtifact`. Neither command sends anything
//! anywhere — this API produces drafts, period.

use crate::{
    analysis::artifacts::{
        appeal::draft_appeal, followup_request::draft_followup, ArtifactError, DraftedArtifact,
    },
    error::AppError,
    storage::sqlite::DuckDbRepository,
};
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
    }
}

#[tauri::command]
pub async fn draft_followup_from_finding(
    finding_id: String,
    jurisdiction: Option<String>,
    repository: State<'_, DuckDbRepository>,
) -> Result<DraftedArtifact, AppError> {
    let jurisdiction = jurisdiction.unwrap_or_else(|| DEFAULT_JURISDICTION.to_string());
    let finding = repository
        .get_finding_by_id(finding_id.clone())
        .await?
        .ok_or_else(|| AppError::Analysis(format!("No finding found with id {finding_id}")))?;
    draft_followup(&finding, &jurisdiction).map_err(map_artifact_error)
}

#[tauri::command]
pub async fn draft_appeal_from_finding(
    finding_id: String,
    jurisdiction: Option<String>,
    repository: State<'_, DuckDbRepository>,
) -> Result<DraftedArtifact, AppError> {
    let jurisdiction = jurisdiction.unwrap_or_else(|| DEFAULT_JURISDICTION.to_string());
    let finding = repository
        .get_finding_by_id(finding_id.clone())
        .await?
        .ok_or_else(|| AppError::Analysis(format!("No finding found with id {finding_id}")))?;
    draft_appeal(&finding, &jurisdiction).map_err(map_artifact_error)
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
