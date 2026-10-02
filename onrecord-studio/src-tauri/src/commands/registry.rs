//! Registry snapshots: import a local public-registry list and anchor extracted identifiers to it.

use std::path::PathBuf;

use tauri::State;

use crate::commands::validation::required_text;
use crate::error::AppError;
use crate::storage::corpus::CorpusIndex;
use crate::types::{CorpusEntityKind, RegistryAnchor, RegistryKind, RegistrySnapshotSummary};

/// Replaces the registry's snapshot with the file at `local_path`. Nothing is fetched: the operator
/// downloads the list themselves, and the file's digest is recorded with the snapshot.
#[tauri::command]
pub async fn import_registry_snapshot(
    registry: RegistryKind,
    local_path: String,
    index: State<'_, CorpusIndex>,
) -> Result<RegistrySnapshotSummary, AppError> {
    let path = required_text(&local_path, "Registry file path", 4096)?;
    index
        .import_registry_snapshot(registry, PathBuf::from(path))
        .await
}

#[tauri::command]
pub async fn list_registry_snapshots(
    index: State<'_, CorpusIndex>,
) -> Result<Vec<RegistrySnapshotSummary>, AppError> {
    index.registry_snapshots().await
}

/// What the imported registry says about one identifier value. A `REGISTERED` answer means the
/// number is registered to that name in that snapshot; it is not a statement about any party in
/// the documents the number was read from.
#[tauri::command]
pub async fn lookup_registry_anchor(
    identifier_kind: CorpusEntityKind,
    value: String,
    index: State<'_, CorpusIndex>,
) -> Result<RegistryAnchor, AppError> {
    let value = required_text(&value, "Identifier value", 64)?;
    index
        .registry_anchor(identifier_kind, value.to_string())
        .await
}
