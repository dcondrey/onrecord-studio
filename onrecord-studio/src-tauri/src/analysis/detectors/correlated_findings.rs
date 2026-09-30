//! Cross-detector correlation over the stored findings inbox.
//!
//! Groups non-dismissed findings that name exactly the same document, request or agency, and emits
//! one finding when at least [`MIN_DISTINCT_KINDS`] different kinds converge on it. Only an exact
//! subject match correlates: a document-level finding is never attributed to its agency by
//! inference. Runs after every other detector so it reads a settled inbox.
//!
//! The finding restates what its members observed and claims nothing further. Confidence is the
//! weakest member's, and unassessed when any member is, so a correlation is never stronger than
//! its evidence.

use crate::{
    analysis::finding::{
        finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSubject,
    },
    error::AppError,
    storage::{findings::FindingsFilter, DuckDbRepository},
};
use chrono::Utc;
use std::collections::{BTreeMap, BTreeSet};

pub const SLUG: &str = "correlated_findings";
const MIN_DISTINCT_KINDS: usize = 2;
const MAX_CITATIONS: usize = 12;
const MAX_CORRELATIONS: usize = 100;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Anchor {
    Document(i64),
    Request(i64),
    Agency(i64),
}

impl Anchor {
    fn label(&self) -> &'static str {
        match self {
            Self::Document(_) => "document",
            Self::Request(_) => "request",
            Self::Agency(_) => "agency",
        }
    }

    fn subject(&self, portal_key: &str) -> FindingSubject {
        let mut subject = FindingSubject {
            portal_key: Some(portal_key.to_string()),
            ..Default::default()
        };
        match self {
            Self::Document(id) => subject.document_id = Some(*id),
            Self::Request(id) => subject.request_id = Some(*id),
            Self::Agency(id) => subject.agency_id = Some(*id),
        }
        subject
    }
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let stored = repository
        .list_findings(FindingsFilter::default(), 500)
        .await?;
    Ok(correlate(&stored))
}

fn correlate(stored: &[Finding]) -> Vec<Finding> {
    let mut groups: BTreeMap<(String, Anchor), Vec<&Finding>> = BTreeMap::new();
    for finding in stored {
        if finding.kind == FindingKind::CorrelatedFindings {
            continue;
        }
        let Some(portal) = finding.subject.portal_key.as_ref() else {
            continue;
        };
        let anchors = [
            finding.subject.document_id.map(Anchor::Document),
            finding.subject.request_id.map(Anchor::Request),
            finding.subject.agency_id.map(Anchor::Agency),
        ];
        for anchor in anchors.into_iter().flatten() {
            groups
                .entry((portal.clone(), anchor))
                .or_default()
                .push(finding);
        }
    }

    let now = Utc::now();
    let mut correlated: Vec<Finding> = groups
        .into_iter()
        .filter_map(|((portal, anchor), members)| {
            let kinds: BTreeSet<&str> = members.iter().map(|m| m.kind.as_str()).collect();
            if kinds.len() < MIN_DISTINCT_KINDS {
                return None;
            }
            let severity = members.iter().map(|m| m.severity).max()?;
            let confidence = members
                .iter()
                .map(|m| m.confidence)
                .try_fold(10_000u16, |weakest, confidence| {
                    confidence.stored_basis_points().map(|bp| weakest.min(bp))
                })
                .map_or(FindingConfidence::Unassessed, FindingConfidence::measured);
            let mut ids: Vec<&str> = members.iter().map(|m| m.id.as_str()).collect();
            ids.sort_unstable();
            ids.dedup();
            let subject = anchor.subject(&portal);
            let id = finding_id(FindingKind::CorrelatedFindings, SLUG, &subject, &ids);

            let mut citations: Vec<FindingCitation> = Vec::new();
            for citation in members.iter().flat_map(|m| m.citations.iter()) {
                if citations.len() < MAX_CITATIONS && !citations.contains(citation) {
                    citations.push(citation.clone());
                }
            }
            let kind_list = kinds.iter().copied().collect::<Vec<_>>().join(", ");
            Some(Finding {
                id,
                kind: FindingKind::CorrelatedFindings,
                severity,
                confidence,
                subject,
                headline: format!(
                    "{} separate findings converge on one {}",
                    kinds.len(),
                    anchor.label()
                ),
                explanation: format!(
                    "{} stored findings of {} different kinds name this {} as their subject: {}. \
                     Each keeps its own basis; the kinds are separate signals, not one conclusion.",
                    ids.len(),
                    kinds.len(),
                    anchor.label(),
                    kind_list
                ),
                citations,
                discovered_at: now,
                last_seen_at: now,
                dismissed_at: None,
                source_detector: SLUG.to_string(),
            })
        })
        .collect();
    correlated.sort_by_key(|f| std::cmp::Reverse(f.rank_key()));
    correlated.truncate(MAX_CORRELATIONS);
    correlated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::finding::FindingSeverity;

    fn member(kind: FindingKind, document: i64, agency: Option<i64>, bp: Option<u16>) -> Finding {
        let subject = FindingSubject {
            portal_key: Some("p".into()),
            document_id: Some(document),
            agency_id: agency,
            ..Default::default()
        };
        let now = Utc::now();
        Finding {
            id: finding_id(kind, "d", &subject, &[]),
            kind,
            severity: FindingSeverity::Notable,
            confidence: FindingConfidence::from_stored(bp),
            subject,
            headline: String::new(),
            explanation: String::new(),
            citations: vec![],
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: "d".into(),
        }
    }

    #[test]
    fn distinct_kinds_on_one_subject_correlate_at_the_weakest_confidence() {
        let out = correlate(&[
            member(FindingKind::HeldBackResponse, 1, None, Some(9000)),
            member(FindingKind::RegeneratedResponse, 1, None, Some(6000)),
            member(FindingKind::HeldBackResponse, 2, None, Some(9000)),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].subject.document_id, Some(1));
        assert_eq!(out[0].confidence, FindingConfidence::measured(6000));
    }

    #[test]
    fn an_unassessed_member_makes_the_correlation_unassessed() {
        let out = correlate(&[
            member(FindingKind::HeldBackResponse, 1, None, Some(9000)),
            member(FindingKind::RegeneratedResponse, 1, None, None),
        ]);
        assert_eq!(out[0].confidence, FindingConfidence::Unassessed);
    }

    #[test]
    fn same_kind_twice_and_inferred_agency_membership_do_not_correlate() {
        let out = correlate(&[
            member(FindingKind::HeldBackResponse, 1, Some(7), Some(9000)),
            member(FindingKind::HeldBackResponse, 2, Some(7), Some(9000)),
            member(FindingKind::RegeneratedResponse, 3, None, Some(9000)),
        ]);
        assert!(out.is_empty());
    }

    #[test]
    fn correlations_are_not_recorrelated() {
        let mut inbox = vec![
            member(FindingKind::HeldBackResponse, 1, None, Some(9000)),
            member(FindingKind::RegeneratedResponse, 1, None, Some(9000)),
        ];
        inbox.extend(correlate(&inbox));
        let out = correlate(&inbox);
        assert_eq!(out.len(), 1);
        assert!(out[0].explanation.starts_with("2 stored"));
    }
}
