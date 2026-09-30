//! IPC command for the verifiable report export.

use crate::{
    analysis::finding::Finding,
    commands::profile::disclosure_context,
    commands::validation::{optional_text, portal_key as validate_portal_key, MAXIMUM_PATH_BYTES},
    error::AppError,
    integrity::{load_or_create_signing_key, sha256_hex},
    profile::ProfileStore,
    report::{
        builder::{build_manifest, ManifestOptions},
        output::{commit_new_file, validate_output_path},
        sign_and_render,
        source::load_source_documents,
        template_sha256,
    },
    storage::{corpus::CorpusIndex, findings::FindingsFilter, sqlite::DuckDbRepository},
    types::report::VerifiableReportExportResult,
};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};
use tauri::State;

const MAXIMUM_SELECTED_FINDINGS: usize = 200;
const MAXIMUM_FINDING_ID_BYTES: usize = 64;
const DEFAULT_FINDING_LIMIT: usize = 500;

pub(crate) fn validate_finding_ids(ids: Vec<String>) -> Result<Vec<String>, AppError> {
    if ids.len() > MAXIMUM_SELECTED_FINDINGS {
        return Err(AppError::DataParsing(format!(
            "At most {MAXIMUM_SELECTED_FINDINGS} findings can be selected for one report"
        )));
    }
    let mut unique = BTreeSet::new();
    for id in ids {
        let id = id.trim();
        if id.is_empty()
            || id.len() > MAXIMUM_FINDING_ID_BYTES
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(AppError::DataParsing("Finding id is malformed".to_string()));
        }
        unique.insert(id.to_string());
    }
    Ok(unique.into_iter().collect())
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiableReportRequest {
    pub portal_key: Option<String>,
    pub finding_ids: Option<Vec<String>>,
    pub target_path: Option<String>,
    pub author_name: Option<String>,
}

/// Exports a signed, self-verifying HTML report of findings. With `finding_ids`, exactly those
/// findings are considered; otherwise the highest-ranked active findings, optionally for one
/// portal. Inferred-redaction hypotheses are never included.
#[tauri::command]
pub async fn export_verifiable_report(
    request: VerifiableReportRequest,
    repository: State<'_, DuckDbRepository>,
    report_directory: State<'_, PathBuf>,
    profile_store: State<'_, Arc<ProfileStore>>,
    corpus: State<'_, CorpusIndex>,
) -> Result<VerifiableReportExportResult, AppError> {
    let VerifiableReportRequest {
        portal_key,
        finding_ids,
        target_path,
        author_name,
    } = request;
    let portal_key = portal_key
        .as_deref()
        .map(|value| validate_portal_key(value).map(str::to_string))
        .transpose()?;
    let author_name = optional_text(author_name, "Report author name", 256)?;
    let target_path = optional_text(target_path, "Report output path", MAXIMUM_PATH_BYTES)?;
    let finding_ids = finding_ids.map(validate_finding_ids).transpose()?;

    let mut extra_limitations = Vec::new();
    let findings: Vec<Finding> = match finding_ids {
        Some(ids) => {
            if ids.is_empty() {
                return Err(AppError::DataParsing(
                    "Select at least one finding, or omit the selection".to_string(),
                ));
            }
            let mut found = Vec::with_capacity(ids.len());
            for id in ids {
                let finding = repository
                    .get_finding_by_id(id.clone())
                    .await?
                    .ok_or_else(|| AppError::DataParsing(format!("Finding {id} was not found")))?;
                found.push(finding);
            }
            found
        }
        None => {
            let listed = repository
                .list_findings(
                    FindingsFilter {
                        portal_key: portal_key.clone(),
                        ..FindingsFilter::default()
                    },
                    DEFAULT_FINDING_LIMIT,
                )
                .await?;
            if listed.len() >= DEFAULT_FINDING_LIMIT {
                extra_limitations.push(format!(
                    "The report holds the {DEFAULT_FINDING_LIMIT} highest-ranked findings; more may exist."
                ));
            }
            listed
        }
    };
    if findings.is_empty() {
        return Err(AppError::Export(
            "There are no findings to report".to_string(),
        ));
    }

    let document_keys = findings
        .iter()
        .flat_map(|finding| finding.citations.iter())
        .map(|citation| (citation.portal_key.clone(), citation.document_id))
        .collect::<BTreeSet<_>>();
    let documents = load_source_documents(&repository, document_keys).await?;

    let disclosure = disclosure_context(&profile_store, &repository, &corpus).await?;
    let generated_at = chrono::Utc::now().to_rfc3339();
    let manifest = build_manifest(
        findings,
        &documents,
        ManifestOptions {
            template_sha256: template_sha256(),
            generated_at: generated_at.clone(),
            author_name,
            scope_portal_key: portal_key,
            extra_limitations,
            disclosure,
        },
    )?;
    let claim_count = manifest.claims.len();
    let withheld_count = manifest.withheld.iter().map(|item| item.count).sum();
    if claim_count == 0 {
        return Err(AppError::Export(
            "Every selected finding was withheld, so there is nothing to report".to_string(),
        ));
    }

    let target_path = target_path.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        report_directory
            .join(format!("onrecord-verifiable-report-{timestamp}.html"))
            .to_string_lossy()
            .into_owned()
    });
    let output_path = validate_output_path(&target_path, &report_directory).await?;
    let signing_key = load_or_create_signing_key(&report_directory).await?;
    let rendered = sign_and_render(manifest, &signing_key)?;
    commit_new_file(&output_path, rendered.html.as_bytes()).await?;
    Ok(VerifiableReportExportResult {
        output_path: output_path.to_string_lossy().into_owned(),
        file_sha256: sha256_hex(rendered.html.as_bytes()),
        payload_sha256: rendered.payload_sha256,
        signer_key_id: rendered.signer_key_id,
        identity_assurance: rendered.identity_assurance,
        claim_count,
        withheld_count,
        generated_at,
    })
}
