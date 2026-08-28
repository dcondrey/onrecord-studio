use crate::analysis::AnalyticsEngine;
use crate::blocking::spawn as spawn_blocking;
use crate::commands::validation::{
    managed_evidence_file, optional_portal_key, optional_text, portal_key as validate_portal_key,
    readable_evidence_file, required_text,
};
use crate::error::AppError;
use crate::parsers::{parse_legal_authority, recovery, unredactor};
use crate::storage::duckdb::DuckDbWorkspace;
use crate::storage::sqlite::{ProvenanceEvidenceFile, SqliteRepository};
use crate::types::{
    AgencyPerformanceProfile, AnalysisConfig, CrossAgencyCorrelation,
    DocumentProvenanceVerification, DocumentTextChunk, LegalAuthorityImport,
    LegalAuthorityImportResult, LegalAuthoritySummary, LegalComplianceFinding,
    LegalEvaluationResult, LegalFindingReviewStatus, ProvenanceVerificationRecord,
    RequestDocumentSummary,
};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;
use tokio::sync::Semaphore;

const MAXIMUM_REDACTION_AUDIT_BYTES: u64 = 512 * 1024 * 1024;
const MAXIMUM_VERSION_RECOVERY_BYTES: u64 = 128 * 1024 * 1024;
const DEFAULT_VERIFICATION_HISTORY_LIMIT: usize = 50;
const MAXIMUM_VERIFICATION_HISTORY_LIMIT: usize = 500;
const MAXIMUM_CONCURRENT_REDACTION_AUDITS: usize = 1;
const MAXIMUM_CONCURRENT_VERSION_RECOVERIES: usize = 1;
/// Digest verification streams through a fixed buffer, so it is bounded by disk rather than
/// memory. The limit is higher than the redaction audit's, which materializes its input, but still
/// finite: a request-level "verify everything" would otherwise queue one blocking hash per
/// document against a single disk.
const MAXIMUM_CONCURRENT_PROVENANCE_VERIFICATIONS: usize = 4;

pub struct AppInvestigationPaths {
    pub document_root: PathBuf,
    pub redaction_audit_permits: Arc<Semaphore>,
    pub version_recovery_permits: Arc<Semaphore>,
    pub provenance_verification_permits: Arc<Semaphore>,
}

impl AppInvestigationPaths {
    pub fn new(document_root: PathBuf) -> Self {
        Self {
            document_root,
            redaction_audit_permits: Arc::new(Semaphore::new(MAXIMUM_CONCURRENT_REDACTION_AUDITS)),
            version_recovery_permits: Arc::new(Semaphore::new(
                MAXIMUM_CONCURRENT_VERSION_RECOVERIES,
            )),
            provenance_verification_permits: Arc::new(Semaphore::new(
                MAXIMUM_CONCURRENT_PROVENANCE_VERIFICATIONS,
            )),
        }
    }
}

#[tauri::command]
pub async fn get_agency_performance_profiles(
    config: Option<AnalysisConfig>,
    agency_engine: State<'_, AnalyticsEngine>,
) -> Result<Vec<AgencyPerformanceProfile>, AppError> {
    let config = config.unwrap_or_default();
    validate_analysis_config(&config)?;
    agency_engine
        .calculate_agency_outlier_profiles(&config)
        .await
}

#[tauri::command]
pub async fn get_cross_agency_correlations(
    portal_key: Option<String>,
    minimum_complaints: Option<usize>,
    result_limit: Option<usize>,
    correlation_engine: State<'_, DuckDbWorkspace>,
    repository: State<'_, SqliteRepository>,
) -> Result<Vec<CrossAgencyCorrelation>, AppError> {
    let portal_key = optional_portal_key(portal_key)?;
    let minimum_complaints = minimum_complaints.unwrap_or(2);
    let result_limit = result_limit.unwrap_or(1_000);
    let correlations = correlation_engine
        .correlate_311_with_enforcement_requests(
            portal_key.as_deref(),
            minimum_complaints,
            result_limit,
        )
        .await?;
    repository
        .synchronize_cross_correlation_alerts(&correlations)
        .await?;
    Ok(correlations)
}

#[tauri::command]
pub async fn import_legal_authority(
    input: LegalAuthorityImport,
    repository: State<'_, SqliteRepository>,
) -> Result<LegalAuthorityImportResult, AppError> {
    let parse_input = input.clone();
    let parsed = spawn_blocking("Legal authority parsing", move || {
        parse_legal_authority(&parse_input)
    })
    .await??;
    repository.import_legal_authority(&input, &parsed).await
}

#[tauri::command]
pub async fn list_legal_authorities(
    repository: State<'_, SqliteRepository>,
) -> Result<Vec<LegalAuthoritySummary>, AppError> {
    repository.list_legal_authorities().await
}

#[tauri::command]
pub async fn evaluate_request_legal_compliance(
    request_id: u64,
    portal_key: String,
    repository: State<'_, SqliteRepository>,
) -> Result<LegalEvaluationResult, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    repository
        .refresh_request_timeline_audit(request_id, portal_key)
        .await?;
    repository
        .evaluate_request_legal_compliance(request_id, portal_key)
        .await
}

#[tauri::command]
pub async fn list_request_legal_findings(
    request_id: u64,
    portal_key: String,
    repository: State<'_, SqliteRepository>,
) -> Result<Vec<LegalComplianceFinding>, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    repository
        .list_request_legal_findings(request_id, portal_key)
        .await
}

#[tauri::command]
pub async fn review_legal_finding(
    violation_id: String,
    request_id: u64,
    portal_key: String,
    review_status: LegalFindingReviewStatus,
    reviewer_notes: Option<String>,
    repository: State<'_, SqliteRepository>,
) -> Result<Vec<LegalComplianceFinding>, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    let violation_id = required_text(&violation_id, "Legal finding identifier", 256)?;
    if !violation_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(AppError::DataParsing(
            "Legal finding identifier contains unsupported characters".to_string(),
        ));
    }
    let reviewer_notes = optional_text(reviewer_notes, "Legal finding reviewer notes", 8_192)?;
    repository
        .review_legal_finding(
            violation_id,
            request_id,
            portal_key,
            review_status,
            reviewer_notes.as_deref(),
        )
        .await
}

/// Verifies a stored document against the digest recorded for it.
///
/// The evidence path is resolved and gated here rather than inside the repository, so that this
/// command reads evidence through the same managed-store containment every other evidence-reading
/// command uses. A path that fails the gate does not fail the command: it becomes an
/// `EVIDENCE_FILE_UNAVAILABLE` verification carrying the reason, which is recorded in the
/// append-only log like any other outcome.
#[tauri::command]
pub async fn verify_document_provenance(
    document_id: u64,
    portal_key: String,
    repository: State<'_, SqliteRepository>,
    paths: State<'_, AppInvestigationPaths>,
) -> Result<DocumentProvenanceVerification, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    let _permit = paths
        .provenance_verification_permits
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| {
            AppError::Internal("Provenance verification scheduler is unavailable".to_string())
        })?;
    let evidence = match repository
        .resolve_document_evidence_path(document_id, portal_key)
        .await?
    {
        Some(stored_path) => {
            match readable_evidence_file(&stored_path, &paths.document_root).await {
                Ok(path) => ProvenanceEvidenceFile::Readable(path),
                Err(reason) => ProvenanceEvidenceFile::Unavailable(reason),
            }
        }
        None => ProvenanceEvidenceFile::Unavailable(
            "No local evidence file is recorded for this document".to_string(),
        ),
    };
    repository
        .verify_document_provenance(document_id, portal_key, evidence)
        .await
}

/// Reads a document's provenance verification history, newest first.
///
/// The log is append-only and is what shows a document was checked, when, and what the check
/// found. Without this it was written and never read, so the history an investigator would cite in
/// a filing was unreachable from the application that produced it.
#[tauri::command]
pub async fn list_document_provenance_verifications(
    document_id: u64,
    portal_key: String,
    limit: Option<usize>,
    repository: State<'_, SqliteRepository>,
) -> Result<Vec<ProvenanceVerificationRecord>, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    let limit = limit.unwrap_or(DEFAULT_VERIFICATION_HISTORY_LIMIT);
    if !(1..=MAXIMUM_VERIFICATION_HISTORY_LIMIT).contains(&limit) {
        return Err(AppError::DataParsing(format!(
            "Verification history limit must be between 1 and {MAXIMUM_VERIFICATION_HISTORY_LIMIT}"
        )));
    }
    repository
        .list_document_provenance_verifications(document_id, portal_key, limit)
        .await
}

#[tauri::command]
pub async fn list_request_documents(
    request_id: u64,
    portal_key: String,
    repository: State<'_, SqliteRepository>,
) -> Result<Vec<RequestDocumentSummary>, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    repository
        .list_request_documents_for_view(request_id, portal_key)
        .await
}

#[tauri::command]
pub async fn get_document_text_chunk(
    document_id: u64,
    portal_key: String,
    character_offset: Option<usize>,
    character_limit: Option<usize>,
    repository: State<'_, SqliteRepository>,
) -> Result<DocumentTextChunk, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    let character_limit = character_limit.unwrap_or(100_000);
    if !(1..=100_000).contains(&character_limit) {
        return Err(AppError::DataParsing(
            "Document chunk size must be between 1 and 100000 characters".to_string(),
        ));
    }
    repository
        .load_document_text_chunk(
            document_id,
            portal_key,
            character_offset.unwrap_or(0),
            character_limit,
        )
        .await
}

#[tauri::command]
pub async fn verify_document_redaction_integrity(
    document_id: u64,
    portal_key: String,
    repository: State<'_, SqliteRepository>,
    paths: State<'_, AppInvestigationPaths>,
) -> Result<unredactor::UnredactorForensicReport, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    let permit = paths
        .redaction_audit_permits
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal("Redaction audit scheduler is unavailable".to_string()))?;
    let stored_path = repository
        .resolve_public_document_path(document_id, portal_key)
        .await?;
    let path = managed_evidence_file(
        stored_path,
        &paths.document_root,
        MAXIMUM_REDACTION_AUDIT_BYTES,
        "Redaction audit",
    )
    .await?;
    spawn_blocking("Redaction audit", move || {
        let _permit = permit;
        unredactor::evaluate_vector_redaction_leaks(path)
    })
    .await?
}

#[tauri::command]
pub async fn carve_document_version_history(
    document_id: u64,
    portal_key: String,
    repository: State<'_, SqliteRepository>,
    paths: State<'_, AppInvestigationPaths>,
) -> Result<recovery::RecoveredVersionProfile, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    let permit = paths
        .version_recovery_permits
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal("Version recovery scheduler is unavailable".to_string()))?;
    let stored_path = repository
        .resolve_public_document_path(document_id, portal_key)
        .await?;
    let path = managed_evidence_file(
        stored_path,
        &paths.document_root,
        MAXIMUM_VERSION_RECOVERY_BYTES,
        "Version recovery",
    )
    .await?;
    spawn_blocking("Version recovery", move || {
        let _permit = permit;
        recovery::carve_historical_pdf_versions(path)
    })
    .await?
}

fn validate_analysis_config(config: &AnalysisConfig) -> Result<(), AppError> {
    if !config.latency_threshold_days.is_finite()
        || !(0.0..=36_500.0).contains(&config.latency_threshold_days)
    {
        return Err(AppError::DataParsing(
            "Latency threshold must be finite and between 0 and 36500 days".to_string(),
        ));
    }
    if config.min_ocr_character_density > 1_000_000 {
        return Err(AppError::DataParsing(
            "Minimum OCR character density exceeds the supported limit".to_string(),
        ));
    }
    if !(0..=43_200).contains(&config.suspicious_closure_window_minutes) {
        return Err(AppError::DataParsing(
            "Suspicious closure window must be between 0 and 43200 minutes".to_string(),
        ));
    }
    if config.target_policy_keywords.len() > 256 {
        return Err(AppError::DataParsing(
            "Analysis configuration exceeds 256 policy keywords".to_string(),
        ));
    }
    for keyword in &config.target_policy_keywords {
        required_text(keyword, "Policy keyword", 128)?;
    }
    Ok(())
}
