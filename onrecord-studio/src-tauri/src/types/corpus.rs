use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CorpusPatternKind {
    ExactDuplicate,
    NearDuplicate,
    SharedStatute,
    SharedEntity,
    SharedTerm,
    TemporalCluster,
}

impl CorpusPatternKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExactDuplicate => "EXACT_DUPLICATE",
            Self::NearDuplicate => "NEAR_DUPLICATE",
            Self::SharedStatute => "SHARED_STATUTE",
            Self::SharedEntity => "SHARED_ENTITY",
            Self::SharedTerm => "SHARED_TERM",
            Self::TemporalCluster => "TEMPORAL_CLUSTER",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusEvidenceReference {
    pub portal_key: String,
    pub request_id: u64,
    pub document_id: u64,
    pub segment_index: Option<usize>,
    pub character_start: Option<usize>,
    pub character_end: Option<usize>,
    pub evidence_label: String,
    pub content_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusPattern {
    pub pattern_id: String,
    pub kind: CorpusPatternKind,
    pub title: String,
    pub explanation: String,
    pub confidence_basis_points: u16,
    pub evidence: Vec<CorpusEvidenceReference>,
    pub algorithm_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusPatternQuery {
    pub portal_key: Option<String>,
    pub minimum_evidence_count: usize,
    #[serde(default)]
    pub minimum_confidence_basis_points: u16,
    #[serde(default)]
    pub indexed_from: Option<String>,
    #[serde(default)]
    pub indexed_to: Option<String>,
    pub result_limit: usize,
}

impl Default for CorpusPatternQuery {
    fn default() -> Self {
        Self {
            portal_key: None,
            minimum_evidence_count: 2,
            minimum_confidence_basis_points: 0,
            indexed_from: None,
            indexed_to: None,
            result_limit: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CorpusIndexStatus {
    pub indexed_document_count: usize,
    pub indexed_segment_count: usize,
    pub indexed_term_count: usize,
    /// Remaining bounded writer-queue slots. A high value means idle capacity; zero means new
    /// corpus writes will wait for the serialized writer.
    pub available_write_capacity: usize,
    pub algorithm_version: String,
}
