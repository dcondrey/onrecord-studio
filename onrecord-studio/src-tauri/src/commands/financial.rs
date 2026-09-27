use crate::{error::AppError, storage::sqlite::DuckDbRepository, types::*};
use tauri::State;
#[tauri::command]
pub async fn get_financial_workspace(
    repository: State<'_, DuckDbRepository>,
) -> Result<FinancialWorkspace, AppError> {
    repository.financial_workspace().await
}
#[tauri::command]
pub async fn save_financial_mapping(
    mapping: FinancialMapping,
    repository: State<'_, DuckDbRepository>,
) -> Result<FinancialWorkspace, AppError> {
    repository.save_financial_mapping(mapping).await
}
#[tauri::command]
pub async fn run_financial_investigation(
    scope: FinancialScope,
    repository: State<'_, DuckDbRepository>,
) -> Result<FinancialReport, AppError> {
    repository.run_financial_investigation(scope).await
}
#[tauri::command]
pub async fn evaluate_financial_scenario(
    input: FinancialScenarioInput,
    repository: State<'_, DuckDbRepository>,
) -> Result<FinancialScenario, AppError> {
    repository.evaluate_financial_scenario(input).await
}
