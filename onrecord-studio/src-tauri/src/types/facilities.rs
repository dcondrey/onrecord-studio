use serde::{Deserialize, Serialize};

/// Inclusive latitude/longitude rectangle supplied by the user. Never inferred from addresses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FacilityBoundingBox {
    pub min_latitude: f64,
    pub min_longitude: f64,
    pub max_latitude: f64,
    pub max_longitude: f64,
}

/// A named area defined by exactly one of a bounding box or an explicit list of facility ids.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FacilityArea {
    pub name: String,
    #[serde(default)]
    pub bounding_box: Option<FacilityBoundingBox>,
    #[serde(default)]
    pub facility_ids: Vec<String>,
}

/// Column roles for one imported facility list, and the areas it is summarized over.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FacilityCoverageMapping {
    pub snapshot_id: String,
    pub name_column: String,
    #[serde(default)]
    pub id_column: Option<String>,
    #[serde(default)]
    pub address_column: Option<String>,
    #[serde(default)]
    pub latitude_column: Option<String>,
    #[serde(default)]
    pub longitude_column: Option<String>,
    #[serde(default)]
    pub hours_column: Option<String>,
    #[serde(default)]
    pub category_column: Option<String>,
    /// Category values (case-insensitive, exact) that identify a public restroom row.
    #[serde(default)]
    pub restroom_categories: Vec<String>,
    pub areas: Vec<FacilityArea>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FacilityHoursStatus {
    /// Daily hours parsed; an explicit "closed" is recorded with no open intervals.
    Recorded,
    /// Text is present but is not a day-independent daily schedule this parser accepts.
    Uninterpretable,
    NotRecorded,
}

/// Half-open interval `[start_minute, end_minute)` within one day; `end_minute` may be 1440.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FacilityOpenInterval {
    pub start_minute: u32,
    pub end_minute: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FacilityCoverageFacility {
    pub table_row: u64,
    pub facility_id: Option<String>,
    pub name: Option<String>,
    pub address: Option<String>,
    pub hours_text: Option<String>,
    pub hours_status: FacilityHoursStatus,
    pub open_intervals: Vec<FacilityOpenInterval>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FacilityAreaCoverage {
    pub area_name: String,
    pub boundary: String,
    pub public_restrooms: u64,
    pub with_recorded_hours: u64,
    pub hours_uninterpretable: u64,
    pub hours_not_recorded: u64,
    /// Minutes of the day in which at least one facility with recorded hours is open.
    /// `None` when no facility in the area has recorded hours.
    pub open_minutes_covered: Option<u32>,
    /// Minutes of the day in which no facility with recorded hours is open.
    /// `None` when no facility in the area has recorded hours.
    pub minutes_with_none_open: Option<u32>,
    pub unmatched_facility_ids: Vec<String>,
    pub facilities: Vec<FacilityCoverageFacility>,
    pub facilities_omitted: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FacilityCoverageReport {
    pub mapping: FacilityCoverageMapping,
    pub dataset_name: String,
    pub table_name: String,
    pub source_uri: Option<String>,
    pub source_sha256: String,
    pub total_rows: u64,
    pub public_restroom_rows: u64,
    /// Restroom rows without numeric coordinates; `None` when coordinates are not mapped.
    pub location_not_recorded: Option<u64>,
    pub areas: Vec<FacilityAreaCoverage>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FacilityCoverageWorkspace {
    pub reports: Vec<FacilityCoverageReport>,
    pub warnings: Vec<String>,
}
