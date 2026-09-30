//! IPC commands for the shareable read-only bundle: a preview of exactly what would be shared, and
//! an export that writes it only for the content the caller confirmed.

use crate::{
    blocking,
    bundle::{
        self, content_sha256, parse_embedded, preview_of, render_html, Assembly, BundleDocument,
        BundlePreview, ShareableBundleResult, MAXIMUM_BUNDLE_BYTES,
    },
    commands::{
        profile::disclosure_context,
        report::validate_finding_ids,
        validation::{optional_text, MAXIMUM_PATH_BYTES},
    },
    error::AppError,
    integrity::sha256_hex,
    profile::ProfileStore,
    report::output::validate_output_path,
    storage::{corpus::CorpusIndex, sqlite::DuckDbRepository},
};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};
use tauri::State;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareableBundleRequest {
    pub finding_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareableBundleExportRequest {
    pub finding_ids: Vec<String>,
    /// The `contentSha256` of the preview the person confirmed. The export refuses when what would
    /// be written now hashes differently.
    pub confirmed_content_sha256: String,
    pub target_path: Option<String>,
}

/// Lists exactly what a bundle of these findings would contain and what it would leave out, and
/// writes nothing.
#[tauri::command]
pub async fn preview_shareable_bundle(
    request: ShareableBundleRequest,
    repository: State<'_, DuckDbRepository>,
    profile_store: State<'_, Arc<ProfileStore>>,
    corpus: State<'_, CorpusIndex>,
) -> Result<BundlePreview, AppError> {
    let ids = validate_finding_ids(request.finding_ids)?;
    let disclosure = disclosure_context(&profile_store, &repository, &corpus).await?;
    let assembly = bundle::prepare(&repository, &disclosure, ids).await?;
    preview_of(&assembly)
}

/// Writes the bundle as one HTML file into the managed report directory, never over an existing
/// file. Refuses unless `confirmed_content_sha256` matches the content as it stands now.
#[tauri::command]
pub async fn export_shareable_bundle(
    request: ShareableBundleExportRequest,
    repository: State<'_, DuckDbRepository>,
    report_directory: State<'_, PathBuf>,
    profile_store: State<'_, Arc<ProfileStore>>,
    corpus: State<'_, CorpusIndex>,
) -> Result<ShareableBundleResult, AppError> {
    let ShareableBundleExportRequest {
        finding_ids,
        confirmed_content_sha256,
        target_path,
    } = request;
    let ids = validate_finding_ids(finding_ids)?;
    let target_path = optional_text(target_path, "Bundle output path", MAXIMUM_PATH_BYTES)?;
    let disclosure = disclosure_context(&profile_store, &repository, &corpus).await?;
    let assembly = bundle::prepare(&repository, &disclosure, ids).await?;
    let target_path = target_path.unwrap_or_else(|| {
        let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        report_directory
            .join(format!("onrecord-bundle-{timestamp}.html"))
            .to_string_lossy()
            .into_owned()
    });
    let output_path = validate_output_path(&target_path, &report_directory).await?;
    write_bundle(
        assembly,
        &confirmed_content_sha256,
        output_path,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
}

/// Renders and atomically writes a confirmed assembly. Split from the command so the written file
/// can be read back in a test.
pub(crate) async fn write_bundle(
    assembly: Assembly,
    confirmed_content_sha256: &str,
    output_path: PathBuf,
    generated_at: String,
) -> Result<ShareableBundleResult, AppError> {
    let excluded_count = assembly.exclusions.len();
    if assembly.content.findings.is_empty() {
        return Err(AppError::Export(
            "Every selected finding was left out, so there is nothing to share".to_string(),
        ));
    }
    if content_sha256(&assembly.content)? != confirmed_content_sha256.trim() {
        return Err(AppError::Export(
            "The bundle's content changed since it was reviewed. Review it again before sharing."
                .to_string(),
        ));
    }
    let finding_count = assembly.content.findings.len();
    let document = BundleDocument::new(assembly.content, generated_at.clone())?;
    let html = render_html(&document)?;
    if html.len() > MAXIMUM_BUNDLE_BYTES {
        return Err(AppError::Export(format!(
            "The bundle exceeds the {MAXIMUM_BUNDLE_BYTES}-byte limit. Select fewer findings."
        )));
    }
    // The page that is written must read back as the data that was confirmed.
    if parse_embedded(&html)? != document {
        return Err(AppError::Internal(
            "The rendered bundle does not read back as its own data".to_string(),
        ));
    }
    let file_sha256 = sha256_hex(html.as_bytes());
    let destination = output_path.clone();
    blocking::spawn("Write shareable bundle", move || {
        crate::atomic_write::write_atomic(&destination, "the shareable bundle", html.as_bytes())
    })
    .await??;
    Ok(ShareableBundleResult {
        output_path: output_path.to_string_lossy().into_owned(),
        file_sha256,
        content_sha256: document.content_sha256,
        generated_at,
        finding_count,
        excluded_count,
    })
}
