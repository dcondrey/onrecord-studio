//! Explainable, provenance-linked cross-document pattern queries.

use crate::commands::validation::optional_portal_key;
use crate::error::AppError;
use crate::storage::corpus::CorpusIndex;
use crate::types::{CorpusIndexStatus, CorpusPattern, CorpusPatternQuery};
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
