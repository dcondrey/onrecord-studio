//! Exact counts and cursor pages for evidence search and entity neighbourhoods.
//!
//! A result set is a total plus pages of any size, never a list that stopped at a ceiling.

use crate::commands::query::refuse_while_indexing;
use crate::commands::validation::{optional_portal_key, optional_text, validate_text};
use crate::error::AppError;
use crate::storage::corpus::neighbors::{
    EntityNeighborCounts, EntityNeighborFilter, EntityNeighborPage,
};
use crate::storage::corpus::CorpusIndex;
use crate::storage::search_paging::{AuditRecordCount, AuditRecordPage};
use crate::storage::sqlite::DuckDbRepository;
use tauri::State;

struct SearchArguments {
    search_term: String,
    portal_key: Option<String>,
    status: Option<String>,
    policy_deviation: Option<String>,
}

fn validated(
    search_term: String,
    portal_key: Option<String>,
    status: Option<String>,
    policy_deviation: Option<String>,
    repository: &DuckDbRepository,
) -> Result<SearchArguments, AppError> {
    validate_text(&search_term, "Search term", 4_096)?;
    let arguments = SearchArguments {
        search_term: search_term.trim().to_string(),
        portal_key: optional_portal_key(portal_key)?,
        status: optional_text(status, "Request status", 128)?,
        policy_deviation: optional_text(policy_deviation, "Policy deviation", 128)?,
    };
    refuse_while_indexing(repository, &arguments.search_term)?;
    Ok(arguments)
}

/// Exact number of requests matching the search. Refuses while the search index is still being
/// built, because a partial index gives a false total.
#[tauri::command]
pub async fn count_audit_records(
    search_term: String,
    portal_key: Option<String>,
    agency_id: Option<u64>,
    status: Option<String>,
    policy_deviation: Option<String>,
    statutory_deadline_exceeded: Option<bool>,
    repository: State<'_, DuckDbRepository>,
) -> Result<AuditRecordCount, AppError> {
    let arguments = validated(
        search_term,
        portal_key,
        status,
        policy_deviation,
        &repository,
    )?;
    let total = repository
        .count_requests(
            &arguments.search_term,
            arguments.portal_key.as_deref(),
            agency_id,
            arguments.status.as_deref(),
            arguments.policy_deviation.as_deref(),
            statutory_deadline_exceeded.unwrap_or(false),
        )
        .await?;
    Ok(AuditRecordCount { total })
}

/// One page of matches in `created_at DESC, portal, id` order. Pass the previous page's
/// `next_cursor` back unchanged for the next page; the first page also carries the exact total.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn query_audit_records_page(
    search_term: String,
    portal_key: Option<String>,
    agency_id: Option<u64>,
    status: Option<String>,
    policy_deviation: Option<String>,
    statutory_deadline_exceeded: Option<bool>,
    cursor: Option<String>,
    page_size: usize,
    repository: State<'_, DuckDbRepository>,
) -> Result<AuditRecordPage, AppError> {
    let arguments = validated(
        search_term,
        portal_key,
        status,
        policy_deviation,
        &repository,
    )?;
    repository
        .search_requests_page(
            &arguments.search_term,
            arguments.portal_key.as_deref(),
            agency_id,
            arguments.status.as_deref(),
            arguments.policy_deviation.as_deref(),
            statutory_deadline_exceeded.unwrap_or(false),
            cursor.as_deref(),
            page_size,
        )
        .await
}

/// One page of an entity's neighbours, strongest first. `None` when the id names no entity.
#[tauri::command]
pub async fn page_corpus_entity_neighbors(
    entity_id: String,
    cursor: Option<String>,
    page_size: usize,
    filter: Option<EntityNeighborFilter>,
    index: State<'_, CorpusIndex>,
) -> Result<Option<EntityNeighborPage>, AppError> {
    index
        .entity_neighbors_page(entity_id, cursor, page_size, filter)
        .await
}

/// Exact neighbour totals for an entity, per kind and overall. `None` when the id names no entity.
#[tauri::command]
pub async fn count_corpus_entity_neighbors(
    entity_id: String,
    filter: Option<EntityNeighborFilter>,
    index: State<'_, CorpusIndex>,
) -> Result<Option<EntityNeighborCounts>, AppError> {
    index.entity_neighbor_counts(entity_id, filter).await
}
