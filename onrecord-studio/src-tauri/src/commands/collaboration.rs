//! IPC commands for serverless collaboration: the contributor trust list and the shared-folder
//! packet exchange. Nothing here uses the network.

use crate::{
    blocking,
    collaboration::{self, ContributorStore, SharedFolderListing, TrustedContributor},
    error::AppError,
    integrity::{load_or_create_signing_key, sha256_hex},
};
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalContributorIdentity {
    pub key_id: String,
}

#[tauri::command]
pub async fn get_local_contributor_identity(
    key_directory: State<'_, crate::integrity::SigningKeyDirectory>,
) -> Result<LocalContributorIdentity, AppError> {
    let key = load_or_create_signing_key(&key_directory.0).await?;
    let public_key = key.verifying_key().to_bytes();
    Ok(LocalContributorIdentity {
        key_id: sha256_hex(&public_key),
    })
}

#[tauri::command]
pub async fn list_trusted_contributors(
    store: State<'_, Arc<ContributorStore>>,
) -> Result<Vec<TrustedContributor>, AppError> {
    blocking::on_shared(&store, "Contributor list", ContributorStore::list).await
}

#[tauri::command]
pub async fn trust_contributor(
    key_id: String,
    label: String,
    store: State<'_, Arc<ContributorStore>>,
) -> Result<TrustedContributor, AppError> {
    blocking::on_shared(&store, "Contributor trust", move |store| {
        store.trust(&key_id, &label)
    })
    .await
}

#[tauri::command]
pub async fn untrust_contributor(
    key_id: String,
    store: State<'_, Arc<ContributorStore>>,
) -> Result<(), AppError> {
    blocking::on_shared(&store, "Contributor removal", move |store| {
        store.untrust(&key_id)
    })
    .await
}

#[tauri::command]
pub async fn list_shared_folder_packets(folder: String) -> Result<SharedFolderListing, AppError> {
    blocking::spawn("Shared folder listing", move || {
        collaboration::list_packets(&folder)
    })
    .await?
}

#[tauri::command]
pub async fn read_shared_folder_packet(
    folder: String,
    file_name: String,
) -> Result<String, AppError> {
    blocking::spawn("Shared folder read", move || {
        collaboration::read_packet(&folder, &file_name)
    })
    .await?
}

#[tauri::command]
pub async fn publish_packet_to_folder(
    folder: String,
    file_name: String,
    payload: String,
) -> Result<(), AppError> {
    blocking::spawn("Shared folder write", move || {
        collaboration::publish_packet(&folder, &file_name, &payload)
    })
    .await?
}
