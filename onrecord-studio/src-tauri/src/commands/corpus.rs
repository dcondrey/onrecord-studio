//! Explainable, provenance-linked cross-document pattern queries.

use crate::commands::validation::optional_portal_key;
use crate::error::AppError;
use crate::storage::corpus::CorpusIndex;
use crate::types::{
    CorpusEntityNeighborhood, CorpusEntityNode, CorpusIndexStatus, CorpusPattern,
    CorpusPatternQuery,
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
