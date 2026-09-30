use crate::commands::validation::{optional_portal_key, optional_text, validate_text};
use crate::error::AppError;
use crate::ingestion::regional::source_jurisdiction;
use crate::storage::sqlite::DuckDbRepository;
use crate::storage::vector::VectorIndex;
use crate::types::{
    EmbeddingIndexReport, EmbeddingIndexStatus, EmbeddingModelStatus, HybridSearchFilter,
    HybridSearchResult,
};
use tauri::State;

#[tauri::command]
pub async fn get_embedding_model_status(
    index: State<'_, VectorIndex>,
) -> Result<EmbeddingModelStatus, AppError> {
    index.model_status().await
}

#[tauri::command]
pub async fn initialize_embedding_model(
    index: State<'_, VectorIndex>,
) -> Result<EmbeddingModelStatus, AppError> {
    index.initialize_model().await
}

#[tauri::command]
pub async fn get_embedding_index_status(
    repository: State<'_, DuckDbRepository>,
    index: State<'_, VectorIndex>,
) -> Result<EmbeddingIndexStatus, AppError> {
    let total = repository.count_vector_source_documents().await?;
    index.index_status(total).await
}

#[tauri::command]
pub async fn rebuild_embedding_index(
    rebuild: Option<bool>,
    repository: State<'_, DuckDbRepository>,
    index: State<'_, VectorIndex>,
) -> Result<EmbeddingIndexReport, AppError> {
    let documents = repository.list_vector_source_documents().await?;
    index
        .index_documents(documents, rebuild.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn cancel_embedding_index(index: State<'_, VectorIndex>) -> Result<bool, AppError> {
    index.cancel_indexing();
    Ok(true)
}

#[tauri::command]
pub async fn hybrid_search(
    query: String,
    filter: Option<HybridSearchFilter>,
    limit: Option<usize>,
    repository: State<'_, DuckDbRepository>,
    index: State<'_, VectorIndex>,
) -> Result<Vec<HybridSearchResult>, AppError> {
    validate_text(&query, "Hybrid search query", 4_096)?;
    let mut filter = filter.unwrap_or(HybridSearchFilter {
        portal_key: None,
        agency_name: None,
        jurisdiction: None,
        document_type: None,
        date_from: None,
        date_to: None,
    });
    filter.portal_key = optional_portal_key(filter.portal_key)?;
    filter.agency_name = optional_text(filter.agency_name, "Agency filter", 256)?;
    filter.jurisdiction = optional_text(filter.jurisdiction, "Jurisdiction filter", 256)?;
    filter.document_type = optional_text(filter.document_type, "Document type filter", 256)?;
    filter.date_from = optional_text(filter.date_from, "Date-from filter", 64)?;
    filter.date_to = optional_text(filter.date_to, "Date-to filter", 64)?;
    let limit = limit.unwrap_or(25).clamp(1, 100);
    // Lexical-only retrieval is the documented mode before the embedding model is initialized,
    // so that absence is decided here explicitly. Any other failure from the semantic side is a
    // real error and must surface rather than read as "no semantic matches".
    let semantic = if index.model_ready().await {
        index.semantic_search(&query, filter.clone(), limit).await?
    } else {
        Vec::new()
    };
    let passages = repository
        .search_document_passages(&query, filter.clone(), limit)
        .await?;
    let mut combined = fuse_document_results(semantic, passages);
    let lexical = repository
        .search_requests(
            query.trim(),
            filter.portal_key.as_deref(),
            None,
            None,
            None,
            5_000,
        )
        .await?;
    let sources = repository
        .list_portal_registrations()
        .await?
        .into_iter()
        .map(|source| (source.registration.portal_key.clone(), source.registration))
        .collect::<std::collections::HashMap<_, _>>();
    for (rank, result) in lexical.into_iter().enumerate() {
        let source = sources.get(&result.portal_key);
        let jurisdiction = source
            .map(|source| source_jurisdiction(&source.canonical_url))
            .unwrap_or("Unclassified source");
        let publication_date = source
            .filter(|source| source.source_kind == crate::types::SourceKind::NextRequest)
            .map(|_| result.created_at.clone());
        if !lexical_context_matches(
            &filter,
            &result.agency_name,
            jurisdiction,
            publication_date.as_deref(),
        ) {
            continue;
        }
        let stable_id = format!("lexical:{}:{}", result.portal_key, result.request_id);
        if combined.iter().any(|existing| {
            existing.portal_key == result.portal_key && existing.request_id == result.request_id
        }) {
            continue;
        }
        combined.push(HybridSearchResult {
            stable_id,
            portal_key: result.portal_key,
            request_id: result.request_id,
            document_id: 0,
            file_name: "request metadata".to_string(),
            title: result.title,
            agency_name: result.agency_name,
            jurisdiction: jurisdiction.to_string(),
            document_type: Some("REQUEST_METADATA".to_string()),
            publication_date,
            observation_date: None,
            effective_date: None,
            character_start: 0,
            character_end: 0,
            excerpt: result.description_preview,
            lexical_rank: Some(rank + 1),
            semantic_score: None,
            fused_score: 0.5 * reciprocal_rank(rank + 1, 60.0),
            retrieval_status: "LEXICAL_FALLBACK_OR_EXACT_MATCH".to_string(),
        });
    }
    combined.sort_by(|left, right| {
        right
            .fused_score
            .total_cmp(&left.fused_score)
            .then_with(|| left.stable_id.cmp(&right.stable_id))
    });
    let mut visible = Vec::new();
    for mut result in combined {
        if result.document_id != 0 {
            // Recheck current visibility and use the same context-aware masking boundary as
            // the evidence viewer, including values that cross an excerpt edge.
            let Ok(chunk) = repository
                .load_document_text_chunk(
                    result.document_id,
                    &result.portal_key,
                    result.character_start,
                    result
                        .character_end
                        .saturating_sub(result.character_start)
                        .clamp(1, 1600),
                )
                .await
            else {
                continue;
            };
            result.excerpt = chunk.masked_text;
            result.character_end = result.character_start + chunk.returned_character_count;
        } else {
            result.excerpt = crate::storage::pii::mask_pii(&result.excerpt)?.masked_text;
        }
        result.file_name = crate::storage::pii::mask_pii(&result.file_name)?.masked_text;
        result.title = crate::storage::pii::mask_pii(&result.title)?.masked_text;
        result.agency_name = crate::storage::pii::mask_pii(&result.agency_name)?.masked_text;
        visible.push(result);
        if visible.len() == limit {
            break;
        }
    }
    Ok(visible)
}

// Rank fusion compares ranks from both retrieval methods, never cosine similarity with
// reciprocal-rank scores. Keep separate documents in the same request distinct.
fn fuse_document_results(
    semantic: Vec<HybridSearchResult>,
    lexical: Vec<HybridSearchResult>,
) -> Vec<HybridSearchResult> {
    let mut results = Vec::<HybridSearchResult>::new();
    for (rank, mut result) in semantic.into_iter().enumerate() {
        if results.iter().any(|item| {
            item.portal_key == result.portal_key && item.document_id == result.document_id
        }) {
            continue;
        }
        result.fused_score = reciprocal_rank(rank + 1, 60.0);
        results.push(result);
    }
    for (rank, mut result) in lexical.into_iter().enumerate() {
        let score = reciprocal_rank(rank + 1, 60.0);
        if let Some(existing) = results.iter_mut().find(|item| {
            item.portal_key == result.portal_key && item.document_id == result.document_id
        }) {
            // Prefer the exact matching passage for a document found by both methods.
            result.semantic_score = existing.semantic_score;
            result.fused_score = existing.fused_score + score;
            result.retrieval_status = format!("HYBRID_{}", result.retrieval_status);
            *existing = result;
        } else {
            result.fused_score = score;
            results.push(result);
        }
    }
    results
}

fn reciprocal_rank(rank: usize, k: f64) -> f64 {
    1.0 / (k + rank as f64)
}

fn lexical_context_matches(
    filter: &HybridSearchFilter,
    agency: &str,
    jurisdiction: &str,
    date: Option<&str>,
) -> bool {
    if filter
        .jurisdiction
        .as_ref()
        .is_some_and(|value| !value.eq_ignore_ascii_case(jurisdiction))
        || filter
            .agency_name
            .as_ref()
            .is_some_and(|value| !agency.to_lowercase().contains(&value.to_lowercase()))
        || filter
            .document_type
            .as_ref()
            .is_some_and(|value| !value.eq_ignore_ascii_case("REQUEST_METADATA"))
    {
        return false;
    }
    if filter.date_from.is_some() || filter.date_to.is_some() {
        let Some(date) = date.and_then(|value| value.get(..10)) else {
            return false;
        };
        if filter
            .date_from
            .as_deref()
            .is_some_and(|start| date < start)
            || filter.date_to.as_deref().is_some_and(|end| date > end)
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fusion_uses_ranks_and_preserves_distinct_documents_within_a_request() {
        let result = |document_id, score, status: &str| HybridSearchResult {
            stable_id: format!("{status}:{document_id}"),
            portal_key: "city".into(),
            request_id: 1,
            document_id,
            file_name: "budget.pdf".into(),
            title: String::new(),
            agency_name: String::new(),
            jurisdiction: String::new(),
            document_type: None,
            publication_date: None,
            observation_date: None,
            effective_date: None,
            character_start: 0,
            character_end: 10,
            excerpt: status.into(),
            lexical_rank: None,
            semantic_score: score,
            fused_score: score.unwrap_or(0.0),
            retrieval_status: status.into(),
        };
        let fused = fuse_document_results(
            vec![
                result(1, Some(0.99), "semantic"),
                result(1, Some(0.98), "duplicate chunk"),
                result(2, Some(0.8), "semantic"),
            ],
            vec![
                result(2, None, "DOCUMENT_TEXT_MATCH"),
                result(3, None, "DOCUMENT_TEXT_MATCH"),
            ],
        );
        assert_eq!(fused.len(), 3);
        let both = fused.iter().find(|item| item.document_id == 2).unwrap();
        assert!(
            both.fused_score
                > fused
                    .iter()
                    .find(|item| item.document_id == 1)
                    .unwrap()
                    .fused_score
        );
        assert!(
            both.fused_score < 0.1,
            "cosine scores must not be added to rank scores"
        );
        assert_eq!(both.excerpt, "DOCUMENT_TEXT_MATCH");
        assert_eq!(both.semantic_score, Some(0.8));
    }
    #[test]
    fn lexical_fallback_respects_publisher_agency_type_and_known_dates() {
        let mut filter: HybridSearchFilter =
            serde_json::from_value(serde_json::json!({"jurisdiction":"San Diego County"})).unwrap();
        assert!(lexical_context_matches(
            &filter,
            "Health",
            "San Diego County",
            None
        ));
        assert!(!lexical_context_matches(
            &filter,
            "Health",
            "City of San Diego",
            None
        ));
        filter.agency_name = Some("Housing".into());
        assert!(!lexical_context_matches(
            &filter,
            "Health",
            "San Diego County",
            None
        ));
        filter.agency_name = None;
        filter.date_from = Some("2026-01-01".into());
        assert!(!lexical_context_matches(
            &filter,
            "Health",
            "San Diego County",
            None
        ));
        assert!(lexical_context_matches(
            &filter,
            "Health",
            "San Diego County",
            Some("2026-02-01T00:00:00Z")
        ));
        filter.document_type = Some("application/pdf".into());
        assert!(!lexical_context_matches(
            &filter,
            "Health",
            "San Diego County",
            Some("2026-02-01T00:00:00Z")
        ));
    }
}
