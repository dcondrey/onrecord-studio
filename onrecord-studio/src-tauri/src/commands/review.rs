use crate::{error::AppError, storage::sqlite::DuckDbRepository, types::review::*};
use tauri::State;
#[tauri::command]
pub async fn get_review_workspace(
    report_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<ReviewWorkspace, AppError> {
    repository.review_workspace(report_id).await
}
#[tauri::command]
pub async fn save_finding_review(
    input: FindingReviewInput,
    repository: State<'_, DuckDbRepository>,
) -> Result<ReviewWorkspace, AppError> {
    repository.save_finding_review(input).await
}
#[tauri::command]
pub async fn replay_review_evidence(
    report_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<ReviewWorkspace, AppError> {
    repository.replay_review_evidence(report_id).await
}
#[tauri::command]
pub async fn save_public_relationship(
    relationship: PublicRelationship,
    expected_revision: u64,
    repository: State<'_, DuckDbRepository>,
) -> Result<ReviewWorkspace, AppError> {
    repository
        .save_public_relationship(relationship, expected_revision)
        .await
}
#[tauri::command]
pub async fn get_investigation_evaluations(
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<InvestigationEvaluation>, AppError> {
    repository.investigation_evaluations().await
}
#[tauri::command]
pub async fn save_pilot_assessment(
    assessment: PilotAssessment,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<InvestigationEvaluation>, AppError> {
    repository.save_pilot_assessment(assessment).await
}
