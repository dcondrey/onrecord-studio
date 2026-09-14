//! Tauri IPC boundaries for discovering, starting, pausing, resuming, withdrawing, and
//! observing ingestion.

use crate::commands::validation::{portal_key as validate_portal_key, required_text};
use crate::error::AppError;
use crate::ingestion::client::{ApiClient, HttpClientConfig};
use crate::ingestion::limiter::RateLimiter;
use crate::ingestion::queue::{ingestion_telemetry_envelope, IngestionProgress, QueueManager};
use crate::storage::corpus::CorpusIndex;
use crate::storage::sqlite::{DuckDbRepository, SourceConnectorRecipe};
use crate::types::{
    CrawlPolicy, DocumentPurge, MetadataPageWindow, PastedContentInput, PortalRuntimeSnapshot,
    RateLimitPolicy, SavedSource, SourceDiscoveryPreview, SourceGuidance, SourceIngestionStart,
    SourceKind, SourceLifecycleState, SourcePurge, SourcePurgePreview, SourceRemoval,
};
use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Runtime, State};
use tokio::sync::mpsc;

const STARTUP_RECOVERY_CAPACITY_ATTEMPTS: usize = 30;
const STARTUP_RECOVERY_CAPACITY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

async fn report_startup_recovery_failure(
    repository: &DuckDbRepository,
    progress_sender: &mpsc::Sender<IngestionProgress>,
    source_key: &str,
    stage: &str,
    error: AppError,
) {
    let persistence_error = repository
        .record_source_recovery_failure(source_key, stage, &error)
        .await
        .err();
    let error_msg = match persistence_error {
        Some(persistence_error) => format!(
            "Startup recovery failed during {stage}: {error}. The failure could not be persisted: {persistence_error}"
        ),
        None => format!("Startup recovery failed during {stage}: {error}"),
    };
    let _ = progress_sender
        .send(IngestionProgress::TaskFailed {
            portal_key: source_key.to_string(),
            target_id: format!("startup-recovery:{stage}"),
            error_msg,
        })
        .await;
}

#[tauri::command]
pub async fn preview_data_source(
    url: String,
    guidance: Option<SourceGuidance>,
    manager: State<'_, Arc<QueueManager>>,
) -> Result<SourceDiscoveryPreview, AppError> {
    let url = required_text(&url, "Data source URL", 4_096)?;
    manager.preview_public_source(url, guidance).await
}

/// `metadata_page_window` selects which slice of a portal's metadata API the synchronization
/// starts from. It reaches only the NextRequest path below; a source that falls through to the
/// public-crawl path has no metadata API, so the window is dropped rather than stored against a
/// source that could never use it.
#[tauri::command]
pub async fn start_data_source(
    url: String,
    crawl_policy: Option<CrawlPolicy>,
    guidance: Option<SourceGuidance>,
    rate_limit_policy: Option<RateLimitPolicy>,
    metadata_page_window: Option<MetadataPageWindow>,
    manager: State<'_, Arc<QueueManager>>,
    repository: State<'_, DuckDbRepository>,
) -> Result<SourceIngestionStart, AppError> {
    let url = required_text(&url, "Data source URL", 4_096)?;
    let rate_limit_policy = rate_limit_policy.unwrap_or_default();
    let metadata_page_window = metadata_page_window.unwrap_or_default();
    match manager
        .discover_and_start(url, metadata_page_window, rate_limit_policy)
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
            // Derived from the recipe's contents, not from the row existing: a row may exist
            // only to hold a metadata page window, and claiming saved guidance was applied when
            // none was is a false statement about how the source was ingested.
            let used_saved_guidance = saved_recipe
                .as_ref()
                .is_some_and(SourceConnectorRecipe::carries_guidance);
            let (crawl_policy, guidance) = match saved_recipe {
                Some(recipe) if recipe.carries_guidance() => (recipe.crawl_policy, recipe.guidance),
                _ => (
                    crawl_policy.unwrap_or_default(),
                    guidance.unwrap_or_default(),
                ),
            };
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
    repository: State<'_, DuckDbRepository>,
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
        .load_source_connector_recipe_for_source(&saved.registration)
        .await?;
    let used_saved_guidance = saved_recipe
        .as_ref()
        .is_some_and(SourceConnectorRecipe::carries_guidance);
    let (crawl_policy, guidance, metadata_page_window) = match saved_recipe {
        Some(recipe) => (
            recipe.crawl_policy,
            recipe.guidance,
            recipe.metadata_page_window,
        ),
        None => (CrawlPolicy::default(), SourceGuidance::default(), None),
    };
    let detected_kind = saved.registration.source_kind;
    let sync_run_id = manager
        .resume_registered_source(
            saved.registration.clone(),
            crawl_policy,
            guidance,
            metadata_page_window,
        )
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

/// Counts what [`purge_source`] would destroy, so the confirmation can name the numbers.
///
/// A destructive confirmation that says only "this cannot be undone" asks the operator to approve
/// an unknown quantity. This command exists so the dialog can say how many documents, requests,
/// and evidence files are at stake before the operator commits to losing them.
#[tauri::command]
pub async fn preview_source_purge(
    source_key: String,
    manager: State<'_, Arc<QueueManager>>,
) -> Result<SourcePurgePreview, AppError> {
    let source_key = validate_portal_key(&source_key)?;
    manager.preview_source_purge(source_key).await
}

/// Destroys a source and every document ingested under it, then removes the derived corpus rows
/// and unlinks the evidence files.
///
/// Distinct from `remove_source`, which withdraws a source and keeps its documents. This command
/// exists so that "I want to start over" is expressible; the front end must confirm it separately
/// and must not present the two as one control with a checkbox.
///
/// The corpus index is purged after the core commits, not before. The corpus is a rebuildable
/// projection, so a corpus row surviving a failed core delete is a stale projection that the next
/// reindex corrects, whereas a core row surviving a successful corpus delete would be evidence the
/// index can no longer describe.
#[tauri::command]
pub async fn purge_source(
    source_key: String,
    manager: State<'_, Arc<QueueManager>>,
    index: State<'_, CorpusIndex>,
) -> Result<SourcePurge, AppError> {
    let _maintenance = super::maintenance::MAINTENANCE_LOCK.lock().await;
    let source_key = validate_portal_key(&source_key)?;
    let purge = manager.purge_source(source_key).await?;
    index.purge_portal(source_key.to_string()).await?;
    remove_evidence_files(&purge.local_paths).await;
    Ok(purge)
}

/// Destroys one ingested document and everything derived from it. See [`purge_source`].
#[tauri::command]
pub async fn purge_document(
    source_key: String,
    document_id: u64,
    manager: State<'_, Arc<QueueManager>>,
    index: State<'_, CorpusIndex>,
) -> Result<DocumentPurge, AppError> {
    let _maintenance = super::maintenance::MAINTENANCE_LOCK.lock().await;
    let source_key = validate_portal_key(&source_key)?;
    let purge = manager.purge_document(source_key, document_id).await?;
    index
        .purge_document(source_key.to_string(), document_id)
        .await?;
    if let Some(path) = purge.local_path.as_ref() {
        remove_evidence_files(std::slice::from_ref(path)).await;
    }
    remove_evidence_files(&purge.additional_local_paths).await;
    Ok(purge)
}

/// Unlinks purged evidence files, reporting nothing.
///
/// A file that is already gone, or that cannot be unlinked because the volume is read-only, must
/// not fail a purge that has already committed: the rows describing the file are gone, so the file
/// is an orphan blob rather than evidence, and reporting an error here would tell the operator the
/// purge did not happen when it did.
async fn remove_evidence_files(paths: &[String]) {
    for path in paths {
        let _ = tokio::fs::remove_file(path).await;
    }
}

#[tauri::command]
pub async fn list_portal_runtimes(
    manager: State<'_, Arc<QueueManager>>,
) -> Result<Vec<PortalRuntimeSnapshot>, AppError> {
    manager.runtime_snapshots().await
}

#[tauri::command]
pub async fn list_saved_portals(
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<SavedSource>, AppError> {
    repository.list_portal_registrations().await
}

pub async fn initialize_queue_runtime<R: Runtime, P: AsRef<Path>>(
    app_handle: AppHandle<R>,
    repository: DuckDbRepository,
    corpus_index: CorpusIndex,
    download_root: P,
) -> Result<Arc<QueueManager>, AppError> {
    let client = ApiClient::new(HttpClientConfig::default())?;
    let limiter = RateLimiter::new(5)?;
    let (progress_sender, mut progress_receiver) = mpsc::channel::<IngestionProgress>(2_048);
    let recovery_progress_sender = progress_sender.clone();
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
    // A failure to enumerate durable work is a startup failure, not an empty queue. Waiting here
    // makes setup surface the error instead of presenting an apparently healthy workspace.
    let interrupted_keys = recovery_repository
        .recover_interrupted_source_keys()
        .await?;
    let sources = if interrupted_keys.is_empty() {
        Vec::new()
    } else {
        recovery_repository.list_portal_registrations().await?
    };
    let recovery_manager = Arc::clone(&manager);
    tokio::spawn(async move {
        for source_key in interrupted_keys {
            let Some(source) = sources
                .iter()
                .find(|saved| saved.registration.portal_key == source_key)
                .map(|saved| saved.registration.clone())
            else {
                report_startup_recovery_failure(
                    &recovery_repository,
                    &recovery_progress_sender,
                    &source_key,
                    "registration lookup",
                    AppError::DataParsing(format!(
                        "Durable work refers to source '{source_key}', but no resumable registration exists"
                    )),
                )
                .await;
                continue;
            };
            let saved_recipe = match recovery_repository
                .load_source_connector_recipe_for_source(&source)
                .await
            {
                Ok(recipe) => recipe,
                Err(error) => {
                    report_startup_recovery_failure(
                        &recovery_repository,
                        &recovery_progress_sender,
                        &source_key,
                        "connector recipe loading",
                        error,
                    )
                    .await;
                    continue;
                }
            };
            let (crawl_policy, guidance, metadata_page_window) = match saved_recipe {
                Some(recipe) => (
                    recipe.crawl_policy,
                    recipe.guidance,
                    recipe.metadata_page_window,
                ),
                None => (CrawlPolicy::default(), SourceGuidance::default(), None),
            };
            let mut capacity_attempts = 0_usize;
            loop {
                match recovery_manager
                    .resume_registered_source(
                        source.clone(),
                        crawl_policy.clone(),
                        guidance.clone(),
                        metadata_page_window,
                    )
                    .await
                {
                    Ok(_) => break,
                    Err(AppError::PortalCapacity(_)) => {
                        if recovery_manager.is_source_running(&source.portal_key).await {
                            break;
                        }
                        capacity_attempts = capacity_attempts.saturating_add(1);
                        if capacity_attempts >= STARTUP_RECOVERY_CAPACITY_ATTEMPTS {
                            report_startup_recovery_failure(
                                &recovery_repository,
                                &recovery_progress_sender,
                                &source_key,
                                "queue capacity waiting",
                                AppError::PortalCapacity(format!(
                                    "No queue slot became available after {} seconds",
                                    STARTUP_RECOVERY_CAPACITY_DELAY.as_secs()
                                        * STARTUP_RECOVERY_CAPACITY_ATTEMPTS as u64
                                )),
                            )
                            .await;
                            break;
                        }
                        tokio::time::sleep(STARTUP_RECOVERY_CAPACITY_DELAY).await;
                    }
                    Err(error) => {
                        report_startup_recovery_failure(
                            &recovery_repository,
                            &recovery_progress_sender,
                            &source_key,
                            "source resume",
                            error,
                        )
                        .await;
                        break;
                    }
                }
            }
        }
    });
    Ok(manager)
}
