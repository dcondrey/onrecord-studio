use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringEvidence {
    pub portal_key: String,
    pub document_id: u64,
    pub title: String,
    pub source_url: String,
    pub source_sha256: String,
    pub extraction_sha256: String,
    pub quote: String,
    pub character_offset: usize,
    pub snapshot_id: Option<String>,
    pub row_number: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageExpectation {
    pub id: String,
    pub case_id: Option<String>,
    pub topic: String,
    pub agency: String,
    pub record_type: String,
    pub period: String,
    pub query_terms: Vec<String>,
    pub consequence: String,
    pub acquisition_action: String,
    pub state: String,
    pub evidence: Vec<MonitoringEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageAssessment {
    pub expectation: CoverageExpectation,
    pub status: String,
    pub explanation: String,
    pub candidates: Vec<MonitoringEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringFact {
    pub category: String,
    pub quote: String,
    pub character_offset: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordChange {
    pub id: String,
    pub source_url: String,
    pub title: String,
    pub observed_at: String,
    pub category: String,
    pub before_sha256: String,
    pub after_sha256: String,
    pub before_quote: Option<String>,
    pub after_quote: Option<String>,
    pub interpretation: String,
    pub review_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceObservation {
    pub id: String,
    pub case_id: Option<String>,
    pub topic: String,
    pub kind: String,
    pub place: String,
    pub observed_on: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub hours_per_week: Option<f64>,
    pub status: String,
    pub evidence: MonitoringEvidence,
    pub reviewed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceComparison {
    pub kind: String,
    pub title: String,
    pub observation_ids: Vec<String>,
    pub calculation: String,
    pub implication: String,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitoringStatus {
    pub topic: String,
    pub case_id: Option<String>,
    pub coverage: Vec<CoverageAssessment>,
    pub changes: Vec<RecordChange>,
    pub observations: Vec<ServiceObservation>,
    pub comparisons: Vec<ServiceComparison>,
    pub source_candidates: Vec<MonitoringEvidence>,
    pub limitations: Vec<String>,
}
