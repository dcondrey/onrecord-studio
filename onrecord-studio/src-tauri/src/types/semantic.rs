use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingModelStatus {
    pub model_id: String,
    pub revision: String,
    pub dimension: usize,
    pub distance_metric: String,
    pub normalization: String,
    pub cache_dir: String,
    pub installed: bool,
    pub ready: bool,
    pub artifact_count: usize,
    pub artifact_manifest_sha256: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingIndexStatus {
    pub indexed_document_count: usize,
    pub indexed_chunk_count: usize,
    pub pending_document_count: usize,
    pub failed_document_count: usize,
    pub running: bool,
    pub cancelled: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingIndexReport {
    pub indexed_documents: usize,
    pub indexed_chunks: usize,
    pub skipped_documents: usize,
    pub failed_documents: usize,
    pub cancelled: bool,
    pub first_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchFilter {
    pub portal_key: Option<String>,
    pub agency_name: Option<String>,
    pub jurisdiction: Option<String>,
    pub document_type: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HybridSearchResult {
    pub stable_id: String,
    pub portal_key: String,
    pub request_id: u64,
    pub document_id: u64,
    pub file_name: String,
    pub title: String,
    pub agency_name: String,
    pub jurisdiction: String,
    pub document_type: Option<String>,
    pub publication_date: Option<String>,
    pub observation_date: Option<String>,
    pub effective_date: Option<String>,
    pub character_start: usize,
    pub character_end: usize,
    pub excerpt: String,
    pub lexical_rank: Option<usize>,
    pub semantic_score: Option<f64>,
    pub fused_score: f64,
    pub retrieval_status: String,
}
