//! Research snapshot export: a cited, read-only copy of the evidence projection for outside tools.

use std::path::PathBuf;

use tauri::State;

use crate::atomic_write::write_atomic;
use crate::error::AppError;
use crate::integrity::{load_or_create_signing_key, sha256_file_async, sign_package};
use crate::storage::sqlite::DuckDbRepository;
use crate::types::snapshot::{
    ResearchSnapshotManifest, ResearchSnapshotResult, RESEARCH_SNAPSHOT_SCHEMA,
};

const SNAPSHOT_DIRECTORY: &str = "snapshots";

/// Writes `reports/snapshots/onrecord-snapshot-<instant>.duckdb` plus a signed manifest sidecar
/// naming the file digest and corpus state. The signature is this installation's export key, so it
/// proves the file is unchanged since export, not who exported it.
#[tauri::command]
pub async fn export_research_snapshot(
    repository: State<'_, DuckDbRepository>,
    report_directory: State<'_, PathBuf>,
) -> Result<ResearchSnapshotResult, AppError> {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let file_name = format!("onrecord-snapshot-{stamp}.duckdb");
    let path = report_directory.join(SNAPSHOT_DIRECTORY).join(&file_name);
    let build = repository
        .export_research_snapshot(path.clone(), env!("CARGO_PKG_VERSION").to_string())
        .await?;
    let file_sha256 = sha256_file_async(&path)
        .await
        .map_err(|error| AppError::Export(format!("Failed to hash the snapshot: {error}")))?;
    let manifest = ResearchSnapshotManifest {
        schema_version: RESEARCH_SNAPSHOT_SCHEMA.to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: build.created_at,
        file_name,
        file_sha256,
        corpus_state_digest: build.corpus_state_digest,
        findings_digest: build.findings_digest,
        table_rows: build.table_rows,
    };
    let signing_key = load_or_create_signing_key(&report_directory).await?;
    let signed = sign_package(manifest.clone(), &signing_key).map_err(AppError::Export)?;
    let manifest_path = path.with_extension("duckdb.manifest.json");
    let bytes = serde_json::to_vec_pretty(&signed)
        .map_err(|error| AppError::Export(format!("Failed to encode the manifest: {error}")))?;
    write_atomic(&manifest_path, "snapshot manifest", &bytes)?;
    Ok(ResearchSnapshotResult {
        path: path.to_string_lossy().into_owned(),
        manifest_path: manifest_path.to_string_lossy().into_owned(),
        manifest,
    })
}
