use crate::{
    error::AppError,
    ingestion::queue::QueueManager,
    research::{investigate_with_collection, RESEARCH_LOCK},
    storage::sqlite::DuckDbRepository,
    types::{ResearchPolicy, ResearchStatus},
};
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn get_research_status(
    repository: State<'_, DuckDbRepository>,
) -> Result<ResearchStatus, AppError> {
    repository.research_status().await
}

#[tauri::command]
pub async fn set_research_policy(
    policy: ResearchPolicy,
    repository: State<'_, DuckDbRepository>,
) -> Result<ResearchStatus, AppError> {
    repository.save_research_policy(policy).await?;
    repository.research_status().await
}

#[tauri::command]
pub async fn run_research_now(
    topic: String,
    repository: State<'_, DuckDbRepository>,
    manager: State<'_, Arc<QueueManager>>,
) -> Result<(), AppError> {
    if !["homelessness", "abatement", "bathrooms", "police", "waste"].contains(&topic.as_str()) {
        return Err(AppError::DataParsing("Unknown research topic".into()));
    }
    let guard = RESEARCH_LOCK.try_lock().map_err(|_| {
        AppError::DataParsing("A research investigation is already running.".into())
    })?;
    let repository = repository.inner().clone();
    let manager = manager.inner().clone();
    repository
        .research_phase("Starting investigation", None)
        .await?;
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        if let Err(e) = investigate_with_collection(&repository, Some(&manager), &topic, true).await
        {
            let _ = repository
                .research_phase("Research needs attention", Some(e.to_string()))
                .await;
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn set_research_action_state(
    action_key: String,
    state: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<(), AppError> {
    repository.research_action(action_key, state).await
}
