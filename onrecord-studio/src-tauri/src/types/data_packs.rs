use super::analysis_workbench::{AnalyticalDatasetKind, AnalyticalImportResult};
use serde::{Deserialize, Serialize};

/// A data pack whose signed manifest has verified against a pinned publisher key. From a local
/// folder every listed file has verified too; from a published pack only the manifest has, and
/// each file is checked when it is downloaded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataPackSummary {
    pub pack: String,
    /// The pack keeps dated snapshots of each table's file; choosing a table of it downloads
    /// only the latest snapshot.
    pub snapshots: bool,
    pub pack_version: String,
    pub schema_version: String,
    /// When the pack builder finished, RFC 3339; its rows are the builder's retrieval.
    pub built_at: String,
    pub signer_key_id: String,
    pub licence_id: String,
    pub licence_url: String,
    pub disclaimer: String,
    /// SHA-256 of the verified manifest; an import names it, so it imports only this manifest.
    pub manifest_sha256: String,
    pub tables: Vec<DataPackTable>,
}

/// One conformed table of a pack and its original files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataPackTable {
    pub table: String,
    pub files: Vec<DataPackFileSummary>,
    /// Retrievals a snapshot pack recorded as unchanged from the table's latest snapshot.
    pub unchanged_checks: u64,
    /// The latest of those checks, RFC 3339; `None` when there is none.
    pub last_checked_at: Option<String>,
    /// Change records between the table's snapshots, which are listed but never imported.
    pub change_records: u64,
    /// What each of those change records compares and counts, oldest pair first.
    #[serde(default)]
    pub changes: Vec<DataPackChange>,
}

/// One change record between two retrievals of a table's publisher file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataPackChange {
    /// The change record's own path, as the manifest lists it.
    pub path: String,
    /// The earlier snapshot's original, as the manifest lists it.
    pub previous: String,
    /// When the earlier snapshot was retrieved, RFC 3339; `None` when the manifest no longer
    /// lists that original.
    pub previous_retrieved_at: Option<String>,
    /// The later snapshot's original.
    pub current: String,
    pub current_retrieved_at: Option<String>,
    pub added: u64,
    pub removed: u64,
    /// Changed records listed field by field; `None` when `whole_records`, because those records
    /// were listed as removed and added, so the changes are counted but not separated.
    pub changed: Option<u64>,
    /// More than half the records changed, so changed records are listed as removed and added.
    pub whole_records: bool,
}

/// Where an inspected pack is: a folder the user chose, or a published pack's kept manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
pub enum DataPackLocation {
    Folder { path: String },
    Published { pack: String, pack_version: String },
}

/// What one row of a change record says changed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataPackChangeKind {
    Added,
    Removed,
    Changed,
    ColumnAdded,
    ColumnRemoved,
}

/// One row of a change record, as the publisher's cell text; a field the row does not carry is
/// `None`, never an empty string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataPackChangeRow {
    pub change: DataPackChangeKind,
    pub publisher_record_id: Option<String>,
    /// Which repeat of the publisher key, in file order, the row compares.
    pub key_occurrence: Option<i64>,
    pub field: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub note: Option<String>,
}

/// A page of a change record's rows, in file order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataPackChangeRows {
    pub path: String,
    /// Rows in the whole record.
    pub total: u64,
    pub offset: u64,
    pub rows: Vec<DataPackChangeRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataPackFileSummary {
    pub path: String,
    /// The publisher's year; `None` for a table published as one file.
    pub year: Option<u16>,
    pub rows: u64,
    pub bytes: u64,
    pub partial: bool,
    pub source_url: String,
    /// When the pack builder retrieved the file from its publisher, RFC 3339.
    pub retrieved_at: String,
    /// `None`: no source list the app holds publishes this file, so the app does not import it.
    pub dataset_kind: Option<AnalyticalDatasetKind>,
    /// The file's own licence, as the signed manifest states it; `None` when the pack's licence
    /// covers it.
    pub licence: Option<DataPackFileLicence>,
}

/// The licence one file of a pack is published under, where a pack mixes publishers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataPackFileLicence {
    /// An SPDX id, or a named public-domain basis such as `public-domain-us-government`.
    pub id: String,
    pub url: String,
    /// The credit line the licence requires; `None` when it requires none.
    pub attribution: Option<String>,
    /// The disclaimer the publisher requires; `None` when it requires none.
    pub disclaimer: Option<String>,
}

/// A pack OnRecord publishes, and the public repository the app fetches it from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishedPack {
    pub pack: String,
    pub repository_url: String,
}

/// One table of a published pack to download, and the year of it for a table split by year.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishedPackSelection {
    pub table: String,
    pub year: Option<u16>,
}

/// What importing one selected file did: exactly one of `result` and `error` is set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataPackImportOutcome {
    pub path: String,
    pub table: String,
    pub year: Option<u16>,
    /// The manifest's retrieval time for this file.
    pub retrieved_at: String,
    pub result: Option<AnalyticalImportResult>,
    /// Set with `result`: the same bytes were already a stored snapshot, so nothing was imported
    /// and the pack was recorded beside it; `result` describes that snapshot.
    pub already_stored: Option<bool>,
    pub error: Option<String>,
}
