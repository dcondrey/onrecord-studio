//! Standing searches: saved query ASTs re-run per corpus state, and the series-number absence
//! search over extracted identifiers.

use tauri::State;

use crate::commands::query::refuse_while_indexing;
use crate::error::AppError;
use crate::storage::corpus::CorpusIndex;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{
    SeriesGapReport, StandingQuery, StandingQueryInput, StandingQueryOutcome, StandingQuerySummary,
};

#[tauri::command]
pub async fn save_standing_query(
    input: StandingQueryInput,
    repository: State<'_, DuckDbRepository>,
) -> Result<StandingQuery, AppError> {
    repository.save_standing_query(input).await
}

#[tauri::command]
pub async fn list_standing_queries(
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<StandingQuerySummary>, AppError> {
    repository.list_standing_queries().await
}

#[tauri::command]
pub async fn delete_standing_query(
    query_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<(), AppError> {
    repository.delete_standing_query(query_id).await
}

/// Re-runs the queries whose last run saw a different corpus state (all of them with `force`).
/// A query that reads document text is deferred while the search index is incomplete, for the
/// reason `refuse_while_indexing` gives: it would report fewer matches than the corpus holds.
#[tauri::command]
pub async fn run_standing_queries(
    query_id: Option<String>,
    force: Option<bool>,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<StandingQueryOutcome>, AppError> {
    let index_pending = refuse_while_indexing(&repository, "pending")
        .err()
        .map(|error| error.to_string());
    repository
        .run_standing_queries(query_id, force.unwrap_or(false), index_pending)
        .await
}

/// Numbers missing from labelled document series the corpus carries. Absence from the corpus,
/// never from the agency's files; see `analysis::series_gaps`.
#[tauri::command]
pub async fn list_series_number_gaps(
    limit: Option<usize>,
    index: State<'_, CorpusIndex>,
) -> Result<SeriesGapReport, AppError> {
    index
        .series_number_gaps(limit.unwrap_or(50).clamp(1, 500))
        .await
}
