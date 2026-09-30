//! Drafts a records-request follow-up from a [`Finding`].
//!
//! Every specific fact in the output traces to `finding.subject` or `finding.citations`. When a
//! required fact is missing, a `[BRACKETED PLACEHOLDER]` is emitted for the user to fill in —
//! never a guess.

use super::{
    evidence_bullets, find_agency_label, find_request_label,
    templates::{find_template, render_authority_line},
    today_iso, ArtifactError, ArtifactKind, DraftedArtifact,
};
use crate::analysis::finding::Finding;
use chrono::Utc;

/// Builds a follow-up records request for the given finding, in the requested jurisdiction (e.g.
/// `"US-CA"` for CPRA). Returns `NoApplicableTemplate` when no template covers the finding's kind
/// in that jurisdiction.
pub fn draft_followup(
    finding: &Finding,
    jurisdiction: &str,
) -> Result<DraftedArtifact, ArtifactError> {
    let template = find_template(ArtifactKind::FollowupRequest, jurisdiction, finding.kind).ok_or(
        ArtifactError::NoApplicableTemplate {
            kind: ArtifactKind::FollowupRequest,
            finding_kind: finding.kind,
        },
    )?;
    let now = Utc::now();
    let bullets = evidence_bullets(finding).join("\n");
    let authority_line = render_authority_line(template);
    let filled = template
        .body
        .replace("{TODAY}", &today_iso(now))
        .replace("{AGENCY_LABEL}", &find_agency_label(finding))
        .replace("{REQUEST_LABEL}", &find_request_label(finding))
        .replace("{FINDING_HEADLINE}", &finding.headline)
        .replace("{FINDING_EXPLANATION}", &finding.explanation)
        .replace("{EVIDENCE_BULLETS}", &bullets)
        .replace("{AUTHORITY_CITATIONS}", &authority_line);
    let id = DraftedArtifact::build_id(
        ArtifactKind::FollowupRequest,
        &finding.id,
        template.id,
        jurisdiction,
    );
    let cited_authorities = template
        .cited_authorities
        .iter()
        .map(|authority| authority.to_owned_authority())
        .collect();
    let with_subject_header = format!("Subject: {}\n\n{filled}", template.subject);
    Ok(DraftedArtifact {
        id,
        kind: ArtifactKind::FollowupRequest,
        subject_finding_id: finding.id.clone(),
        jurisdiction: jurisdiction.to_string(),
        template_id: template.id.to_string(),
        generated_text: with_subject_header,
        cited_authorities,
        generated_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::finding::{
        Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
    };

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
        let artifact = draft_followup(&finding, "US-CA").expect("template exists");
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
        let artifact = draft_followup(&finding, "US-CA").expect("template exists");
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
        let result = draft_followup(&finding, "EU-DE");
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
        let a = draft_followup(&finding, "US-CA").unwrap();
        let b = draft_followup(&finding, "US-CA").unwrap();
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
        let artifact = draft_followup(&finding, "US-CA").unwrap();
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
        let artifact = draft_followup(&finding, "US-CA").unwrap();
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
}
