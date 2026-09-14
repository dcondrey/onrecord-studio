use crate::commands::validation::{required_text, validate_text};
use crate::error::AppError;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{
    AnalyticalImportCandidate, AnalyticalImportRequest, AnalyticalImportResult, AnalyticalSchema,
    SqlExportResult, SqlQueryResult,
};
use tauri::State;

#[tauri::command]
pub async fn import_analytical_dataset(
    input: AnalyticalImportRequest,
    repository: State<'_, DuckDbRepository>,
) -> Result<AnalyticalImportResult, AppError> {
    required_text(&input.display_name, "Dataset display name", 256)?;
    required_text(&input.local_path, "Dataset file path", 4096)?;
    if let Some(uri) = &input.source_uri {
        let url = reqwest::Url::parse(required_text(uri, "Original source URL", 4096)?).map_err(
            |_| AppError::DataParsing("Original source URL must be an absolute URL".into()),
        )?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(AppError::DataParsing(
                "Record the original source URL without a username or password".into(),
            ));
        }
    }
    for date in [
        &input.observation_period_start,
        &input.observation_period_end,
    ]
    .into_iter()
    .flatten()
    {
        if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_err() {
            return Err(AppError::DataParsing(
                "Observation dates must use YYYY-MM-DD".into(),
            ));
        }
    }
    if let (Some(start), Some(end)) = (
        &input.observation_period_start,
        &input.observation_period_end,
    ) {
        if start > end {
            return Err(AppError::DataParsing(
                "Observation end must be on or after its start".into(),
            ));
        }
    }
    repository.import_analytical_csv(input).await
}

#[tauri::command]
pub async fn list_analytical_import_candidates(
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<AnalyticalImportCandidate>, AppError> {
    repository.analytical_import_candidates().await
}

#[tauri::command]
pub async fn get_analytical_schema(
    repository: State<'_, DuckDbRepository>,
) -> Result<AnalyticalSchema, AppError> {
    repository.analytical_schema().await
}

#[tauri::command]
pub async fn run_analytical_sql(
    sql: String,
    limit: Option<usize>,
    repository: State<'_, DuckDbRepository>,
) -> Result<SqlQueryResult, AppError> {
    validate_text(&sql, "SQL", 64 * 1024)?;
    repository
        .execute_restricted_sql(&sql, limit.unwrap_or(500))
        .await
}

#[tauri::command]
pub async fn export_analytical_sql_csv(
    sql: String,
    output_path: String,
    limit: Option<usize>,
    repository: State<'_, DuckDbRepository>,
) -> Result<SqlExportResult, AppError> {
    validate_text(&sql, "SQL", 64 * 1024)?;
    required_text(&output_path, "Export path", 4096)?;
    repository
        .export_restricted_sql_csv(&sql, &output_path, limit.unwrap_or(10_000))
        .await
}
