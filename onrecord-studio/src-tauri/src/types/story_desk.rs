use crate::analysis::finding::{FindingConfidence, FindingKind, FindingSeverity, FindingSubject};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One observable reason a lead ranks where it does, with the fact behind it spelled out so a
/// reader can check it against the finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterestFactor {
    pub name: String,
    pub basis_points: u16,
    pub evidence: String,
}

/// A ranking heuristic over facts the detectors observed. It orders leads for a reader's
/// attention; it is not a measurement of newsworthiness and never a claim about the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicInterestScore {
    pub total_basis_points: u32,
    pub factors: Vec<InterestFactor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryLead {
    pub finding_id: String,
    pub kind: FindingKind,
    pub headline: String,
    pub severity: FindingSeverity,
    pub confidence: FindingConfidence,
    pub subject: FindingSubject,
    pub portal_key: Option<String>,
    pub public_interest: PublicInterestScore,
    /// Other undismissed findings about the same request, document or entity.
    pub related_finding_ids: Vec<String>,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}
