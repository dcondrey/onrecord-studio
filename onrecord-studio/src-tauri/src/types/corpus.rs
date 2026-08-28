use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CorpusEntityKind {
    Statute,
    Date,
    EmailDomain,
    UrlHost,
    Acronym,
    Organization,
}

impl CorpusEntityKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Statute => "STATUTE",
            Self::Date => "DATE",
            Self::EmailDomain => "EMAIL_DOMAIN",
            Self::UrlHost => "URL_HOST",
            Self::Acronym => "ACRONYM",
            Self::Organization => "ORGANIZATION",
        }
    }
}

impl std::fmt::Display for CorpusEntityKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CorpusPatternKind {
    ExactDuplicate,
    NearDuplicate,
    SharedStatute,
    SharedEntity,
    SharedTerm,
    TemporalCluster,
    EntityRelationship,
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
            Self::EntityRelationship => "ENTITY_RELATIONSHIP",
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
    #[serde(default)]
    pub pattern_kinds: Vec<CorpusPatternKind>,
    #[serde(default)]
    pub required_terms: Vec<String>,
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
            pattern_kinds: Vec::new(),
            required_terms: Vec::new(),
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
    pub indexed_relationship_count: usize,
    /// Remaining bounded writer-queue slots. A high value means idle capacity; zero means new
    /// corpus writes will wait for the serialized writer.
    pub available_write_capacity: usize,
    pub algorithm_version: String,
}
