//! Thread-safe DuckDB repository layer managing persistent storage and data indexing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use duckdb::{params, params_from_iter, Connection, OptionalExt, Result, Row, ToSql, Transaction};
use tokio::sync::Mutex;

use crate::analysis::{
    analyze_redaction_density, extract_enforcement_keyword_counts, ClusterAlertState,
    SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS,
};
use crate::blocking::spawn_with;
use crate::error::AppError;
use crate::integrity::{constant_time_equal, sha256_file, sha256_hex};
use crate::normalization::collapse_whitespace;
use crate::parsers::{policy_tokens, token_sequence_match_count, ParsedLegalAuthority};
use crate::provenance::{classify_confidence, read_asset_provenance, AssetProvenance};
use crate::storage::fts::{clean_plain_text_for_index, core_query_tokens, token_frequencies};
use crate::types::c2pa::{ProvenanceConfidence, RecordedDigestSource, SignatureVerificationState};
use crate::types::ProvenanceVerificationRecord;
use crate::types::{
    AuxiliaryEventInput, AuxiliaryImportResult, AuxiliaryStreamKind, CrawlPolicy,
    CrossAgencyCorrelation, DocumentMetadata, DocumentProvenanceVerification, DocumentTextChunk,
    LegalAuthorityImport, LegalAuthorityImportResult, LegalAuthorityKind, LegalAuthoritySummary,
    LegalAuthorityVersionDetail, LegalAuthorityVersionSummary, LegalBriefExportResult,
    LegalComplianceFinding, LegalEvaluationResult, LegalFindingReviewStatus, PortalRegistration,
    PublicRequest, RequestDocumentSummary, RequestSearchResult, SavedSource, SourceGuidance,
    SourceLifecycleState, StatutoryExtensionReason, StatutoryExtensionRecord,
    TimelineAnomalyMarker,
};

const MAXIMUM_DOCUMENT_CHUNK_CHARACTERS: usize = 250_000;
const DOCUMENT_CHUNK_PII_BOUNDARY_CHARACTERS: usize = 256;

const DOWNLOAD_STATUS_COMPLETED: &str = "completed";
const VERIFICATION_STATUS_COMPLIANT: &str = "Compliant";
const VERIFICATION_STATUS_ANOMALY: &str = "DataAnomalyFlagged";
const VERIFICATION_STATUS_STATUTORY: &str = "StatutoryDeadlineExceeded";
const FLAG_UPDATE_BEFORE_CREATION: &str = "UPDATE_BEFORE_CREATION";
const FLAG_CLOSE_BEFORE_CREATION: &str = TimelineAnomalyMarker::CloseBeforeCreation.as_str();
const FLAG_SUSPICIOUSLY_FAST_CLOSURE: &str =
    TimelineAnomalyMarker::SuspiciouslyFastClosure.as_str();
const FLAG_HEAVY_REDACTION_DENSITY: &str = "HEAVY_REDACTION_DENSITY";
const FLAG_SUSPICIOUS_DATA_DENSITY: &str = "SUSPICIOUS_DATA_DENSITY";
const FLAG_STATUTORY_LATENCY_VIOLATION: &str =
    TimelineAnomalyMarker::StatutoryLatencyViolation.as_str();
const FLAG_STATUTORY_EXTENSION_DEFECTIVE: &str =
    TimelineAnomalyMarker::StatutoryExtensionDefective.as_str();
const FLAG_POTENTIAL_POLICY_VIOLATION: &str = "POTENTIAL_POLICY_VIOLATION";
const POLICY_CLASSIFIER_VERSION: &str = "structural-evidence-gap-v1";
const MAXIMUM_POLICY_CORPUS_BYTES: usize = 32 * 1024 * 1024;
const LEGAL_AUTHORITY_SUMMARY_SELECT: &str = "SELECT
    legal_statutes.statute_id, legal_statutes.jurisdiction,
    legal_statutes.statute_citation, legal_statutes.title,
    legal_statutes.authority_kind, legal_statutes.current_version_id,
    legal_statutes.content_sha256, legal_statutes.mandated_deadline_days,
    legal_statutes.canonical_source_uri, legal_statutes.effective_from,
    legal_statutes.effective_to,
    (SELECT COUNT(*) FROM legal_provisions
     WHERE legal_provisions.version_id = legal_statutes.current_version_id),
    (SELECT COUNT(*) FROM legal_mandates
     WHERE legal_mandates.version_id = legal_statutes.current_version_id
       AND legal_mandates.enabled = 1),
    legal_statutes.imported_at
 FROM legal_statutes";

const DERIVED_DOCUMENT_FLAG_TYPES: [&str; 5] = [
    FLAG_UPDATE_BEFORE_CREATION,
    FLAG_CLOSE_BEFORE_CREATION,
    FLAG_SUSPICIOUSLY_FAST_CLOSURE,
    FLAG_HEAVY_REDACTION_DENSITY,
    FLAG_SUSPICIOUS_DATA_DENSITY,
];

const DERIVED_TIMELINE_FLAG_TYPES: [&str; 5] = [
    FLAG_UPDATE_BEFORE_CREATION,
    FLAG_CLOSE_BEFORE_CREATION,
    FLAG_SUSPICIOUSLY_FAST_CLOSURE,
    FLAG_STATUTORY_LATENCY_VIOLATION,
    FLAG_STATUTORY_EXTENSION_DEFECTIVE,
];

#[derive(Debug)]
struct RequestAnalyticalState {
    request_id: i64,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
    status: String,
    file_name: Option<String>,
}

struct RequestSearchRow {
    request_id: u64,
    portal_key: String,
    agency_id: Option<u64>,
    agency_name: String,
    title: String,
    description: String,
    status: String,
    created_at: String,
    closed_at: Option<String>,
    document_count: i64,
    integrity_flags: String,
}

struct ProvenanceRow {
    expected_sha256: Option<String>,
    recorded_text_projection_sha256: Option<String>,
}

struct RequestDocumentRow {
    document_id: u64,
    portal_key: String,
    file_name: String,
    file_size: Option<i64>,
    content_type: Option<String>,
    visibility: String,
    download_status: String,
    raw_file_sha256: Option<String>,
    extracted_text_sha256: Option<String>,
    extraction_status: String,
    extraction_format: Option<String>,
    extraction_deferred_reason: Option<String>,
}

struct LegalBriefRow {
    request_id: u64,
    portal_key: String,
    portal_name: String,
    portal_url: String,
    agency_name: String,
    rpid: Option<String>,
    title: String,
    description: String,
    status: String,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
    determination_at: Option<String>,
    determination_source: Option<String>,
    extension_notice_at: Option<String>,
    extension_expected_determination_at: Option<String>,
    extension_reason: Option<String>,
    extension_notice_source: Option<String>,
}

struct RawLegalAuthoritySummary {
    statute_id: String,
    jurisdiction: String,
    statute_citation: String,
    title: String,
    authority_kind: String,
    current_version_id: String,
    content_sha256: String,
    mandated_deadline_days: Option<i64>,
    canonical_source_uri: Option<String>,
    effective_from: Option<String>,
    effective_to: Option<String>,
    provision_count: i64,
    mandate_count: i64,
    imported_at: String,
}

struct RawLegalFinding {
    violation_id: String,
    request_id: i64,
    portal_key: String,
    matched_statute_id: String,
    matched_version_id: String,
    mandate_id: Option<String>,
    statute_citation: String,
    authority_title: String,
    mandate_title: Option<String>,
    classification: String,
    calculated_variance_metric: Option<String>,
    variance_days: Option<i64>,
    confidence_basis_points: i64,
    evidence_summary: String,
    supporting_document_ids_json: String,
    missing_evidence_json: String,
    review_status: String,
    first_detected_at: String,
    evaluated_at: String,
}

/// Complete terminal state produced by a successful document extraction task.
#[derive(Clone, Copy, Debug)]
pub struct DocumentExtractionFinalization<'a> {
    pub document_id: u64,
    pub portal_subdomain: &'a str,
    pub extracted_text: &'a str,
    pub extracted_text_sha256: &'a str,
    pub raw_file_sha256: &'a str,
    pub was_ocr_processed: bool,
    pub raw_file_size: u64,
    pub local_path: &'a Path,
    /// `None` when entropy was never computed for this file. 0.0 is a measurement, not an
    /// absence, so the two cannot share a representation.
    pub shannon_entropy: Option<f64>,
    pub entropy_sample_bytes: u64,
    pub suspicious_data_density: bool,
}

/// Terminal metadata for a downloaded document that has no searchable text layer.
#[derive(Clone, Copy, Debug)]
pub struct DocumentDownloadFinalization<'a> {
    pub document_id: u64,
    pub portal_subdomain: &'a str,
    pub raw_file_size: u64,
    pub raw_file_sha256: &'a str,
    pub local_path: &'a Path,
    /// `None` when entropy was never computed for this file. See
    /// [`DocumentExtractionFinalization::shannon_entropy`].
    pub shannon_entropy: Option<f64>,
    pub entropy_sample_bytes: u64,
    pub suspicious_data_density: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct PortalSyncRunFinalization<'a> {
    pub run_id: &'a str,
    pub portal_key: &'a str,
    pub status: &'a str,
    pub pages_processed: u64,
    pub requests_processed: u64,
    pub documents_processed: u64,
    pub failures: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IngestionSchedulerCounts {
    pub crawl_pages_started: u64,
    pub resources_started: u64,
}

#[derive(Debug)]
pub struct ClaimedIngestionTask {
    pub frontier_id: i64,
    pub payload_json: String,
    pub attempts: u64,
}

/// A document's recorded evidence file, already resolved and gated by the command layer.
///
/// Resolution is split out of the repository on purpose: deciding whether a path is inside the
/// managed evidence store is filesystem policy, and the repository has no business knowing where
/// the store lives. See `commands::validation::readable_evidence_file`.
pub enum ProvenanceEvidenceFile {
    /// A regular file inside the managed evidence store, safe to hash.
    Readable(PathBuf),
    /// No usable evidence file. The string is the reason, recorded on the verification.
    Unavailable(String),
}

/// Cloneable handle to the serialized DuckDB writer.
#[derive(Clone)]
pub struct DuckDbRepository {
    conn: Arc<Mutex<Connection>>,
    database_path: Arc<PathBuf>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceDocument {
    pub document_id: u64,
    pub file_name: String,
    pub file_size: Option<u64>,
    pub visibility: String,
    pub raw_file_sha256: Option<String>,
    pub extracted_text_sha256: Option<String>,
    /// `None` for a document whose entropy was never computed -- one ingested before the column
    /// existed, or imported. Readers must render that as "not computed", never as 0.0.
    pub shannon_entropy: Option<f64>,
    pub entropy_sample_bytes: u64,
    pub suspicious_data_density: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegalBriefEvidence {
    pub request_id: u64,
    pub portal_key: String,
    pub portal_name: String,
    pub portal_url: String,
    pub agency_name: String,
    pub rpid: Option<String>,
    pub title: String,
    pub description: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub determination_at: Option<String>,
    pub determination_source: Option<String>,
    pub extension: Option<StatutoryExtensionRecord>,
    pub integrity_flags: Vec<String>,
    pub documents: Vec<EvidenceDocument>,
}

/// One extracted document yielded by the keyset-paginated corpus backfill reader.
pub struct CorpusSourceDocument {
    pub portal_key: String,
    pub request_id: u64,
    pub document_id: u64,
    pub raw_file_sha256: String,
    pub extracted_text_sha256: String,
}

struct PolicyCorpusSource {
    document_id: Option<u64>,
    tokens: Vec<String>,
}

struct StoredLegalMandate {
    mandate_id: String,
    statute_id: String,
    version_id: String,
    title: String,
    applicability_phrases: Vec<String>,
    required_evidence_phrases: Vec<String>,
}

struct RawStoredLegalMandate {
    mandate_id: String,
    statute_id: String,
    version_id: String,
    title: String,
    applicability_json: String,
    evidence_json: String,
}

struct LegacyLegalAuthorityRow {
    statute_id: String,
    full_text: String,
    stored_sha256: String,
    stored_version_id: String,
    imported_at: String,
    effective_from: Option<String>,
    effective_to: Option<String>,
    source_uri: Option<String>,
}

struct PolicyEvaluationState {
    evaluated_mandate_count: usize,
    applicable_mandate_count: usize,
    corpus_character_count: usize,
    evaluation_complete: bool,
    warnings: Vec<String>,
}

impl DuckDbRepository {
    /// Establishes a persistent DuckDB connection and configures conservative local analytics
    /// settings. All mutations are serialized through this connection because DuckDB's durable
    /// single-process writer model is the reliability boundary for the desktop application.
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, AppError> {
        let supplied_path = db_path.as_ref();
        let database_path = if supplied_path == Path::new(":memory:") || supplied_path.is_absolute()
        {
            supplied_path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| {
                    AppError::Storage(format!("Failed to resolve working directory: {error}"))
                })?
                .join(supplied_path)
        };
        let conn = if database_path == Path::new(":memory:") {
            Connection::open_in_memory()
        } else {
            Connection::open(&database_path)
        }
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to open DuckDB database '{}': {error}",
                database_path.display()
            ))
        })?;
        conn.execute_batch(
            "SET preserve_insertion_order = false;
             SET threads = 1;
             SET enable_progress_bar = false;",
        )
        .map_err(|error| AppError::Internal(format!("Failed to configure DuckDB: {error}")))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            database_path: Arc::new(database_path),
        })
    }

    /// Opens an additional connection to the *same live* DuckDB instance.
    ///
    /// Analytics engines must obtain their connection here. A connection that instead opens or
    /// `ATTACH`es the database file independently observes a snapshot frozen at attach time and
    /// never sees a later write, so every analytical result would be stale from process start.
    pub async fn analytics_connection(&self) -> Result<Connection, AppError> {
        let connection = self.conn.lock().await;
        connection.try_clone().map_err(|error| {
            AppError::Internal(format!("Failed to open an analytics connection: {error}"))
        })
    }

    async fn query_connection<T, F>(&self, operation: &'static str, query: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, AppError> + Send + 'static,
    {
        let connection = Arc::clone(&self.conn);
        spawn_with(operation, AppError::Internal, move || {
            let connection = connection.blocking_lock();
            query(&connection)
        })
        .await?
    }

    /// Loads at most one document descriptor using a stable keyset cursor. The corresponding
    /// full text is loaded only when the derived corpus index is stale or missing.
    pub async fn next_corpus_source_document(
        &self,
        after: Option<(String, u64)>,
    ) -> Result<Option<CorpusSourceDocument>, AppError> {
        let (after_portal, after_document_id) = match after {
            Some((portal_key, document_id)) => (Some(portal_key), document_id),
            None => (None, 0),
        };
        let after_document_id = i64::try_from(after_document_id).map_err(|_| {
            AppError::DataParsing("Corpus backfill cursor exceeds SQLite capacity".to_string())
        })?;
        self.query_connection("Corpus backfill source query", move |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT
                        documents.portal_subdomain,
                        documents.request_id,
                        documents.id,
                        documents.raw_file_sha256,
                        documents.extracted_text_sha256
                     FROM documents
                     WHERE documents.download_status = 'completed'
                       AND LENGTH(documents.raw_file_sha256) = 64
                       AND LENGTH(documents.extracted_text_sha256) = 64
                       AND EXISTS (
                           SELECT 1 FROM documents_fts
                           WHERE documents_fts.document_id = documents.id
                             AND documents_fts.portal_subdomain = documents.portal_subdomain
                             AND documents_fts.extracted_text <> ''
                       )
                       AND (?1 IS NULL
                            OR documents.portal_subdomain > ?1
                            OR (documents.portal_subdomain = ?1 AND documents.id > ?2))
                     ORDER BY documents.portal_subdomain, documents.id
                     LIMIT 1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare corpus backfill source: {error}"))
                })?;
            let mut rows = statement
                .query(params![after_portal, after_document_id])
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query corpus backfill source: {error}"))
                })?;
            let Some(row) = rows.next().map_err(|error| {
                AppError::Storage(format!("Failed to read corpus backfill source: {error}"))
            })?
            else {
                return Ok(None);
            };
            let request_id = row.get::<_, i64>(1).map_err(|error| {
                AppError::Storage(format!("Invalid corpus request identifier: {error}"))
            })?;
            let document_id = row.get::<_, i64>(2).map_err(|error| {
                AppError::Storage(format!("Invalid corpus document identifier: {error}"))
            })?;
            Ok(Some(CorpusSourceDocument {
                portal_key: row.get(0).map_err(|error| {
                    AppError::Storage(format!("Invalid corpus portal key: {error}"))
                })?,
                request_id: u64::try_from(request_id).map_err(|_| {
                    AppError::Storage("Corpus request identifier was negative".to_string())
                })?,
                document_id: u64::try_from(document_id).map_err(|_| {
                    AppError::Storage("Corpus document identifier was negative".to_string())
                })?,
                raw_file_sha256: row.get(3).map_err(|error| {
                    AppError::Storage(format!("Invalid corpus raw digest: {error}"))
                })?,
                extracted_text_sha256: row.get(4).map_err(|error| {
                    AppError::Storage(format!("Invalid corpus text digest: {error}"))
                })?,
            }))
        })
        .await
    }

    pub async fn load_corpus_source_text(
        &self,
        portal_key: &str,
        document_id: u64,
    ) -> Result<String, AppError> {
        let portal_key = portal_key.to_string();
        self.query_connection("Corpus source text query", move |connection| {
            connection
                .query_row(
                    "SELECT extracted_text
                     FROM documents_fts
                     WHERE document_id = ?1 AND portal_subdomain = ?2
                     ORDER BY document_id DESC
                     LIMIT 1",
                    params![document_id, portal_key],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to load corpus source text for {portal_key}/{document_id}: {error}"
                    ))
                })
        })
        .await
    }

    /// Run embedded raw text scripts to establish layout boundaries.
    pub async fn run_migrations(&self, migration_sql: &str) -> Result<(), AppError> {
        let mut conn = self.conn.lock().await;
        // Legacy repair must precede the idempotent base script; `ensure_column` calls that follow
        // are compatibility shims for databases created by older, partially versioned builds.
        prepare_legacy_legal_schema(&conn)?;
        reject_narrow_identifier_core(&conn)?;
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
        ensure_column(&conn, "portals", "origin_url", "TEXT DEFAULT ''")?;
        ensure_column(&conn, "portals", "requests_endpoint", "TEXT DEFAULT ''")?;
        ensure_column(
            &conn,
            "portals",
            "requests_pointer",
            "TEXT DEFAULT '/requests'",
        )?;
        ensure_column(&conn, "portals", "metadata_pointer", "TEXT")?;
        ensure_column(&conn, "portals", "page_parameter", "TEXT DEFAULT 'page'")?;
        ensure_column(
            &conn,
            "portals",
            "page_size_parameter",
            "TEXT DEFAULT 'per_page'",
        )?;
        ensure_column(&conn, "portals", "verified_at", "TEXT")?;
        ensure_column(&conn, "portals", "health_status", "TEXT DEFAULT 'verified'")?;
        ensure_column(&conn, "portals", "last_error", "TEXT")?;
        ensure_column(
            &conn,
            "portals",
            "source_kind",
            "TEXT DEFAULT 'NEXT_REQUEST'",
        )?;
        // A source that predates the lifecycle column really was never paused or removed, so
        // 'ACTIVE' is the fact about it rather than a stand-in for one that was never recorded.
        // Readers still coalesce, because DuckDB's own backfill for an added column is what this
        // depends on and it is not worth trusting silently.
        ensure_column(&conn, "portals", "lifecycle_state", "TEXT DEFAULT 'ACTIVE'")?;
        // These admission counters bound an interrupted crawl across process restarts. Older runs
        // predate durable counting, so zero is the only evidence-backed backfill available.
        ensure_column(
            &conn,
            "portal_sync_runs",
            "crawl_pages_started",
            "BIGINT DEFAULT 0",
        )?;
        ensure_column(
            &conn,
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
                &conn,
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
            ensure_column(&conn, "provenance_verification_log", column, definition)?;
        }
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
            &conn,
            "documents",
            "extracted_text_projection_sha256",
            "TEXT",
        )?;
        ensure_column(&conn, "requests", "neighborhood_block", "TEXT")?;
        ensure_column(&conn, "requests", "determination_at", "TEXT")?;
        ensure_column(&conn, "requests", "determination_source", "TEXT")?;
        ensure_column(
            &conn,
            "cross_correlation_alerts",
            "complaint_count",
            "BIGINT DEFAULT 0",
        )?;
        ensure_column(
            &conn,
            "cross_correlation_alerts",
            "correlation_window_hours",
            "BIGINT DEFAULT 48",
        )?;
        ensure_column(&conn, "documents", "raw_file_sha256", "TEXT")?;
        // Same rule as the portals block above. The base schema bounds these three; an upgraded
        // database carries the DEFAULT but not the range check.
        //
        // `shannon_entropy` takes no DEFAULT: 0.0 is a measurement -- every byte in the sample
        // identical -- not "never computed", and a document that predates this column was never
        // assessed. Backfilling it to 0.0 would state the strongest possible uniformity signal
        // about a file nothing ever read. NULL is representable and says exactly that.
        ensure_column(&conn, "documents", "shannon_entropy", "REAL")?;
        // The other two keep DEFAULT 0. 0 sampled bytes genuinely means no bytes were sampled,
        // and 0 density is the correct default claim: nothing was flagged. Neither is a
        // measurement standing in for an absence.
        ensure_column(
            &conn,
            "documents",
            "entropy_sample_bytes",
            "BIGINT DEFAULT 0",
        )?;
        ensure_column(
            &conn,
            "documents",
            "suspicious_data_density",
            "BIGINT DEFAULT 0",
        )?;
        ensure_column(
            &conn,
            "documents",
            "extraction_status",
            "TEXT DEFAULT 'PENDING'",
        )?;
        ensure_column(&conn, "documents", "extraction_format", "TEXT")?;
        ensure_column(&conn, "documents", "extraction_deferred_reason", "TEXT")?;
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
        backfill_legal_authority_versions(&conn)?;
        migrate_archived_statutory_findings(&conn)?;
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
        backfill_search_token_index(&mut conn)?;
        verify_logical_relations(&conn)?;
        Ok(())
    }

    pub fn database_path(&self) -> &Path {
        self.database_path.as_path()
    }

    pub async fn insert_or_update_portal_registration(
        &self,
        portal: &PortalRegistration,
    ) -> Result<(), AppError> {
        validate_portal_registration(portal)?;
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO portals (
                subdomain, display_name, base_url, origin_url, requests_endpoint,
                requests_pointer, metadata_pointer, page_parameter, page_size_parameter,
                verified_at, source_kind, health_status, last_error, lifecycle_state
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'verified', NULL, 'ACTIVE')
             ON CONFLICT(subdomain) DO UPDATE SET
                display_name = excluded.display_name,
                base_url = excluded.base_url,
                origin_url = excluded.origin_url,
                requests_endpoint = excluded.requests_endpoint,
                requests_pointer = excluded.requests_pointer,
                metadata_pointer = excluded.metadata_pointer,
                page_parameter = excluded.page_parameter,
                page_size_parameter = excluded.page_size_parameter,
                verified_at = excluded.verified_at,
                source_kind = excluded.source_kind,
                health_status = 'verified',
                last_error = NULL,
                -- Provisioning a pool for a source is the act of synchronizing it, so it is also
                -- what un-pauses one and what brings a withdrawn source back. Without this a
                -- re-added source stays filtered out of every listing and re-adding it looks
                -- like it did nothing at all.
                lifecycle_state = 'ACTIVE'",
            params![
                portal.portal_key,
                portal.display_name,
                portal.canonical_url,
                portal.origin_url,
                portal.api_layout.requests_endpoint,
                portal.api_layout.requests_pointer,
                portal.api_layout.metadata_pointer,
                portal.api_layout.page_parameter,
                portal.api_layout.page_size_parameter,
                portal.verified_at,
                portal.source_kind.as_str(),
            ],
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to provision portal partition: {error}"))
        })?;
        Ok(())
    }

    pub async fn begin_portal_sync_run(
        &self,
        run_id: &str,
        portal_key: &str,
    ) -> Result<(), AppError> {
        if run_id.trim().is_empty() || portal_key.trim().is_empty() {
            return Err(AppError::DataParsing(
                "Portal sync run identifiers must not be empty".to_string(),
            ));
        }
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO portal_sync_runs (
                run_id, portal_subdomain, started_at, status
             ) VALUES (?1, ?2, strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ'), 'RUNNING')",
            params![run_id, portal_key],
        )
        .map_err(|error| AppError::Storage(format!("Failed to begin portal sync run: {error}")))?;
        Ok(())
    }

    pub async fn ingestion_scheduler_counts(
        &self,
        run_id: &str,
        portal_key: &str,
    ) -> Result<IngestionSchedulerCounts, AppError> {
        let conn = self.conn.lock().await;
        let (crawl_pages_started, resources_started): (i64, i64) = conn
            .query_row(
                "SELECT crawl_pages_started, resources_started
                 FROM portal_sync_runs
                 WHERE run_id = ?1 AND portal_subdomain = ?2",
                params![run_id, portal_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to load ingestion scheduler counters: {error}"
                ))
            })?;
        Ok(IngestionSchedulerCounts {
            crawl_pages_started: u64::try_from(crawl_pages_started).map_err(|_| {
                AppError::Storage("Stored crawl page counter was negative".to_string())
            })?,
            resources_started: u64::try_from(resources_started).map_err(|_| {
                AppError::Storage("Stored resource counter was negative".to_string())
            })?,
        })
    }

    pub async fn reserve_ingestion_crawl_page(
        &self,
        run_id: &str,
        portal_key: &str,
        maximum: usize,
    ) -> Result<Option<u64>, AppError> {
        self.reserve_ingestion_budget(run_id, portal_key, "crawl_pages_started", maximum)
            .await
    }

    pub async fn reserve_ingestion_resource(
        &self,
        run_id: &str,
        portal_key: &str,
        maximum: usize,
    ) -> Result<Option<u64>, AppError> {
        self.reserve_ingestion_budget(run_id, portal_key, "resources_started", maximum)
            .await
    }

    async fn reserve_ingestion_budget(
        &self,
        run_id: &str,
        portal_key: &str,
        counter_column: &'static str,
        maximum: usize,
    ) -> Result<Option<u64>, AppError> {
        debug_assert!(matches!(
            counter_column,
            "crawl_pages_started" | "resources_started"
        ));
        let maximum = i64::try_from(maximum).map_err(|_| {
            AppError::PortalCapacity(
                "Ingestion scheduler budget exceeds DuckDB capacity".to_string(),
            )
        })?;
        let conn = self.conn.lock().await;
        let statement = format!(
            "UPDATE portal_sync_runs
             SET {counter_column} = {counter_column} + 1
             WHERE run_id = ?1 AND portal_subdomain = ?2 AND status = 'RUNNING'
               AND {counter_column} < ?3"
        );
        let updated = conn
            .execute(&statement, params![run_id, portal_key, maximum])
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to reserve ingestion scheduler capacity: {error}"
                ))
            })?;
        if updated == 0 {
            return Ok(None);
        }
        let current: i64 = conn
            .query_row(
                &format!(
                    "SELECT {counter_column} FROM portal_sync_runs
                     WHERE run_id = ?1 AND portal_subdomain = ?2"
                ),
                params![run_id, portal_key],
                |row| row.get(0),
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to read reserved ingestion scheduler capacity: {error}"
                ))
            })?;
        Ok(Some(u64::try_from(current).map_err(|_| {
            AppError::Storage("Stored ingestion scheduler counter was negative".to_string())
        })?))
    }

    pub async fn record_source_task_failure(
        &self,
        run_id: &str,
        source_key: &str,
        target_id: &str,
        error: &AppError,
    ) -> Result<(), AppError> {
        let error_type = error.category();
        let error_message = error.to_string();
        let retryable = error.is_retryable();
        let repairable = error.is_source_repairable();
        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "source task failure recording", |transaction| {
            transaction
                .execute(
                    "INSERT INTO source_ingestion_failures (
                        run_id, source_key, target_id, error_type, error_message,
                        retryable, repairable
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(run_id, target_id, error_type) DO UPDATE SET
                        error_message = excluded.error_message,
                        retryable = excluded.retryable,
                        repairable = excluded.repairable,
                        occurrences = source_ingestion_failures.occurrences + 1,
                        last_occurred_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')",
                    params![
                        run_id,
                        source_key,
                        target_id,
                        error_type,
                        error_message,
                        retryable,
                        repairable
                    ],
                )
                .map_err(|database_error| {
                    AppError::Storage(format!(
                        "Failed to record source ingestion failure: {database_error}"
                    ))
                })?;
            transaction
                .execute(
                    "UPDATE portals
                     SET health_status = 'degraded', last_error = ?1
                     WHERE subdomain = ?2",
                    params![error_message, source_key],
                )
                .map_err(|database_error| {
                    AppError::Storage(format!(
                        "Failed to update source health after a task failure: {database_error}"
                    ))
                })?;
            Ok(())
        })
    }

    /// Persists a failure that happens before startup recovery can create a replacement run.
    ///
    /// These failures cannot use `source_ingestion_failures`, whose `run_id` is a required foreign
    /// key. The portal health row is the durable source of truth exposed by `SavedSource`, while
    /// live telemetry provides the same detail to an already-open webview.
    pub async fn record_source_recovery_failure(
        &self,
        source_key: &str,
        stage: &str,
        error: &AppError,
    ) -> Result<(), AppError> {
        let message = format!("Startup recovery failed during {stage}: {error}");
        let conn = self.conn.lock().await;
        let updated = conn
            .execute(
                "UPDATE portals
                 SET health_status = 'degraded', last_error = ?1
                 WHERE subdomain = ?2",
                params![message, source_key],
            )
            .map_err(|database_error| {
                AppError::Storage(format!(
                    "Failed to persist startup recovery failure for {source_key}: {database_error}"
                ))
            })?;
        if updated != 1 {
            return Err(AppError::Storage(format!(
                "Cannot persist startup recovery failure for unknown source '{source_key}'"
            )));
        }
        Ok(())
    }

    pub async fn persist_ingestion_frontier_task(
        &self,
        run_id: &str,
        source_key: &str,
        target_id: &str,
        payload_json: &str,
    ) -> Result<bool, AppError> {
        let conn = self.conn.lock().await;
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO ingestion_task_frontier (
                    run_id, source_key, target_id, payload_json
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![run_id, source_key, target_id, payload_json],
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to persist a bounded ingestion frontier task: {error}"
                ))
            })?;
        Ok(inserted == 1)
    }

    pub async fn persist_ingestion_frontier_tasks(
        &self,
        run_id: &str,
        source_key: &str,
        tasks: &[(String, String)],
    ) -> Result<u64, AppError> {
        if tasks.is_empty() {
            return Ok(0);
        }
        if tasks.len() > 100_000 {
            return Err(AppError::PortalCapacity(
                "A single ingestion frontier batch may not exceed 100000 tasks".to_string(),
            ));
        }
        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "ingestion frontier batch", |transaction| {
            let mut statement = transaction
                .prepare_cached(
                    "INSERT OR IGNORE INTO ingestion_task_frontier (
                        run_id, source_key, target_id, payload_json
                     ) VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare ingestion frontier batch: {error}"
                    ))
                })?;
            let mut inserted = 0_u64;
            for (target_id, payload_json) in tasks {
                let changed = statement
                    .execute(params![run_id, source_key, target_id, payload_json])
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to persist ingestion frontier batch task '{target_id}': {error}"
                        ))
                    })?;
                inserted = inserted.saturating_add(changed as u64);
            }
            Ok(inserted)
        })
    }

    pub async fn claim_ingestion_frontier_task(
        &self,
        run_id: &str,
        source_key: &str,
    ) -> Result<Option<ClaimedIngestionTask>, AppError> {
        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "ingestion frontier claim", |transaction| {
            transaction
                .execute(
                    "UPDATE ingestion_task_frontier
                     SET status = 'PENDING', lease_expires_at = NULL,
                         updated_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')
                     WHERE run_id = ?1 AND source_key = ?2 AND status = 'LEASED'
                       AND lease_expires_at <= strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')",
                    params![run_id, source_key],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to recover expired ingestion frontier leases: {error}"
                    ))
                })?;
            let candidate = transaction
                .query_row(
                    "SELECT frontier_id, payload_json, attempts
                     FROM ingestion_task_frontier
                     WHERE run_id = ?1 AND source_key = ?2 AND status = 'PENDING'
                     ORDER BY frontier_id ASC
                     LIMIT 1",
                    params![run_id, source_key],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, u64>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to select an ingestion frontier task: {error}"
                    ))
                })?;
            let Some((frontier_id, payload_json, attempts)) = candidate else {
                return Ok(None);
            };
            let claimed = transaction
                .execute(
                    "UPDATE ingestion_task_frontier
                     SET status = 'LEASED', attempts = attempts + 1,
                         lease_expires_at = strftime(CAST(current_timestamp AS TIMESTAMP) + INTERVAL 10 MINUTE, '%Y-%m-%dT%H:%M:%fZ'),
                         updated_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')
                     WHERE frontier_id = ?1 AND status = 'PENDING'",
                    params![frontier_id],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to lease an ingestion frontier task: {error}"
                    ))
                })?;
            if claimed != 1 {
                return Err(AppError::Storage(format!(
                    "Ingestion frontier task {frontier_id} changed while being claimed"
                )));
            }
            Ok(Some(ClaimedIngestionTask {
                frontier_id,
                payload_json,
                attempts: attempts.saturating_add(1),
            }))
        })
    }

    pub async fn defer_ingestion_frontier_task(
        &self,
        frontier_id: i64,
        delay_seconds: u64,
    ) -> Result<(), AppError> {
        let delay_modifier = format!("+{} seconds", delay_seconds.clamp(1, 300));
        let conn = self.conn.lock().await;
        let changed = conn
            .execute(
                "UPDATE ingestion_task_frontier
                 SET status = 'LEASED',
                     lease_expires_at = strftime(CAST(current_timestamp AS TIMESTAMP) + CAST(?2 AS INTERVAL), '%Y-%m-%dT%H:%M:%fZ'),
                     updated_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')
                 WHERE frontier_id = ?1 AND status = 'LEASED'",
                params![frontier_id, delay_modifier],
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to defer ingestion frontier task {frontier_id}: {error}"
                ))
            })?;
        if changed != 1 {
            return Err(AppError::Storage(format!(
                "Ingestion frontier task {frontier_id} was missing during retry scheduling"
            )));
        }
        Ok(())
    }

    pub async fn complete_ingestion_frontier_task(&self, frontier_id: i64) -> Result<(), AppError> {
        let conn = self.conn.lock().await;
        let deleted = conn
            .execute(
                "DELETE FROM ingestion_task_frontier WHERE frontier_id = ?1",
                params![frontier_id],
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to acknowledge ingestion frontier task {frontier_id}: {error}"
                ))
            })?;
        if deleted != 1 {
            return Err(AppError::Storage(format!(
                "Ingestion frontier task {frontier_id} was missing during acknowledgement"
            )));
        }
        Ok(())
    }

    pub async fn recover_interrupted_source_keys(&self) -> Result<Vec<String>, AppError> {
        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "interrupted ingestion recovery", |transaction| {
            let mut statement = transaction
                .prepare(
                    "SELECT candidate_key FROM (
                         SELECT source_key AS candidate_key FROM ingestion_task_frontier
                         UNION
                         SELECT portal_subdomain AS candidate_key FROM portal_sync_runs
                         WHERE status = 'RUNNING'
                     ) candidates
                     JOIN portals ON portals.subdomain = candidates.candidate_key
                     WHERE COALESCE(portals.lifecycle_state, 'ACTIVE') = 'ACTIVE'
                     ORDER BY 1",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare interrupted source recovery: {error}"
                    ))
                })?;
            let keys = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to query interrupted source recovery: {error}"
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to read interrupted source recovery: {error}"
                    ))
                })?;
            drop(statement);
            transaction
                .execute(
                    "UPDATE portal_sync_runs
                     SET status = 'INTERRUPTED',
                         completed_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')
                     WHERE status = 'RUNNING'",
                    [],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to mark interrupted ingestion runs: {error}"
                    ))
                })?;
            Ok(keys)
        })
    }

    /// Hands every frontier row no live run owns to `new_run_id`.
    ///
    /// The predicate is "no run that is still `RUNNING` holds this work", not any particular
    /// terminal status. A crash leaves `INTERRUPTED`, an operator stop leaves `CANCELLED`, and the
    /// queued work each abandons is equally resumable: why a run ended says nothing about whether
    /// its frontier survives. Matching `INTERRUPTED` alone stranded every stopped source's rows,
    /// which were then never adopted and never deleted, so the source stayed in the startup
    /// recovery list forever while each relaunch re-crawled from the first page.
    pub async fn adopt_orphaned_frontier_tasks(
        &self,
        source_key: &str,
        new_run_id: &str,
    ) -> Result<u64, AppError> {
        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "orphaned frontier adoption", |transaction| {
            let inherited_counts: (i64, i64) = transaction
                .query_row(
                    "SELECT
                        COALESCE(MAX(previous.crawl_pages_started), 0),
                        COALESCE(MAX(previous.resources_started), 0)
                     FROM portal_sync_runs previous
                     WHERE previous.portal_subdomain = ?1
                       AND previous.status <> 'RUNNING'
                       AND EXISTS (
                           SELECT 1 FROM ingestion_task_frontier frontier
                           WHERE frontier.run_id = previous.run_id
                             AND frontier.source_key = ?1
                       )",
                    params![source_key],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to recover ingestion scheduler counters: {error}"
                    ))
                })?;
            let counter_update = transaction
                .execute(
                    "UPDATE portal_sync_runs
                     SET crawl_pages_started = ?1, resources_started = ?2
                     WHERE run_id = ?3 AND portal_subdomain = ?4 AND status = 'RUNNING'",
                    params![
                        inherited_counts.0,
                        inherited_counts.1,
                        new_run_id,
                        source_key
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to restore ingestion scheduler counters: {error}"
                    ))
                })?;
            if counter_update != 1 {
                return Err(AppError::Storage(format!(
                    "New ingestion run '{new_run_id}' was unavailable during frontier adoption"
                )));
            }
            let adopted = transaction
                .execute(
                    "UPDATE ingestion_task_frontier
                     SET run_id = ?1, status = 'PENDING', lease_expires_at = NULL,
                         updated_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')
                     WHERE source_key = ?2
                       AND run_id IN (
                           SELECT run_id FROM portal_sync_runs
                           WHERE portal_subdomain = ?2 AND status <> 'RUNNING'
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM ingestion_task_frontier current_frontier
                           WHERE current_frontier.run_id = ?1
                             AND current_frontier.source_key = ingestion_task_frontier.source_key
                             AND current_frontier.target_id = ingestion_task_frontier.target_id
                       )",
                    params![new_run_id, source_key],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to adopt orphaned ingestion tasks: {error}"))
                })?;
            transaction
                .execute(
                    "DELETE FROM ingestion_task_frontier
                     WHERE source_key = ?1
                       AND run_id IN (
                           SELECT run_id FROM portal_sync_runs
                           WHERE portal_subdomain = ?1 AND status <> 'RUNNING'
                       )",
                    params![source_key],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to remove duplicate orphaned ingestion tasks: {error}"
                    ))
                })?;
            u64::try_from(adopted).map_err(|_| {
                AppError::Storage(
                    "Adopted ingestion frontier count exceeds supported capacity".to_string(),
                )
            })
        })
    }

    pub async fn save_source_connector_recipe(
        &self,
        source_key: &str,
        root_url: &str,
        crawl_policy: &CrawlPolicy,
        guidance: &SourceGuidance,
    ) -> Result<(), AppError> {
        let crawl_policy_json = serde_json::to_string(crawl_policy).map_err(|error| {
            AppError::DataParsing(format!("Failed to encode source crawl policy: {error}"))
        })?;
        let guidance_json = serde_json::to_string(guidance).map_err(|error| {
            AppError::DataParsing(format!("Failed to encode source repair guidance: {error}"))
        })?;
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO source_connector_recipes (
                source_key, root_url, recipe_version, crawl_policy_json, guidance_json, updated_at
             ) VALUES (?1, ?2, 1, ?3, ?4, strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ'))
             ON CONFLICT(source_key) DO UPDATE SET
                root_url = excluded.root_url,
                recipe_version = excluded.recipe_version,
                crawl_policy_json = excluded.crawl_policy_json,
                guidance_json = excluded.guidance_json,
                updated_at = excluded.updated_at",
            params![source_key, root_url, crawl_policy_json, guidance_json],
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to save source connector recipe: {error}"))
        })?;
        Ok(())
    }

    pub async fn load_source_connector_recipe(
        &self,
        root_url: &str,
    ) -> Result<Option<(CrawlPolicy, SourceGuidance)>, AppError> {
        let root_url = root_url.to_string();
        let encoded = self
            .query_connection("Source connector recipe query", move |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT crawl_policy_json, guidance_json
                         FROM source_connector_recipes
                         WHERE root_url = ?1
                         LIMIT 1",
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to prepare source connector recipe: {error}"
                        ))
                    })?;
                let mut rows = statement.query(params![root_url]).map_err(|error| {
                    AppError::Storage(format!("Failed to query source connector recipe: {error}"))
                })?;
                let Some(row) = rows.next().map_err(|error| {
                    AppError::Storage(format!("Failed to read source connector recipe: {error}"))
                })?
                else {
                    return Ok(None);
                };
                Ok(Some((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            })
            .await?;
        let Some((crawl_policy_json, guidance_json)) = encoded else {
            return Ok(None);
        };
        let crawl_policy = serde_json::from_str(&crawl_policy_json).map_err(|error| {
            AppError::Storage(format!("Stored source crawl policy is invalid: {error}"))
        })?;
        let guidance = serde_json::from_str(&guidance_json).map_err(|error| {
            AppError::Storage(format!("Stored source repair guidance is invalid: {error}"))
        })?;
        Ok(Some((crawl_policy, guidance)))
    }

    pub async fn finish_portal_sync_run(
        &self,
        finalization: PortalSyncRunFinalization<'_>,
    ) -> Result<(), AppError> {
        let PortalSyncRunFinalization {
            run_id,
            portal_key,
            status,
            pages_processed,
            requests_processed,
            documents_processed,
            failures,
        } = finalization;
        if !matches!(status, "COMPLETED" | "COMPLETED_WITH_ERRORS" | "CANCELLED") {
            return Err(AppError::DataParsing(format!(
                "Unsupported portal sync terminal status '{status}'"
            )));
        }
        let pages_processed = i64::try_from(pages_processed).map_err(|_| {
            AppError::Storage("Portal page count exceeds SQLite capacity".to_string())
        })?;
        let requests_processed = i64::try_from(requests_processed).map_err(|_| {
            AppError::Storage("Portal request count exceeds SQLite capacity".to_string())
        })?;
        let documents_processed = i64::try_from(documents_processed).map_err(|_| {
            AppError::Storage("Portal document count exceeds SQLite capacity".to_string())
        })?;
        let failures = i64::try_from(failures).map_err(|_| {
            AppError::Storage("Portal failure count exceeds SQLite capacity".to_string())
        })?;
        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "portal sync finalization", |transaction| {
            let updated = transaction
                .execute(
                    "UPDATE portal_sync_runs SET
                    completed_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ'),
                    status = ?1,
                    pages_processed = ?2,
                    requests_processed = ?3,
                    documents_processed = ?4,
                    failures = ?5
                 WHERE run_id = ?6 AND portal_subdomain = ?7 AND status = 'RUNNING'",
                    params![
                        status,
                        pages_processed,
                        requests_processed,
                        documents_processed,
                        failures,
                        run_id,
                        portal_key,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to finalize portal sync run: {error}"))
                })?;
            if updated != 1 {
                return Err(AppError::Storage(format!(
                    "Portal sync run '{run_id}' was missing or already finalized"
                )));
            }
            transaction
                .execute(
                    "UPDATE portals SET
                        last_synced_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ'),
                        health_status = CASE WHEN ?2 = 'COMPLETED' THEN 'verified' ELSE 'degraded' END,
                        last_error = CASE WHEN ?2 = 'COMPLETED' THEN NULL ELSE last_error END
                     WHERE subdomain = ?1",
                    params![portal_key, status],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to update portal sync time: {error}"))
                })?;
            Ok(())
        })
    }

    /// Records whether a saved source still synchronizes, and returns whether a source by that
    /// key existed to record it against.
    pub async fn set_source_lifecycle_state(
        &self,
        source_key: &str,
        state: SourceLifecycleState,
    ) -> Result<bool, AppError> {
        let conn = self.conn.lock().await;
        let updated = conn
            .execute(
                "UPDATE portals SET lifecycle_state = ?2 WHERE subdomain = ?1",
                params![source_key, state.as_str()],
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to record source lifecycle state: {error}"))
            })?;
        Ok(updated > 0)
    }

    /// `None` distinguishes "no source by this key" from any lifecycle state a source can hold.
    pub async fn source_lifecycle_state(
        &self,
        source_key: &str,
    ) -> Result<Option<SourceLifecycleState>, AppError> {
        let source_key = source_key.to_string();
        self.query_connection("Source lifecycle lookup", move |conn| {
            let stored = conn
                .query_row(
                    "SELECT COALESCE(lifecycle_state, 'ACTIVE') FROM portals WHERE subdomain = ?1",
                    params![source_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read source lifecycle state: {error}"))
                })?;
            Ok(stored.as_deref().map(SourceLifecycleState::from_stored))
        })
        .await
    }

    /// Drops every queued and leased frontier row belonging to a source, and reports how many.
    ///
    /// Only a withdrawal calls this. A pause must not, because the frontier is the entire
    /// difference between resuming a crawl and restarting it.
    pub async fn discard_source_frontier(&self, source_key: &str) -> Result<u64, AppError> {
        let conn = self.conn.lock().await;
        let discarded = conn
            .execute(
                "DELETE FROM ingestion_task_frontier WHERE source_key = ?1",
                params![source_key],
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to discard source frontier: {error}"))
            })?;
        u64::try_from(discarded).map_err(|_| {
            AppError::Storage("Discarded frontier count exceeds supported capacity".to_string())
        })
    }

    /// Forgets the learned crawl recipe for a source. Withdrawing a source withdraws the
    /// instructions for crawling it; re-adding it later must re-derive them rather than silently
    /// reuse a recipe the operator believed they had discarded.
    pub async fn delete_source_connector_recipe(&self, source_key: &str) -> Result<(), AppError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "DELETE FROM source_connector_recipes WHERE source_key = ?1",
            params![source_key],
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to discard source connector recipe: {error}"
            ))
        })?;
        Ok(())
    }

    /// How many ingested documents a source withdrawal leaves behind.
    pub async fn count_source_documents(&self, source_key: &str) -> Result<u64, AppError> {
        let source_key = source_key.to_string();
        self.query_connection("Source document count", move |conn| {
            let count = conn
                .query_row(
                    "SELECT COUNT(*) FROM documents WHERE portal_subdomain = ?1",
                    params![source_key],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to count source documents: {error}"))
                })?;
            u64::try_from(count).map_err(|_| {
                AppError::Storage("Source document count exceeds supported capacity".to_string())
            })
        })
        .await
    }

    pub async fn insert_or_update_portal(&self, subdomain: &str) -> Result<(), AppError> {
        let subdomain = subdomain.trim();
        if subdomain.is_empty() {
            return Err(AppError::DataParsing(
                "Portal subdomain must not be empty".to_string(),
            ));
        }

        let base_url = format!("https://{subdomain}.nextrequest.com");
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO portals (subdomain, display_name, base_url)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(subdomain) DO UPDATE SET base_url = excluded.base_url",
            params![subdomain, subdomain, base_url],
        )
        .map_err(|error| {
            AppError::Internal(format!("Failed to persist portal metadata: {error}"))
        })?;
        Ok(())
    }

    pub async fn insert_or_update_document(
        &self,
        subdomain: &str,
        request_id: u64,
        document: &DocumentMetadata,
    ) -> Result<(), AppError> {
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO documents (
                id, request_id, portal_subdomain, file_name, file_size, description,
                content_type, download_url, visibility, created_at, updated_at, local_path,
                extracted_text_sha256, was_ocr_processed
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(id, portal_subdomain) DO UPDATE SET
                request_id = excluded.request_id,
                file_name = excluded.file_name,
                file_size = excluded.file_size,
                description = excluded.description,
                content_type = excluded.content_type,
                download_url = excluded.download_url,
                visibility = excluded.visibility,
                updated_at = excluded.updated_at,
                local_path = COALESCE(excluded.local_path, documents.local_path)",
            params![
                document.id,
                request_id,
                subdomain,
                document.file_name,
                document.file_size,
                document.description,
                document.content_type,
                document.download_url,
                document.visibility,
                document.created_at,
                document.updated_at,
                document.local_path,
                document.extracted_text_sha256,
                if document.was_ocr_processed {
                    1_i64
                } else {
                    0_i64
                },
            ],
        )
        .map_err(|error| {
            AppError::Internal(format!("Failed to persist document metadata: {error}"))
        })?;
        Ok(())
    }

    pub async fn mark_document_downloaded(
        &self,
        finalization: DocumentDownloadFinalization<'_>,
    ) -> Result<(), AppError> {
        let DocumentDownloadFinalization {
            document_id,
            portal_subdomain: subdomain,
            raw_file_size,
            raw_file_sha256,
            local_path,
            shannon_entropy,
            entropy_sample_bytes,
            suspicious_data_density,
        } = finalization;
        let document_id = i64::try_from(document_id).map_err(|error| {
            AppError::DataParsing(format!(
                "Document identifier cannot be represented by SQLite: {error}"
            ))
        })?;
        let file_size = i64::try_from(raw_file_size).map_err(|error| {
            AppError::DataParsing(format!(
                "Document size {raw_file_size} cannot be persisted: {error}"
            ))
        })?;
        let local_path = local_path.to_str().ok_or_else(|| {
            AppError::DataParsing("Document path must be valid UTF-8".to_string())
        })?;
        let raw_file_sha256 = normalize_sha256(raw_file_sha256)?;
        let (entropy_sample_bytes, suspicious_data_density) = validate_entropy_metadata(
            shannon_entropy,
            entropy_sample_bytes,
            raw_file_size,
            suspicious_data_density,
        )?;
        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "document download finalization", |tx| {
            let updated = tx
                .execute(
                    "UPDATE documents SET file_size = ?1, local_path = ?2, raw_file_sha256 = ?3,
                    shannon_entropy = ?4, entropy_sample_bytes = ?5,
                    suspicious_data_density = ?6, download_status = ?7,
                    extraction_status = 'DOWNLOADED', extraction_format = NULL,
                    extraction_deferred_reason = NULL
                 WHERE id = ?8 AND portal_subdomain = ?9",
                    params![
                        file_size,
                        local_path,
                        raw_file_sha256,
                        shannon_entropy,
                        entropy_sample_bytes,
                        suspicious_data_density,
                        DOWNLOAD_STATUS_COMPLETED,
                        document_id,
                        subdomain
                    ],
                )
                .map_err(|error| {
                    AppError::Internal(format!("Failed to mark document downloaded: {error}"))
                })?;
            if updated != 1 {
                return Err(AppError::Internal(format!(
                    "Document download update affected {updated} rows for {subdomain}/{document_id}"
                )));
            }
            synchronize_density_audit(tx, document_id, subdomain)?;
            Ok(())
        })
    }

    pub async fn mark_document_extraction_state(
        &self,
        document_id: u64,
        portal_subdomain: &str,
        status: &str,
        format: Option<&str>,
        deferred_reason: Option<&str>,
    ) -> Result<(), AppError> {
        if !matches!(
            status,
            "SEARCHABLE" | "SALVAGED" | "EXPANDED" | "DEFERRED" | "DOWNLOADED" | "FAILED"
        ) {
            return Err(AppError::DataParsing(format!(
                "Unsupported document extraction status '{status}'"
            )));
        }
        if portal_subdomain.trim().is_empty()
            || format.is_some_and(|value| value.trim().is_empty() || value.len() > 128)
            || deferred_reason.is_some_and(|value| value.trim().is_empty() || value.len() > 2_048)
            || (status == "DEFERRED" && deferred_reason.is_none())
            || (status != "DEFERRED" && deferred_reason.is_some())
        {
            return Err(AppError::DataParsing(
                "Document extraction state is invalid".to_string(),
            ));
        }
        let document_id = i64::try_from(document_id).map_err(|_| {
            AppError::DataParsing("Document identifier exceeds DuckDB capacity".to_string())
        })?;
        let conn = self.conn.lock().await;
        let updated = conn
            .execute(
                "UPDATE documents
                 SET extraction_status = ?1, extraction_format = ?2,
                     extraction_deferred_reason = ?3
                 WHERE id = ?4 AND portal_subdomain = ?5",
                params![
                    status,
                    format,
                    deferred_reason,
                    document_id,
                    portal_subdomain
                ],
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to persist document extraction state: {error}"
                ))
            })?;
        if updated != 1 {
            return Err(AppError::Storage(format!(
                "Document extraction state update affected {updated} rows for {portal_subdomain}/{document_id}"
            )));
        }
        Ok(())
    }

    /// Commits a PublicRequest object and its sub-relations inside an isolated ACID transaction block.
    pub async fn insert_or_update_request(
        &self,
        subdomain: &str,
        request: &PublicRequest,
    ) -> Result<(), AppError> {
        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "request persistence", |tx| {
            if let Some(ref agency) = request.agency {
                tx.execute(
                    "INSERT INTO agencies (id, portal_subdomain, name, department_name)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id, portal_subdomain) DO UPDATE SET
                    name = excluded.name,
                    department_name = excluded.department_name",
                    params![agency.id, subdomain, agency.name, agency.department_name],
                )
                .map_err(|e| {
                    AppError::Internal(format!("Agency mapping write block failed: {}", e))
                })?;
            }

            if let Some(ref category) = request.category {
                tx.execute(
                    "INSERT INTO categories (id, portal_subdomain, name)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(id, portal_subdomain) DO UPDATE SET name = excluded.name",
                    params![category.id, subdomain, category.name],
                )
                .map_err(|e| {
                    AppError::Internal(format!("Category mapping write block failed: {}", e))
                })?;
            }

            let request_updated = tx
                .execute(
                    "UPDATE requests SET
                rpid = ?3, title = ?4, description = ?5, status = ?6,
                current_status_id = ?7, updated_at = ?8, closed_at = ?9,
                due_date = ?10, neighborhood_block = COALESCE(?11, neighborhood_block),
                agency_id = ?12, category_id = ?13,
                determination_at = ?14, determination_source = ?15
             WHERE id = ?1 AND portal_subdomain = ?2",
                    params![
                        request.id,
                        subdomain,
                        request.rpid,
                        request.title,
                        request.description,
                        request.status,
                        request.current_status_id,
                        request.updated_at,
                        request.closed_at,
                        request.due_date,
                        request.neighborhood_block,
                        request.agency.as_ref().map(|a| a.id),
                        request.category.as_ref().map(|c| c.id),
                        request.determination_at,
                        request.determination_source,
                    ],
                )
                .map_err(|e| {
                    AppError::Internal(format!("Main case envelope update failure: {}", e))
                })?;
            if request_updated == 0 {
                tx.execute(
                    "INSERT INTO requests (
                id, portal_subdomain, rpid, title, description, status, current_status_id,
                created_at, updated_at, closed_at, due_date, neighborhood_block,
                agency_id, category_id, determination_at, determination_source
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
             ",
                    params![
                        request.id,
                        subdomain,
                        request.rpid,
                        request.title,
                        request.description,
                        request.status,
                        request.current_status_id,
                        request.created_at,
                        request.updated_at,
                        request.closed_at,
                        request.due_date,
                        request.neighborhood_block,
                        request.agency.as_ref().map(|a| a.id),
                        request.category.as_ref().map(|c| c.id),
                        request.determination_at,
                        request.determination_source,
                    ],
                )
                .map_err(|e| {
                    AppError::Internal(format!(
                        "Main case envelope transaction write failure: {}",
                        e
                    ))
                })?;
            }

            let mut request_text = String::with_capacity(
                request.title.as_deref().map(str::len).unwrap_or(0)
                    + request.description.as_deref().map(str::len).unwrap_or(0)
                    + 1,
            );
            if let Some(title) = &request.title {
                request_text.push_str(title);
            }
            request_text.push(' ');
            if let Some(description) = &request.description {
                request_text.push_str(description);
            }
            let keyword_counts = extract_enforcement_keyword_counts(&request_text);
            tx.execute(
                "DELETE FROM request_keyword_signals
             WHERE request_id = ?1 AND portal_subdomain = ?2",
                params![request.id, subdomain],
            )
            .map_err(|error| {
                AppError::Storage(format!(
                    "Failed to refresh request keyword signals: {error}"
                ))
            })?;
            for (keyword, match_count) in keyword_counts {
                let match_count = i64::try_from(match_count).map_err(|_| {
                    AppError::Storage("Request keyword count exceeds SQLite capacity".to_string())
                })?;
                tx.execute(
                    "INSERT INTO request_keyword_signals (
                    request_id, portal_subdomain, keyword, match_count
                 ) VALUES (?1, ?2, ?3, ?4)",
                    params![request.id, subdomain, keyword, match_count],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to persist request keyword signal: {error}"))
                })?;
            }

            if let Some(extension) = &request.statutory_extension {
                upsert_statutory_extension(tx, request.id, subdomain, extension)?;
            }

            for doc in &request.documents {
                tx.execute(
                    "INSERT INTO documents (
                    id, request_id, portal_subdomain, file_name, file_size, description,
                    content_type, download_url, visibility, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id, portal_subdomain) DO UPDATE SET
                    file_name = excluded.file_name,
                    file_size = CASE
                        WHEN documents.download_status = ?12 THEN documents.file_size
                        ELSE excluded.file_size
                    END,
                    updated_at = excluded.updated_at",
                    params![
                        doc.id,
                        request.id,
                        subdomain,
                        doc.file_name,
                        doc.file_size,
                        doc.description,
                        doc.content_type,
                        doc.download_url,
                        doc.visibility,
                        doc.created_at,
                        doc.updated_at,
                        DOWNLOAD_STATUS_COMPLETED,
                    ],
                )
                .map_err(|e| {
                    AppError::Internal(format!("Document attachment matrix sync error: {}", e))
                })?;
            }

            let audit_id = format!("auto-{}-{}", subdomain, request.id);
            let mut timeline_flags = Vec::with_capacity(DERIVED_TIMELINE_FLAG_TYPES.len());
            for anomaly_type in &request.timeline_anomalies {
                if let Some(known_flag) = DERIVED_TIMELINE_FLAG_TYPES
                    .iter()
                    .copied()
                    .find(|known_flag| anomaly_type == known_flag)
                {
                    if !timeline_flags.contains(&known_flag) {
                        timeline_flags.push(known_flag);
                    }
                }
            }
            synchronize_timeline_audit(tx, &audit_id, request.id, subdomain, &timeline_flags)?;
            let statutory_analysis =
                crate::analysis::evaluate_statutory_compliance_with_determination_at(
                    request.id,
                    &request.created_at,
                    Some(&request.updated_at),
                    request.closed_at.as_deref(),
                    request.determination_at.as_deref(),
                    request.determination_source.as_deref(),
                    request.statutory_extension.as_ref(),
                    &chrono::Utc::now().to_rfc3339(),
                )?;
            synchronize_cpra_finding(tx, request.id, subdomain, &statutory_analysis)?;
            Ok(())
        })
    }

    /// Local document search query execution block.
    pub async fn full_text_search(&self, query_string: &str) -> Result<Vec<u64>, AppError> {
        let tokens = core_query_tokens(query_string)
            .map_err(|message| AppError::DataParsing(message.to_string()))?;
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=tokens.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let required_tokens = tokens.len();
        self.query_connection("Full-text search", move |conn| {
            let query = format!(
                "SELECT document_id
                 FROM (
                    SELECT document_id, portal_subdomain
                    FROM document_search_tokens
                    WHERE token IN ({placeholders})
                    GROUP BY document_id, portal_subdomain
                    HAVING COUNT(*) = {required_tokens}
                 ) matching_documents
                 GROUP BY document_id
                 ORDER BY document_id"
            );
            let mut stmt = conn.prepare(&query).map_err(|e| {
                AppError::Internal(format!("FTS statement initialization breakdown: {}", e))
            })?;
            let rows = stmt
                .query_map(params_from_iter(tokens.iter()), |row| row.get::<_, u64>(0))
                .map_err(|e| AppError::Internal(format!("Search execution query error: {}", e)))?;
            rows.map(|id| {
                id.map_err(|e| {
                    AppError::Internal(format!("Failed to decode an FTS document identifier: {e}"))
                })
            })
            .collect()
        })
        .await
    }

    pub async fn search_requests(
        &self,
        search_term: &str,
        portal_key: Option<&str>,
        agency_id: Option<u64>,
        status: Option<&str>,
        policy_deviation: Option<&str>,
        limit: usize,
    ) -> Result<Vec<RequestSearchResult>, AppError> {
        // The flag separator below is `chr(31)`, not `char(31)`. DuckDB parses `char` as a type
        // keyword, so `char(31)` reads as a type modifier and the whole statement fails to parse
        // at prepare time. SQLite accepts `char(31)`, which is where it came from, and the
        // evidence search returned an error banner instead of results for every query.
        if !(1..=5_000).contains(&limit) {
            return Err(AppError::DataParsing(
                "Request search limit must be between 1 and 5000".to_string(),
            ));
        }
        let search_tokens = core_query_tokens(search_term)
            .map_err(|message| AppError::DataParsing(message.to_string()))?;
        let fts_query = (!search_tokens.is_empty()).then_some(search_term.trim().to_string());
        let token_predicate = if search_tokens.is_empty() {
            "TRUE".to_string()
        } else {
            let values = search_tokens
                .iter()
                .map(|token| format!("'{}'", token.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "(SELECT COUNT(DISTINCT document_search_tokens.token)
                   FROM documents
                   INNER JOIN document_search_tokens
                     ON document_search_tokens.document_id = documents.id
                    AND document_search_tokens.portal_subdomain = documents.portal_subdomain
                  WHERE documents.request_id = requests.id
                    AND documents.portal_subdomain = requests.portal_subdomain
                    AND document_search_tokens.token IN ({values})) = {}",
                search_tokens.len()
            )
        };
        let agency_id = agency_id.map(i64::try_from).transpose().map_err(|_| {
            AppError::DataParsing("Agency identifier exceeds SQLite capacity".to_string())
        })?;
        let limit = i64::try_from(limit).map_err(|_| {
            AppError::DataParsing("Search limit exceeds SQLite capacity".to_string())
        })?;
        let portal_key = portal_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let status = status
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let policy_deviation = policy_deviation
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        self.query_connection("Request search", move |conn| {
            let query = format!(
                "SELECT
                    requests.id,
                    requests.portal_subdomain,
                    requests.agency_id,
                    COALESCE(agencies.name, 'Unmapped Department'),
                    COALESCE(requests.title, ''),
                    COALESCE(requests.description, ''),
                    requests.status,
                    requests.created_at,
                    requests.closed_at,
                    (SELECT COUNT(*) FROM documents
                     WHERE documents.request_id = requests.id
                       AND documents.portal_subdomain = requests.portal_subdomain
                       AND LOWER(TRIM(documents.visibility)) = 'public'),
                    COALESCE((
                        SELECT GROUP_CONCAT(distinct_flags.flag_type, chr(31))
                        FROM (
                            SELECT DISTINCT document_integrity_flags.flag_type
                            FROM case_audits
                            INNER JOIN document_integrity_flags
                              ON document_integrity_flags.audit_id = case_audits.audit_id
                            WHERE case_audits.target_request_id = requests.id
                              AND case_audits.target_subdomain = requests.portal_subdomain
                            ORDER BY document_integrity_flags.flag_type
                        ) AS distinct_flags
                    ), '')
                 FROM requests
                 LEFT JOIN agencies
                   ON agencies.id = requests.agency_id
                  AND agencies.portal_subdomain = requests.portal_subdomain
                 WHERE (?1 IS NULL OR requests.portal_subdomain = ?1)
                   AND (?2 IS NULL OR requests.agency_id = ?2)
                   AND (?3 IS NULL OR LOWER(requests.status) = LOWER(?3))
                   AND (?4 IS NULL OR EXISTS (
                        SELECT 1
                        FROM case_audits
                        INNER JOIN document_integrity_flags
                          ON document_integrity_flags.audit_id = case_audits.audit_id
                        WHERE case_audits.target_request_id = requests.id
                          AND case_audits.target_subdomain = requests.portal_subdomain
                          AND document_integrity_flags.flag_type = ?4
                   ))
                   AND (?5 IS NULL OR {token_predicate})
                 ORDER BY requests.created_at DESC, requests.portal_subdomain, requests.id
                 LIMIT ?6"
            );
            let mut statement = conn.prepare(&query).map_err(|error| {
                AppError::Storage(format!("Failed to prepare request search: {error}"))
            })?;
            let rows = statement
                .query_map(
                    params![
                        portal_key,
                        agency_id,
                        status,
                        policy_deviation,
                        fts_query,
                        limit
                    ],
                    map_request_search_row,
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to execute request search: {error}"))
                })?;
            let mut results = Vec::new();
            for row in rows {
                let row = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode request search result: {error}"))
                })?;
                let title = crate::storage::pii::mask_pii(&row.title)?.masked_text;
                let description = crate::storage::pii::mask_pii(&row.description)?.masked_text;
                let agency_name = crate::storage::pii::mask_pii(&row.agency_name)?.masked_text;
                results.push(RequestSearchResult {
                    request_id: row.request_id,
                    portal_key: row.portal_key,
                    agency_id: row.agency_id,
                    agency_name,
                    title,
                    description_preview: description.chars().take(320).collect(),
                    status: row.status,
                    created_at: row.created_at,
                    closed_at: row.closed_at,
                    document_count: usize::try_from(row.document_count).map_err(|_| {
                        AppError::Storage("Document count was negative".to_string())
                    })?,
                    integrity_flags: row
                        .integrity_flags
                        .split('\u{1f}')
                        .filter(|value| !value.is_empty())
                        .map(str::to_string)
                        .collect(),
                });
            }
            Ok(results)
        })
        .await
    }

    pub async fn list_request_documents_for_view(
        &self,
        request_id: u64,
        portal_key: &str,
    ) -> Result<Vec<RequestDocumentSummary>, AppError> {
        let portal_key = portal_key.to_string();
        let rows = self
            .query_connection("Request document listing", move |conn| {
                let mut statement = conn
                    .prepare(
                        "SELECT
                    id, portal_subdomain, COALESCE(file_name, 'Untitled document'),
                    file_size, content_type, visibility, COALESCE(download_status, 'pending'),
                    raw_file_sha256, extracted_text_sha256,
                    COALESCE(extraction_status, 'PENDING'), extraction_format,
                    extraction_deferred_reason
                 FROM documents
                 WHERE request_id = ?1
                   AND portal_subdomain = ?2
                   AND LOWER(TRIM(visibility)) = 'public'
                 ORDER BY created_at, id",
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to prepare request document list: {error}"
                        ))
                    })?;
                let rows = statement
                    .query_map(params![request_id, portal_key], map_request_document_row)
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to query request documents: {error}"))
                    })?;
                rows.map(|row| {
                    row.map_err(|error| {
                        AppError::Storage(format!("Failed to decode request document: {error}"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
            })
            .await?;
        let mut documents = Vec::with_capacity(rows.len());
        for row in rows {
            let file_size = row
                .file_size
                .map(u64::try_from)
                .transpose()
                .map_err(|_| AppError::Storage("Document size was negative".to_string()))?;
            documents.push(RequestDocumentSummary {
                document_id: row.document_id,
                portal_key: row.portal_key,
                file_name: crate::storage::pii::mask_pii(&row.file_name)?.masked_text,
                file_size,
                content_type: row.content_type,
                visibility: row.visibility,
                download_status: row.download_status,
                raw_file_sha256: row.raw_file_sha256,
                extracted_text_sha256: row.extracted_text_sha256,
                extraction_status: row.extraction_status,
                extraction_format: row.extraction_format,
                extraction_deferred_reason: row.extraction_deferred_reason,
            });
        }
        Ok(documents)
    }

    pub async fn resolve_public_document_path(
        &self,
        document_id: u64,
        portal_key: &str,
    ) -> Result<PathBuf, AppError> {
        let portal_key = portal_key.to_string();
        let error_portal_key = portal_key.clone();
        let local_path = self.query_connection("Public document path lookup", move |conn| {
            conn.query_row(
                "SELECT local_path
                 FROM documents
                 WHERE id = ?1
                   AND portal_subdomain = ?2
                   AND LOWER(TRIM(visibility)) = 'public'",
                params![document_id, portal_key],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|error| match error {
                duckdb::Error::QueryReturnedNoRows => AppError::Storage(format!(
                    "Public document {error_portal_key}/{document_id} was not found"
                )),
                other => AppError::Storage(format!(
                    "Failed to resolve local evidence path for {error_portal_key}/{document_id}: {other}"
                )),
            })?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                AppError::Storage(format!(
                    "Public document {error_portal_key}/{document_id} is not available in the local evidence store"
                ))
            })
        }).await?;
        Ok(PathBuf::from(local_path))
    }

    pub async fn load_document_text_chunk(
        &self,
        document_id: u64,
        portal_key: &str,
        character_offset: usize,
        character_limit: usize,
    ) -> Result<DocumentTextChunk, AppError> {
        if !(1..=MAXIMUM_DOCUMENT_CHUNK_CHARACTERS).contains(&character_limit) {
            return Err(AppError::DataParsing(format!(
                "Document chunk size must be between 1 and {MAXIMUM_DOCUMENT_CHUNK_CHARACTERS} characters"
            )));
        }
        let requested_character_offset = character_offset;
        let requested_character_limit = character_limit;
        // Inspect context outside the requested window so a PII token spanning either boundary is
        // detected before the response is trimmed back to the caller's range.
        let query_character_offset =
            requested_character_offset.saturating_sub(DOCUMENT_CHUNK_PII_BOUNDARY_CHARACTERS);
        let query_character_limit = character_limit
            .checked_add(DOCUMENT_CHUNK_PII_BOUNDARY_CHARACTERS * 2)
            .ok_or_else(|| {
                AppError::DataParsing("Document chunk boundary overflowed".to_string())
            })?;
        let character_offset = i64::try_from(query_character_offset).map_err(|_| {
            AppError::DataParsing("Document text offset exceeds SQLite capacity".to_string())
        })?;
        let character_limit = i64::try_from(query_character_limit).map_err(|_| {
            AppError::DataParsing("Document text limit exceeds SQLite capacity".to_string())
        })?;
        let portal_key = portal_key.to_string();
        let query_portal_key = portal_key.clone();
        let (total_character_count, text) = self
            .query_connection("Document text chunk query", move |conn| {
                conn.query_row(
                    "SELECT
                    COALESCE((
                        SELECT LENGTH(documents_fts.extracted_text)
                        FROM documents_fts
                        WHERE documents_fts.document_id = documents.id
                          AND documents_fts.portal_subdomain = documents.portal_subdomain
                        ORDER BY documents_fts.document_id
                        LIMIT 1
                    ), 0),
                    COALESCE((
                        SELECT SUBSTR(documents_fts.extracted_text, ?3 + 1, ?4)
                        FROM documents_fts
                        WHERE documents_fts.document_id = documents.id
                          AND documents_fts.portal_subdomain = documents.portal_subdomain
                        ORDER BY documents_fts.document_id
                        LIMIT 1
                    ), '')
                 FROM documents
                 WHERE documents.id = ?1
                   AND documents.portal_subdomain = ?2
                   AND LOWER(TRIM(documents.visibility)) = 'public'",
                    params![
                        document_id,
                        query_portal_key,
                        character_offset,
                        character_limit
                    ],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
                )
                .map_err(|error| match error {
                    duckdb::Error::QueryReturnedNoRows => AppError::Storage(format!(
                        "Public document {query_portal_key}/{document_id} was not found"
                    )),
                    other => AppError::Storage(format!("Failed to load document text: {other}")),
                })
            })
            .await?;
        let total_character_count = usize::try_from(total_character_count).map_err(|_| {
            AppError::Storage("Document text length was negative or too large".to_string())
        })?;
        if requested_character_offset > total_character_count {
            return Err(AppError::DataParsing(format!(
                "Document text offset {requested_character_offset} exceeds length {total_character_count}"
            )));
        }
        let returned_character_count = requested_character_limit
            .min(total_character_count.saturating_sub(requested_character_offset));
        let next_offset = requested_character_offset
            .checked_add(returned_character_count)
            .ok_or_else(|| AppError::Storage("Document text position overflowed".to_string()))?;
        let visible_character_start = requested_character_offset - query_character_offset;
        let masked = crate::storage::pii::mask_pii_character_range(
            &text,
            visible_character_start,
            returned_character_count,
        )?;
        Ok(DocumentTextChunk {
            document_id,
            portal_key,
            character_offset: requested_character_offset,
            returned_character_count,
            next_character_offset: (next_offset < total_character_count).then_some(next_offset),
            total_character_count,
            masked_finding_count: masked.findings.len(),
            masked_text: masked.masked_text,
        })
    }

    /// Atomically finalizes document metadata, derived audit flags, and the search-text projection.
    pub async fn finalize_document_extraction(
        &self,
        finalization: DocumentExtractionFinalization<'_>,
    ) -> Result<(), AppError> {
        let DocumentExtractionFinalization {
            document_id: doc_id,
            portal_subdomain: subdomain,
            extracted_text: text_content,
            extracted_text_sha256: sha256_hash,
            raw_file_sha256,
            was_ocr_processed: was_ocr,
            raw_file_size,
            local_path,
            shannon_entropy,
            entropy_sample_bytes,
            suspicious_data_density,
        } = finalization;
        let document_id = i64::try_from(doc_id).map_err(|_| {
            AppError::DataParsing(format!(
                "Document identifier {} exceeds SQLite INTEGER capacity",
                doc_id
            ))
        })?;
        let stored_file_size = i64::try_from(raw_file_size).map_err(|_| {
            AppError::DataParsing(format!(
                "Raw file size {} exceeds SQLite INTEGER capacity",
                raw_file_size
            ))
        })?;
        let stored_path = local_path
            .to_str()
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| {
                AppError::DataParsing(
                    "Document extraction disk path must be non-empty, valid UTF-8".to_string(),
                )
            })?;
        let normalized_sha256 = validate_extracted_text_sha256(text_content, sha256_hash)?;
        let normalized_raw_file_sha256 = normalize_sha256(raw_file_sha256)?;
        let cleaned_text = clean_plain_text_for_index(text_content);
        // `extracted_text_sha256` covers `text_content`, which is discarded after this call: only
        // the normalized projection is kept. That digest is therefore checkable at write time and
        // never again, while the text search returns and legal briefs quote had no digest at all.
        // Recording one over the projection makes the derived evidence verifiable later.
        let projection_sha256 = compute_extracted_text_sha256(&cleaned_text);
        let (entropy_sample_bytes, suspicious_data_density) = validate_entropy_metadata(
            shannon_entropy,
            entropy_sample_bytes,
            raw_file_size,
            suspicious_data_density,
        )?;

        if subdomain.trim().is_empty() {
            return Err(AppError::DataParsing(
                "Document portal subdomain must not be empty".to_string(),
            ));
        }

        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "document extraction finalization", |tx| {
            let analytical_state = tx
                .query_row(
                    "SELECT
                requests.id,
                requests.created_at,
                requests.updated_at,
                requests.closed_at,
                requests.status,
                documents.file_name
             FROM documents
             INNER JOIN requests
                ON requests.id = documents.request_id
               AND requests.portal_subdomain = documents.portal_subdomain
             WHERE documents.id = ?1 AND documents.portal_subdomain = ?2",
                    params![document_id, subdomain],
                    map_request_analytical_state,
                )
                .map_err(|error| match error {
                    duckdb::Error::QueryReturnedNoRows => AppError::Internal(format!(
                        "Document {} for portal '{}' does not exist or has no associated request",
                        doc_id, subdomain
                    )),
                    other => AppError::Internal(format!(
                        "Failed to load document analytical context: {}",
                        other
                    )),
                })?;

            let request_has_heavy_redaction = if has_heavy_redaction_density(text_content)? {
                true
            } else {
                let mut statement = tx
                    .prepare(
                        "SELECT documents_fts.extracted_text
                 FROM documents
                 INNER JOIN documents_fts
                    ON documents_fts.document_id = documents.id
                   AND documents_fts.portal_subdomain = documents.portal_subdomain
                 WHERE documents.request_id = ?1
                   AND documents.portal_subdomain = ?2
                   AND documents.id <> ?3",
                    )
                    .map_err(|e| {
                        AppError::Internal(format!(
                            "Failed to prepare request redaction scan: {}",
                            e
                        ))
                    })?;
                let indexed_text_rows = statement
                    .query_map(
                        params![analytical_state.request_id, subdomain, document_id],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|e| {
                        AppError::Internal(format!(
                            "Failed to execute request redaction scan: {}",
                            e
                        ))
                    })?;
                let mut heavy_redaction_found = false;
                for indexed_text in indexed_text_rows {
                    let indexed_text = indexed_text.map_err(|e| {
                        AppError::Internal(format!(
                            "Failed to decode indexed text during redaction scan: {}",
                            e
                        ))
                    })?;
                    if has_heavy_redaction_density(&indexed_text)? {
                        heavy_redaction_found = true;
                        break;
                    }
                }
                heavy_redaction_found
            };
            let audit_id = format!("auto-{}-{}", subdomain, analytical_state.request_id);

            let updated_documents = tx
                .execute(
                    "UPDATE documents SET
                file_size = ?1,
                local_path = ?2,
                extracted_text_sha256 = ?3,
                raw_file_sha256 = ?4,
                was_ocr_processed = ?5,
                shannon_entropy = ?6,
                entropy_sample_bytes = ?7,
                suspicious_data_density = ?8,
                download_status = ?9,
                extracted_text_projection_sha256 = ?12,
                extraction_status = 'SEARCHABLE', extraction_format = NULL,
                extraction_deferred_reason = NULL
             WHERE id = ?10 AND portal_subdomain = ?11",
                    params![
                        stored_file_size,
                        stored_path,
                        normalized_sha256,
                        normalized_raw_file_sha256,
                        if was_ocr { 1_i64 } else { 0_i64 },
                        shannon_entropy,
                        entropy_sample_bytes,
                        suspicious_data_density,
                        DOWNLOAD_STATUS_COMPLETED,
                        document_id,
                        subdomain,
                        projection_sha256,
                    ],
                )
                .map_err(|e| {
                    AppError::Internal(format!("Failed to update document metadata record: {}", e))
                })?;

            if updated_documents != 1 {
                return Err(AppError::Internal(format!(
                    "Document extraction update affected {} rows for document {} in portal '{}'",
                    updated_documents, doc_id, subdomain
                )));
            }

            let request_has_suspicious_data_density =
                request_has_suspicious_density(tx, analytical_state.request_id, subdomain)?;
            let derived_flags = derive_integrity_flags(
                &analytical_state,
                request_has_heavy_redaction,
                request_has_suspicious_data_density,
            );
            let verification_status = if derived_flags.is_empty() {
                VERIFICATION_STATUS_COMPLIANT
            } else {
                VERIFICATION_STATUS_ANOMALY
            };

            upsert_automatic_case_audit(
                tx,
                &audit_id,
                analytical_state.request_id,
                subdomain,
                "Automatically derived during document extraction finalization",
                verification_status,
                true,
            )
            .map_err(|error| {
                AppError::Internal(format!(
                    "Failed to synchronize automatic case audit state: {error}"
                ))
            })?;

            replace_managed_integrity_flags(
                tx,
                &audit_id,
                &DERIVED_DOCUMENT_FLAG_TYPES,
                &derived_flags,
            )?;

            synchronize_fts_row(
                tx,
                document_id,
                subdomain,
                analytical_state.file_name.as_deref(),
                &cleaned_text,
            )?;
            let request_id = u64::try_from(analytical_state.request_id).map_err(|_| {
                AppError::Storage("Document request identifier was negative".to_string())
            })?;
            synchronize_policy_evidence_gaps(tx, request_id, subdomain)?;
            update_automatic_audit_status(tx, &audit_id)?;
            Ok(())
        })
    }

    /// The owning agency's name and department, joined into one searchable label, for the
    /// request a document belongs to. `None` covers both "the request has no agency on file" and
    /// "the request row itself is not found" -- either way there is no organization signal to
    /// evaluate against, and callers must treat that as the predicate contributing nothing rather
    /// than as a reason to exclude.
    pub async fn agency_label_for_request(
        &self,
        subdomain: &str,
        request_id: u64,
    ) -> Result<Option<String>, AppError> {
        let request_id = i64::try_from(request_id).map_err(|error| {
            AppError::DataParsing(format!(
                "Request identifier cannot be represented by SQLite: {error}"
            ))
        })?;
        let subdomain = subdomain.to_string();
        self.query_connection("Request agency label lookup", move |conn| {
            let row = conn
                .query_row(
                    "SELECT agencies.name, agencies.department_name
                     FROM requests
                     LEFT JOIN agencies
                        ON agencies.id = requests.agency_id
                       AND agencies.portal_subdomain = requests.portal_subdomain
                     WHERE requests.id = ?1 AND requests.portal_subdomain = ?2",
                    params![request_id, subdomain],
                    |row| {
                        let name: Option<String> = row.get(0)?;
                        let department: Option<String> = row.get(1)?;
                        Ok((name, department))
                    },
                )
                .optional()
                .map_err(|error| {
                    AppError::Internal(format!("Failed to look up request agency label: {error}"))
                })?;
            Ok(row.and_then(|(name, department)| match (name, department) {
                (Some(name), Some(department)) if !department.trim().is_empty() => {
                    Some(format!("{name} {department}"))
                }
                (Some(name), _) => Some(name),
                (None, Some(department)) => Some(department),
                (None, None) => None,
            }))
        })
        .await
    }

    /// Discards a document row that guidance refused after extraction, before any extracted text
    /// was ever committed for it. Scoped to that narrow case: a document with no `documents_fts`
    /// row, which is exactly the state a stub row is in between `insert_or_update_document` and a
    /// successful `finalize_document_extraction`. The `NOT EXISTS` guard makes that precondition
    /// real rather than documentation-only: a resumed sync re-extracts every document regardless
    /// of prior completion, and a guidance edit between syncs must not retroactively purge a
    /// document a prior sync already finalized as evidence -- `documents_fts` carries no foreign
    /// key back to `documents`, so an unconditional delete here would orphan it (and a stale
    /// corpus index entry) silently. Returns whether a row was actually removed; `false` means
    /// the document was already finalized and was left in place, and callers must not count that
    /// as a refusal. `provenance_manifests` needs no matching guard: its foreign key to
    /// `documents` already makes DuckDB refuse the delete outright if a manifest was recorded.
    pub async fn delete_unextracted_document(
        &self,
        subdomain: &str,
        document_id: u64,
    ) -> Result<bool, AppError> {
        let document_id = i64::try_from(document_id).map_err(|error| {
            AppError::DataParsing(format!(
                "Document identifier cannot be represented by SQLite: {error}"
            ))
        })?;
        let conn = self.conn.lock().await;
        let removed = conn
            .execute(
                "DELETE FROM documents
                 WHERE id = ?1 AND portal_subdomain = ?2
                   AND NOT EXISTS (
                     SELECT 1 FROM documents_fts
                     WHERE documents_fts.document_id = documents.id
                       AND documents_fts.portal_subdomain = documents.portal_subdomain
                   )",
                params![document_id, subdomain],
            )
            .map_err(|error| {
                AppError::Internal(format!("Failed to discard refused document: {error}"))
            })?;
        Ok(removed > 0)
    }

    /// Submits manual case verification details and updates investigator flags.
    pub async fn record_manual_case_audit(
        &self,
        audit_id: &str,
        request_id: u64,
        subdomain: &str,
        notes: Option<&str>,
        status: &str,
        integrity_flags: &[String],
    ) -> Result<(), AppError> {
        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "manual case audit", |tx| {
            tx.execute(
            "INSERT INTO case_audits (audit_id, target_request_id, target_subdomain, auditor_notes, verified_at, verification_status)
             VALUES (?1, ?2, ?3, ?4, strftime(current_timestamp, '%Y-%m-%dT%H:%M:%SZ'), ?5)
             ON CONFLICT(audit_id) DO UPDATE SET
                auditor_notes = excluded.auditor_notes,
                verified_at = excluded.verified_at,
                verification_status = excluded.verification_status",
            params![audit_id, request_id, subdomain, notes, status],
            ).map_err(|e| AppError::Internal(format!("Audit envelope update statement failure: {}", e)))?;

            tx.execute(
                "DELETE FROM document_integrity_flags WHERE audit_id = ?1",
                params![audit_id],
            )
            .map_err(|e| {
                AppError::Internal(format!("Failed to clear stale integrity parameters: {}", e))
            })?;

            let mut insert = tx
                .prepare_cached(
                    "INSERT INTO document_integrity_flags (audit_id, flag_type) VALUES (?1, ?2)",
                )
                .map_err(|e| {
                    AppError::Internal(format!(
                        "Failed to prepare case integrity parameters: {}",
                        e
                    ))
                })?;
            for flag in integrity_flags {
                insert.execute(params![audit_id, flag]).map_err(|e| {
                    AppError::Internal(format!("Failed to map case integrity parameters: {}", e))
                })?;
            }
            Ok(())
        })
    }

    /// Every source the operator still holds, paused ones included.
    ///
    /// A withdrawn source is excluded here rather than deleted: the `portals` row is the parent of
    /// the requests and documents ingested from it, so removing it would take the evidence with
    /// it. Withdrawal is therefore a state on the row, and this listing is the one place that
    /// state decides visibility.
    pub async fn list_portal_registrations(&self) -> Result<Vec<SavedSource>, AppError> {
        self.query_connection("Portal registration listing", |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT subdomain, display_name, base_url, origin_url, requests_endpoint,
                    requests_pointer, metadata_pointer, page_parameter, page_size_parameter,
                    COALESCE(verified_at, created_at), source_kind,
                    COALESCE(lifecycle_state, 'ACTIVE'),
                    COALESCE(health_status, 'verified'), last_error
                 FROM portals
                 WHERE requests_endpoint <> ''
                   AND COALESCE(lifecycle_state, 'ACTIVE') <> 'REMOVED'
                 ORDER BY display_name COLLATE NOCASE",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare portal listing: {error}"))
                })?;
            let rows = statement
                .query_map([], |row| {
                    Ok(SavedSource {
                        registration: PortalRegistration {
                            portal_key: row.get(0)?,
                            display_name: row.get(1)?,
                            canonical_url: row.get(2)?,
                            origin_url: row.get(3)?,
                            api_layout: crate::types::PortalApiLayout {
                                requests_endpoint: row.get(4)?,
                                requests_pointer: row.get(5)?,
                                metadata_pointer: row.get(6)?,
                                page_parameter: row.get(7)?,
                                page_size_parameter: row.get(8)?,
                            },
                            verified_at: row.get(9)?,
                            source_kind: crate::types::SourceKind::from_stored(
                                &row.get::<_, String>(10)?,
                            ),
                        },
                        lifecycle_state: SourceLifecycleState::from_stored(
                            &row.get::<_, String>(11)?,
                        ),
                        health_status: row.get(12)?,
                        last_error: row.get(13)?,
                    })
                })
                .map_err(|error| AppError::Storage(format!("Failed to query portals: {error}")))?;
            rows.map(|row| {
                row.map_err(|error| AppError::Storage(format!("Failed to decode portal: {error}")))
            })
            .collect()
        })
        .await
    }

    pub async fn record_statutory_extension(
        &self,
        request_id: u64,
        portal_key: &str,
        extension: &StatutoryExtensionRecord,
    ) -> Result<(), AppError> {
        chrono::DateTime::parse_from_rfc3339(&extension.notice_at).map_err(|error| {
            AppError::DataParsing(format!("Invalid extension notice timestamp: {error}"))
        })?;
        chrono::DateTime::parse_from_rfc3339(&extension.expected_determination_at).map_err(
            |error| {
                AppError::DataParsing(format!("Invalid expected determination timestamp: {error}"))
            },
        )?;
        if extension.notice_source.trim().is_empty() {
            return Err(AppError::DataParsing(
                "Statutory extension evidence must identify its notice source".to_string(),
            ));
        }
        let conn = self.conn.lock().await;
        upsert_statutory_extension(&conn, request_id, portal_key, extension)
    }

    pub async fn refresh_request_timeline_audit(
        &self,
        request_id: u64,
        portal_key: &str,
    ) -> Result<Vec<String>, AppError> {
        let evidence = self
            .load_legal_brief_evidence(request_id, portal_key)
            .await?;
        let operational = crate::analysis::calculate_operational_latency_for_status(
            request_id,
            &evidence.status,
            &evidence.created_at,
            evidence.closed_at.as_deref(),
        )?;
        let statutory = crate::analysis::evaluate_statutory_compliance_with_determination_at(
            request_id,
            &evidence.created_at,
            Some(&evidence.updated_at),
            evidence.closed_at.as_deref(),
            evidence.determination_at.as_deref(),
            evidence.determination_source.as_deref(),
            evidence.extension.as_ref(),
            &chrono::Utc::now().to_rfc3339(),
        )?;
        let mut flags = operational
            .anomaly_markers
            .into_iter()
            .chain(statutory.anomaly_markers.iter().copied())
            .map(|marker| marker.as_str().to_string())
            .collect::<Vec<_>>();
        let created =
            chrono::DateTime::parse_from_rfc3339(&evidence.created_at).map_err(|error| {
                AppError::DataParsing(format!("Invalid request creation timestamp: {error}"))
            })?;
        let updated =
            chrono::DateTime::parse_from_rfc3339(&evidence.updated_at).map_err(|error| {
                AppError::DataParsing(format!("Invalid request update timestamp: {error}"))
            })?;
        if updated < created {
            flags.push(FLAG_UPDATE_BEFORE_CREATION.to_string());
        }
        flags.sort();
        flags.dedup();
        let known_flags = flags.iter().map(String::as_str).collect::<Vec<_>>();
        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "timeline audit refresh", |tx| {
            synchronize_timeline_audit(
                tx,
                &format!("auto-{portal_key}-{request_id}"),
                request_id,
                portal_key,
                &known_flags,
            )?;
            synchronize_cpra_finding(tx, request_id, portal_key, &statutory)
        })?;
        Ok(flags)
    }

    pub async fn import_auxiliary_events(
        &self,
        portal_key: Option<&str>,
        source_kind: AuxiliaryStreamKind,
        display_name: &str,
        source_uri: Option<&str>,
        events: &[AuxiliaryEventInput],
        source_sha256: &str,
    ) -> Result<AuxiliaryImportResult, AppError> {
        if display_name.trim().is_empty() || events.is_empty() {
            return Err(AppError::DataParsing(
                "Auxiliary imports require a display name and at least one event".to_string(),
            ));
        }
        let identity = format!(
            "{}|{}|{}|{}",
            portal_key.unwrap_or("global"),
            source_kind.as_str(),
            source_uri.unwrap_or("local-import"),
            source_sha256
        );
        let identity_sha256 = sha256_hex(identity.as_bytes());
        let source_id = format!("aux-{}", &identity_sha256[..32]);
        let imported_at = chrono::Utc::now().to_rfc3339();
        let mut accepted = Vec::with_capacity(events.len());
        let mut rejected_count = 0_usize;
        for event in events {
            if validate_auxiliary_event(event).is_ok() {
                accepted.push(event);
            } else {
                rejected_count += 1;
            }
        }
        if accepted.is_empty() {
            return Err(AppError::DataParsing(
                "Every auxiliary event failed structural validation".to_string(),
            ));
        }

        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "auxiliary import", |tx| {
            tx.execute(
                "INSERT INTO auxiliary_data_sources (
                source_id, portal_subdomain, source_kind, display_name, source_uri,
                source_sha256, imported_at, record_count
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(source_id) DO UPDATE SET
                display_name = excluded.display_name,
                source_uri = excluded.source_uri,
                imported_at = excluded.imported_at,
                record_count = excluded.record_count",
                params![
                    source_id,
                    portal_key,
                    source_kind.as_str(),
                    display_name.trim(),
                    source_uri,
                    source_sha256,
                    imported_at,
                    accepted.len(),
                ],
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to persist auxiliary source: {error}"))
            })?;

            let mut imported_count = 0_usize;
            let mut updated_count = 0_usize;
            for event in accepted {
                let existed = tx
                    .query_row(
                        "SELECT EXISTS(
                        SELECT 1 FROM auxiliary_events WHERE source_id = ?1 AND external_id = ?2
                     )",
                        params![source_id, event.external_id],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to inspect auxiliary event: {error}"))
                    })?;
                let source_payload =
                    serde_json::to_string(&event.source_payload).map_err(|error| {
                        AppError::DataParsing(format!(
                            "Failed to encode auxiliary payload: {error}"
                        ))
                    })?;
                tx.execute(
                    "INSERT INTO auxiliary_events (
                    source_id, external_id, occurred_at, neighborhood_block, event_type,
                    description, latitude, longitude, source_payload_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(source_id, external_id) DO UPDATE SET
                    occurred_at = excluded.occurred_at,
                    neighborhood_block = excluded.neighborhood_block,
                    event_type = excluded.event_type,
                    description = excluded.description,
                    latitude = excluded.latitude,
                    longitude = excluded.longitude,
                    source_payload_json = excluded.source_payload_json,
                    imported_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')",
                    params![
                        source_id,
                        event.external_id,
                        event.occurred_at,
                        normalize_neighborhood(&event.neighborhood_block),
                        event.event_type.trim(),
                        event.description,
                        event.latitude,
                        event.longitude,
                        source_payload,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to persist auxiliary event: {error}"))
                })?;
                if existed {
                    updated_count += 1;
                } else {
                    imported_count += 1;
                }
            }
            Ok(AuxiliaryImportResult {
                source_id,
                imported_count,
                updated_count,
                rejected_count,
                source_sha256: source_sha256.to_string(),
            })
        })
    }

    pub async fn import_legal_authority(
        &self,
        input: &LegalAuthorityImport,
        parsed: &ParsedLegalAuthority,
    ) -> Result<LegalAuthorityImportResult, AppError> {
        let imported_at = chrono::Utc::now().to_rfc3339();
        let statute_id = input.statute_id.trim();
        let source_uri = input
            .canonical_source_uri
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let effective_from = input
            .effective_from
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let effective_to = input
            .effective_to
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let mandated_deadline_days = input.mandated_deadline_days.map(i64::from);
        let inserted_new_version = {
            let mut conn = self.conn.lock().await;
            execute_transaction(&mut conn, "legal authority import", |tx| {
                let inserted_new_version = tx
                    .query_row(
                        "SELECT NOT EXISTS(
                    SELECT 1 FROM legal_statute_versions
                    WHERE statute_id = ?1 AND content_sha256 = ?2
                 )",
                        params![statute_id, parsed.content_sha256],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(|error| {
                        AppError::Storage(format!(
                            "Failed to inspect legal authority version: {error}"
                        ))
                    })?;

                tx.execute(
                    "INSERT INTO legal_statutes (
                statute_id, jurisdiction, statute_citation, title, authority_kind, full_text,
                mandated_deadline_days, canonical_source_uri, current_version_id,
                content_sha256, effective_from, effective_to, imported_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
             ON CONFLICT(statute_id) DO UPDATE SET
                jurisdiction = excluded.jurisdiction,
                statute_citation = excluded.statute_citation,
                title = excluded.title,
                authority_kind = excluded.authority_kind,
                full_text = excluded.full_text,
                mandated_deadline_days = excluded.mandated_deadline_days,
                canonical_source_uri = excluded.canonical_source_uri,
                current_version_id = excluded.current_version_id,
                content_sha256 = excluded.content_sha256,
                effective_from = excluded.effective_from,
                effective_to = excluded.effective_to,
                updated_at = excluded.updated_at",
                    params![
                        statute_id,
                        input.jurisdiction.trim(),
                        input.statute_citation.trim(),
                        input.title.trim(),
                        input.authority_kind.as_str(),
                        parsed.normalized_text,
                        mandated_deadline_days,
                        source_uri,
                        parsed.version_id,
                        parsed.content_sha256,
                        effective_from,
                        effective_to,
                        imported_at,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to persist legal authority: {error}"))
                })?;

                tx.execute(
                    "INSERT INTO legal_statute_versions (
                version_id, statute_id, full_text, content_sha256, effective_from,
                effective_to, source_uri, imported_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(version_id) DO UPDATE SET
                effective_from = excluded.effective_from,
                effective_to = excluded.effective_to,
                source_uri = excluded.source_uri",
                    params![
                        parsed.version_id,
                        statute_id,
                        parsed.normalized_text,
                        parsed.content_sha256,
                        effective_from,
                        effective_to,
                        source_uri,
                        imported_at,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to persist legal authority version: {error}"
                    ))
                })?;

                tx.execute(
                    "DELETE FROM legal_provisions_fts WHERE version_id = ?1",
                    params![parsed.version_id],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to clear legal provision index: {error}"))
                })?;
                tx.execute(
                    "DELETE FROM legal_provisions WHERE version_id = ?1",
                    params![parsed.version_id],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to clear legal provisions: {error}"))
                })?;
                for provision in &parsed.provisions {
                    let ordinal = i64::try_from(provision.ordinal).map_err(|_| {
                        AppError::Storage(
                            "Legal provision ordinal exceeds SQLite capacity".to_string(),
                        )
                    })?;
                    let heading_level = i64::try_from(provision.heading_level).map_err(|_| {
                        AppError::Storage("Legal heading level exceeds SQLite capacity".to_string())
                    })?;
                    let token_count = i64::try_from(provision.token_count).map_err(|_| {
                        AppError::Storage(
                            "Legal provision token count exceeds SQLite capacity".to_string(),
                        )
                    })?;
                    tx.execute(
                        "INSERT INTO legal_provisions (
                    provision_id, version_id, statute_id, ordinal, heading_level,
                    heading_path, title, provision_text, token_count, content_sha256
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![
                            provision.provision_id,
                            parsed.version_id,
                            statute_id,
                            ordinal,
                            heading_level,
                            provision.heading_path,
                            provision.title,
                            provision.provision_text,
                            token_count,
                            provision.content_sha256,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to persist legal provision: {error}"))
                    })?;
                    tx.execute(
                        "INSERT INTO legal_provisions_fts (
                    provision_id, version_id, statute_id, title, heading_path, provision_text
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            provision.provision_id,
                            parsed.version_id,
                            statute_id,
                            provision.title,
                            provision.heading_path,
                            provision.provision_text,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to index legal provision: {error}"))
                    })?;
                }

                tx.execute(
                    "UPDATE legal_mandates SET enabled = 0 WHERE statute_id = ?1",
                    params![statute_id],
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to retire superseded legal mandates: {error}"
                    ))
                })?;
                let version_suffix = parsed
                    .version_id
                    .rsplit('-')
                    .next()
                    .unwrap_or(parsed.version_id.as_str());
                for mandate in &input.mandates {
                    let mandate_id = format!(
                        "{}:{}:{}",
                        statute_id,
                        mandate.mandate_id.trim(),
                        &version_suffix[..version_suffix.len().min(12)]
                    );
                    let applicability_json = serde_json::to_string(&mandate.applicability_phrases)
                        .map_err(|error| {
                            AppError::DataParsing(format!(
                                "Failed to encode mandate applicability phrases: {error}"
                            ))
                        })?;
                    let evidence_json = serde_json::to_string(&mandate.required_evidence_phrases)
                        .map_err(|error| {
                        AppError::DataParsing(format!(
                            "Failed to encode mandate evidence phrases: {error}"
                        ))
                    })?;
                    tx.execute(
                        "INSERT INTO legal_mandates (
                    mandate_id, statute_id, version_id, title, description,
                    applicability_phrases_json, required_evidence_phrases_json,
                    severity, enabled, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9)
                 ON CONFLICT(mandate_id) DO UPDATE SET
                    title = excluded.title,
                    description = excluded.description,
                    applicability_phrases_json = excluded.applicability_phrases_json,
                    required_evidence_phrases_json = excluded.required_evidence_phrases_json,
                    severity = excluded.severity,
                    enabled = 1",
                        params![
                            mandate_id,
                            statute_id,
                            parsed.version_id,
                            mandate.title.trim(),
                            mandate.description.trim(),
                            applicability_json,
                            evidence_json,
                            mandate.severity.as_str(),
                            imported_at,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to persist legal mandate: {error}"))
                    })?;
                }
                Ok(inserted_new_version)
            })?
        };
        let authority = self.load_legal_authority_summary(statute_id).await?;
        Ok(LegalAuthorityImportResult {
            authority,
            inserted_new_version,
        })
    }

    pub async fn list_legal_authorities(&self) -> Result<Vec<LegalAuthoritySummary>, AppError> {
        let conn = self.conn.lock().await;
        let query = format!(
            "{LEGAL_AUTHORITY_SUMMARY_SELECT}
             ORDER BY legal_statutes.jurisdiction COLLATE NOCASE,
                      legal_statutes.statute_citation COLLATE NOCASE"
        );
        let mut statement = conn.prepare(&query).map_err(|error| {
            AppError::Storage(format!(
                "Failed to prepare legal authority listing: {error}"
            ))
        })?;
        let rows = statement
            .query_map([], map_legal_authority_summary_row)
            .map_err(|error| {
                AppError::Storage(format!("Failed to query legal authorities: {error}"))
            })?;
        rows.map(|row| {
            row.map_err(|error| {
                AppError::Storage(format!("Failed to decode legal authority: {error}"))
            })
            .and_then(convert_legal_authority_summary)
        })
        .collect()
    }

    pub async fn list_legal_authority_versions(
        &self,
        statute_id: &str,
    ) -> Result<Vec<LegalAuthorityVersionSummary>, AppError> {
        let conn = self.conn.lock().await;
        let mut statement = conn
            .prepare(
                "SELECT version_id, statute_id, content_sha256, effective_from, effective_to,
                        source_uri, imported_at
                 FROM legal_statute_versions
                 WHERE statute_id = ?1
                 ORDER BY imported_at DESC, version_id DESC
                 LIMIT 500",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to prepare legal version listing: {error}"))
            })?;
        let rows = statement
            .query_map(params![statute_id], map_legal_authority_version_summary)
            .map_err(|error| {
                AppError::Storage(format!("Failed to query legal versions: {error}"))
            })?;
        rows.map(|row| {
            row.map_err(|error| {
                AppError::Storage(format!("Failed to decode legal version: {error}"))
            })
        })
        .collect()
    }

    pub async fn get_legal_authority_version(
        &self,
        statute_id: &str,
        version_id: &str,
    ) -> Result<LegalAuthorityVersionDetail, AppError> {
        let conn = self.conn.lock().await;
        conn.query_row(
            "SELECT version_id, statute_id, content_sha256, effective_from, effective_to,
                    source_uri, imported_at, full_text
             FROM legal_statute_versions
             WHERE statute_id = ?1 AND version_id = ?2",
            params![statute_id, version_id],
            |row| {
                Ok((
                    map_legal_authority_version_summary(row)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            AppError::Storage(format!("Failed to read legal authority version: {error}"))
        })?
        .map(|(version, full_text)| LegalAuthorityVersionDetail { version, full_text })
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "Legal authority version '{version_id}' was not found for '{statute_id}'"
            ))
        })
    }

    pub async fn evaluate_request_legal_compliance(
        &self,
        request_id: u64,
        portal_key: &str,
    ) -> Result<LegalEvaluationResult, AppError> {
        let state = {
            let mut conn = self.conn.lock().await;
            execute_transaction(&mut conn, "policy evaluation", |tx| {
                synchronize_policy_evidence_gaps(tx, request_id, portal_key)
            })?
        };
        let findings = self
            .list_request_legal_findings(request_id, portal_key)
            .await?;
        Ok(LegalEvaluationResult {
            request_id,
            portal_key: portal_key.to_string(),
            evaluated_mandate_count: state.evaluated_mandate_count,
            applicable_mandate_count: state.applicable_mandate_count,
            corpus_character_count: state.corpus_character_count,
            evaluation_complete: state.evaluation_complete,
            warnings: state.warnings,
            findings,
        })
    }

    pub async fn list_request_legal_findings(
        &self,
        request_id: u64,
        portal_key: &str,
    ) -> Result<Vec<LegalComplianceFinding>, AppError> {
        let conn = self.conn.lock().await;
        let mut statement = conn
            .prepare(
                "SELECT
                    statutory_violations.violation_id, statutory_violations.request_id,
                    statutory_violations.portal_subdomain,
                    statutory_violations.matched_statute_id,
                    statutory_violations.matched_version_id,
                    statutory_violations.mandate_id,
                    legal_statutes.statute_citation, legal_statutes.title,
                    legal_mandates.title, statutory_violations.classification,
                    statutory_violations.calculated_variance_metric,
                    statutory_violations.variance_days,
                    statutory_violations.confidence_basis_points,
                    statutory_violations.evidence_summary,
                    statutory_violations.supporting_document_ids_json,
                    statutory_violations.missing_evidence_json,
                    statutory_violations.review_status,
                    statutory_violations.first_detected_at,
                    statutory_violations.evaluated_at
                 FROM statutory_violations
                 INNER JOIN legal_statutes
                    ON legal_statutes.statute_id = statutory_violations.matched_statute_id
                 LEFT JOIN legal_mandates
                    ON legal_mandates.mandate_id = statutory_violations.mandate_id
                 WHERE statutory_violations.request_id = ?1
                   AND statutory_violations.portal_subdomain = ?2
                 ORDER BY
                    CASE statutory_violations.review_status
                        WHEN 'OPEN' THEN 0 WHEN 'CONFIRMED' THEN 1
                        WHEN 'RESOLVED_BY_NEW_EVIDENCE' THEN 2 ELSE 3
                    END,
                    statutory_violations.confidence_basis_points DESC,
                    statutory_violations.evaluated_at DESC",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to prepare legal findings query: {error}"))
            })?;
        let rows = statement
            .query_map(params![request_id, portal_key], map_legal_finding_row)
            .map_err(|error| {
                AppError::Storage(format!("Failed to query legal findings: {error}"))
            })?;
        rows.map(|row| {
            row.map_err(|error| {
                AppError::Storage(format!("Failed to decode legal finding: {error}"))
            })
            .and_then(convert_legal_finding)
        })
        .collect()
    }

    pub async fn review_legal_finding(
        &self,
        violation_id: &str,
        request_id: u64,
        portal_key: &str,
        review_status: LegalFindingReviewStatus,
        reviewer_notes: Option<&str>,
    ) -> Result<Vec<LegalComplianceFinding>, AppError> {
        {
            let mut conn = self.conn.lock().await;
            execute_transaction(&mut conn, "legal finding review", |tx| {
                let classification = tx
                    .query_row(
                        "SELECT classification
                     FROM statutory_violations
                     WHERE violation_id = ?1
                       AND request_id = ?2
                       AND portal_subdomain = ?3",
                        params![violation_id, request_id, portal_key],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|error| match error {
                        duckdb::Error::QueryReturnedNoRows => AppError::Storage(format!(
                        "Legal finding '{violation_id}' was not found for {portal_key}/{request_id}"
                    )),
                        other => AppError::Storage(format!(
                            "Failed to load the legal finding for review: {other}"
                        )),
                    })?;
                let updated = tx
                    .execute(
                        "UPDATE statutory_violations
                     SET review_status = ?1,
                         reviewed_at = CASE
                            WHEN ?1 = 'OPEN' THEN NULL
                            ELSE strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')
                         END,
                         reviewer_notes = ?2
                     WHERE violation_id = ?3
                       AND request_id = ?4
                       AND portal_subdomain = ?5",
                        params![
                            review_status.as_str(),
                            reviewer_notes,
                            violation_id,
                            request_id,
                            portal_key,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to record legal finding review: {error}"))
                    })?;
                if updated != 1 {
                    return Err(AppError::Storage(format!(
                        "Legal finding review affected {updated} records"
                    )));
                }
                if classification == FLAG_POTENTIAL_POLICY_VIOLATION {
                    synchronize_policy_audit_alert(tx, request_id, portal_key)?;
                }
                Ok(())
            })?;
        }
        self.list_request_legal_findings(request_id, portal_key)
            .await
    }

    async fn load_legal_authority_summary(
        &self,
        statute_id: &str,
    ) -> Result<LegalAuthoritySummary, AppError> {
        let conn = self.conn.lock().await;
        let query = format!(
            "{LEGAL_AUTHORITY_SUMMARY_SELECT}
             WHERE legal_statutes.statute_id = ?1"
        );
        let row = conn
            .query_row(&query, params![statute_id], map_legal_authority_summary_row)
            .map_err(|error| match error {
                duckdb::Error::QueryReturnedNoRows => {
                    AppError::Storage(format!("Legal authority '{statute_id}' was not found"))
                }
                other => AppError::Storage(format!("Failed to load legal authority: {other}")),
            })?;
        convert_legal_authority_summary(row)
    }

    pub async fn load_legal_brief_evidence(
        &self,
        request_id: u64,
        portal_key: &str,
    ) -> Result<LegalBriefEvidence, AppError> {
        let conn = self.conn.lock().await;
        let row = conn
            .query_row(
                "SELECT
                    requests.id, requests.portal_subdomain, portals.display_name,
                    portals.base_url, COALESCE(agencies.name, 'Unmapped Department'),
                    requests.rpid, COALESCE(requests.title, ''),
                    COALESCE(requests.description, ''), requests.status,
                    requests.created_at, requests.updated_at, requests.closed_at,
                    requests.determination_at, requests.determination_source,
                    statutory_extensions.notice_at,
                    statutory_extensions.expected_determination_at,
                    statutory_extensions.reason, statutory_extensions.notice_source
                 FROM requests
                 INNER JOIN portals ON portals.subdomain = requests.portal_subdomain
                 LEFT JOIN agencies
                    ON agencies.id = requests.agency_id
                   AND agencies.portal_subdomain = requests.portal_subdomain
                LEFT JOIN statutory_extensions
                    ON statutory_extensions.request_id = requests.id
                   AND statutory_extensions.portal_subdomain = requests.portal_subdomain
                 WHERE requests.id = ?1 AND requests.portal_subdomain = ?2",
                params![request_id, portal_key],
                map_legal_brief_row,
            )
            .map_err(|error| match error {
                duckdb::Error::QueryReturnedNoRows => {
                    AppError::Storage(format!("Request {portal_key}/{request_id} was not found"))
                }
                other => AppError::Storage(format!("Failed to load legal evidence: {other}")),
            })?;
        let extension = match (
            row.extension_notice_at,
            row.extension_expected_determination_at,
            row.extension_reason,
            row.extension_notice_source,
        ) {
            (
                Some(notice_at),
                Some(expected_determination_at),
                Some(reason),
                Some(notice_source),
            ) => Some(StatutoryExtensionRecord {
                notice_at,
                expected_determination_at,
                reason: parse_extension_reason(&reason)?,
                notice_source,
            }),
            (None, None, None, None) => None,
            _ => {
                return Err(AppError::Storage(format!(
                    "Request {portal_key}/{request_id} has incomplete statutory extension evidence"
                )))
            }
        };
        let mut flags_statement = conn
            .prepare(
                "SELECT DISTINCT document_integrity_flags.flag_type
                 FROM case_audits
                 INNER JOIN document_integrity_flags
                    ON document_integrity_flags.audit_id = case_audits.audit_id
                 WHERE case_audits.target_request_id = ?1
                   AND case_audits.target_subdomain = ?2
                 ORDER BY document_integrity_flags.flag_type",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to prepare evidence flags: {error}"))
            })?;
        let integrity_flags = flags_statement
            .query_map(params![request_id, portal_key], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| AppError::Storage(format!("Failed to query evidence flags: {error}")))?
            .map(|value| value.map_err(AppError::from))
            .collect::<Result<Vec<_>, _>>()?;
        let mut documents_statement = conn
            .prepare(
                "SELECT id, COALESCE(file_name, ''), file_size, visibility,
                    raw_file_sha256, extracted_text_sha256, shannon_entropy,
                    entropy_sample_bytes, suspicious_data_density
                 FROM documents
                 WHERE request_id = ?1 AND portal_subdomain = ?2
                 ORDER BY id",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to prepare evidence documents: {error}"))
            })?;
        let documents = documents_statement
            .query_map(params![request_id, portal_key], map_evidence_document_row)
            .map_err(|error| {
                AppError::Storage(format!("Failed to query evidence documents: {error}"))
            })?
            .map(|value| value.map_err(AppError::from))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(LegalBriefEvidence {
            request_id: row.request_id,
            portal_key: row.portal_key,
            portal_name: row.portal_name,
            portal_url: row.portal_url,
            agency_name: row.agency_name,
            rpid: row.rpid,
            title: row.title,
            description: row.description,
            status: row.status,
            created_at: row.created_at,
            updated_at: row.updated_at,
            closed_at: row.closed_at,
            determination_at: row.determination_at,
            determination_source: row.determination_source,
            extension,
            integrity_flags,
            documents,
        })
    }

    pub async fn synchronize_cross_correlation_alerts(
        &self,
        correlations: &[CrossAgencyCorrelation],
    ) -> Result<(), AppError> {
        if correlations.is_empty() {
            return Ok(());
        }
        let mut identities = std::collections::BTreeSet::new();
        for correlation in correlations {
            if !identities.insert((correlation.portal_key.clone(), correlation.request_id)) {
                return Err(AppError::Analysis(format!(
                    "Duplicate correlation identity {}/{}",
                    correlation.portal_key, correlation.request_id
                )));
            }
            if !correlation.correlation_density.is_finite() || correlation.correlation_density < 0.0
            {
                return Err(AppError::Analysis(
                    "Correlation density must be finite and non-negative".to_string(),
                ));
            }
        }
        let evaluated_at = chrono::Utc::now().to_rfc3339();
        let mut conn = self.conn.lock().await;
        execute_transaction(&mut conn, "correlation alert synchronization", |tx| {
            for correlation in correlations {
                tx.execute(
                    "INSERT INTO cross_correlation_alerts (
                    request_id, portal_subdomain, neighborhood_block, complaint_count,
                    correlation_window_hours, distinct_source_count, keyword_signal_count, correlation_density,
                    first_correlated_event_at, last_correlated_event_at, evaluated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(request_id, portal_subdomain) DO UPDATE SET
                    neighborhood_block = excluded.neighborhood_block,
                    complaint_count = excluded.complaint_count,
                    correlation_window_hours = excluded.correlation_window_hours,
                    distinct_source_count = excluded.distinct_source_count,
                    keyword_signal_count = excluded.keyword_signal_count,
                    correlation_density = excluded.correlation_density,
                    first_correlated_event_at = excluded.first_correlated_event_at,
                    last_correlated_event_at = excluded.last_correlated_event_at,
                    evaluated_at = excluded.evaluated_at",
                    params![
                        correlation.request_id,
                        correlation.portal_key,
                        correlation.neighborhood_block,
                        correlation.complaint_count,
                        correlation.correlation_window_hours,
                        correlation.distinct_source_count,
                        correlation.keyword_signal_count,
                        correlation.correlation_density,
                        correlation.first_correlated_event_at,
                        correlation.last_correlated_event_at,
                        evaluated_at,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to persist correlation alert: {error}"))
                })?;
                let audit_id =
                    format!("auto-{}-{}", correlation.portal_key, correlation.request_id);
                tx.execute(
                    "INSERT INTO case_audits (
                    audit_id, target_request_id, target_subdomain, auditor_notes,
                    verified_at, verification_status
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(audit_id) DO UPDATE SET verified_at = excluded.verified_at",
                    params![
                        audit_id,
                        correlation.request_id,
                        correlation.portal_key,
                        "Vectorized 311/enforcement correlation evidence",
                        evaluated_at,
                        VERIFICATION_STATUS_ANOMALY,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to synchronize correlation audit: {error}"))
                })?;
                tx.execute(
                    "INSERT INTO document_integrity_flags (audit_id, flag_type)
                 SELECT ?1, 'CROSS_AGENCY_311_CORRELATION'
                 WHERE NOT EXISTS (
                    SELECT 1 FROM document_integrity_flags
                    WHERE audit_id = ?1 AND flag_type = 'CROSS_AGENCY_311_CORRELATION'
                 )",
                    params![audit_id],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to synchronize correlation flag: {error}"))
                })?;
            }
            Ok(())
        })
    }

    /// Resolves the evidence path recorded for a document, for the command layer to gate.
    ///
    /// Visibility-agnostic on purpose. `resolve_public_document_path` restricts to public
    /// documents because it feeds parsers that render content; an integrity check has to be able
    /// to confirm a staff-only document is still the file that was ingested.
    pub async fn resolve_document_evidence_path(
        &self,
        document_id: u64,
        portal_key: &str,
    ) -> Result<Option<PathBuf>, AppError> {
        let portal_key = portal_key.to_string();
        let error_portal_key = portal_key.clone();
        self.query_connection("Document evidence path lookup", move |conn| {
            conn.query_row(
                "SELECT local_path
                 FROM documents
                 WHERE id = ?1 AND portal_subdomain = ?2",
                params![document_id, portal_key],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|error| match error {
                duckdb::Error::QueryReturnedNoRows => AppError::Provenance(format!(
                    "Document {error_portal_key}/{document_id} was not found"
                )),
                other => AppError::Provenance(format!(
                    "Failed to resolve the evidence path for {error_portal_key}/{document_id}: {other}"
                )),
            })
            .map(|path| {
                path.filter(|value| !value.trim().is_empty())
                    .map(PathBuf::from)
            })
        })
        .await
    }

    pub async fn verify_document_provenance(
        &self,
        document_id: u64,
        portal_key: &str,
        evidence: ProvenanceEvidenceFile,
    ) -> Result<DocumentProvenanceVerification, AppError> {
        let (context, current_text_projection_sha256) = {
            let conn = self.conn.lock().await;
            let context = conn
                .query_row(
                    // Only the ingestion digest and the projection digest are read here. Manifest
                    // state used to come from `provenance_manifests` and is now validated live from
                    // the asset, because a stored row describes what was true when it was written and
                    // a verification has to describe the file in front of it.
                    //
                    // `expected_sha256` was `COALESCE(provenance_manifests.file_hash_sha256,
                    // documents.raw_file_sha256)`. That was inert while nothing wrote manifests; once
                    // they are written it would silently move every comparison off the digest recorded
                    // at download onto one observed later, which is the wrong side of a substitution.
                    "SELECT documents.raw_file_sha256,
                    documents.extracted_text_projection_sha256
                 FROM documents
                 WHERE documents.id = ?1 AND documents.portal_subdomain = ?2",
                    params![document_id, portal_key],
                    map_provenance_row,
                )
                .map_err(|error| match error {
                    duckdb::Error::QueryReturnedNoRows => AppError::Provenance(format!(
                        "Document {portal_key}/{document_id} was not found"
                    )),
                    other => {
                        AppError::Provenance(format!("Failed to load provenance state: {other}"))
                    }
                })?;
            // Hashed inside the engine rather than in process: a large text projection should not
            // have to be materialized here just to be digested.
            let current_projection = conn
                .query_row(
                    "SELECT sha256(extracted_text)
                     FROM documents_fts
                     WHERE document_id = ?1 AND portal_subdomain = ?2",
                    params![document_id, portal_key],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .map_err(|error| {
                    AppError::Provenance(format!(
                        "Failed to digest the extracted-text projection: {error}"
                    ))
                })?
                .flatten();
            (context, current_projection)
        };
        // A document whose evidence file cannot be read is still verified and still logged. The
        // previous implementation returned an error here, so the one fact an investigator most
        // needs on the record -- that the file was gone when they checked -- was the one fact the
        // append-only log never captured.
        let (computed_sha256, asset_provenance, evidence_file_present, mut verification_errors) =
            match evidence {
                ProvenanceEvidenceFile::Readable(path) => {
                    // One worker makes both passes over the file: the digest, and the C2PA read.
                    let (digest, provenance) =
                        spawn_with("Provenance worker", AppError::Provenance, move || {
                            let digest = sha256_file(&path).map_err(|error| {
                                AppError::Provenance(format!(
                                    "Failed to hash '{}': {error}",
                                    path.display()
                                ))
                            })?;
                            let provenance = read_asset_provenance(&path)?;
                            Ok((digest, provenance))
                        })
                        .await??;
                    let findings = provenance.findings.clone();
                    (digest, provenance, true, findings)
                }
                ProvenanceEvidenceFile::Unavailable(reason) => (
                    String::new(),
                    AssetProvenance::default(),
                    false,
                    vec![reason],
                ),
            };

        // A recorded digest that is not a well-formed SHA-256 is a corrupt record, not evidence of
        // substitution. Comparing against it would produce a mismatch and accuse the file.
        let recorded_digest = context
            .expected_sha256
            .as_deref()
            .filter(|expected| is_well_formed_sha256(expected));
        if context.expected_sha256.is_some() && recorded_digest.is_none() {
            verification_errors.push(
                "The recorded digest is not a well-formed SHA-256 value, so the stored bytes \
                 could not be compared against it"
                    .to_string(),
            );
        }
        let hash_matches = evidence_file_present
            && recorded_digest
                .is_some_and(|expected| constant_time_ascii_equal(expected, &computed_sha256));

        // The derived-text check is a separate axis from the asset check. A document can be
        // byte-identical to what was ingested while the text quoted from it in a brief has drifted,
        // and the reverse is equally possible, so neither verdict is allowed to stand in for the
        // other.
        let recorded_projection = context
            .recorded_text_projection_sha256
            .as_deref()
            .filter(|digest| is_well_formed_sha256(digest));
        let text_projection_matches = match (
            recorded_projection,
            current_text_projection_sha256.as_deref(),
        ) {
            (Some(recorded), Some(current)) => Some(constant_time_ascii_equal(recorded, current)),
            _ => None,
        };
        if text_projection_matches == Some(false) {
            verification_errors.push(
                "The extracted-text projection no longer matches the digest recorded when the \
                 document was extracted, so text quoted from this document may not reflect what \
                 was released"
                    .to_string(),
            );
        } else if recorded_projection.is_some() && current_text_projection_sha256.is_none() {
            verification_errors.push(
                "A digest was recorded for the extracted-text projection, but the projection is \
                 missing"
                    .to_string(),
            );
        } else if recorded_projection.is_none() && current_text_projection_sha256.is_some() {
            verification_errors.push(
                "No digest was recorded for the extracted-text projection, so text quoted from \
                 this document cannot be verified"
                    .to_string(),
            );
        }
        let assertion_count = asset_provenance.assertions.len();
        let invalid_assertion_count = asset_provenance.invalid_assertion_count();
        if context.expected_sha256.is_none() {
            verification_errors.push("No expected raw-file SHA-256 is recorded".to_string());
        } else if evidence_file_present && recorded_digest.is_some() && !hash_matches {
            verification_errors
                .push("Raw-file SHA-256 does not match the recorded digest".to_string());
        }
        if evidence_file_present && !asset_provenance.manifest_present {
            verification_errors
                .push("The document carries no embedded C2PA provenance manifest".to_string());
        }
        if invalid_assertion_count > 0 {
            verification_errors.push(format!(
                "{invalid_assertion_count} manifest assertions are not covered by a valid \
                 signature and cannot be relied on"
            ));
        }

        // Which digest was compared decides what a match is worth. A manifest digest is a third
        // party's statement about the asset; an ingestion digest is this application's own note of
        // what it downloaded, so matching it proves only that local storage is intact.
        let recorded_digest_source = if context.expected_sha256.is_some() {
            RecordedDigestSource::IngestionRecord
        } else {
            RecordedDigestSource::None
        };
        let signature_verification = asset_provenance.signature_verification();
        let provenance_confidence = classify_confidence(
            evidence_file_present,
            recorded_digest.is_some(),
            hash_matches,
            asset_provenance.manifest_present,
            asset_provenance.validation,
        );
        // Always false today, by construction rather than by accident: the only state that earns
        // this label requires a verified signature chaining to a trusted anchor.
        let chain_of_custody_confirmed = provenance_confidence.is_independent_custody_evidence();
        let result = DocumentProvenanceVerification {
            document_id,
            portal_key: portal_key.to_string(),
            expected_sha256: context.expected_sha256,
            computed_sha256,
            evidence_file_present,
            hash_matches,
            recorded_digest_source,
            manifest_present: asset_provenance.manifest_present,
            is_cryptographically_signed: asset_provenance.claims_signature(),
            signature_verification,
            signature_issuer: asset_provenance.signature_issuer.clone(),
            signing_timestamp: asset_provenance.signing_timestamp.clone(),
            assertion_count,
            invalid_assertion_count,
            claim_graph: asset_provenance.claim_graph.clone(),
            text_projection_sha256: context.recorded_text_projection_sha256.clone(),
            text_projection_matches,
            provenance_confidence,
            chain_of_custody_confirmed,
            verification_errors,
        };
        let errors_json = serde_json::to_string(&result.verification_errors).map_err(|error| {
            AppError::Provenance(format!("Failed to encode verification errors: {error}"))
        })?;
        let mut conn = self.conn.lock().await;
        // The manifest tables record what the asset carried at this verification, and the log
        // records the verification itself. They are written together so the log can never point at
        // manifest state that was not committed.
        let transaction = conn.transaction().map_err(|error| {
            AppError::Provenance(format!(
                "Failed to open the provenance transaction: {error}"
            ))
        })?;
        record_asset_manifest(
            &transaction,
            document_id,
            portal_key,
            &result.computed_sha256,
            &asset_provenance,
        )?;
        transaction
            .execute(
                "INSERT INTO provenance_verification_log (
                verification_id, document_id, portal_subdomain, verified_at,
                expected_sha256, computed_sha256, hash_matches, manifest_present,
                is_cryptographically_signed, invalid_assertion_count,
                chain_of_custody_confirmed, verification_errors_json,
                recorded_digest_source, signature_verification, provenance_confidence,
                text_projection_matches, evidence_file_present, assertion_count,
                signature_issuer, signing_timestamp, text_projection_sha256
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                       ?17, ?18, ?19, ?20, ?21)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    document_id,
                    portal_key,
                    chrono::Utc::now().to_rfc3339(),
                    result.expected_sha256,
                    result.computed_sha256,
                    result.hash_matches,
                    result.manifest_present,
                    result.is_cryptographically_signed,
                    result.invalid_assertion_count,
                    result.chain_of_custody_confirmed,
                    errors_json,
                    result.recorded_digest_source.as_str(),
                    result.signature_verification.as_str(),
                    result.provenance_confidence.as_str(),
                    result.text_projection_matches,
                    result.evidence_file_present,
                    i64::try_from(result.assertion_count).map_err(|_| {
                        AppError::Provenance("Assertion count exceeds storage capacity".to_string())
                    })?,
                    result.signature_issuer,
                    result.signing_timestamp,
                    result.text_projection_sha256,
                ],
            )
            .map_err(|error| {
                AppError::Provenance(format!("Failed to append verification log: {error}"))
            })?;
        transaction.commit().map_err(|error| {
            AppError::Provenance(format!("Failed to commit the provenance record: {error}"))
        })?;
        Ok(result)
    }

    /// Reads a document's verification history, newest first.
    ///
    /// The log is the evidentiary artifact this subsystem produces: it shows that a document was
    /// checked, when, and what the check found. Until this existed the log was written and never
    /// read, so none of that was reachable from the application.
    pub async fn list_document_provenance_verifications(
        &self,
        document_id: u64,
        portal_key: &str,
        limit: usize,
    ) -> Result<Vec<ProvenanceVerificationRecord>, AppError> {
        let portal_key = portal_key.to_string();
        let limit = i64::try_from(limit).map_err(|_| {
            AppError::DataParsing("Verification history limit exceeds capacity".to_string())
        })?;
        self.query_connection("Provenance verification history", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT verification_id, document_id, portal_subdomain, verified_at,
                        expected_sha256, computed_sha256, hash_matches, manifest_present,
                        is_cryptographically_signed, invalid_assertion_count,
                        chain_of_custody_confirmed, verification_errors_json,
                        recorded_digest_source, signature_verification, provenance_confidence,
                        text_projection_matches, evidence_file_present, assertion_count,
                        signature_issuer, signing_timestamp, text_projection_sha256
                     FROM provenance_verification_log
                     WHERE document_id = ?1 AND portal_subdomain = ?2
                     ORDER BY verified_at DESC, verification_id DESC
                     LIMIT ?3",
                )
                .map_err(|error| {
                    AppError::Provenance(format!(
                        "Failed to prepare the verification history query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(
                    params![document_id, portal_key, limit],
                    map_provenance_verification_record,
                )
                .map_err(|error| {
                    AppError::Provenance(format!(
                        "Failed to read the verification history: {error}"
                    ))
                })?;
            rows.map(|row| {
                convert_provenance_verification_record(row.map_err(|error| {
                    AppError::Provenance(format!("Failed to decode a verification row: {error}"))
                })?)
            })
            .collect()
        })
        .await
    }

    pub async fn record_report_export(
        &self,
        result: &LegalBriefExportResult,
    ) -> Result<(), AppError> {
        let markers =
            serde_json::to_string(&result.included_violation_markers).map_err(|error| {
                AppError::Export(format!("Failed to encode report markers: {error}"))
            })?;
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO report_exports (
                export_id, request_id, portal_subdomain, output_path, report_sha256,
                generated_at, violation_markers_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                uuid::Uuid::new_v4().to_string(),
                result.request_id,
                result.portal_key,
                result.output_path,
                result.report_sha256,
                result.generated_at,
                markers,
            ],
        )
        .map_err(|error| {
            AppError::Export(format!("Failed to record report provenance: {error}"))
        })?;
        Ok(())
    }
}

fn execute_transaction<T, F>(
    connection: &mut Connection,
    operation_name: &str,
    operation: F,
) -> Result<T, AppError>
where
    F: FnOnce(&Transaction<'_>) -> Result<T, AppError>,
{
    let transaction = connection.transaction().map_err(|error| {
        AppError::Storage(format!(
            "Failed to open {operation_name} transaction: {error}"
        ))
    })?;
    let result = operation(&transaction)?;
    transaction.commit().map_err(|error| {
        AppError::Storage(format!(
            "Failed to commit {operation_name} transaction: {error}"
        ))
    })?;
    Ok(result)
}

fn upsert_statutory_extension(
    connection: &Connection,
    request_id: u64,
    portal_key: &str,
    extension: &StatutoryExtensionRecord,
) -> Result<(), AppError> {
    connection
        .execute(
            "INSERT INTO statutory_extensions (
                request_id, portal_subdomain, notice_at, expected_determination_at,
                reason, notice_source
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(request_id, portal_subdomain) DO UPDATE SET
                notice_at = excluded.notice_at,
                expected_determination_at = excluded.expected_determination_at,
                reason = excluded.reason,
                notice_source = excluded.notice_source,
                recorded_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')",
            params![
                request_id,
                portal_key,
                extension.notice_at,
                extension.expected_determination_at,
                extension.reason.as_str(),
                extension.notice_source,
            ],
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to persist statutory extension evidence: {error}"
            ))
        })?;
    Ok(())
}

fn map_request_search_row(row: &Row<'_>) -> duckdb::Result<RequestSearchRow> {
    Ok(RequestSearchRow {
        request_id: row.get(0)?,
        portal_key: row.get(1)?,
        agency_id: row.get(2)?,
        agency_name: row.get(3)?,
        title: row.get(4)?,
        description: row.get(5)?,
        status: row.get(6)?,
        created_at: row.get(7)?,
        closed_at: row.get(8)?,
        document_count: row.get(9)?,
        integrity_flags: row.get(10)?,
    })
}

fn map_request_analytical_state(row: &Row<'_>) -> duckdb::Result<RequestAnalyticalState> {
    Ok(RequestAnalyticalState {
        request_id: row.get(0)?,
        created_at: row.get(1)?,
        updated_at: row.get(2)?,
        closed_at: row.get(3)?,
        status: row.get(4)?,
        file_name: row.get(5)?,
    })
}

fn map_raw_stored_legal_mandate(row: &Row<'_>) -> duckdb::Result<RawStoredLegalMandate> {
    Ok(RawStoredLegalMandate {
        mandate_id: row.get(0)?,
        statute_id: row.get(1)?,
        version_id: row.get(2)?,
        title: row.get(3)?,
        applicability_json: row.get(4)?,
        evidence_json: row.get(5)?,
    })
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

struct RawProvenanceVerificationRecord {
    verification_id: String,
    document_id: i64,
    portal_key: String,
    verified_at: String,
    expected_sha256: Option<String>,
    computed_sha256: String,
    hash_matches: bool,
    manifest_present: bool,
    is_cryptographically_signed: bool,
    invalid_assertion_count: i64,
    chain_of_custody_confirmed: bool,
    verification_errors_json: String,
    recorded_digest_source: Option<String>,
    signature_verification: Option<String>,
    provenance_confidence: Option<String>,
    text_projection_matches: Option<bool>,
    evidence_file_present: Option<bool>,
    assertion_count: Option<i64>,
    signature_issuer: Option<String>,
    signing_timestamp: Option<String>,
    text_projection_sha256: Option<String>,
}

/// Replaces the recorded manifest and assertions for a document with what the asset carries now.
///
/// A full replace rather than a merge: these rows describe one observation of one file, so leaving
/// a stale assertion behind would blend two different readings of the document into one record.
/// `file_hash_sha256` is the digest of the bytes the manifest was read from, which is what ties the
/// recorded claims to a specific state of the file.
fn record_asset_manifest(
    tx: &Transaction<'_>,
    document_id: u64,
    portal_key: &str,
    computed_sha256: &str,
    provenance: &AssetProvenance,
) -> Result<(), AppError> {
    tx.execute(
        "DELETE FROM provenance_assertions
         WHERE document_id = ?1 AND portal_subdomain = ?2",
        params![document_id, portal_key],
    )
    .map_err(|error| AppError::Provenance(format!("Failed to clear prior assertions: {error}")))?;
    tx.execute(
        "DELETE FROM provenance_manifests
         WHERE document_id = ?1 AND portal_subdomain = ?2",
        params![document_id, portal_key],
    )
    .map_err(|error| {
        AppError::Provenance(format!("Failed to clear the prior manifest: {error}"))
    })?;

    if !provenance.manifest_present || computed_sha256.is_empty() {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO provenance_manifests (
            document_id, portal_subdomain, file_hash_sha256,
            is_cryptographically_signed, signature_issuer, signing_timestamp
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            document_id,
            portal_key,
            computed_sha256,
            provenance.claims_signature(),
            provenance.signature_issuer,
            provenance.signing_timestamp,
        ],
    )
    .map_err(|error| AppError::Provenance(format!("Failed to record the manifest: {error}")))?;

    let mut insert = tx
        .prepare_cached(
            "INSERT INTO provenance_assertions (
                document_id, portal_subdomain, assertion_type, actor_identifier, is_tamper_evident
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(|error| {
            AppError::Provenance(format!("Failed to prepare assertion insert: {error}"))
        })?;
    for assertion in &provenance.assertions {
        insert
            .execute(params![
                document_id,
                portal_key,
                assertion.assertion_type,
                assertion.actor_identifier,
                assertion.is_tamper_evident,
            ])
            .map_err(|error| {
                AppError::Provenance(format!("Failed to record a manifest assertion: {error}"))
            })?;
    }
    Ok(())
}

fn map_provenance_verification_record(
    row: &Row<'_>,
) -> duckdb::Result<RawProvenanceVerificationRecord> {
    Ok(RawProvenanceVerificationRecord {
        verification_id: row.get(0)?,
        document_id: row.get(1)?,
        portal_key: row.get(2)?,
        verified_at: row.get(3)?,
        expected_sha256: row.get(4)?,
        computed_sha256: row.get(5)?,
        hash_matches: row.get(6)?,
        manifest_present: row.get(7)?,
        is_cryptographically_signed: row.get(8)?,
        invalid_assertion_count: row.get(9)?,
        chain_of_custody_confirmed: row.get(10)?,
        verification_errors_json: row.get(11)?,
        recorded_digest_source: row.get(12)?,
        signature_verification: row.get(13)?,
        provenance_confidence: row.get(14)?,
        text_projection_matches: row.get(15)?,
        evidence_file_present: row.get(16)?,
        assertion_count: row.get(17)?,
        signature_issuer: row.get(18)?,
        signing_timestamp: row.get(19)?,
        text_projection_sha256: row.get(20)?,
    })
}

fn convert_provenance_verification_record(
    raw: RawProvenanceVerificationRecord,
) -> Result<ProvenanceVerificationRecord, AppError> {
    let provenance_confidence = raw
        .provenance_confidence
        .as_deref()
        .and_then(ProvenanceConfidence::from_stored);
    Ok(ProvenanceVerificationRecord {
        verification_id: raw.verification_id,
        document_id: u64::try_from(raw.document_id).map_err(|_| {
            AppError::Provenance(format!("Invalid logged document id {}", raw.document_id))
        })?,
        portal_key: raw.portal_key,
        verified_at: raw.verified_at,
        expected_sha256: raw.expected_sha256,
        computed_sha256: raw.computed_sha256,
        evidence_file_present: raw.evidence_file_present,
        hash_matches: raw.hash_matches,
        manifest_present: raw.manifest_present,
        is_cryptographically_signed: raw.is_cryptographically_signed,
        signature_issuer: raw.signature_issuer,
        signing_timestamp: raw.signing_timestamp,
        assertion_count: raw
            .assertion_count
            .map(|value| {
                usize::try_from(value).map_err(|_| {
                    AppError::Provenance(format!("Invalid logged assertion count {value}"))
                })
            })
            .transpose()?,
        invalid_assertion_count: usize::try_from(raw.invalid_assertion_count).map_err(|_| {
            AppError::Provenance("Logged invalid-assertion count was negative".to_string())
        })?,
        recorded_digest_source: raw
            .recorded_digest_source
            .as_deref()
            .and_then(RecordedDigestSource::from_stored),
        signature_verification: raw
            .signature_verification
            .as_deref()
            .and_then(SignatureVerificationState::from_stored),
        // A row whose confidence does not parse predates classification. That is not a
        // low-confidence result and must never be shown as one.
        predates_classification: provenance_confidence.is_none(),
        provenance_confidence,
        text_projection_sha256: raw.text_projection_sha256,
        text_projection_matches: raw.text_projection_matches,
        chain_of_custody_confirmed: raw.chain_of_custody_confirmed,
        verification_errors: serde_json::from_str(&raw.verification_errors_json).map_err(
            |error| {
                AppError::Provenance(format!("Logged verification errors are invalid: {error}"))
            },
        )?,
    })
}

fn map_provenance_row(row: &Row<'_>) -> duckdb::Result<ProvenanceRow> {
    Ok(ProvenanceRow {
        expected_sha256: row.get(0)?,
        recorded_text_projection_sha256: row.get(1)?,
    })
}

fn map_request_document_row(row: &Row<'_>) -> duckdb::Result<RequestDocumentRow> {
    Ok(RequestDocumentRow {
        document_id: row.get(0)?,
        portal_key: row.get(1)?,
        file_name: row.get(2)?,
        file_size: row.get(3)?,
        content_type: row.get(4)?,
        visibility: row.get(5)?,
        download_status: row.get(6)?,
        raw_file_sha256: row.get(7)?,
        extracted_text_sha256: row.get(8)?,
        extraction_status: row.get(9)?,
        extraction_format: row.get(10)?,
        extraction_deferred_reason: row.get(11)?,
    })
}

fn map_legal_brief_row(row: &Row<'_>) -> duckdb::Result<LegalBriefRow> {
    Ok(LegalBriefRow {
        request_id: row.get(0)?,
        portal_key: row.get(1)?,
        portal_name: row.get(2)?,
        portal_url: row.get(3)?,
        agency_name: row.get(4)?,
        rpid: row.get(5)?,
        title: row.get(6)?,
        description: row.get(7)?,
        status: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        closed_at: row.get(11)?,
        determination_at: row.get(12)?,
        determination_source: row.get(13)?,
        extension_notice_at: row.get(14)?,
        extension_expected_determination_at: row.get(15)?,
        extension_reason: row.get(16)?,
        extension_notice_source: row.get(17)?,
    })
}

fn map_legal_authority_summary_row(row: &Row<'_>) -> duckdb::Result<RawLegalAuthoritySummary> {
    Ok(RawLegalAuthoritySummary {
        statute_id: row.get(0)?,
        jurisdiction: row.get(1)?,
        statute_citation: row.get(2)?,
        title: row.get(3)?,
        authority_kind: row.get(4)?,
        current_version_id: row.get(5)?,
        content_sha256: row.get(6)?,
        mandated_deadline_days: row.get(7)?,
        canonical_source_uri: row.get(8)?,
        effective_from: row.get(9)?,
        effective_to: row.get(10)?,
        provision_count: row.get(11)?,
        mandate_count: row.get(12)?,
        imported_at: row.get(13)?,
    })
}

fn map_legal_authority_version_summary(
    row: &Row<'_>,
) -> duckdb::Result<LegalAuthorityVersionSummary> {
    Ok(LegalAuthorityVersionSummary {
        version_id: row.get(0)?,
        statute_id: row.get(1)?,
        content_sha256: row.get(2)?,
        effective_from: row.get(3)?,
        effective_to: row.get(4)?,
        source_uri: row.get(5)?,
        imported_at: row.get(6)?,
    })
}

fn convert_legal_authority_summary(
    row: RawLegalAuthoritySummary,
) -> Result<LegalAuthoritySummary, AppError> {
    let authority_kind = match row.authority_kind.as_str() {
        "STATE_STATUTE" => LegalAuthorityKind::StateStatute,
        "MUNICIPAL_ORDINANCE" => LegalAuthorityKind::MunicipalOrdinance,
        "AGENCY_POLICY" => LegalAuthorityKind::AgencyPolicy,
        "PROCEDURE_MANUAL" => LegalAuthorityKind::ProcedureManual,
        "REGULATION" => LegalAuthorityKind::Regulation,
        "OTHER" => LegalAuthorityKind::Other,
        value => {
            return Err(AppError::Storage(format!(
                "Legal authority '{}' has unsupported kind '{value}'",
                row.statute_id
            )))
        }
    };
    let mandated_deadline_days = row
        .mandated_deadline_days
        .map(|value| {
            u32::try_from(value).map_err(|_| {
                AppError::Storage(format!(
                    "Legal authority '{}' has an invalid deadline",
                    row.statute_id
                ))
            })
        })
        .transpose()?;
    let provision_count = usize::try_from(row.provision_count).map_err(|_| {
        AppError::Storage(format!(
            "Legal authority '{}' has an invalid provision count",
            row.statute_id
        ))
    })?;
    let mandate_count = usize::try_from(row.mandate_count).map_err(|_| {
        AppError::Storage(format!(
            "Legal authority '{}' has an invalid mandate count",
            row.statute_id
        ))
    })?;
    Ok(LegalAuthoritySummary {
        statute_id: row.statute_id,
        jurisdiction: row.jurisdiction,
        statute_citation: row.statute_citation,
        title: row.title,
        authority_kind,
        current_version_id: row.current_version_id,
        content_sha256: row.content_sha256,
        mandated_deadline_days,
        canonical_source_uri: row.canonical_source_uri,
        effective_from: row.effective_from,
        effective_to: row.effective_to,
        provision_count,
        mandate_count,
        imported_at: row.imported_at,
    })
}

fn map_legal_finding_row(row: &Row<'_>) -> duckdb::Result<RawLegalFinding> {
    Ok(RawLegalFinding {
        violation_id: row.get(0)?,
        request_id: row.get(1)?,
        portal_key: row.get(2)?,
        matched_statute_id: row.get(3)?,
        matched_version_id: row.get(4)?,
        mandate_id: row.get(5)?,
        statute_citation: row.get(6)?,
        authority_title: row.get(7)?,
        mandate_title: row.get(8)?,
        classification: row.get(9)?,
        calculated_variance_metric: row.get(10)?,
        variance_days: row.get(11)?,
        confidence_basis_points: row.get(12)?,
        evidence_summary: row.get(13)?,
        supporting_document_ids_json: row.get(14)?,
        missing_evidence_json: row.get(15)?,
        review_status: row.get(16)?,
        first_detected_at: row.get(17)?,
        evaluated_at: row.get(18)?,
    })
}

fn convert_legal_finding(row: RawLegalFinding) -> Result<LegalComplianceFinding, AppError> {
    let request_id = u64::try_from(row.request_id).map_err(|_| {
        AppError::Storage(format!(
            "Legal finding '{}' has an invalid request identifier",
            row.violation_id
        ))
    })?;
    let confidence_basis_points = u16::try_from(row.confidence_basis_points).map_err(|_| {
        AppError::Storage(format!(
            "Legal finding '{}' has an invalid confidence score",
            row.violation_id
        ))
    })?;
    if confidence_basis_points > 10_000 {
        return Err(AppError::Storage(format!(
            "Legal finding '{}' exceeds the confidence scale",
            row.violation_id
        )));
    }
    let supporting_document_ids =
        serde_json::from_str::<Vec<u64>>(&row.supporting_document_ids_json).map_err(|error| {
            AppError::Storage(format!(
                "Legal finding '{}' has invalid supporting evidence: {error}",
                row.violation_id
            ))
        })?;
    let missing_evidence_phrases = serde_json::from_str::<Vec<String>>(&row.missing_evidence_json)
        .map_err(|error| {
            AppError::Storage(format!(
                "Legal finding '{}' has invalid missing-evidence data: {error}",
                row.violation_id
            ))
        })?;
    Ok(LegalComplianceFinding {
        violation_id: row.violation_id,
        request_id,
        portal_key: row.portal_key,
        matched_statute_id: row.matched_statute_id,
        matched_version_id: row.matched_version_id,
        mandate_id: row.mandate_id,
        statute_citation: row.statute_citation,
        authority_title: row.authority_title,
        mandate_title: row.mandate_title,
        classification: row.classification,
        calculated_variance_metric: row.calculated_variance_metric,
        variance_days: row.variance_days,
        confidence_basis_points,
        evidence_summary: row.evidence_summary,
        supporting_document_ids,
        missing_evidence_phrases,
        review_status: row.review_status,
        first_detected_at: row.first_detected_at,
        evaluated_at: row.evaluated_at,
    })
}

fn map_evidence_document_row(row: &Row<'_>) -> duckdb::Result<EvidenceDocument> {
    Ok(EvidenceDocument {
        document_id: row.get(0)?,
        file_name: row.get(1)?,
        file_size: row.get(2)?,
        visibility: row.get(3)?,
        raw_file_sha256: row.get(4)?,
        extracted_text_sha256: row.get(5)?,
        shannon_entropy: row.get(6)?,
        entropy_sample_bytes: row.get(7)?,
        suspicious_data_density: row.get(8)?,
    })
}

/// Grades a CPRA calendar finding by what the evaluation actually observed.
///
/// The finding rests on two dates. Receipt is always recorded. The determination is a sourced
/// determination event, a generic portal-activity proxy, or the observation instant. Where an
/// extension moved the deadline, its notice is either recorded or claimed without a source.
///
/// ```text
/// confidence = 10000 - uncertainty + margin_relief
/// uncertainty   = determination_penalty + extension_penalty
/// margin_relief = uncertainty / 2 * min(variance_days, 30) / 30
/// ```
///
/// `10000` is therefore reserved for the case where every date input was recorded: no penalty,
/// no relief to apply. `margin_relief` is capped at half the uncertainty so it can never reach
/// it, which keeps every other case strictly below 10000. Its shape reflects that a one-day
/// overrun turns on the exact instants while a month-long one survives any plausible correction
/// to them, so a long overrun recovers most of what the unobserved input cost.
fn cpra_calendar_confidence_basis_points(
    analysis: &crate::analysis::StatutoryComplianceAnalysis,
    variance_days: i64,
) -> i64 {
    use crate::analysis::StatutoryExtensionStatus;

    let determination_penalty = match analysis.determination_basis {
        crate::analysis::DeterminationBasis::SourcedDeterminationEvent => 0,
        crate::analysis::DeterminationBasis::EarliestRecordedActivityProxy => 1_000,
        crate::analysis::DeterminationBasis::ObservationWithoutRecordedActivity => 2_500,
    };
    let extension_penalty = match analysis.extension_status {
        // No extension date entered the evaluation, or the notice behind it was recorded.
        StatutoryExtensionStatus::NotInvoked | StatutoryExtensionStatus::Valid => 0,
        // An extension was claimed and its notice was never recorded, so the deadline the
        // overrun is measured against rests on an input nothing observed.
        StatutoryExtensionStatus::MissingNoticeSource => 1_200,
        // The notice was recorded but contradicts the record around it. The dates exist; which
        // deadline they establish is contested.
        StatutoryExtensionStatus::NoticeBeforeReceipt
        | StatutoryExtensionStatus::NoticeAfterInitialDeadline
        | StatutoryExtensionStatus::ExpectedDateNotExtended
        | StatutoryExtensionStatus::ExtensionExceedsFourteenDays => 600,
    };
    let uncertainty = determination_penalty + extension_penalty;
    let margin_days = variance_days.clamp(0, 30);
    let margin_relief = uncertainty / 2 * margin_days / 30;
    10_000 - uncertainty + margin_relief
}

fn synchronize_cpra_finding(
    tx: &Transaction<'_>,
    request_id: u64,
    portal_key: &str,
    analysis: &crate::analysis::StatutoryComplianceAnalysis,
) -> Result<(), AppError> {
    let version_id = tx
        .query_row(
            "SELECT current_version_id
             FROM legal_statutes
             WHERE statute_id = 'CPRA_7922.535'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to resolve the CPRA authority version: {error}"
            ))
        })?;
    let identity = format!(
        "{portal_key}\0{request_id}\0CPRA_7922.535\0{}",
        FLAG_STATUTORY_LATENCY_VIOLATION
    );
    let digest = sha256_hex(identity.as_bytes());
    let violation_id = format!("cpra-latency-{}", &digest[..32]);
    if !analysis.is_violation {
        tx.execute(
            "UPDATE statutory_violations
             SET review_status = 'RESOLVED_BY_NEW_EVIDENCE',
                 evaluated_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')
             WHERE violation_id = ?1 AND review_status = 'OPEN'",
            params![violation_id],
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to resolve the CPRA timeline finding: {error}"
            ))
        })?;
        return Ok(());
    }
    let variance_days = analysis
        .calendar_days_elapsed
        .checked_sub(analysis.allowed_calendar_days)
        .ok_or_else(|| AppError::Compliance("CPRA deadline variance overflowed".to_string()))?;
    let variance_metric =
        format!("Exceeded the effective determination deadline by {variance_days} calendar days");
    tx.execute(
        "INSERT INTO statutory_violations (
            violation_id, portal_subdomain, request_id, matched_statute_id,
            matched_version_id, mandate_id, classification,
            calculated_variance_metric, variance_days, confidence_basis_points,
            evidence_summary, supporting_document_ids_json, missing_evidence_json,
            classifier_version, review_status, first_detected_at, evaluated_at
         ) VALUES (
            ?1, ?2, ?3, 'CPRA_7922.535', ?4, NULL, ?5, ?6, ?7, ?9,
            ?8, '[]', '[]', 'cpra-calendar-v1', 'OPEN',
            strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ'),
            strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')
         )
         ON CONFLICT(violation_id) DO UPDATE SET
            matched_version_id = excluded.matched_version_id,
            calculated_variance_metric = excluded.calculated_variance_metric,
            variance_days = excluded.variance_days,
            confidence_basis_points = excluded.confidence_basis_points,
            evidence_summary = excluded.evidence_summary,
            classifier_version = excluded.classifier_version,
            review_status = CASE
                WHEN statutory_violations.review_status IN ('CONFIRMED', 'DISMISSED')
                THEN statutory_violations.review_status
                ELSE 'OPEN'
            END,
            evaluated_at = excluded.evaluated_at",
        params![
            violation_id,
            portal_key,
            request_id,
            version_id,
            FLAG_STATUTORY_LATENCY_VIOLATION,
            variance_metric,
            variance_days,
            analysis.evidence_summary,
            cpra_calendar_confidence_basis_points(analysis, variance_days),
        ],
    )
    .map_err(|error| {
        AppError::Storage(format!(
            "Failed to persist the CPRA timeline finding: {error}"
        ))
    })?;
    Ok(())
}

fn synchronize_policy_evidence_gaps(
    tx: &Transaction<'_>,
    request_id: u64,
    portal_key: &str,
) -> Result<PolicyEvaluationState, AppError> {
    let (title, description) = tx
        .query_row(
            "SELECT COALESCE(title, ''), COALESCE(description, '')
             FROM requests
             WHERE id = ?1 AND portal_subdomain = ?2",
            params![request_id, portal_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| match error {
            duckdb::Error::QueryReturnedNoRows => {
                AppError::Storage(format!("Request {portal_key}/{request_id} was not found"))
            }
            other => AppError::Storage(format!("Failed to load request policy context: {other}")),
        })?;
    let (public_document_count, indexed_document_count) = tx
        .query_row(
            "SELECT
                COUNT(*) FILTER (WHERE LOWER(TRIM(documents.visibility)) = 'public'),
                COUNT(*) FILTER (
                    WHERE LOWER(TRIM(documents.visibility)) = 'public'
                      AND EXISTS (
                        SELECT 1 FROM documents_fts
                        WHERE documents_fts.document_id = documents.id
                          AND documents_fts.portal_subdomain = documents.portal_subdomain
                      )
                )
             FROM documents
             WHERE documents.request_id = ?1
               AND documents.portal_subdomain = ?2",
            params![request_id, portal_key],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to inspect policy evidence completeness: {error}"
            ))
        })?;
    let public_document_count = usize::try_from(public_document_count).map_err(|_| {
        AppError::Storage("Public document count was negative or too large".to_string())
    })?;
    let indexed_document_count = usize::try_from(indexed_document_count).map_err(|_| {
        AppError::Storage("Indexed document count was negative or too large".to_string())
    })?;
    if public_document_count == 0 || public_document_count != indexed_document_count {
        let warning = if public_document_count == 0 {
            "Policy evaluation deferred because the request has no public document evidence"
                .to_string()
        } else {
            format!(
                "Policy evaluation deferred because only {indexed_document_count} of {public_document_count} public documents have searchable text"
            )
        };
        return Ok(PolicyEvaluationState {
            evaluated_mandate_count: 0,
            applicable_mandate_count: 0,
            corpus_character_count: 0,
            evaluation_complete: false,
            warnings: vec![warning],
        });
    }

    let metadata = collapse_whitespace(&format!("{title} {description}"));
    let mut corpus_bytes = metadata.len();
    let mut corpus_character_count = metadata.chars().count();
    let mut sources = vec![PolicyCorpusSource {
        document_id: None,
        tokens: policy_tokens(&metadata),
    }];
    {
        let mut statement = tx
            .prepare(
                "SELECT documents.id,
                    (SELECT documents_fts.extracted_text
                     FROM documents_fts
                     WHERE documents_fts.document_id = documents.id
                       AND documents_fts.portal_subdomain = documents.portal_subdomain
                     ORDER BY documents_fts.document_id
                     LIMIT 1)
                 FROM documents
                 WHERE documents.request_id = ?1
                   AND documents.portal_subdomain = ?2
                   AND LOWER(TRIM(documents.visibility)) = 'public'
                   AND EXISTS (
                        SELECT 1 FROM documents_fts
                        WHERE documents_fts.document_id = documents.id
                          AND documents_fts.portal_subdomain = documents.portal_subdomain
                   )
                 ORDER BY documents.id",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to prepare policy evidence scan: {error}"))
            })?;
        let rows = statement
            .query_map(params![request_id, portal_key], |row| {
                Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| {
                AppError::Storage(format!("Failed to query policy evidence: {error}"))
            })?;
        for row in rows {
            let (document_id, text) = row.map_err(|error| {
                AppError::Storage(format!("Failed to decode policy evidence: {error}"))
            })?;
            corpus_bytes = corpus_bytes.checked_add(text.len()).ok_or_else(|| {
                AppError::Storage("Policy evidence corpus size overflowed".to_string())
            })?;
            corpus_character_count = corpus_character_count
                .checked_add(text.chars().count())
                .ok_or_else(|| {
                    AppError::Storage("Policy evidence character count overflowed".to_string())
                })?;
            if corpus_bytes > MAXIMUM_POLICY_CORPUS_BYTES {
                return Ok(PolicyEvaluationState {
                    evaluated_mandate_count: 0,
                    applicable_mandate_count: 0,
                    corpus_character_count,
                    evaluation_complete: false,
                    warnings: vec![format!(
                        "Policy evaluation deferred because the searchable corpus exceeds the {} MiB safety limit",
                        MAXIMUM_POLICY_CORPUS_BYTES / (1024 * 1024)
                    )],
                });
            }
            sources.push(PolicyCorpusSource {
                document_id: Some(document_id),
                tokens: policy_tokens(&text),
            });
        }
    }

    let mandates = {
        let mut statement = tx
            .prepare(
                "SELECT legal_mandates.mandate_id, legal_mandates.statute_id,
                    legal_mandates.version_id, legal_mandates.title,
                    legal_mandates.applicability_phrases_json,
                    legal_mandates.required_evidence_phrases_json
                 FROM legal_mandates
                 INNER JOIN legal_statutes
                    ON legal_statutes.statute_id = legal_mandates.statute_id
                   AND legal_statutes.current_version_id = legal_mandates.version_id
                 WHERE legal_mandates.enabled = 1
                 ORDER BY legal_mandates.statute_id, legal_mandates.mandate_id",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to prepare legal mandate scan: {error}"))
            })?;
        let rows = statement
            .query_map([], map_raw_stored_legal_mandate)
            .map_err(|error| {
                AppError::Storage(format!("Failed to query legal mandates: {error}"))
            })?;
        let mut mandates = Vec::new();
        for row in rows {
            let row = row.map_err(|error| {
                AppError::Storage(format!("Failed to decode legal mandate: {error}"))
            })?;
            let applicability_phrases =
                serde_json::from_str::<Vec<String>>(&row.applicability_json).map_err(|error| {
                    AppError::Storage(format!(
                        "Mandate '{}' has invalid applicability phrases: {error}",
                        row.mandate_id
                    ))
                })?;
            let required_evidence_phrases = serde_json::from_str::<Vec<String>>(&row.evidence_json)
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Mandate '{}' has invalid evidence phrases: {error}",
                        row.mandate_id
                    ))
                })?;
            if applicability_phrases.is_empty() || required_evidence_phrases.is_empty() {
                return Err(AppError::Storage(format!(
                    "Mandate '{}' has an empty phrase set",
                    row.mandate_id
                )));
            }
            mandates.push(StoredLegalMandate {
                mandate_id: row.mandate_id,
                statute_id: row.statute_id,
                version_id: row.version_id,
                title: row.title,
                applicability_phrases,
                required_evidence_phrases,
            });
        }
        mandates
    };

    tx.execute(
        "UPDATE statutory_violations
         SET review_status = 'RESOLVED_BY_NEW_EVIDENCE',
             evaluated_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')
         WHERE request_id = ?1
           AND portal_subdomain = ?2
           AND classification = ?3
           AND review_status = 'OPEN'",
        params![request_id, portal_key, FLAG_POTENTIAL_POLICY_VIOLATION],
    )
    .map_err(|error| {
        AppError::Storage(format!("Failed to retire stale policy findings: {error}"))
    })?;

    let mut applicable_mandate_count = 0_usize;
    for mandate in &mandates {
        let applicability_tokens = mandate
            .applicability_phrases
            .iter()
            .map(|phrase| policy_tokens(phrase))
            .collect::<Vec<_>>();
        let evidence_tokens = mandate
            .required_evidence_phrases
            .iter()
            .map(|phrase| policy_tokens(phrase))
            .collect::<Vec<_>>();
        let mut applicability_match_count = 0_usize;
        let mut supporting_document_ids = std::collections::BTreeSet::new();
        for source in &sources {
            let source_match_count =
                applicability_tokens
                    .iter()
                    .try_fold(0_usize, |total, phrase| {
                        total
                            .checked_add(token_sequence_match_count(&source.tokens, phrase))
                            .ok_or_else(|| {
                                AppError::Analysis(
                                    "Policy source applicability count overflowed".to_string(),
                                )
                            })
                    })?;
            applicability_match_count = applicability_match_count
                .checked_add(source_match_count)
                .ok_or_else(|| {
                AppError::Analysis("Policy applicability count overflowed".to_string())
            })?;
            if source_match_count > 0 {
                if let Some(document_id) = source.document_id {
                    supporting_document_ids.insert(document_id);
                }
            }
        }
        if applicability_match_count == 0 {
            continue;
        }
        applicable_mandate_count = applicable_mandate_count
            .checked_add(1)
            .ok_or_else(|| AppError::Analysis("Applicable mandate count overflowed".to_string()))?;
        let missing_evidence_phrases = mandate
            .required_evidence_phrases
            .iter()
            .zip(evidence_tokens.iter())
            .filter(|(_, phrase)| {
                !sources
                    .iter()
                    .any(|source| token_sequence_match_count(&source.tokens, phrase) > 0)
            })
            .map(|(phrase, _)| phrase.clone())
            .collect::<Vec<_>>();
        if missing_evidence_phrases.is_empty() {
            continue;
        }
        let identity = format!(
            "{portal_key}\0{request_id}\0{}\0{}",
            mandate.mandate_id, FLAG_POTENTIAL_POLICY_VIOLATION
        );
        let digest = sha256_hex(identity.as_bytes());
        let violation_id = format!("policy-gap-{}", &digest[..32]);
        let longest_applicability_phrase =
            applicability_tokens.iter().map(Vec::len).max().unwrap_or(0);
        let match_signal = applicability_match_count.saturating_mul(350).min(1_400);
        let specificity_signal = longest_applicability_phrase.saturating_mul(150).min(1_200);
        let confidence_basis_points = 6_200_usize
            .saturating_add(match_signal)
            .saturating_add(specificity_signal)
            .min(8_800);
        let supporting_document_ids = supporting_document_ids.into_iter().collect::<Vec<_>>();
        let supporting_json = serde_json::to_string(&supporting_document_ids).map_err(|error| {
            AppError::Storage(format!("Failed to encode supporting evidence: {error}"))
        })?;
        let missing_json = serde_json::to_string(&missing_evidence_phrases).map_err(|error| {
            AppError::Storage(format!("Failed to encode missing policy evidence: {error}"))
        })?;
        let variance_metric = format!(
            "0 of {} configured evidence phrases found across {} indexed evidence sources",
            mandate.required_evidence_phrases.len(),
            sources.len()
        );
        let evidence_summary = format!(
            "Applicability language for '{}' matched {} times, while {} configured evidence indicators were absent from the complete indexed corpus. This is an investigative evidence gap, not a legal conclusion.",
            mandate.title,
            applicability_match_count,
            missing_evidence_phrases.len()
        );
        tx.execute(
            "INSERT INTO statutory_violations (
                violation_id, portal_subdomain, request_id, matched_statute_id,
                matched_version_id, mandate_id, classification,
                calculated_variance_metric, variance_days, confidence_basis_points,
                evidence_summary, supporting_document_ids_json, missing_evidence_json,
                classifier_version, review_status, first_detected_at, evaluated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10, ?11, ?12,
                ?13, 'OPEN', strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ'),
                strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')
             )
             ON CONFLICT(violation_id) DO UPDATE SET
                calculated_variance_metric = excluded.calculated_variance_metric,
                confidence_basis_points = excluded.confidence_basis_points,
                evidence_summary = excluded.evidence_summary,
                supporting_document_ids_json = excluded.supporting_document_ids_json,
                missing_evidence_json = excluded.missing_evidence_json,
                classifier_version = excluded.classifier_version,
                review_status = CASE
                    WHEN statutory_violations.review_status IN ('CONFIRMED', 'DISMISSED')
                    THEN statutory_violations.review_status
                    ELSE 'OPEN'
                END,
                evaluated_at = excluded.evaluated_at",
            params![
                violation_id,
                portal_key,
                request_id,
                mandate.statute_id,
                mandate.version_id,
                mandate.mandate_id,
                FLAG_POTENTIAL_POLICY_VIOLATION,
                variance_metric,
                confidence_basis_points,
                evidence_summary,
                supporting_json,
                missing_json,
                POLICY_CLASSIFIER_VERSION,
            ],
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to persist policy evidence gap: {error}"))
        })?;
    }

    synchronize_policy_audit_alert(tx, request_id, portal_key)?;

    Ok(PolicyEvaluationState {
        evaluated_mandate_count: mandates.len(),
        applicable_mandate_count,
        corpus_character_count,
        evaluation_complete: true,
        warnings: Vec::new(),
    })
}

fn synchronize_policy_audit_alert(
    tx: &Transaction<'_>,
    request_id: u64,
    portal_key: &str,
) -> Result<(), AppError> {
    let active_finding_count = tx
        .query_row(
            "SELECT COUNT(*)
             FROM statutory_violations
             WHERE request_id = ?1
               AND portal_subdomain = ?2
               AND classification = ?3
               AND review_status IN ('OPEN', 'CONFIRMED')",
            params![request_id, portal_key, FLAG_POTENTIAL_POLICY_VIOLATION],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to aggregate policy findings: {error}"))
        })?;
    let audit_id = format!("auto-{portal_key}-{request_id}");
    let audit_exists = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM case_audits WHERE audit_id = ?1)",
            params![audit_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to inspect policy audit state: {error}"))
        })?;
    if active_finding_count > 0 || audit_exists {
        upsert_automatic_case_audit(
            tx,
            &audit_id,
            request_id,
            portal_key,
            "Automatically derived from versioned policy evidence rules",
            if active_finding_count > 0 {
                VERIFICATION_STATUS_ANOMALY
            } else {
                VERIFICATION_STATUS_COMPLIANT
            },
            false,
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to synchronize policy audit: {error}"))
        })?;
        tx.execute(
            "DELETE FROM document_integrity_flags
             WHERE audit_id = ?1 AND flag_type = ?2",
            params![audit_id, FLAG_POTENTIAL_POLICY_VIOLATION],
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to clear stale policy alert: {error}"))
        })?;
        if active_finding_count > 0 {
            tx.execute(
                "INSERT INTO document_integrity_flags (audit_id, flag_type)
                 VALUES (?1, ?2)",
                params![audit_id, FLAG_POTENTIAL_POLICY_VIOLATION],
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to synchronize policy alert: {error}"))
            })?;
        }
        update_automatic_audit_status(tx, &audit_id)?;
    }
    Ok(())
}

fn synchronize_timeline_audit(
    tx: &Transaction<'_>,
    audit_id: &str,
    request_id: u64,
    subdomain: &str,
    timeline_flags: &[&str],
) -> Result<(), AppError> {
    let audit_exists = tx
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM case_audits WHERE audit_id = ?1
             )",
            params![audit_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to inspect the automatic timeline audit state: {error}"
            ))
        })?;

    if timeline_flags.is_empty() && !audit_exists {
        return Ok(());
    }

    upsert_automatic_case_audit(
        tx,
        audit_id,
        request_id,
        subdomain,
        "Automatically derived during request timeline synchronization",
        if timeline_flags.contains(&FLAG_STATUTORY_LATENCY_VIOLATION) {
            VERIFICATION_STATUS_STATUTORY
        } else if timeline_flags.is_empty() {
            VERIFICATION_STATUS_COMPLIANT
        } else {
            VERIFICATION_STATUS_ANOMALY
        },
        false,
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "Failed to synchronize the automatic timeline audit: {error}"
        ))
    })?;

    replace_managed_integrity_flags(tx, audit_id, &DERIVED_TIMELINE_FLAG_TYPES, timeline_flags)?;

    update_automatic_audit_status(tx, audit_id)
}

fn replace_managed_integrity_flags(
    tx: &Transaction<'_>,
    audit_id: &str,
    managed_flags: &[&str],
    replacement_flags: &[&str],
) -> Result<(), AppError> {
    if managed_flags.is_empty()
        || replacement_flags
            .iter()
            .any(|flag| !managed_flags.contains(flag))
    {
        return Err(AppError::Internal(
            "Integrity-flag synchronization received an invalid managed set".to_string(),
        ));
    }
    let placeholders = (2..=managed_flags.len() + 1)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let delete_sql = format!(
        "DELETE FROM document_integrity_flags \
         WHERE audit_id = ?1 AND flag_type IN ({placeholders})"
    );
    tx.execute(
        &delete_sql,
        params_from_iter(std::iter::once(audit_id).chain(managed_flags.iter().copied())),
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "Failed to clear stale managed integrity flags: {error}"
        ))
    })?;

    let mut insert = tx
        .prepare_cached(
            "INSERT INTO document_integrity_flags (audit_id, flag_type) VALUES (?1, ?2)",
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to prepare integrity-flag synchronization: {error}"
            ))
        })?;
    for flag in replacement_flags {
        insert.execute(params![audit_id, flag]).map_err(|error| {
            AppError::Internal(format!(
                "Failed to synchronize managed integrity flag '{flag}': {error}"
            ))
        })?;
    }
    Ok(())
}

fn update_automatic_audit_status(tx: &Transaction<'_>, audit_id: &str) -> Result<(), AppError> {
    let updated = tx
        .execute(
            "UPDATE case_audits
             SET
                verified_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ'),
                verification_status = CASE
                    WHEN EXISTS (
                        SELECT 1
                        FROM document_integrity_flags
                        WHERE document_integrity_flags.audit_id = case_audits.audit_id
                          AND document_integrity_flags.flag_type = ?4
                    ) THEN ?5
                    WHEN EXISTS (
                        SELECT 1
                        FROM document_integrity_flags
                        WHERE document_integrity_flags.audit_id = case_audits.audit_id
                    ) THEN ?2
                    ELSE ?3
                END
             WHERE audit_id = ?1",
            params![
                audit_id,
                VERIFICATION_STATUS_ANOMALY,
                VERIFICATION_STATUS_COMPLIANT,
                FLAG_STATUTORY_LATENCY_VIOLATION,
                VERIFICATION_STATUS_STATUTORY,
            ],
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to refresh the automatic audit verification status: {error}"
            ))
        })?;

    if updated != 1 {
        return Err(AppError::Internal(format!(
            "Automatic audit status refresh affected {updated} rows for audit '{audit_id}'"
        )));
    }

    Ok(())
}

fn synchronize_fts_row(
    tx: &Transaction<'_>,
    document_id: i64,
    subdomain: &str,
    file_name: Option<&str>,
    cleaned_text: &str,
) -> Result<(), AppError> {
    let updated_rows = tx
        .execute(
            "UPDATE documents_fts
             SET file_name = ?1, extracted_text = ?2
             WHERE document_id = ?3 AND portal_subdomain = ?4",
            params![file_name, cleaned_text, document_id, subdomain],
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Full-text search conflict-update branch failed: {error}"
            ))
        })?;

    if updated_rows == 0 {
        tx.execute(
            "INSERT INTO documents_fts (
                document_id,
                portal_subdomain,
                file_name,
                extracted_text
             ) VALUES (?1, ?2, ?3, ?4)",
            params![document_id, subdomain, file_name, cleaned_text],
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Full-text search conflict-insert branch failed: {error}"
            ))
        })?;
    }

    let synchronized_row_count = tx
        .query_row(
            "SELECT COUNT(*)
             FROM documents_fts
             WHERE document_id = ?1 AND portal_subdomain = ?2",
            params![document_id, subdomain],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to verify full-text search synchronization: {error}"
            ))
        })?;

    if synchronized_row_count != 1 {
        return Err(AppError::Internal(format!(
            "Full-text search synchronization produced {synchronized_row_count} rows for document {document_id} in portal '{subdomain}'"
        )));
    }

    synchronize_search_tokens(tx, document_id, subdomain, cleaned_text)?;

    Ok(())
}

const SEARCH_TOKEN_INDEX_VERSION: i64 = 1;
const SEARCH_TOKEN_BACKFILL_BATCH_SIZE: i64 = 64;

fn synchronize_search_tokens(
    tx: &Transaction<'_>,
    document_id: i64,
    portal_subdomain: &str,
    cleaned_text: &str,
) -> Result<(), AppError> {
    tx.execute(
        "DELETE FROM document_search_tokens
         WHERE document_id = ?1 AND portal_subdomain = ?2",
        params![document_id, portal_subdomain],
    )
    .map_err(|error| AppError::Storage(format!("Failed to clear stale search tokens: {error}")))?;

    let frequencies = token_frequencies(cleaned_text);
    for (token, frequency) in &frequencies {
        let frequency = i64::try_from(*frequency).map_err(|_| {
            AppError::Storage("Search term frequency exceeds DuckDB capacity".to_string())
        })?;
        tx.execute(
            "INSERT INTO document_search_tokens (
                document_id, portal_subdomain, token, term_frequency
             ) VALUES (?1, ?2, ?3, ?4)",
            params![document_id, portal_subdomain, token, frequency],
        )
        .map_err(|error| AppError::Storage(format!("Failed to persist a search token: {error}")))?;
    }

    let token_count = i64::try_from(frequencies.len())
        .map_err(|_| AppError::Storage("Search token count exceeds DuckDB capacity".to_string()))?;
    tx.execute(
        "INSERT INTO document_search_index_state (
            document_id, portal_subdomain, index_version, token_count
         ) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(document_id, portal_subdomain) DO UPDATE SET
            index_version = excluded.index_version,
            token_count = excluded.token_count",
        params![
            document_id,
            portal_subdomain,
            SEARCH_TOKEN_INDEX_VERSION,
            token_count
        ],
    )
    .map_err(|error| AppError::Storage(format!("Failed to persist search index state: {error}")))?;
    Ok(())
}

/// Builds the token projection for documents created before the persistent index existed.
/// Empty documents receive a state row with a zero token count, so the keyset scan still
/// terminates. Each bounded batch is atomic and restartable.
fn backfill_search_token_index(conn: &mut Connection) -> Result<(), AppError> {
    loop {
        let rows = {
            let mut statement = conn
                .prepare(
                    "SELECT
                        documents_fts.document_id,
                        documents_fts.portal_subdomain,
                        documents_fts.extracted_text
                     FROM documents_fts
                     LEFT JOIN document_search_index_state
                       ON document_search_index_state.document_id = documents_fts.document_id
                      AND document_search_index_state.portal_subdomain = documents_fts.portal_subdomain
                      AND document_search_index_state.index_version = ?1
                     WHERE document_search_index_state.document_id IS NULL
                     ORDER BY documents_fts.portal_subdomain, documents_fts.document_id
                     LIMIT ?2",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare search index backfill: {error}"))
                })?;
            statement
                .query_map(
                    params![SEARCH_TOKEN_INDEX_VERSION, SEARCH_TOKEN_BACKFILL_BATCH_SIZE],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query search index backfill: {error}"))
                })?
                .collect::<duckdb::Result<Vec<_>>>()
                .map_err(|error| {
                    AppError::Storage(format!("Failed to decode search index backfill: {error}"))
                })?
        };
        if rows.is_empty() {
            return Ok(());
        }

        let transaction = conn.transaction().map_err(|error| {
            AppError::Storage(format!("Failed to begin search index backfill: {error}"))
        })?;
        for (document_id, portal_subdomain, extracted_text) in rows {
            synchronize_search_tokens(
                &transaction,
                document_id,
                &portal_subdomain,
                &extracted_text,
            )?;
        }
        transaction.commit().map_err(|error| {
            AppError::Storage(format!("Failed to commit search index backfill: {error}"))
        })?;
    }
}

/// Audits relations kept logical because DuckDB cannot support the required parent updates with
/// physical foreign keys. Canonical evidence is never auto-deleted or guessed into a parent; the
/// operator receives exact relation/count pairs and can restore from the retained source data.
fn verify_logical_relations(conn: &Connection) -> Result<(), AppError> {
    const CHECKS: [(&str, &str); 9] = [
        (
            "statutory_extensions -> requests",
            "SELECT COUNT(*) FROM statutory_extensions child
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

/// Whether a stored digest is a well-formed SHA-256 value.
///
/// Read paths need this as a question rather than an error. A digest that was never valid must be
/// reported as a corrupt record; comparing against it instead would fail and present an intact
/// file as a substituted one.
fn is_well_formed_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn normalize_sha256(sha256_hash: &str) -> Result<String, AppError> {
    let trimmed_hash = sha256_hash.trim();
    if !is_well_formed_sha256(trimmed_hash) {
        return Err(AppError::DataParsing(
            "Document SHA-256 signature must contain exactly 64 hexadecimal characters".to_string(),
        ));
    }

    Ok(trimmed_hash.to_ascii_lowercase())
}

fn validate_extracted_text_sha256(
    extracted_text: &str,
    supplied_sha256: &str,
) -> Result<String, AppError> {
    let normalized_sha256 = normalize_sha256(supplied_sha256)?;
    let computed_sha256 = compute_extracted_text_sha256(extracted_text);
    if normalized_sha256 != computed_sha256 {
        return Err(AppError::DataParsing(format!(
            "Extracted text SHA-256 mismatch: expected {computed_sha256}, received {normalized_sha256}"
        )));
    }

    Ok(normalized_sha256)
}

fn compute_extracted_text_sha256(extracted_text: &str) -> String {
    sha256_hex(extracted_text.as_bytes())
}

fn derive_integrity_flags(
    analytical_state: &RequestAnalyticalState,
    request_has_heavy_redaction: bool,
    request_has_suspicious_data_density: bool,
) -> Vec<&'static str> {
    let mut flags = Vec::with_capacity(DERIVED_DOCUMENT_FLAG_TYPES.len());
    let created_at = chrono::DateTime::parse_from_rfc3339(&analytical_state.created_at).ok();
    let updated_at = chrono::DateTime::parse_from_rfc3339(&analytical_state.updated_at).ok();
    let closed_at = analytical_state
        .closed_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());

    if matches!((&created_at, &updated_at), (Some(created), Some(updated)) if updated < created) {
        flags.push(FLAG_UPDATE_BEFORE_CREATION);
    }

    if let (Some(created), Some(closed)) = (&created_at, &closed_at) {
        let lifespan = closed.signed_duration_since(*created);
        if lifespan < chrono::TimeDelta::zero() {
            flags.push(FLAG_CLOSE_BEFORE_CREATION);
        } else if lifespan <= chrono::TimeDelta::seconds(SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS)
            && analytical_state.status.eq_ignore_ascii_case("closed")
        {
            flags.push(FLAG_SUSPICIOUSLY_FAST_CLOSURE);
        }
    }

    if request_has_heavy_redaction {
        flags.push(FLAG_HEAVY_REDACTION_DENSITY);
    }
    if request_has_suspicious_data_density {
        flags.push(FLAG_SUSPICIOUS_DATA_DENSITY);
    }

    flags
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

fn validate_entropy_metadata(
    shannon_entropy: Option<f64>,
    entropy_sample_bytes: u64,
    raw_file_size: u64,
    suspicious_data_density: bool,
) -> Result<(i64, i64), AppError> {
    match shannon_entropy {
        Some(score) => {
            if !score.is_finite() || !(0.0..=8.0).contains(&score) {
                return Err(AppError::DataParsing(format!(
                    "Shannon entropy must be finite and between 0.0 and 8.0, received {score}"
                )));
            }
            if entropy_sample_bytes == 0 && score != 0.0 {
                return Err(AppError::DataParsing(
                    "A zero-byte entropy sample must have an entropy score of 0.0".to_string(),
                ));
            }
        }
        None => {
            if entropy_sample_bytes != 0 {
                return Err(AppError::DataParsing(format!(
                    "Entropy was sampled over {entropy_sample_bytes} bytes but no score was \
                     computed; absence and measurement cannot be mixed"
                )));
            }
        }
    }
    if entropy_sample_bytes > raw_file_size {
        return Err(AppError::DataParsing(format!(
            "Entropy sample size {entropy_sample_bytes} exceeds raw file size {raw_file_size}"
        )));
    }
    let entropy_sample_bytes = i64::try_from(entropy_sample_bytes).map_err(|error| {
        AppError::DataParsing(format!("Entropy sample size cannot be persisted: {error}"))
    })?;
    Ok((
        entropy_sample_bytes,
        if suspicious_data_density { 1 } else { 0 },
    ))
}

fn request_has_suspicious_density(
    tx: &Transaction<'_>,
    request_id: i64,
    subdomain: &str,
) -> Result<bool, AppError> {
    tx.query_row(
        "SELECT EXISTS(
            SELECT 1
            FROM documents
            WHERE request_id = ?1
              AND portal_subdomain = ?2
              AND suspicious_data_density = 1
         )",
        params![request_id, subdomain],
        |row| row.get::<_, bool>(0),
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "Failed to aggregate request entropy state: {error}"
        ))
    })
}

fn upsert_automatic_case_audit<T: ToSql>(
    tx: &Transaction<'_>,
    audit_id: &str,
    request_id: T,
    subdomain: &str,
    notes: &str,
    verification_status: &str,
    replace_conclusion: bool,
) -> duckdb::Result<()> {
    let exists = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM case_audits WHERE audit_id = ?1)",
        params![audit_id],
        |row| row.get::<_, bool>(0),
    )?;
    if exists {
        tx.execute(
            "UPDATE case_audits
             SET target_request_id = ?2,
                 target_subdomain = ?3,
                 auditor_notes = CASE WHEN ?6 = 1 THEN ?4 ELSE auditor_notes END,
                 verified_at = strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ'),
                 verification_status = CASE WHEN ?6 = 1 THEN ?5 ELSE verification_status END
             WHERE audit_id = ?1",
            params![
                audit_id,
                request_id,
                subdomain,
                notes,
                verification_status,
                if replace_conclusion { 1_i64 } else { 0_i64 }
            ],
        )?;
    } else {
        tx.execute(
            "INSERT INTO case_audits (
                audit_id, target_request_id, target_subdomain, auditor_notes,
                verified_at, verification_status
             ) VALUES (?1, ?2, ?3, ?4, strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ'), ?5)",
            params![audit_id, request_id, subdomain, notes, verification_status],
        )?;
    }
    Ok(())
}

fn synchronize_density_audit(
    tx: &Transaction<'_>,
    document_id: i64,
    subdomain: &str,
) -> Result<(), AppError> {
    let request_id = tx
        .query_row(
            "SELECT request_id
             FROM documents
             WHERE id = ?1 AND portal_subdomain = ?2",
            params![document_id, subdomain],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to load document request for entropy synchronization: {error}"
            ))
        })?;
    let audit_id = format!("auto-{subdomain}-{request_id}");
    let request_has_suspicious_data_density =
        request_has_suspicious_density(tx, request_id, subdomain)?;
    // DuckDB types `EXISTS(...)` as BOOLEAN, not an integer, and decoding it as `i64` fails the
    // whole transaction. The other two call sites already read it as `bool`; this one did not,
    // and it is the only reader on `mark_document_downloaded`, the terminal write for every
    // document with no text layer. Nothing covered that path until the entropy test above.
    let audit_exists = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM case_audits WHERE audit_id = ?1)",
            params![audit_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| {
            AppError::Internal(format!("Failed to inspect entropy audit state: {error}"))
        })?;
    if !request_has_suspicious_data_density && !audit_exists {
        return Ok(());
    }

    upsert_automatic_case_audit(
        tx,
        &audit_id,
        request_id,
        subdomain,
        "Automatically derived during document entropy synchronization",
        if request_has_suspicious_data_density {
            VERIFICATION_STATUS_ANOMALY
        } else {
            VERIFICATION_STATUS_COMPLIANT
        },
        false,
    )
    .map_err(|error| {
        AppError::Internal(format!("Failed to synchronize entropy case audit: {error}"))
    })?;
    tx.execute(
        "DELETE FROM document_integrity_flags
         WHERE audit_id = ?1 AND flag_type = ?2",
        params![audit_id, FLAG_SUSPICIOUS_DATA_DENSITY],
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "Failed to clear stale entropy integrity flag: {error}"
        ))
    })?;
    if request_has_suspicious_data_density {
        tx.execute(
            "INSERT INTO document_integrity_flags (audit_id, flag_type)
             VALUES (?1, ?2)",
            params![audit_id, FLAG_SUSPICIOUS_DATA_DENSITY],
        )
        .map_err(|error| {
            AppError::Internal(format!("Failed to insert entropy integrity flag: {error}"))
        })?;
    }
    update_automatic_audit_status(tx, &audit_id)
}

fn has_heavy_redaction_density(extracted_text: &str) -> Result<bool, AppError> {
    Ok(analyze_redaction_density(extracted_text)?
        .alert_states
        .contains(&ClusterAlertState::HeavyRedactionDensity))
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
                strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ'),
                strftime(current_timestamp, '%Y-%m-%dT%H:%M:%fZ')
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

fn ensure_column(
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

fn validate_portal_registration(portal: &PortalRegistration) -> Result<(), AppError> {
    if portal.portal_key.is_empty()
        || portal.portal_key.len() > 128
        || !portal
            .portal_key
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'-' | b'_'))
    {
        return Err(AppError::DataParsing(format!(
            "Portal key '{}' is invalid",
            portal.portal_key
        )));
    }
    for (label, value) in [
        ("canonical URL", portal.canonical_url.as_str()),
        ("origin URL", portal.origin_url.as_str()),
        (
            "requests endpoint",
            portal.api_layout.requests_endpoint.as_str(),
        ),
    ] {
        let url = reqwest::Url::parse(value).map_err(|error| {
            AppError::DataParsing(format!("Portal {label} '{value}' is invalid: {error}"))
        })?;
        let local_source_uri = matches!(
            portal.source_kind,
            crate::types::SourceKind::LocalFile | crate::types::SourceKind::PastedContent
        ) && url.scheme() == "onrecord"
            && url.host_str() == Some("local");
        if url.scheme() != "https" && !local_source_uri {
            return Err(AppError::PortalSecurity(format!(
                "Portal {label} must use HTTPS or an internal onrecord://local URI"
            )));
        }
    }
    if portal.display_name.trim().is_empty()
        || portal.api_layout.page_parameter.trim().is_empty()
        || portal.api_layout.page_size_parameter.trim().is_empty()
    {
        return Err(AppError::DataParsing(
            "Portal display name and pagination parameters must not be empty".to_string(),
        ));
    }
    chrono::DateTime::parse_from_rfc3339(&portal.verified_at).map_err(|error| {
        AppError::DataParsing(format!("Portal verification timestamp is invalid: {error}"))
    })?;
    Ok(())
}

fn validate_auxiliary_event(event: &AuxiliaryEventInput) -> Result<(), AppError> {
    if event.external_id.trim().is_empty()
        || event.external_id.len() > 256
        || event.event_type.trim().is_empty()
        || event.neighborhood_block.trim().is_empty()
    {
        return Err(AppError::DataParsing(
            "Auxiliary event identifiers, type, and neighborhood are required".to_string(),
        ));
    }
    chrono::DateTime::parse_from_rfc3339(&event.occurred_at).map_err(|error| {
        AppError::DataParsing(format!("Auxiliary event timestamp is invalid: {error}"))
    })?;
    if event
        .latitude
        .is_some_and(|value| !value.is_finite() || !(-90.0..=90.0).contains(&value))
        || event
            .longitude
            .is_some_and(|value| !value.is_finite() || !(-180.0..=180.0).contains(&value))
    {
        return Err(AppError::DataParsing(
            "Auxiliary event coordinates are outside valid bounds".to_string(),
        ));
    }
    Ok(())
}

fn normalize_neighborhood(value: &str) -> String {
    collapse_whitespace(value)
}

fn parse_extension_reason(value: &str) -> Result<StatutoryExtensionReason, AppError> {
    match value {
        "SEPARATE_FIELD_FACILITIES" => Ok(StatutoryExtensionReason::SeparateFieldFacilities),
        "VOLUMINOUS_RECORDS" => Ok(StatutoryExtensionReason::VoluminousRecords),
        "INTERAGENCY_CONSULTATION" => Ok(StatutoryExtensionReason::InteragencyConsultation),
        "DATA_COMPILATION" => Ok(StatutoryExtensionReason::DataCompilation),
        "CYBERATTACK_SYSTEMS_UNAVAILABLE" => {
            Ok(StatutoryExtensionReason::CyberattackSystemsUnavailable)
        }
        "DECLARED_EMERGENCY_OPERATIONAL_IMPACT" => {
            Ok(StatutoryExtensionReason::DeclaredEmergencyOperationalImpact)
        }
        other => Err(AppError::DataParsing(format!(
            "Unknown statutory extension reason '{other}'"
        ))),
    }
}

fn constant_time_ascii_equal(left: &str, right: &str) -> bool {
    let left = left.trim().to_ascii_lowercase();
    let right = right.trim().to_ascii_lowercase();
    constant_time_equal(left.as_bytes(), right.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use duckdb::params;

    use crate::error::AppError;
    use crate::types::PublicRequest;

    use super::{
        compute_extracted_text_sha256, cpra_calendar_confidence_basis_points,
        validate_entropy_metadata, verify_logical_relations, DocumentDownloadFinalization,
        DocumentExtractionFinalization, DuckDbRepository, IngestionSchedulerCounts,
        PortalSyncRunFinalization, ProvenanceEvidenceFile, SourceLifecycleState,
    };

    const MIGRATION: &str = include_str!("../../migrations/0001_initial_schema.sql");
    const PORTAL_SUBDOMAIN: &str = "test-portal";
    const REQUEST_ID: i64 = 41;
    const DOCUMENT_ID: i64 = 73;
    const RAW_FILE_SHA256: &str =
        "1111111111111111111111111111111111111111111111111111111111111111";

    async fn repository_with_document() -> DuckDbRepository {
        file_repository_with_document(Path::new(":memory:")).await
    }

    async fn file_repository_with_document(path: &Path) -> DuckDbRepository {
        let repository = DuckDbRepository::new(path).expect("DuckDB core must open");
        repository
            .run_migrations(MIGRATION)
            .await
            .expect("test migration must succeed");

        {
            let connection = repository.conn.lock().await;
            connection
                .execute(
                    "INSERT INTO portals (subdomain, display_name, base_url)
                     VALUES (?1, ?2, ?3)",
                    params![PORTAL_SUBDOMAIN, "Test Portal", "https://example.test"],
                )
                .expect("portal fixture must insert");
            connection
                .execute(
                    "INSERT INTO requests (
                        id,
                        portal_subdomain,
                        status,
                        created_at,
                        updated_at,
                        closed_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        REQUEST_ID,
                        PORTAL_SUBDOMAIN,
                        "closed",
                        "2026-08-24T12:00:00Z",
                        "2026-08-24T11:59:00Z",
                        "2026-08-24T12:04:00Z",
                    ],
                )
                .expect("request fixture must insert");
            connection
                .execute(
                    "INSERT INTO documents (
                        id,
                        request_id,
                        portal_subdomain,
                        file_name,
                        visibility,
                        created_at,
                        updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        DOCUMENT_ID,
                        REQUEST_ID,
                        PORTAL_SUBDOMAIN,
                        "release.pdf",
                        "public",
                        "2026-08-24T12:01:00Z",
                        "2026-08-24T12:01:00Z",
                    ],
                )
                .expect("document fixture must insert");
        }

        repository
    }

    /// Analytics nodes are constructed once at startup, before any document is ingested. A node
    /// that opens or `ATTACH`es the core database file gets a snapshot frozen at that moment and
    /// never observes a later write, which silently reported empty analytics for the whole
    /// session. Both nodes must therefore read through a live sibling connection.
    #[tokio::test]
    async fn analytics_nodes_observe_writes_made_after_they_were_bound() {
        use crate::analysis::AnalyticsEngine;
        use crate::storage::duckdb::DuckDbWorkspace;
        use crate::types::AnalysisConfig;

        // File-backed on purpose: the frozen-snapshot failure this pins only occurs for a
        // database that lives on disk, which is what production uses.
        let directory = tempfile::tempdir().expect("temp dir must be created");
        let repository = file_repository_with_document(&directory.path().join("core.db")).await;

        // Bind both nodes first, exactly as `configure_onrecord_studio_runtime` does at startup.
        let agency_engine = AnalyticsEngine::bind_to_core(
            repository
                .analytics_connection()
                .await
                .expect("analytics connection must open"),
        );
        let correlation_engine = DuckDbWorkspace::bind_to_core(
            repository
                .analytics_connection()
                .await
                .expect("correlation connection must open"),
        );

        let config = AnalysisConfig::default();
        assert!(
            agency_engine
                .calculate_agency_outlier_profiles(&config)
                .await
                .expect("profiles must resolve against an unmapped corpus")
                .is_empty(),
            "no agency is mapped yet"
        );

        // Now write, after both nodes were bound.
        {
            let connection = repository.conn.lock().await;
            connection
                .execute(
                    "INSERT INTO agencies (id, portal_subdomain, name) VALUES (?1, ?2, ?3)",
                    params![9_i64, PORTAL_SUBDOMAIN, "Records Division"],
                )
                .expect("agency fixture must insert");
            connection
                .execute(
                    "UPDATE requests SET agency_id = ?1 WHERE id = ?2",
                    params![9_i64, REQUEST_ID],
                )
                .expect("request must be mapped to the agency");
        }

        let profiles = agency_engine
            .calculate_agency_outlier_profiles(&config)
            .await
            .expect("profiles must resolve after the write");
        assert_eq!(
            profiles.len(),
            1,
            "the agency written after binding must be visible to the analytics node"
        );
        assert_eq!(profiles[0].agency_name, "Records Division");

        // The correlation workspace shares the same liveness requirement; it must at minimum
        // execute against the live catalog rather than reporting a missing attachment.
        correlation_engine
            .correlate_311_with_enforcement_requests(Some(PORTAL_SUBDOMAIN), 1, 10, 48)
            .await
            .expect("correlation must execute against the live core database");
    }

    async fn set_document_file(
        repository: &DuckDbRepository,
        local_path: &Path,
        recorded_digest: Option<&str>,
    ) {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "UPDATE documents
                 SET local_path = ?1, raw_file_sha256 = ?2, download_status = 'completed'
                 WHERE id = ?3 AND portal_subdomain = ?4",
                params![
                    local_path.to_string_lossy().to_string(),
                    recorded_digest,
                    DOCUMENT_ID,
                    PORTAL_SUBDOMAIN
                ],
            )
            .expect("document file fixture must update");
    }

    async fn record_manifest_claim(
        repository: &DuckDbRepository,
        file_hash_sha256: &str,
        claims_signature: bool,
    ) {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO provenance_manifests (
                    document_id, portal_subdomain, file_hash_sha256,
                    is_cryptographically_signed, signature_issuer, signing_timestamp
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    DOCUMENT_ID,
                    PORTAL_SUBDOMAIN,
                    file_hash_sha256,
                    claims_signature,
                    "Example Issuer",
                    "2026-08-24T12:02:00Z",
                ],
            )
            .expect("manifest fixture must insert");
    }

    /// Each provenance state must be reachable and distinguishable, and none of the states this
    /// build can produce may present recorded claims as verified chain-of-custody evidence.
    ///
    /// The fourth case is the one that matters: a manifest asserting a cryptographic signature,
    /// over a file whose digest matches, previously reported `chain_of_custody_confirmed = true`
    /// without any signature ever being checked.
    #[tokio::test]
    async fn provenance_states_are_distinguished_and_never_claim_verified_custody() {
        use crate::types::{
            ProvenanceConfidence, RecordedDigestSource, SignatureVerificationState,
        };

        let directory = tempfile::tempdir().expect("temp dir must be created");
        let file_path = directory.path().join("release.txt");
        std::fs::write(&file_path, b"released bytes").expect("evidence file must be written");
        let actual_digest = crate::integrity::sha256_hex(b"released bytes");
        let substituted_digest = crate::integrity::sha256_hex(b"substituted bytes");
        let document_id = DOCUMENT_ID as u64;

        // 1. Nothing recorded: there is no digest to compare the stored bytes against.
        let repository = repository_with_document().await;
        set_document_file(&repository, &file_path, None).await;
        let unrecorded = repository
            .verify_document_provenance(
                document_id,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Readable(file_path.clone()),
            )
            .await
            .expect("verification must run");
        assert_eq!(
            unrecorded.provenance_confidence,
            ProvenanceConfidence::NoRecordedDigest
        );
        assert_eq!(
            unrecorded.recorded_digest_source,
            RecordedDigestSource::None
        );
        assert!(!unrecorded.hash_matches);
        assert!(!unrecorded.manifest_present);

        // 2. The ingestion digest matches: local integrity, and nothing about provenance.
        let repository = repository_with_document().await;
        set_document_file(&repository, &file_path, Some(&actual_digest)).await;
        let local_match = repository
            .verify_document_provenance(
                document_id,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Readable(file_path.clone()),
            )
            .await
            .expect("verification must run");
        assert_eq!(
            local_match.provenance_confidence,
            ProvenanceConfidence::LocalDigestMatch
        );
        assert_eq!(
            local_match.recorded_digest_source,
            RecordedDigestSource::IngestionRecord
        );
        assert!(local_match.hash_matches);

        // 3. The stored bytes are not the bytes that were recorded.
        let repository = repository_with_document().await;
        set_document_file(&repository, &file_path, Some(&substituted_digest)).await;
        let mismatch = repository
            .verify_document_provenance(
                document_id,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Readable(file_path.clone()),
            )
            .await
            .expect("verification must run");
        assert_eq!(
            mismatch.provenance_confidence,
            ProvenanceConfidence::DigestMismatch
        );
        assert!(!mismatch.hash_matches);

        // 4. A stale manifest row claiming a signature, over a file that carries no embedded
        // manifest at all. Manifest state is read from the asset, so the stored claim must not
        // survive into the result: a row recorded earlier describes a file that may since have
        // been replaced, which is the whole reason verification reads the bytes in front of it.
        let repository = repository_with_document().await;
        set_document_file(&repository, &file_path, Some(&actual_digest)).await;
        record_manifest_claim(&repository, &actual_digest, true).await;
        let stale_claim = repository
            .verify_document_provenance(
                document_id,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Readable(file_path.clone()),
            )
            .await
            .expect("verification must run");
        assert!(
            !stale_claim.manifest_present,
            "a stored manifest row must not make an asset without one report as having one"
        );
        assert!(!stale_claim.is_cryptographically_signed);
        assert_eq!(
            stale_claim.signature_verification,
            SignatureVerificationState::NotAttempted,
            "there is no manifest to check, so no signature was checked"
        );
        assert_eq!(
            stale_claim.recorded_digest_source,
            RecordedDigestSource::IngestionRecord
        );
        assert_eq!(
            stale_claim.provenance_confidence,
            ProvenanceConfidence::LocalDigestMatch
        );
        // The superseded row is gone from storage, not merely ignored in the response.
        {
            let connection = repository.conn.lock().await;
            let remaining: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM provenance_manifests
                     WHERE document_id = ?1 AND portal_subdomain = ?2",
                    params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                    |row| row.get(0),
                )
                .expect("manifest count must read");
            assert_eq!(remaining, 0);
        }

        for result in [&unrecorded, &local_match, &mismatch, &stale_claim] {
            assert!(
                !result.chain_of_custody_confirmed,
                "no state this build can produce is independent custody evidence, but {:?} claimed it",
                result.provenance_confidence
            );
            assert!(!result.provenance_confidence.is_cryptographically_verified());
        }
    }

    /// A document whose evidence file cannot be read must still produce a recorded verification.
    ///
    /// This is the case the previous implementation lost: it returned an error, so nothing reached
    /// the append-only log and an investigator had no record that they looked and found the file
    /// gone. The reason has to survive into the result, because "never downloaded" and "the
    /// recorded path escaped the evidence store" are very different facts.
    #[tokio::test]
    async fn unreadable_evidence_is_recorded_rather_than_raised_as_an_error() {
        use crate::types::{ProvenanceConfidence, RecordedDigestSource};

        let repository = repository_with_document().await;
        // No local_path has been written for this document yet.
        assert_eq!(
            repository
                .resolve_document_evidence_path(DOCUMENT_ID as u64, PORTAL_SUBDOMAIN)
                .await
                .expect("path resolution must succeed"),
            None
        );

        let result = repository
            .verify_document_provenance(
                DOCUMENT_ID as u64,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Unavailable(
                    "The recorded evidence path is a symbolic link, which the managed evidence \
                     store does not permit"
                        .to_string(),
                ),
            )
            .await
            .expect("an unreadable evidence file must not fail the verification");

        assert_eq!(
            result.provenance_confidence,
            ProvenanceConfidence::EvidenceFileUnavailable
        );
        assert!(!result.evidence_file_present);
        assert!(!result.hash_matches);
        assert!(result.computed_sha256.is_empty());
        assert_eq!(result.recorded_digest_source, RecordedDigestSource::None);
        assert!(
            result
                .verification_errors
                .iter()
                .any(|message| message.contains("symbolic link")),
            "the specific reason must survive into the result, got {:?}",
            result.verification_errors
        );

        let connection = repository.conn.lock().await;
        let logged: String = connection
            .query_row(
                "SELECT provenance_confidence FROM provenance_verification_log
                 WHERE document_id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                |row| row.get(0),
            )
            .expect("the verification must be appended to the log");
        assert_eq!(logged, "EVIDENCE_FILE_UNAVAILABLE");
    }

    /// Drift in the derived text must be detected, and must not implicate the asset.
    ///
    /// `extracted_text_sha256` covers the raw extracted text, which is discarded after
    /// finalization, so it can never be re-checked. The normalized projection is what search
    /// returns and what a legal brief quotes, and until it carried its own digest nothing could
    /// tell whether quoted text still matched the document it was drawn from.
    #[tokio::test]
    async fn extracted_text_drift_is_detected_without_implicating_the_asset() {
        use crate::types::ProvenanceConfidence;

        let directory = tempfile::tempdir().expect("temp dir must be created");
        let file_path = directory.path().join("release.txt");
        std::fs::write(&file_path, b"released bytes").expect("evidence file must be written");
        let file_digest = crate::integrity::sha256_hex(b"released bytes");

        let repository = repository_with_document().await;
        let extracted_text = "Encampment sweep notice\r\n[REDACTED]";
        let text_digest = compute_extracted_text_sha256(extracted_text);
        repository
            .finalize_document_extraction(DocumentExtractionFinalization {
                document_id: DOCUMENT_ID as u64,
                portal_subdomain: PORTAL_SUBDOMAIN,
                extracted_text,
                extracted_text_sha256: &text_digest,
                raw_file_sha256: &file_digest,
                was_ocr_processed: false,
                raw_file_size: 14,
                local_path: &file_path,
                shannon_entropy: Some(3.5),
                entropy_sample_bytes: 14,
                suspicious_data_density: false,
            })
            .await
            .expect("finalization must commit");

        let intact = repository
            .verify_document_provenance(
                DOCUMENT_ID as u64,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Readable(file_path.clone()),
            )
            .await
            .expect("verification must run");
        assert_eq!(
            intact.provenance_confidence,
            ProvenanceConfidence::LocalDigestMatch
        );
        assert_eq!(intact.text_projection_matches, Some(true));
        assert!(intact.text_projection_sha256.is_some());

        // Rewrite the projection while leaving the asset alone.
        {
            let connection = repository.conn.lock().await;
            connection
                .execute(
                    "UPDATE documents_fts SET extracted_text = ?1
                     WHERE document_id = ?2 AND portal_subdomain = ?3",
                    params![
                        "Encampment sweep notice with a sentence nobody released",
                        DOCUMENT_ID,
                        PORTAL_SUBDOMAIN
                    ],
                )
                .expect("projection must update");
        }

        let drifted = repository
            .verify_document_provenance(
                DOCUMENT_ID as u64,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Readable(file_path.clone()),
            )
            .await
            .expect("verification must run");
        assert!(drifted.hash_matches);
        assert_eq!(
            drifted.provenance_confidence,
            ProvenanceConfidence::LocalDigestMatch,
            "the asset is untouched, so the file verdict must not move"
        );
        assert_eq!(drifted.text_projection_matches, Some(false));
        assert!(
            drifted
                .verification_errors
                .iter()
                .any(|message| message.contains("quoted")),
            "drift must be described in terms of what it costs a citation, got {:?}",
            drifted.verification_errors
        );

        // A row imported from a legacy core has no recorded projection digest. That is reported
        // as unverifiable, never as a pass.
        {
            let connection = repository.conn.lock().await;
            connection
                .execute(
                    "UPDATE documents SET extracted_text_projection_sha256 = NULL
                     WHERE id = ?1 AND portal_subdomain = ?2",
                    params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                )
                .expect("legacy shape must be reproducible");
        }
        let unverifiable = repository
            .verify_document_provenance(
                DOCUMENT_ID as u64,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Readable(file_path),
            )
            .await
            .expect("verification must run");
        assert_eq!(unverifiable.text_projection_matches, None);
        assert!(unverifiable
            .verification_errors
            .iter()
            .any(|message| message.contains("cannot be verified")));

        let connection = repository.conn.lock().await;
        let drift_rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM provenance_verification_log
                 WHERE text_projection_matches = FALSE",
                [],
                |row| row.get(0),
            )
            .expect("the drift verdict must reach the log");
        assert_eq!(drift_rows, 1);
    }

    /// The append-only log must be readable, and a row written before classification existed must
    /// read back as unclassified rather than as a low-confidence result.
    #[tokio::test]
    async fn the_verification_log_reads_back_newest_first() {
        use crate::types::{ProvenanceConfidence, RecordedDigestSource};

        let directory = tempfile::tempdir().expect("temp dir must be created");
        let file_path = directory.path().join("release.txt");
        std::fs::write(&file_path, b"released bytes").expect("evidence file must be written");
        let digest = crate::integrity::sha256_hex(b"released bytes");

        let repository = repository_with_document().await;
        set_document_file(&repository, &file_path, Some(&digest)).await;
        for _ in 0..2 {
            repository
                .verify_document_provenance(
                    DOCUMENT_ID as u64,
                    PORTAL_SUBDOMAIN,
                    ProvenanceEvidenceFile::Readable(file_path.clone()),
                )
                .await
                .expect("verification must run");
        }
        {
            let connection = repository.conn.lock().await;
            connection
                .execute(
                    "INSERT INTO provenance_verification_log (
                        verification_id, document_id, portal_subdomain, verified_at,
                        expected_sha256, computed_sha256, hash_matches, manifest_present,
                        is_cryptographically_signed, invalid_assertion_count,
                        chain_of_custody_confirmed, verification_errors_json,
                        recorded_digest_source, signature_verification, provenance_confidence
                     ) VALUES ('legacy-row', ?1, ?2, '2020-01-01T00:00:00Z', NULL, 'abc', TRUE,
                        FALSE, FALSE, 0, FALSE, '[]', 'UNCLASSIFIED_LEGACY_RECORD',
                        'UNCLASSIFIED_LEGACY_RECORD', 'UNCLASSIFIED_LEGACY_RECORD')",
                    params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                )
                .expect("a pre-classification row must be insertable");
        }

        let history = repository
            .list_document_provenance_verifications(DOCUMENT_ID as u64, PORTAL_SUBDOMAIN, 50)
            .await
            .expect("history must read");
        assert_eq!(history.len(), 3);

        let current = &history[0];
        assert!(!current.predates_classification);
        assert_eq!(
            current.provenance_confidence,
            Some(ProvenanceConfidence::LocalDigestMatch)
        );
        assert_eq!(
            current.recorded_digest_source,
            Some(RecordedDigestSource::IngestionRecord)
        );
        assert_eq!(current.evidence_file_present, Some(true));
        assert_eq!(current.assertion_count, Some(0));
        assert_eq!(current.computed_sha256, digest);

        let legacy = history.last().expect("the oldest row must be present");
        assert_eq!(legacy.verification_id, "legacy-row");
        assert!(
            legacy.predates_classification,
            "a sentinel row must be marked unclassified"
        );
        assert_eq!(legacy.provenance_confidence, None);
        assert_eq!(legacy.recorded_digest_source, None);
        assert_eq!(legacy.signature_verification, None);
        assert_eq!(legacy.assertion_count, None);
        assert_eq!(legacy.evidence_file_present, None);

        let limited = repository
            .list_document_provenance_verifications(DOCUMENT_ID as u64, PORTAL_SUBDOMAIN, 1)
            .await
            .expect("history must read");
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].verification_id, current.verification_id);
    }

    /// A corrupt recorded digest must not be reported as a digest mismatch.
    ///
    /// `mark_document_downloaded` normalizes on write, but a row imported from a legacy core
    /// carries whatever that database held. Comparing against a truncated or non-hex value would
    /// produce `DIGEST_MISMATCH` and accuse an intact file of having been substituted.
    #[tokio::test]
    async fn a_malformed_recorded_digest_is_not_reported_as_tampering() {
        use crate::types::ProvenanceConfidence;

        let directory = tempfile::tempdir().expect("temp dir must be created");
        let file_path = directory.path().join("release.txt");
        std::fs::write(&file_path, b"released bytes").expect("evidence file must be written");

        let repository = repository_with_document().await;
        set_document_file(&repository, &file_path, Some("not-a-sha256")).await;
        let result = repository
            .verify_document_provenance(
                DOCUMENT_ID as u64,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Readable(file_path.clone()),
            )
            .await
            .expect("verification must run");

        assert_ne!(
            result.provenance_confidence,
            ProvenanceConfidence::DigestMismatch,
            "a corrupt record must not be presented as evidence of substitution"
        );
        assert_eq!(
            result.provenance_confidence,
            ProvenanceConfidence::NoRecordedDigest
        );
        assert!(!result.hash_matches);
        assert!(result.evidence_file_present);
        assert!(
            result
                .verification_errors
                .iter()
                .any(|message| message.contains("well-formed")),
            "the corrupt record must be named as such, got {:?}",
            result.verification_errors
        );
    }

    /// Startup runs the migration script on every launch, so a second run must be a no-op rather
    /// than an error, and the provenance classification columns must survive it.
    #[tokio::test]
    async fn migrations_are_idempotent_and_persist_provenance_classification() {
        let directory = tempfile::tempdir().expect("temp dir must be created");
        let file_path = directory.path().join("release.txt");
        std::fs::write(&file_path, b"released bytes").expect("evidence file must be written");
        let actual_digest = crate::integrity::sha256_hex(b"released bytes");

        const CLASSIFICATION_COLUMNS: [&str; 3] = [
            "recorded_digest_source",
            "signature_verification",
            "provenance_confidence",
        ];

        let repository = repository_with_document().await;
        repository
            .run_migrations(MIGRATION)
            .await
            .expect("a second migration run must succeed");
        repository
            .run_migrations(MIGRATION)
            .await
            .expect("a third migration run must succeed");

        set_document_file(&repository, &file_path, Some(&actual_digest)).await;
        let before_upgrade = repository
            .verify_document_provenance(
                DOCUMENT_ID as u64,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Readable(file_path.clone()),
            )
            .await
            .expect("verification must run after repeated migrations");

        // Reproduce a database written before classification existed, with a row already in the
        // table. Adding a NOT NULL DEFAULT column to a non-empty table is what the upgrade path
        // actually performs, and what would fail every existing installation at startup if DuckDB
        // rejected it. A fresh database never reaches that branch, because the base schema creates
        // the columns outright and `ensure_column` is then a no-op.
        {
            let connection = repository.conn.lock().await;
            // DuckDB's rule, measured rather than assumed: an index blocks DROP COLUMN on its
            // table but not ADD COLUMN. That is what makes `ensure_column` safe to run against an
            // indexed log, and it is why reproducing the pre-classification shape below has to
            // drop the index first.
            connection
                .execute_batch(
                    "ALTER TABLE provenance_verification_log ADD COLUMN probe_column TEXT;",
                )
                .expect("ADD COLUMN must remain possible on an indexed table");
            connection
                .execute_batch("ALTER TABLE provenance_verification_log DROP COLUMN probe_column;")
                .expect_err("DROP COLUMN must be refused while an index depends on the table");
            connection
                .execute_batch("DROP INDEX idx_provenance_verification_document;")
                .expect("the log index must be droppable");
            connection
                .execute_batch("ALTER TABLE provenance_verification_log DROP COLUMN probe_column;")
                .expect("with the index gone the column must be removable");
            for column in CLASSIFICATION_COLUMNS {
                connection
                    .execute_batch(&format!(
                        "ALTER TABLE provenance_verification_log DROP COLUMN {column};"
                    ))
                    .expect("the pre-classification shape must be reproducible");
            }
        }
        repository
            .run_migrations(MIGRATION)
            .await
            .expect("the upgrade path must restore the classification columns");

        let after_upgrade = repository
            .verify_document_provenance(
                DOCUMENT_ID as u64,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Readable(file_path.clone()),
            )
            .await
            .expect("verification must run after the upgrade");
        assert_eq!(
            before_upgrade.provenance_confidence, after_upgrade.provenance_confidence,
            "the upgrade must not change how a document is classified"
        );

        let connection = repository.conn.lock().await;
        // A row that predates the columns cannot be reclassified after the fact, so it must carry
        // the sentinel rather than a plausible-looking value.
        let legacy_rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM provenance_verification_log
                 WHERE provenance_confidence = 'UNCLASSIFIED_LEGACY_RECORD'
                   AND signature_verification = 'UNCLASSIFIED_LEGACY_RECORD'
                   AND recorded_digest_source = 'UNCLASSIFIED_LEGACY_RECORD'",
                [],
                |row| row.get(0),
            )
            .expect("the sentinel default must reach rows written before the upgrade");
        assert_eq!(legacy_rows, 1);

        let (confidence, digest_source, signature_state): (String, String, String) = connection
            .query_row(
                "SELECT provenance_confidence, recorded_digest_source, signature_verification
                 FROM provenance_verification_log
                 WHERE document_id = ?1 AND portal_subdomain = ?2
                   AND provenance_confidence <> 'UNCLASSIFIED_LEGACY_RECORD'",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("a verification written after the upgrade must be classified");
        assert_eq!(confidence, after_upgrade.provenance_confidence.as_str());
        assert_eq!(digest_source, after_upgrade.recorded_digest_source.as_str());
        assert_eq!(
            signature_state,
            after_upgrade.signature_verification.as_str()
        );
    }

    /// DuckDB's ADD COLUMN rule, measured against the linked library rather than carried over
    /// from a CLI or a release note. The shape of every `ensure_column` definition depends on it,
    /// and a future DuckDB bump that adds support should surface here rather than silently leave
    /// the shims weaker than they need to be.
    #[test]
    fn duckdb_add_column_takes_a_default_but_no_constraint() {
        let connection = duckdb::Connection::open_in_memory().expect("an in-memory core must open");
        connection
            .execute_batch("CREATE TABLE probe (id INTEGER); INSERT INTO probe VALUES (1);")
            .expect("the probe table must be created");

        for definition in [
            "TEXT NOT NULL DEFAULT ''",
            "TEXT NOT NULL",
            "REAL DEFAULT 0.0 CHECK (added >= 0.0)",
            "INTEGER CHECK (added >= 0)",
        ] {
            connection
                .execute_batch(&format!("ALTER TABLE probe ADD COLUMN added {definition}"))
                .expect_err("a constrained ADD COLUMN must stay refused");
        }
        connection
            .execute_batch("ALTER TABLE probe ADD CONSTRAINT bound CHECK (id >= 0)")
            .expect_err("adding a constraint after the fact must stay unimplemented");

        connection
            .execute_batch("ALTER TABLE probe ADD COLUMN added TEXT DEFAULT 'backfilled'")
            .expect("a bare DEFAULT must remain accepted");
        let backfilled: String = connection
            .query_row("SELECT added FROM probe", [], |row| row.get(0))
            .expect("the default must reach a row written before the column existed");
        assert_eq!(
            backfilled, "backfilled",
            "the DEFAULT is what makes an upgraded database hold the same values as a fresh one"
        );

        // Not a hypothetical: every table these shims touch except `portals` is indexed, and
        // `portals` gaining an index later must not turn a working migration into a startup
        // failure. That is why no shim reaches for SET NOT NULL as a second step.
        connection
            .execute_batch("CREATE INDEX idx_probe ON probe(id);")
            .expect("the probe index must be created");
        connection
            .execute_batch("ALTER TABLE probe ALTER COLUMN added SET NOT NULL")
            .expect_err("SET NOT NULL must stay refused while an index depends on the table");
    }

    /// The rule rather than the symptom. A definition DuckDB cannot apply is refused here, where
    /// the message names the column and the fix, instead of at startup on an installation old
    /// enough to be missing the column -- which is never a developer's machine, which is why
    /// fifteen of these went unnoticed.
    #[test]
    fn ensure_column_refuses_a_definition_duckdb_cannot_add() {
        let connection = duckdb::Connection::open_in_memory().expect("an in-memory core must open");
        connection
            .execute_batch("CREATE TABLE probe (id INTEGER);")
            .expect("the probe table must be created");

        let refusal =
            super::ensure_column(&connection, "probe", "added", "TEXT NOT NULL DEFAULT ''")
                .expect_err("a constrained definition must be refused");
        assert!(
            format!("{refusal:?}").contains("probe.added"),
            "the refusal must name the offending column, got: {refusal:?}"
        );
        super::ensure_column(&connection, "probe", "added", "TEXT DEFAULT ''")
            .expect("a type and a DEFAULT must be accepted");
        // The match is on the parser's token, not the word: this definition is legal SQL and
        // refusing it would abort startup for the reason the guard exists to prevent.
        super::ensure_column(&connection, "probe", "state", "TEXT DEFAULT 'UNCHECKED'")
            .expect("a DEFAULT literal containing the word must still be accepted");
    }

    /// The compatibility shims only ever ran against databases that already had the column, so a
    /// definition DuckDB cannot apply was a no-op everywhere except the upgrade the shim exists to
    /// perform. This builds the two pre-upgrade shapes directly rather than stripping columns from
    /// a current database -- DuckDB refuses DROP COLUMN on a table anything depends on, and both
    /// of these have dependents -- so what the migration runs against is the column set an older
    /// build actually wrote. It must complete, and the rows already there must come back carrying
    /// the declared defaults rather than NULL.
    #[tokio::test]
    async fn migrations_restore_columns_missing_from_an_older_database() {
        let repository =
            DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
        {
            let connection = repository.conn.lock().await;
            // `CREATE TABLE IF NOT EXISTS` in the base schema leaves both of these alone, so the
            // shims are the only thing that can complete them.
            connection
                .execute_batch(
                    "CREATE TABLE portals (
                        subdomain TEXT PRIMARY KEY,
                        display_name TEXT NOT NULL,
                        base_url TEXT NOT NULL
                     );
                     CREATE TABLE legal_statutes (
                        statute_id TEXT PRIMARY KEY,
                        jurisdiction TEXT NOT NULL,
                        statute_citation TEXT NOT NULL,
                        title TEXT NOT NULL,
                        full_text TEXT NOT NULL,
                        mandated_deadline_days BIGINT
                     );
                     INSERT INTO legal_statutes VALUES (
                        'legacy-statute', 'CA', 'CPRA 7922.535', 'Legacy Authority',
                        'The agency shall respond within ten days.', 10
                     );
                     CREATE TABLE documents (
                        id BIGINT NOT NULL,
                        request_id BIGINT NOT NULL,
                        portal_subdomain TEXT NOT NULL,
                        visibility TEXT NOT NULL,
                        created_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL,
                        download_status TEXT DEFAULT 'pending',
                        PRIMARY KEY (id, portal_subdomain)
                     );
                     CREATE TABLE portal_sync_runs (
                        run_id TEXT PRIMARY KEY,
                        portal_subdomain TEXT NOT NULL,
                        started_at TEXT NOT NULL,
                        completed_at TEXT,
                        status TEXT NOT NULL,
                        pages_processed BIGINT NOT NULL DEFAULT 0,
                        requests_processed BIGINT NOT NULL DEFAULT 0,
                        documents_processed BIGINT NOT NULL DEFAULT 0,
                        failures BIGINT NOT NULL DEFAULT 0
                     );
                     INSERT INTO portals
                     VALUES ('legacy-portal', 'Legacy Portal', 'https://legacy.test');
                     INSERT INTO portal_sync_runs (
                        run_id, portal_subdomain, started_at, status
                     ) VALUES (
                        'legacy-run', 'legacy-portal', '2026-08-24T12:00:00Z', 'CANCELLED'
                     );",
                )
                .expect("the pre-upgrade tables must be creatable");
            connection
                .execute(
                    "INSERT INTO documents (
                        id, request_id, portal_subdomain, visibility, created_at, updated_at
                     ) VALUES (?1, ?2, 'legacy-portal', 'public', ?3, ?3)",
                    params![DOCUMENT_ID, REQUEST_ID, "2026-08-24T12:00:00Z"],
                )
                .expect("the pre-upgrade document row must insert");
        }

        repository
            .run_migrations(MIGRATION)
            .await
            .expect("the upgrade path must complete against tables written before it");

        let connection = repository.conn.lock().await;
        let (origin, pointer, health, kind): (String, String, String, String) = connection
            .query_row(
                "SELECT origin_url, requests_pointer, health_status, source_kind
                 FROM portals WHERE subdomain = 'legacy-portal'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("the portal row must survive the upgrade carrying its defaults");
        assert_eq!(origin, "");
        assert_eq!(pointer, "/requests");
        assert_eq!(health, "verified");
        assert_eq!(kind, "NEXT_REQUEST");
        let scheduler_counts: (i64, i64) = connection
            .query_row(
                "SELECT crawl_pages_started, resources_started
                 FROM portal_sync_runs WHERE run_id = 'legacy-run'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("legacy scheduler counters must be backfilled explicitly");
        assert_eq!(scheduler_counts, (0, 0));

        // `prepare_legacy_legal_schema` runs before the base schema and is gated on the table
        // already existing, so its five shims are unreachable on a fresh database and were never
        // covered until this stub put a pre-upgrade `legal_statutes` in front of them.
        let (authority_kind, digest): (String, String) = connection
            .query_row(
                "SELECT authority_kind, content_sha256 FROM legal_statutes
                 WHERE statute_id = 'legacy-statute'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("the legacy statute must survive the repair path carrying its default");
        assert_eq!(authority_kind, "OTHER");
        assert_eq!(
            digest.len(),
            64,
            "the repair path must replace the placeholder digest with one over the stored text"
        );

        let (entropy, sample_bytes, density): (Option<f64>, i64, i64) = connection
            .query_row(
                "SELECT shannon_entropy, entropy_sample_bytes, suspicious_data_density
                 FROM documents WHERE id = ?1 AND portal_subdomain = 'legacy-portal'",
                params![DOCUMENT_ID],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("the document row must survive the upgrade carrying its defaults");
        // `shannon_entropy` alone takes no DEFAULT: this row was never assessed, and 0.0 would
        // claim every byte of it was identical. The other two defaults are honest claims.
        assert_eq!(
            entropy, None,
            "an upgraded row that was never assessed must read as absent, not as measured 0.0"
        );
        assert_eq!(sample_bytes, 0);
        assert_eq!(density, 0);
    }

    /// Core search uses the persistent exact-token projection and requires every plain-text term.
    /// Boolean words have no special meaning because the typed frontend does not offer a Boolean
    /// grammar; qualifiers are passed separately to `search_requests`.
    #[tokio::test]
    async fn core_search_uses_persistent_exact_tokens_and_all_term_matching() {
        let repository = repository_with_document().await;
        {
            let connection = repository.conn.lock().await;
            connection
                .execute(
                    "INSERT INTO documents_fts (
                        document_id, portal_subdomain, file_name, extracted_text
                     ) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        DOCUMENT_ID,
                        PORTAL_SUBDOMAIN,
                        "release.pdf",
                        "Encampment sweep notice",
                    ],
                )
                .expect("search projection fixture must insert");
        }
        repository
            .run_migrations(MIGRATION)
            .await
            .expect("startup must backfill a legacy projection row");
        let document_id = DOCUMENT_ID as u64;

        assert!(repository
            .full_text_search("ncampmen")
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            repository
                .full_text_search("encampment notice")
                .await
                .unwrap(),
            vec![document_id]
        );
        assert!(repository
            .full_text_search("encampment absent")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn legal_authority_versions_are_listed_and_loaded_without_losing_text() {
        let repository = DuckDbRepository::new(":memory:").expect("DuckDB core must open");
        repository
            .run_migrations(MIGRATION)
            .await
            .expect("migration must run");
        {
            let connection = repository.conn.lock().await;
            connection.execute(
                "INSERT INTO legal_statutes (
                    statute_id, jurisdiction, statute_citation, title, authority_kind, full_text,
                    current_version_id, content_sha256, imported_at, updated_at
                 ) VALUES ('policy-1', 'Test', 'Policy 1', 'Test policy', 'AGENCY_POLICY',
                    'Second text', 'version-2', ?1, '2026-02-01T00:00:00Z', '2026-02-01T00:00:00Z')",
                params!["b".repeat(64)],
            ).expect("authority must insert");
            connection.execute(
                "INSERT INTO legal_statute_versions (
                    version_id, statute_id, full_text, content_sha256, effective_from, imported_at
                 ) VALUES ('version-1', 'policy-1', 'First text', ?1, '2026-01-01', '2026-01-01T00:00:00Z'),
                          ('version-2', 'policy-1', 'Second text', ?2, '2026-02-01', '2026-02-01T00:00:00Z')",
                params!["a".repeat(64), "b".repeat(64)],
            ).expect("versions must insert");
        }
        let versions = repository
            .list_legal_authority_versions("policy-1")
            .await
            .unwrap();
        assert_eq!(
            versions
                .iter()
                .map(|version| version.version_id.as_str())
                .collect::<Vec<_>>(),
            vec!["version-2", "version-1"]
        );
        let first = repository
            .get_legal_authority_version("policy-1", "version-1")
            .await
            .unwrap();
        assert_eq!(first.full_text, "First text");
        assert_eq!(first.version.effective_from.as_deref(), Some("2026-01-01"));
    }

    #[tokio::test]
    async fn durable_frontier_leases_and_acknowledges_tasks_exactly_once() {
        let repository = repository_with_document().await;
        repository
            .begin_portal_sync_run("run-frontier", PORTAL_SUBDOMAIN)
            .await
            .expect("sync run should begin");
        assert!(repository
            .persist_ingestion_frontier_task(
                "run-frontier",
                PORTAL_SUBDOMAIN,
                "target-1",
                r#"{"taskKind":"fetchMetadataPage"}"#,
            )
            .await
            .expect("frontier task should persist"));
        assert!(!repository
            .persist_ingestion_frontier_task(
                "run-frontier",
                PORTAL_SUBDOMAIN,
                "target-1",
                r#"{"taskKind":"fetchMetadataPage"}"#,
            )
            .await
            .expect("duplicate frontier task should be ignored"));

        let claimed = repository
            .claim_ingestion_frontier_task("run-frontier", PORTAL_SUBDOMAIN)
            .await
            .expect("frontier claim should succeed")
            .expect("frontier task should be available");
        assert!(repository
            .claim_ingestion_frontier_task("run-frontier", PORTAL_SUBDOMAIN)
            .await
            .expect("second frontier claim should succeed")
            .is_none());
        repository
            .complete_ingestion_frontier_task(claimed.frontier_id)
            .await
            .expect("frontier acknowledgement should succeed");
        assert!(repository
            .claim_ingestion_frontier_task("run-frontier", PORTAL_SUBDOMAIN)
            .await
            .expect("empty frontier claim should succeed")
            .is_none());
    }

    #[tokio::test]
    async fn interrupted_frontier_is_recovered_into_a_new_run() {
        let repository = repository_with_document().await;
        repository
            .begin_portal_sync_run("run-before-crash", PORTAL_SUBDOMAIN)
            .await
            .expect("first sync run should begin");
        for target in ["leased-at-crash", "pending-at-crash"] {
            repository
                .persist_ingestion_frontier_task(
                    "run-before-crash",
                    PORTAL_SUBDOMAIN,
                    target,
                    r#"{"taskKind":"fetchMetadataPage","subdomain":"test-portal","page":1,"perPage":50}"#,
                )
                .await
                .expect("recovery fixture should persist");
        }
        repository
            .claim_ingestion_frontier_task("run-before-crash", PORTAL_SUBDOMAIN)
            .await
            .expect("frontier lease should succeed")
            .expect("first task should be leased");
        assert_eq!(
            repository
                .reserve_ingestion_crawl_page("run-before-crash", PORTAL_SUBDOMAIN, 2)
                .await
                .unwrap(),
            Some(1)
        );
        assert_eq!(
            repository
                .reserve_ingestion_crawl_page("run-before-crash", PORTAL_SUBDOMAIN, 2)
                .await
                .unwrap(),
            Some(2)
        );
        assert_eq!(
            repository
                .reserve_ingestion_resource("run-before-crash", PORTAL_SUBDOMAIN, 1)
                .await
                .unwrap(),
            Some(1)
        );

        let interrupted = repository
            .recover_interrupted_source_keys()
            .await
            .expect("interrupted run scan should succeed");
        assert_eq!(interrupted, vec![PORTAL_SUBDOMAIN.to_string()]);

        repository
            .begin_portal_sync_run("run-after-restart", PORTAL_SUBDOMAIN)
            .await
            .expect("replacement sync run should begin");
        assert_eq!(
            repository
                .adopt_orphaned_frontier_tasks(PORTAL_SUBDOMAIN, "run-after-restart")
                .await
                .expect("interrupted tasks should be adopted"),
            2
        );
        assert_eq!(
            repository
                .ingestion_scheduler_counts("run-after-restart", PORTAL_SUBDOMAIN)
                .await
                .unwrap(),
            IngestionSchedulerCounts {
                crawl_pages_started: 2,
                resources_started: 1,
            }
        );
        assert_eq!(
            repository
                .reserve_ingestion_crawl_page("run-after-restart", PORTAL_SUBDOMAIN, 2)
                .await
                .unwrap(),
            None,
            "restart must not reset the configured page budget"
        );
        for _ in 0..2 {
            let task = repository
                .claim_ingestion_frontier_task("run-after-restart", PORTAL_SUBDOMAIN)
                .await
                .expect("adopted task claim should succeed")
                .expect("both interrupted tasks must be recoverable");
            repository
                .complete_ingestion_frontier_task(task.frontier_id)
                .await
                .expect("adopted task should acknowledge");
        }
        assert!(repository
            .claim_ingestion_frontier_task("run-after-restart", PORTAL_SUBDOMAIN)
            .await
            .expect("empty recovered frontier query should succeed")
            .is_none());
    }

    #[tokio::test]
    async fn startup_recovery_failure_is_exposed_through_saved_source_health() {
        let repository = repository_with_document().await;
        {
            let connection = repository.conn.lock().await;
            connection
                .execute(
                    "UPDATE portals SET requests_endpoint = '/requests' WHERE subdomain = ?1",
                    params![PORTAL_SUBDOMAIN],
                )
                .expect("source fixture must be listable");
        }
        repository
            .record_source_recovery_failure(
                PORTAL_SUBDOMAIN,
                "connector recipe loading",
                &AppError::DataParsing("recipe bytes are invalid".to_string()),
            )
            .await
            .expect("recovery failure must persist");

        let saved = repository
            .list_portal_registrations()
            .await
            .expect("saved sources must load");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].health_status, "degraded");
        let last_error = saved[0]
            .last_error
            .as_deref()
            .expect("durable error must be exposed");
        assert!(last_error.contains("connector recipe loading"));
        assert!(last_error.contains("recipe bytes are invalid"));
    }

    /// A run the operator stopped ends `CANCELLED`, not `INTERRUPTED`, yet its frontier is just as
    /// resumable: the reason a run stopped says nothing about whether its queued work survives.
    /// Before this was fixed, adoption matched `INTERRUPTED` alone, so a stopped source's frontier
    /// rows were never adopted and never deleted. They kept the source in the startup recovery
    /// list forever while every relaunch re-crawled from page one.
    #[tokio::test]
    async fn a_cancelled_runs_frontier_is_adopted_by_the_run_that_replaces_it() {
        let repository = repository_with_document().await;
        repository
            .begin_portal_sync_run("run-before-stop", PORTAL_SUBDOMAIN)
            .await
            .expect("first sync run should begin");
        for target in ["queued-at-stop", "leased-at-stop"] {
            repository
                .persist_ingestion_frontier_task(
                    "run-before-stop",
                    PORTAL_SUBDOMAIN,
                    target,
                    r#"{"taskKind":"fetchMetadataPage","subdomain":"test-portal","page":1,"perPage":50}"#,
                )
                .await
                .expect("stop fixture should persist");
        }
        repository
            .claim_ingestion_frontier_task("run-before-stop", PORTAL_SUBDOMAIN)
            .await
            .expect("frontier lease should succeed")
            .expect("first task should be leased");
        repository
            .finish_portal_sync_run(PortalSyncRunFinalization {
                run_id: "run-before-stop",
                portal_key: PORTAL_SUBDOMAIN,
                status: "CANCELLED",
                pages_processed: 0,
                requests_processed: 0,
                documents_processed: 0,
                failures: 0,
            })
            .await
            .expect("an operator stop should finalize the run");

        repository
            .begin_portal_sync_run("run-after-resume", PORTAL_SUBDOMAIN)
            .await
            .expect("replacement sync run should begin");
        assert_eq!(
            repository
                .adopt_orphaned_frontier_tasks(PORTAL_SUBDOMAIN, "run-after-resume")
                .await
                .expect("stopped tasks should be adopted"),
            2,
            "a resumed source must inherit the frontier its stopped run left behind"
        );
        for _ in 0..2 {
            let task = repository
                .claim_ingestion_frontier_task("run-after-resume", PORTAL_SUBDOMAIN)
                .await
                .expect("adopted task claim should succeed")
                .expect("both stopped tasks must be recoverable");
            repository
                .complete_ingestion_frontier_task(task.frontier_id)
                .await
                .expect("adopted task should acknowledge");
        }
    }

    /// Pins the upgrade path and the guard it exists to support, together.
    ///
    /// The lifecycle marker has to be durable, because before it existed a stopped source was
    /// only stopped in memory: the next launch saw its frontier rows, read them as an interrupted
    /// crawl, and resumed a source the operator had deliberately halted. Nothing on a fresh
    /// database can catch that, so this builds the pre-lifecycle `portals` shape directly and
    /// upgrades it, the way an existing installation does.
    #[tokio::test]
    async fn a_halted_source_survives_the_upgrade_and_is_not_resumed_at_startup() {
        const LEGACY_PORTALS: &str = "CREATE TABLE portals (
            subdomain TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            base_url TEXT NOT NULL,
            origin_url TEXT NOT NULL DEFAULT '',
            requests_endpoint TEXT NOT NULL DEFAULT '',
            requests_pointer TEXT NOT NULL DEFAULT '/requests',
            metadata_pointer TEXT,
            page_parameter TEXT NOT NULL DEFAULT 'page',
            page_size_parameter TEXT NOT NULL DEFAULT 'per_page',
            verified_at TEXT,
            health_status TEXT NOT NULL DEFAULT 'verified',
            last_error TEXT,
            last_synced_at TEXT,
            source_kind TEXT NOT NULL DEFAULT 'NEXT_REQUEST',
            created_at TEXT DEFAULT strftime(current_timestamp, '%Y-%m-%dT%H:%M:%SZ')
        );";

        let repository =
            DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
        {
            let connection = repository.conn.lock().await;
            connection
                .execute_batch(LEGACY_PORTALS)
                .expect("the pre-lifecycle portal shape must be creatable");
            connection
                .execute(
                    "INSERT INTO portals (subdomain, display_name, base_url, requests_endpoint)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        PORTAL_SUBDOMAIN,
                        "Test Portal",
                        "https://example.test",
                        "/api/requests",
                    ],
                )
                .expect("a source registered before the upgrade must insert");
        }
        repository
            .run_migrations(MIGRATION)
            .await
            .expect("the upgrade path must add the lifecycle column to a populated table");

        assert_eq!(
            repository
                .source_lifecycle_state(PORTAL_SUBDOMAIN)
                .await
                .expect("lifecycle lookup must succeed"),
            Some(SourceLifecycleState::Active),
            "a source that predates the column was never halted, and must read as active"
        );

        repository
            .begin_portal_sync_run("run-before-pause", PORTAL_SUBDOMAIN)
            .await
            .expect("sync run should begin");
        repository
            .persist_ingestion_frontier_task(
                "run-before-pause",
                PORTAL_SUBDOMAIN,
                "queued-at-pause",
                r#"{"taskKind":"fetchMetadataPage","subdomain":"test-portal","page":1,"perPage":50}"#,
            )
            .await
            .expect("frontier fixture should persist");

        assert_eq!(
            repository
                .recover_interrupted_source_keys()
                .await
                .expect("startup recovery scan must succeed"),
            vec![PORTAL_SUBDOMAIN.to_string()],
            "an active source with queued work is exactly what startup recovery is for"
        );

        repository
            .set_source_lifecycle_state(PORTAL_SUBDOMAIN, SourceLifecycleState::Paused)
            .await
            .expect("pausing must record");
        assert!(
            repository
                .recover_interrupted_source_keys()
                .await
                .expect("startup recovery scan must succeed")
                .is_empty(),
            "a paused source must not be resumed at startup by the frontier it deliberately kept"
        );
        assert_eq!(
            repository
                .list_portal_registrations()
                .await
                .expect("listing must succeed")
                .len(),
            1,
            "a paused source is still the operator's source and stays listed"
        );

        repository
            .set_source_lifecycle_state(PORTAL_SUBDOMAIN, SourceLifecycleState::Removed)
            .await
            .expect("withdrawal must record");
        assert!(
            repository
                .list_portal_registrations()
                .await
                .expect("listing must succeed")
                .is_empty(),
            "a withdrawn source leaves the listing"
        );
        assert!(
            repository
                .recover_interrupted_source_keys()
                .await
                .expect("startup recovery scan must succeed")
                .is_empty(),
            "a withdrawn source must not be resurrected by startup recovery"
        );
    }

    #[tokio::test]
    async fn extraction_finalization_commits_metadata_audits_flags_and_fts_atomically() {
        let repository = repository_with_document().await;
        let extracted_text = "sweep\r\n[REDACTED]\0 [REDACTED]";
        let uppercase_hash = compute_extracted_text_sha256(extracted_text).to_ascii_uppercase();

        repository
            .finalize_document_extraction(DocumentExtractionFinalization {
                document_id: DOCUMENT_ID as u64,
                portal_subdomain: PORTAL_SUBDOMAIN,
                extracted_text,
                extracted_text_sha256: &uppercase_hash,
                raw_file_sha256: RAW_FILE_SHA256,
                was_ocr_processed: true,
                raw_file_size: 9_876,
                local_path: Path::new("/var/data/test-portal/release.pdf"),
                shannon_entropy: Some(7.875),
                entropy_sample_bytes: 9_876,
                suspicious_data_density: true,
            })
            .await
            .expect("finalization must commit");

        {
            let connection = repository.conn.lock().await;
            let document_state: (i64, String, String, i64, f64, i64, i64, String, String) =
                connection
                    .query_row(
                        "SELECT
                        file_size,
                        local_path,
                        extracted_text_sha256,
                        was_ocr_processed,
                        shannon_entropy,
                        entropy_sample_bytes,
                        suspicious_data_density,
                        download_status,
                        extraction_status
                     FROM documents
                     WHERE id = ?1 AND portal_subdomain = ?2",
                        params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                        |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                                row.get(5)?,
                                row.get(6)?,
                                row.get(7)?,
                                row.get(8)?,
                            ))
                        },
                    )
                    .expect("document state must exist");
            assert_eq!(
                document_state,
                (
                    9_876,
                    "/var/data/test-portal/release.pdf".to_string(),
                    uppercase_hash.to_ascii_lowercase(),
                    1,
                    7.875,
                    9_876,
                    1,
                    "completed".to_string(),
                    "SEARCHABLE".to_string(),
                )
            );

            let audit_status: String = connection
                .query_row(
                    "SELECT verification_status
                     FROM case_audits
                     WHERE audit_id = ?1",
                    params![format!("auto-{}-{}", PORTAL_SUBDOMAIN, REQUEST_ID)],
                    |row| row.get(0),
                )
                .expect("automatic audit must exist");
            assert_eq!(audit_status, "DataAnomalyFlagged");

            let mut flag_statement = connection
                .prepare(
                    "SELECT flag_type
                     FROM document_integrity_flags
                     WHERE audit_id = ?1
                     ORDER BY flag_type",
                )
                .expect("flag query must prepare");
            let flags = flag_statement
                .query_map(
                    params![format!("auto-{}-{}", PORTAL_SUBDOMAIN, REQUEST_ID)],
                    |row| row.get::<_, String>(0),
                )
                .expect("flag query must execute")
                .collect::<duckdb::Result<Vec<_>>>()
                .expect("flags must decode");
            assert_eq!(
                flags,
                vec![
                    "HEAVY_REDACTION_DENSITY".to_string(),
                    "SUSPICIOUSLY_FAST_CLOSURE".to_string(),
                    "SUSPICIOUS_DATA_DENSITY".to_string(),
                    "UPDATE_BEFORE_CREATION".to_string(),
                ]
            );

            let indexed_row: (i64, String) = connection
                .query_row(
                    "SELECT COUNT(*), MIN(extracted_text)
                     FROM documents_fts
                     WHERE document_id = ?1 AND portal_subdomain = ?2",
                    params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("FTS state must exist");
            assert_eq!(indexed_row.0, 1);
            assert_eq!(indexed_row.1, "sweep [REDACTED] [REDACTED]");
        }

        assert_eq!(
            repository
                .full_text_search("sweep")
                .await
                .expect("search must succeed"),
            vec![DOCUMENT_ID as u64]
        );
    }

    #[tokio::test]
    async fn guidance_discard_removes_only_unfinalized_documents() {
        let unfinalized = repository_with_document().await;
        assert!(unfinalized
            .delete_unextracted_document(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64)
            .await
            .expect("an unfinalized stub must be removable"));
        {
            let connection = unfinalized.conn.lock().await;
            let remaining: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM documents WHERE id = ?1 AND portal_subdomain = ?2",
                    params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                    |row| row.get(0),
                )
                .expect("document count must decode");
            assert_eq!(remaining, 0);
        }

        let finalized = repository_with_document().await;
        let extracted_text = "responsive record";
        let extracted_text_sha256 = compute_extracted_text_sha256(extracted_text);
        finalized
            .finalize_document_extraction(DocumentExtractionFinalization {
                document_id: DOCUMENT_ID as u64,
                portal_subdomain: PORTAL_SUBDOMAIN,
                extracted_text,
                extracted_text_sha256: &extracted_text_sha256,
                raw_file_sha256: RAW_FILE_SHA256,
                was_ocr_processed: false,
                raw_file_size: 100,
                local_path: Path::new("/var/data/test-portal/release.pdf"),
                shannon_entropy: Some(4.0),
                entropy_sample_bytes: 100,
                suspicious_data_density: false,
            })
            .await
            .expect("the document must finalize");
        assert!(!finalized
            .delete_unextracted_document(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64)
            .await
            .expect("a finalized document must be retained without error"));
        let connection = finalized.conn.lock().await;
        let counts: (i64, i64) = connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM documents WHERE id = ?1 AND portal_subdomain = ?2),
                    (SELECT COUNT(*) FROM documents_fts WHERE document_id = ?1 AND portal_subdomain = ?2)",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("document and FTS counts must decode");
        assert_eq!(counts, (1, 1));
    }

    #[tokio::test]
    async fn repeated_finalization_updates_the_single_fts_row_and_refreshes_flags() {
        let repository = repository_with_document().await;
        let path = Path::new("/var/data/test-portal/release.pdf");
        let first_text = "sweep [REDACTED] [REDACTED]";
        let first_hash = compute_extracted_text_sha256(first_text);
        let second_text = "ordinary retention schedule";
        let second_hash = compute_extracted_text_sha256(second_text);

        repository
            .finalize_document_extraction(DocumentExtractionFinalization {
                document_id: DOCUMENT_ID as u64,
                portal_subdomain: PORTAL_SUBDOMAIN,
                extracted_text: first_text,
                extracted_text_sha256: &first_hash,
                raw_file_sha256: RAW_FILE_SHA256,
                was_ocr_processed: false,
                raw_file_size: 100,
                local_path: path,
                shannon_entropy: Some(4.5),
                entropy_sample_bytes: 100,
                suspicious_data_density: false,
            })
            .await
            .expect("initial finalization must succeed");
        repository
            .finalize_document_extraction(DocumentExtractionFinalization {
                document_id: DOCUMENT_ID as u64,
                portal_subdomain: PORTAL_SUBDOMAIN,
                extracted_text: second_text,
                extracted_text_sha256: &second_hash,
                raw_file_sha256: RAW_FILE_SHA256,
                was_ocr_processed: false,
                raw_file_size: 120,
                local_path: path,
                shannon_entropy: Some(4.25),
                entropy_sample_bytes: 120,
                suspicious_data_density: false,
            })
            .await
            .expect("re-finalization must succeed");

        {
            let connection = repository.conn.lock().await;
            let indexed_row: (i64, String) = connection
                .query_row(
                    "SELECT COUNT(*), MIN(extracted_text)
                     FROM documents_fts
                     WHERE document_id = ?1 AND portal_subdomain = ?2",
                    params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("one refreshed FTS row must exist");
            assert_eq!(indexed_row, (1, "ordinary retention schedule".to_string()));

            let heavy_flag_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*)
                     FROM document_integrity_flags
                     WHERE audit_id = ?1 AND flag_type = ?2",
                    params![
                        format!("auto-{}-{}", PORTAL_SUBDOMAIN, REQUEST_ID),
                        "HEAVY_REDACTION_DENSITY",
                    ],
                    |row| row.get(0),
                )
                .expect("heavy flag count must decode");
            assert_eq!(heavy_flag_count, 0);
        }

        assert!(repository
            .full_text_search("sweep")
            .await
            .expect("old token search must succeed")
            .is_empty());
        assert_eq!(
            repository
                .full_text_search("retention")
                .await
                .expect("new token search must succeed"),
            vec![DOCUMENT_ID as u64]
        );
        assert_eq!(
            repository
                .full_text_search("ordinary retention")
                .await
                .expect("all-term search must succeed"),
            vec![DOCUMENT_ID as u64]
        );
        assert!(repository
            .full_text_search("sweep retention")
            .await
            .expect("all-term search must succeed")
            .is_empty());
    }

    #[tokio::test]
    async fn request_timeline_synchronization_is_idempotent_and_removes_stale_flags() {
        let repository = repository_with_document().await;
        let mut request = PublicRequest {
            id: REQUEST_ID as u64,
            rpid: None,
            title: Some("Timeline synchronization".to_string()),
            description: None,
            status: "closed".to_string(),
            current_status_id: None,
            created_at: "2026-08-24T12:00:00Z".to_string(),
            updated_at: "2026-08-24T11:59:00Z".to_string(),
            closed_at: Some("2026-08-24T12:04:00Z".to_string()),
            due_date: None,
            determination_at: None,
            determination_source: None,
            neighborhood_block: None,
            statutory_extension: None,
            documents: Vec::new(),
            agency: None,
            category: None,
            timeline_anomalies: vec![
                "UPDATE_BEFORE_CREATION".to_string(),
                "UPDATE_BEFORE_CREATION".to_string(),
                "SUSPICIOUSLY_FAST_CLOSURE".to_string(),
                "UNRECOGNIZED_CALLER_VALUE".to_string(),
            ],
        };

        repository
            .insert_or_update_request(PORTAL_SUBDOMAIN, &request)
            .await
            .expect("first request synchronization must succeed");
        repository
            .insert_or_update_request(PORTAL_SUBDOMAIN, &request)
            .await
            .expect("repeated request synchronization must succeed");

        let audit_id = format!("auto-{}-{}", PORTAL_SUBDOMAIN, REQUEST_ID);
        {
            let connection = repository.conn.lock().await;
            let flags: Vec<String> = connection
                .prepare(
                    "SELECT flag_type
                     FROM document_integrity_flags
                     WHERE audit_id = ?1
                     ORDER BY flag_type",
                )
                .expect("timeline flag query must prepare")
                .query_map(params![audit_id], |row| row.get(0))
                .expect("timeline flag query must execute")
                .collect::<duckdb::Result<Vec<_>>>()
                .expect("timeline flags must decode");
            assert_eq!(
                flags,
                vec![
                    "SUSPICIOUSLY_FAST_CLOSURE".to_string(),
                    "UPDATE_BEFORE_CREATION".to_string(),
                ]
            );
        }

        request.timeline_anomalies.clear();
        repository
            .insert_or_update_request(PORTAL_SUBDOMAIN, &request)
            .await
            .expect("cleared request synchronization must succeed");

        let connection = repository.conn.lock().await;
        let audit_state: (String, i64) = connection
            .query_row(
                "SELECT
                    verification_status,
                    (
                        SELECT COUNT(*)
                        FROM document_integrity_flags
                        WHERE document_integrity_flags.audit_id = case_audits.audit_id
                    )
                 FROM case_audits
                 WHERE audit_id = ?1",
                params![audit_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("cleared timeline audit state must decode");
        assert_eq!(audit_state, ("Compliant".to_string(), 0));
    }

    /// A finding whose two dates were both recorded is the only one this path may store at
    /// certainty. Nothing was inferred, so there is nothing to discount.
    #[tokio::test]
    async fn a_cpra_finding_built_from_recorded_dates_is_stored_at_certainty() {
        use crate::analysis::{DeterminationBasis, StatutoryExtensionStatus};

        // Receipt and a recorded portal activity 46 calendar days later, well past the 10-day
        // determination deadline, with no extension claimed.
        let analysis = crate::analysis::evaluate_statutory_compliance_at(
            REQUEST_ID as u64,
            "2026-01-05T09:00:00Z",
            Some("2026-02-20T09:00:00Z"),
            Some("2026-02-20T09:00:00Z"),
            None,
            "2026-03-01T00:00:00Z",
        )
        .expect("the evaluation must succeed");
        assert!(analysis.is_violation);
        assert_eq!(
            analysis.determination_basis,
            DeterminationBasis::EarliestRecordedActivityProxy
        );
        assert_eq!(
            analysis.extension_status,
            StatutoryExtensionStatus::NotInvoked
        );

        let variance_days = analysis.calendar_days_elapsed - analysis.allowed_calendar_days;
        assert_eq!(
            cpra_calendar_confidence_basis_points(&analysis, variance_days),
            10_000
        );
    }

    /// The same overrun measured to the observation instant rather than to anything the agency
    /// did is not the same claim, and neither is one whose extension notice was never recorded.
    /// Both used to be stored at the identical 10000 basis points.
    #[tokio::test]
    async fn a_cpra_finding_resting_on_an_unobserved_date_is_stored_below_certainty() {
        use crate::analysis::{DeterminationBasis, StatutoryExtensionStatus};

        // Nothing was recorded at or after receipt, so the period was measured to "now" and is
        // still running.
        let inferred = crate::analysis::evaluate_statutory_compliance_at(
            REQUEST_ID as u64,
            "2026-01-05T09:00:00Z",
            None,
            None,
            None,
            "2026-02-20T09:00:00Z",
        )
        .expect("the evaluation must succeed");
        assert!(inferred.is_violation);
        assert_eq!(
            inferred.determination_basis,
            DeterminationBasis::ObservationWithoutRecordedActivity
        );

        let variance_days = inferred.calendar_days_elapsed - inferred.allowed_calendar_days;
        let inferred_confidence = cpra_calendar_confidence_basis_points(&inferred, variance_days);
        assert!(
            inferred_confidence < 10_000,
            "an inferred determination date must not be stored at certainty, got {inferred_confidence}"
        );
        // 10000 - 2500 + (2500 / 2 * 30 / 30): a 36-day overrun recovers the full half-share of
        // the deduction the formula allows, and still lands short of certainty.
        assert_eq!(inferred_confidence, 8_750);

        // An extension claimed with no recorded notice discounts a finding whose determination
        // date was itself recorded.
        let mut unsourced_extension = inferred.clone();
        unsourced_extension.determination_basis = DeterminationBasis::EarliestRecordedActivityProxy;
        unsourced_extension.extension_status = StatutoryExtensionStatus::MissingNoticeSource;
        assert_eq!(
            cpra_calendar_confidence_basis_points(&unsourced_extension, variance_days),
            9_400
        );
    }

    /// Measures the rule `reject_narrow_identifier_core` rests on, against the linked library.
    ///
    /// If a future DuckDB lifts this, the narrow-core refusal can become a real migration and this
    /// test is what will say so.
    #[test]
    fn duckdb_refuses_to_widen_a_key_column() {
        let connection = duckdb::Connection::open_in_memory().expect("DuckDB must open");
        connection
            .execute_batch(
                "CREATE TABLE parent (id INTEGER NOT NULL, portal TEXT NOT NULL,
                     PRIMARY KEY (id, portal));
                 CREATE TABLE child (document_id INTEGER NOT NULL, portal TEXT NOT NULL,
                     FOREIGN KEY (document_id, portal) REFERENCES parent(id, portal));",
            )
            .expect("the narrow fixture must apply");

        let child =
            connection.execute_batch("ALTER TABLE child ALTER COLUMN document_id TYPE BIGINT");
        let parent = connection.execute_batch("ALTER TABLE parent ALTER COLUMN id TYPE BIGINT");
        assert!(
            child.is_err() && parent.is_err(),
            "DuckDB is expected to refuse widening a key column: child {child:?}, parent {parent:?}"
        );
    }

    /// A core written before identifiers were widened must say so, not fail as a binder error.
    ///
    /// Without this the widened schema aborts on the first foreign key whose `BIGINT` child will
    /// not bind to an `INTEGER` parent, and that error is raised from the setup hook, which panics.
    /// The operator sees a crash instead of a sentence naming what happened.
    #[tokio::test]
    async fn a_core_with_thirty_two_bit_identifiers_is_refused_with_its_reason() {
        let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB must open");
        {
            let connection = repository.conn.lock().await;
            connection
                .execute_batch(
                    "CREATE TABLE requests (id INTEGER NOT NULL, portal_subdomain TEXT NOT NULL,
                         status TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                         PRIMARY KEY (id, portal_subdomain));",
                )
                .expect("the narrow core fixture must apply");
        }

        let error = repository
            .run_migrations(MIGRATION)
            .await
            .expect_err("a narrow core must be refused");
        let message = error.to_string();
        assert!(
            message.contains("2147483647") && message.contains("cannot be upgraded automatically"),
            "the refusal must name the limit and say it cannot be repaired: {message}"
        );
    }

    /// Evidence search must prepare and run against DuckDB.
    ///
    /// It did not: the projection used `char(31)` as the separator, which SQLite accepts and
    /// DuckDB parses as a type keyword, so `search_requests` failed at prepare time with
    /// "Parser Error: syntax error at or near )". Every search in the evidence workspace returned
    /// an error banner and no results. Nothing caught it because no test ever called this method.
    #[tokio::test]
    async fn evidence_search_prepares_and_returns_matching_requests() {
        let repository = repository_with_document().await;
        let results = repository
            .search_requests("", None, None, None, None, 25)
            .await
            .expect("the evidence search must prepare against DuckDB");
        assert!(
            results
                .iter()
                .any(|result| result.request_id == REQUEST_ID as u64),
            "the fixture request must be findable: {results:?}"
        );

        // The separator is what broke, so exercise the path that reads it back apart.
        let filtered = repository
            .search_requests(
                "sweep",
                Some(PORTAL_SUBDOMAIN),
                None,
                Some("closed"),
                None,
                25,
            )
            .await
            .expect("a filtered evidence search must prepare too");
        assert!(filtered.len() <= results.len());
    }

    /// A portal identifier does not fit in 32 bits, and DuckDB's `INTEGER` is 32 bits.
    ///
    /// This schema was written against SQLite, where `INTEGER` is 64-bit. Under DuckDB a real
    /// source produced request id 5614575451713354 -- 53 bits -- and the entire ingestion
    /// transaction aborted with "Conversion Error: Type UINT64 with value 5614575451713354 can't
    /// be cast because the value is out of range for the destination type INT32", so the record
    /// was never stored at all. Every other test used identifiers small enough to fit.
    #[tokio::test]
    async fn a_portal_identifier_wider_than_thirty_two_bits_is_stored() {
        const WIDE_REQUEST_ID: u64 = 5_614_575_451_713_354;
        assert!(WIDE_REQUEST_ID > i64::from(i32::MAX) as u64);

        let repository = repository_with_document().await;
        let request = PublicRequest {
            id: WIDE_REQUEST_ID,
            rpid: None,
            title: Some("Nonprofit Explorer filing".to_string()),
            description: None,
            status: "open".to_string(),
            current_status_id: None,
            created_at: "2026-08-25T09:00:00Z".to_string(),
            updated_at: "2026-08-25T09:00:00Z".to_string(),
            closed_at: None,
            due_date: None,
            determination_at: None,
            determination_source: None,
            neighborhood_block: None,
            statutory_extension: None,
            documents: Vec::new(),
            agency: None,
            category: None,
            timeline_anomalies: Vec::new(),
        };
        repository
            .insert_or_update_request(PORTAL_SUBDOMAIN, &request)
            .await
            .expect("a 53-bit portal identifier must be storable");

        let stored: i64 = {
            let connection = repository.conn.lock().await;
            connection
                .query_row(
                    "SELECT id FROM requests WHERE id = ?1 AND portal_subdomain = ?2",
                    params![WIDE_REQUEST_ID as i64, PORTAL_SUBDOMAIN],
                    |row| row.get(0),
                )
                .expect("the wide identifier must round trip")
        };
        assert_eq!(stored as u64, WIDE_REQUEST_ID);
    }

    /// 0.0 Shannon entropy is a measurement -- every byte in the sample identical -- and the
    /// strongest uniformity signal the assessment can produce. A document nothing ever assessed
    /// must not read as that. With no DEFAULT on the column the two are separate values, and the
    /// validator refuses to mix them.
    #[tokio::test]
    async fn never_computed_entropy_reads_absent_while_a_measured_zero_reads_zero() {
        let repository = repository_with_document().await;

        async fn recorded_entropy(repository: &DuckDbRepository) -> Option<f64> {
            let connection = repository.conn.lock().await;
            connection
                .query_row(
                    "SELECT shannon_entropy FROM documents
                     WHERE id = ?1 AND portal_subdomain = ?2",
                    params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                    |row| row.get(0),
                )
                .expect("the fixture document must exist")
        }

        assert_eq!(
            recorded_entropy(&repository).await,
            None,
            "a document written before its entropy was computed must read as absent"
        );

        // A zero-byte evidence file: `calculate_shannon_entropy` scores an empty sample 0.0, so
        // this is a genuine measurement of zero and not a stand-in for absence.
        repository
            .mark_document_downloaded(DocumentDownloadFinalization {
                document_id: DOCUMENT_ID as u64,
                portal_subdomain: PORTAL_SUBDOMAIN,
                raw_file_size: 0,
                raw_file_sha256: RAW_FILE_SHA256,
                local_path: Path::new("/var/data/test-portal/empty.pdf"),
                shannon_entropy: Some(0.0),
                entropy_sample_bytes: 0,
                suspicious_data_density: false,
            })
            .await
            .expect("a zero-byte evidence file must finalize");

        assert_eq!(
            recorded_entropy(&repository).await,
            Some(0.0),
            "a measured zero must survive the round trip as a value, not collapse into absence"
        );

        // The two cannot be mixed: bytes that were read produced a score.
        assert!(matches!(
            validate_entropy_metadata(None, 64, 128, false),
            Err(crate::error::AppError::DataParsing(_))
        ));
    }

    #[tokio::test]
    async fn hash_mismatch_rejects_finalization_without_persisting_partial_state() {
        let repository = repository_with_document().await;
        let result = repository
            .finalize_document_extraction(DocumentExtractionFinalization {
                document_id: DOCUMENT_ID as u64,
                portal_subdomain: PORTAL_SUBDOMAIN,
                extracted_text: "responsive text",
                extracted_text_sha256:
                    "0000000000000000000000000000000000000000000000000000000000000000",
                raw_file_sha256: RAW_FILE_SHA256,
                was_ocr_processed: true,
                raw_file_size: 500,
                local_path: Path::new("/var/data/test-portal/release.pdf"),
                shannon_entropy: Some(4.0),
                entropy_sample_bytes: 500,
                suspicious_data_density: false,
            })
            .await;
        assert!(matches!(
            result,
            Err(crate::error::AppError::DataParsing(_))
        ));

        let connection = repository.conn.lock().await;
        let document_state: (Option<i64>, Option<String>, Option<String>, String) = connection
            .query_row(
                "SELECT file_size, local_path, extracted_text_sha256, download_status
                 FROM documents
                 WHERE id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("unfinalized document must still exist");
        assert_eq!(document_state, (None, None, None, "pending".to_string()));

        let audit_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM case_audits", [], |row| row.get(0))
            .expect("audit count must decode");
        let fts_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM documents_fts", [], |row| row.get(0))
            .expect("FTS count must decode");
        assert_eq!((audit_count, fts_count), (0, 0));
    }

    #[tokio::test]
    async fn logical_relation_audit_names_orphans_without_modifying_them() {
        let repository = repository_with_document().await;
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO documents (
                    id, request_id, portal_subdomain, visibility, created_at, updated_at
                 ) VALUES (999, 998, ?1, 'public', '2026-08-27T00:00:00Z', '2026-08-27T00:00:00Z')",
                params![PORTAL_SUBDOMAIN],
            )
            .expect("the intentionally unconstrained orphan fixture must insert");

        let error = verify_logical_relations(&connection)
            .expect_err("the startup consistency audit must refuse an orphan");
        assert!(
            error
                .to_string()
                .contains("documents -> requests: 1 orphan row"),
            "the report must identify the broken relation and count: {error}"
        );
        let retained: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE id = 999 AND portal_subdomain = ?1",
                params![PORTAL_SUBDOMAIN],
                |row| row.get(0),
            )
            .expect("the orphan count must decode");
        assert_eq!(
            retained, 1,
            "the audit must never delete canonical evidence"
        );
    }

    #[tokio::test]
    async fn fts_failure_rolls_back_document_and_audit_updates() {
        let repository = repository_with_document().await;
        let extracted_text = "responsive text";
        let extracted_text_sha256 = compute_extracted_text_sha256(extracted_text);
        {
            let connection = repository.conn.lock().await;
            connection
                .execute("DROP TABLE documents_fts", [])
                .expect("FTS table must drop for the failure fixture");
        }

        let result = repository
            .finalize_document_extraction(DocumentExtractionFinalization {
                document_id: DOCUMENT_ID as u64,
                portal_subdomain: PORTAL_SUBDOMAIN,
                extracted_text,
                extracted_text_sha256: &extracted_text_sha256,
                raw_file_sha256: RAW_FILE_SHA256,
                was_ocr_processed: false,
                raw_file_size: 500,
                local_path: Path::new("/var/data/test-portal/release.pdf"),
                shannon_entropy: Some(4.0),
                entropy_sample_bytes: 500,
                suspicious_data_density: false,
            })
            .await;
        assert!(result.is_err());

        let connection = repository.conn.lock().await;
        let document_state: (Option<i64>, Option<String>, Option<String>, String) = connection
            .query_row(
                "SELECT file_size, local_path, extracted_text_sha256, download_status
                 FROM documents
                 WHERE id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("rolled-back document must still exist");
        assert_eq!(document_state, (None, None, None, "pending".to_string()));

        let audit_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM case_audits WHERE audit_id = ?1",
                params![format!("auto-{}-{}", PORTAL_SUBDOMAIN, REQUEST_ID)],
                |row| row.get(0),
            )
            .expect("audit count must decode");
        assert_eq!(audit_count, 0);
    }
}
