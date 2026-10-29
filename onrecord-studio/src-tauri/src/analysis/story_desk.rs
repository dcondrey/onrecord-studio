//! Orders findings for a reader's attention by facts the detectors observed. The score is a
//! ranking heuristic, not a measurement: every factor names the fact behind it so a reader can
//! check it against the finding, and nothing here is a statement about the record itself.

use crate::analysis::assertion_index::AssertionValue;
use crate::analysis::claim_validation::{scan_money, Hedge};
use crate::analysis::finding::{Finding, FindingConfidence, FindingKind};
use crate::types::{InterestFactor, PublicInterestScore, StoryLead};
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeSet;

// Weights in basis points. One table so the ranking can be read in one place.
// Five figures is where a discrepancy stops reading as rounding; each further order of magnitude
// widens who is affected.
const MONEY_10K_BP: u16 = 800;
const MONEY_100K_BP: u16 = 1_600;
const MONEY_1M_BP: u16 = 2_500;
// A finding drawn from several documents is harder to explain away than one drawn from one.
const SCALE_PER_EXTRA_DOCUMENT_BP: u16 = 150;
const SCALE_DOCUMENTS_CAP_BP: u16 = 900;
// The same signal across requests, and above all across agencies, points at practice rather
// than accident.
const SCALE_MULTI_REQUEST_BP: u16 = 400;
const SCALE_MULTI_PORTAL_BP: u16 = 1_200;
const RECURRENCE_PER_SIBLING_BP: u16 = 200;
const RECURRENCE_CAP_BP: u16 = 1_000;
// A record contradicting a statement is the shape accountability reporting is built on.
const CONTRADICTION_BP: u16 = 2_000;
// A counted violation is directly reportable; it is the only factor allowed to use the word.
const LEGAL_CLAIM_BP: u16 = 1_800;
// Measured confidence scaled to a fifth, so certainty helps but cannot outrank substance.
const STRENGTH_MAX_BP: u16 = 2_000;
const FRESH_7D_BP: u16 = 400;
const FRESH_30D_BP: u16 = 200;

pub const CONTEXT_ONLY_KINDS: &[FindingKind] = &[
    FindingKind::AbsenceOfRecords,
    FindingKind::RedactionCandidateInferred,
    FindingKind::CorrelatedFindings,
    FindingKind::RegeneratedResponse,
    FindingKind::JustInTimeCompliance,
    FindingKind::EvasionRedactionAtLimit,
    FindingKind::EvasionRoundNumberThresholds,
    FindingKind::ReleaseTimingConcentration,
    FindingKind::CrossCorpusOrphanTopic,
];

fn actionable(finding: &Finding) -> bool {
    if CONTEXT_ONLY_KINDS.contains(&finding.kind) {
        return false;
    }
    // Older sweeps stored generic words such as "award is" as monetary identifiers.
    // Preserve them for record inspection, but do not rank them as discrepancies.
    if finding.kind == FindingKind::ContradictoryStatement {
        if let Some((subject, amounts)) = finding.headline.split_once("' stated amount:") {
            let identified = subject
                .rsplit('\'')
                .next()
                .is_some_and(|id| id.bytes().any(|b| b.is_ascii_digit()));
            let disagreement =
                amounts
                    .split_once(" in one document, ")
                    .is_some_and(|(a, b)| {
                        match (scan_money(a, Hedge::Exact), scan_money(b, Hedge::Exact)) {
                            (Some(a), Some(b)) => AssertionValue::Cents(a.cents)
                                .materially_differs(&AssertionValue::Cents(b.cents)),
                            _ => false,
                        }
                    });
            return identified && disagreement;
        }
    }
    true
}

pub fn rank_leads(findings: &[Finding], now: DateTime<Utc>, limit: usize) -> Vec<StoryLead> {
    let live: Vec<&Finding> = findings
        .iter()
        .filter(|finding| finding.dismissed_at.is_none() && actionable(finding))
        .collect();
    let mut leads: Vec<StoryLead> = live
        .iter()
        .map(|finding| lead_for(finding, &live, now))
        .collect();
    leads.sort_by(|a, b| {
        b.public_interest
            .total_basis_points
            .cmp(&a.public_interest.total_basis_points)
            .then_with(|| b.severity.rank_weight().cmp(&a.severity.rank_weight()))
            .then_with(|| b.last_seen_at.cmp(&a.last_seen_at))
            .then_with(|| a.finding_id.cmp(&b.finding_id))
    });
    leads.truncate(limit);
    leads
}

fn lead_for(finding: &Finding, live: &[&Finding], now: DateTime<Utc>) -> StoryLead {
    let related: Vec<&Finding> = live
        .iter()
        .copied()
        .filter(|other| other.id != finding.id && shares_subject(finding, other))
        .collect();
    let mut factors = Vec::new();

    let texts = std::iter::once(finding.headline.as_str())
        .chain(std::iter::once(finding.explanation.as_str()))
        .chain(finding.citations.iter().filter_map(|c| c.quote.as_deref()));
    if let Some(cents) = largest_dollar_cents(texts) {
        let basis_points = if cents >= 100_000_000 {
            MONEY_1M_BP
        } else if cents >= 10_000_000 {
            MONEY_100K_BP
        } else if cents >= 1_000_000 {
            MONEY_10K_BP
        } else {
            0
        };
        if basis_points > 0 {
            factors.push(InterestFactor {
                name: "money at stake".into(),
                basis_points,
                evidence: format!(
                    "largest figure named: {}",
                    AssertionValue::Cents(cents).display()
                ),
            });
        }
    }

    let mut documents: BTreeSet<(String, i64)> = finding
        .citations
        .iter()
        .map(|c| (c.portal_key.clone(), c.document_id))
        .collect();
    let mut portals: BTreeSet<&str> = finding
        .citations
        .iter()
        .map(|c| c.portal_key.as_str())
        .collect();
    let mut requests: BTreeSet<(Option<&str>, i64)> = BTreeSet::new();
    if let (Some(portal), Some(document)) =
        (&finding.subject.portal_key, finding.subject.document_id)
    {
        documents.insert((portal.clone(), document));
    }
    if let Some(portal) = &finding.subject.portal_key {
        portals.insert(portal.as_str());
    }
    if let Some(request) = finding.subject.request_id {
        requests.insert((finding.subject.portal_key.as_deref(), request));
    }
    for other in &related {
        for citation in &other.citations {
            documents.insert((citation.portal_key.clone(), citation.document_id));
            portals.insert(citation.portal_key.as_str());
        }
        if let Some(request) = other.subject.request_id {
            requests.insert((other.subject.portal_key.as_deref(), request));
        }
    }
    let multi_agency = portals.len() > 1 || finding.kind.as_str().starts_with("CROSS_CORPUS_");
    let extra_documents = documents.len().saturating_sub(1);
    let scale_bp = (SCALE_PER_EXTRA_DOCUMENT_BP
        .saturating_mul(extra_documents.min(u16::MAX as usize) as u16))
    .min(SCALE_DOCUMENTS_CAP_BP)
    .saturating_add(if requests.len() > 1 {
        SCALE_MULTI_REQUEST_BP
    } else {
        0
    })
    .saturating_add(if multi_agency {
        SCALE_MULTI_PORTAL_BP
    } else {
        0
    });
    if scale_bp > 0 {
        factors.push(InterestFactor {
            name: "scale".into(),
            basis_points: scale_bp,
            evidence: format!(
                "{} document{}, {} request{}, {} agenc{}{}",
                documents.len(),
                if documents.len() == 1 { "" } else { "s" },
                requests.len(),
                if requests.len() == 1 { "" } else { "s" },
                portals.len(),
                if portals.len() == 1 { "y" } else { "ies" },
                if finding.kind.as_str().starts_with("CROSS_CORPUS_") {
                    " (cross-corpus finding)"
                } else {
                    ""
                },
            ),
        });
    }

    let siblings = live
        .iter()
        .filter(|other| {
            other.id != finding.id
                && other.source_detector == finding.source_detector
                && other.subject.portal_key == finding.subject.portal_key
        })
        .count();
    if siblings > 0 {
        factors.push(InterestFactor {
            name: "recurrence".into(),
            basis_points: RECURRENCE_PER_SIBLING_BP
                .saturating_mul(siblings.min(u16::MAX as usize) as u16)
                .min(RECURRENCE_CAP_BP),
            evidence: format!(
                "{siblings} other undismissed finding{} from {} for the same source",
                if siblings == 1 { "" } else { "s" },
                finding.source_detector
            ),
        });
    }

    if matches!(
        finding.kind,
        FindingKind::ContradictoryStatement
            | FindingKind::MetadataContradictsContent
            | FindingKind::NoRecordsContradictedByRelease
            | FindingKind::ClaimContradictsData
    ) {
        factors.push(InterestFactor {
            name: "record contradicts statement".into(),
            basis_points: CONTRADICTION_BP,
            evidence: format!("finding kind {}", finding.kind.as_str()),
        });
    }

    if finding.kind.is_legal_claim() {
        factors.push(InterestFactor {
            name: "counted statutory or policy violation".into(),
            basis_points: LEGAL_CLAIM_BP,
            evidence: format!(
                "finding kind {} is a counted violation",
                finding.kind.as_str()
            ),
        });
    }

    match finding.confidence {
        FindingConfidence::Measured { basis_points } => {
            let scaled = (u32::from(basis_points) / 5).min(u32::from(STRENGTH_MAX_BP)) as u16;
            factors.push(InterestFactor {
                name: "evidence strength".into(),
                basis_points: scaled,
                evidence: format!(
                    "measured confidence {}",
                    crate::report::percent(basis_points)
                ),
            });
        }
        FindingConfidence::Unassessed => factors.push(InterestFactor {
            name: "evidence strength".into(),
            basis_points: 0,
            evidence: "confidence unassessed".into(),
        }),
    }

    let age = now.signed_duration_since(finding.last_seen_at);
    if age <= Duration::days(7) {
        factors.push(InterestFactor {
            name: "freshness".into(),
            basis_points: FRESH_7D_BP,
            evidence: "seen within the last 7 days".into(),
        });
    } else if age <= Duration::days(30) {
        factors.push(InterestFactor {
            name: "freshness".into(),
            basis_points: FRESH_30D_BP,
            evidence: "seen within the last 30 days".into(),
        });
    }

    let total_basis_points = factors.iter().fold(0u32, |sum, factor| {
        sum.saturating_add(u32::from(factor.basis_points))
    });
    StoryLead {
        finding_id: finding.id.clone(),
        kind: finding.kind,
        headline: finding.headline.clone(),
        severity: finding.severity,
        confidence: finding.confidence,
        subject: finding.subject.clone(),
        portal_key: finding.subject.portal_key.clone(),
        public_interest: PublicInterestScore {
            total_basis_points,
            factors,
        },
        related_finding_ids: related.iter().map(|other| other.id.clone()).collect(),
        first_seen_at: finding.discovered_at,
        last_seen_at: finding.last_seen_at,
    }
}

fn shares_subject(a: &Finding, b: &Finding) -> bool {
    if a.subject.portal_key != b.subject.portal_key {
        return false;
    }
    (a.subject.entity_id.is_some() && a.subject.entity_id == b.subject.entity_id)
        || (a.subject.request_id.is_some() && a.subject.request_id == b.subject.request_id)
        || (a.subject.document_id.is_some() && a.subject.document_id == b.subject.document_id)
}

/// Every `$` figure in the texts, scanned one at a time so a sentence naming two amounts still
/// yields both (the claim scanner alone refuses such a sentence on purpose).
fn largest_dollar_cents<'a>(texts: impl Iterator<Item = &'a str>) -> Option<i64> {
    let mut best: Option<i64> = None;
    for text in texts {
        let mut rest = text;
        while let Some(start) = rest.find('$') {
            let after = &rest[start + 1..];
            let end = after.find('$').map_or(rest.len(), |next| start + 1 + next);
            if let Some(money) = scan_money(&rest[start..end], Hedge::Exact) {
                best = Some(best.map_or(money.cents, |b| b.max(money.cents)));
            }
            rest = &rest[end..];
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::finding::{FindingCitation, FindingSeverity, FindingSubject};

    fn finding(
        id: &str,
        kind: FindingKind,
        headline: &str,
        confidence: FindingConfidence,
    ) -> Finding {
        let now = Utc::now();
        Finding {
            id: id.into(),
            kind,
            severity: FindingSeverity::Notable,
            confidence,
            subject: FindingSubject {
                portal_key: Some("sd-city".into()),
                request_id: Some(7),
                ..Default::default()
            },
            headline: headline.into(),
            explanation: String::new(),
            citations: vec![FindingCitation {
                portal_key: "sd-city".into(),
                document_id: 1,
                character_start: None,
                character_end: None,
                quote: None,
                label: None,
            }],
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: "test".into(),
        }
    }

    fn factor<'a>(lead: &'a StoryLead, name: &str) -> Option<&'a InterestFactor> {
        lead.public_interest.factors.iter().find(|f| f.name == name)
    }

    #[test]
    fn contextual_clues_and_stale_generic_amounts_do_not_become_leads() {
        let now = Utc::now();
        let mut findings: Vec<Finding> = CONTEXT_ONLY_KINDS
            .iter()
            .enumerate()
            .map(|(index, kind)| {
                finding(
                    &format!("context-{index}"),
                    *kind,
                    "$900 million in 50 documents",
                    FindingConfidence::measured(10_000),
                )
            })
            .collect();
        for (id, headline) in [
            (
                "generic",
                "'award is' stated amount: $100000 in one document, $200000 in another",
            ),
            (
                "identical",
                "'contract 123' stated amount: $100000 in one document, $100000 in another",
            ),
            (
                "real",
                "'contract 123' stated amount: $100000 in one document, $200000 in another",
            ),
        ] {
            findings.push(finding(
                id,
                FindingKind::ContradictoryStatement,
                headline,
                FindingConfidence::measured(9_000),
            ));
        }
        findings.push(finding(
            "services",
            FindingKind::ServiceReportLead,
            "Open sanitation reports remain unresolved after a year",
            FindingConfidence::measured(9_000),
        ));
        let leads = rank_leads(&findings, now, 100);
        assert_eq!(leads.len(), 2);
        assert!(leads.iter().any(|lead| lead.finding_id == "real"));
        assert!(leads.iter().any(|lead| lead.finding_id == "services"));
        assert!(leads.iter().all(|lead| lead.related_finding_ids.len() == 1));
    }

    #[test]
    fn factors_follow_the_table_and_ties_are_stable() {
        let now = Utc::now();
        let money = finding(
            "money",
            FindingKind::HeldBackResponse,
            "The contract grew from $1.0 million to $2.5 million",
            FindingConfidence::measured(5_000),
        );
        let legal = finding(
            "legal",
            FindingKind::PolicyViolationDetected,
            "Deadline missed",
            FindingConfidence::measured(9_000),
        );
        let mut cross = finding(
            "cross",
            FindingKind::CrossCorpusSharedLanguage,
            "Same denial text across agencies",
            FindingConfidence::Unassessed,
        );
        cross.citations.push(FindingCitation {
            portal_key: "sd-county".into(),
            document_id: 9,
            character_start: None,
            character_end: None,
            quote: None,
            label: None,
        });
        let contradiction = finding(
            "contra",
            FindingKind::ClaimContradictsData,
            "Stated total does not match the table",
            FindingConfidence::measured(7_500),
        );
        let mut dismissed = finding(
            "gone",
            FindingKind::HeldBackResponse,
            "old",
            FindingConfidence::Unassessed,
        );
        dismissed.dismissed_at = Some(now);
        let leads = rank_leads(&[money, legal, cross, contradiction, dismissed], now, 10);

        assert_eq!(leads.len(), 4, "dismissed findings never become leads");
        let money = leads.iter().find(|l| l.finding_id == "money").unwrap();
        assert_eq!(
            factor(money, "money at stake").unwrap().basis_points,
            MONEY_1M_BP
        );
        assert!(factor(money, "money at stake")
            .unwrap()
            .evidence
            .contains("2500000"));
        let legal = leads.iter().find(|l| l.finding_id == "legal").unwrap();
        assert_eq!(
            factor(legal, "counted statutory or policy violation")
                .unwrap()
                .basis_points,
            LEGAL_CLAIM_BP
        );
        assert_eq!(
            factor(legal, "evidence strength").unwrap().basis_points,
            1_800
        );
        let cross = leads.iter().find(|l| l.finding_id == "cross").unwrap();
        assert!(factor(cross, "scale").unwrap().basis_points >= SCALE_MULTI_PORTAL_BP);
        assert_eq!(factor(cross, "evidence strength").unwrap().basis_points, 0);
        assert_eq!(
            factor(cross, "evidence strength").unwrap().evidence,
            "confidence unassessed"
        );
        let contra = leads.iter().find(|l| l.finding_id == "contra").unwrap();
        assert_eq!(
            factor(contra, "record contradicts statement")
                .unwrap()
                .basis_points,
            CONTRADICTION_BP
        );
        // Recurrence counts siblings from the same detector and source, excluding self.
        assert_eq!(
            factor(contra, "recurrence").unwrap().basis_points,
            3 * RECURRENCE_PER_SIBLING_BP
        );
        // Every lead shares request 7, so each relates to the other three.
        assert_eq!(contra.related_finding_ids.len(), 3);

        let tied = rank_leads(
            &[
                finding(
                    "b",
                    FindingKind::HeldBackResponse,
                    "x",
                    FindingConfidence::Unassessed,
                ),
                finding(
                    "a",
                    FindingKind::HeldBackResponse,
                    "x",
                    FindingConfidence::Unassessed,
                ),
            ],
            now,
            10,
        );
        assert_eq!(
            tied.iter()
                .map(|l| l.finding_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
    }
}
