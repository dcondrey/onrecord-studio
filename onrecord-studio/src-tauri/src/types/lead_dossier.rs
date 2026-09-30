use crate::analysis::finding::{Finding, FindingConfidence, FindingKind, FindingSeverity};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One cited span, resolved to the record it lives in. `file_name` is `None` when the document
/// could not be resolved from the finding's request; that absence is shown, not filled in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceLink {
    pub portal_key: String,
    pub document_id: i64,
    pub file_name: Option<String>,
    pub request_id: Option<i64>,
    pub agency_label: Option<String>,
    pub raw_file_sha256: Option<String>,
    pub character_start: Option<i64>,
    pub character_end: Option<i64>,
    pub quote: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RelationBasis {
    SameRequest,
    CitesSameDocument,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedFinding {
    pub id: String,
    pub kind: FindingKind,
    pub headline: String,
    pub severity: FindingSeverity,
    pub confidence: FindingConfidence,
    pub basis: RelationBasis,
    pub discovered_at: DateTime<Utc>,
}

/// What would disconfirm the lead, stated as records to look for rather than conclusions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DossierFalsification {
    pub statement: String,
    pub records_to_seek: Vec<String>,
}

/// A gap is either a figure in a cited document that could not be checked, or a document whose
/// figures could not be examined at all. The second is never rendered as "no gaps".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "gap", rename_all = "camelCase")]
pub enum DossierGap {
    UnreconciledClaim {
        portal_key: String,
        document_id: i64,
        claim_text: String,
        reason: String,
        detail: String,
    },
    NotComputed {
        portal_key: String,
        document_id: i64,
        reason: String,
    },
}

/// Observations assembled around one finding. Nothing here is a legal claim: a pattern signal
/// stays a pattern signal, and the falsification section says what would disconfirm it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeadDossier {
    pub finding: Finding,
    pub evidence: Vec<EvidenceLink>,
    pub evidence_truncated: bool,
    pub related: Vec<RelatedFinding>,
    pub related_truncated: bool,
    pub falsification: DossierFalsification,
    pub gaps: Vec<DossierGap>,
    pub gaps_truncated: bool,
    pub assembled_at: DateTime<Utc>,
}
