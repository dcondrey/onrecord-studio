use crate::analysis::StatutoryComplianceAnalysis;
use crate::commands::validation::{
    optional_text, portal_key as validate_portal_key, MAXIMUM_PATH_BYTES,
};
use crate::error::AppError;
use crate::integrity::{
    check_revocation, is_sha256_hex, load_or_create_signing_key, load_revocations,
    rotate_signing_key, sha256_hex, sign_package, verify_package, verify_package_detailed,
    SignedPackage, SIGNATURE_ALGORITHM,
};
use crate::storage::casework::{EXPORT_OUTCOME_FAILED, EXPORT_OUTCOME_WRITTEN};
use crate::storage::pii::mask_pii;
use crate::storage::sqlite::{DuckDbRepository, EvidenceDocument, LegalBriefEvidence};
use crate::types::casework::{RequestFilingRecordResult, WithheldCitationRequest};
use crate::types::{
    CaseEvidencePackageResult, LegalBriefExportResult, LegalComplianceFinding,
    PackageVerificationResult, SigningKeyRevocations, SigningKeyRotationResult,
    TimelineAnomalyMarker,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tauri::State;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

const REPORT_TEMPLATE: &str = include_str!("../../../templates/report_template.md");
/// Embedded so an installed app can produce a package's verifier alongside it. The UI must never
/// tell a reader to run a path that only exists in this repository's working tree — a court or an
/// opposing party who receives an exported package has no checkout to run it from.
const VERIFIER_SCRIPT: &str = include_str!("../../../scripts/verify-evidence-package.mjs");
const VERIFIER_SCRIPT_FILE: &str = "verify-evidence-package.mjs";
/// The same checks as the script, for a recipient who has a browser but not Node. Self-contained
/// and network-free by construction (its own CSP forbids any fetch), so it can be opened from a
/// file:// path or handed over on media with the package.
const VERIFIER_PAGE: &str = include_str!("../../../scripts/verify-evidence-package.html");
const VERIFIER_PAGE_FILE: &str = "verify-evidence-package.html";
const MAXIMUM_REPORT_BYTES: usize = 32 * 1024 * 1024;
const MAXIMUM_EVIDENCE_PACKAGE_BYTES: usize = 64 * 1024 * 1024;
const MAXIMUM_CASE_EVIDENCE_PACKAGE_BYTES: usize = 8 * 1024 * 1024;
/// The larger of the two signed package types' own export limits, since a package handed back in
/// for verification may be either kind.
const MAXIMUM_VERIFIABLE_PACKAGE_BYTES: usize = MAXIMUM_EVIDENCE_PACKAGE_BYTES;
const EVIDENCE_PACKAGE_SCHEMA: &str = "onrecord-evidence-package-v3";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct EvidencePackageManifest {
    schema_version: String,
    generated_at: String,
    request_id: u64,
    portal_key: String,
    report: PackagedReport,
    source_documents: Vec<PackagedSourceDocument>,
    legal_authorities: Vec<PackagedLegalAuthority>,
    legal_findings: Vec<PackagedLegalFinding>,
    violation_markers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PackagedReport {
    file_name: String,
    media_type: String,
    sha256: String,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PackagedSourceDocument {
    document_id: u64,
    file_size: Option<u64>,
    visibility: String,
    raw_file_sha256: Option<String>,
    extracted_text_sha256: Option<String>,
}

/// A timestamp copied out of storage into a signed package.
///
/// Builds before 929af7a wrote `evaluated_at` and `imported_at` through a `strftime` format whose
/// `%f` meant microseconds, so the stored strings carry no seconds field at all
/// (`2026-08-29T14:14:190749Z`). The seconds are gone and no backfill can recover them. Copying
/// such a string into the instant position of a signed package states a time the record never
/// established, which is the same false claim about evidence that `RecordedDigestSource::None` and
/// the nullable `documents.shannon_entropy` exist to prevent: absence and measurement do not share
/// a representation. Refusing the export instead would withhold a whole legitimate brief over a
/// legacy row, so the package says which of the two it holds and preserves the recorded text
/// either way.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", tag = "kind")]
enum RecordedInstant {
    /// The stored value parsed as an RFC 3339 instant. Carried verbatim, not reformatted, so the
    /// package reproduces what storage holds.
    Instant { value: String },
    /// The stored value is not an instant. `recorded` is what the row actually carries.
    Unparseable { recorded: String },
}

impl RecordedInstant {
    fn from_stored(value: String) -> Self {
        if chrono::DateTime::parse_from_rfc3339(&value).is_ok() {
            Self::Instant { value }
        } else {
            Self::Unparseable { recorded: value }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PackagedLegalAuthority {
    statute_id: String,
    version_id: String,
    citation: String,
    title: String,
    content_sha256: String,
    effective_from: Option<String>,
    effective_to: Option<String>,
    source_uri: Option<String>,
    imported_at: RecordedInstant,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PackagedLegalFinding {
    violation_id: String,
    statute_id: String,
    version_id: String,
    classification: String,
    confidence_basis_points: u16,
    supporting_document_ids: Vec<u64>,
    review_status: String,
    evaluated_at: RecordedInstant,
}

const TEMPLATE_VARIABLES: [&str; 25] = [
    "GENERATED_AT",
    "AGENCY_NAME",
    "PORTAL_NAME",
    "PORTAL_URL",
    "REQUEST_REFERENCE",
    "REQUEST_TITLE",
    "REQUEST_ID",
    "PORTAL_KEY",
    "RPID",
    "STATUS",
    "CREATED_AT",
    "UPDATED_AT",
    "CLOSED_AT",
    "INITIAL_DEADLINE_AT",
    "EFFECTIVE_DEADLINE_AT",
    "DETERMINATION_AT",
    "DETERMINATION_SOURCE",
    "CALENDAR_DAYS_ELAPSED",
    "EXTENSION_STATUS",
    "EXTENSION_REASON",
    "EXTENSION_SOURCE",
    "VIOLATION_FINDINGS",
    "REQUEST_DESCRIPTION",
    "DOCUMENT_TABLE",
    "AUTHOR_NAME",
];

#[tauri::command]
pub async fn export_legal_brief(
    request_id: u64,
    portal_key: String,
    target_path: Option<String>,
    author_name: Option<String>,
    repository: State<'_, DuckDbRepository>,
    report_directory: State<'_, PathBuf>,
    key_directory: State<'_, crate::integrity::SigningKeyDirectory>,
) -> Result<LegalBriefExportResult, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    let author_name = optional_text(author_name, "Report author name", 256)?;
    let target_path = optional_text(target_path, "Report output path", MAXIMUM_PATH_BYTES)?;
    let evidence = repository
        .load_legal_brief_evidence(request_id, portal_key)
        .await?;
    let compliance = crate::analysis::evaluate_statutory_compliance_with_determination_at(
        request_id,
        &evidence.created_at,
        Some(&evidence.updated_at),
        evidence.closed_at.as_deref(),
        evidence.determination_at.as_deref(),
        evidence.determination_source.as_deref(),
        evidence.extension.as_ref(),
        &chrono::Utc::now().to_rfc3339(),
    )?;
    let legal_findings = repository
        .list_request_legal_findings(request_id, portal_key)
        .await?;
    let legal_authorities = load_referenced_legal_authorities(&repository, &legal_findings).await?;
    let markers = reportable_markers(&evidence, &compliance);
    if markers.is_empty() {
        return Err(AppError::Export(format!(
            "Request {}/{request_id} has no statutory-latency or fast-closure finding",
            evidence.portal_key
        )));
    }
    let generated_at = chrono::Utc::now().to_rfc3339();
    let report = render_report(
        &evidence,
        &compliance,
        &markers,
        &legal_findings,
        author_name
            .as_deref()
            .unwrap_or("Investigative Records Reviewer"),
        &generated_at,
    )?;
    let report_sha256 = sha256_hex(report.as_bytes());
    let target_path = target_path.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        report_directory
            .join(format!(
                "{}-request-{}-{timestamp}.md",
                evidence.portal_key, evidence.request_id
            ))
            .to_string_lossy()
            .into_owned()
    });
    let output_path = validate_report_path(&target_path, &report_directory, "md").await?;
    let package_path = output_path.with_extension("evidence.json");
    if tokio::fs::try_exists(&package_path)
        .await
        .map_err(|error| AppError::Export(format!("Failed to inspect package path: {error}")))?
    {
        return Err(AppError::Export(format!(
            "Evidence package path '{}' already exists; evidence exports are never overwritten",
            package_path.display()
        )));
    }
    let signing_key = load_or_create_signing_key(&key_directory.0).await?;
    let verifiers = ensure_verifier_script(&report_directory).await?;
    let manifest = validate_evidence_manifest(EvidencePackageManifest {
        schema_version: EVIDENCE_PACKAGE_SCHEMA.to_string(),
        generated_at: generated_at.clone(),
        request_id,
        portal_key: evidence.portal_key.clone(),
        report: PackagedReport {
            file_name: output_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("report.md")
                .to_string(),
            media_type: "text/markdown; charset=utf-8".to_string(),
            sha256: report_sha256.clone(),
            content: report.clone(),
        },
        source_documents: evidence
            .documents
            .iter()
            .map(|document| PackagedSourceDocument {
                document_id: document.document_id,
                file_size: document.file_size,
                visibility: document.visibility.clone(),
                raw_file_sha256: document.raw_file_sha256.clone(),
                extracted_text_sha256: document.extracted_text_sha256.clone(),
            })
            .collect(),
        legal_authorities,
        legal_findings: legal_findings
            .iter()
            .filter(|finding| matches!(finding.review_status.as_str(), "OPEN" | "CONFIRMED"))
            .map(|finding| PackagedLegalFinding {
                violation_id: finding.violation_id.clone(),
                statute_id: finding.matched_statute_id.clone(),
                version_id: finding.matched_version_id.clone(),
                classification: finding.classification.clone(),
                confidence_basis_points: finding.confidence_basis_points,
                supporting_document_ids: finding.supporting_document_ids.clone(),
                review_status: finding.review_status.clone(),
                evaluated_at: RecordedInstant::from_stored(finding.evaluated_at.clone()),
            })
            .collect(),
        violation_markers: markers.clone(),
    })?;
    let package = sign_package(manifest, &signing_key).map_err(AppError::Export)?;
    // Never ship a package this installation cannot itself re-verify: catches a signing defect
    // before it reaches a reader who trusts the signature, not after.
    verify_package(&package).map_err(|error| {
        AppError::Export(format!(
            "Freshly signed evidence package failed self-verification: {error}"
        ))
    })?;
    let package_bytes = serde_json::to_vec_pretty(&package)
        .map_err(|error| AppError::Export(format!("Failed to encode evidence package: {error}")))?;
    if package_bytes.len() > MAXIMUM_EVIDENCE_PACKAGE_BYTES {
        return Err(AppError::Export(format!(
            "Evidence package exceeds the {MAXIMUM_EVIDENCE_PACKAGE_BYTES}-byte limit"
        )));
    }
    let package_sha256 = sha256_hex(&package_bytes);
    commit_new_report(&output_path, report.as_bytes()).await?;
    if let Err(error) = commit_new_report(&package_path, &package_bytes).await {
        return match tokio::fs::remove_file(&output_path).await {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(AppError::Export(format!(
                "{error}; the committed report could not be rolled back: {cleanup_error}"
            ))),
        };
    }
    let result = LegalBriefExportResult {
        request_id,
        portal_key: evidence.portal_key,
        output_path: output_path.to_string_lossy().into_owned(),
        report_sha256,
        package_path: package_path.to_string_lossy().into_owned(),
        package_sha256,
        signature_algorithm: SIGNATURE_ALGORITHM.to_string(),
        signer_key_id: package.signature.signer_key_id,
        generated_at,
        included_violation_markers: markers,
        verifier_script_path: verifiers.script_path.to_string_lossy().into_owned(),
        verifier_page_path: verifiers.page_path.to_string_lossy().into_owned(),
    };
    if let Err(error) = repository.record_report_export(&result).await {
        let report_cleanup = tokio::fs::remove_file(&output_path).await.err();
        let package_cleanup = tokio::fs::remove_file(&package_path).await.err();
        return match (report_cleanup, package_cleanup) {
            (None, None) => Err(error),
            (report_cleanup, package_cleanup) => Err(AppError::Export(format!(
                "Failed to record report provenance ({error}); rollback failures: report={report_cleanup:?}, package={package_cleanup:?}"
            ))),
        };
    }
    Ok(result)
}

/// Exports a case as a signed chain-of-custody package: the full case (fresh-read, so every
/// citation's verification reflects current storage) plus every cited source document's hash,
/// covered by the same Ed25519 signing key `export_legal_brief` uses. The signature proves the
/// exported bytes have not changed since this installation produced them; it does not and cannot
/// prove the underlying facts, so an `UNVERIFIED_PORTABLE` citation stays visibly unverified
/// inside the package rather than being smoothed over by the presence of a signature.
///
/// Independent verification does not require this application: `scripts/verify-evidence-package.mjs`
/// checks the signature and hashes from the exported file alone.
#[tauri::command]
pub async fn export_case_evidence_package(
    case_id: String,
    target_path: Option<String>,
    withheld_citations: Vec<WithheldCitationRequest>,
    repository: State<'_, DuckDbRepository>,
    report_directory: State<'_, PathBuf>,
    key_directory: State<'_, crate::integrity::SigningKeyDirectory>,
) -> Result<CaseEvidencePackageResult, AppError> {
    let target_path = optional_text(target_path, "Case export output path", MAXIMUM_PATH_BYTES)?;
    let target_path = target_path.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        report_directory
            .join(format!("case-{case_id}-custody-{timestamp}.json"))
            .to_string_lossy()
            .into_owned()
    });
    // Resolved and checked for a pre-existing file *before* the export is signed and the ledger
    // sequence reserved, so a bad output path (the overwhelmingly common failure) never burns a
    // position in the case's append-only export chain.
    let output_path = validate_report_path(&target_path, &report_directory, "json").await?;
    let signing_key = load_or_create_signing_key(&key_directory.0).await?;
    let verifiers = ensure_verifier_script(&report_directory).await?;
    let generated_at = chrono::Utc::now().to_rfc3339();
    let package = repository
        .export_case_evidence_package_signed(
            case_id.clone(),
            generated_at.clone(),
            withheld_citations,
            signing_key,
        )
        .await?;
    let sequence = package.manifest.ledger_sequence;
    let package_bytes = serde_json::to_vec_pretty(&package).map_err(|error| {
        AppError::Export(format!(
            "Failed to encode chain-of-custody package: {error}"
        ))
    })?;
    if package_bytes.len() > MAXIMUM_CASE_EVIDENCE_PACKAGE_BYTES {
        let _ = repository
            .record_case_export_outcome(
                case_id,
                sequence,
                EXPORT_OUTCOME_FAILED,
                Some(format!(
                    "Encoded package of {} bytes exceeds the {MAXIMUM_CASE_EVIDENCE_PACKAGE_BYTES}-byte limit.",
                    package_bytes.len()
                )),
            )
            .await;
        return Err(AppError::Export(format!(
            "Chain-of-custody export exceeds the {MAXIMUM_CASE_EVIDENCE_PACKAGE_BYTES}-byte limit"
        )));
    }
    let package_sha256 = sha256_hex(&package_bytes);
    if let Err(error) = commit_new_report(&output_path, &package_bytes).await {
        // The ledger reservation for `sequence` now stands with no confirmed outcome; a chain
        // reader must see it as "reserved, not written," never silently as delivered or as a gap
        // that looks like a withheld export. Recording the failure is best-effort: if it too fails,
        // `case_export_chain` already reports an outcome-less reservation the same honest way.
        let _ = repository
            .record_case_export_outcome(
                case_id,
                sequence,
                EXPORT_OUTCOME_FAILED,
                Some(error.to_string()),
            )
            .await;
        return Err(error);
    }
    // The package is already on disk and chain-linked, so a failed outcome record must not read
    // as a failed export: retrying would write a second package while this one stays delivered.
    repository
        .record_case_export_outcome(
            case_id.clone(),
            sequence,
            EXPORT_OUTCOME_WRITTEN,
            Some(output_path.to_string_lossy().into_owned()),
        )
        .await
        .map_err(|error| {
            AppError::Export(format!(
                "The evidence package was written to '{}', but recording that it was written \
                 failed, so the export ledger still shows it as reserved: {error}",
                output_path.display()
            ))
        })?;
    Ok(CaseEvidencePackageResult {
        case_id,
        case_revision: package.manifest.case.revision,
        output_path: output_path.to_string_lossy().into_owned(),
        package_sha256,
        signature_algorithm: SIGNATURE_ALGORITHM.to_string(),
        signer_key_id: package.signature.signer_key_id,
        generated_at,
        verified_citation_count: package.manifest.verified_citation_count,
        unverified_citation_labels: package.manifest.unverified_citation_labels,
        withheld_citation_labels: package
            .manifest
            .withheld_citations
            .iter()
            .map(|w| format!("{} ({})", w.label, w.reason))
            .collect(),
        ledger_sequence: sequence,
        previous_export_sha256: package.manifest.previous_export_sha256,
        verifier_script_path: verifiers.script_path.to_string_lossy().into_owned(),
        verifier_page_path: verifiers.page_path.to_string_lossy().into_owned(),
    })
}

/// Signs and writes a filing record for one records request, through the same package path the
/// legal brief and chain-of-custody exports use.
///
/// The operator files the request through their own channel and records the confirmed send date on
/// the request; this writes what was filed, what it covers, and the modeled determination clock
/// derived from that date. No transport exists here and none is implied: the signature proves the
/// bytes have not changed since this installation produced them, nothing about who filed or
/// whether a send occurred.
#[tauri::command]
pub async fn export_request_filing_record(
    case_id: String,
    request_id: String,
    target_path: Option<String>,
    repository: State<'_, DuckDbRepository>,
    report_directory: State<'_, PathBuf>,
    key_directory: State<'_, crate::integrity::SigningKeyDirectory>,
) -> Result<RequestFilingRecordResult, AppError> {
    let target_path = optional_text(target_path, "Filing record output path", MAXIMUM_PATH_BYTES)?;
    let target_path = target_path.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        report_directory
            .join(format!(
                "case-{case_id}-filing-{request_id}-{timestamp}.json"
            ))
            .to_string_lossy()
            .into_owned()
    });
    let output_path = validate_report_path(&target_path, &report_directory, "json").await?;
    let signing_key = load_or_create_signing_key(&key_directory.0).await?;
    let verifiers = ensure_verifier_script(&report_directory).await?;
    let generated_at = chrono::Utc::now().to_rfc3339();
    let package = repository
        .sign_request_filing_record(
            case_id.clone(),
            request_id.clone(),
            generated_at.clone(),
            signing_key,
        )
        .await?;
    let package_bytes = serde_json::to_vec_pretty(&package)
        .map_err(|error| AppError::Export(format!("Failed to encode filing record: {error}")))?;
    if package_bytes.len() > MAXIMUM_CASE_EVIDENCE_PACKAGE_BYTES {
        return Err(AppError::Export(format!(
            "Filing record exceeds the {MAXIMUM_CASE_EVIDENCE_PACKAGE_BYTES}-byte limit"
        )));
    }
    let package_sha256 = sha256_hex(&package_bytes);
    commit_new_report(&output_path, &package_bytes).await?;
    Ok(RequestFilingRecordResult {
        case_id,
        request_id,
        output_path: output_path.to_string_lossy().into_owned(),
        package_sha256,
        signature_algorithm: SIGNATURE_ALGORITHM.to_string(),
        signer_key_id: package.signature.signer_key_id,
        generated_at,
        determination_due_on: package.manifest.filing_plan.determination_due_on,
        remaining_omissions: package.manifest.remaining_omissions,
        verifier_script_path: verifiers.script_path.to_string_lossy().into_owned(),
        verifier_page_path: verifiers.page_path.to_string_lossy().into_owned(),
    })
}

/// Reads back this case's local export ledger for the "verify this chain" surface in the UI. The
/// ledger is this installation's own unsigned record, so its continuity says nothing about the
/// exported files; passing every exported package of the case to
/// `scripts/verify-evidence-package.mjs` checks the chain against the files themselves.
#[tauri::command]
pub async fn case_export_chain(
    case_id: String,
    repository: State<'_, DuckDbRepository>,
) -> Result<Vec<crate::types::casework::CaseExportLedgerEntry>, AppError> {
    repository.case_export_chain(case_id).await
}

/// Retires this installation's current export signing key and starts a new one. Every package
/// exported before this call keeps verifying exactly as it did (the signature math over its own
/// bytes never changes); what changes is that a *future* verification of one of those older
/// packages will now also see the old key listed in `get_signing_key_revocations` and flag it,
/// per `reason`. The old key file is archived, never deleted — see `rotate_signing_key`.
#[tauri::command]
pub async fn rotate_export_signing_key(
    reason: String,
    compromised: bool,
    report_directory: State<'_, PathBuf>,
    key_directory: State<'_, crate::integrity::SigningKeyDirectory>,
) -> Result<SigningKeyRotationResult, AppError> {
    rotate_signing_key(&report_directory, &key_directory.0, reason, compromised).await
}

/// Escalates a key already listed as retired (routine rotation) to compromised, without rotating
/// the currently active key — the ordinary path when a compromise is discovered after the fact,
/// which is not the same moment as when the key happened to be rotated.
#[tauri::command]
pub async fn mark_signing_key_compromised(
    key_id: String,
    reason: String,
    report_directory: State<'_, PathBuf>,
    key_directory: State<'_, crate::integrity::SigningKeyDirectory>,
) -> Result<(), AppError> {
    crate::integrity::mark_key_compromised(&report_directory, &key_directory.0, key_id, reason)
        .await
}

/// This installation's local record of retired signing keys, for display in the UI. The
/// distributable, independently verifiable form of the same data is the signed file at
/// `report_directory`/`SIGNING_KEY_REVOCATIONS_FILE`, which a recipient can pass to
/// `scripts/verify-evidence-package.mjs --revocations` without running this application.
#[tauri::command]
pub async fn get_signing_key_revocations(
    report_directory: State<'_, PathBuf>,
) -> Result<SigningKeyRevocations, AppError> {
    load_revocations(&report_directory).await
}

/// Verifies a signed evidence or chain-of-custody package's own text content, exactly as
/// `scripts/verify-evidence-package.mjs` does, so a recipient never needs Node installed just to
/// check what they were given. Takes the raw file text the way `preview_casework_import` does,
/// not a path, so the browser file input's `.text()` result works directly.
#[tauri::command]
pub async fn verify_evidence_package(
    payload: String,
    report_directory: State<'_, PathBuf>,
) -> Result<PackageVerificationResult, AppError> {
    verify_evidence_package_inner(payload, &report_directory).await
}

/// Split from the `#[tauri::command]` wrapper so tests can call it with a plain `&Path` — Tauri's
/// `State` has no public test constructor in the version this crate uses.
async fn verify_evidence_package_inner(
    payload: String,
    report_directory: &Path,
) -> Result<PackageVerificationResult, AppError> {
    if payload.len() > MAXIMUM_VERIFIABLE_PACKAGE_BYTES {
        return Err(AppError::Export(format!(
            "Package exceeds the {MAXIMUM_VERIFIABLE_PACKAGE_BYTES}-byte limit."
        )));
    }
    let package: SignedPackage<serde_json::Value> = match serde_json::from_str(&payload) {
        Ok(package) => package,
        Err(error) => {
            return Ok(PackageVerificationResult {
                schema_version: None,
                canonical_bytes_hash_matches: false,
                signature_valid: false,
                manifest_matches_canonical_bytes: false,
                overall_passed: false,
                identity_assurance: None,
                unverified_citation_labels: Vec::new(),
                withheld_citation_labels: Vec::new(),
                ledger_sequence: None,
                previous_export_sha256: None,
                revoked_signer: None,
                parse_error: Some(format!(
                    "Not a signed package this application produces: {error}"
                )),
            })
        }
    };
    let checks = verify_package_detailed(&package);
    let schema_version = package
        .manifest
        .get("schemaVersion")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let string_array = |field: &str| {
        package
            .manifest
            .get(field)
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let unverified_citation_labels = string_array("unverifiedCitationLabels");
    // `withheldCitations` is `[{label, reason}]` in the manifest, not a flat string list like the
    // field above; a withheld citation must never be indistinguishable from an unverified one, so
    // this is rendered as its own labeled list rather than folded into `unverified_citation_labels`.
    let withheld_citation_labels = package
        .manifest
        .get("withheldCitations")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .map(|entry| {
                    // A malformed entry is still a withheld citation; dropping it would understate
                    // what the package withholds.
                    let text = |field: &str| entry.get(field).and_then(|value| value.as_str());
                    let label = text("label").unwrap_or("[unlabeled citation]");
                    let reason = text("reason").unwrap_or("no reason recorded");
                    format!("{label} ({reason})")
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let ledger_sequence = package
        .manifest
        .get("ledgerSequence")
        .and_then(|value| value.as_u64());
    let previous_export_sha256 = package
        .manifest
        .get("previousExportSha256")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    // A revoked signer never fails the cryptographic checks above — the signature over these
    // bytes is still mathematically valid. Whether that revocation fails the overall verdict
    // depends on why the key was revoked: a routine, non-compromising rotation leaves every
    // package that key already signed trusted (`RETIRED`, shown for information only); a
    // compromised key fails everything it ever signed (`COMPROMISED`). Conflating the two would
    // make ordinary key hygiene retroactively fail every package this installation ever exported.
    let revocations = load_revocations(report_directory).await?;
    let revoked_signer = check_revocation(&package.signature.signer_key_id, &revocations).cloned();
    // Fail closed: only an exact "RETIRED" status is informational; anything else (a typo, a
    // future status value, corrupted data) is treated as compromised, never silently trusted.
    let signer_compromised = revoked_signer
        .as_ref()
        .is_some_and(|revoked| revoked.status != crate::integrity::REVOCATION_STATUS_RETIRED);
    Ok(PackageVerificationResult {
        schema_version,
        canonical_bytes_hash_matches: checks.canonical_bytes_hash_matches,
        signature_valid: checks.signature_valid,
        manifest_matches_canonical_bytes: checks.manifest_matches_canonical_bytes,
        overall_passed: checks.all_passed() && !signer_compromised,
        identity_assurance: Some(package.signature.identity_assurance),
        unverified_citation_labels,
        withheld_citation_labels,
        ledger_sequence,
        previous_export_sha256,
        revoked_signer,
        parse_error: None,
    })
}

async fn load_referenced_legal_authorities(
    repository: &DuckDbRepository,
    findings: &[LegalComplianceFinding],
) -> Result<Vec<PackagedLegalAuthority>, AppError> {
    let mut references = BTreeMap::new();
    for finding in findings
        .iter()
        .filter(|finding| matches!(finding.review_status.as_str(), "OPEN" | "CONFIRMED"))
    {
        references
            .entry((
                finding.matched_statute_id.clone(),
                finding.matched_version_id.clone(),
            ))
            .or_insert_with(|| {
                (
                    finding.statute_citation.clone(),
                    finding.authority_title.clone(),
                )
            });
    }
    let mut authorities = Vec::with_capacity(references.len());
    for ((statute_id, version_id), (citation, title)) in references {
        let detail = repository
            .get_legal_authority_version(&statute_id, &version_id)
            .await?;
        let computed_sha256 = sha256_hex(detail.full_text.as_bytes());
        if computed_sha256 != detail.version.content_sha256 {
            return Err(AppError::Export(format!(
                "Legal authority {statute_id}/{version_id} no longer matches its recorded SHA-256"
            )));
        }
        authorities.push(PackagedLegalAuthority {
            statute_id,
            version_id,
            citation,
            title,
            content_sha256: detail.version.content_sha256,
            effective_from: detail.version.effective_from,
            effective_to: detail.version.effective_to,
            source_uri: detail.version.source_uri,
            imported_at: RecordedInstant::from_stored(detail.version.imported_at),
        });
    }
    Ok(authorities)
}

fn validate_evidence_manifest(
    manifest: EvidencePackageManifest,
) -> Result<EvidencePackageManifest, AppError> {
    if sha256_hex(manifest.report.content.as_bytes()) != manifest.report.sha256 {
        return Err(AppError::Export(
            "Evidence package report content does not match its SHA-256".to_string(),
        ));
    }
    for document in &manifest.source_documents {
        for digest in [
            document.raw_file_sha256.as_deref(),
            document.extracted_text_sha256.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if !is_sha256_hex(digest) {
                return Err(AppError::Export(format!(
                    "Document {} contains an invalid recorded SHA-256",
                    document.document_id
                )));
            }
        }
    }
    Ok(manifest)
}

/// Writes the standalone verifier next to the report directory so a signed package's recipient
/// always has a runnable path, never a reference to this repository's working tree. Overwritten on
/// every export rather than left stale, since the verifier ships with the app and can change
/// between versions; it never depends on anything exported alongside it.
async fn ensure_verifier_script(report_directory: &Path) -> Result<EmittedVerifiers, AppError> {
    let script_path = report_directory.join(VERIFIER_SCRIPT_FILE);
    tokio::fs::write(&script_path, VERIFIER_SCRIPT)
        .await
        .map_err(|error| {
            AppError::Export(format!("Failed to write the verifier script: {error}"))
        })?;
    let page_path = report_directory.join(VERIFIER_PAGE_FILE);
    tokio::fs::write(&page_path, VERIFIER_PAGE)
        .await
        .map_err(|error| AppError::Export(format!("Failed to write the verifier page: {error}")))?;
    Ok(EmittedVerifiers {
        script_path,
        page_path,
    })
}

struct EmittedVerifiers {
    script_path: PathBuf,
    page_path: PathBuf,
}

fn reportable_markers(
    evidence: &LegalBriefEvidence,
    compliance: &StatutoryComplianceAnalysis,
) -> Vec<String> {
    let allowed = [
        TimelineAnomalyMarker::StatutoryLatencyViolation.as_str(),
        TimelineAnomalyMarker::StatutoryExtensionDefective.as_str(),
        TimelineAnomalyMarker::SuspiciouslyFastClosure.as_str(),
    ];
    let mut markers = evidence
        .integrity_flags
        .iter()
        .filter(|marker| allowed.contains(&marker.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    markers.extend(
        compliance
            .anomaly_markers
            .iter()
            .map(|marker| marker.as_str().to_string()),
    );
    markers.sort();
    markers.dedup();
    markers
}

fn render_report(
    evidence: &LegalBriefEvidence,
    compliance: &StatutoryComplianceAnalysis,
    markers: &[String],
    legal_findings: &[LegalComplianceFinding],
    author_name: &str,
    generated_at: &str,
) -> Result<String, AppError> {
    let extension_source = evidence
        .extension
        .as_ref()
        .map(|value| value.notice_source.as_str())
        .unwrap_or("No written extension source recorded");
    let request_reference = evidence
        .rpid
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| evidence.request_id.to_string());
    validate_template_variables()?;
    let mut finding_lines = Vec::with_capacity(markers.len() + 1);
    for marker in markers {
        finding_lines.push(format!("- `{marker}`"));
    }
    finding_lines.push(format!(
        "- {}",
        markdown_inline(&compliance.evidence_summary)?
    ));
    for finding in legal_findings
        .iter()
        .filter(|finding| matches!(finding.review_status.as_str(), "OPEN" | "CONFIRMED"))
    {
        finding_lines.push(format!(
            "- `{}` — {} — {} Review status: `{}`; rule confidence: {:.2}%.",
            markdown_inline(&finding.classification)?,
            markdown_inline(&finding.statute_citation)?,
            markdown_inline(&finding.evidence_summary)?,
            markdown_inline(&finding.review_status)?,
            f64::from(finding.confidence_basis_points) / 100.0,
        ));
    }
    let findings = finding_lines.join("\n");
    let documents = document_table(&evidence.documents)?;
    let mut values = BTreeMap::new();
    values.insert("GENERATED_AT", markdown_inline(generated_at)?);
    values.insert("AGENCY_NAME", markdown_inline(&evidence.agency_name)?);
    values.insert("PORTAL_NAME", markdown_inline(&evidence.portal_name)?);
    values.insert("PORTAL_URL", markdown_inline(&evidence.portal_url)?);
    values.insert("REQUEST_REFERENCE", markdown_inline(&request_reference)?);
    values.insert("REQUEST_TITLE", markdown_inline(&evidence.title)?);
    values.insert("REQUEST_ID", evidence.request_id.to_string());
    values.insert("PORTAL_KEY", markdown_inline(&evidence.portal_key)?);
    values.insert(
        "RPID",
        markdown_inline(evidence.rpid.as_deref().unwrap_or("Not recorded"))?,
    );
    values.insert("STATUS", markdown_inline(&evidence.status)?);
    values.insert("CREATED_AT", markdown_inline(&evidence.created_at)?);
    values.insert("UPDATED_AT", markdown_inline(&evidence.updated_at)?);
    values.insert(
        "CLOSED_AT",
        markdown_inline(evidence.closed_at.as_deref().unwrap_or("Not recorded"))?,
    );
    values.insert(
        "INITIAL_DEADLINE_AT",
        markdown_inline(&compliance.initial_deadline_at)?,
    );
    values.insert(
        "EFFECTIVE_DEADLINE_AT",
        markdown_inline(&compliance.effective_deadline_at)?,
    );
    values.insert(
        "DETERMINATION_AT",
        markdown_inline(
            compliance
                .determination_at
                .as_deref()
                .unwrap_or("No qualifying milestone recorded"),
        )?,
    );
    values.insert(
        "DETERMINATION_SOURCE",
        markdown_inline(
            compliance
                .determination_source
                .as_deref()
                .unwrap_or("No sourced determination event recorded"),
        )?,
    );
    values.insert(
        "CALENDAR_DAYS_ELAPSED",
        compliance.calendar_days_elapsed.to_string(),
    );
    values.insert(
        "EXTENSION_STATUS",
        markdown_inline(&format!("{:?}", compliance.extension_status))?,
    );
    values.insert(
        "EXTENSION_REASON",
        markdown_inline(
            compliance
                .extension_reason
                .as_deref()
                .unwrap_or("No statutory extension recorded"),
        )?,
    );
    values.insert("EXTENSION_SOURCE", markdown_inline(extension_source)?);
    values.insert("VIOLATION_FINDINGS", findings);
    values.insert(
        "REQUEST_DESCRIPTION",
        markdown_blockquote(&evidence.description)?,
    );
    values.insert("DOCUMENT_TABLE", documents);
    values.insert("AUTHOR_NAME", markdown_inline(author_name)?);

    let mut report = REPORT_TEMPLATE.to_string();
    for (key, value) in values {
        report = report.replace(&format!("{{{{{key}}}}}"), &value);
    }
    if report.len() > MAXIMUM_REPORT_BYTES {
        return Err(AppError::Export(format!(
            "Rendered report exceeds the {MAXIMUM_REPORT_BYTES}-byte limit"
        )));
    }
    Ok(report)
}

fn document_table(documents: &[EvidenceDocument]) -> Result<String, AppError> {
    if documents.is_empty() {
        return Ok("No documents were recorded in the portal dataset.".to_string());
    }
    let mut lines = vec![
        "| Document ID | File name | Bytes | Visibility | Raw SHA-256 | Text SHA-256 | Entropy | Sampled bytes | Density alert |".to_string(),
        "| --- | --- | ---: | --- | --- | --- | ---: | ---: | --- |".to_string(),
    ];
    for document in documents {
        // `None` is a document nothing ever assessed. 0.0 is a measurement -- every byte in the
        // sample identical -- so absence is printed rather than rounded into `0.0000`, matching
        // the wording the other unrecorded columns in this table already use.
        let entropy = document
            .shannon_entropy
            .map(|score| format!("{score:.4}"))
            .unwrap_or_else(|| "Not recorded".to_string());
        lines.push(format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            document.document_id,
            markdown_inline(&document.file_name)?,
            document
                .file_size
                .map(|value| value.to_string())
                .unwrap_or_else(|| "Not recorded".to_string()),
            markdown_inline(&document.visibility)?,
            markdown_inline(
                document
                    .raw_file_sha256
                    .as_deref()
                    .unwrap_or("Not recorded")
            )?,
            markdown_inline(
                document
                    .extracted_text_sha256
                    .as_deref()
                    .unwrap_or("Not recorded"),
            )?,
            entropy,
            document.entropy_sample_bytes,
            if document.suspicious_data_density {
                "SUSPICIOUS_DATA_DENSITY"
            } else {
                "None"
            },
        ));
    }
    Ok(lines.join("\n"))
}

fn markdown_inline(value: &str) -> Result<String, AppError> {
    let masked = mask_pii(value)?
        .masked_text
        .replace(['\r', '\n'], " ")
        .trim()
        .to_string();
    Ok(escape_markdown(&masked))
}

fn markdown_blockquote(value: &str) -> Result<String, AppError> {
    Ok(mask_pii(value)?
        .masked_text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(|line| escape_markdown(line.trim()))
        .collect::<Vec<_>>()
        .join("\n> "))
}

fn escape_markdown(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(
            character,
            '\\' | '`'
                | '*'
                | '_'
                | '{'
                | '}'
                | '['
                | ']'
                | '<'
                | '>'
                | '('
                | ')'
                | '#'
                | '+'
                | '-'
                | '.'
                | '!'
                | '|'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn validate_template_variables() -> Result<(), AppError> {
    let mut remainder = REPORT_TEMPLATE;
    while let Some(start) = remainder.find("{{") {
        remainder = &remainder[start + 2..];
        let end = remainder.find("}}").ok_or_else(|| {
            AppError::Export("Report template contains an unterminated variable".to_string())
        })?;
        let variable = &remainder[..end];
        if !TEMPLATE_VARIABLES.contains(&variable) {
            return Err(AppError::Export(format!(
                "Report template contains unsupported variable '{{{{{variable}}}}}'"
            )));
        }
        remainder = &remainder[end + 2..];
    }
    Ok(())
}

/// Shared by every export command that writes to the managed report directory. `allowed_extension`
/// exists because each caller's output is a different, fixed format: `export_legal_brief` always
/// writes `.md`, `export_case_evidence_package` always writes `.json`. This is not a general path
/// validator; a caller that accepted either extension for both exports would let a legal brief
/// silently overwrite where a chain-of-custody package was expected, or vice versa.
async fn validate_report_path(
    value: &str,
    output_root: &Path,
    allowed_extension: &str,
) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(value.trim());
    if path.as_os_str().is_empty()
        || !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case(allowed_extension))
    {
        return Err(AppError::Export(format!(
            "Output must be a non-empty .{allowed_extension} file path"
        )));
    }
    if tokio::fs::try_exists(&path)
        .await
        .map_err(|error| AppError::Export(format!("Failed to inspect report path: {error}")))?
    {
        return Err(AppError::Export(format!(
            "Report path '{}' already exists; evidence exports are never overwritten",
            path.display()
        )));
    }
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .ok_or_else(|| AppError::Export("Output must include a parent directory".to_string()))?;
    let parent = tokio::fs::canonicalize(parent).await.map_err(|error| {
        AppError::Export(format!("Report parent directory is inaccessible: {error}"))
    })?;
    let output_root = tokio::fs::canonicalize(output_root)
        .await
        .map_err(|error| {
            AppError::Export(format!("Managed report directory is inaccessible: {error}"))
        })?;
    if !parent.starts_with(&output_root) {
        return Err(AppError::Export(
            "Exports are restricted to the managed report directory".to_string(),
        ));
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Export("Output has no file name".to_string()))?;
    Ok(parent.join(file_name))
}

async fn commit_new_report(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| {
        AppError::Export("Legal brief output has no parent directory".to_string())
    })?;
    let temporary = parent.join(format!(".onrecord-report-{}.tmp", Uuid::new_v4()));
    let mut file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .await
        .map_err(|error| {
            AppError::Export(format!("Failed to create report staging file: {error}"))
        })?;
    if let Err(error) = file.write_all(bytes).await {
        drop(file);
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(AppError::Export(format!("Failed to write report: {error}")));
    }
    if let Err(error) = file.sync_all().await {
        drop(file);
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(AppError::Export(format!("Failed to sync report: {error}")));
    }
    drop(file);
    if let Err(error) = tokio::fs::hard_link(&temporary, path).await {
        let cleanup_error = tokio::fs::remove_file(&temporary).await.err();
        return Err(AppError::Export(format!(
            "Failed to commit report: {error}{}",
            cleanup_error
                .map(|cleanup| format!("; staging cleanup also failed: {cleanup}"))
                .unwrap_or_default()
        )));
    }
    // The final report is already committed at this point; a stale hidden staging link
    // must not turn a successful, non-overwriting export into a misleading failure.
    let _ = tokio::fs::remove_file(&temporary).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::casework::CASE_EVIDENCE_PACKAGE_SCHEMA;
    use crate::types::casework::{CaseworkCase, CaseworkCitation};
    use ed25519_dalek::SigningKey;

    fn evidence_manifest(generated_at: &str) -> EvidencePackageManifest {
        let report = "# Evidence report\n".to_string();
        EvidencePackageManifest {
            schema_version: EVIDENCE_PACKAGE_SCHEMA.to_string(),
            generated_at: generated_at.to_string(),
            request_id: 7,
            portal_key: "records".to_string(),
            report: PackagedReport {
                file_name: "report.md".to_string(),
                media_type: "text/markdown; charset=utf-8".to_string(),
                sha256: sha256_hex(report.as_bytes()),
                content: report,
            },
            source_documents: vec![PackagedSourceDocument {
                document_id: 9,
                file_size: Some(12),
                visibility: "public".to_string(),
                raw_file_sha256: Some(sha256_hex(b"source")),
                extracted_text_sha256: Some(sha256_hex(b"text")),
            }],
            legal_authorities: Vec::new(),
            legal_findings: Vec::new(),
            violation_markers: vec!["STATUTORY_LATENCY_VIOLATION".to_string()],
        }
    }

    fn case_manifest(
        unverified_citation_labels: Vec<String>,
    ) -> crate::types::casework::CaseEvidenceManifest {
        crate::types::casework::CaseEvidenceManifest {
            schema_version: CASE_EVIDENCE_PACKAGE_SCHEMA.to_string(),
            generated_at: "2026-09-28T00:00:00Z".to_string(),
            case: CaseworkCase::default(),
            verified_citation_count: 0,
            unverified_citation_labels,
            withheld_citations: Vec::new(),
            previous_export_sha256: None,
            ledger_sequence: 1,
        }
    }

    /// Locks the bytes the browser verifier (`scripts/verify-evidence-package.html`, exercised by
    /// `src/tests/staticPackageVerifier.test.ts`) is tested against for this schema. Regenerate the
    /// committed fixture from this same construction if the package layout changes deliberately.
    #[test]
    fn evidence_package_bytes_match_the_committed_browser_verifier_fixture() {
        let manifest =
            validate_evidence_manifest(evidence_manifest("2026-10-02T12:00:00Z")).unwrap();
        let key = SigningKey::from_bytes(&[11_u8; 32]);
        let package = sign_package(manifest, &key).unwrap();
        verify_package(&package).expect("the generated fixture must self-verify");
        assert_eq!(
            String::from_utf8_lossy(&serde_json::to_vec_pretty(&package).unwrap()),
            String::from_utf8_lossy(include_bytes!(
                "../../tests/fixtures/evidence-package-v3-fixture.json"
            )),
            "the signed package's byte layout has changed; see this test's doc comment"
        );
    }

    #[test]
    fn signed_package_is_externally_verifiable_and_tamper_evident() {
        let manifest =
            validate_evidence_manifest(evidence_manifest("2026-08-28T12:00:00Z")).unwrap();
        let key = SigningKey::from_bytes(&[7_u8; 32]);
        let mut package = sign_package(manifest, &key).unwrap();
        assert!(verify_package(&package).is_ok());
        assert_eq!(
            package.signature.identity_assurance,
            "SELF_MANAGED_LOCAL_KEY_NOT_EXTERNALLY_ATTESTED"
        );

        // Revocation is looked up by the claimed id, so a validly signed package naming another
        // key must fail rather than borrow that key's standing.
        let mut impersonating = package.clone();
        impersonating.signature.signer_key_id = sha256_hex(b"another installation's public key");
        assert!(verify_package(&impersonating).is_err());

        package
            .manifest
            .violation_markers
            .push("TAMPERED".to_string());
        assert!(verify_package(&package).is_err());
    }

    #[tokio::test]
    async fn verify_evidence_package_reports_each_check_and_never_hides_an_unverified_citation() {
        let mut manifest = case_manifest(vec!["Imported claim (UNVERIFIED_PORTABLE)".to_string()]);
        manifest.case.evidence = vec![CaseworkCitation {
            label: "Imported claim".into(),
            verification: "UNVERIFIED_PORTABLE".into(),
            ..Default::default()
        }];
        let key = SigningKey::from_bytes(&[5_u8; 32]);
        let package = sign_package(manifest, &key).unwrap();
        let payload = serde_json::to_string(&package).unwrap();
        let directory = tempfile::tempdir().unwrap();

        let passing = verify_evidence_package_inner(payload.clone(), directory.path())
            .await
            .unwrap();
        assert!(passing.overall_passed);
        assert!(passing.canonical_bytes_hash_matches);
        assert!(passing.signature_valid);
        assert!(passing.manifest_matches_canonical_bytes);
        assert_eq!(
            passing.identity_assurance.as_deref(),
            Some("SELF_MANAGED_LOCAL_KEY_NOT_EXTERNALLY_ATTESTED")
        );
        assert_eq!(
            passing.unverified_citation_labels,
            vec!["Imported claim (UNVERIFIED_PORTABLE)".to_string()]
        );
        assert!(passing.revoked_signer.is_none());

        let mut tampered: serde_json::Value = serde_json::from_str(&payload).unwrap();
        tampered["manifest"]["verifiedCitationCount"] = serde_json::json!(999);
        let failing = verify_evidence_package_inner(tampered.to_string(), directory.path())
            .await
            .unwrap();
        assert!(!failing.overall_passed);
        assert!(!failing.manifest_matches_canonical_bytes);
        assert!(failing.parse_error.is_none());

        let garbage = verify_evidence_package_inner("not json".to_string(), directory.path())
            .await
            .unwrap();
        assert!(!garbage.overall_passed);
        assert!(garbage.parse_error.is_some());

        let foreign = sign_package(
            serde_json::json!({
                "withheldCitations": [{ "label": "Memo", "reason": "privileged" }, { "label": "Note" }]
            }),
            &key,
        )
        .unwrap();
        let foreign = verify_evidence_package_inner(
            serde_json::to_string(&foreign).unwrap(),
            directory.path(),
        )
        .await
        .unwrap();
        assert_eq!(
            foreign.withheld_citation_labels,
            vec![
                "Memo (privileged)".to_string(),
                "Note (no reason recorded)".to_string()
            ],
            "a withheld entry missing its reason is still listed"
        );
    }

    /// Only a compromise fails what a revoked key already signed; a routine rotation stays
    /// visible but trusted.
    #[tokio::test]
    async fn verify_evidence_package_fails_a_revoked_signer_only_when_compromised() {
        for (reason, compromised, expected_status) in [
            ("Suspected compromise.", true, None),
            (
                "Routine hygiene rotation.",
                false,
                Some(crate::integrity::REVOCATION_STATUS_RETIRED),
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let key = SigningKey::from_bytes(&[6_u8; 32]);
            let package = sign_package(case_manifest(Vec::new()), &key).unwrap();
            let payload = serde_json::to_string(&package).unwrap();
            // Seed the active key file with exactly this key so rotation retires it.
            tokio::fs::write(
                directory.path().join(crate::integrity::SIGNING_KEY_FILE),
                key.to_bytes(),
            )
            .await
            .unwrap();
            rotate_signing_key(
                directory.path(),
                directory.path(),
                reason.to_string(),
                compromised,
            )
            .await
            .unwrap();

            let result = verify_evidence_package_inner(payload, directory.path())
                .await
                .unwrap();
            assert!(result.canonical_bytes_hash_matches && result.signature_valid);
            assert_eq!(result.overall_passed, !compromised, "{reason}");
            let revoked = result.revoked_signer.expect("the revocation stays visible");
            assert_eq!(revoked.reason, reason);
            if let Some(status) = expected_status {
                assert_eq!(revoked.status, status);
            }
        }
    }

    // REGRESSION(export): a legal brief copied `evaluated_at` and `imported_at` verbatim, so a row
    // written before 929af7a put `2026-08-29T14:14:190749Z` into the instant position of a signed
    // package and it read as a time the record never established. Proven by replacing
    // `RecordedInstant::from_stored` with `RecordedInstant::Instant { value }`: the malformed cases
    // below then serialize as `INSTANT`.
    #[test]
    fn a_stored_timestamp_that_is_not_an_instant_is_packaged_as_unparseable() {
        // The exact shape `%Y-%m-%dT%H:%M:%fZ` produced: no seconds field, microseconds in its place.
        let malformed = "2026-08-29T14:14:190749Z".to_string();
        assert_eq!(
            RecordedInstant::from_stored(malformed.clone()),
            RecordedInstant::Unparseable {
                recorded: malformed.clone()
            },
            "the seconds are unrecoverable, so this is not an instant"
        );
        assert_eq!(
            serde_json::to_value(RecordedInstant::from_stored(malformed.clone())).unwrap(),
            serde_json::json!({ "kind": "UNPARSEABLE", "recorded": malformed }),
            "a reader must be able to tell the two apart without reparsing"
        );

        let recorded = "2026-08-29T14:14:19.074900Z".to_string();
        assert_eq!(
            serde_json::to_value(RecordedInstant::from_stored(recorded.clone())).unwrap(),
            serde_json::json!({ "kind": "INSTANT", "value": recorded }),
            "a parseable value is carried verbatim, not reformatted"
        );
    }
}
