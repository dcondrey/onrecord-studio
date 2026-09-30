//! IPC command for the deterministic investigative intent router.

use crate::{
    error::AppError,
    storage::{
        corpus::CorpusIndex,
        cross_agency::{self, DEFAULT_ROW_LIMIT},
        sqlite::DuckDbRepository,
    },
    types::intent::IntentAnswer,
};
use tauri::State;

/// Answers an investigative question from stored findings, the corpus graph, the text index and
/// the request timeline. Deterministic: no model is consulted, and an utterance that fits no
/// supported intent is reported as not understood together with what can be asked.
#[tauri::command]
pub async fn run_intent_query(
    utterance: String,
    limit: Option<usize>,
    repository: State<'_, DuckDbRepository>,
    index: State<'_, CorpusIndex>,
) -> Result<IntentAnswer, AppError> {
    cross_agency::answer_utterance(
        &repository,
        Some(index.inner()),
        &utterance,
        limit.unwrap_or(DEFAULT_ROW_LIMIT),
    )
    .await
}
