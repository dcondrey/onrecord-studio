//! Explainable, provenance-linked cross-document pattern queries.

use crate::commands::validation::optional_portal_key;
use crate::error::AppError;
use crate::storage::corpus::CorpusIndex;
use crate::types::{
    CorpusEntityNeighborhood, CorpusEntityNode, CorpusIndexStatus, CorpusPattern,
    CorpusPatternQuery, EntityBroker, EntityPathOutcome,
};
use tauri::State;

#[tauri::command]
pub async fn query_corpus_patterns(
    query: Option<CorpusPatternQuery>,
    index: State<'_, CorpusIndex>,
) -> Result<Vec<CorpusPattern>, AppError> {
    let mut query = query.unwrap_or_default();
    query.portal_key = optional_portal_key(query.portal_key)?;
    index.query_patterns(query).await
}

#[tauri::command]
pub async fn get_corpus_index_status(
    index: State<'_, CorpusIndex>,
) -> Result<CorpusIndexStatus, AppError> {
    index.status().await
}

/// The entities worth starting a graph from, most-documented first.
///
/// `limit` is the caller's, bounded by the index rather than trusted: a node-link picture past a
/// couple of hundred arcs is unreadable, and an unbounded read of a corpus index is not a query.
#[tauri::command]
pub async fn list_top_corpus_entities(
    limit: Option<usize>,
    index: State<'_, CorpusIndex>,
) -> Result<Vec<CorpusEntityNode>, AppError> {
    index.top_entities(limit.unwrap_or(24)).await
}

/// Entity kinds whose extraction stopped at its per-kind budget for one document.
///
/// Empty means every kind ran to completion, or the document was indexed before truncation was
/// recorded; it never means "not checked". A viewer shows a non-empty answer as an incomplete
/// entity list rather than a complete one.
#[tauri::command]
pub async fn get_document_entity_truncations(
    document_id: u64,
    portal_key: String,
    index: State<'_, CorpusIndex>,
) -> Result<Vec<String>, AppError> {
    index.truncated_entity_kinds(portal_key, document_id).await
}

/// The subgraph around one entity.
///
/// `None` when the identifier names no entity, which is a real answer and not an error: a corpus
/// rebuild discards every entity id, so a view holding one from before it can ask and be told the
/// node is gone rather than shown an empty graph that looks like an isolated node.
#[tauri::command]
pub async fn get_corpus_entity_neighborhood(
    entity_id: String,
    limit: Option<usize>,
    index: State<'_, CorpusIndex>,
) -> Result<Option<CorpusEntityNeighborhood>, AppError> {
    index
        .entity_neighborhood(entity_id, limit.unwrap_or(24))
        .await
}

/// Shortest path between two entities, up to `max_hops` edges.
///
/// `None` when the ids name entities that exist but the graph does not connect them within the
/// budget — a real answer distinct from an error, and distinct from an id the index does not
/// hold. A zero-hop `Found` when `from_entity_id == to_entity_id`, so a reader distinguishes
/// "same node" from "unreachable".
#[tauri::command]
pub async fn find_entity_paths(
    from_entity_id: String,
    to_entity_id: String,
    max_hops: Option<usize>,
    index: State<'_, CorpusIndex>,
) -> Result<EntityPathOutcome, AppError> {
    index
        .find_entity_paths(from_entity_id, to_entity_id, max_hops.unwrap_or(4))
        .await
}

/// Entities information flows through, ranked by an approximation of betweenness centrality.
///
/// Every row states `is_approximation = true` — the score is weighted degree, not computed
/// betweenness. A reader must not present this as a defended centrality measure.
#[tauri::command]
pub async fn list_entity_brokers(
    subject_scope: Option<String>,
    limit: Option<usize>,
    index: State<'_, CorpusIndex>,
) -> Result<Vec<EntityBroker>, AppError> {
    index
        .list_entity_brokers(optional_portal_key(subject_scope)?, limit.unwrap_or(12))
        .await
}
