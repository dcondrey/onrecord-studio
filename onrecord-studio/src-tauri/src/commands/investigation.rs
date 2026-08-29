use crate::analysis::AnalyticsEngine;
use crate::blocking::spawn as spawn_blocking;
use crate::commands::validation::{
    managed_evidence_file, optional_portal_key, optional_text, portal_key as validate_portal_key,
    readable_evidence_file, required_text, validate_text,
};
use crate::error::AppError;
use crate::integrity::sha256_hex;
use crate::parsers::{
    audit_document_path, parse_legal_authority, recovery, unredactor, EvidenceBundle,
    ForensicAuditOptions,
};
use crate::storage::duckdb::DuckDbWorkspace;
use crate::storage::sqlite::{DuckDbRepository, ProvenanceEvidenceFile};
use crate::types::{
    AgencyPerformanceProfile, AnalysisConfig, AuxiliaryEventInput, AuxiliaryImportResult,
    AuxiliaryStreamKind, CrossAgencyCorrelation, DocumentProvenanceVerification, DocumentTextChunk,
    LegalAuthorityImport, LegalAuthorityImportResult, LegalAuthoritySummary,
    LegalComplianceFinding, LegalEvaluationResult, LegalFindingReviewStatus,
    ProvenanceVerificationRecord, RequestDocumentSummary, StatutoryExtensionRecord,
};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;
use tokio::sync::Semaphore;

const MAXIMUM_REDACTION_AUDIT_BYTES: u64 = 512 * 1024 * 1024;
const MAXIMUM_VERSION_RECOVERY_BYTES: u64 = 128 * 1024 * 1024;
const MAXIMUM_FORENSIC_AUDIT_BYTES: u64 = 1024 * 1024 * 1024;
const MAXIMUM_AUXILIARY_EVENTS: usize = 50_000;
const MAXIMUM_AUXILIARY_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_VERIFICATION_HISTORY_LIMIT: usize = 50;
const MAXIMUM_VERIFICATION_HISTORY_LIMIT: usize = 500;
const MAXIMUM_CONCURRENT_REDACTION_AUDITS: usize = 1;
const MAXIMUM_CONCURRENT_VERSION_RECOVERIES: usize = 1;
const MAXIMUM_CONCURRENT_FORENSIC_AUDITS: usize = 1;
/// Digest verification streams through a fixed buffer, so it is bounded by disk rather than
/// memory. The limit is higher than the redaction audit's, which materializes its input, but still
/// finite: a request-level "verify everything" would otherwise queue one blocking hash per
/// document against a single disk.
const MAXIMUM_CONCURRENT_PROVENANCE_VERIFICATIONS: usize = 4;

pub struct AppInvestigationPaths {
    pub document_root: PathBuf,
    pub redaction_audit_permits: Arc<Semaphore>,
    pub version_recovery_permits: Arc<Semaphore>,
    pub forensic_audit_permits: Arc<Semaphore>,
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
            forensic_audit_permits: Arc::new(Semaphore::new(MAXIMUM_CONCURRENT_FORENSIC_AUDITS)),
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
    window_hours: Option<usize>,
    correlation_engine: State<'_, DuckDbWorkspace>,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<CrossAgencyCorrelation>, AppError> {
    let portal_key = optional_portal_key(portal_key)?;
    let minimum_complaints = minimum_complaints.unwrap_or(2);
    let result_limit = result_limit.unwrap_or(1_000);
    let window_hours = window_hours.unwrap_or(48);
    let correlations = correlation_engine
        .correlate_311_with_enforcement_requests(
            portal_key.as_deref(),
            minimum_complaints,
            result_limit,
            window_hours,
        )
        .await?;
    repository
        .synchronize_cross_correlation_alerts(&correlations)
        .await?;
    Ok(correlations)
}

#[tauri::command]
pub async fn import_auxiliary_events(
    portal_key: Option<String>,
    source_kind: AuxiliaryStreamKind,
    display_name: String,
    source_uri: Option<String>,
    events: Vec<AuxiliaryEventInput>,
    repository: State<'_, DuckDbRepository>,
) -> Result<AuxiliaryImportResult, AppError> {
    let portal_key = optional_portal_key(portal_key)?;
    let display_name = required_text(&display_name, "Auxiliary source display name", 256)?;
    let source_uri = optional_text(source_uri, "Auxiliary source URI", 2_048)?;
    let source_sha256 = validate_auxiliary_events(&events)?;
    repository
        .import_auxiliary_events(
            portal_key.as_deref(),
            source_kind,
            display_name,
            source_uri.as_deref(),
            &events,
            &source_sha256,
        )
        .await
}

#[tauri::command]
pub async fn register_statutory_extension(
    request_id: u64,
    portal_key: String,
    extension: StatutoryExtensionRecord,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<String>, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    required_text(
        &extension.notice_source,
        "Statutory extension notice source",
        4_096,
    )?;
    repository
        .record_statutory_extension(request_id, portal_key, &extension)
        .await?;
    repository
        .refresh_request_timeline_audit(request_id, portal_key)
        .await
}

/// Returns the extension already recorded for a request, or `None` when none is.
///
/// [`register_statutory_extension`] upserts, so the recording surface has to be able to show what
/// it would replace. A caller that cannot read this cannot tell an unrecorded extension from one
/// it is about to overwrite.
#[tauri::command]
pub async fn get_request_statutory_extension(
    request_id: u64,
    portal_key: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<Option<StatutoryExtensionRecord>, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    repository
        .load_statutory_extension(request_id, portal_key)
        .await
}

#[tauri::command]
pub async fn import_legal_authority(
    input: LegalAuthorityImport,
    repository: State<'_, DuckDbRepository>,
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
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<LegalAuthoritySummary>, AppError> {
    repository.list_legal_authorities().await
}

#[tauri::command]
pub async fn list_legal_authority_versions(
    statute_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<crate::types::LegalAuthorityVersionSummary>, AppError> {
    let statute_id = required_text(&statute_id, "Authority identifier", 128)?;
    repository.list_legal_authority_versions(statute_id).await
}

#[tauri::command]
pub async fn get_legal_authority_version(
    statute_id: String,
    version_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<crate::types::LegalAuthorityVersionDetail, AppError> {
    let statute_id = required_text(&statute_id, "Authority identifier", 128)?;
    let version_id = required_text(&version_id, "Authority version identifier", 256)?;
    repository
        .get_legal_authority_version(statute_id, version_id)
        .await
}

#[tauri::command]
pub async fn evaluate_request_legal_compliance(
    request_id: u64,
    portal_key: String,
    repository: State<'_, DuckDbRepository>,
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
    repository: State<'_, DuckDbRepository>,
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
    repository: State<'_, DuckDbRepository>,
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
    repository: State<'_, DuckDbRepository>,
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
    repository: State<'_, DuckDbRepository>,
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
    repository: State<'_, DuckDbRepository>,
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
    repository: State<'_, DuckDbRepository>,
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
    repository: State<'_, DuckDbRepository>,
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
    repository: State<'_, DuckDbRepository>,
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

#[tauri::command]
pub async fn audit_document_residual_evidence(
    document_id: u64,
    portal_key: String,
    include_hypotheses: Option<bool>,
    repository: State<'_, DuckDbRepository>,
    paths: State<'_, AppInvestigationPaths>,
) -> Result<EvidenceBundle, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    let permit = paths
        .forensic_audit_permits
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal("Forensic audit scheduler is unavailable".to_string()))?;
    let stored_path = repository
        .resolve_public_document_path(document_id, portal_key)
        .await?;
    let path = managed_evidence_file(
        stored_path,
        &paths.document_root,
        MAXIMUM_FORENSIC_AUDIT_BYTES,
        "Residual evidence audit",
    )
    .await?;
    let options = ForensicAuditOptions {
        include_hypotheses: include_hypotheses.unwrap_or(false),
        preserve_raw_values: false,
    };
    spawn_blocking("Residual evidence audit", move || {
        let _permit = permit;
        audit_document_path(path, options)
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
    let created_from = config
        .created_from
        .as_deref()
        .map(|value| {
            chrono::DateTime::parse_from_rfc3339(value).map(|date| date.with_timezone(&chrono::Utc))
        })
        .transpose()
        .map_err(|error| {
            AppError::DataParsing(format!("Analysis start timestamp is invalid: {error}"))
        })?;
    let created_to = config
        .created_to
        .as_deref()
        .map(|value| {
            chrono::DateTime::parse_from_rfc3339(value).map(|date| date.with_timezone(&chrono::Utc))
        })
        .transpose()
        .map_err(|error| {
            AppError::DataParsing(format!("Analysis end timestamp is invalid: {error}"))
        })?;
    if created_from
        .zip(created_to)
        .is_some_and(|(from, to)| from >= to)
    {
        return Err(AppError::DataParsing(
            "Analysis start timestamp must be earlier than the end timestamp".to_string(),
        ));
    }
    Ok(())
}

fn validate_auxiliary_events(events: &[AuxiliaryEventInput]) -> Result<String, AppError> {
    if events.is_empty() || events.len() > MAXIMUM_AUXILIARY_EVENTS {
        return Err(AppError::DataParsing(format!(
            "Auxiliary imports must contain between 1 and {MAXIMUM_AUXILIARY_EVENTS} events"
        )));
    }
    for event in events {
        required_text(&event.external_id, "Auxiliary event identifier", 256)?;
        required_text(&event.occurred_at, "Auxiliary event timestamp", 64)?;
        required_text(
            &event.neighborhood_block,
            "Auxiliary event neighborhood",
            512,
        )?;
        required_text(&event.event_type, "Auxiliary event type", 256)?;
        validate_text(&event.description, "Auxiliary event description", 64 * 1024)?;
    }
    let canonical_payload = serde_json::to_vec(events)
        .map_err(|error| AppError::DataParsing(format!("Auxiliary events are invalid: {error}")))?;
    if canonical_payload.len() > MAXIMUM_AUXILIARY_PAYLOAD_BYTES {
        return Err(AppError::DataParsing(format!(
            "Auxiliary import exceeds the {MAXIMUM_AUXILIARY_PAYLOAD_BYTES}-byte payload limit"
        )));
    }
    Ok(sha256_hex(&canonical_payload))
}
