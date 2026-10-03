use crate::analysis::dispositions::DepartmentDispositions;
use crate::{error::AppError, storage::DuckDbRepository};
use tauri::State;

/// Disposition counts per department, for one portal or, when `portal_subdomain` is absent, every
/// portal. Descriptive counts of what each agency answered, not findings.
#[tauri::command]
pub async fn get_department_dispositions(
    portal_subdomain: Option<String>,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<DepartmentDispositions>, AppError> {
    repository.department_dispositions(portal_subdomain).await
}
