//! Turns stored findings into a redaction-safe manifest. Pure over its inputs so the invariants
//! (hypotheses excluded, quotes masked with whole-document context, unassessed never scored) are
//! testable without a database.

use super::{SourceDocument, SourceDocuments};
use crate::analysis::finding::{Finding, FindingCitation, FindingConfidence, FindingKind};
use crate::error::AppError;
use crate::profile::DisclosureContext;
use crate::storage::pii::{mask_pii, mask_pii_character_range};
use crate::types::report::{
    QuoteStatus, ReportCitation, ReportClaim, ReportConfidence, ReportEvidenceBasis,
    VerifiableReportManifest, WithheldItem, WithheldReason, REPORT_SCHEMA_VERSION,
};
use std::collections::BTreeMap;

pub const MAXIMUM_QUOTE_CHARACTERS: usize = 2_000;
pub const MAXIMUM_CITATIONS_PER_CLAIM: usize = 50;

pub struct ManifestOptions {
    pub template_sha256: String,
    pub generated_at: String,
    pub author_name: Option<String>,
    pub scope_portal_key: Option<String>,
    pub extra_limitations: Vec<String>,
    /// Required, not optional: the author's declared affiliation is part of the signed payload.
    pub disclosure: DisclosureContext,
}

#[derive(Default)]
struct Withholdings(BTreeMap<WithheldReason, usize>);

impl Withholdings {
    fn note(&mut self, reason: WithheldReason) {
        *self.0.entry(reason).or_insert(0) += 1;
    }
}

pub fn build_manifest(
    findings: Vec<Finding>,
    documents: &SourceDocuments,
    options: ManifestOptions,
) -> Result<VerifiableReportManifest, AppError> {
    let mut withheld = Withholdings::default();
    let mut claims = Vec::new();
    let mut omitted_citations = 0_usize;
    let mut own_subject_finding_ids = Vec::new();
    for finding in findings {
        if finding.kind == FindingKind::RedactionCandidateInferred {
            withheld.note(WithheldReason::InferredRedactionIdentity);
            continue;
        }
        if finding.dismissed_at.is_some() {
            withheld.note(WithheldReason::DismissedByReviewer);
            continue;
        }
        if finding.kind.is_legal_claim()
            && !matches!(finding.confidence, FindingConfidence::Measured { .. })
        {
            withheld.note(WithheldReason::LegalClaimWithoutMeasuredConfidence);
            continue;
        }
        if options
            .disclosure
            .own_subject_sentence(&finding.subject)
            .is_some()
        {
            own_subject_finding_ids.push(finding.id.clone());
        }
        claims.push(build_claim(
            finding,
            documents,
            &mut withheld,
            &mut omitted_citations,
        )?);
    }
    let mut limitations = vec![
        "The signature shows this file is unchanged since it was signed by the holder of the \
         embedded key. It does not say who that is: the key is generated and held locally and is \
         not externally attested."
            .to_string(),
        "This page cannot check whether the signing key has since been revoked.".to_string(),
        "Personal-information masking is structural (Social Security numbers, telephone numbers, \
         street addresses). It is not a guarantee that no sensitive value remains; review before \
         publishing."
            .to_string(),
        "A pattern signal is not a legal conclusion. Only a counted violation is presented as a \
         legal claim."
            .to_string(),
    ];
    if let Some(sentence) = options.disclosure.sentence() {
        limitations.insert(0, sentence.to_string());
    }
    if !own_subject_finding_ids.is_empty() {
        limitations.insert(
            1,
            format!(
                "{} Findings concerning it: {}.",
                crate::profile::OWN_SUBJECT_SENTENCE,
                own_subject_finding_ids.join(", ")
            ),
        );
    }
    limitations.extend(options.extra_limitations);
    if omitted_citations > 0 {
        limitations.push(format!(
            "{omitted_citations} citation(s) beyond {MAXIMUM_CITATIONS_PER_CLAIM} per claim are not listed."
        ));
    }
    Ok(VerifiableReportManifest {
        schema_version: REPORT_SCHEMA_VERSION.to_string(),
        template_sha256: options.template_sha256,
        generated_at: options.generated_at,
        title: "Verifiable findings report".to_string(),
        author_name: options.author_name,
        scope_portal_key: options.scope_portal_key,
        claims,
        withheld: withheld
            .0
            .into_iter()
            .map(|(reason, count)| WithheldItem {
                reason,
                count,
                explanation: reason.explanation().to_string(),
            })
            .collect(),
        limitations,
    })
}

fn build_claim(
    finding: Finding,
    documents: &SourceDocuments,
    withheld: &mut Withholdings,
    omitted_citations: &mut usize,
) -> Result<ReportClaim, AppError> {
    // Detector prose can embed the same values a quote can, so it takes the same masking.
    let headline = mask_pii(&finding.headline)?.masked_text;
    let explanation = mask_pii(&finding.explanation)?.masked_text;
    *omitted_citations += finding
        .citations
        .len()
        .saturating_sub(MAXIMUM_CITATIONS_PER_CLAIM);
    let citations = finding
        .citations
        .iter()
        .take(MAXIMUM_CITATIONS_PER_CLAIM)
        .map(|citation| build_citation(citation, documents, withheld))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ReportClaim {
        finding_id: finding.id,
        kind: finding.kind.as_str().to_string(),
        severity: finding.severity.as_str().to_string(),
        evidence_basis: if finding.kind.is_legal_claim() {
            ReportEvidenceBasis::CountedViolation
        } else {
            ReportEvidenceBasis::PatternSignal
        },
        confidence: match finding.confidence {
            FindingConfidence::Unassessed => ReportConfidence::Unassessed,
            FindingConfidence::Measured { basis_points } => {
                ReportConfidence::Measured { basis_points }
            }
        },
        headline,
        explanation,
        source_detector: finding.source_detector,
        discovered_at: finding.discovered_at.to_rfc3339(),
        last_seen_at: finding.last_seen_at.to_rfc3339(),
        portal_key: finding.subject.portal_key,
        request_id: finding.subject.request_id,
        citations,
    })
}

struct Excerpt {
    quote: Option<String>,
    status: QuoteStatus,
    truncated: bool,
    masked_count: usize,
}

impl Excerpt {
    fn none(status: QuoteStatus) -> Self {
        Self {
            quote: None,
            status,
            truncated: false,
            masked_count: 0,
        }
    }
}

fn build_citation(
    citation: &FindingCitation,
    documents: &SourceDocuments,
    withheld: &mut Withholdings,
) -> Result<ReportCitation, AppError> {
    let document = documents.get(&(citation.portal_key.clone(), citation.document_id));
    let public =
        document.is_some_and(|document| document.visibility.trim().eq_ignore_ascii_case("public"));
    let label = citation
        .label
        .as_deref()
        .map(|label| mask_pii(label).map(|masked| masked.masked_text))
        .transpose()?;
    let base = ReportCitation {
        portal_key: citation.portal_key.clone(),
        document_id: citation.document_id,
        label,
        file_name: None,
        character_start: citation.character_start,
        character_end: citation.character_end,
        quote: None,
        quote_status: QuoteStatus::NoQuote,
        quote_truncated: false,
        pii_masked_count: 0,
        raw_file_sha256: None,
        extracted_text_projection_sha256: None,
    };
    // A document that is not public contributes nothing beyond its id: not its name, not its
    // digests, not an excerpt.
    let Some(document) = document.filter(|_| public) else {
        withheld.note(WithheldReason::QuoteNonPublicDocument);
        return Ok(ReportCitation {
            quote_status: QuoteStatus::WithheldNonPublicDocument,
            ..base
        });
    };
    let excerpt = excerpt_for(citation, document)?;
    if excerpt.status == QuoteStatus::WithheldMismatch {
        withheld.note(WithheldReason::QuoteMismatch);
    }
    Ok(ReportCitation {
        file_name: document
            .file_name
            .as_deref()
            .map(|name| mask_pii(name).map(|masked| masked.masked_text))
            .transpose()?,
        quote: excerpt.quote,
        quote_status: excerpt.status,
        quote_truncated: excerpt.truncated,
        pii_masked_count: excerpt.masked_count,
        raw_file_sha256: document.raw_file_sha256.clone(),
        extracted_text_projection_sha256: document.extracted_text_projection_sha256.clone(),
        ..base
    })
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn excerpt_for(citation: &FindingCitation, document: &SourceDocument) -> Result<Excerpt, AppError> {
    let detector_quote = citation
        .quote
        .as_deref()
        .filter(|quote| !quote.trim().is_empty());
    let range = document.text.as_deref().and_then(|text| {
        let start = usize::try_from(citation.character_start?).ok()?;
        let end = usize::try_from(citation.character_end?).ok()?;
        let total = text.chars().count();
        (start < end && end <= total).then_some((text, start, end))
    });
    if let Some((text, start, end)) = range {
        let range_text: String = text.chars().skip(start).take(end - start).collect();
        if let Some(quote) = detector_quote {
            if !collapse_whitespace(&range_text).contains(&collapse_whitespace(quote)) {
                return Ok(Excerpt::none(QuoteStatus::WithheldMismatch));
            }
        }
        // Detection runs over the whole document so a value that straddles the range boundary is
        // masked here exactly as it would be when the document is read in full.
        let masked = mask_pii_character_range(text, start, end - start)?;
        let (quote, truncated) = truncate_characters(masked.masked_text);
        return Ok(Excerpt {
            quote: Some(quote),
            status: if detector_quote.is_some() {
                QuoteStatus::VerifiedAgainstStoredText
            } else {
                QuoteStatus::FromStoredTextRange
            },
            truncated,
            masked_count: masked.findings.len(),
        });
    }
    match detector_quote {
        Some(quote) => {
            let masked = mask_pii(quote)?;
            let (quote, truncated) = truncate_characters(masked.masked_text);
            Ok(Excerpt {
                quote: Some(quote),
                status: QuoteStatus::UncheckedDetectorQuote,
                truncated,
                masked_count: masked.findings.len(),
            })
        }
        None => Ok(Excerpt::none(QuoteStatus::NoQuote)),
    }
}

fn truncate_characters(value: String) -> (String, bool) {
    match value.char_indices().nth(MAXIMUM_QUOTE_CHARACTERS) {
        Some((index, _)) => (value[..index].to_string(), true),
        None => (value, false),
    }
}
