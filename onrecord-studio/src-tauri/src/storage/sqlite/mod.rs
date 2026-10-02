//! Thread-safe DuckDB repository layer managing persistent storage and data indexing.

mod analytical;
mod connection;
mod documents;
mod frontier;
mod integrity;
mod legal;
mod migrations;
mod policy;
mod provenance;
mod purge;
mod search;
mod sources;
#[cfg(test)]
mod tests;
mod timeline;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use duckdb::{params, params_from_iter, Connection, OptionalExt, Result, Row, ToSql, Transaction};
use tokio::sync::Mutex;

use crate::analysis::{
    analyze_redaction_density, extract_enforcement_keyword_counts, ClusterAlertState,
    SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS,
};
use crate::blocking::spawn_with;
use crate::error::AppError;
use crate::ingestion::client::normalize_submitted_url;
use crate::integrity::{constant_time_equal, sha256_file, sha256_hex};
use crate::normalization::collapse_whitespace;
use crate::parsers::{policy_tokens, token_sequence_match_count, ParsedLegalAuthority};
use crate::provenance::{classify_confidence, read_asset_provenance, AssetProvenance};
use crate::storage::fts::{clean_plain_text_for_index, core_query_tokens, token_frequencies};
use crate::storage::vector::VectorSourceDocument;
use crate::types::c2pa::{ProvenanceConfidence, RecordedDigestSource, SignatureVerificationState};
use crate::types::ProvenanceVerificationRecord;
use crate::types::{
    AnalyticalColumnSummary, AnalyticalDatasetKind, AnalyticalDatasetSummary,
    AnalyticalImportRequest, AnalyticalImportResult, AnalyticalSchema, AuxiliaryEventInput,
    AuxiliaryImportResult, AuxiliaryStreamKind, CrawlPolicy, CrossAgencyCorrelation,
    DocumentMetadata, DocumentProvenanceVerification, DocumentPurge, DocumentTextChunk,
    LegalAuthorityImport, LegalAuthorityImportResult, LegalAuthorityKind, LegalAuthoritySummary,
    LegalAuthorityVersionDetail, LegalAuthorityVersionSummary, LegalBriefExportResult,
    LegalComplianceFinding, LegalEvaluationResult, LegalFindingReviewStatus, MetadataPageWindow,
    PortalRegistration, PublicRequest, RequestDocumentSummary, RequestSearchResult, SavedSource,
    SourceGuidance, SourceLifecycleState, SourcePurge, SourcePurgePreview, SqlExportResult,
    SqlQueryResult, SqlTemplate, StatutoryExtensionReason, StatutoryExtensionRecord,
    TimelineAnomalyMarker,
};

pub(crate) use analytical::{analytical_query_provenance, ANALYTICAL_PARSER_VERSION};
pub(crate) use search::request_search_filters;

/// Everything a saved source remembers about how to synchronize it.
///
/// `metadata_page_window` is `Option` because absence is a fact here: a recipe row written before
/// the column existed, or by the crawl path which has no page window, recorded no operator choice.
/// The built-in window then applies, and that is not the same statement as an operator having
/// picked page 1 at size 50.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceConnectorRecipe {
    pub crawl_policy: CrawlPolicy,
    pub guidance: SourceGuidance,
    pub metadata_page_window: Option<MetadataPageWindow>,
}

impl SourceConnectorRecipe {
    /// Whether this row actually carries a saved connector recipe, as opposed to existing only to
    /// hold a page window. `SourceIngestionStart::used_saved_guidance` is a claim to the operator
    /// that saved guidance was applied, so it must be derived from the guidance, not from the
    /// presence of a row.
    pub fn carries_guidance(&self) -> bool {
        self.guidance != SourceGuidance::default() || self.crawl_policy != CrawlPolicy::default()
    }
}

/// The only SQL spelling of "now" this repository may write, pinned by
/// `tests::every_sql_timestamp_is_written_as_parseable_utc`.
///
/// Two mistakes it exists to keep out. `current_timestamp` is bound as a *column reference* inside
/// `ON CONFLICT ... DO UPDATE SET`, where DuckDB then refuses to bind the whole statement; `now()`
/// is a function call in every position. And `%f` is microseconds, not fractional seconds, so the
/// former `'%H:%M:%fZ'` dropped seconds entirely and wrote microseconds into the seconds field,
/// producing a string `TRY_CAST(... AS TIMESTAMPTZ)` returns NULL for and `parse_from_rfc3339`
/// rejects. The session zone is set to UTC in [`DuckDbRepository::new`], so the literal `Z` is true.
#[cfg(test)]
const SQL_UTC_NOW: &str = "strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')";

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
    original_source_uri: Option<String>,
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
    /// Documents still missing from the token projection. Counted during migration and drained by
    /// the background backfill, so a search can refuse rather than under-report while it runs.
    search_index_pending: Arc<AtomicU64>,
    /// Set when the background drain stopped on an error. The arrears then stay where they are,
    /// and "the index is being built" would be a false statement about why a search cannot answer.
    search_index_stalled: Arc<AtomicBool>,
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
