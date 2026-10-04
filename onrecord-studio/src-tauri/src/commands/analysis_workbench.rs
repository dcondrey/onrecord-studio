use crate::commands::validation::{
    required_text, validate_filesystem_target_path, validate_import_source_path, validate_text,
};
use crate::error::AppError;
use crate::ingestion::queue::QueueManager;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{
    AnalyticalImportCandidate, AnalyticalImportRequest, AnalyticalImportResult, AnalyticalSchema,
    CityDataset, EncampmentOrdinanceReport, SqlExportResult, SqlQueryResult,
};
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn import_analytical_dataset(
    input: AnalyticalImportRequest,
    repository: State<'_, DuckDbRepository>,
) -> Result<AnalyticalImportResult, AppError> {
    required_text(&input.display_name, "Dataset display name", 256)?;
    validate_import_source_path(&input.local_path, "Dataset file path", 4096)?;
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
pub async fn fetch_city_dataset(
    dataset: CityDataset,
    year: Option<u16>,
    manager: State<'_, Arc<QueueManager>>,
) -> Result<AnalyticalImportResult, AppError> {
    manager.fetch_city_dataset(dataset, year).await
}

#[tauri::command]
pub async fn get_encampment_ordinance_report(
    repository: State<'_, DuckDbRepository>,
) -> Result<EncampmentOrdinanceReport, AppError> {
    repository.encampment_ordinance_report().await
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
    validate_filesystem_target_path(&output_path, "Export path", 4096)?;
    repository
        .export_restricted_sql_csv(&sql, &output_path, limit.unwrap_or(10_000))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_path_validation_rejects_a_symlinked_target() {
        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("real.csv");
        std::fs::write(&real, b"data").unwrap();
        let link = directory.path().join("link.csv");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(unix)]
        {
            let error =
                validate_filesystem_target_path(link.to_str().unwrap(), "Export path", 4096)
                    .unwrap_err();
            assert!(
                error.to_string().contains("symbolic link"),
                "unexpected error: {error}"
            );
        }
    }

    #[test]
    fn export_path_validation_accepts_a_target_that_does_not_exist_yet() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("new-export.csv");
        assert!(
            validate_filesystem_target_path(target.to_str().unwrap(), "Export path", 4096).is_ok()
        );
    }

    #[test]
    fn import_path_validation_rejects_a_path_that_does_not_exist() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.csv");
        let error =
            validate_import_source_path(missing.to_str().unwrap(), "Dataset file path", 4096)
                .unwrap_err();
        assert!(
            error.to_string().contains("IMPORT_INSPECTION_FAILED"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn import_path_validation_rejects_a_directory() {
        let directory = tempfile::tempdir().unwrap();
        let error = validate_import_source_path(
            directory.path().to_str().unwrap(),
            "Dataset file path",
            4096,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("regular, non-symlink file"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn import_path_validation_rejects_a_symlinked_source() {
        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("real.csv");
        std::fs::write(&real, b"data").unwrap();
        let link = directory.path().join("link.csv");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(unix)]
        {
            let error =
                validate_import_source_path(link.to_str().unwrap(), "Dataset file path", 4096)
                    .unwrap_err();
            assert!(
                error.to_string().contains("non-symlink"),
                "unexpected error: {error}"
            );
        }
    }
}
