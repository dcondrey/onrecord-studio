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
    /// Set only by the app when the imported file is a filtered subset of a publisher's file.
    #[serde(skip)]
    pub derived_subset: Option<DerivedSubset>,
}

/// Which publisher file a filtered import came from, so the snapshot's own digest (of the subset)
/// is never mistaken for the digest of what the publisher released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedSubset {
    pub publisher_sha256: String,
    pub publisher_rows: usize,
    pub filter: String,
}

/// What imported tables show about SDMC 63.0404/63.0405 enforcement. Observations only: none of
/// these fields asserts that a citation was unlawful. `None` means the inputs were not imported,
/// which is distinct from a count of zero.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EncampmentOrdinanceReport {
    pub night_hours: Option<NightHoursObservation>,
    pub locations: Option<EncampmentLocationObservation>,
    pub not_measured: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObservationSource {
    pub snapshot_id: String,
    pub display_name: String,
    pub source_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NightHoursObservation {
    pub sources: Vec<ObservationSource>,
    /// Earliest and latest activity of any section in the imported logs, as recorded.
    pub first_activity: Option<String>,
    pub last_activity: Option<String>,
    pub sections: Vec<NightHoursSectionCount>,
    /// Activities inside the window or exactly on one of its endpoints.
    pub listed_activities: Vec<RecordedActivity>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NightHoursSectionCount {
    pub section: String,
    pub activity_type: String,
    pub activities: usize,
    pub inside_window: usize,
    pub at_window_boundary: usize,
    pub outside_window: usize,
    pub time_not_recorded: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordedActivity {
    pub activity_number: String,
    pub activity_type: String,
    pub section: String,
    pub recorded_at: String,
    pub at_window_boundary: bool,
    pub location: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EncampmentLocationObservation {
    pub incident_sources: Vec<ObservationSource>,
    pub school_source: Option<ObservationSource>,
    pub station_source: Option<ObservationSource>,
    pub sections: Vec<LocationSectionCount>,
    pub incidents: Vec<EncampmentIncidentDistance>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocationSectionCount {
    pub section: String,
    pub offenses: usize,
    pub distinct_cases: usize,
    pub not_geocoded: usize,
    /// `None` unless both the school and the station layer were imported.
    pub within_near_band: Option<usize>,
    pub within_far_band: Option<usize>,
    pub beyond_far_band: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EncampmentIncidentDistance {
    pub case_number: String,
    pub occurred_on: Option<String>,
    pub section: String,
    pub block_address: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub nearest_school: Option<NearestFeature>,
    pub nearest_station: Option<NearestFeature>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NearestFeature {
    pub name: String,
    pub distance_feet: f64,
}

/// A published City table the app fetches by name. The frontend names one and, where the publisher
/// splits by year, a year; it never supplies a URL.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CityDataset {
    NibrsOffenses,
    /// SDPD calls for service, one published file per year.
    CallsForService,
    GetItDoneEncampments,
    GetItDoneOpen,
    /// A closed-year file, kept only for reports already seen in an imported open copy.
    GetItDoneOutcomes,
    Schools,
    TransitStops,
    /// Every published year of the operating budget in one file.
    OperatingBudget,
    /// SDPD stop records (RIPA): one row per person stopped.
    RipaStops,
    RipaStopReasons,
    RipaStopResults,
    RipaPropertySeizureBasis,
    /// Emergency-shelter projects per year, as UCSD's Homelessness Hub compiled them from the
    /// Regional Task Force on Homelessness Housing Inventory Count.
    ShelterInventory,
    /// Every published year of operating actuals in one file: the budget file's columns without
    /// a budget cycle.
    OperatingActuals,
    /// Every published fiscal year of the Capital Improvements Program budget in one file.
    CapitalBudget,
    /// Every published fiscal year of Capital Improvements Program actuals in one file.
    CapitalActuals,
    /// City-owned property sites, replaced by the publisher daily.
    CityProperties,
    /// The parcel numbers (APNs) of each City-owned site.
    CityParcels,
    /// Leases and permits on City-owned property.
    CityLeases,
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
    /// Cells kept as NULL because their text did not convert to the column's type.
    pub unconverted_cells: usize,
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
    /// Cells stored as NULL because their text did not convert, per column alias. Snapshots from
    /// before this was counted have none: that import rejected such rows instead.
    #[serde(default)]
    pub unconverted_by_column: std::collections::BTreeMap<String, usize>,
}

impl AnalyticalDatasetSummary {
    pub fn unconverted_cells(&self) -> usize {
        self.unconverted_by_column.values().sum()
    }

    /// Unconverted cells in the named columns, so a gate can ignore a column it never reads.
    pub fn unconverted_in<'a>(&self, columns: impl IntoIterator<Item = &'a str>) -> usize {
        columns
            .into_iter()
            .filter_map(|column| self.unconverted_by_column.get(column))
            .sum()
    }
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
