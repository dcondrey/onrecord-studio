use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Identifies the stable projection a research snapshot carries. Bump only when a documented
/// table or column changes meaning or disappears; adding a table or column keeps the version.
pub const RESEARCH_SNAPSHOT_SCHEMA: &str = "onrecord-research-snapshot-v1";

/// What a reader needs to cite a query result: which corpus state answered it, and that the file
/// they hold is the one this installation wrote. Signed as a sidecar beside the `.duckdb` file and
/// repeated (minus the file digest, which cannot describe itself) in the `snapshot_manifest` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSnapshotManifest {
    pub schema: String,
    pub app_version: String,
    pub created_at: String,
    pub file_name: String,
    pub file_sha256: String,
    /// SHA-256 over every document's (portal, id, extracted-text digest), in key order. Two
    /// snapshots with the same digest hold the same evidence corpus whatever their row layout.
    pub corpus_state_digest: String,
    /// SHA-256 over every finding's (id, last seen), in id order, dismissed ones included.
    pub findings_digest: String,
    pub table_rows: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSnapshotResult {
    pub path: String,
    pub manifest_path: String,
    pub manifest: ResearchSnapshotManifest,
}
