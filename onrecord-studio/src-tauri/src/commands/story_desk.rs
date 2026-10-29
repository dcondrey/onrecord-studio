use crate::analysis::story_desk::rank_leads;
use crate::error::AppError;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::StoryLead;
use tauri::State;

/// Enough undismissed findings to rank without reading the whole table on a large corpus.
const MAXIMUM_RANKED_FINDINGS: usize = 2_000;

#[tauri::command]
pub async fn list_story_leads(
    limit: Option<usize>,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<StoryLead>, AppError> {
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let findings = repository
        .list_story_candidates(MAXIMUM_RANKED_FINDINGS)
        .await?;
    Ok(rank_leads(&findings, chrono::Utc::now(), limit))
}
