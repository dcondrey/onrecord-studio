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
