//! Drafts an appeal or complaint from a [`Finding`] whose kind is a legal claim.
//!
//! A pattern or statistical finding never reaches a template: the guard here rejects it before
//! lookup, so a later edit to the template table cannot reintroduce an accusation the evidence
//! does not support. A request whose statutory deadline the evaluator counts as exceeded is
//! drafted by [`super::statutory_appeal`], not from a finding.

use super::{
    render_from_template, ArtifactError, ArtifactKind, DraftEvidenceBasis, DraftedArtifact,
};
use crate::analysis::finding::{Finding, FindingKind};
use crate::profile::DisclosureContext;

/// Builds an appeal for the given finding. `NotALegalClaim` for any pattern kind; the finding must
/// also satisfy the write-time legal-claim guard and, for a policy comparison, quote the
/// commitment it compares against.
pub fn draft_appeal(
    finding: &Finding,
    jurisdiction: &str,
    disclosure: &DisclosureContext,
) -> Result<DraftedArtifact, ArtifactError> {
    if !finding.kind.is_legal_claim() {
        return Err(ArtifactError::NotALegalClaim {
            finding_kind: finding.kind,
        });
    }
    finding
        .check_recordable()
        .map_err(ArtifactError::AppealNotSupported)?;
    let basis = match finding.kind {
        FindingKind::PolicyViolationDetected => {
            let quotes_commitment = finding.citations.iter().any(|cite| {
                cite.quote
                    .as_deref()
                    .is_some_and(|quote| !quote.trim().is_empty())
            });
            if !quotes_commitment {
                return Err(ArtifactError::MissingField(
                    "quoted text of the agency's stated commitment".to_string(),
                ));
            }
            DraftEvidenceBasis::CountedPolicyComparison
        }
        other => {
            return Err(ArtifactError::NoApplicableTemplate {
                kind: ArtifactKind::Appeal,
                finding_kind: other,
            })
        }
    };
    render_from_template(
        ArtifactKind::Appeal,
        finding,
        jurisdiction,
        basis,
        disclosure,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::artifacts::tests::all_kinds;
    use crate::analysis::finding::{
        FindingCitation, FindingConfidence, FindingSeverity, FindingSubject,
    };
    use chrono::Utc;

    fn finding_of(kind: FindingKind, quote: Option<&str>) -> Finding {
        let now = Utc::now();
        Finding {
            id: "policy-finding".into(),
            kind,
            severity: FindingSeverity::Serious,
            confidence: FindingConfidence::measured(8000),
            subject: FindingSubject {
                portal_key: Some("sd".into()),
                agency_id: Some(9),
                request_id: Some(2),
                document_id: Some(100),
                entity_id: None,
            },
            headline: "3 of 8 requests exceeded the 10-day response time the agency's own policy states".into(),
            explanation: "'policy.pdf' states a response within 10 calendar days. Of 8 requests, 3 took longer.".into(),
            citations: vec![FindingCitation {
                portal_key: "sd".into(),
                document_id: 100,
                character_start: Some(12),
                character_end: Some(52),
                quote: quote.map(str::to_string),
                label: Some("Stated commitment".into()),
            }],
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: "policy_violation".into(),
        }
    }

    #[test]
    fn a_pattern_kind_can_never_yield_an_appeal() {
        for kind in all_kinds()
            .into_iter()
            .filter(|kind| !kind.is_legal_claim())
        {
            let result = draft_appeal(
                &finding_of(kind, Some("quote")),
                "US-CA",
                &DisclosureContext::unset(),
            );
            assert!(
                matches!(result, Err(ArtifactError::NotALegalClaim { finding_kind }) if finding_kind == kind),
                "{kind:?} produced an appeal draft"
            );
        }
    }

    #[test]
    fn a_policy_appeal_quotes_the_commitment_with_its_range_and_the_counts_and_claims_no_statutory_breach(
    ) {
        let quote = "Response will be provided within 10 calendar days";
        let artifact = draft_appeal(
            &finding_of(FindingKind::PolicyViolationDetected, Some(quote)),
            "US-CA",
            &DisclosureContext::unset(),
        )
        .unwrap();
        let text = &artifact.generated_text;
        assert!(text.contains(&format!("\"{quote}\"")));
        assert!(text.contains("characters 12-52"));
        assert!(text.contains("3 of 8 requests exceeded"));
        assert!(text.contains("7922.535"));
        assert!(text.contains("does not assert that the statutory period was exceeded"));
        assert!(!text.contains('{'));
        assert_eq!(
            artifact.evidence_basis,
            DraftEvidenceBasis::CountedPolicyComparison
        );
        assert_eq!(artifact.kind, ArtifactKind::Appeal);
        assert_eq!(artifact.citations.len(), 1);
        assert!(artifact
            .unknown_facts
            .iter()
            .any(|fact| fact == "Agency name"));
    }

    #[test]
    fn a_policy_appeal_without_a_quoted_commitment_is_refused_not_paraphrased() {
        for quote in [None, Some("   ")] {
            let result = draft_appeal(
                &finding_of(FindingKind::PolicyViolationDetected, quote),
                "US-CA",
                &DisclosureContext::unset(),
            );
            assert!(matches!(result, Err(ArtifactError::MissingField(_))));
        }
    }

    #[test]
    fn a_legal_finding_failing_the_write_guard_is_refused() {
        let mut finding = finding_of(FindingKind::PolicyViolationDetected, Some("q"));
        finding.confidence = FindingConfidence::Unassessed;
        assert!(matches!(
            draft_appeal(&finding, "US-CA", &DisclosureContext::unset()),
            Err(ArtifactError::AppealNotSupported(_))
        ));
    }

    #[test]
    fn appeal_id_is_stable_across_regeneration() {
        let finding = finding_of(FindingKind::PolicyViolationDetected, Some("q"));
        assert_eq!(
            draft_appeal(&finding, "US-CA", &DisclosureContext::unset())
                .unwrap()
                .id,
            draft_appeal(&finding, "US-CA", &DisclosureContext::unset())
                .unwrap()
                .id
        );
    }
}
