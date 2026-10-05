use crate::{
    error::AppError,
    ingestion::client::{ApiClient, HttpClientConfig},
    ingestion::organisation_filings,
    storage::sqlite::DuckDbRepository,
    types::*,
};
use tauri::State;

#[tauri::command]
pub async fn search_organisations(query: String) -> Result<Vec<OrganisationMatch>, AppError> {
    let client = ApiClient::new(HttpClientConfig::default())?;
    organisation_filings::search_organisations(&client, &query).await
}

/// Downloads each IRS index year not yet held (about 90 MB a year) and the organisation's
/// returns, then the summary and, when a key is given, the single audits.
#[tauri::command]
pub async fn check_organisation_filings(
    ein: String,
    audit_key: Option<String>,
    repository: State<'_, DuckDbRepository>,
) -> Result<OrganisationFilings, AppError> {
    let client = ApiClient::new(HttpClientConfig::default())?;
    organisation_filings::check_organisation(&client, &repository, &ein, audit_key.as_deref()).await
}

#[tauri::command]
pub async fn get_organisation_filings(
    ein: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<Option<OrganisationFilings>, AppError> {
    repository
        .organisation_filings(&organisation_filings::normalise_ein(&ein)?)
        .await
}

#[tauri::command]
pub async fn list_organisation_checks(
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<OrganisationCheckSummary>, AppError> {
    repository.list_organisation_checks().await
}

#[tauri::command]
pub async fn list_organisation_payment_mappings(
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<OrganisationPaymentMapping>, AppError> {
    repository.organisation_payment_mappings().await
}

#[tauri::command]
pub async fn save_organisation_payment_mapping(
    mapping: OrganisationPaymentMapping,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<OrganisationPaymentMapping>, AppError> {
    repository.save_organisation_payment_mapping(mapping).await
}

#[tauri::command]
pub async fn remove_organisation_payment_mapping(
    snapshot_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<OrganisationPaymentMapping>, AppError> {
    repository
        .remove_organisation_payment_mapping(&snapshot_id)
        .await
}

/// Writes a table the view built (the checked-organisations CSV) to a path the user chose.
#[tauri::command]
pub async fn export_organisation_portfolio_csv(
    output_path: String,
    content: String,
) -> Result<(), AppError> {
    use crate::commands::validation::{validate_filesystem_target_path, validate_text};
    validate_filesystem_target_path(&output_path, "Export path", 4096)?;
    validate_text(&content, "Portfolio CSV", 16 * 1024 * 1024)?;
    tokio::task::spawn_blocking(move || {
        crate::atomic_write::write_atomic(
            std::path::Path::new(&output_path),
            "Portfolio CSV",
            content.as_bytes(),
        )
    })
    .await
    .map_err(|error| AppError::Internal(format!("Portfolio CSV export task: {error}")))?
}
