use super::research::ResearchSourceReference;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkCitation {
    pub id: String,
    pub report_id: String,
    pub evidence_id: String,
    pub label: String,
    pub quote: String,
    pub source_url: Option<String>,
    pub source_sha256: String,
    pub source_references: Vec<ResearchSourceReference>,
    pub portal_key: Option<String>,
    pub document_id: Option<u64>,
    pub character_offset: Option<usize>,
    /// SAVED_REPORT, HISTORICAL_REPORT or UNVERIFIED_PORTABLE. Recomputed by the repository.
    pub verification: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkQuestion {
    pub id: String,
    pub question: String,
    pub hypothesis: String,
    pub next_test: String,
    pub disposition: String,
    pub owner: String,
    pub citation_ids: Vec<String>,
    pub counter_citation_ids: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkProduction {
    pub id: String,
    pub received_at: String,
    pub description: String,
    pub record_types: Vec<String>,
    pub fields: Vec<String>,
    pub period: String,
    pub citation_ids: Vec<String>,
    pub notes: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkRequest {
    pub id: String,
    pub title: String,
    pub custodian: String,
    pub gap: String,
    pub resolves: String,
    pub draft: String,
    pub state: String,
    pub expected_record_types: Vec<String>,
    pub expected_fields: Vec<String>,
    pub expected_period: String,
    pub submitted_at: Option<String>,
    pub response_due: Option<String>,
    pub external_reference: String,
    pub productions: Vec<CaseworkProduction>,
    pub unresolved_omissions: Vec<String>,
    pub follow_up_draft: String,
    pub citation_ids: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkDecision {
    pub id: String,
    pub body: String,
    pub date: String,
    pub agenda_item: String,
    pub proposed_decision: String,
    pub action: String,
    pub outcome: String,
    pub vote: String,
    pub state: String,
    pub citation_ids: Vec<String>,
    pub outcome_citation_ids: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkCommitment {
    pub id: String,
    pub promise: String,
    pub responsible_party: String,
    pub due_date: Option<String>,
    pub completion_test: String,
    pub state: String,
    pub assessment: String,
    pub citation_ids: Vec<String>,
    pub completion_citation_ids: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct CaseworkCase {
    pub id: String,
    pub report_id: String,
    pub topic: String,
    pub title: String,
    pub owner: String,
    pub disposition: String,
    pub revision: u64,
    pub created_at: String,
    pub updated_at: String,
    pub questions: Vec<CaseworkQuestion>,
    pub requests: Vec<CaseworkRequest>,
    pub decisions: Vec<CaseworkDecision>,
    pub commitments: Vec<CaseworkCommitment>,
    pub evidence: Vec<CaseworkCitation>,
    pub notes: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseworkEvent {
    pub case_id: String,
    pub revision: u64,
    pub author: String,
    pub at: String,
    pub summary: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseworkRankFactor {
    pub label: String,
    pub score: u32,
    pub maximum: u32,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseworkLead {
    pub report_id: String,
    pub finding_index: usize,
    pub title: String,
    pub next_action: String,
    pub score: u32,
    pub factors: Vec<CaseworkRankFactor>,
    pub citations: Vec<CaseworkCitation>,
    pub case_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseworkState {
    pub available_evidence: Vec<CaseworkCitation>,
    pub cases: Vec<CaseworkCase>,
    pub events: Vec<CaseworkEvent>,
    pub leads: Vec<CaseworkLead>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaseworkPacket {
    pub schema_version: u32,
    pub base_revision: u64,
    pub base_sha256: String,
    pub exported_at: String,
    pub contributor: String,
    pub case: CaseworkCase,
    pub history: Vec<CaseworkEvent>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseworkImportPreview {
    pub mode: String,
    pub local_revision: Option<u64>,
    pub packet_sha256: String,
    pub warnings: Vec<String>,
    pub case: CaseworkCase,
    pub contributor: String,
}
