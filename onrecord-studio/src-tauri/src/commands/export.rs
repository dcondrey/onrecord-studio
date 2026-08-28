use crate::analysis::StatutoryComplianceAnalysis;
use crate::commands::validation::{
    optional_text, portal_key as validate_portal_key, MAXIMUM_PATH_BYTES,
};
use crate::error::AppError;
use crate::integrity::{digest_hex, sha256_hex};
use crate::storage::pii::mask_pii;
use crate::storage::sqlite::{DuckDbRepository, EvidenceDocument, LegalBriefEvidence};
use crate::types::{LegalBriefExportResult, LegalComplianceFinding, TimelineAnomalyMarker};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tauri::State;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

const REPORT_TEMPLATE: &str = include_str!("../../../templates/report_template.md");
const MAXIMUM_REPORT_BYTES: usize = 32 * 1024 * 1024;
const MAXIMUM_EVIDENCE_PACKAGE_BYTES: usize = 64 * 1024 * 1024;
const EVIDENCE_PACKAGE_SCHEMA: &str = "onrecord-evidence-package-v1";
const SIGNATURE_ALGORITHM: &str = "Ed25519";
const SIGNING_KEY_FILE: &str = ".onrecord-export-signing-key-v1";

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
    imported_at: String,
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
    evaluated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct EvidencePackageSignature {
    algorithm: String,
    public_key_hex: String,
    signer_key_id: String,
    signature_hex: String,
    identity_assurance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct SignedEvidencePackage {
    manifest_sha256: String,
    manifest: EvidencePackageManifest,
    signature: EvidencePackageSignature,
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
    let output_path = validate_report_path(&target_path, &report_directory).await?;
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
    let signing_key = load_or_create_signing_key(&report_directory).await?;
    let package = sign_evidence_package(
        EvidencePackageManifest {
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
                    evaluated_at: finding.evaluated_at.clone(),
                })
                .collect(),
            violation_markers: markers.clone(),
        },
        &signing_key,
    )?;
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
            imported_at: detail.version.imported_at,
        });
    }
    Ok(authorities)
}

fn sign_evidence_package(
    manifest: EvidencePackageManifest,
    signing_key: &SigningKey,
) -> Result<SignedEvidencePackage, AppError> {
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
            if !is_sha256(digest) {
                return Err(AppError::Export(format!(
                    "Document {} contains an invalid recorded SHA-256",
                    document.document_id
                )));
            }
        }
    }
    let manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|error| AppError::Export(format!("Failed to encode package manifest: {error}")))?;
    let manifest_sha256 = sha256_hex(&manifest_bytes);
    let verifying_key = signing_key.verifying_key();
    let public_key = verifying_key.to_bytes();
    let signature = signing_key.sign(&manifest_bytes).to_bytes();
    Ok(SignedEvidencePackage {
        manifest_sha256,
        manifest,
        signature: EvidencePackageSignature {
            algorithm: SIGNATURE_ALGORITHM.to_string(),
            public_key_hex: digest_hex(public_key),
            signer_key_id: sha256_hex(&public_key),
            signature_hex: digest_hex(signature),
            identity_assurance: "SELF_MANAGED_LOCAL_KEY_NOT_EXTERNALLY_ATTESTED".to_string(),
        },
    })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn load_or_create_signing_key(report_directory: &Path) -> Result<SigningKey, AppError> {
    let key_path = report_directory.join(SIGNING_KEY_FILE);
    match tokio::fs::read(&key_path).await {
        Ok(bytes) => return signing_key_from_bytes(&bytes, &key_path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::Export(format!(
                "Failed to read the local export signing key: {error}"
            )))
        }
    }

    let mut entropy = Sha256::new();
    entropy.update(Uuid::new_v4().as_bytes());
    entropy.update(Uuid::new_v4().as_bytes());
    entropy.update(
        chrono::Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or_default()
            .to_le_bytes(),
    );
    let seed: [u8; 32] = entropy.finalize().into();

    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    match options.open(&key_path).await {
        Ok(mut file) => {
            if let Err(error) = file.write_all(&seed).await {
                drop(file);
                let _ = tokio::fs::remove_file(&key_path).await;
                return Err(AppError::Export(format!(
                    "Failed to write the local export signing key: {error}"
                )));
            }
            if let Err(error) = file.sync_all().await {
                drop(file);
                let _ = tokio::fs::remove_file(&key_path).await;
                return Err(AppError::Export(format!(
                    "Failed to sync the local export signing key: {error}"
                )));
            }
            Ok(SigningKey::from_bytes(&seed))
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let bytes = tokio::fs::read(&key_path).await.map_err(|read_error| {
                AppError::Export(format!(
                    "The export signing key was created concurrently but could not be read: {read_error}"
                ))
            })?;
            signing_key_from_bytes(&bytes, &key_path)
        }
        Err(error) => Err(AppError::Export(format!(
            "Failed to create the local export signing key: {error}"
        ))),
    }
}

fn signing_key_from_bytes(bytes: &[u8], path: &Path) -> Result<SigningKey, AppError> {
    let seed: [u8; 32] = bytes.try_into().map_err(|_| {
        AppError::Export(format!(
            "Export signing key '{}' must contain exactly 32 bytes",
            path.display()
        ))
    })?;
    Ok(SigningKey::from_bytes(&seed))
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

async fn validate_report_path(value: &str, output_root: &Path) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(value.trim());
    if path.as_os_str().is_empty()
        || !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
    {
        return Err(AppError::Export(
            "Legal brief output must be a non-empty .md file path".to_string(),
        ));
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
        .ok_or_else(|| {
            AppError::Export("Legal brief output must include a parent directory".to_string())
        })?;
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
            "Legal brief exports are restricted to the managed report directory".to_string(),
        ));
    }
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Export("Legal brief output has no file name".to_string()))?;
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
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    fn decode_hex<const SIZE: usize>(value: &str) -> [u8; SIZE] {
        assert_eq!(value.len(), SIZE * 2);
        let mut bytes = [0_u8; SIZE];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
        }
        bytes
    }

    fn verifies(package: &SignedEvidencePackage) -> bool {
        let manifest = serde_json::to_vec(&package.manifest).unwrap();
        if sha256_hex(&manifest) != package.manifest_sha256 {
            return false;
        }
        let public_key = decode_hex::<32>(&package.signature.public_key_hex);
        let signature = decode_hex::<64>(&package.signature.signature_hex);
        VerifyingKey::from_bytes(&public_key)
            .and_then(|key| key.verify(&manifest, &Signature::from_bytes(&signature)))
            .is_ok()
    }

    #[test]
    fn signed_package_is_externally_verifiable_and_tamper_evident() {
        let report = "# Evidence report\n".to_string();
        let manifest = EvidencePackageManifest {
            schema_version: EVIDENCE_PACKAGE_SCHEMA.to_string(),
            generated_at: "2026-08-28T12:00:00Z".to_string(),
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
        };
        let key = SigningKey::from_bytes(&[7_u8; 32]);
        let mut package = sign_evidence_package(manifest, &key).unwrap();
        assert!(verifies(&package));
        assert_eq!(
            package.signature.identity_assurance,
            "SELF_MANAGED_LOCAL_KEY_NOT_EXTERNALLY_ATTESTED"
        );

        package
            .manifest
            .violation_markers
            .push("TAMPERED".to_string());
        assert!(!verifies(&package));
    }

    #[tokio::test]
    async fn export_signing_key_is_created_once_and_reused() {
        let directory = tempfile::tempdir().unwrap();
        let first = load_or_create_signing_key(directory.path()).await.unwrap();
        let second = load_or_create_signing_key(directory.path()).await.unwrap();
        assert_eq!(first.verifying_key(), second.verifying_key());
        assert_eq!(
            tokio::fs::read(directory.path().join(SIGNING_KEY_FILE))
                .await
                .unwrap()
                .len(),
            32
        );
    }
}
