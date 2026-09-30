//! Actionable artifacts drafted from an autonomous [`Finding`].
//!
//! The findings inbox (Wave 1) surfaces observations. This module turns those observations into
//! drafts a user can send: a follow-up records request that would test or close the gap, or an
//! appeal that would challenge a denial the finding shows contradicted evidence. Every specific
//! claim in a draft traces back to the source finding's citations or subject — nothing is invented.
//! When the finding does not carry a specific fact the template needs, the draft emits a
//! `[BRACKETED PLACEHOLDER]` for the user to fill rather than guessing.
//!
//! Templates live in [`templates`] as a plain data table so a new jurisdiction can be added by
//! extending the table, not by editing generator code.
//!
//! **What this module will not do:** send anything. It produces drafts. Delivery is the user's,
//! not the app's.

pub mod appeal;
pub mod followup_request;
pub mod templates;

use crate::{
    analysis::finding::{Finding, FindingKind},
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

/// A drafted artifact ready for user editing.
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
    pub cited_authorities: Vec<CitedAuthority>,
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
}

/// Read-only helpers shared between followup and appeal draft paths.
pub(super) fn find_agency_label(finding: &Finding) -> String {
    finding
        .subject
        .agency_id
        .map(|id| format!("Agency ID {id}"))
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

/// Iterates the finding's citations to produce a bulletable evidence list. Falls back to a single
/// placeholder line when the finding carries none (which would be surprising — Wave 1 requires
/// every finding to have at least one citation — but the generator does not assume that
/// invariant in case a future stored row lost its citations).
pub(super) fn evidence_bullets(finding: &Finding) -> Vec<String> {
    if finding.citations.is_empty() {
        return vec!["- [ADD SPECIFIC EVIDENCE FROM YOUR RECORDS]".to_string()];
    }
    finding
        .citations
        .iter()
        .map(|cite| {
            let doc_ref = format!("document {} on {}", cite.document_id, cite.portal_key);
            let mut line = format!("- {doc_ref}");
            if let Some(label) = cite.label.as_ref() {
                line.push_str(&format!(" ({label})"));
            }
            if let Some(quote) = cite.quote.as_ref() {
                let trimmed = quote.trim();
                if !trimmed.is_empty() {
                    // Keep the quote short (single line) — user can expand from source if needed.
                    let capped: String = trimmed.chars().take(180).collect();
                    line.push_str(&format!(": \"{capped}\""));
                }
            }
            line
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::finding::{
        FindingCitation, FindingConfidence, FindingSeverity, FindingSubject,
    };

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
        let mut finding = sample_finding();
        finding.subject.agency_id = None;
        finding.subject.request_id = None;
        assert_eq!(find_agency_label(&finding), "[AGENCY NAME]");
        assert_eq!(find_request_label(&finding), "[YOUR REQUEST NUMBER]");
    }
}
