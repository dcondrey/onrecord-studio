//! Drafts a records-request follow-up from a [`Finding`].
//!
//! Every specific fact in the output traces to `finding.subject` or `finding.citations`. When a
//! required fact is missing, a `[BRACKETED PLACEHOLDER]` is emitted for the user to fill in —
//! never a guess.

use super::{
    render_from_template, ArtifactError, ArtifactKind, DraftEvidenceBasis, DraftedArtifact,
};
use crate::analysis::finding::{Finding, FindingKind};
use crate::profile::DisclosureContext;

/// Builds a narrowing records request for the given finding, in the requested jurisdiction (e.g.
/// `"US-CA"` for CPRA). Returns `NoApplicableTemplate` when no template covers the finding's kind
/// in that jurisdiction.
pub fn draft_followup(
    finding: &Finding,
    jurisdiction: &str,
    disclosure: &DisclosureContext,
) -> Result<DraftedArtifact, ArtifactError> {
    let basis = match finding.kind {
        FindingKind::PolicyViolationDetected => DraftEvidenceBasis::CountedPolicyComparison,
        _ => DraftEvidenceBasis::PatternObservation,
    };
    render_from_template(
        ArtifactKind::FollowupRequest,
        finding,
        jurisdiction,
        basis,
        disclosure,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::artifacts::{tests::all_kinds, DraftFamily};
    use crate::analysis::finding::{
        Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
    };
    use chrono::Utc;

    fn finding_with(
        kind: FindingKind,
        agency_id: Option<i64>,
        request_id: Option<i64>,
        headline: &str,
        explanation: &str,
        citation_quote: Option<&str>,
    ) -> Finding {
        let now = Utc::now();
        let citations = match citation_quote {
            None => vec![],
            Some(quote) => vec![FindingCitation {
                portal_key: "test-portal".into(),
                document_id: 101,
                character_start: Some(0),
                character_end: Some(20),
                quote: Some(quote.into()),
                label: Some("cited quote".into()),
            }],
        };
        Finding {
            id: "finding-xyz".into(),
            kind,
            severity: FindingSeverity::Serious,
            confidence: FindingConfidence::measured(7500),
            subject: FindingSubject {
                portal_key: Some("test-portal".into()),
                agency_id,
                request_id,
                document_id: Some(101),
                entity_id: None,
            },
            headline: headline.into(),
            explanation: explanation.into(),
            citations,
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: "test".into(),
        }
    }

    #[test]
    fn followup_for_absence_uses_the_specific_template_and_includes_finding_facts() {
        let finding = finding_with(
            FindingKind::AbsenceOfRecords,
            Some(7),
            Some(42),
            "Invoice sequence 001-047 missing item 023",
            "Released set includes 46 of 47 sequential invoice numbers; #023 absent.",
            Some("Invoice-2024-024 (following #023)"),
        );
        let artifact = draft_followup(&finding, "US-CA", &DisclosureContext::unset())
            .expect("template exists");
        assert_eq!(artifact.template_id, "cpra_followup_absence_v1");
        assert_eq!(artifact.kind, ArtifactKind::FollowupRequest);
        assert!(artifact
            .generated_text
            .contains("Invoice sequence 001-047 missing item 023"));
        assert!(artifact
            .generated_text
            .contains("Released set includes 46 of 47"));
        assert!(artifact.generated_text.contains("Invoice-2024-024"));
        assert!(artifact.generated_text.contains("Agency ID 7"));
        assert_eq!(
            artifact.evidence_basis,
            DraftEvidenceBasis::PatternObservation
        );
        assert_eq!(artifact.citations, finding.citations);
        assert!(artifact.generated_text.contains("Request #42"));
        assert!(artifact.generated_text.contains("§ 7920.000"));
    }

    #[test]
    fn followup_omits_agency_name_when_missing_and_substitutes_placeholder_never_a_guess() {
        let finding = finding_with(
            FindingKind::HeldBackResponse,
            None, // no agency
            None, // no request id
            "Response held back 12 days",
            "PDF metadata shows ModDate 12 days before release.",
            Some("ModDate: 2024-03-01"),
        );
        let artifact = draft_followup(&finding, "US-CA", &DisclosureContext::unset())
            .expect("template exists");
        assert!(
            artifact.generated_text.contains("[AGENCY NAME]"),
            "should carry the placeholder, not fabricate an agency"
        );
        assert!(
            artifact.generated_text.contains("[YOUR REQUEST NUMBER]"),
            "should carry the placeholder, not fabricate a request id"
        );
        // But the placeholder text for the SENDER's name is also always literal, since we never
        // know it.
        assert!(artifact.generated_text.contains("[YOUR NAME]"));
    }

    #[test]
    fn followup_returns_no_template_error_for_unsupported_jurisdiction() {
        let finding = finding_with(
            FindingKind::HeldBackResponse,
            Some(7),
            Some(42),
            "held",
            "explanation",
            None,
        );
        let result = draft_followup(&finding, "EU-DE", &DisclosureContext::unset());
        assert!(matches!(
            result,
            Err(ArtifactError::NoApplicableTemplate {
                kind: ArtifactKind::FollowupRequest,
                ..
            })
        ));
    }

    #[test]
    fn followup_id_is_stable_across_regenerations_of_the_same_finding() {
        let finding = finding_with(
            FindingKind::HeldBackResponse,
            Some(7),
            Some(42),
            "held 12 days",
            "explanation",
            Some("q"),
        );
        let a = draft_followup(&finding, "US-CA", &DisclosureContext::unset()).unwrap();
        let b = draft_followup(&finding, "US-CA", &DisclosureContext::unset()).unwrap();
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn followup_body_never_carries_an_unfilled_placeholder_marker() {
        // A missed placeholder is a bug: it means a `{PLACEHOLDER}` shipped verbatim to the user.
        // We use "{" as a smoke check — CPRA statute cites use "§" not "{", so any "{...}"
        // remnant is a marker the template.rs authors forgot to replace or the drafter forgot to
        // fill.
        let finding = finding_with(
            FindingKind::ExtensionAbuse,
            Some(3),
            Some(19),
            "Systemic reliance on volume ground",
            "Agency invoked VoluminousRecords on 92% of extensions over the past year.",
            Some("statutory_extensions.reason=VoluminousRecords"),
        );
        let artifact = draft_followup(&finding, "US-CA", &DisclosureContext::unset()).unwrap();
        assert!(
            !artifact.generated_text.contains('{'),
            "unfilled template placeholder remnant in body: {}",
            artifact.generated_text
        );
    }

    #[test]
    fn followup_cites_the_specific_statutes_from_the_template_registry() {
        let finding = finding_with(
            FindingKind::AbsenceOfRecords,
            Some(7),
            Some(42),
            "missing invoice items",
            "gap in the sequence",
            Some("Invoice-2024-024"),
        );
        let artifact = draft_followup(&finding, "US-CA", &DisclosureContext::unset()).unwrap();
        // Cited authorities are the structured field, not just body text.
        assert!(artifact
            .cited_authorities
            .iter()
            .any(|a| a.statute.contains("§ 7920.000")));
        assert!(artifact
            .cited_authorities
            .iter()
            .any(|a| a.statute.contains("§ 7922.525")));
    }

    #[test]
    fn a_pattern_followup_asks_for_the_gap_and_never_accuses() {
        for kind in all_kinds()
            .into_iter()
            .filter(|kind| !kind.is_legal_claim())
            .filter(|kind| DraftFamily::for_kind(*kind) != DraftFamily::NoDraft)
        {
            let finding = finding_with(
                kind,
                Some(7),
                Some(42),
                "neutral headline",
                "neutral explanation",
                Some("q"),
            );
            let text = draft_followup(&finding, "US-CA", &DisclosureContext::unset())
                .unwrap()
                .generated_text
                // The basis line disclaims any violation; the one sentence allowed to say so.
                .replace(DraftEvidenceBasis::PatternObservation.statement(), "")
                .to_lowercase();
            for word in [
                "violat",
                "unlawful",
                "illegal",
                "misconduct",
                "bad faith",
                "fraud",
            ] {
                assert!(!text.contains(word), "{kind:?} draft contains '{word}'");
            }
        }
    }

    #[test]
    fn the_email_gap_request_carries_the_message_id_the_finding_names_and_nothing_else() {
        let mut finding = finding_with(
            FindingKind::EmailThreadGap,
            Some(7),
            Some(42),
            "Released email references unreleased Message-ID <abc@x.gov>",
            "\"a.msg\" names <abc@x.gov> through its References header.",
            None,
        );
        finding.citations = vec![FindingCitation {
            portal_key: "test-portal".into(),
            document_id: 5,
            character_start: None,
            character_end: None,
            quote: None,
            label: Some("a.msg references <abc@x.gov>".into()),
        }];
        let artifact = draft_followup(&finding, "US-CA", &DisclosureContext::unset()).unwrap();
        assert_eq!(artifact.template_id, "cpra_followup_email_gap_v1");
        assert!(artifact.generated_text.contains("<abc@x.gov>"));
        assert!(artifact
            .generated_text
            .contains("character range not recorded"));
        assert!(artifact
            .unknown_facts
            .iter()
            .any(|fact| fact.contains("document 5")));
        let with_id = artifact.generated_text.matches('<').count();
        assert_eq!(
            with_id,
            artifact.generated_text.matches("<abc@x.gov>").count()
        );
    }

    #[test]
    fn a_policy_finding_still_gets_a_records_request_but_not_from_a_pattern_basis() {
        let finding = finding_with(
            FindingKind::PolicyViolationDetected,
            Some(7),
            Some(42),
            "h",
            "e",
            Some("q"),
        );
        let artifact = draft_followup(&finding, "US-CA", &DisclosureContext::unset()).unwrap();
        assert_eq!(
            artifact.evidence_basis,
            DraftEvidenceBasis::CountedPolicyComparison
        );
    }

    #[test]
    fn a_declared_affiliation_is_in_every_draft_and_an_own_subject_is_named() {
        use crate::profile::matching::ConfirmedSubjects;
        use crate::profile::{build_disclosure, DisclosureContext};
        use crate::types::profile::{
            Affiliation, AffiliationBasis, UserProfile, PROFILE_SCHEMA_VERSION,
        };
        let profile = UserProfile {
            schema_version: PROFILE_SCHEMA_VERSION,
            display_name: None,
            affiliations: vec![Affiliation {
                id: "a".into(),
                name: "City Clerk".into(),
                aliases: vec![],
                basis: AffiliationBasis::SelfDeclared,
                created_at: "2026-01-01T00:00:00Z".into(),
                verified_domain: None,
            }],
            match_decisions: vec![],
            verified_domains: vec![],
            updated_at: "2026-01-01T00:00:00Z".into(),
        };
        let finding = finding_with(
            FindingKind::AbsenceOfRecords,
            Some(7),
            Some(42),
            "h",
            "e",
            Some("q"),
        );
        let plain = draft_followup(&finding, "US-CA", &DisclosureContext::unset()).unwrap();
        assert_eq!(plain.disclosure, None);

        let elsewhere = DisclosureContext::from_disclosure(&build_disclosure(
            Some(&profile),
            ConfirmedSubjects::default(),
        ));
        let drafted = draft_followup(&finding, "US-CA", &elsewhere).unwrap();
        let text = drafted
            .disclosure
            .expect("declared affiliation is disclosed");
        assert_eq!(
            text,
            "Prepared by a user who has declared an affiliation with City Clerk (self-declared, not verified)."
        );
        assert!(!drafted.generated_text.contains("declared an affiliation"));

        let own = DisclosureContext::from_disclosure(&build_disclosure(
            Some(&profile),
            ConfirmedSubjects {
                portal_keys: ["test-portal".to_string()].into(),
                ..Default::default()
            },
        ));
        let text = draft_followup(&finding, "US-CA", &own)
            .unwrap()
            .disclosure
            .unwrap();
        assert!(
            text.contains("subject of this document is an organization the author has declared")
        );
    }
}
