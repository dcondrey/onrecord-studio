//! macOS runtime environment hooks, folder provisioning, and window decoration profiles.

use crate::analysis::AnalyticsEngine;
use crate::commands::ingest::initialize_queue_runtime;
use crate::commands::investigation::AppInvestigationPaths;
use crate::error::AppError;
use crate::ingestion::queue::{ingestion_telemetry_envelope, IngestionProgress};
use crate::storage::corpus::{CorpusDocumentInput, CorpusIndex};
use crate::storage::duckdb::DuckDbWorkspace;
use crate::storage::legacy_import::{
    classify_core_database, migrate_legacy_sqlite_core, CoreDatabaseKind,
};
use crate::storage::sqlite::DuckDbRepository;
use std::path::Path;
use tauri::{App, Emitter, Manager, Runtime};
#[cfg(target_os = "windows")]
use window_vibrancy::apply_mica;
#[cfg(target_os = "macos")]
use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};

/// Provisions native assets and sets up local database tables prior to showing the interface.
pub fn configure_onrecord_studio_runtime<R: Runtime>(app: &mut App<R>) -> Result<(), AppError> {
    // 1. Resolve and provision explicit app data tracking slots in the home directory path
    let app_data_dir = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("./.onrecord_studio_cache"));

    create_runtime_directory(&app_data_dir)?;
    let report_directory = app_data_dir.join("reports");
    create_runtime_directory(&report_directory)?;
    let core_db_file = app_data_dir.join("onrecord_studio_core.db");
    let corpus_db_file = app_data_dir.join("onrecord_studio_corpus.db");
    let initial_migration = include_str!("../migrations/0001_initial_schema.sql");

    // 2. Upgrade a pre-DuckDB core before anything tries to open it. DuckDB cannot read a SQLite
    // file, so without this an installation from an earlier release fails at startup with an
    // engine error and its corpus becomes unreachable.
    let migration_report = match classify_core_database(&core_db_file)? {
        CoreDatabaseKind::LegacySqlite => {
            Some(migrate_legacy_sqlite_core(&core_db_file, initial_migration)?)
        }
        CoreDatabaseKind::Unrecognized => {
            return Err(AppError::Storage(format!(
                "The file at '{}' is neither a DuckDB nor a SQLite database. Move it aside before                  starting OnRecord Studio; it will never be overwritten automatically.",
                core_db_file.display()
            )))
        }
        CoreDatabaseKind::DuckDb | CoreDatabaseKind::Missing => None,
    };

    // 3. Initialize the DuckDB core repository and run structural tracking migrations.
    let repository = DuckDbRepository::new(&core_db_file)?;
    let corpus_index = CorpusIndex::open(&corpus_db_file)?;
    tauri::async_runtime::block_on(repository.run_migrations(initial_migration))?;

    // Both analytics nodes read through live sibling connections to the repository's DuckDB
    // instance. Opening or attaching the core file separately would pin each node to the database
    // as it stood at launch, so no document ingested during the session would ever be analyzed.
    let agency_engine = AnalyticsEngine::bind_to_core(tauri::async_runtime::block_on(
        repository.analytics_connection(),
    )?);
    let correlation_engine = DuckDbWorkspace::bind_to_core(tauri::async_runtime::block_on(
        repository.analytics_connection(),
    )?);

    // 4. Boot up the asynchronous queue workspace pipeline
    let app_handle = app.handle().clone();
    let download_root = app_data_dir.join("documents");
    create_runtime_directory(&download_root)?;
    let manager = tauri::async_runtime::block_on(initialize_queue_runtime(
        app_handle.clone(),
        repository.clone(),
        corpus_index.clone(),
        &download_root,
    ))?;
    if let Some(report) = migration_report {
        let _ = app_handle.emit(
            "ingestion-telemetry",
            ingestion_telemetry_envelope(IngestionProgress::LegacyCoreDatabaseMigrated {
                backup_path: report.backup_path.display().to_string(),
                copied_tables: report.copied_tables.len() as u64,
                copied_rows: report.total_rows(),
                tables_absent_from_legacy: report.absent_from_legacy.clone(),
                tables_dropped_from_legacy: report.dropped_from_legacy.clone(),
            }),
        );
    }
    start_corpus_reconciliation(app_handle, repository.clone(), corpus_index.clone());
    app.manage(repository);
    app.manage(corpus_index);
    app.manage(report_directory);
    app.manage(AppInvestigationPaths::new(download_root));
    app.manage(manager);
    app.manage(agency_engine);
    app.manage(correlation_engine);

    // 5. Apply the native window material.
    if let Some(main_window) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        let _ = apply_vibrancy(&main_window, NSVisualEffectMaterial::Sidebar, None, None);

        #[cfg(target_os = "windows")]
        let _ = apply_mica(&main_window, None);
    }

    Ok(())
}

fn start_corpus_reconciliation<R: Runtime>(
    app_handle: tauri::AppHandle<R>,
    repository: DuckDbRepository,
    corpus_index: CorpusIndex,
) {
    // The core repository is canonical and the corpus is a rebuildable projection. Reconciliation
    // therefore advances by a stable keyset cursor and reports failures without blocking startup.
    tauri::async_runtime::spawn(async move {
        let _ = app_handle.emit(
            "ingestion-telemetry",
            ingestion_telemetry_envelope(IngestionProgress::CorpusReconciliationStarted {}),
        );
        let mut cursor = None;
        let mut indexed_documents = 0_u64;
        let mut skipped_documents = 0_u64;
        let mut failed_documents = 0_u64;
        let mut first_error = None;
        loop {
            let source = match repository.next_corpus_source_document(cursor).await {
                Ok(Some(source)) => source,
                Ok(None) => break,
                Err(error) => {
                    failed_documents = failed_documents.saturating_add(1);
                    first_error.get_or_insert_with(|| error.to_string());
                    break;
                }
            };
            cursor = Some((source.portal_key.clone(), source.document_id));
            let is_current = corpus_index
                .is_document_current(
                    source.portal_key.clone(),
                    source.document_id,
                    source.extracted_text_sha256.clone(),
                )
                .await;
            match is_current {
                Ok(true) => {
                    skipped_documents = skipped_documents.saturating_add(1);
                    continue;
                }
                Ok(false) => {}
                Err(error) => {
                    failed_documents = failed_documents.saturating_add(1);
                    first_error.get_or_insert_with(|| error.to_string());
                    continue;
                }
            }
            let result = async {
                let text = repository
                    .load_corpus_source_text(&source.portal_key, source.document_id)
                    .await?;
                corpus_index
                    .index_document(CorpusDocumentInput {
                        portal_key: source.portal_key,
                        request_id: source.request_id,
                        document_id: source.document_id,
                        raw_file_sha256: source.raw_file_sha256,
                        extracted_text_sha256: source.extracted_text_sha256,
                        text,
                    })
                    .await
            }
            .await;
            match result {
                Ok(()) => indexed_documents = indexed_documents.saturating_add(1),
                Err(error) => {
                    failed_documents = failed_documents.saturating_add(1);
                    first_error.get_or_insert_with(|| error.to_string());
                }
            }
            tokio::task::yield_now().await;
        }
        let _ = app_handle.emit(
            "ingestion-telemetry",
            ingestion_telemetry_envelope(IngestionProgress::CorpusReconciliationCompleted {
                indexed_documents,
                skipped_documents,
                failed_documents,
                first_error,
            }),
        );
    });
}

fn create_runtime_directory(path: &Path) -> Result<(), AppError> {
    std::fs::create_dir_all(path).map_err(|error| {
        AppError::Storage(format!(
            "Failed to provision runtime directory '{}': {error}",
            path.display()
        ))
    })
}
