use super::{ResearchCitation, ResearchReport};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindingReviewInput {
    pub report_id: String,
    pub finding_index: usize,
    pub expected_revision: u64,
    pub reviewer: String,
    pub assigned_to: String,
    pub verdict: String,
    pub reasoning: String,
    pub falsification_test: String,
    pub counter_search: String,
    pub counter_evidence: Vec<ResearchCitation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingReview {
    pub id: String,
    pub revision: u64,
    pub created_at: String,
    pub input: FindingReviewInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceReplay {
    pub evidence_id: String,
    pub checked_at: String,
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingChallenge {
    pub finding_index: usize,
    pub title: String,
    pub independent_sources: usize,
    pub checks: Vec<String>,
    pub blockers: Vec<String>,
    pub next_test: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicEntityRef {
    pub key: String,
    pub name: String,
    pub kind: String,
    pub jurisdiction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicRelationship {
    pub id: String,
    pub report_id: String,
    pub revision: u64,
    pub subject: PublicEntityRef,
    pub predicate: String,
    pub object: PublicEntityRef,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub citations: Vec<ResearchCitation>,
    pub status: String,
    pub reviewer: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicConnection {
    pub subject: String,
    pub via: String,
    pub object: String,
    pub relationship_ids: Vec<String>,
    pub report_ids: Vec<String>,
    pub explanation: String,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewWorkspace {
    pub report: ResearchReport,
    pub reviews: Vec<FindingReview>,
    pub challenges: Vec<FindingChallenge>,
    pub replays: Vec<EvidenceReplay>,
    pub relationships: Vec<PublicRelationship>,
    pub connections: Vec<PublicConnection>,
    pub identity_warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PilotAssessment {
    pub report_id: String,
    pub assessor: String,
    pub usefulness: u8,
    pub supported_discoveries: u32,
    pub false_leads: u32,
    pub minutes_spent: u32,
    pub baseline_minutes: Option<u32>,
    pub result_used: String,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluationCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvestigationEvaluation {
    pub id: String,
    pub title: String,
    pub checked_at: String,
    pub checks: Vec<EvaluationCheck>,
    pub assessments: Vec<PilotAssessment>,
    pub limitations: Vec<String>,
}
