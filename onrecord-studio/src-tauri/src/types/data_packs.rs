use super::analysis_workbench::{AnalyticalDatasetKind, AnalyticalImportResult};
use serde::{Deserialize, Serialize};

/// A data pack whose signed manifest has verified against a pinned publisher key. From a local
/// folder every listed file has verified too; from a published pack only the manifest has, and
/// each file is checked when it is downloaded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataPackSummary {
    pub pack: String,
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
    /// `None`: the City source list does not publish this file, so the app does not import it.
    pub dataset_kind: Option<AnalyticalDatasetKind>,
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
