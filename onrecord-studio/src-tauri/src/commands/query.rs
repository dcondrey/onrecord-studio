use crate::commands::validation::{optional_portal_key, optional_text, validate_text};
use crate::error::AppError;
use crate::storage::sqlite::SqliteRepository;
use crate::types::RequestSearchResult;
use tauri::State;

// TODO(query): Accept the typed QueryAst payload used by the frontend once
// all filter operators have a corresponding repository execution plan.
// FEATURE(query): Add saved-query identifiers and explain-query metadata so
// investigators can reproduce and cite the exact search that produced a result.

#[tauri::command]
pub async fn query_audit_records(
    search_term: String,
    portal_key: Option<String>,
    agency_id: Option<u64>,
    status: Option<String>,
    policy_deviation: Option<String>,
    limit: Option<usize>,
    repository: State<'_, SqliteRepository>,
) -> Result<Vec<RequestSearchResult>, AppError> {
    validate_text(&search_term, "Search term", 4_096)?;
    let portal_key = optional_portal_key(portal_key)?;
    let status = optional_text(status, "Request status", 128)?;
    let policy_deviation = optional_text(policy_deviation, "Policy deviation", 128)?;
    let limit = limit.unwrap_or(500);
    // TODO(query): Replace these flat filters with a typed expression tree so grouped AND/OR,
    // negation, ranges, and document-level qualifiers are enforced inside the Rust query plan.
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
