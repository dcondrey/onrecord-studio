use super::sqlite::DuckDbRepository;
use crate::{
    error::AppError,
    integrity::{sha256_file, sha256_hex},
    types::{
        AnalyticalImportRequest, DataMaintenanceEvent, DataMaintenancePolicy, DataMaintenanceStatus,
    },
};
use chrono::{Duration, Utc};
use duckdb::{params, OptionalExt};
use std::path::{Path, PathBuf};

fn storage(error: impl std::fmt::Display) -> AppError {
    AppError::Storage(error.to_string())
}
pub(crate) fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

const CITY_FINANCE_EXPORTS: [&str; 2] = [
    "https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv",
    "https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv",
];
const PUBLISHED_EXPORT_SCOPE: &str = "Complete published City operating-finance CSV export, verified against its acquisition SHA-256 and parsed without rejected rows. Coverage is the published file, not all City costs, a complete fiscal year, or finalized/audited accounts.";

/// Automatic imports start conservatively partial. Only these two documented bulk exports
/// can acquire file-complete coverage after the parser and acquisition identity are verified.
/// The origin receipt distinguishes old automatic imports from intentionally partial manual
/// snapshots. All checks and writes share one transaction; no live network/file assumptions.
fn reconcile_city_finance_exports(conn: &mut duckdb::Connection) -> Result<bool, AppError> {
    let tx = conn.transaction().map_err(storage)?;
    let candidates = {
        let mut statement = tx.prepare("SELECT DISTINCT s.snapshot_id, s.validation_summary_json,
                o.portal_key, o.document_id, s.source_sha256
            FROM analytical_snapshots s
            JOIN data_maintenance_origins o ON o.snapshot_id = s.snapshot_id
            JOIN documents d ON d.id = o.document_id AND d.portal_subdomain = o.portal_key
            WHERE s.partial_dataset = 1 AND s.rejected_rows = 0 AND s.imported_rows > 0
                AND s.source_uri IN (?1, ?2) AND s.source_uri = d.download_url
                AND d.download_status = 'completed' AND LOWER(TRIM(d.visibility)) = 'public'
                AND LENGTH(d.raw_file_sha256) = 64 AND LOWER(s.source_sha256) = LOWER(d.raw_file_sha256)
                AND json_extract_string(s.validation_summary_json, '$.parserVersion') = ?3
                AND json_extract_string(s.validation_summary_json, '$.sourceFormat') = 'CSV'
                AND json_extract_string(s.validation_summary_json, '$.sourceDeclaresPartial') = 'false'
            ORDER BY s.snapshot_id LIMIT 2").map_err(storage)?;
        statement
            .query_map(
                params![
                    CITY_FINANCE_EXPORTS[0],
                    CITY_FINANCE_EXPORTS[1],
                    super::sqlite::ANALYTICAL_PARSER_VERSION
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, u64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?
    };
    for (snapshot, raw, portal, document, hash) in &candidates {
        let mut validation: serde_json::Value = serde_json::from_str(raw).map_err(storage)?;
        validation["coverage"] = serde_json::json!({"basis":"COMPLETE_PUBLISHED_EXPORT", "scope":PUBLISHED_EXPORT_SCOPE});
        if let Some(warnings) = validation["warnings"].as_array_mut() {
            warnings.push(serde_json::json!(PUBLISHED_EXPORT_SCOPE));
        }
        tx.execute("UPDATE analytical_snapshots SET partial_dataset = 0, validation_summary_json = ?2 WHERE snapshot_id = ?1", params![snapshot, validation.to_string()]).map_err(storage)?;
        tx.execute("INSERT INTO data_maintenance_jobs VALUES (?1, 1, ?2, 'COVERAGE_VERIFIED', ?3, ?2) ON CONFLICT (job_key) DO UPDATE SET detail = excluded.detail, recorded_at = excluded.recorded_at", params![format!("import:{portal}:{document}:{hash}:published-coverage"), now(), PUBLISHED_EXPORT_SCOPE]).map_err(storage)?;
    }
    tx.commit().map_err(storage)?;
    Ok(!candidates.is_empty())
}

impl DuckDbRepository {
    pub async fn recover_maintenance_storage(&self, root: &Path) -> Result<bool, AppError> {
        let root = root.to_path_buf();
        self.query_connection("Recover pending retention", move |conn| {
            recover_retention(conn, &root)
        })
        .await
    }

    pub async fn maintenance_status_with_storage(
        &self,
        root: &Path,
    ) -> Result<DataMaintenanceStatus, AppError> {
        let mut status = self.maintenance_status().await?;
        status.retained_file_bytes = managed_file_bytes(root).await?;
        Ok(status)
    }

    pub async fn check_retained_file_budget(&self, root: &Path) -> Result<(), AppError> {
        let managed_bytes = managed_file_bytes(root).await?;
        self.query_connection("Retained file capacity", move |conn| {
            let policy: Option<String> = conn.query_row("SELECT policy_json FROM data_maintenance_policy WHERE singleton", [], |row| row.get(0)).optional().map_err(storage)?;
            let policy = policy.map(|value| serde_json::from_str::<DataMaintenancePolicy>(&value)).transpose().map_err(storage)?.unwrap_or_default();
            let bytes: i64 = conn.query_row("SELECT COALESCE(SUM(bytes), 0) FROM (SELECT local_path, MAX(COALESCE(file_size, 0)) AS bytes FROM documents WHERE local_path IS NOT NULL GROUP BY local_path)", [], |row| row.get(0)).map_err(storage)?;
            if managed_bytes.max(bytes.max(0) as u64) >= policy.retained_file_budget_bytes { return Err(AppError::PortalCapacity("Managed-file budget reached. Finish processing, enable compact retention for new acquisitions, or adjust the budget before downloading more. In-flight files and the database are additional storage; disk usage is cached for up to five seconds.".into())); }
            Ok(())
        }).await
    }
    pub async fn maintenance_status(&self) -> Result<DataMaintenanceStatus, AppError> {
        self.query_connection("Data maintenance status", |conn| {
            let default = serde_json::to_string(&DataMaintenancePolicy::default()).map_err(storage)?;
            conn.execute("INSERT INTO data_maintenance_policy VALUES (TRUE, ?1, ?2) ON CONFLICT DO NOTHING", params![default, now()]).map_err(storage)?;
            let (policy, effective_at): (String, String) = conn.query_row("SELECT policy_json, effective_at FROM data_maintenance_policy WHERE singleton", [], |row| Ok((row.get(0)?, row.get(1)?))).map_err(storage)?;
            let retained: i64 = conn.query_row("SELECT COALESCE(SUM(bytes), 0) FROM (SELECT local_path, MAX(COALESCE(file_size, 0)) AS bytes FROM documents WHERE local_path IS NOT NULL GROUP BY local_path)", [], |row| row.get(0)).map_err(storage)?;
            let referenced: i64 = conn.query_row("SELECT COUNT(*) FROM documents WHERE extraction_deferred_reason LIKE 'REFERENCE_ONLY:%' AND local_path IS NULL", [], |row| row.get(0)).map_err(storage)?;
            let reclaimed: i64 = conn.query_row("SELECT COALESCE(SUM(byte_count), 0) FROM data_retention_receipts WHERE state = 'RECLAIMED'", [], |row| row.get(0)).map_err(storage)?;
            let mut statement = conn.prepare("SELECT action, detail, recorded_at FROM data_maintenance_jobs ORDER BY recorded_at DESC LIMIT 20").map_err(storage)?;
            let events = statement.query_map([], |row| Ok(DataMaintenanceEvent { action: row.get(0)?, detail: row.get(1)?, recorded_at: row.get(2)? })).map_err(storage)?.collect::<Result<Vec<_>, _>>().map_err(storage)?;
            Ok(DataMaintenanceStatus { policy: serde_json::from_str(&policy).map_err(storage)?, effective_at, retained_file_bytes: retained.max(0) as u64, referenced_documents: referenced.max(0) as u64, reclaimed_file_bytes: reclaimed.max(0) as u64, events })
        }).await
    }

    pub async fn save_maintenance_policy(
        &self,
        policy: DataMaintenancePolicy,
    ) -> Result<DataMaintenanceStatus, AppError> {
        if !(1..=720).contains(&policy.refresh_hours)
            || !(100 * 1024 * 1024..=1024_u64.pow(4)).contains(&policy.retained_file_budget_bytes)
            || !matches!(
                policy.retention_mode.as_str(),
                "ORIGINALS" | "REFERENCE_ONLY"
            )
        {
            return Err(AppError::DataParsing("Use a refresh interval of 1–720 hours, a retained-file budget of 100 MiB–1 TiB, and ORIGINALS or REFERENCE_ONLY retention.".into()));
        }
        let encoded = serde_json::to_string(&policy).map_err(storage)?;
        self.query_connection("Save maintenance policy", move |conn| {
            let previous: Option<String> = conn.query_row("SELECT policy_json FROM data_maintenance_policy WHERE singleton", [], |row| row.get(0)).optional().map_err(storage)?;
            let new_retention = previous.and_then(|raw| serde_json::from_str::<DataMaintenancePolicy>(&raw).ok()).is_none_or(|old| old.retention_mode != policy.retention_mode);
            conn.execute("INSERT INTO data_maintenance_policy VALUES (TRUE, ?1, ?2) ON CONFLICT (singleton) DO UPDATE SET policy_json = excluded.policy_json, effective_at = CASE WHEN ?3 THEN excluded.effective_at ELSE data_maintenance_policy.effective_at END", params![encoded, now(), new_retention]).map_err(storage)?;
            Ok(())
        }).await?;
        self.maintenance_status().await
    }

    pub(crate) async fn record_maintenance(
        &self,
        key: String,
        action: &str,
        detail: String,
        delay_hours: u32,
    ) -> Result<(), AppError> {
        let action = action.to_string();
        self.query_connection("Record data maintenance", move |conn| {
            let next = (Utc::now() + Duration::hours(i64::from(delay_hours))).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            conn.execute("INSERT INTO data_maintenance_jobs VALUES (?1, 1, ?2, ?3, ?4, ?5) ON CONFLICT (job_key) DO UPDATE SET attempts = data_maintenance_jobs.attempts + 1, next_attempt_at = excluded.next_attempt_at, action = excluded.action, detail = excluded.detail, recorded_at = excluded.recorded_at", params![key, next, action, detail, now()]).map_err(storage)?;
            Ok(())
        }).await
    }

    /// Search all eligible documents, rather than the UI's latest-500 convenience list.
    pub async fn maintain_one_table(&self) -> Result<bool, AppError> {
        let reconciled = self
            .query_connection(
                "Verify published finance export coverage",
                reconcile_city_finance_exports,
            )
            .await?;
        let candidate = self.query_connection("Pending automatic table import", |conn| {
            let mut statement = conn.prepare("SELECT d.id, d.portal_subdomain, d.file_name, d.local_path, d.download_url, d.raw_file_sha256
                FROM documents d WHERE d.local_path IS NOT NULL AND d.download_status = 'completed'
                AND LOWER(TRIM(d.visibility)) = 'public' AND LENGTH(d.raw_file_sha256) = 64
                AND regexp_matches(lower(COALESCE(d.file_name, '')), '\\.(csv|tsv|json|jsonl|ndjson|geojson|xlsx|xls|ods)$')
                AND NOT EXISTS (SELECT 1 FROM analytical_snapshots s WHERE s.source_sha256 = d.raw_file_sha256
                    AND s.source_uri IS NOT DISTINCT FROM d.download_url AND json_extract_string(s.validation_summary_json, '$.parserVersion') = ?2)
                AND NOT EXISTS (SELECT 1 FROM data_maintenance_jobs j WHERE j.job_key = 'import:' || d.portal_subdomain || ':' || CAST(d.id AS VARCHAR) || ':' || d.raw_file_sha256 AND j.next_attempt_at > ?1)
                ORDER BY d.updated_at, d.portal_subdomain, d.id LIMIT 1").map_err(storage)?;
            statement.query_row(params![now(), super::sqlite::ANALYTICAL_PARSER_VERSION], |row| Ok(crate::types::AnalyticalImportCandidate {
                document_id: row.get(0)?, portal_key: row.get(1)?, file_name: row.get(2)?, local_path: row.get(3)?, source_uri: row.get(4)?, source_sha256: row.get(5)?,
            })).optional().map_err(storage)
        }).await?;
        let Some(candidate) = candidate else {
            return Ok(reconciled);
        };
        let key = format!(
            "import:{}:{}:{}",
            candidate.portal_key,
            candidate.document_id,
            candidate.source_sha256.as_deref().unwrap_or_default()
        );
        let input = AnalyticalImportRequest {
            dataset_kind: crate::types::AnalyticalDatasetKind::RegionalData,
            display_name: candidate.file_name.clone(),
            source_uri: candidate.source_uri,
            local_path: candidate.local_path,
            observation_period_start: None,
            observation_period_end: None,
            // Parser validation and the automatic-origin receipt must exist before granting
            // file-complete coverage, even when the URL names a known bulk CSV export.
            partial_dataset: true,
            worksheet: None,
            source_file_name: Some(candidate.file_name.clone()),
            expected_source_sha256: candidate.source_sha256,
        };
        // The snapshot write and its origin receipt must commit or roll back together: a crash
        // between two separate transactions would leave a snapshot on disk that later
        // maintenance/purge logic cannot tell apart from one nobody ever recorded an origin for.
        let origin = (candidate.portal_key.clone(), candidate.document_id);
        match self
            .import_analytical_csv_recording_origin(input, Some(origin))
            .await
        {
            Ok(result) => {
                self.query_connection(
                    "Verify published finance export coverage",
                    reconcile_city_finance_exports,
                )
                .await?;
                self.record_maintenance(key, "IMPORTED", format!("{}: {} rows imported, {} rejected. {}", candidate.file_name, result.imported_rows, result.rejected_rows, result.schema_warnings.join(" ")), 24).await?;
            },
            Err(error) => self.record_maintenance(key, "IMPORT_NEEDS_REVIEW", format!("{}: {error}. Other files continue; this file retries in 24 hours or when its source hash changes.", candidate.file_name), 24).await?,
        }
        Ok(true)
    }

    pub(crate) async fn source_refresh_due(&self, key: &str, hours: u32) -> Result<bool, AppError> {
        let key = key.to_string();
        self.query_connection("Source refresh schedule", move |conn| {
            let cutoff = (Utc::now() - Duration::hours(i64::from(hours))).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            conn.query_row("SELECT NOT EXISTS (SELECT 1 FROM portal_sync_runs WHERE portal_subdomain = ?1 AND started_at > ?2)
                AND NOT EXISTS (SELECT 1 FROM data_maintenance_jobs WHERE job_key = 'refresh:' || ?1 AND next_attempt_at > ?3)", params![key, cutoff, now()], |row| row.get(0)).map_err(storage)
        }).await
    }

    /// Raw bytes are retired only after an explicit reference-retention policy was enabled,
    /// complete table ingestion (for tabular files), digest verification, and a durable receipt.
    pub async fn compact_one_document(
        &self,
        root: &Path,
        effective_at: &str,
    ) -> Result<bool, AppError> {
        let root = root.to_path_buf();
        let effective = effective_at.to_string();
        self.query_connection("Compact referenced document", move |conn| {
            if recover_retention(conn, &root)? { return Ok(true); }
            let candidate = conn.query_row("SELECT d.portal_subdomain, d.id, d.local_path, d.download_url, d.raw_file_sha256, d.file_size
                FROM documents d JOIN portals p ON p.subdomain = d.portal_subdomain
                WHERE d.local_path IS NOT NULL AND LOWER(TRIM(d.visibility)) = 'public'
                AND d.download_status = 'completed' AND d.extraction_status = 'SEARCHABLE'
                AND d.retention_eligible_at >= ?1 AND LENGTH(d.raw_file_sha256) = 64
                AND p.lifecycle_state = 'ACTIVE' AND p.source_kind NOT IN ('LOCAL_FILE', 'PASTED_CONTENT')
                AND EXISTS (SELECT 1 FROM documents_fts f WHERE f.document_id = d.id AND f.portal_subdomain = d.portal_subdomain AND LENGTH(f.extracted_text) > 0 AND sha256(f.extracted_text) = LOWER(d.extracted_text_projection_sha256))
                AND (SELECT COUNT(*) FROM documents other WHERE other.local_path = d.local_path) = 1
                AND (regexp_matches(lower(d.file_name), '\\.(pdf|html|htm|txt|md|rtf|docx|pptx|odt|odp)$') OR (regexp_matches(lower(d.file_name), '\\.(csv|tsv|json|jsonl|ndjson|geojson)$') AND EXISTS (
                    SELECT 1 FROM analytical_snapshots s WHERE s.source_sha256 = d.raw_file_sha256 AND s.source_uri = d.download_url AND s.rejected_rows = 0 AND s.imported_rows > 0 AND COALESCE(json_extract_string(s.validation_summary_json, '$.sourceDeclaresPartial'), 'true') = 'false')))
                AND NOT EXISTS (SELECT 1 FROM data_maintenance_jobs j WHERE j.job_key = 'compact:' || d.portal_subdomain || ':' || CAST(d.id AS VARCHAR) || ':' || d.raw_file_sha256 AND j.next_attempt_at > ?2)
                ORDER BY d.file_size DESC LIMIT 1", params![effective, now()], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, String>(4)?, row.get::<_, i64>(5)?))).optional().map_err(storage)?;
            let Some((portal, document_id, path, url, hash, bytes)) = candidate else { return Ok(false); };
            let url = url.and_then(|value| reqwest::Url::parse(&value).ok());
            let Some(url) = url.filter(stable_public_reference) else {
                conn.execute("INSERT INTO data_maintenance_jobs VALUES (?1, 1, ?2, 'RETAINED_ORIGINAL', 'Original retained: no stable public URL without credentials or expiring query parameters.', ?3) ON CONFLICT (job_key) DO UPDATE SET next_attempt_at = excluded.next_attempt_at, recorded_at = excluded.recorded_at", params![format!("compact:{portal}:{document_id}:{hash}"), (Utc::now() + Duration::hours(24)).to_rfc3339(), now()]).map_err(storage)?;
                return Ok(true);
            };
            let original = match checked_path(&root, &path, &hash) {
                Ok(path) => path,
                Err(error) => {
                    conn.execute("INSERT INTO data_maintenance_jobs VALUES (?1, 1, ?2, 'RETAINED_ORIGINAL', ?3, ?4) ON CONFLICT (job_key) DO UPDATE SET next_attempt_at = excluded.next_attempt_at, detail = excluded.detail, recorded_at = excluded.recorded_at", params![format!("compact:{portal}:{document_id}:{hash}"), (Utc::now() + Duration::hours(24)).to_rfc3339(), error.to_string(), now()]).map_err(storage)?;
                    return Ok(true);
                }
            };
            let receipt_id = sha256_hex(format!("{portal}:{document_id}:{hash}:{}", now()).as_bytes());
            let pending = original.with_file_name(format!(".retire-{receipt_id}"));
            if pending.exists() { return Err(storage("Retention staging path already exists")); }
            conn.execute("INSERT INTO data_retention_receipts VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'PREPARED', ?9)", params![receipt_id, portal, document_id, original.to_string_lossy(), pending.to_string_lossy(), url.as_str(), hash, bytes, now()]).map_err(storage)?;
            std::fs::rename(&original, &pending).map_err(storage)?;
            let tx = conn.transaction().map_err(storage)?;
            let changed = tx.execute("UPDATE documents SET local_path = NULL, extraction_deferred_reason = 'REFERENCE_ONLY: Original bytes were released after extraction. Derived text/tables, original URL and SHA-256 are retained; the publisher may change or remove the original.' WHERE id = ?1 AND portal_subdomain = ?2 AND raw_file_sha256 = ?3", params![document_id, portal, hash]).map_err(storage)?;
            if changed != 1 { return Err(storage("Document changed before retention commit; original will be restored")); }
            tx.execute("UPDATE data_retention_receipts SET state = 'REFERENCED' WHERE receipt_id = ?1", params![receipt_id]).map_err(storage)?;
            tx.commit().map_err(storage)?;
            // On any interruption before this point, the next pass restores PREPARED files.
            // After the commit, the receipt makes pending deletion recoverable and auditable.
            std::fs::remove_file(&pending).map_err(storage)?;
            conn.execute("UPDATE data_retention_receipts SET state = 'RECLAIMED' WHERE receipt_id = ?1", params![receipt_id]).map_err(storage)?;
            Ok(true)
        }).await
    }
}

/// Count all managed files, including older content-addressed versions and crash leftovers.
/// A short shared cache prevents a directory walk for every parallel download. Never follow
/// links outside the managed tree. Apparent file sizes deliberately overcount sparse files.
async fn managed_file_bytes(root: &Path) -> Result<u64, AppError> {
    type UsageCache = std::collections::HashMap<PathBuf, (std::time::Instant, u64)>;
    static CACHE: std::sync::LazyLock<std::sync::Mutex<UsageCache>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(UsageCache::new()));
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut cache = CACHE.lock().map_err(storage)?;
        if let Some((at, bytes)) = cache.get(&root) {
            if at.elapsed() < std::time::Duration::from_secs(5) { return Ok(*bytes); }
        }
        let mut pending = vec![root.clone()];
        let mut bytes = 0_u64;
        let mut entries = 0;
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(directory).map_err(storage)? {
                let entry = entry.map_err(storage)?;
                entries += 1;
                if entries > 1_000_000 { return Err(storage("Managed-file inventory exceeds one million entries; review storage before further acquisition")); }
                let metadata = entry.path().symlink_metadata().map_err(storage)?;
                if metadata.is_dir() { pending.push(entry.path()); }
                else if metadata.is_file() { bytes = bytes.saturating_add(metadata.len()); }
            }
        }
        if cache.len() >= 16 { cache.clear(); }
        cache.insert(root, (std::time::Instant::now(), bytes));
        Ok(bytes)
    }).await.map_err(storage)?
}

/// Purging acquired evidence also removes automatically derived tables. Manually imported
/// independent snapshots keep their existing ownership. Called inside the caller's transaction.
///
/// The `analytical_snapshots` row itself is not deleted here: DuckDB resolves foreign keys
/// through an index whose entries are not released until the referencing delete commits, so
/// deleting a parent in the same transaction that emptied its children fails with "still
/// referenced by a foreign key in a different table" even though no child row will survive.
/// This is the same constraint limitation CLAUDE.md records for `ADD CONSTRAINT` and
/// `ALTER COLUMN SET NOT NULL`, and the same fix [`crate::storage::sqlite::DuckDbRepository::purge_source`]
/// already applies to the `portals` row. The caller commits this transaction, then deletes the
/// returned snapshot ids from `analytical_snapshots` in a second transaction.
pub(crate) fn purge_automatic_origins(
    conn: &duckdb::Connection,
    portal: &str,
    document: Option<i64>,
) -> Result<Vec<String>, AppError> {
    super::review::purge_review_references(conn, portal, document)?;
    super::monitoring::purge_monitoring_references(conn, portal, document)?;
    super::watchlists::purge_watchlist_references(conn, portal, document)?;
    super::casework::purge_casework_references(conn, portal, document.map(|id| id as u64))?;
    super::research::purge_research_references(conn, portal, document)?;
    let mut retired = Vec::new();
    let snapshots = {
        let mut query = conn.prepare("SELECT DISTINCT snapshot_id FROM data_maintenance_origins WHERE portal_key = ?1 AND (?2 IS NULL OR document_id = ?2)").map_err(storage)?;
        query
            .query_map(params![portal, document], |row| row.get::<_, String>(0))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?
    };
    conn.execute("DELETE FROM data_maintenance_origins WHERE portal_key = ?1 AND (?2 IS NULL OR document_id = ?2)", params![portal, document]).map_err(storage)?;
    for snapshot in snapshots {
        let others: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM data_maintenance_origins WHERE snapshot_id = ?1",
                params![snapshot],
                |row| row.get(0),
            )
            .map_err(storage)?;
        if others > 0 {
            continue;
        }
        let table: Option<String> = conn
            .query_row(
                "SELECT table_name FROM analytical_snapshots WHERE snapshot_id = ?1",
                params![snapshot],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage)?;
        if let Some(table) = table {
            let views = {
                let mut query = conn.prepare("SELECT view_name FROM duckdb_views() WHERE schema_name = 'main' AND starts_with(view_name, 'analysis_') AND contains(sql, ?1)").map_err(storage)?;
                query
                    .query_map(params![table], |row| row.get::<_, String>(0))
                    .map_err(storage)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(storage)?
            };
            let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
            for view in views {
                conn.execute(&format!("DROP VIEW {}", quote(&view)), [])
                    .map_err(storage)?;
            }
            conn.execute(&format!("DROP TABLE IF EXISTS {}", quote(&table)), [])
                .map_err(storage)?;
        }
        for table in ["analytical_rows", "analytical_columns"] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE snapshot_id = ?1"),
                params![snapshot],
            )
            .map_err(storage)?;
        }
        retired.push(snapshot);
    }
    Ok(retired)
}

fn checked_path(root: &Path, path: &str, expected_hash: &str) -> Result<PathBuf, AppError> {
    let path = Path::new(path);
    if std::fs::symlink_metadata(path)
        .map_err(storage)?
        .file_type()
        .is_symlink()
    {
        return Err(storage("Refusing a symbolic-link retention target"));
    }
    let canonical = path.canonicalize().map_err(storage)?;
    if !canonical.starts_with(root.canonicalize().map_err(storage)?) || !canonical.is_file() {
        return Err(storage("Retention target must be a managed regular file"));
    }
    if !sha256_file(&canonical)
        .map_err(storage)?
        .eq_ignore_ascii_case(expected_hash)
    {
        return Err(storage("Original digest changed; retention stopped"));
    }
    Ok(canonical)
}

fn recover_retention(conn: &mut duckdb::Connection, root: &Path) -> Result<bool, AppError> {
    let receipt = conn.query_row("SELECT receipt_id, original_path, pending_path, source_sha256, state FROM data_retention_receipts WHERE state IN ('PREPARED', 'REFERENCED') ORDER BY recorded_at LIMIT 1", [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?))).optional().map_err(storage)?;
    let Some((id, original, pending, hash, state)) = receipt else {
        return Ok(false);
    };
    if state == "PREPARED" {
        if Path::new(&pending).exists() {
            let safe = checked_path(root, &pending, &hash)?;
            if Path::new(&original).exists() {
                checked_path(root, &original, &hash)?;
                std::fs::remove_file(safe).map_err(storage)?;
            } else {
                let parent = Path::new(&original)
                    .parent()
                    .ok_or_else(|| storage("Original has no parent"))?
                    .canonicalize()
                    .map_err(storage)?;
                if !parent.starts_with(root.canonicalize().map_err(storage)?) {
                    return Err(storage("Unsafe recovery path"));
                }
                std::fs::rename(safe, &original).map_err(storage)?;
            }
        }
        checked_path(root, &original, &hash)?;
        conn.execute(
            "UPDATE data_retention_receipts SET state = 'RESTORED' WHERE receipt_id = ?1",
            params![id],
        )
        .map_err(storage)?;
    } else {
        if Path::new(&pending).exists() {
            let safe = checked_path(root, &pending, &hash)?;
            std::fs::remove_file(safe).map_err(storage)?;
        }
        conn.execute(
            "UPDATE data_retention_receipts SET state = 'RECLAIMED' WHERE receipt_id = ?1",
            params![id],
        )
        .map_err(storage)?;
    }
    Ok(true)
}

/// Publisher-owned permanent identifiers are allowed; arbitrary signed/session query strings
/// remain ineligible for automatic deletion of the original.
fn stable_public_reference(url: &reqwest::Url) -> bool {
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }
    if url.query().is_none_or(str::is_empty) {
        return true;
    }
    if url.scheme() != "https"
        || url.host_str() != Some("sandiego.hylandcloud.com")
        || url.port_or_known_default() != Some(443)
        || !url
            .path()
            .starts_with("/211agendaonlinecouncil/Documents/DownloadFile/")
    {
        return false;
    }
    let pairs: Vec<_> = url.query_pairs().collect();
    if pairs.len() != 6 {
        return false;
    }
    let values: std::collections::BTreeMap<_, _> = pairs
        .iter()
        .map(|(k, v)| (k.as_ref(), v.as_ref()))
        .collect();
    if values.len() != 6
        || values.get("documentType") != Some(&"1")
        || values.get("isSection") != Some(&"False")
        || values.get("isAttachment") != Some(&"True")
    {
        return false;
    }
    ["meetingId", "itemId", "publishId"].iter().all(|key| {
        values.get(key).is_some_and(|value| {
            !value.is_empty() && value.len() <= 20 && value.bytes().all(|b| b.is_ascii_digit())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn official_agenda_identifiers_are_stable_but_tokens_and_lookalikes_are_not() {
        let stable = "https://sandiego.hylandcloud.com/211agendaonlinecouncil/Documents/DownloadFile/Grant.pdf?documentType=1&meetingId=7192&itemId=265583&publishId=1130922&isSection=False&isAttachment=True";
        assert!(stable_public_reference(
            &reqwest::Url::parse(stable).unwrap()
        ));
        for value in [
            format!("{stable}&token=secret"),
            stable.replace(
                "sandiego.hylandcloud.com",
                "sandiego.hylandcloud.com.example.org",
            ),
            stable.replace("https:", "http:"),
            stable.replace("meetingId=7192", "meetingId=abc"),
            stable.replace("publishId=1130922", "itemId=1130922"),
        ] {
            assert!(
                !stable_public_reference(&reqwest::Url::parse(&value).unwrap()),
                "{value}"
            );
        }
    }
    use crate::{
        storage::sqlite::DocumentDownloadFinalization,
        types::{PortalRegistration, PublicRequest},
    };
    use serde_json::json;

    async fn setup() -> (tempfile::TempDir, DuckDbRepository) {
        let root = tempfile::tempdir().unwrap();
        let db = DuckDbRepository::new(root.path().join("data.db")).unwrap();
        db.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let portal: PortalRegistration = serde_json::from_value(json!({"portalKey":"fixture", "displayName":"Public fixture", "canonicalUrl":"https://example.test", "originUrl":"https://example.test", "verifiedAt":"2026-09-13T00:00:00Z", "sourceKind":"WEB_CRAWL", "apiLayout":{"requestsEndpoint":"https://example.test", "requestsPointer":"/resources", "pageParameter":"page", "pageSizeParameter":"limit"}})).unwrap();
        db.insert_or_update_portal_registration(&portal)
            .await
            .unwrap();
        db.maintenance_status().await.unwrap();
        (root, db)
    }
    async fn seed(
        db: &DuckDbRepository,
        root: &Path,
        id: u64,
        name: &str,
        bytes: &[u8],
    ) -> PathBuf {
        let path = root.join(format!("{id}-{name}"));
        std::fs::write(&path, bytes).unwrap();
        let record: PublicRequest = serde_json::from_value(json!({"id":id, "title":"Public fixture", "status":"open", "created_at":"2026-09-13T00:00:00Z", "updated_at":"2026-09-13T00:00:00Z", "documents":[{"id":id,"file_name":name,"download_url":format!("https://example.test/{id}/{name}"), "visibility":"public", "created_at":"2026-09-13T00:00:00Z", "updated_at":"2026-09-13T00:00:00Z"}]})).unwrap();
        db.insert_or_update_request("fixture", &record)
            .await
            .unwrap();
        db.mark_document_downloaded(DocumentDownloadFinalization {
            document_id: id,
            portal_subdomain: "fixture",
            raw_file_sha256: &sha256_hex(bytes),
            raw_file_size: bytes.len() as u64,
            local_path: &path,
            shannon_entropy: None,
            entropy_sample_bytes: 0,
            suspicious_data_density: false,
        })
        .await
        .unwrap();
        let conn = db.analytics_connection().await.unwrap();
        let text = String::from_utf8_lossy(bytes).into_owned();
        conn.execute("INSERT INTO documents_fts(document_id, portal_subdomain, file_name, extracted_text) VALUES (?1, 'fixture', ?2, ?3)", params![id, name, text]).unwrap();
        conn.execute("UPDATE documents SET extraction_status = 'SEARCHABLE', extracted_text_projection_sha256 = ?2 WHERE id = ?1 AND portal_subdomain = 'fixture'", params![id, sha256_hex(text.as_bytes())]).unwrap();
        path
    }
    async fn references(db: &DuckDbRepository) -> DataMaintenanceStatus {
        db.save_maintenance_policy(DataMaintenancePolicy {
            retention_mode: "REFERENCE_ONLY".into(),
            ..Default::default()
        })
        .await
        .unwrap()
    }

    async fn seed_finance_export(
        db: &DuckDbRepository,
        root: &Path,
        id: u64,
        url: &str,
        bytes: &[u8],
    ) -> PathBuf {
        let path = seed(db, root, id, &format!("finance-{id}.csv"), bytes).await;
        let url = url.to_string();
        db.query_connection("Set acquired finance publication URL", move |conn| {
            conn.execute(
                "UPDATE documents SET download_url=?2 WHERE id=?1 AND portal_subdomain='fixture'",
                params![id, url],
            )
            .map_err(storage)?;
            Ok(())
        })
        .await
        .unwrap();
        path
    }

    #[tokio::test]
    async fn automatic_finance_import_enables_cited_comparisons_only_for_complete_published_csv() {
        let (root, db) = setup().await;
        let header = "report_fy,fund_number,account_number,dept_name,funds_center_number,budget_cycle,amount\n";
        let budget =
            format!("{header}2023,100000,512059,Homelessness Strategies,1716000011,Adopted,100\n");
        let actual =
            format!("{header}2023,100000,512059,Homelessness Strategies,1716000011,Actual,70\n");
        for (id, url, bytes) in [
            (1, CITY_FINANCE_EXPORTS[0], budget.as_bytes()),
            (2, CITY_FINANCE_EXPORTS[1], actual.as_bytes()),
        ] {
            seed_finance_export(&db, root.path(), id, url, bytes).await;
            assert!(db.maintain_one_table().await.unwrap());
        }
        let schema = db.analytical_schema().await.unwrap();
        assert_eq!(schema.datasets.len(), 2);
        for dataset in &schema.datasets {
            assert!(!dataset.partial_dataset);
            assert_eq!(
                dataset.source_sha256,
                sha256_hex(
                    if dataset.source_uri.as_deref() == Some(CITY_FINANCE_EXPORTS[0]) {
                        budget.as_bytes()
                    } else {
                        actual.as_bytes()
                    }
                )
            );
            assert!(dataset
                .validation_summary
                .contains("COMPLETE_PUBLISHED_EXPORT"));
            assert!(dataset.validation_summary.contains("not all City costs"));
            assert!(
                dataset.observation_period_start.is_none()
                    && dataset.observation_period_end.is_none()
            );
        }
        // Exercise the real automatic mapping and deterministic analysis, without manual import.
        let workspace = db.financial_workspace().await.unwrap();
        assert_eq!(workspace.mappings.len(), 2);
        let report = db
            .run_financial_investigation(workspace.default_scope)
            .await
            .unwrap();
        assert!(
            report
                .reconciliation
                .iter()
                .any(|r| r.adopted.as_deref() == Some("100.00")
                    && r.actual.as_deref() == Some("70.00")
                    && !r.citations.is_empty()),
            "Automatic collection must actually reach cited financial comparisons: {:?}",
            report.reconciliation
        );
        assert!(!db.maintain_one_table().await.unwrap());

        let unknown = "https://example.test/operating_budget/budget_operating_datasd.csv";
        let paginated_url = format!("{}?page=1", CITY_FINANCE_EXPORTS[0]);
        let cases = [
            (3, unknown, b"city,count\nVista,3\n".as_slice()),
            (4, paginated_url.as_str(), b"city,count\nVista,4\n".as_slice()),
            (5, CITY_FINANCE_EXPORTS[0], br#"{"exceededTransferLimit":true,"features":[{"attributes":{"city":"Vista","count":5}}]}"#.as_slice()),
            (6, CITY_FINANCE_EXPORTS[1], b"city,count\nVista,6\nBad,7,unexpected\n".as_slice()),
            (7, CITY_FINANCE_EXPORTS[0], br#"[{"city":"Vista","count":7}]"#.as_slice()),
        ];
        for (id, url, bytes) in cases {
            seed_finance_export(&db, root.path(), id, url, bytes).await;
            assert!(db.maintain_one_table().await.unwrap());
            let dataset = db
                .analytical_schema()
                .await
                .unwrap()
                .datasets
                .into_iter()
                .find(|d| d.display_name == format!("finance-{id}.csv"))
                .unwrap();
            assert!(dataset.partial_dataset, "Unknown, paginated, rejected or non-CSV projection {id} cannot support complete totals");
            assert!(!dataset
                .validation_summary
                .contains("COMPLETE_PUBLISHED_EXPORT"));
            if id == 5 {
                assert!(dataset
                    .validation_summary
                    .contains("\"sourceDeclaresPartial\":true"));
            }
            if id == 6 {
                assert_eq!(dataset.rejected_rows, 1);
            }
        }
        let changed = seed_finance_export(
            &db,
            root.path(),
            8,
            CITY_FINANCE_EXPORTS[0],
            b"city,count\nVista,8\n",
        )
        .await;
        std::fs::write(changed, b"city,count\nVista,999\n").unwrap();
        assert!(db.maintain_one_table().await.unwrap());
        assert!(!db
            .analytical_schema()
            .await
            .unwrap()
            .datasets
            .iter()
            .any(|d| d.display_name == "finance-8.csv"));
        assert!(db
            .maintenance_status()
            .await
            .unwrap()
            .events
            .iter()
            .any(|e| e.action == "IMPORT_NEEDS_REVIEW" && e.detail.contains("SHA-256")));
    }

    #[tokio::test]
    async fn automatic_finance_coverage_reconciles_legacy_origins_but_preserves_manual_and_unverified_partial(
    ) {
        let (root, db) = setup().await;
        let mut snapshots = Vec::new();
        for id in 1..=6 {
            let bytes = format!("city,count\nVista,{id}\n");
            let path = seed_finance_export(
                &db,
                root.path(),
                id,
                CITY_FINANCE_EXPORTS[0],
                bytes.as_bytes(),
            )
            .await;
            let result = db
                .import_analytical_csv(AnalyticalImportRequest {
                    dataset_kind: crate::types::AnalyticalDatasetKind::RegionalData,
                    display_name: format!("legacy-{id}"),
                    source_uri: Some(CITY_FINANCE_EXPORTS[0].into()),
                    local_path: path.to_string_lossy().into_owned(),
                    observation_period_start: None,
                    observation_period_end: None,
                    partial_dataset: true,
                    worksheet: None,
                    source_file_name: Some(format!("finance-{id}.csv")),
                    expected_source_sha256: Some(sha256_hex(bytes.as_bytes())),
                })
                .await
                .unwrap();
            let snapshot = result.snapshot_id.clone();
            db.query_connection("Reproduce legacy automatic import metadata", move |conn| {
                if id!=2 { conn.execute("INSERT INTO data_maintenance_origins VALUES(?1,'fixture',?2)",params![snapshot,id]).map_err(storage)?; }
                match id {
                    3 => { conn.execute("UPDATE documents SET raw_file_sha256=repeat('0',64) WHERE id=?1",[id]).map_err(storage)?; },
                    4..=6 => {
                        let raw:String=conn.query_row("SELECT validation_summary_json FROM analytical_snapshots WHERE snapshot_id=?1",[&snapshot],|row|row.get(0)).map_err(storage)?;
                        let mut validation:serde_json::Value=serde_json::from_str(&raw).map_err(storage)?;
                        if id==4 { validation["parserVersion"]=json!("old-parser"); }
                        if id==5 { validation["sourceDeclaresPartial"]=json!(true); }
                        if id==6 { validation["sourceFormat"]=json!("Socrata JSON"); }
                        conn.execute("UPDATE analytical_snapshots SET validation_summary_json=?2 WHERE snapshot_id=?1",params![snapshot,validation.to_string()]).map_err(storage)?;
                    }, _=>{}
                }
                // Legacy originals may already have been compacted. A verified projection
                // and matching acquisition receipt remain sufficient for file coverage.
                conn.execute("UPDATE documents SET local_path=NULL WHERE id=?1",[id]).map_err(storage)?;
                Ok(())
            }).await.unwrap();
            snapshots.push(result.snapshot_id);
        }
        assert!(db.maintain_one_table().await.unwrap());
        let schema = db.analytical_schema().await.unwrap();
        for (index, snapshot) in snapshots.iter().enumerate() {
            let dataset = schema
                .datasets
                .iter()
                .find(|d| &d.snapshot_id == snapshot)
                .unwrap();
            assert_eq!(dataset.partial_dataset,index!=0,"Only a current, complete, automatic projection with unchanged source identity is eligible: {index}");
        }
        assert!(!db.maintain_one_table().await.unwrap());
        assert_eq!(
            db.maintenance_status()
                .await
                .unwrap()
                .events
                .iter()
                .filter(|e| e.action == "COVERAGE_VERIFIED")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn automatic_import_repairs_audits_and_does_not_retry_unchanged_snapshots() {
        let (root, db) = setup().await;
        seed(
            &db,
            root.path(),
            1,
            "counts.csv",
            b"city;count\n\"San Diego; Central\";3; \nVista;4\nBad;5;unexplained\n",
        )
        .await;
        assert!(db.maintain_one_table().await.unwrap());
        let schema = db.analytical_schema().await.unwrap();
        let dataset = &schema.datasets[0];
        assert_eq!((dataset.imported_rows, dataset.rejected_rows), (2, 1));
        assert!(dataset.validation_summary.contains("emptyTrailingFields"));
        assert!(dataset.validation_summary.contains("semicolon"));
        let rows = db.execute_restricted_sql(&format!("SELECT source_payload_json FROM analytical_rows WHERE snapshot_id = '{}' AND row_number = 1", dataset.snapshot_id), 10).await.unwrap();
        assert!(rows.rows[0][0]
            .as_str()
            .unwrap()
            .contains("San Diego; Central"));
        assert!(rows.rows[0][0].as_str().unwrap().contains("\" \""));
        assert!(!db.maintain_one_table().await.unwrap());
    }

    #[tokio::test]
    async fn bad_import_backs_off_without_starving_other_files() {
        let (root, db) = setup().await;
        seed(
            &db,
            root.path(),
            1,
            "broken.csv",
            b"<html>Unavailable</html>",
        )
        .await;
        seed(&db, root.path(), 2, "good.csv", b"city,count\nVista,3").await;
        assert!(db.maintain_one_table().await.unwrap());
        assert!(db.maintain_one_table().await.unwrap());
        assert!(!db.maintain_one_table().await.unwrap());
        assert_eq!(db.analytical_schema().await.unwrap().datasets.len(), 1);
        assert!(db
            .maintenance_status()
            .await
            .unwrap()
            .events
            .iter()
            .any(|event| event.action == "IMPORT_NEEDS_REVIEW"));
    }

    #[tokio::test]
    async fn legacy_empty_query_urls_can_release_verified_originals() {
        let (root, db) = setup().await;
        let policy = references(&db).await;
        let file = seed(
            &db,
            root.path(),
            7,
            "legacy.txt",
            b"Public records remain searchable after compaction.",
        )
        .await;
        db.query_connection("Legacy empty query fixture", |conn| {
            conn.execute(
                "UPDATE documents SET download_url = download_url || '?' WHERE id = 7",
                [],
            )
            .map_err(storage)?;
            Ok(())
        })
        .await
        .unwrap();
        assert!(db
            .compact_one_document(root.path(), &policy.effective_at)
            .await
            .unwrap());
        assert!(!file.exists());
        assert_eq!(
            db.maintenance_status().await.unwrap().referenced_documents,
            1
        );
        assert!(db
            .load_document_text_chunk(7, "fixture", 0, 1000)
            .await
            .unwrap()
            .masked_text
            .contains("remain searchable"));
    }

    #[tokio::test]
    async fn reference_policy_only_retires_new_verified_acquisitions() {
        let (root, db) = setup().await;
        let old = seed(
            &db,
            root.path(),
            1,
            "old.txt",
            b"Existing evidence remains available.",
        )
        .await;
        let policy = references(&db).await;
        let new = seed(
            &db,
            root.path(),
            2,
            "new.txt",
            b"Public community findings.",
        )
        .await;
        assert!(db
            .compact_one_document(root.path(), &policy.effective_at)
            .await
            .unwrap());
        assert!(old.exists());
        assert!(!new.exists());
        let status = db.maintenance_status().await.unwrap();
        assert_eq!(status.referenced_documents, 1);
        assert_eq!(
            status.reclaimed_file_bytes,
            b"Public community findings.".len() as u64
        );
        let documents = db
            .list_request_documents_for_view(2, "fixture")
            .await
            .unwrap();
        assert!(documents[0]
            .extraction_deferred_reason
            .as_ref()
            .unwrap()
            .starts_with("REFERENCE_ONLY:"));
        assert_eq!(
            documents[0].original_source_uri.as_deref(),
            Some("https://example.test/2/new.txt")
        );
        let chunk = db
            .load_document_text_chunk(2, "fixture", 0, 1000)
            .await
            .unwrap();
        assert!(chunk.masked_text.contains("Public community findings"));
        assert!(!db
            .compact_one_document(root.path(), &policy.effective_at)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn tabular_reference_requires_a_complete_validated_projection() {
        let (root, db) = setup().await;
        let policy = references(&db).await;
        let good = seed(&db, root.path(), 1, "complete.csv", b"city,count\nVista,3").await;
        let bad = seed(
            &db,
            root.path(),
            2,
            "ragged.csv",
            b"city,count\nVista,3\nSan Diego,4,unknown",
        )
        .await;
        let partial = seed(&db, root.path(), 3, "page.json", br#"{"exceededTransferLimit":true,"features":[{"attributes":{"city":"Vista","count":3}}]}"#).await;
        assert!(!db
            .compact_one_document(root.path(), &policy.effective_at)
            .await
            .unwrap());
        for _ in 0..3 {
            assert!(db.maintain_one_table().await.unwrap());
        }
        assert!(db
            .compact_one_document(root.path(), &policy.effective_at)
            .await
            .unwrap());
        assert!(!good.exists());
        assert!(bad.exists());
        assert!(partial.exists());
        assert!(!db
            .compact_one_document(root.path(), &policy.effective_at)
            .await
            .unwrap());
        let schema = db.analytical_schema().await.unwrap();
        let dataset = schema
            .datasets
            .iter()
            .find(|s| s.display_name == "complete.csv")
            .unwrap();
        assert_eq!(
            db.execute_restricted_sql(&format!("SELECT COUNT(*) FROM {}", dataset.table_name), 10)
                .await
                .unwrap()
                .rows[0][0],
            json!(1)
        );
    }

    #[tokio::test]
    async fn unsafe_or_incomplete_originals_are_kept_and_do_not_starve_eligible_files() {
        let (root, db) = setup().await;
        let policy = references(&db).await;
        let changed = seed(
            &db,
            root.path(),
            1,
            "changed.txt",
            b"This original changes after acquisition.",
        )
        .await;
        std::fs::write(&changed, b"different bytes").unwrap();
        let signed = seed(
            &db,
            root.path(),
            2,
            "signed.txt",
            b"Signed original must remain locally.",
        )
        .await;
        let salvaged = seed(&db, root.path(), 3, "salvaged.txt", b"Partial extraction.").await;
        let corrupt_text = seed(
            &db,
            root.path(),
            4,
            "projection.txt",
            b"Text whose projection is corrupt.",
        )
        .await;
        let good = seed(&db, root.path(), 5, "good.txt", b"Safe public text.").await;
        let conn = db.analytics_connection().await.unwrap();
        conn.execute("UPDATE documents SET download_url = 'https://example.test/file?token=expiring' WHERE id = 2", []).unwrap();
        conn.execute(
            "UPDATE documents SET extraction_status = 'SALVAGED' WHERE id = 3",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE documents_fts SET extracted_text = 'corrupted' WHERE document_id = 4",
            [],
        )
        .unwrap();
        for _ in 0..3 {
            assert!(db
                .compact_one_document(root.path(), &policy.effective_at)
                .await
                .unwrap());
        }
        assert!(changed.exists() && signed.exists() && salvaged.exists() && corrupt_text.exists());
        assert!(!good.exists());
        assert!(!db
            .compact_one_document(root.path(), &policy.effective_at)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn retention_recovery_restores_uncommitted_files_and_finishes_committed_deletions() {
        let (root, db) = setup().await;
        references(&db).await;
        let original = seed(&db, root.path(), 1, "recover.txt", b"Recoverable evidence.").await;
        let pending = root.path().join(".retire-recovery");
        let conn = db.analytics_connection().await.unwrap();
        conn.execute("INSERT INTO data_retention_receipts VALUES ('recovery', 'fixture', 1, ?1, ?2, 'https://example.test/recover.txt', ?3, 21, 'PREPARED', ?4)", params![original.to_string_lossy(), pending.to_string_lossy(), sha256_hex(b"Recoverable evidence."), now()]).unwrap();
        std::fs::rename(&original, &pending).unwrap();
        assert!(db.recover_maintenance_storage(root.path()).await.unwrap());
        assert!(original.exists());
        assert!(!pending.exists());
        std::fs::rename(&original, &pending).unwrap();
        conn.execute(
            "UPDATE data_retention_receipts SET state = 'REFERENCED'",
            [],
        )
        .unwrap();
        conn.execute("UPDATE documents SET local_path = NULL, extraction_deferred_reason = 'REFERENCE_ONLY: fixture' WHERE id = 1", []).unwrap();
        assert!(db.recover_maintenance_storage(root.path()).await.unwrap());
        assert!(!original.exists());
        assert!(!pending.exists());
        assert!(!db.recover_maintenance_storage(root.path()).await.unwrap());
    }

    #[tokio::test]
    async fn normal_extraction_finalization_marks_new_downloads_for_retention() {
        let (root, db) = setup().await;
        let policy = references(&db).await;
        let text = "Published public evidence.";
        let path = seed(&db, root.path(), 1, "extracted.txt", text.as_bytes()).await;
        // The ordinary SEARCHABLE route finalizes directly, without mark_document_downloaded.
        db.analytics_connection().await.unwrap().execute("UPDATE documents SET retention_eligible_at = NULL, raw_file_sha256 = NULL, download_status = 'pending' WHERE id = 1", []).unwrap();
        db.finalize_document_extraction(crate::storage::sqlite::DocumentExtractionFinalization {
            document_id: 1,
            portal_subdomain: "fixture",
            extracted_text: text,
            extracted_text_sha256: &sha256_hex(text.as_bytes()),
            raw_file_sha256: &sha256_hex(text.as_bytes()),
            was_ocr_processed: false,
            raw_file_size: text.len() as u64,
            local_path: &path,
            shannon_entropy: None,
            entropy_sample_bytes: 0,
            suspicious_data_density: false,
        })
        .await
        .unwrap();
        assert!(db
            .compact_one_document(root.path(), &policy.effective_at)
            .await
            .unwrap());
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn purging_acquisitions_removes_automatic_tables_jobs_and_retention_references() {
        let (root, db) = setup().await;
        let policy = references(&db).await;
        seed(&db, root.path(), 1, "counts.csv", b"city,count\nVista,3").await;
        db.maintain_one_table().await.unwrap();
        let dataset = db.analytical_schema().await.unwrap().datasets.remove(0);
        db.compact_one_document(root.path(), &policy.effective_at)
            .await
            .unwrap();
        db.purge_document("fixture", 1).await.unwrap();
        assert!(db.analytical_schema().await.unwrap().datasets.is_empty());
        assert!(db
            .execute_restricted_sql(&format!("SELECT * FROM {}", dataset.table_name), 10)
            .await
            .is_err());
        let status = db.maintenance_status().await.unwrap();
        assert_eq!(status.referenced_documents, 0);
        assert!(status.events.is_empty());
        seed(&db, root.path(), 2, "more.csv", b"city,count\nVista,5").await;
        db.maintain_one_table().await.unwrap();
        db.compact_one_document(root.path(), &policy.effective_at)
            .await
            .unwrap();
        db.purge_source("fixture").await.unwrap();
        assert!(db.analytical_schema().await.unwrap().datasets.is_empty());
        let conn = db.analytics_connection().await.unwrap();
        for table in [
            "data_maintenance_origins",
            "data_retention_receipts",
            "data_maintenance_jobs",
        ] {
            assert_eq!(
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }

    #[tokio::test]
    async fn inventory_counts_old_versions_and_unindexed_managed_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("objects")).unwrap();
        std::fs::write(root.path().join("objects/old.bin"), vec![7; 4096]).unwrap();
        std::fs::write(root.path().join(".retire-pending"), vec![9; 2048]).unwrap();
        assert_eq!(managed_file_bytes(root.path()).await.unwrap(), 6144);
    }

    #[tokio::test]
    async fn retention_policy_validates_budget_and_refresh_leases() {
        let (root, db) = setup().await;
        assert!(db
            .save_maintenance_policy(DataMaintenancePolicy {
                refresh_hours: 0,
                ..Default::default()
            })
            .await
            .is_err());
        seed(&db, root.path(), 1, "large.txt", b"Fixture").await;
        let policy = DataMaintenancePolicy {
            retained_file_budget_bytes: 100 * 1024 * 1024,
            ..Default::default()
        };
        db.save_maintenance_policy(policy).await.unwrap();
        db.check_retained_file_budget(root.path()).await.unwrap();
        db.analytics_connection()
            .await
            .unwrap()
            .execute("UPDATE documents SET file_size = 104857600", [])
            .unwrap();
        assert!(matches!(
            db.check_retained_file_budget(root.path())
                .await
                .unwrap_err(),
            AppError::PortalCapacity(_)
        ));
        assert!(db.source_refresh_due("fixture", 24).await.unwrap());
        db.record_maintenance(
            "refresh:fixture".into(),
            "REFRESH_STARTED",
            "Fixture".into(),
            24,
        )
        .await
        .unwrap();
        assert!(!db.source_refresh_due("fixture", 24).await.unwrap());
    }
}
