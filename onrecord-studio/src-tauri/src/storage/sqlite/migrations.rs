//! Schema migration, upgrade shims for older cores, and the startup relation audit.

use super::*;
use super::{integrity::normalize_sha256, search::count_pending_search_index_documents};

impl DuckDbRepository {
    /// Run embedded raw text scripts to establish layout boundaries.
    pub async fn run_migrations(&self, migration_sql: &'static str) -> Result<(), AppError> {
        let pending = self
            .query_connection("Run schema migrations", move |conn| {
                Self::run_migrations_sync(conn, migration_sql)
            })
            .await?;
        self.search_index_pending.store(pending, Ordering::Relaxed);
        Ok(())
    }

    fn run_migrations_sync(conn: &mut Connection, migration_sql: &str) -> Result<u64, AppError> {
        // Legacy repair must precede the idempotent base script; `ensure_column` calls that follow
        // are compatibility shims for databases created by older, partially versioned builds.
        prepare_legacy_legal_schema(conn)?;
        reject_narrow_identifier_core(conn)?;
        conn.execute_batch(migration_sql).map_err(|e| {
            AppError::Internal(format!(
                "Failed to execute initialization migrations: {}",
                e
            ))
        })?;
        // Every definition below is a type plus a DEFAULT and nothing else, and it has to stay
        // that way. Measured against the linked DuckDB rather than assumed, in
        // `duckdb_add_column_takes_a_default_but_no_constraint`: `ALTER TABLE ... ADD COLUMN`
        // refuses NOT NULL and CHECK with "Adding columns with constraints not yet supported",
        // `ALTER TABLE ... ADD CONSTRAINT` is unimplemented, and `ALTER COLUMN ... SET NOT NULL`
        // is refused on any table an index depends on. So there is no order of statements that
        // reaches a constrained column on a table that already holds rows.
        //
        // A constrained definition here parses on a fresh database, where the base schema already
        // created the column and this call is a no-op, and aborts startup on exactly the older
        // installations the shim exists to upgrade. That is why the failure was invisible.
        //
        // What readers see is the DEFAULT, and ADD COLUMN backfills existing rows with it, so an
        // upgraded database holds the same values as a fresh one. What it does not hold is the
        // NOT NULL and CHECK that `0001_initial_schema.sql` declares. Those stay there because
        // they are live protection on a fresh install; deleting them so the two paths match would
        // trade a real guarantee for symmetry.
        ensure_column(
            conn,
            "document_search_index_state",
            "indexed_characters",
            "BIGINT",
        )?;
        ensure_column(conn, "portals", "origin_url", "TEXT DEFAULT ''")?;
        ensure_column(conn, "portals", "requests_endpoint", "TEXT DEFAULT ''")?;
        ensure_column(
            conn,
            "portals",
            "requests_pointer",
            "TEXT DEFAULT '/requests'",
        )?;
        ensure_column(conn, "portals", "metadata_pointer", "TEXT")?;
        ensure_column(conn, "portals", "page_parameter", "TEXT DEFAULT 'page'")?;
        ensure_column(
            conn,
            "portals",
            "page_size_parameter",
            "TEXT DEFAULT 'per_page'",
        )?;
        ensure_column(conn, "portals", "verified_at", "TEXT")?;
        ensure_column(conn, "portals", "health_status", "TEXT DEFAULT 'verified'")?;
        ensure_column(conn, "portals", "last_error", "TEXT")?;
        ensure_column(
            conn,
            "portals",
            "source_kind",
            "TEXT DEFAULT 'NEXT_REQUEST'",
        )?;
        // A source that predates the lifecycle column really was never paused or removed, so
        // 'ACTIVE' is the fact about it rather than a stand-in for one that was never recorded.
        // Readers still coalesce, because DuckDB's own backfill for an added column is what this
        // depends on and it is not worth trusting silently.
        ensure_column(conn, "portals", "lifecycle_state", "TEXT DEFAULT 'ACTIVE'")?;
        ensure_column(
            conn,
            "source_connector_recipes",
            "publisher_pattern",
            "TEXT",
        )?;
        // No DEFAULT, for the reason the fresh schema gives: a recipe written before this column
        // recorded no submitted URL, and no string stands in for one. It cannot be backfilled
        // from `root_url` either -- that is the URL *after* redirects, so copying it would assert
        // the submission did not redirect, which is the one case this column exists to record.
        ensure_column(conn, "source_connector_recipes", "submitted_url", "TEXT")?;
        ensure_column(
            conn,
            "source_connector_recipes",
            "reuse_approved",
            "BIGINT DEFAULT 0",
        )?;
        // No DEFAULT: a recipe written before this column existed recorded no window, and NULL is
        // the only honest way to say that. 1 and 50 are legal windows, so defaulting to them would
        // make "never chosen" indistinguishable from "chosen to be the same as the built-in".
        ensure_column(
            conn,
            "source_connector_recipes",
            "metadata_page_window_json",
            "TEXT",
        )?;
        // These admission counters bound an interrupted crawl across process restarts. Older runs
        // predate durable counting, so zero is the only evidence-backed backfill available.
        ensure_column(
            conn,
            "portal_sync_runs",
            "crawl_pages_started",
            "BIGINT DEFAULT 0",
        )?;
        // Older builds charged an attempt per claim, so a row queued across a few launches reached
        // the abandonment cap without ever running. Those counts measured nothing about the task,
        // so the replacement counter starts every existing row at zero executions.
        ensure_column(
            conn,
            "ingestion_task_frontier",
            "executions",
            "BIGINT DEFAULT 0",
        )?;
        ensure_column(
            conn,
            "portal_sync_runs",
            "resources_started",
            "BIGINT DEFAULT 0",
        )?;
        // Provenance results gained an explicit confidence classification. Rows written before it
        // existed cannot be reclassified after the fact -- the file they described may since have
        // changed -- so they keep a sentinel rather than being backfilled into a real state.
        //
        // The definitions carry a DEFAULT but no NOT NULL, and the base schema matches them,
        // because DuckDB rejects `ALTER TABLE ... ADD COLUMN` with any constraint attached
        // ("Adding columns with constraints not yet supported"). A constrained definition here
        // would parse fine on a fresh database, where the base schema already created the column
        // and this call is a no-op, and then fail at startup on exactly the installations the
        // shim exists to upgrade. Keeping both shapes identical means a fresh database and an
        // upgraded one are the same database.
        for column in [
            "recorded_digest_source",
            "signature_verification",
            "provenance_confidence",
        ] {
            ensure_column(
                conn,
                "provenance_verification_log",
                column,
                "TEXT DEFAULT 'UNCLASSIFIED_LEGACY_RECORD'",
            )?;
        }
        for (column, definition) in [
            ("text_projection_matches", "BOOLEAN"),
            ("evidence_file_present", "BOOLEAN"),
            ("assertion_count", "BIGINT"),
            ("signature_issuer", "TEXT"),
            ("signing_timestamp", "TEXT"),
            ("text_projection_sha256", "TEXT"),
        ] {
            ensure_column(conn, "provenance_verification_log", column, definition)?;
        }
        // Distinguishes two certificates that happen to share an issuer/organization name; nullable
        // with no default, per the absence-vs-measurement rule, since "not recorded" and "no serial
        // on this certificate" are different facts.
        ensure_column(
            conn,
            "provenance_manifests",
            "signing_certificate_serial_number",
            "TEXT",
        )?;
        ensure_column(
            conn,
            "provenance_verification_log",
            "signing_certificate_serial_number",
            "TEXT",
        )?;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_provenance_verification_document
             ON provenance_verification_log(document_id, portal_subdomain, verified_at DESC);",
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to index the provenance verification log: {error}"
            ))
        })?;
        // Nullable with no default: absence is meaningful. A document extracted before this
        // column existed, or a row imported from a legacy core, genuinely has no recorded digest
        // for its text projection, and a verification must say so rather than invent one.
        ensure_column(
            conn,
            "documents",
            "extracted_text_projection_sha256",
            "TEXT",
        )?;
        ensure_column(conn, "requests", "neighborhood_block", "TEXT")?;
        ensure_column(conn, "requests", "determination_at", "TEXT")?;
        ensure_column(conn, "requests", "determination_source", "TEXT")?;
        // Nullable on purpose — NULL means "no attribution recorded" and reads differently from a
        // filled string. The differential-treatment detector runs and emits nothing when no rows
        // carry attribution; population of this field belongs to ingestion adapters that can
        // actually derive it.
        ensure_column(conn, "requests", "requester_attribution", "TEXT")?;
        ensure_column(
            conn,
            "cross_correlation_alerts",
            "complaint_count",
            "BIGINT DEFAULT 0",
        )?;
        ensure_column(
            conn,
            "cross_correlation_alerts",
            "correlation_window_hours",
            "BIGINT DEFAULT 48",
        )?;
        ensure_column(conn, "documents", "raw_file_sha256", "TEXT")?;
        // Same rule as the portals block above. The base schema bounds these three; an upgraded
        // database carries the DEFAULT but not the range check.
        //
        // `shannon_entropy` takes no DEFAULT: 0.0 is a measurement -- every byte in the sample
        // identical -- not "never computed", and a document that predates this column was never
        // assessed. Backfilling it to 0.0 would state the strongest possible uniformity signal
        // about a file nothing ever read. NULL is representable and says exactly that.
        ensure_column(conn, "documents", "shannon_entropy", "REAL")?;
        // The other two keep DEFAULT 0. 0 sampled bytes genuinely means no bytes were sampled,
        // and 0 density is the correct default claim: nothing was flagged. Neither is a
        // measurement standing in for an absence.
        ensure_column(
            conn,
            "documents",
            "entropy_sample_bytes",
            "BIGINT DEFAULT 0",
        )?;
        ensure_column(
            conn,
            "documents",
            "suspicious_data_density",
            "BIGINT DEFAULT 0",
        )?;
        ensure_column(
            conn,
            "documents",
            "extraction_status",
            "TEXT DEFAULT 'PENDING'",
        )?;
        ensure_column(conn, "documents", "extraction_format", "TEXT")?;
        ensure_column(conn, "documents", "extraction_deferred_reason", "TEXT")?;
        ensure_column(conn, "documents", "retention_eligible_at", "TEXT")?;
        for column in [
            "package_path",
            "package_sha256",
            "signature_algorithm",
            "signer_key_id",
        ] {
            ensure_column(conn, "report_exports", column, "TEXT")?;
        }
        conn.execute_batch(
            "UPDATE documents
             SET extraction_status = CASE
                WHEN EXISTS (
                    SELECT 1 FROM documents_fts
                    WHERE documents_fts.document_id = documents.id
                      AND documents_fts.portal_subdomain = documents.portal_subdomain
                ) THEN 'SEARCHABLE'
                WHEN LOWER(COALESCE(download_status, 'pending')) = 'completed' THEN 'DOWNLOADED'
                WHEN LOWER(COALESCE(download_status, 'pending')) = 'failed' THEN 'FAILED'
                ELSE 'PENDING'
             END
             WHERE extraction_status IS NULL OR extraction_status = 'PENDING';",
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to backfill document extraction state: {error}"
            ))
        })?;
        backfill_legal_authority_versions(conn)?;
        migrate_archived_statutory_findings(conn)?;
        super::spans::backfill_finding_citation_spans(conn)?;
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_requests_neighborhood_time
             ON requests(neighborhood_block, created_at);
             CREATE INDEX IF NOT EXISTS idx_documents_request_density
             ON documents(request_id, portal_subdomain, suspicious_data_density);
             DROP INDEX IF EXISTS idx_statutory_violations_request;
             CREATE INDEX idx_statutory_violations_request
             ON statutory_violations(request_id, portal_subdomain, review_status);",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to finalize migration indexes: {error}"))
        })?;
        // Tokenizing every pre-index document is minutes of single-threaded work on a large
        // corpus, and this runs inside the Tauri setup hook, before the event loop presents the
        // window: doing it here means the application shows no window at all until it finishes.
        // The migration therefore only counts the arrears; `backfill_search_token_index` drains
        // them in the background, and a search refuses while the count is above zero.
        let pending = count_pending_search_index_documents(conn)?;
        verify_logical_relations(conn)?;
        Ok(pending)
    }
}

fn map_legacy_legal_authority_row(row: &Row<'_>) -> duckdb::Result<LegacyLegalAuthorityRow> {
    Ok(LegacyLegalAuthorityRow {
        statute_id: row.get(0)?,
        full_text: row.get(1)?,
        stored_sha256: row.get(2)?,
        stored_version_id: row.get(3)?,
        imported_at: row.get(4)?,
        effective_from: row.get(5)?,
        effective_to: row.get(6)?,
        source_uri: row.get(7)?,
    })
}

/// Audits relations kept logical because DuckDB cannot support the required parent updates with
/// physical foreign keys. Canonical evidence is never auto-deleted or guessed into a parent; the
/// operator receives exact relation/count pairs and can restore from the retained source data.
pub(super) fn verify_logical_relations(conn: &Connection) -> Result<(), AppError> {
    const CHECKS: [(&str, &str); 13] = [
        (
            "document_acquisitions -> documents",
            "SELECT COUNT(*) FROM document_acquisitions child
             LEFT JOIN documents parent ON parent.id = child.document_id
                AND parent.portal_subdomain = child.portal_subdomain
             WHERE parent.id IS NULL",
        ),
        (
            "document_source_checks -> documents",
            "SELECT COUNT(*) FROM document_source_checks child
             LEFT JOIN documents parent ON parent.id = child.document_id
                AND parent.portal_subdomain = child.portal_subdomain
             WHERE parent.id IS NULL",
        ),
        (
            "spans -> documents",
            "SELECT COUNT(*) FROM spans child
             LEFT JOIN documents parent ON parent.id = child.document_id
                AND parent.portal_subdomain = child.portal_subdomain
             WHERE parent.id IS NULL",
        ),
        (
            "statutory_extensions -> requests",
            "SELECT COUNT(*) FROM statutory_extensions child
             LEFT JOIN requests parent ON parent.id = child.request_id
                AND parent.portal_subdomain = child.portal_subdomain
             WHERE parent.id IS NULL",
        ),
        (
            "request_release_events -> requests",
            "SELECT COUNT(*) FROM request_release_events child
             LEFT JOIN requests parent ON parent.id = child.request_id
                AND parent.portal_subdomain = child.portal_subdomain
             WHERE parent.id IS NULL",
        ),
        (
            "request_keyword_signals -> requests",
            "SELECT COUNT(*) FROM request_keyword_signals child
             LEFT JOIN requests parent ON parent.id = child.request_id
                AND parent.portal_subdomain = child.portal_subdomain
             WHERE parent.id IS NULL",
        ),
        (
            "documents -> requests",
            "SELECT COUNT(*) FROM documents child
             LEFT JOIN requests parent ON parent.id = child.request_id
                AND parent.portal_subdomain = child.portal_subdomain
             WHERE parent.id IS NULL",
        ),
        (
            "case_audits -> requests",
            "SELECT COUNT(*) FROM case_audits child
             LEFT JOIN requests parent ON parent.id = child.target_request_id
                AND parent.portal_subdomain = child.target_subdomain
             WHERE parent.id IS NULL",
        ),
        (
            "document_integrity_flags -> case_audits",
            "SELECT COUNT(*) FROM document_integrity_flags child
             LEFT JOIN case_audits parent ON parent.audit_id = child.audit_id
             WHERE parent.audit_id IS NULL",
        ),
        (
            "documents_fts -> documents",
            "SELECT COUNT(*) FROM documents_fts child
             LEFT JOIN documents parent ON parent.id = child.document_id
                AND parent.portal_subdomain = child.portal_subdomain
             WHERE parent.id IS NULL",
        ),
        (
            "document_search_tokens -> documents_fts",
            "SELECT COUNT(*) FROM document_search_tokens child
             LEFT JOIN documents_fts parent ON parent.document_id = child.document_id
                AND parent.portal_subdomain = child.portal_subdomain
             WHERE parent.document_id IS NULL",
        ),
        (
            "document_search_index_state -> documents_fts",
            "SELECT COUNT(*) FROM document_search_index_state child
             LEFT JOIN documents_fts parent ON parent.document_id = child.document_id
                AND parent.portal_subdomain = child.portal_subdomain
             WHERE parent.document_id IS NULL",
        ),
        (
            "legal_provisions_fts -> legal_provisions",
            "SELECT COUNT(*) FROM legal_provisions_fts child
             LEFT JOIN legal_provisions parent ON parent.provision_id = child.provision_id
                AND parent.version_id = child.version_id
             WHERE parent.provision_id IS NULL",
        ),
    ];

    let mut failures = Vec::new();
    for (relation, query) in CHECKS {
        let count = conn
            .query_row(query, [], |row| row.get::<_, i64>(0))
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to audit logical relation '{relation}': {error}"
                ))
            })?;
        if count > 0 {
            failures.push(format!("{relation}: {count} orphan row(s)"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::Storage(format!(
            "Core consistency audit failed; no evidence was modified: {}",
            failures.join("; ")
        )))
    }
}

/// Checks the three entropy columns against each other before they are persisted.
///
/// `shannon_entropy` is `None` when entropy was never computed. That is only coherent with a
/// zero-byte sample: bytes that were read produced a score. The converse is not asserted --
/// `Some(0.0)` with zero sampled bytes is the real reading for an empty file, which
/// `calculate_shannon_entropy` scores 0.0 over an empty sample.
/// Refuses a core database whose identifier columns are 32 bits wide, with the reason.
///
/// DuckDB's `INTEGER` is 32-bit. This schema was written against SQLite, where it is 64-bit, so
/// every identifier column was declared `INTEGER` and silently capped at 2147483647. Portal
/// identifiers are not small: a real source produced request id 5614575451713354, and storing it
/// aborted the whole ingestion transaction with `Conversion Error: Type UINT64 with value
/// 5614575451713354 can't be cast because the value is out of range for the destination type
/// INT32`. `0001_initial_schema.sql` now declares every integer column `BIGINT`.
///
/// A database created before that cannot be repaired in place. Measured against the linked
/// library in `duckdb_refuses_to_widen_a_key_column`: `ALTER COLUMN ... TYPE` is refused on any
/// column carrying a PRIMARY KEY, UNIQUE, or FOREIGN KEY constraint, which is every identifier
/// column here. Running the widened schema against such a database fails when a child table's
/// `BIGINT` foreign key will not bind to a parent's `INTEGER` key, and that error reaches the
/// setup hook, which panics.
///
/// So it is caught here and named. The core is canonical evidence and is never deleted on the
/// application's own initiative; the operator decides.
fn reject_narrow_identifier_core(conn: &Connection) -> Result<(), AppError> {
    let width: Option<String> = conn
        .query_row(
            "SELECT data_type FROM information_schema.columns
             WHERE table_name = 'requests' AND column_name = 'id'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to inspect the core identifier width: {error}"
            ))
        })?;
    match width.as_deref() {
        // No `requests` table yet: a fresh database, about to be created at the current width.
        None => Ok(()),
        Some("BIGINT") => Ok(()),
        Some(narrow) => Err(AppError::Storage(format!(
            "This evidence core stores request identifiers as {narrow}, which holds at most \
             2147483647, and portal identifiers exceed that. DuckDB cannot widen a key column in \
             place, so the core cannot be upgraded automatically. Export the workspace, or move \
             the core database aside and re-ingest, before continuing."
        ))),
    }
}

fn prepare_legacy_legal_schema(connection: &Connection) -> Result<(), AppError> {
    if table_exists(connection, "legal_statutes")? {
        // Type and DEFAULT only, for the reason spelled out over the portals shims in
        // `run_migrations`: DuckDB cannot add a constrained column to a table that already exists,
        // and this repair path only ever runs against one that does.
        ensure_column(
            connection,
            "legal_statutes",
            "authority_kind",
            "TEXT DEFAULT 'OTHER'",
        )?;
        ensure_column(connection, "legal_statutes", "canonical_source_uri", "TEXT")?;
        ensure_column(
            connection,
            "legal_statutes",
            "current_version_id",
            "TEXT DEFAULT ''",
        )?;
        ensure_column(
            connection,
            "legal_statutes",
            "content_sha256",
            "TEXT DEFAULT '0000000000000000000000000000000000000000000000000000000000000000'",
        )?;
        ensure_column(connection, "legal_statutes", "effective_from", "TEXT")?;
        ensure_column(connection, "legal_statutes", "effective_to", "TEXT")?;
        ensure_column(
            connection,
            "legal_statutes",
            "imported_at",
            "TEXT DEFAULT ''",
        )?;
        ensure_column(
            connection,
            "legal_statutes",
            "updated_at",
            "TEXT DEFAULT ''",
        )?;
        normalize_legal_authority_rows(connection)?;
    }

    if table_exists(connection, "statutory_violations")?
        && !table_has_column(
            connection,
            "statutory_violations",
            "confidence_basis_points",
        )?
    {
        if !table_has_column(connection, "statutory_violations", "confidence_score")? {
            return Err(AppError::Storage(
                "The existing statutory_violations table has an unsupported legacy layout"
                    .to_string(),
            ));
        }
        if table_exists(connection, "statutory_violations_legacy_archive")? {
            return Err(AppError::Storage(
                "A legacy statutory findings archive already exists while the active table still requires migration"
                    .to_string(),
            ));
        }
        connection
            .execute_batch(
                "ALTER TABLE statutory_violations
                 RENAME TO statutory_violations_legacy_archive;",
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to preserve the legacy statutory findings table: {error}"
                ))
            })?;
    }
    Ok(())
}

fn normalize_legal_authority_rows(connection: &Connection) -> Result<(), AppError> {
    let records = {
        let mut statement = connection
            .prepare(
                "SELECT statute_id, full_text, content_sha256, current_version_id,
                    imported_at, effective_from, effective_to, canonical_source_uri
                 FROM legal_statutes",
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to prepare legacy legal authority upgrade: {error}"
                ))
            })?;
        let rows = statement
            .query_map([], map_legacy_legal_authority_row)
            .map_err(|error| {
                AppError::Storage(format!("Failed to query legacy legal authorities: {error}"))
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
            AppError::Storage(format!("Failed to decode legacy legal authority: {error}"))
        })?
    };
    let upgraded_at = chrono::Utc::now().to_rfc3339();
    for row in records {
        let content_sha256 = normalize_sha256(&row.stored_sha256)
            .ok()
            .filter(|value| value.bytes().any(|byte| byte != b'0'))
            .unwrap_or_else(|| sha256_hex(row.full_text.as_bytes()));
        let current_version_id = if row.stored_version_id.trim().is_empty() {
            let identity = format!("{}\0{}", row.statute_id.trim(), content_sha256);
            let digest = sha256_hex(identity.as_bytes());
            format!("legal-version-{}", &digest[..32])
        } else {
            row.stored_version_id
        };
        let imported_at = if row.imported_at.trim().is_empty() {
            upgraded_at.as_str()
        } else {
            row.imported_at.as_str()
        };
        connection
            .execute(
                "UPDATE legal_statutes
                 SET content_sha256 = ?1,
                     current_version_id = ?2,
                     imported_at = ?3,
                     updated_at = CASE WHEN updated_at = '' THEN ?3 ELSE updated_at END
                 WHERE statute_id = ?4",
                params![
                    content_sha256,
                    current_version_id,
                    imported_at,
                    row.statute_id
                ],
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to upgrade legal authority '{}': {error}",
                    row.statute_id
                ))
            })?;
        if table_exists(connection, "legal_statute_versions")? {
            connection
                .execute(
                    "INSERT OR IGNORE INTO legal_statute_versions (
                        version_id, statute_id, full_text, content_sha256, effective_from,
                        effective_to, source_uri, imported_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        current_version_id,
                        row.statute_id,
                        row.full_text,
                        content_sha256,
                        row.effective_from,
                        row.effective_to,
                        row.source_uri,
                        imported_at,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to create the legal authority version during upgrade: {error}"
                    ))
                })?;
        }
    }
    Ok(())
}

fn backfill_legal_authority_versions(connection: &Connection) -> Result<(), AppError> {
    if table_exists(connection, "legal_statutes")?
        && table_exists(connection, "legal_statute_versions")?
    {
        normalize_legal_authority_rows(connection)?;
    }
    Ok(())
}

fn migrate_archived_statutory_findings(connection: &Connection) -> Result<(), AppError> {
    if !table_exists(connection, "statutory_violations_legacy_archive")? {
        return Ok(());
    }
    connection
        .execute(
            "INSERT OR IGNORE INTO statutory_violations (
                violation_id, portal_subdomain, request_id, matched_statute_id,
                matched_version_id, mandate_id, classification,
                calculated_variance_metric, variance_days, confidence_basis_points,
                evidence_summary, supporting_document_ids_json, missing_evidence_json,
                classifier_version, review_status, first_detected_at, evaluated_at
             )
             SELECT
                'legacy-' || CAST(legacy.violation_id AS TEXT),
                legacy.portal_subdomain,
                legacy.request_id,
                legacy.matched_statute_id,
                legal_statutes.current_version_id,
                NULL,
                'LEGACY_STATUTORY_FINDING',
                legacy.calculated_variance_metric,
                NULL,
                CAST(ROUND(
                    CASE
                        WHEN legacy.confidence_score < 0 THEN 0
                        WHEN legacy.confidence_score > 1 THEN 10000
                        ELSE legacy.confidence_score * 10000
                    END
                ) AS INTEGER),
                COALESCE(legacy.calculated_variance_metric, 'Imported legacy statutory finding'),
                '[]',
                '[]',
                'legacy-schema-import-v1',
                'OPEN',
                strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
                strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
             FROM statutory_violations_legacy_archive AS legacy
             INNER JOIN legal_statutes
                ON legal_statutes.statute_id = legacy.matched_statute_id",
            [],
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to migrate archived statutory findings: {error}"
            ))
        })?;
    Ok(())
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, AppError> {
    if !is_safe_schema_identifier(table) {
        return Err(AppError::Storage(
            "Migration table identifier contains prohibited characters".to_string(),
        ));
    }
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM information_schema.tables
                WHERE table_schema = current_schema() AND table_name = ?1
             )",
            params![table],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| AppError::Storage(format!("Failed to inspect table '{table}': {error}")))
}

fn table_has_column(connection: &Connection, table: &str, column: &str) -> Result<bool, AppError> {
    if !is_safe_schema_identifier(table) || !is_safe_schema_identifier(column) {
        return Err(AppError::Storage(
            "Migration column identifier contains prohibited characters".to_string(),
        ));
    }
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM information_schema.columns
                WHERE table_schema = current_schema()
                  AND table_name = ?1
                  AND column_name = ?2
             )",
            params![table, column],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| AppError::Storage(format!("Failed to inspect table '{table}': {error}")))
}

fn is_safe_schema_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

pub(super) fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), AppError> {
    if !is_safe_schema_identifier(table) || !is_safe_schema_identifier(column) {
        return Err(AppError::Storage(
            "Migration identifiers contain prohibited characters".to_string(),
        ));
    }
    // DuckDB's own refusal names neither the caller nor the fix, and it only ever fires on an
    // installation old enough to be missing the column, which is never a developer's machine.
    // Rejecting here fails in the migration tests instead, where the message can say what to do.
    //
    // Matched on the token DuckDB's parser actually chokes on, not on the bare word: a definition
    // like `TEXT DEFAULT 'UNCHECKED'` is legal, and refusing it would abort startup for the same
    // reason this guard exists to prevent.
    let normalized_definition = definition.to_ascii_uppercase();
    if normalized_definition.contains("NOT NULL")
        || normalized_definition.contains("CHECK (")
        || normalized_definition.contains("CHECK(")
    {
        return Err(AppError::Storage(format!(
            "Definition for '{table}.{column}' carries a constraint DuckDB cannot add to an \
             existing table. Pass a type and a DEFAULT only, and declare NOT NULL and CHECK in \
             the base schema."
        )));
    }
    if table_has_column(connection, table, column)? {
        return Ok(());
    }
    connection
        .execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to add required column '{table}.{column}': {error}"
            ))
        })?;
    Ok(())
}
