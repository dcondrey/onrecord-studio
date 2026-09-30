//! Drafts an appeal from a [`Finding`] whose kind supports one.
//!
//! Only a small subset of findings map naturally to an appeal today (`NoRecordsContradictedByRelease`,
//! `BoilerplateDenial`, `ExtensionAbuse`). For other kinds, `draft_appeal` returns
//! `NoApplicableTemplate` — the caller (usually the frontend) surfaces that to the user rather
//! than fabricating an appeal that doesn't rest on statute-relevant facts.

use super::{
    evidence_bullets, find_agency_label, find_request_label,
    templates::{find_template, render_authority_line},
    today_iso, ArtifactError, ArtifactKind, DraftedArtifact,
};
use crate::analysis::finding::Finding;
use chrono::Utc;

/// Builds an appeal for the given finding, in the requested jurisdiction. Returns
/// `NoApplicableTemplate` when the finding's kind has no appeal template in that jurisdiction.
pub fn draft_appeal(
    finding: &Finding,
    jurisdiction: &str,
) -> Result<DraftedArtifact, ArtifactError> {
    let template = find_template(ArtifactKind::Appeal, jurisdiction, finding.kind).ok_or(
        ArtifactError::NoApplicableTemplate {
            kind: ArtifactKind::Appeal,
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
    let id =
        DraftedArtifact::build_id(ArtifactKind::Appeal, &finding.id, template.id, jurisdiction);
    let cited_authorities = template
        .cited_authorities
        .iter()
        .map(|authority| authority.to_owned_authority())
        .collect();
    let with_subject_header = format!("Subject: {}\n\n{filled}", template.subject);
    Ok(DraftedArtifact {
        id,
        kind: ArtifactKind::Appeal,
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
        citations: Vec<FindingCitation>,
    ) -> Finding {
        let now = Utc::now();
        Finding {
            id: "finding-appeal-xyz".into(),
            kind,
            severity: FindingSeverity::Serious,
            confidence: FindingConfidence::measured(8000),
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

    fn cite(document_id: i64, quote: &str) -> FindingCitation {
        FindingCitation {
            portal_key: "test-portal".into(),
            document_id,
            character_start: None,
            character_end: None,
            quote: Some(quote.into()),
            label: None,
        }
    }

    #[test]
    fn appeal_for_no_records_carries_the_denial_and_the_released_contradicting_documents() {
        let finding = finding_with(
            FindingKind::NoRecordsContradictedByRelease,
            Some(7),
            Some(42),
            "Denial says no responsive records; 4 other requests received records on the topic",
            "Response to request 42 stated no responsive records exist, but documents 55, 56, 57, and 58 released to other requesters describe the same topic.",
            vec![
                cite(55, "records regarding TOPIC"),
                cite(56, "TOPIC contract addendum"),
                cite(57, "TOPIC internal memorandum"),
                cite(58, "TOPIC policy statement"),
            ],
        );
        let artifact = draft_appeal(&finding, "US-CA").expect("no-records appeal template exists");
        assert_eq!(artifact.template_id, "cpra_appeal_no_records_v1");
        assert_eq!(artifact.kind, ArtifactKind::Appeal);
        // Every specific fact must appear verbatim in the draft, sourced from the finding.
        assert!(artifact.generated_text.contains("Agency ID 7"));
        assert!(artifact.generated_text.contains("Request #42"));
        for doc_id in [55, 56, 57, 58] {
            assert!(
                artifact
                    .generated_text
                    .contains(&format!("document {doc_id} on test-portal")),
                "missing citation to document {doc_id}"
            );
        }
        assert!(artifact.generated_text.contains("§ 7922.525"));
    }

    #[test]
    fn appeal_returns_no_template_for_a_kind_without_an_appeal_template() {
        // JustInTimeCompliance has a follow-up but no appeal template.
        let finding = finding_with(
            FindingKind::JustInTimeCompliance,
            Some(1),
            Some(1),
            "just in time",
            "explanation",
            vec![cite(1, "quote")],
        );
        let result = draft_appeal(&finding, "US-CA");
        assert!(matches!(
            result,
            Err(ArtifactError::NoApplicableTemplate {
                kind: ArtifactKind::Appeal,
                finding_kind: FindingKind::JustInTimeCompliance,
            })
        ));
    }

    #[test]
    fn appeal_never_carries_an_unfilled_placeholder_marker() {
        let finding = finding_with(
            FindingKind::BoilerplateDenial,
            Some(3),
            Some(19),
            "Same denial language across 12 unrelated requests",
            "Cluster of 12 near-identical denials spanning 12 distinct request topics.",
            vec![cite(200, "we cannot locate")],
        );
        let artifact = draft_appeal(&finding, "US-CA").unwrap();
        assert!(
            !artifact.generated_text.contains('{'),
            "unfilled template placeholder remnant: {}",
            artifact.generated_text
        );
    }

    #[test]
    fn appeal_id_is_stable_across_regeneration() {
        let finding = finding_with(
            FindingKind::NoRecordsContradictedByRelease,
            Some(7),
            Some(42),
            "denial contradicted",
            "explanation",
            vec![cite(1, "q")],
        );
        let a = draft_appeal(&finding, "US-CA").unwrap();
        let b = draft_appeal(&finding, "US-CA").unwrap();
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn appeal_cites_written_justification_statute_when_boilerplate_pattern() {
        let finding = finding_with(
            FindingKind::BoilerplateDenial,
            Some(3),
            Some(19),
            "boilerplate",
            "explanation",
            vec![cite(1, "language")],
        );
        let artifact = draft_appeal(&finding, "US-CA").unwrap();
        assert!(
            artifact
                .cited_authorities
                .iter()
                .any(|a| a.statute.contains("§ 7922.540")),
            "boilerplate appeal must cite the written-justification statute"
        );
        assert!(
            artifact
                .cited_authorities
                .iter()
                .any(|a| a.statute.contains("§ 7922.525(b)")),
            "boilerplate appeal must cite the segregability statute"
        );
    }
}
