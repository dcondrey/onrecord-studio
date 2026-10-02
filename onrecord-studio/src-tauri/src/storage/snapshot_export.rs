//! Research snapshot: a read-only DuckDB file holding a documented, stable projection of the
//! evidence corpus, so a query run in Python, R or the DuckDB CLI can be cited against an exact
//! corpus state. The live core file is never handed out: its schema changes between releases and
//! the running app holds its write lock. The projection below is the contract; see
//! `docs/research-snapshot.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use duckdb::Connection;
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::integrity::digest_hex;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::snapshot::RESEARCH_SNAPSHOT_SCHEMA;

const ATTACHED: &str = "onrecord_research_snapshot";

/// Each entry is (snapshot table, SELECT over the core). Columns are the public contract: free
/// text that can carry personal information (request descriptions, reviewer notes, local paths,
/// extracted text) is deliberately absent; digests stand in for content.
const PROJECTION: &[(&str, &str)] = &[
    (
        "portals",
        "SELECT subdomain AS portal_key, display_name, base_url, source_kind, lifecycle_state,
                verified_at, last_synced_at
         FROM portals",
    ),
    (
        "agencies",
        "SELECT id, portal_subdomain AS portal_key, name, department_name FROM agencies",
    ),
    (
        "requests",
        "SELECT id, portal_subdomain AS portal_key, rpid, title, status, agency_id, created_at,
                updated_at, closed_at, due_date, determination_at, determination_source
         FROM requests",
    ),
    (
        "statutory_extensions",
        "SELECT request_id, portal_subdomain AS portal_key, notice_at, expected_determination_at,
                reason, notice_source, recorded_at
         FROM statutory_extensions",
    ),
    (
        "documents",
        "SELECT id, request_id, portal_subdomain AS portal_key, file_name, file_size, content_type,
                download_url, visibility, created_at, updated_at, raw_file_sha256,
                extracted_text_sha256, extraction_status, extraction_format, was_ocr_processed,
                shannon_entropy, entropy_sample_bytes, suspicious_data_density
         FROM documents",
    ),
    (
        "statutory_violations",
        "SELECT violation_id, portal_subdomain AS portal_key, request_id, matched_statute_id,
                matched_version_id, mandate_id, classification, calculated_variance_metric,
                variance_days, confidence_basis_points, evidence_summary,
                supporting_document_ids_json, missing_evidence_json, classifier_version,
                review_status, first_detected_at, evaluated_at
         FROM statutory_violations",
    ),
    (
        "findings",
        "SELECT id, kind, severity, confidence_basis_points, subject_portal_key AS portal_key,
                subject_agency_id, subject_request_id, subject_document_id, subject_entity_id,
                headline, explanation, citations_json, discovered_at, last_seen_at, dismissed_at,
                source_detector
         FROM findings",
    ),
];

pub struct SnapshotBuild {
    pub created_at: String,
    pub corpus_state_digest: String,
    pub findings_digest: String,
    pub table_rows: BTreeMap<String, u64>,
}

impl DuckDbRepository {
    /// The corpus digest a snapshot would carry, without writing one, so a search can name the
    /// corpus state that answered it. Covers document text only: request metadata such as status
    /// or title is not part of it.
    pub async fn corpus_state_identity(&self) -> Result<(String, u64), AppError> {
        self.query_connection("Read corpus state", |conn| {
            let digest = corpus_state_digest(conn)?;
            let documents: i64 = conn
                .query_row("SELECT count(*) FROM documents", [], |row| row.get(0))
                .map_err(|error| {
                    AppError::Storage(format!("Failed to count documents: {error}"))
                })?;
            Ok((digest, u64::try_from(documents).unwrap_or(0)))
        })
        .await
    }

    /// Writes the projection to a new DuckDB file at `output`. Refuses to overwrite: a snapshot
    /// that has been cited must never change underneath its citation.
    pub async fn export_research_snapshot(
        &self,
        output: PathBuf,
        app_version: String,
    ) -> Result<SnapshotBuild, AppError> {
        if output.exists() {
            return Err(AppError::Export(format!(
                "Snapshot already exists: {}",
                output.display()
            )));
        }
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                AppError::Export(format!("Failed to create the snapshot folder: {error}"))
            })?;
        }
        let created_at = chrono::Utc::now().to_rfc3339();
        let manifest_created_at = created_at.clone();
        self.query_connection("Export research snapshot", move |conn| {
            let corpus_state_digest = corpus_state_digest(conn)?;
            let findings_digest = findings_digest(conn)?;
            let result = build_snapshot(
                conn,
                &output,
                &[
                    ("schema_version", RESEARCH_SNAPSHOT_SCHEMA),
                    ("app_version", app_version.as_str()),
                    ("created_at", manifest_created_at.as_str()),
                    ("corpus_state_digest", corpus_state_digest.as_str()),
                    ("findings_digest", findings_digest.as_str()),
                ],
            );
            // DETACH must run even when a table failed, or the live connection keeps the file open.
            let detach = conn.execute_batch(&format!("DETACH {ATTACHED}"));
            let table_rows = result?;
            detach.map_err(|error| {
                AppError::Export(format!("Failed to release the snapshot file: {error}"))
            })?;
            Ok(SnapshotBuild {
                created_at,
                corpus_state_digest,
                findings_digest,
                table_rows,
            })
        })
        .await
    }
}

fn build_snapshot(
    conn: &mut Connection,
    output: &Path,
    manifest: &[(&str, &str)],
) -> Result<BTreeMap<String, u64>, AppError> {
    let export =
        |error: duckdb::Error| AppError::Export(format!("Snapshot export failed: {error}"));
    let path = output.to_string_lossy().replace('\'', "''");
    // Written for recipients, so at the oldest storage version any current DuckDB reads rather
    // than the app's own (`storage::open_duckdb`). A snapshot carries digests, not text, so the
    // compression that version gives up does not apply here.
    conn.execute_batch(&format!(
        "ATTACH '{path}' AS {ATTACHED} (STORAGE_VERSION 'v0.10.2')"
    ))
    .map_err(export)?;
    let mut table_rows = BTreeMap::new();
    for (table, select) in PROJECTION {
        conn.execute_batch(&format!("CREATE TABLE {ATTACHED}.{table} AS {select}"))
            .map_err(export)?;
        let rows: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM {ATTACHED}.{table}"),
                [],
                |row| row.get(0),
            )
            .map_err(export)?;
        table_rows.insert((*table).to_string(), u64::try_from(rows).unwrap_or(0));
    }
    conn.execute_batch(&format!(
        "CREATE TABLE {ATTACHED}.snapshot_manifest (key TEXT PRIMARY KEY, value TEXT NOT NULL)"
    ))
    .map_err(export)?;
    for (key, value) in manifest {
        conn.execute(
            &format!("INSERT INTO {ATTACHED}.snapshot_manifest VALUES (?, ?)"),
            duckdb::params![key, value],
        )
        .map_err(export)?;
    }
    for (table, rows) in &table_rows {
        conn.execute(
            &format!("INSERT INTO {ATTACHED}.snapshot_manifest VALUES (?, ?)"),
            duckdb::params![format!("rows.{table}"), rows.to_string()],
        )
        .map_err(export)?;
    }
    Ok(table_rows)
}

/// The identity of the evidence corpus, independent of row layout: one line per document in
/// (portal, id) order, each carrying the digest of its extracted text or nothing.
pub(crate) fn corpus_state_digest(conn: &Connection) -> Result<String, AppError> {
    digest_rows(
        conn,
        "SELECT portal_subdomain, id, extracted_text_sha256 FROM documents
         ORDER BY portal_subdomain, id",
    )
}

fn findings_digest(conn: &Connection) -> Result<String, AppError> {
    digest_rows(
        conn,
        "SELECT id, last_seen_at, '' FROM findings ORDER BY id",
    )
}

fn digest_rows(conn: &Connection, sql: &str) -> Result<String, AppError> {
    let export =
        |error: duckdb::Error| AppError::Export(format!("Snapshot digest failed: {error}"));
    let mut statement = conn.prepare(sql).map_err(export)?;
    let mut hasher = Sha256::new();
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, duckdb::types::Value>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(export)?;
    for row in rows {
        let (first, second, third) = row.map_err(export)?;
        let second = match second {
            duckdb::types::Value::BigInt(value) => value.to_string(),
            duckdb::types::Value::Int(value) => value.to_string(),
            duckdb::types::Value::Text(value) => value,
            other => format!("{other:?}"),
        };
        hasher.update(first.as_bytes());
        hasher.update(b"\t");
        hasher.update(second.as_bytes());
        hasher.update(b"\t");
        hasher.update(third.unwrap_or_default().as_bytes());
        hasher.update(b"\n");
    }
    Ok(digest_hex(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::{AccessMode, Config};

    const MIGRATION: &str = include_str!("../../migrations/0001_initial_schema.sql");

    #[tokio::test]
    async fn snapshot_is_a_readable_projection_whose_digest_tracks_the_corpus() {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = DuckDbRepository::new(dir.path().join("core.duckdb")).expect("core opens");
        repo.run_migrations(MIGRATION).await.expect("migrations");
        repo.query_connection("seed", |conn| {
            conn.execute_batch(
                "INSERT INTO portals (subdomain, display_name, base_url, origin_url)
                 VALUES ('sd', 'San Diego', 'https://sd.example', 'https://sd.example');
                 INSERT INTO requests (id, portal_subdomain, status, created_at, updated_at)
                 VALUES (1, 'sd', 'Closed', '2026-01-01', '2026-01-02');
                 INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility,
                     created_at, updated_at, extracted_text_sha256)
                 VALUES (10, 1, 'sd', 'a.pdf', 'public', '2026-01-01', '2026-01-01', 'abc');",
            )
            .map_err(|error| AppError::Storage(error.to_string()))
        })
        .await
        .expect("seed rows");

        let first = dir.path().join("snapshots/one.duckdb");
        let built = repo
            .export_research_snapshot(first.clone(), "1.0.0".into())
            .await
            .expect("first snapshot");
        assert_eq!(built.table_rows["documents"], 1);
        assert!(
            repo.export_research_snapshot(first.clone(), "1.0.0".into())
                .await
                .is_err(),
            "an existing snapshot must never be overwritten"
        );

        let reader = Connection::open_with_flags(
            &first,
            Config::default()
                .access_mode(AccessMode::ReadOnly)
                .expect("read-only config"),
        )
        .expect("snapshot opens read-only while the core stays open");
        let digest: String = reader
            .query_row(
                "SELECT value FROM snapshot_manifest WHERE key = 'corpus_state_digest'",
                [],
                |row| row.get(0),
            )
            .expect("manifest row");
        assert_eq!(digest, built.corpus_state_digest);
        let storage_version: String = reader
            .query_row(
                "SELECT tags['storage_version'] FROM duckdb_databases() WHERE NOT internal
                 AND path IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .expect("storage version tag");
        assert_eq!(
            storage_version, "v1.0.0+",
            "a snapshot is written for recipients' readers, not the app's own storage version"
        );
        let tables: i64 = reader
            .query_row(
                "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'main'",
                [],
                |row| row.get(0),
            )
            .expect("table count");
        assert_eq!(tables as usize, PROJECTION.len() + 1);
        let text_columns: i64 = reader
            .query_row(
                "SELECT count(*) FROM information_schema.columns
                 WHERE column_name IN ('description', 'local_path', 'reviewer_notes')",
                [],
                |row| row.get(0),
            )
            .expect("column scan");
        assert_eq!(text_columns, 0, "free-text columns must not leave the core");

        repo.query_connection("change corpus", |conn| {
            conn.execute_batch("UPDATE documents SET extracted_text_sha256 = 'def' WHERE id = 10")
                .map_err(|error| AppError::Storage(error.to_string()))
        })
        .await
        .expect("update");
        let second = repo
            .export_research_snapshot(dir.path().join("snapshots/two.duckdb"), "1.0.0".into())
            .await
            .expect("second snapshot");
        assert_ne!(second.corpus_state_digest, built.corpus_state_digest);
        let (identity, documents) = repo.corpus_state_identity().await.expect("identity");
        assert_eq!(identity, second.corpus_state_digest);
        assert_eq!(documents, 1);
        assert_eq!(second.findings_digest, built.findings_digest);
    }
}
