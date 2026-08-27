//! Tauri IPC boundaries for discovering, starting, pausing, resuming, withdrawing, and
//! observing ingestion.

use crate::commands::validation::{portal_key as validate_portal_key, required_text};
use crate::error::AppError;
use crate::ingestion::client::{ApiClient, HttpClientConfig};
use crate::ingestion::limiter::RateLimiter;
use crate::ingestion::queue::{ingestion_telemetry_envelope, IngestionProgress, QueueManager};
use crate::storage::corpus::CorpusIndex;
use crate::storage::sqlite::SqliteRepository;
use crate::types::{
    CrawlPolicy, PastedContentInput, PortalRuntimeSnapshot, PortalSyncStart, RateLimitPolicy,
    SavedSource, SourceDiscoveryPreview, SourceGuidance, SourceIngestionStart, SourceKind,
    SourceLifecycleState, SourceRemoval,
};
use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Runtime, State};
use tokio::sync::mpsc;

#[tauri::command]
pub async fn start_portal_sync(
    portal_url: String,
    start_page: Option<usize>,
    per_page: Option<usize>,
    rate_limit_policy: Option<RateLimitPolicy>,
    manager: State<'_, Arc<QueueManager>>,
) -> Result<PortalSyncStart, AppError> {
    let portal_url = required_text(&portal_url, "Portal URL", 2_048)?;
    manager
        .discover_and_start(
            portal_url,
            start_page.unwrap_or(1),
            per_page.unwrap_or(50),
            rate_limit_policy.unwrap_or_default(),
        )
        .await
}

#[tauri::command]
pub async fn preview_data_source(
    url: String,
    manager: State<'_, Arc<QueueManager>>,
) -> Result<SourceDiscoveryPreview, AppError> {
    let url = required_text(&url, "Data source URL", 4_096)?;
    manager.preview_public_source(url).await
}

#[tauri::command]
pub async fn start_data_source(
    url: String,
    crawl_policy: Option<CrawlPolicy>,
    guidance: Option<SourceGuidance>,
    rate_limit_policy: Option<RateLimitPolicy>,
    manager: State<'_, Arc<QueueManager>>,
    repository: State<'_, SqliteRepository>,
) -> Result<SourceIngestionStart, AppError> {
    let url = required_text(&url, "Data source URL", 4_096)?;
    let rate_limit_policy = rate_limit_policy.unwrap_or_default();
    match manager
        .discover_and_start(url, 1, 50, rate_limit_policy)
        .await
    {
        Ok(start) => Ok(SourceIngestionStart {
            detected_kind: SourceKind::NextRequest,
            source: start.portal,
            sync_run_id: start.sync_run_id,
            used_saved_guidance: false,
        }),
        Err(AppError::PortalDiscovery(_)) => {
            let explicit_recipe = crawl_policy.is_some() || guidance.is_some();
            let saved_recipe = if explicit_recipe {
                None
            } else {
                repository.load_source_connector_recipe(url).await?
            };
            let used_saved_guidance = saved_recipe.is_some();
            let (crawl_policy, guidance) = saved_recipe.unwrap_or_else(|| {
                (
                    crawl_policy.unwrap_or_default(),
                    guidance.unwrap_or_default(),
                )
            });
            let mut start = manager
                .discover_and_start_public_source(
                    url,
                    crawl_policy.clone(),
                    guidance.clone(),
                    rate_limit_policy,
                )
                .await?;
            start.used_saved_guidance = used_saved_guidance;
            Ok(start)
        }
        Err(error) => Err(error),
    }
}

#[tauri::command]
pub async fn ingest_local_files(
    paths: Vec<String>,
    manager: State<'_, Arc<QueueManager>>,
) -> Result<SourceIngestionStart, AppError> {
    manager.start_local_files(paths).await
}

#[tauri::command]
pub async fn ingest_pasted_content(
    input: PastedContentInput,
    manager: State<'_, Arc<QueueManager>>,
) -> Result<SourceIngestionStart, AppError> {
    manager.start_pasted_content(input).await
}

/// Halts a source and records the halt so the next launch does not resume it.
///
/// Returns whether a pool was actually running. A saved source that was idle is still marked
/// paused, because the marker is what startup recovery reads.
#[tauri::command]
pub async fn pause_source_sync(
    source_key: String,
    manager: State<'_, Arc<QueueManager>>,
) -> Result<bool, AppError> {
    let source_key = validate_portal_key(&source_key)?;
    manager.pause_source(source_key).await
}

/// Restarts a paused source along the same path startup recovery uses, so a resume inherits the
/// frontier its pause left behind instead of crawling the source again from the beginning.
#[tauri::command]
pub async fn resume_source_sync(
    source_key: String,
    repository: State<'_, SqliteRepository>,
    manager: State<'_, Arc<QueueManager>>,
) -> Result<SourceIngestionStart, AppError> {
    let source_key = validate_portal_key(&source_key)?;
    let saved = repository
        .list_portal_registrations()
        .await?
        .into_iter()
        .find(|saved| saved.registration.portal_key == source_key)
        .ok_or_else(|| {
            AppError::DataParsing(format!("No saved source is registered as '{source_key}'"))
        })?;
    // Marked active before the pool is provisioned so a failure part-way leaves the source
    // resumable rather than silently paused-but-running.
    repository
        .set_source_lifecycle_state(source_key, SourceLifecycleState::Active)
        .await?;
    let saved_recipe = repository
        .load_source_connector_recipe(&saved.registration.canonical_url)
        .await?;
    let used_saved_guidance = saved_recipe.is_some();
    let (crawl_policy, guidance) = saved_recipe.unwrap_or_default();
    let detected_kind = saved.registration.source_kind;
    let sync_run_id = manager
        .resume_registered_source(saved.registration.clone(), crawl_policy, guidance)
        .await?;
    Ok(SourceIngestionStart {
        source: saved.registration,
        sync_run_id,
        detected_kind,
        used_saved_guidance,
    })
}

/// Withdraws a source. Documents already ingested from it are retained and the count is returned
/// so the caller states what was kept rather than implying everything went.
#[tauri::command]
pub async fn remove_source(
    source_key: String,
    manager: State<'_, Arc<QueueManager>>,
) -> Result<SourceRemoval, AppError> {
    let source_key = validate_portal_key(&source_key)?;
    manager.remove_source(source_key).await
}

#[tauri::command]
pub async fn list_portal_runtimes(
    manager: State<'_, Arc<QueueManager>>,
) -> Result<Vec<PortalRuntimeSnapshot>, AppError> {
    manager.runtime_snapshots().await
}

#[tauri::command]
pub async fn list_saved_portals(
    repository: State<'_, SqliteRepository>,
) -> Result<Vec<SavedSource>, AppError> {
    repository.list_portal_registrations().await
}

pub async fn initialize_queue_runtime<R: Runtime, P: AsRef<Path>>(
    app_handle: AppHandle<R>,
    repository: SqliteRepository,
    corpus_index: CorpusIndex,
    download_root: P,
) -> Result<Arc<QueueManager>, AppError> {
    let client = ApiClient::new(HttpClientConfig::default())?;
    let limiter = RateLimiter::new(5)?;
    let (progress_sender, mut progress_receiver) = mpsc::channel::<IngestionProgress>(2_048);
    let recovery_repository = repository.clone();
    let manager = Arc::new(QueueManager::new(
        client,
        repository,
        corpus_index,
        limiter,
        progress_sender,
        download_root,
    )?);
    tokio::spawn(async move {
        while let Some(progress) = progress_receiver.recv().await {
            let _ = app_handle.emit(
                "ingestion-telemetry",
                ingestion_telemetry_envelope(progress),
            );
        }
    });
    let recovery_manager = Arc::clone(&manager);
    // TODO(recovery): Persist and emit startup-recovery failures instead of returning silently;
    // operators otherwise cannot distinguish a clean startup from an abandoned durable frontier.
    tokio::spawn(async move {
        let Ok(interrupted_keys) = recovery_repository.recover_interrupted_source_keys().await
        else {
            return;
        };
        if interrupted_keys.is_empty() {
            return;
        }
        let Ok(sources) = recovery_repository.list_portal_registrations().await else {
            return;
        };
        for source_key in interrupted_keys {
            let Some(source) = sources
                .iter()
                .find(|saved| saved.registration.portal_key == source_key)
                .map(|saved| saved.registration.clone())
            else {
                continue;
            };
            let saved_recipe = recovery_repository
                .load_source_connector_recipe(&source.canonical_url)
                .await
                .ok()
                .flatten();
            let (crawl_policy, guidance) = saved_recipe.unwrap_or_default();
            loop {
                match recovery_manager
                    .resume_registered_source(
                        source.clone(),
                        crawl_policy.clone(),
                        guidance.clone(),
                    )
                    .await
                {
                    Ok(_) => break,
                    Err(AppError::PortalCapacity(_)) => {
                        if recovery_manager.is_source_running(&source.portal_key).await {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Err(_) => break,
                }
            }
        }
    });
    Ok(manager)
}
