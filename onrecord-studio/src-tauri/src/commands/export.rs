use crate::analysis::{evaluate_statutory_compliance, StatutoryComplianceAnalysis};
use crate::commands::validation::{
    optional_text, portal_key as validate_portal_key, MAXIMUM_PATH_BYTES,
};
use crate::error::AppError;
use crate::integrity::sha256_hex;
use crate::storage::pii::mask_pii;
use crate::storage::sqlite::{EvidenceDocument, LegalBriefEvidence, SqliteRepository};
use crate::types::{LegalBriefExportResult, LegalComplianceFinding, TimelineAnomalyMarker};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tauri::State;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

const REPORT_TEMPLATE: &str = include_str!("../../../templates/report_template.md");
const MAXIMUM_REPORT_BYTES: usize = 32 * 1024 * 1024;

// FEATURE(export): Add a signed evidence-package export containing the report, referenced source
// hashes, legal-authority versions, and a machine-verifiable manifest for external review.
const TEMPLATE_VARIABLES: [&str; 24] = [
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
    repository: State<'_, SqliteRepository>,
    report_directory: State<'_, PathBuf>,
) -> Result<LegalBriefExportResult, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    let author_name = optional_text(author_name, "Report author name", 256)?;
    let target_path = optional_text(target_path, "Report output path", MAXIMUM_PATH_BYTES)?;
    let evidence = repository
        .load_legal_brief_evidence(request_id, portal_key)
        .await?;
    let compliance = evaluate_statutory_compliance(
        request_id,
        &evidence.created_at,
        Some(&evidence.updated_at),
        evidence.closed_at.as_deref(),
        evidence.extension.as_ref(),
    )?;
    let legal_findings = repository
        .list_request_legal_findings(request_id, portal_key)
        .await?;
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
    commit_new_report(&output_path, report.as_bytes()).await?;
    let result = LegalBriefExportResult {
        request_id,
        portal_key: evidence.portal_key,
        output_path: output_path.to_string_lossy().into_owned(),
        report_sha256,
        generated_at,
        included_violation_markers: markers,
    };
    if let Err(error) = repository.record_report_export(&result).await {
        return match tokio::fs::remove_file(&output_path).await {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(AppError::Export(format!(
                "Failed to record report provenance ({error}); the committed report could not be rolled back: {cleanup_error}"
            ))),
        };
    }
    Ok(result)
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
