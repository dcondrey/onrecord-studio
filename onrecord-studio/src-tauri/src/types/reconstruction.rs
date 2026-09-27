use serde::{Deserialize, Serialize};

/// References always address the retained extraction, not a guessed PDF page number.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconstructionCitation {
    pub id: String,
    pub document_id: u64,
    pub portal_key: String,
    pub title: String,
    pub source_url: Option<String>,
    pub source_sha256: String,
    pub extraction_sha256: String,
    pub character_offset: usize,
    pub quote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconstructionCandidate {
    pub value: String,
    pub evidence: Vec<ReconstructionCitation>,
    pub supports: Vec<String>,
    pub limitations: Vec<String>,
    pub contradicted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconstructionFinding {
    pub id: String,
    pub method: String,
    /// corroborated, candidate, ambiguous, or unresolved; never "recovered bytes".
    pub status: String,
    pub subject: String,
    pub observation: ReconstructionCitation,
    pub candidates: Vec<ReconstructionCandidate>,
    pub explanation: String,
    pub next_step: String,
    pub request_draft: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconstructionReport {
    pub analysis_version: String,
    pub document_id: u64,
    pub portal_key: String,
    pub source_sha256: String,
    pub corpus_fingerprint: String,
    pub documents_compared: usize,
    pub passages_compared: usize,
    pub findings: Vec<ReconstructionFinding>,
    pub limitations: Vec<String>,
    pub search_terms: Vec<String>,
}

/// Extra search context must be copied from the target extraction. This is useful when a
/// signature/title is separated from a redacted sender by many pages.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ReconstructionOptions {
    pub context_quote: Option<String>,
    /// An explicit analyst assumption, never inferred from the width of a black box.
    pub fixed_width_masks: bool,
}
