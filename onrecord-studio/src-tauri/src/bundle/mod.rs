//! The shareable read-only bundle: selected findings as one static HTML file that a reader opens
//! with no application, no network and no script. It is a wrapper around a versioned data
//! document embedded in the page; `BundleContent::packets` is where citable evidence packets are
//! embedded later without a schema change. Everything in it is masked text drawn from public
//! records; a finding that touches a private or removed record is left out whole.

mod prepare;
mod render;
#[cfg(test)]
mod tests;

pub use prepare::prepare;
pub use render::{parse_embedded, render_html};

use crate::analysis::finding::Finding;
use crate::error::AppError;
use crate::integrity::sha256_hex;
use crate::profile::{DisclosureContext, OWN_SUBJECT_SENTENCE};
use crate::report::builder::{
    build_manifest, withheld_finding_reason, ManifestOptions, MAXIMUM_CITATIONS_PER_CLAIM,
    PII_MASKING_LIMITATION,
};
use crate::report::SourceDocuments;
use crate::storage::pii::mask_pii;
use crate::types::report::{
    QuoteStatus, ReportCitation, ReportClaim, ReportConfidence, ReportEvidenceBasis, WithheldReason,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const BUNDLE_FORMAT: &str = "onrecord-read-only-bundle";
/// Bump on any change to the shape of `BundleDocument`; a reader refuses a version it does not
/// know rather than guessing.
pub const BUNDLE_SCHEMA_VERSION: u32 = 1;
pub const MAXIMUM_BUNDLE_BYTES: usize = 16 * 1024 * 1024;
pub const MAXIMUM_SELECTED_FINDINGS: usize = 200;

/// A citable evidence packet carried inside a bundle. The bundle does not interpret `payload`; the
/// packet's own format owns its schema, and `packet_sha256` lets a reader check it independently.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedPacket {
    pub packet_format: String,
    pub packet_version: u32,
    pub packet_sha256: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleFinding {
    /// Plain-language basis: counted comparison or pattern signal.
    pub basis_label: String,
    /// What this finding does not establish, derived from what was observed about it.
    pub limits: Vec<String>,
    pub claim: ReportClaim,
}

/// The part of the bundle the content hash covers. `generated_at` is outside it, so the same
/// selection of unchanged findings hashes identically at any time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleContent {
    pub title: String,
    pub findings: Vec<BundleFinding>,
    pub limitations: Vec<String>,
    #[serde(default)]
    pub packets: Vec<EmbeddedPacket>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleDocument {
    pub format: String,
    pub schema_version: u32,
    pub generated_at: String,
    /// SHA-256 of `content` in canonical form: object keys sorted, no whitespace.
    pub content_sha256: String,
    pub content: BundleContent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExclusionReason {
    NotFound,
    InferredRedactionIdentity,
    DismissedByReviewer,
    LegalClaimWithoutMeasuredConfidence,
    NoCitation,
    CitesNonPublicRecord,
    CitesRemovedSource,
}

impl ExclusionReason {
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::NotFound => "No finding with this id exists any longer.",
            Self::InferredRedactionIdentity => {
                "Hypotheses about who a redaction hides are never shared."
            }
            Self::DismissedByReviewer => "A reviewer dismissed this finding.",
            Self::LegalClaimWithoutMeasuredConfidence => {
                "A legal-claim finding with no measured confidence cannot be stated as one."
            }
            Self::NoCitation => "The finding cites no source a reader could check.",
            Self::CitesNonPublicRecord => {
                "The finding rests on a record that is not public, or that this installation \
                 cannot confirm is public."
            }
            Self::CitesRemovedSource => "The finding rests on a source that has been removed.",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleExclusion {
    pub finding_id: String,
    /// Masked. `None` when the finding no longer exists.
    pub headline: Option<String>,
    pub reason: ExclusionReason,
    pub explanation: String,
}

/// Canonical JSON: object keys sorted at every depth, no insignificant whitespace. Independent of
/// serde_json's map ordering feature, which another dependency may switch on.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let members: Vec<String> = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        Value::String(key.clone()),
                        canonical_json(&map[key])
                    )
                })
                .collect();
            format!("{{{}}}", members.join(","))
        }
        Value::Array(items) => {
            let members: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", members.join(","))
        }
        scalar => scalar.to_string(),
    }
}

pub fn content_sha256(content: &BundleContent) -> Result<String, AppError> {
    let value = serde_json::to_value(content)
        .map_err(|error| AppError::Export(format!("Failed to encode bundle content: {error}")))?;
    Ok(sha256_hex(canonical_json(&value).as_bytes()))
}

impl BundleDocument {
    pub fn new(content: BundleContent, generated_at: String) -> Result<Self, AppError> {
        Ok(Self {
            format: BUNDLE_FORMAT.to_string(),
            schema_version: BUNDLE_SCHEMA_VERSION,
            generated_at,
            content_sha256: content_sha256(&content)?,
            content,
        })
    }

    /// True when `content_sha256` matches the content as it stands.
    pub fn hash_is_intact(&self) -> Result<bool, AppError> {
        Ok(content_sha256(&self.content)? == self.content_sha256)
    }
}

/// Everything `assemble` decided: what goes in, and each finding left out with its reason.
pub struct Assembly {
    pub content: BundleContent,
    pub exclusions: Vec<BundleExclusion>,
}

/// How a source is named to a reader: its masked file name, else its label, else its id.
fn source_name(citation: &ReportCitation) -> String {
    citation
        .file_name
        .clone()
        .or_else(|| citation.label.clone())
        .unwrap_or_else(|| format!("Document {}", citation.document_id))
}

/// The portal and document a finding rests on, as checked for sharing.
fn cited_records(finding: &Finding) -> Vec<(String, i64)> {
    let mut records: Vec<(String, i64)> = finding
        .citations
        .iter()
        .map(|citation| (citation.portal_key.clone(), citation.document_id))
        .collect();
    if let (Some(portal), Some(document)) =
        (&finding.subject.portal_key, finding.subject.document_id)
    {
        records.push((portal.clone(), document));
    }
    records
}

/// Documents whose standing `assemble` will consult for these findings.
pub fn document_keys(findings: &[Finding]) -> BTreeSet<(String, i64)> {
    findings.iter().flat_map(cited_records).collect()
}

pub fn portal_keys(findings: &[Finding]) -> BTreeSet<String> {
    findings
        .iter()
        .flat_map(|finding| {
            finding
                .citations
                .iter()
                .map(|citation| citation.portal_key.clone())
                .chain(finding.subject.portal_key.clone())
        })
        .collect()
}

fn ineligibility(
    finding: &Finding,
    documents: &SourceDocuments,
    removed_portals: &BTreeSet<String>,
) -> Option<ExclusionReason> {
    match withheld_finding_reason(finding) {
        Some(WithheldReason::InferredRedactionIdentity) => {
            return Some(ExclusionReason::InferredRedactionIdentity)
        }
        Some(WithheldReason::DismissedByReviewer) => {
            return Some(ExclusionReason::DismissedByReviewer)
        }
        Some(WithheldReason::LegalClaimWithoutMeasuredConfidence) => {
            return Some(ExclusionReason::LegalClaimWithoutMeasuredConfidence)
        }
        Some(WithheldReason::QuoteMismatch | WithheldReason::QuoteNonPublicDocument) | None => {}
    }
    if finding.citations.is_empty() {
        return Some(ExclusionReason::NoCitation);
    }
    // A finding's headline and explanation are written from its records, so one private record
    // withdraws the whole finding, not only that record's excerpt.
    if finding.subject.document_id.is_some() && finding.subject.portal_key.is_none() {
        return Some(ExclusionReason::CitesNonPublicRecord);
    }
    let records = cited_records(finding);
    if records
        .iter()
        .any(|(portal, _)| removed_portals.contains(portal))
        || finding
            .subject
            .portal_key
            .as_ref()
            .is_some_and(|portal| removed_portals.contains(portal))
    {
        return Some(ExclusionReason::CitesRemovedSource);
    }
    let all_public = records.iter().all(|key| {
        documents
            .get(key)
            .is_some_and(|document| document.is_public())
    });
    (!all_public).then_some(ExclusionReason::CitesNonPublicRecord)
}

fn basis_label(basis: ReportEvidenceBasis) -> &'static str {
    match basis {
        ReportEvidenceBasis::CountedViolation => "Counted comparison against a stated rule",
        ReportEvidenceBasis::PatternSignal => "Pattern signal, not a legal conclusion",
    }
}

fn limits_for(claim: &ReportClaim) -> Vec<String> {
    let mut limits = vec![match claim.evidence_basis {
        ReportEvidenceBasis::CountedViolation => {
            "A counted comparison of records against a rule or commitment the record itself \
             states. It is not a ruling by a court or an agency."
                .to_string()
        }
        ReportEvidenceBasis::PatternSignal => {
            "A pattern signal: evidence for review, not a legal conclusion.".to_string()
        }
    }];
    if claim.confidence == ReportConfidence::Unassessed {
        limits.push("Confidence was not assessed for this finding.".to_string());
    }
    let count = |wanted: QuoteStatus| {
        claim
            .citations
            .iter()
            .filter(|citation| citation.quote_status == wanted)
            .count()
    };
    let unchecked = count(QuoteStatus::UncheckedDetectorQuote);
    if unchecked > 0 {
        limits.push(format!(
            "{unchecked} excerpt(s) are the detector's own wording and could not be checked \
             against stored text."
        ));
    }
    let mismatched = count(QuoteStatus::WithheldMismatch);
    if mismatched > 0 {
        limits.push(format!(
            "{mismatched} excerpt(s) were withheld because they did not match the stored text."
        ));
    }
    let masked: usize = claim
        .citations
        .iter()
        .map(|citation| citation.pii_masked_count)
        .sum();
    if masked > 0 {
        limits.push(format!(
            "Personal information was masked in {masked} place(s) in the excerpts."
        ));
    }
    limits
}

pub fn assemble(
    findings: Vec<Finding>,
    documents: &SourceDocuments,
    removed_portals: &BTreeSet<String>,
    disclosure: &DisclosureContext,
) -> Result<Assembly, AppError> {
    let mut exclusions = Vec::new();
    let mut eligible = Vec::new();
    let mut own_subject_ids = Vec::new();
    let mut omitted_citations = 0_usize;
    for finding in findings {
        if let Some(reason) = ineligibility(&finding, documents, removed_portals) {
            // An inferred identity is a hypothesis: even its masked headline stays out.
            let headline = if reason == ExclusionReason::InferredRedactionIdentity {
                None
            } else {
                Some(mask_pii(&finding.headline)?.masked_text)
            };
            exclusions.push(BundleExclusion {
                finding_id: finding.id,
                headline,
                reason,
                explanation: reason.explanation().to_string(),
            });
            continue;
        }
        if disclosure.own_subject_sentence(&finding.subject).is_some() {
            own_subject_ids.push(finding.id.clone());
        }
        omitted_citations += finding
            .citations
            .len()
            .saturating_sub(MAXIMUM_CITATIONS_PER_CLAIM);
        eligible.push(finding);
    }
    let manifest = build_manifest(
        eligible,
        documents,
        ManifestOptions {
            template_sha256: String::new(),
            generated_at: String::new(),
            author_name: None,
            scope_portal_key: None,
            extra_limitations: Vec::new(),
            disclosure: disclosure.clone(),
        },
    )?;
    let findings = manifest
        .claims
        .into_iter()
        .map(|claim| BundleFinding {
            basis_label: basis_label(claim.evidence_basis).to_string(),
            limits: limits_for(&claim),
            claim,
        })
        .collect();
    let mut limitations = Vec::new();
    if let Some(sentence) = disclosure.sentence() {
        limitations.push(sentence.to_string());
    }
    if !own_subject_ids.is_empty() {
        limitations.push(format!(
            "{OWN_SUBJECT_SENTENCE} Findings concerning it: {}.",
            own_subject_ids.join(", ")
        ));
    }
    limitations.extend([
        "This bundle is unsigned. The content hash shows the data has not changed since it was \
         written; it does not say who wrote it."
            .to_string(),
        PII_MASKING_LIMITATION.to_string(),
        "Source documents are not included. Each source is identified by name and SHA-256 so \
         the version this installation held can be recognized."
            .to_string(),
    ]);
    if omitted_citations > 0 {
        limitations.push(format!(
            "{omitted_citations} citation(s) beyond {MAXIMUM_CITATIONS_PER_CLAIM} per finding are \
             not listed."
        ));
    }
    Ok(Assembly {
        content: BundleContent {
            title: "Findings bundle (read-only)".to_string(),
            findings,
            limitations,
            packets: Vec::new(),
        },
        exclusions,
    })
}

/// What the confirmation screen shows: exactly what will be written, by the hash that identifies
/// it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundlePreviewItem {
    pub finding_id: String,
    pub headline: String,
    pub severity: String,
    pub basis_label: String,
    pub source_count: usize,
    pub excerpt_count: usize,
    pub masked_value_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundlePreview {
    pub schema_version: u32,
    pub included: Vec<BundlePreviewItem>,
    pub excluded: Vec<BundleExclusion>,
    /// Distinct source files named in the bundle, as they will appear (masked).
    pub source_names: Vec<String>,
    pub content_sha256: String,
}

pub fn preview_of(assembly: &Assembly) -> Result<BundlePreview, AppError> {
    let mut names = BTreeSet::new();
    let included = assembly
        .content
        .findings
        .iter()
        .map(|finding| {
            let claim = &finding.claim;
            names.extend(claim.citations.iter().map(source_name));
            BundlePreviewItem {
                finding_id: claim.finding_id.clone(),
                headline: claim.headline.clone(),
                severity: claim.severity.clone(),
                basis_label: finding.basis_label.clone(),
                source_count: claim.citations.len(),
                excerpt_count: claim
                    .citations
                    .iter()
                    .filter(|citation| citation.quote.is_some())
                    .count(),
                masked_value_count: claim
                    .citations
                    .iter()
                    .map(|citation| citation.pii_masked_count)
                    .sum(),
            }
        })
        .collect();
    Ok(BundlePreview {
        schema_version: BUNDLE_SCHEMA_VERSION,
        included,
        excluded: assembly.exclusions.clone(),
        source_names: names.into_iter().collect(),
        content_sha256: content_sha256(&assembly.content)?,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ShareableBundleResult {
    pub output_path: String,
    pub file_sha256: String,
    pub content_sha256: String,
    pub generated_at: String,
    pub finding_count: usize,
    pub excluded_count: usize,
}
