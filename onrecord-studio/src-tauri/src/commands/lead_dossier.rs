use crate::analysis::lead_dossier;
use crate::commands::validation::required_text;
use crate::error::AppError;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::LeadDossier;
use tauri::State;

#[tauri::command]
pub async fn build_lead_dossier(
    finding_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<LeadDossier, AppError> {
    let finding_id = required_text(&finding_id, "Finding id", 256)?.to_string();
    lead_dossier::build(repository.inner(), finding_id.clone())
        .await?
        .ok_or_else(|| AppError::Analysis(format!("No finding with id {finding_id}")))
}
