//! Actionable artifacts drafted from an autonomous [`Finding`].
//!
//! The findings inbox (Wave 1) surfaces observations. This module turns those observations into
//! drafts a user can send. A pattern or statistical finding (`is_legal_claim() == false`) yields
//! only a narrowing records request aimed at the specific gap; it never accuses. Only a
//! legal-claim finding, or a request whose statutory deadline the evaluator counts as exceeded
//! (see [`statutory_appeal`]), may also yield an appeal that cites a statute. Every specific
//! claim in a draft traces back to the source finding's citations or subject — nothing is invented.
//! When the finding does not carry a specific fact the template needs, the draft emits a
//! `[BRACKETED PLACEHOLDER]` for the user to fill rather than guessing.
//!
//! Templates live in [`templates`] as a plain data table so a new jurisdiction can be added by
//! extending the table, not by editing generator code.
//!
//! **What this module will not do:** send anything. It produces drafts. Delivery is the user's,
//! not the app's.

pub mod agency_record;
pub mod appeal;
pub mod followup_request;
pub mod overdue_followup;
pub mod statutory_appeal;
pub mod templates;

use crate::profile::DisclosureContext;
use crate::{
    analysis::finding::{Finding, FindingCitation, FindingConfidence, FindingKind},
    integrity::sha256_hex,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which action the draft is designed for.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ArtifactKind {
    FollowupRequest,
    Appeal,
}

impl ArtifactKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FollowupRequest => "FOLLOWUP_REQUEST",
            Self::Appeal => "APPEAL",
        }
    }
}

/// One statute the draft rests on. `quote` is optional; when present it comes from the template,
/// not fabricated at generation time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CitedAuthority {
    pub statute: String,
    pub quote: Option<String>,
}

/// What the draft's claims rest on. Never inferred from the wording; set by the generator that
/// produced the draft.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DraftEvidenceBasis {
    /// A statistical or pattern observation over released documents. Not a finding of violation.
    PatternObservation,
    /// A counted comparison of observed requests against a commitment the agency's own document
    /// states.
    CountedPolicyComparison,
    /// A request whose sourced determination date exceeded the statutory deadline.
    CountedStatutoryDeadline,
    /// A request whose computed deadline passed with nothing recorded in response. States the
    /// count and the absence; it is not a finding about the agency's conduct.
    DeadlinePassedWithoutRecordedResponse,
}

impl DraftEvidenceBasis {
    pub(super) fn statement(self) -> &'static str {
        match self {
            Self::PatternObservation => {
                "This request rests on a pattern observed in documents the agency released. It is \
                 not a claim that any law was violated."
            }
            Self::CountedPolicyComparison => {
                "This complaint rests on a count of requests compared with a commitment stated in \
                 the agency's own policy document, quoted below."
            }
            Self::CountedStatutoryDeadline => {
                "This appeal rests on recorded dates for the request, counted against the \
                 statutory determination period."
            }
            Self::DeadlinePassedWithoutRecordedResponse => {
                "This letter rests on the recorded received date, counted against the statutory \
                 determination period, and on the absence of a recorded response in my own \
                 records. It does not state why no response is recorded."
            }
        }
    }
}

/// What a finding kind can produce. Exhaustive on purpose: a new [`FindingKind`] fails to compile
/// here until someone decides which artifact family it belongs to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DraftFamily {
    /// Narrowing records request only.
    RequestOnly,
    /// Records request, and an appeal or complaint citing statute and the agency's own words.
    RequestAndAppeal,
    /// No specific gap to ask about (aggregate or inference kinds).
    NoDraft,
}

impl DraftFamily {
    pub const fn for_kind(kind: FindingKind) -> Self {
        match kind {
            FindingKind::HeldBackResponse
            | FindingKind::BoilerplateDenial
            | FindingKind::JustInTimeCompliance
            | FindingKind::ExtensionAbuse
            | FindingKind::AbsenceOfRecords
            | FindingKind::DifferentialTreatment
            | FindingKind::MetadataContradictsContent
            | FindingKind::RegeneratedResponse
            | FindingKind::EmailThreadGap
            | FindingKind::ContradictoryStatement
            | FindingKind::NoRecordsContradictedByRelease
            | FindingKind::ClaimContradictsData
            | FindingKind::PracticeDepartureCounted
            | FindingKind::PracticeDepartureStated
            | FindingKind::EncampmentNoticeShortfall
            | FindingKind::EncampmentNoticeStatedShortfall
            | FindingKind::RedactionInconsistentAcrossReleases => Self::RequestOnly,
            FindingKind::PolicyViolationDetected => Self::RequestAndAppeal,
            FindingKind::EvasionParaphrasedDenials
            | FindingKind::EvasionRoundNumberThresholds
            | FindingKind::EvasionRedactionAtLimit
            | FindingKind::EvasionLateArrivingRecords
            | FindingKind::RedactionCandidateInferred
            | FindingKind::CrossCorpusSharedLanguage
            | FindingKind::CrossCorpusTemporalShift
            | FindingKind::CrossCorpusOrphanTopic
            | FindingKind::CrossCorpusReusedDenialGrounds
            | FindingKind::CorrelatedFindings
            // Spending cases cite ledger rows, not a records-request exchange; no CPRA template fits.
            | FindingKind::ContractCeilingExceeded
            | FindingKind::PaymentOutsideContractTerm
            | FindingKind::PaymentWithoutContractRecord
            | FindingKind::AppropriationExceeded
            | FindingKind::RestrictedFundOffProgram
            | FindingKind::ApprovalThresholdExceeded
            | FindingKind::ApprovalRecordAbsent
            | FindingKind::SplitPurchasePattern
            | FindingKind::SplitPurchaseProhibitedByPolicy
            // Dataset aggregates with no portal or document to address a request to.
            | FindingKind::DepartmentSpendingYearOverYear
            | FindingKind::OvertimeAboveAdoptedLine
            | FindingKind::FundActualAboveAdopted
            | FindingKind::ServiceReportLead
            | FindingKind::PoliceStopLead
            | FindingKind::AbatementLogLead
            | FindingKind::ChargeLogLead
            | FindingKind::CapacityComparisonLead
            // A description of how a release was spread out; it identifies no gap to ask about.
            | FindingKind::RollingProduction
            // A count of when uploads were recorded; it identifies no gap in the records.
            | FindingKind::SpendingRecordLead
            | FindingKind::ReleaseTimingConcentration => Self::NoDraft,
        }
    }
}

/// Which drafts the UI may offer for a finding. Computed here so the frontend does not mirror
/// `is_legal_claim` or the template table.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DraftAvailability {
    pub followup: bool,
    pub appeal: bool,
}

impl DraftAvailability {
    pub fn for_kind(kind: FindingKind) -> Self {
        let family = DraftFamily::for_kind(kind);
        Self {
            followup: family != DraftFamily::NoDraft,
            appeal: family == DraftFamily::RequestAndAppeal,
        }
    }
}

/// A drafted artifact ready for user editing. It is always a draft: nothing here is transmitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DraftedArtifact {
    /// Deterministic id over `(kind, subject_finding_id, template_id, jurisdiction)`. A regenerate
    /// with the same inputs returns the same id; the user's saved edits are keyed on it.
    pub id: String,
    pub kind: ArtifactKind,
    pub subject_finding_id: String,
    pub jurisdiction: String,
    pub template_id: String,
    pub generated_text: String,
    /// The author's declared affiliation, kept apart from the editable body so an edit cannot
    /// remove it. Consumers append it to whatever they copy, export or save. `None` when the
    /// profile declares nothing.
    pub disclosure: Option<String>,
    pub cited_authorities: Vec<CitedAuthority>,
    pub evidence_basis: DraftEvidenceBasis,
    /// The source finding's confidence, carried unchanged. `Unassessed` is not zero.
    pub confidence: FindingConfidence,
    /// The source citations with their character ranges and full quotes, as the finding holds
    /// them. Empty when the draft rests on recorded request dates rather than documents.
    pub citations: Vec<FindingCitation>,
    /// Facts the draft needs and does not have. Each is also a bracketed placeholder in the text.
    pub unknown_facts: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

impl DraftedArtifact {
    pub(super) fn build_id(
        kind: ArtifactKind,
        subject_finding_id: &str,
        template_id: &str,
        jurisdiction: &str,
    ) -> String {
        let composite = format!(
            "kind={}\u{1f}finding={}\u{1f}template={}\u{1f}jurisdiction={}",
            kind.as_str(),
            subject_finding_id,
            template_id,
            jurisdiction,
        );
        sha256_hex(composite.as_bytes())[..20].to_string()
    }
}

/// Errors that block draft generation. Callers should surface these verbatim to the user so they
/// understand why a draft could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("No {kind:?} template is available for a finding of kind {finding_kind:?}")]
    NoApplicableTemplate {
        kind: ArtifactKind,
        finding_kind: FindingKind,
    },
    #[error("Finding is missing a required field for drafting: {0}")]
    MissingField(String),
    #[error("A {finding_kind:?} finding is a pattern observation, not a legal claim; only a records request can be drafted from it")]
    NotALegalClaim { finding_kind: FindingKind },
    #[error("No appeal can be drafted: {0}")]
    AppealNotSupported(String),
    #[error("No overdue follow-up letter can be drafted: {0}")]
    FollowUpNotAvailable(String),
}

/// Read-only helpers shared between followup and appeal draft paths.
pub(super) fn find_agency_label(finding: &Finding) -> String {
    finding
        .subject
        .agency_id
        .map(|id| format!("[AGENCY NAME] (Agency ID {id})"))
        .unwrap_or_else(|| "[AGENCY NAME]".to_string())
}

pub(super) fn find_request_label(finding: &Finding) -> String {
    finding
        .subject
        .request_id
        .map(|id| format!("Request #{id}"))
        .unwrap_or_else(|| "[YOUR REQUEST NUMBER]".to_string())
}

pub(super) fn today_iso(now: DateTime<Utc>) -> String {
    now.format("%Y-%m-%d").to_string()
}

const QUOTE_DISPLAY_CHARS: usize = 400;

/// One bullet per citation: document, character range, quote. A range or quote the finding does
/// not carry is said to be unrecorded rather than omitted, and a quote cut for length is marked.
fn citation_line(cite: &FindingCitation) -> String {
    let mut line = format!("- document {} on {}", cite.document_id, cite.portal_key);
    if let Some(label) = cite.label.as_ref() {
        line.push_str(&format!(" ({label})"));
    }
    match (cite.character_start, cite.character_end) {
        (Some(start), Some(end)) => line.push_str(&format!(", characters {start}-{end}")),
        _ => line.push_str(", character range not recorded"),
    }
    if let Some(quote) = cite
        .quote
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
    {
        let capped: String = quote.chars().take(QUOTE_DISPLAY_CHARS).collect();
        let truncated = capped.chars().count() < quote.chars().count();
        line.push_str(&format!(": \"{capped}\""));
        if truncated {
            line.push_str(" [quote truncated; see the cited document for the full text]");
        }
    }
    line
}

/// The finding's citations as a bulletable list, or a placeholder when it carries none (Wave 1
/// requires at least one, but a stored row may have lost them).
pub(super) fn evidence_bullets(finding: &Finding) -> Vec<String> {
    if finding.citations.is_empty() {
        return vec!["- [ADD SPECIFIC EVIDENCE FROM YOUR RECORDS]".to_string()];
    }
    finding.citations.iter().map(citation_line).collect()
}

fn confidence_statement(confidence: FindingConfidence) -> String {
    match confidence {
        FindingConfidence::Unassessed => {
            "Confidence of the underlying observation: not assessed.".to_string()
        }
        FindingConfidence::Measured { basis_points } => format!(
            "Confidence of the underlying observation: {}.{:02} percent (measured).",
            basis_points / 100,
            basis_points % 100
        ),
    }
}

/// Everything the draft needs that the finding does not carry. The sender's own identity is
/// always unknown to the app.
fn unknown_facts(finding: &Finding) -> Vec<String> {
    // A finding carries an internal agency id at most, never the agency's name.
    let mut unknown = vec![
        "Your name and contact details".to_string(),
        "Agency name".to_string(),
    ];
    if finding.subject.request_id.is_none() {
        unknown.push("Your request number".to_string());
    }
    for cite in &finding.citations {
        if cite.character_start.is_none() || cite.character_end.is_none() {
            unknown.push(format!(
                "Character range within document {}",
                cite.document_id
            ));
        }
    }
    if finding.citations.is_empty() {
        unknown.push("Specific evidence".to_string());
    }
    unknown.dedup();
    unknown
}

/// The one generator behind both follow-up and appeal drafts: fills the template for
/// `(kind, jurisdiction, finding.kind)` from the finding alone.
pub(super) fn render_from_template(
    kind: ArtifactKind,
    finding: &Finding,
    jurisdiction: &str,
    basis: DraftEvidenceBasis,
    disclosure: &DisclosureContext,
) -> Result<DraftedArtifact, ArtifactError> {
    let template = templates::find_template(kind, jurisdiction, finding.kind).ok_or(
        ArtifactError::NoApplicableTemplate {
            kind,
            finding_kind: finding.kind,
        },
    )?;
    let now = Utc::now();
    let evidence = format!(
        "{}\n\n{} {}",
        evidence_bullets(finding).join("\n"),
        basis.statement(),
        confidence_statement(finding.confidence)
    );
    let filled = template
        .body
        .replace("{TODAY}", &today_iso(now))
        .replace("{AGENCY_LABEL}", &find_agency_label(finding))
        .replace("{REQUEST_LABEL}", &find_request_label(finding))
        .replace("{FINDING_HEADLINE}", &finding.headline)
        .replace("{FINDING_EXPLANATION}", &finding.explanation)
        .replace("{EVIDENCE_BULLETS}", &evidence)
        .replace(
            "{AUTHORITY_CITATIONS}",
            &templates::render_authority_line(template),
        );
    Ok(DraftedArtifact {
        id: DraftedArtifact::build_id(kind, &finding.id, template.id, jurisdiction),
        kind,
        subject_finding_id: finding.id.clone(),
        jurisdiction: jurisdiction.to_string(),
        template_id: template.id.to_string(),
        generated_text: format!("Subject: {}\n\n{filled}", template.subject),
        disclosure: disclosure.text(&finding.subject),
        cited_authorities: template
            .cited_authorities
            .iter()
            .map(|authority| authority.to_owned_authority())
            .collect(),
        evidence_basis: basis,
        confidence: finding.confidence,
        citations: finding.citations.clone(),
        unknown_facts: unknown_facts(finding),
        generated_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::finding::{FindingSeverity, FindingSubject};

    pub(crate) fn all_kinds() -> [FindingKind; 27] {
        use FindingKind::*;
        [
            HeldBackResponse,
            BoilerplateDenial,
            JustInTimeCompliance,
            ExtensionAbuse,
            AbsenceOfRecords,
            DifferentialTreatment,
            MetadataContradictsContent,
            RegeneratedResponse,
            EmailThreadGap,
            ContradictoryStatement,
            NoRecordsContradictedByRelease,
            EvasionParaphrasedDenials,
            EvasionRoundNumberThresholds,
            EvasionRedactionAtLimit,
            EvasionLateArrivingRecords,
            RedactionCandidateInferred,
            CrossCorpusSharedLanguage,
            CrossCorpusTemporalShift,
            CrossCorpusOrphanTopic,
            CrossCorpusReusedDenialGrounds,
            PolicyViolationDetected,
            ClaimContradictsData,
            EncampmentNoticeShortfall,
            EncampmentNoticeStatedShortfall,
            RedactionInconsistentAcrossReleases,
            RollingProduction,
            ReleaseTimingConcentration,
            SpendingRecordLead,
        ]
    }

    #[test]
    fn only_legal_claim_kinds_are_offered_an_appeal() {
        for kind in all_kinds() {
            let availability = DraftAvailability::for_kind(kind);
            if availability.appeal {
                assert!(
                    kind.is_legal_claim(),
                    "{kind:?} offers an appeal but is not a legal claim"
                );
            }
            if kind.is_legal_claim() {
                assert!(availability.followup, "{kind:?} lost its narrowing request");
            }
        }
        assert!(DraftAvailability::for_kind(FindingKind::PolicyViolationDetected).appeal);
    }

    #[test]
    fn a_range_less_or_truncated_citation_says_so_and_a_full_one_prints_its_range() {
        let mut cite = sample_finding().citations.remove(0);
        assert!(citation_line(&cite).contains("character range not recorded"));
        cite.character_start = Some(10);
        cite.character_end = Some(30);
        assert!(citation_line(&cite).contains("characters 10-30"));
        cite.quote = Some("x".repeat(QUOTE_DISPLAY_CHARS + 1));
        assert!(citation_line(&cite).contains("quote truncated"));
        cite.quote = Some("x".repeat(QUOTE_DISPLAY_CHARS));
        assert!(!citation_line(&cite).contains("quote truncated"));
    }

    #[test]
    fn unassessed_confidence_is_never_rendered_as_a_number() {
        let text = confidence_statement(FindingConfidence::Unassessed);
        assert!(text.contains("not assessed"));
        assert!(!text.chars().any(|c| c.is_ascii_digit()));
        assert!(confidence_statement(FindingConfidence::measured(0)).contains("0.00 percent"));
    }

    fn sample_finding() -> Finding {
        let now = Utc::now();
        Finding {
            id: "abc123".into(),
            kind: FindingKind::HeldBackResponse,
            severity: FindingSeverity::Serious,
            confidence: FindingConfidence::measured(7500),
            subject: FindingSubject {
                portal_key: Some("test-portal".into()),
                agency_id: Some(7),
                request_id: Some(42),
                document_id: Some(101),
                entity_id: None,
            },
            headline: "Response held back 12 days".into(),
            explanation: "PDF metadata shows ModDate 12 days before release.".into(),
            citations: vec![FindingCitation {
                portal_key: "test-portal".into(),
                document_id: 101,
                character_start: None,
                character_end: None,
                quote: Some("ModDate 2025-01-01".into()),
                label: Some("PDF metadata".into()),
            }],
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: "held_back_response".into(),
        }
    }

    #[test]
    fn build_id_is_deterministic_for_equivalent_inputs() {
        let one = DraftedArtifact::build_id(
            ArtifactKind::FollowupRequest,
            "abc123",
            "cpra_followup_v1",
            "US-CA",
        );
        let two = DraftedArtifact::build_id(
            ArtifactKind::FollowupRequest,
            "abc123",
            "cpra_followup_v1",
            "US-CA",
        );
        assert_eq!(one, two);
        assert_eq!(one.len(), 20);
    }

    #[test]
    fn build_id_changes_when_kind_changes() {
        let followup =
            DraftedArtifact::build_id(ArtifactKind::FollowupRequest, "abc123", "cpra_v1", "US-CA");
        let appeal = DraftedArtifact::build_id(ArtifactKind::Appeal, "abc123", "cpra_v1", "US-CA");
        assert_ne!(followup, appeal);
    }

    #[test]
    fn evidence_bullets_uses_citations_and_never_fabricates() {
        let finding = sample_finding();
        let bullets = evidence_bullets(&finding);
        assert_eq!(bullets.len(), 1);
        let line = &bullets[0];
        assert!(line.contains("document 101 on test-portal"));
        assert!(line.contains("PDF metadata"));
        assert!(line.contains("ModDate 2025-01-01"));
    }

    #[test]
    fn evidence_bullets_placeholder_when_no_citations() {
        let mut finding = sample_finding();
        finding.citations.clear();
        let bullets = evidence_bullets(&finding);
        assert_eq!(bullets.len(), 1);
        assert!(bullets[0].contains("[ADD SPECIFIC EVIDENCE"));
    }

    #[test]
    fn agency_and_request_labels_use_placeholder_when_absent() {
        // An id alone never stands in for the agency's name.
        assert_eq!(
            find_agency_label(&sample_finding()),
            "[AGENCY NAME] (Agency ID 7)"
        );
        let mut finding = sample_finding();
        finding.subject.agency_id = None;
        finding.subject.request_id = None;
        assert_eq!(find_agency_label(&finding), "[AGENCY NAME]");
        assert_eq!(find_request_label(&finding), "[YOUR REQUEST NUMBER]");
    }
}
