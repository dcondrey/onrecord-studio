//! On-demand redaction-inference IPC command.
//!
//! The autonomous detector (`analysis::detectors::inferred_redaction_identity`) sweeps the corpus
//! with the lexical path only. This command exposes the full inference engine, including the
//! semantic path via `VectorIndex`, so the interactive redaction panel can offer richer results
//! for a specific span the user is looking at.

use crate::{
    analysis::redaction_inference::{infer_redaction_candidates, InferenceReport, RedactionSpan},
    error::AppError,
    storage::sqlite::DuckDbRepository,
    storage::vector::VectorIndex,
};
use tauri::State;

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
    let span = RedactionSpan {
        start: span_start,
        end: span_end,
    };
    infer_redaction_candidates(
        &repository,
        Some(&vector_index),
        document_id,
        &portal_key,
        span,
    )
    .await
}
