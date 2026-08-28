//! macOS runtime environment hooks, folder provisioning, and window decoration profiles.

use crate::analysis::AnalyticsEngine;
use crate::commands::ingest::initialize_queue_runtime;
use crate::commands::investigation::AppInvestigationPaths;
use crate::error::AppError;
use crate::ingestion::queue::{ingestion_telemetry_envelope, IngestionProgress};
use crate::parsers::staging::is_managed_extraction_staging_name;
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
    clear_abandoned_ingestion_staging(&download_root)?;
    clear_abandoned_extraction_staging(&download_root)?;
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

/// Clears only the queue's dedicated pre-commit area before any worker can start. Durable tasks
/// and document rows reference committed object/attachment paths, never `.staging`, so retaining
/// this directory after a crash consumes disk without helping recovery. Container trees and MSG
/// attachment stores are intentionally untouched because their committed files are evidence.
fn clear_abandoned_ingestion_staging(download_root: &Path) -> Result<(), AppError> {
    let staging = download_root.join(".staging");
    let metadata = match std::fs::symlink_metadata(&staging) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::Storage(format!(
                "Failed to inspect ingestion staging directory '{}': {error}",
                staging.display()
            )))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::PortalSecurity(format!(
            "Ingestion staging path '{}' is not a managed directory",
            staging.display()
        )));
    }
    std::fs::remove_dir_all(&staging).map_err(|error| {
        AppError::Storage(format!(
            "Failed to clear abandoned ingestion staging directory '{}': {error}",
            staging.display()
        ))
    })
}

/// Removes partial atomic container extractions left by a process interruption.
///
/// Container staging lives beside the source document so its final rename remains on one
/// filesystem. The walk never follows symlinks and removes only directories carrying the
/// generated `.<destination>.<uuid>.extracting` name; committed container evidence is untouched.
fn clear_abandoned_extraction_staging(download_root: &Path) -> Result<(), AppError> {
    const MAXIMUM_INSPECTED_ENTRIES: usize = 100_000;
    const MAXIMUM_DIRECTORY_DEPTH: usize = 64;

    let mut pending = vec![(download_root.to_path_buf(), 0_usize)];
    let mut inspected = 0_usize;
    while let Some((directory, depth)) = pending.pop() {
        let entries = std::fs::read_dir(&directory).map_err(|error| {
            AppError::Storage(format!(
                "Failed to inspect extraction staging parent '{}': {error}",
                directory.display()
            ))
        })?;
        for entry in entries {
            inspected = inspected.saturating_add(1);
            if inspected > MAXIMUM_INSPECTED_ENTRIES {
                return Err(AppError::Storage(format!(
                    "Extraction staging reconciliation exceeded {MAXIMUM_INSPECTED_ENTRIES} entries"
                )));
            }
            let entry = entry.map_err(|error| {
                AppError::Storage(format!(
                    "Failed to read an extraction staging entry in '{}': {error}",
                    directory.display()
                ))
            })?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                AppError::Storage(format!(
                    "Failed to inspect possible extraction staging path '{}': {error}",
                    path.display()
                ))
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            let managed_staging = entry
                .file_name()
                .to_str()
                .is_some_and(is_managed_extraction_staging_name);
            if managed_staging {
                std::fs::remove_dir_all(&path).map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to clear abandoned extraction staging directory '{}': {error}",
                        path.display()
                    ))
                })?;
                continue;
            }
            if depth >= MAXIMUM_DIRECTORY_DEPTH {
                return Err(AppError::Storage(format!(
                    "Extraction staging reconciliation exceeded directory depth {MAXIMUM_DIRECTORY_DEPTH} at '{}'",
                    path.display()
                )));
            }
            pending.push((path, depth + 1));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{clear_abandoned_extraction_staging, clear_abandoned_ingestion_staging};

    #[test]
    fn startup_cleanup_removes_only_the_dedicated_staging_area() {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let download_root = directory.path().join("documents");
        let staging = download_root.join(".staging").join("nested");
        let evidence = download_root.join("objects").join("ab").join("evidence");
        std::fs::create_dir_all(&staging).expect("staging fixture must be created");
        std::fs::create_dir_all(evidence.parent().unwrap())
            .expect("object fixture directory must be created");
        std::fs::write(staging.join("partial"), b"incomplete")
            .expect("staged fixture must be written");
        std::fs::write(&evidence, b"committed evidence").expect("evidence fixture must be written");

        clear_abandoned_ingestion_staging(&download_root)
            .expect("abandoned staging cleanup must succeed");

        assert!(!download_root.join(".staging").exists());
        assert_eq!(
            std::fs::read(evidence).expect("committed evidence must remain"),
            b"committed evidence"
        );
        clear_abandoned_ingestion_staging(&download_root)
            .expect("cleanup must be idempotent when staging is absent");
    }

    #[test]
    fn startup_cleanup_removes_only_generated_container_staging_directories() {
        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let root = directory.path().join("documents");
        let parent = root.join("portal").join("objects");
        let abandoned =
            parent.join(".report_contents.550e8400-e29b-41d4-a716-446655440000.extracting");
        let ordinary_hidden = parent.join(".research.extracting");
        let committed = parent.join("report_contents");
        std::fs::create_dir_all(&abandoned).expect("abandoned fixture");
        std::fs::create_dir_all(&ordinary_hidden).expect("ordinary hidden fixture");
        std::fs::create_dir_all(&committed).expect("committed fixture");
        std::fs::write(abandoned.join("partial"), b"partial").expect("partial fixture");
        std::fs::write(committed.join("evidence"), b"retained").expect("evidence fixture");

        clear_abandoned_extraction_staging(&root).expect("reconciliation must succeed");

        assert!(!abandoned.exists());
        assert!(ordinary_hidden.is_dir());
        assert_eq!(
            std::fs::read(committed.join("evidence")).expect("committed evidence must remain"),
            b"retained"
        );
    }

    #[cfg(unix)]
    #[test]
    fn startup_cleanup_refuses_to_follow_a_staging_symlink() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temporary directory must be created");
        let download_root = directory.path().join("documents");
        let outside = directory.path().join("outside");
        std::fs::create_dir_all(&download_root).expect("download root must be created");
        std::fs::create_dir_all(&outside).expect("outside fixture must be created");
        std::fs::write(outside.join("keep"), b"outside").expect("outside fixture must be written");
        symlink(&outside, download_root.join(".staging"))
            .expect("staging symlink fixture must be created");

        assert!(clear_abandoned_ingestion_staging(&download_root).is_err());
        assert_eq!(
            std::fs::read(outside.join("keep")).expect("outside data must remain"),
            b"outside"
        );
    }
}
