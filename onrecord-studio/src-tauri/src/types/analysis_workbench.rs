use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnalyticalDatasetKind {
    RegionalData,
    RipaStops,
    RipaSearchBasis,
    CallsForService,
    CallsCodebook,
    NibrsOffenses,
    OperatingBudget,
    BudgetDimension,
}

impl AnalyticalDatasetKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::RegionalData => "REGIONAL_DATA",
            Self::RipaStops => "RIPA_STOPS",
            Self::RipaSearchBasis => "RIPA_SEARCH_BASIS",
            Self::CallsForService => "CALLS_FOR_SERVICE",
            Self::CallsCodebook => "CALLS_CODEBOOK",
            Self::NibrsOffenses => "NIBRS_OFFENSES",
            Self::OperatingBudget => "OPERATING_BUDGET",
            Self::BudgetDimension => "BUDGET_DIMENSION",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticalImportRequest {
    pub dataset_kind: AnalyticalDatasetKind,
    pub display_name: String,
    pub source_uri: Option<String>,
    pub local_path: String,
    pub observation_period_start: Option<String>,
    pub observation_period_end: Option<String>,
    pub partial_dataset: bool,
    #[serde(default)]
    pub worksheet: Option<String>,
    #[serde(default)]
    pub source_file_name: Option<String>,
    #[serde(default)]
    pub expected_source_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticalImportCandidate {
    pub document_id: u64,
    pub portal_key: String,
    pub file_name: String,
    pub local_path: String,
    pub source_uri: Option<String>,
    pub source_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticalImportResult {
    pub snapshot_id: String,
    pub dataset_kind: AnalyticalDatasetKind,
    pub display_name: String,
    pub table_name: String,
    pub imported_rows: usize,
    pub rejected_rows: usize,
    pub malformed_rows: usize,
    pub duplicate_key_rows: usize,
    pub schema_warnings: Vec<String>,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticalDatasetSummary {
    pub snapshot_id: String,
    pub dataset_kind: AnalyticalDatasetKind,
    pub display_name: String,
    pub table_name: String,
    pub imported_rows: usize,
    pub rejected_rows: usize,
    pub source_uri: Option<String>,
    pub source_sha256: String,
    pub retrieved_at: String,
    pub observation_period_start: Option<String>,
    pub observation_period_end: Option<String>,
    pub partial_dataset: bool,
    pub validation_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticalColumnSummary {
    pub table_name: String,
    pub column_name: String,
    pub logical_type: String,
    pub original_name: Option<String>,
    pub nullable: bool,
    pub dictionary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AnalyticalSchema {
    pub datasets: Vec<AnalyticalDatasetSummary>,
    pub columns: Vec<AnalyticalColumnSummary>,
    pub templates: Vec<SqlTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SqlTemplate {
    pub template_id: String,
    pub title: String,
    pub description: String,
    pub sql: String,
    pub denominator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SqlQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: usize,
    pub truncated: bool,
    pub elapsed_ms: u128,
    pub provenance: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SqlExportResult {
    pub output_path: String,
    pub row_count: usize,
    pub sha256: String,
}
