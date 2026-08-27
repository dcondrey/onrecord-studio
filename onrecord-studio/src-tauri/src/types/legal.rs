use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LegalAuthorityKind {
    StateStatute,
    MunicipalOrdinance,
    AgencyPolicy,
    ProcedureManual,
    Regulation,
    Other,
}

impl LegalAuthorityKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StateStatute => "STATE_STATUTE",
            Self::MunicipalOrdinance => "MUNICIPAL_ORDINANCE",
            Self::AgencyPolicy => "AGENCY_POLICY",
            Self::ProcedureManual => "PROCEDURE_MANUAL",
            Self::Regulation => "REGULATION",
            Self::Other => "OTHER",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LegalMandateSeverity {
    Advisory,
    Moderate,
    High,
    Critical,
}

impl LegalMandateSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Advisory => "ADVISORY",
            Self::Moderate => "MODERATE",
            Self::High => "HIGH",
            Self::Critical => "CRITICAL",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LegalMandateInput {
    pub mandate_id: String,
    pub title: String,
    pub description: String,
    pub applicability_phrases: Vec<String>,
    pub required_evidence_phrases: Vec<String>,
    pub severity: LegalMandateSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LegalAuthorityImport {
    pub statute_id: String,
    pub jurisdiction: String,
    pub statute_citation: String,
    pub title: String,
    pub authority_kind: LegalAuthorityKind,
    pub raw_text: String,
    pub mandated_deadline_days: Option<u32>,
    pub canonical_source_uri: Option<String>,
    pub effective_from: Option<String>,
    pub effective_to: Option<String>,
    pub mandates: Vec<LegalMandateInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LegalAuthoritySummary {
    pub statute_id: String,
    pub jurisdiction: String,
    pub statute_citation: String,
    pub title: String,
    pub authority_kind: LegalAuthorityKind,
    pub current_version_id: String,
    pub content_sha256: String,
    pub mandated_deadline_days: Option<u32>,
    pub canonical_source_uri: Option<String>,
    pub effective_from: Option<String>,
    pub effective_to: Option<String>,
    pub provision_count: usize,
    pub mandate_count: usize,
    pub imported_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LegalAuthorityImportResult {
    pub authority: LegalAuthoritySummary,
    pub inserted_new_version: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LegalComplianceFinding {
    pub violation_id: String,
    pub request_id: u64,
    pub portal_key: String,
    pub matched_statute_id: String,
    pub matched_version_id: String,
    pub mandate_id: Option<String>,
    pub statute_citation: String,
    pub authority_title: String,
    pub mandate_title: Option<String>,
    pub classification: String,
    pub calculated_variance_metric: Option<String>,
    pub variance_days: Option<i64>,
    pub confidence_basis_points: u16,
    pub evidence_summary: String,
    pub supporting_document_ids: Vec<u64>,
    pub missing_evidence_phrases: Vec<String>,
    pub review_status: String,
    pub first_detected_at: String,
    pub evaluated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LegalEvaluationResult {
    pub request_id: u64,
    pub portal_key: String,
    pub evaluated_mandate_count: usize,
    pub applicable_mandate_count: usize,
    pub corpus_character_count: usize,
    pub evaluation_complete: bool,
    pub warnings: Vec<String>,
    pub findings: Vec<LegalComplianceFinding>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LegalFindingReviewStatus {
    Open,
    Confirmed,
    Dismissed,
}

impl LegalFindingReviewStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::Confirmed => "CONFIRMED",
            Self::Dismissed => "DISMISSED",
        }
    }
}
