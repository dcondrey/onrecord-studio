use crate::commands::validation::{optional_portal_key, optional_text, validate_text};
use crate::error::AppError;
use crate::storage::fts::core_query_tokens;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::RequestSearchResult;
use tauri::State;

// The supported query contract is deliberately flat: free text plus one portal, agency, status,
// and policy flag. The frontend rejects or labels syntax that cannot map to these fields.

#[tauri::command]
pub async fn query_audit_records(
    search_term: String,
    portal_key: Option<String>,
    agency_id: Option<u64>,
    status: Option<String>,
    policy_deviation: Option<String>,
    limit: Option<usize>,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<RequestSearchResult>, AppError> {
    validate_text(&search_term, "Search term", 4_096)?;
    let portal_key = optional_portal_key(portal_key)?;
    let status = optional_text(status, "Request status", 128)?;
    let policy_deviation = optional_text(policy_deviation, "Policy deviation", 128)?;
    let limit = limit.unwrap_or(500);
    // A corpus whose token projection is still being built would answer with fewer matches than it
    // holds, and a short result list reads as "these are the records", not as "the index is not
    // ready". Refusing is the only answer that is true while the arrears drain. Only a query that
    // reads the projection is affected: with no searchable term the filters alone decide the
    // result set, and refusing those would hide every record for the whole drain window.
    let reads_token_projection = core_query_tokens(search_term.trim())
        .map(|tokens| !tokens.is_empty())
        .unwrap_or(false);
    let pending = repository.pending_search_index_documents();
    if reads_token_projection && pending > 0 {
        return Err(AppError::Storage(if repository.search_index_stalled() {
            format!(
                "The search index stopped before it finished: {pending} documents are not \
                 indexed. Restart the application to resume building it."
            )
        } else {
            format!(
                "The search index is still being built: {pending} documents are not yet \
                 indexed. Searching now would report fewer matches than the corpus holds."
            )
        }));
    }
    repository
        .search_requests(
            search_term.trim(),
            portal_key.as_deref(),
            agency_id,
            status.as_deref(),
            policy_deviation.as_deref(),
            limit,
        )
        .await
}
