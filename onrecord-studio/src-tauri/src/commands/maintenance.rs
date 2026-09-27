use super::investigation::AppInvestigationPaths;
use crate::{
    error::AppError,
    ingestion::queue::QueueManager,
    storage::sqlite::DuckDbRepository,
    types::{DataMaintenancePolicy, DataMaintenanceStatus, SourceKind, SourceLifecycleState},
};
use std::sync::Arc;
use tauri::State;

pub(crate) static MAINTENANCE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tauri::command]
pub async fn get_data_maintenance_status(
    repository: State<'_, DuckDbRepository>,
    paths: State<'_, AppInvestigationPaths>,
) -> Result<DataMaintenanceStatus, AppError> {
    repository
        .maintenance_status_with_storage(&paths.document_root)
        .await
}

#[tauri::command]
pub async fn set_data_maintenance_policy(
    policy: DataMaintenancePolicy,
    repository: State<'_, DuckDbRepository>,
    paths: State<'_, AppInvestigationPaths>,
) -> Result<DataMaintenanceStatus, AppError> {
    let _guard = MAINTENANCE_LOCK.lock().await;
    repository.save_maintenance_policy(policy).await?;
    repository
        .maintenance_status_with_storage(&paths.document_root)
        .await
}

#[tauri::command]
pub async fn run_data_maintenance(
    repository: State<'_, DuckDbRepository>,
    manager: State<'_, Arc<QueueManager>>,
    paths: State<'_, AppInvestigationPaths>,
) -> Result<DataMaintenanceStatus, AppError> {
    maintain(&repository, &manager, &paths).await
}

pub(crate) async fn maintain(
    repository: &DuckDbRepository,
    manager: &Arc<QueueManager>,
    paths: &AppInvestigationPaths,
) -> Result<DataMaintenanceStatus, AppError> {
    let Ok(_guard) = MAINTENANCE_LOCK.try_lock() else {
        return repository
            .maintenance_status_with_storage(&paths.document_root)
            .await;
    };
    let status = repository
        .maintenance_status_with_storage(&paths.document_root)
        .await?;
    let runtimes = manager.runtime_snapshots().await?;
    let any_running = runtimes.iter().any(|runtime| runtime.is_running);
    if !any_running && manager.maintain_idle_storage(None).await? {
        return repository
            .maintenance_status_with_storage(&paths.document_root)
            .await;
    }
    if !status.policy.enabled {
        return Ok(status);
    }
    if status.policy.auto_import {
        for _ in 0..4 {
            if !repository.maintain_one_table().await? {
                break;
            }
        }
    }
    // Retiring files while an extraction or forensic reader is using them would break that
    // reader. Work on references only when acquisition and evidence auditing are idle.
    if !any_running && status.policy.retention_mode == "REFERENCE_ONLY" {
        let _redaction = paths
            .redaction_audit_permits
            .acquire()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let _versions = paths
            .version_recovery_permits
            .acquire()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let _forensic = paths
            .forensic_audit_permits
            .acquire()
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let _provenance = paths
            .provenance_verification_permits
            .acquire_many(4)
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?;
        for _ in 0..32 {
            if !manager
                .maintain_idle_storage(Some(&status.effective_at))
                .await?
            {
                break;
            }
        }
    }
    if status.policy.auto_refresh
        && !any_running
        && status.retained_file_bytes < status.policy.retained_file_budget_bytes
    {
        for source in repository.list_portal_registrations().await? {
            if source.lifecycle_state != SourceLifecycleState::Active
                || matches!(
                    source.registration.source_kind,
                    SourceKind::LocalFile | SourceKind::PastedContent
                )
            {
                continue;
            }
            if !repository
                .source_refresh_due(&source.registration.portal_key, status.policy.refresh_hours)
                .await?
            {
                continue;
            }
            let recipe = repository
                .load_source_connector_recipe_for_source(&source.registration)
                .await?;
            let (mut policy, guidance, window) = recipe
                .map(|r| (r.crawl_policy, r.guidance, r.metadata_page_window))
                .unwrap_or_default();
            // Rechecks are bounded; they cannot turn a saved entry point into an unbounded crawl.
            policy.maximum_pages = policy.maximum_pages.min(100);
            policy.maximum_resources = policy.maximum_resources.min(500);
            let key = format!("refresh:{}", source.registration.portal_key);
            repository
                .record_maintenance(
                    key.clone(),
                    "REFRESH_STARTED",
                    format!(
                        "Rechecking {} with its saved scope and host rules.",
                        source.registration.display_name
                    ),
                    status.policy.refresh_hours,
                )
                .await?;
            if let Err(error) = manager
                .refresh_registered_source(source.registration, policy, guidance, window)
                .await
            {
                repository
                    .record_maintenance(key, "REFRESH_RETRY", error.to_string(), 1)
                    .await?;
            }
            break;
        }
    }
    repository
        .maintenance_status_with_storage(&paths.document_root)
        .await
}
