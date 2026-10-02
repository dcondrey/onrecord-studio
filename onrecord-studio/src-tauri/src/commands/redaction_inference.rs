//! On-demand redaction-inference IPC command.
//!
//! The autonomous detector (`analysis::detectors::inferred_redaction_identity`) sweeps the corpus
//! with the lexical path only. This command exposes the full inference engine, including the
//! semantic path via `VectorIndex`, so the interactive redaction panel can offer richer results
//! for a specific span the user is looking at.

use crate::{
    analysis::redaction_inference::{
        character_span_to_bytes, infer_redaction_candidates, InferenceReport,
    },
    error::AppError,
    storage::sqlite::DuckDbRepository,
    storage::vector::VectorIndex,
};
use tauri::State;

/// `span_start`/`span_end` are document character offsets (Unicode scalar values), the unit the
/// viewer reports; the engine indexes the stored text by byte, so they are converted here.
#[tauri::command]
pub async fn infer_redaction(
    document_id: i64,
    portal_key: String,
    span_start: usize,
    span_end: usize,
    repository: State<'_, DuckDbRepository>,
    vector_index: State<'_, VectorIndex>,
) -> Result<InferenceReport, AppError> {
    if span_end <= span_start {
        return Err(AppError::DataParsing(
            "Redaction span must have span_end > span_start".to_string(),
        ));
    }
    let stored_id = u64::try_from(document_id)
        .map_err(|_| AppError::DataParsing("Document identifier was negative".to_string()))?;
    let text = repository
        .load_corpus_source_text(&portal_key, stored_id)
        .await?;
    let span = character_span_to_bytes(&text, span_start, span_end).ok_or_else(|| {
        AppError::DataParsing("Redaction span lies outside the document text".to_string())
    })?;
    infer_redaction_candidates(
        &repository,
        Some(&vector_index),
        document_id,
        &portal_key,
        span,
    )
    .await
}
