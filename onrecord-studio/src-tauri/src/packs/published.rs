//! The packs OnRecord publishes on Hugging Face, as `Writerslogic/onrecord-<pack>`
//! (`docs/data-packs.md`). The app downloads a pack's signed manifest, then only the original
//! files the user chooses from it, each kept only once its size and SHA-256 match the manifest,
//! and imports them through [`super::import::import_files`].
//!
//! Every address is built here from the catalog's pack names and a path the verified manifest
//! lists; the frontend names a pack, a version and tables, never a URL or a path.

use super::catalog::pack_names;
use super::import::{file_kind, import_files, manifest_sha256, summarize};
use super::{
    check_pack_file, pack_relative, refused, verify_manifest, verify_manifest_bytes, PackFile,
    PackFileFormat, PackManifest, PACK_MANIFEST_FILE,
};
use crate::error::AppError;
use crate::ingestion::client::{ApiClient, HttpClientConfig};
use crate::storage::sqlite::DuckDbRepository;
use crate::storage::tabular::MAX_STREAMED_BYTES;
use crate::types::{DataPackImportOutcome, DataPackSummary, PublishedPack, PublishedPackSelection};
use ed25519_dalek::VerifyingKey;
use reqwest::Url;
use std::path::{Component, Path, PathBuf};

const PUBLISHER: &str = "Writerslogic";
const HUB: &str = "https://huggingface.co/";
/// A manifest lists a pack's files; `san-diego-reports` 2026.10.7 (94 files) is 123,432 bytes.
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;

/// The app-managed directory published packs are downloaded into, `<app data>/packs`. A pack's
/// files live under `<pack>/<packVersion>/` inside it.
pub struct PublishedPackRoot(pub PathBuf);

fn repository(pack: &str) -> String {
    format!("onrecord-{pack}")
}

/// Every pack OnRecord publishes.
pub(crate) fn published_packs() -> Vec<PublishedPack> {
    pack_names()
        .map(|pack| PublishedPack {
            pack: pack.into(),
            repository_url: format!("{HUB}datasets/{PUBLISHER}/{}", repository(pack)),
        })
        .collect()
}

fn published_pack(pack: &str) -> Result<&'static str, AppError> {
    pack_names()
        .find(|name| *name == pack)
        .ok_or_else(|| AppError::DataParsing(format!("'{pack}' is not a pack OnRecord publishes")))
}

/// The address of `relative`, a path that passed [`pack_relative`], in `pack`'s repository. Each
/// part is pushed as one escaped segment, so no part can become a query, fragment or new path.
fn file_url(pack: &'static str, relative: &Path) -> Result<Url, AppError> {
    let invalid = || AppError::Internal(format!("Published pack path {}", relative.display()));
    let repository = repository(pack);
    let mut url = Url::parse(HUB).map_err(|_| invalid())?;
    {
        let mut segments = url.path_segments_mut().map_err(|_| invalid())?;
        segments.clear().extend([
            "datasets",
            PUBLISHER,
            repository.as_str(),
            "resolve",
            "main",
        ]);
        for part in relative.components() {
            let Component::Normal(part) = part else {
                return Err(invalid());
            };
            segments.push(part.to_str().ok_or_else(invalid)?);
        }
    }
    Ok(url)
}

/// `<root>/<pack>/<version>`, where `version` must be one plain path part.
fn version_directory(root: &Path, pack: &'static str, version: &str) -> Result<PathBuf, AppError> {
    let version = pack_relative(version)
        .filter(|relative| relative.components().count() == 1)
        .ok_or_else(|| AppError::DataParsing(format!("'{version}' is not a pack version")))?;
    Ok(root.join(pack).join(version))
}

fn client() -> Result<ApiClient, AppError> {
    ApiClient::new(HttpClientConfig {
        max_document_bytes: MAX_STREAMED_BYTES as usize,
        ..HttpClientConfig::default()
    })
}

fn staged_name(parent: &Path) -> PathBuf {
    parent.join(format!(".{}.partial", uuid::Uuid::new_v4()))
}

async fn blocking<T: Send + 'static>(
    task: &'static str,
    work: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    crate::blocking::spawn(task, work).await?
}

/// Keeps a downloaded manifest if it verifies and is `pack`'s, as `<pack>/<version>/manifest.json`.
fn keep_manifest(
    root: &Path,
    pack: &'static str,
    staged: &Path,
    pinned: &[VerifyingKey],
) -> Result<DataPackSummary, AppError> {
    let bytes = std::fs::read(staged)
        .map_err(|error| AppError::Storage(format!("Downloaded pack manifest: {error}")))?;
    let signed = verify_manifest_bytes(&bytes, pinned)?;
    if signed.manifest.pack != pack {
        return Err(refused(format!(
            "its manifest is for {}, not {pack}",
            signed.manifest.pack
        )));
    }
    let directory = version_directory(root, pack, &signed.manifest.pack_version)?;
    std::fs::create_dir_all(&directory)
        .and_then(|()| std::fs::rename(staged, directory.join(PACK_MANIFEST_FILE)))
        .map_err(|error| AppError::Storage(format!("Pack directory: {error}")))?;
    summarize(signed.manifest, &signed.signer_key_id)
}

/// Downloads `pack`'s signed manifest and describes what it lists. No listed file is fetched.
pub(crate) async fn fetch_manifest(
    root: PathBuf,
    pack: &str,
    pinned: Vec<VerifyingKey>,
) -> Result<DataPackSummary, AppError> {
    let pack = published_pack(pack)?;
    let pack_root = root.join(pack);
    tokio::fs::create_dir_all(&pack_root)
        .await
        .map_err(|error| AppError::Storage(format!("Pack directory: {error}")))?;
    let staged = staged_name(&pack_root);
    client()?
        .download_url_to_path_within(
            file_url(pack, Path::new(PACK_MANIFEST_FILE))?,
            &staged,
            MAX_MANIFEST_BYTES,
        )
        .await?;
    let kept = staged.clone();
    let summary = blocking("Verify data pack manifest", move || {
        keep_manifest(&root, pack, &kept, &pinned)
    })
    .await;
    if summary.is_err() {
        let _ = tokio::fs::remove_file(&staged).await;
    }
    summary
}

/// The original files `selection` names, each once, refused as a whole before anything is
/// downloaded if any choice matches no file or a file this app cannot import.
fn plan(
    manifest: &PackManifest,
    selection: &[PublishedPackSelection],
) -> Result<Vec<PackFile>, AppError> {
    let mut planned: Vec<PackFile> = Vec::new();
    for chosen in selection {
        let mut matched = false;
        for file in manifest.files.iter().filter(|file| {
            file.format == PackFileFormat::Original
                && file.table == chosen.table
                && file.year == chosen.year
        }) {
            matched = true;
            if file.bytes > MAX_STREAMED_BYTES {
                return Err(AppError::DataParsing(format!(
                    "{} is larger than this app imports",
                    file.path
                )));
            }
            if file_kind(manifest, file).is_none() {
                return Err(AppError::DataParsing(format!(
                    "This app cannot import {}",
                    file.path
                )));
            }
            if !planned.iter().any(|kept| kept.path == file.path) {
                planned.push(file.clone());
            }
        }
        if !matched {
            let year = chosen
                .year
                .map(|year| format!(" {year}"))
                .unwrap_or_default();
            return Err(AppError::DataParsing(format!(
                "This pack lists no {}{year} file",
                chosen.table
            )));
        }
    }
    if planned.is_empty() {
        return Err(AppError::DataParsing(
            "Choose at least one table to download".into(),
        ));
    }
    Ok(planned)
}

/// Moves a downloaded file to `destination` only if it is the file the manifest lists; otherwise
/// deletes it.
fn keep_verified(staged: &Path, destination: &Path, file: &PackFile) -> Result<(), AppError> {
    if let Err(error) = check_pack_file(staged, file) {
        let _ = std::fs::remove_file(staged);
        return Err(match error {
            AppError::Provenance(detail) => AppError::Provenance(format!(
                "{detail}; the download was deleted and nothing was imported from it"
            )),
            other => other,
        });
    }
    std::fs::rename(staged, destination).map_err(|error| {
        let _ = std::fs::remove_file(staged);
        AppError::Storage(format!("{}: {error}", file.path))
    })
}

/// Fetches `file` into `directory` unless a verified copy is already there.
async fn fetch_file(
    client: &ApiClient,
    root: &Path,
    pack: &'static str,
    directory: &Path,
    file: &PackFile,
) -> Result<(), AppError> {
    let relative = pack_relative(&file.path)
        .ok_or_else(|| refused(format!("it lists a path outside the pack: {}", file.path)))?;
    let destination = directory.join(relative);
    let (present, listed) = (destination.clone(), file.clone());
    if blocking("Check data pack file", move || {
        check_pack_file(&present, &listed)
    })
    .await
    .is_ok()
    {
        return Ok(());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| AppError::Internal(format!("Pack file {}", file.path)))?
        .to_path_buf();
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|error| AppError::Storage(format!("Pack directory: {error}")))?;
    let (inside, managed) = (
        tokio::fs::canonicalize(&parent).await,
        tokio::fs::canonicalize(root).await,
    );
    if !matches!((inside, managed), (Ok(inside), Ok(managed)) if inside.starts_with(&managed)) {
        return Err(refused(format!(
            "{} would be written outside the packs directory",
            file.path
        )));
    }
    let maximum_bytes = usize::try_from(file.bytes)
        .map_err(|_| AppError::DataParsing(format!("{} is too large", file.path)))?;
    let staged = staged_name(&parent);
    client
        .download_url_to_path_within(file_url(pack, relative)?, &staged, maximum_bytes)
        .await?;
    let listed = file.clone();
    blocking("Verify data pack file", move || {
        keep_verified(&staged, &destination, &listed)
    })
    .await
}

/// Downloads the original files `selection` names from `pack` `pack_version` (the manifest
/// [`fetch_manifest`] kept, unchanged since `inspected_manifest_sha256`), keeps each only once it
/// matches the manifest, and imports the kept files. A file that fails to download or verify is
/// reported beside the others' results.
pub(crate) async fn fetch_files(
    repository: &DuckDbRepository,
    root: PathBuf,
    pack: &str,
    pack_version: &str,
    inspected_manifest_sha256: &str,
    selection: Vec<PublishedPackSelection>,
    pinned: Vec<VerifyingKey>,
) -> Result<Vec<DataPackImportOutcome>, AppError> {
    let pack = published_pack(pack)?;
    let directory = version_directory(&root, pack, pack_version)?;
    let (manifest_directory, manifest_keys) = (directory.clone(), pinned.clone());
    let signed = blocking("Verify data pack manifest", move || {
        verify_manifest(&manifest_directory, &manifest_keys)
    })
    .await?;
    if signed.manifest.pack != pack
        || signed.manifest.pack_version != pack_version
        || !manifest_sha256(&signed.manifest)?.eq_ignore_ascii_case(inspected_manifest_sha256)
    {
        return Err(refused(
            "its manifest has changed since it was fetched; fetch it again",
        ));
    }
    let planned = plan(&signed.manifest, &selection)?;
    let client = client()?;
    let mut kept = Vec::new();
    let mut outcomes = Vec::new();
    for file in &planned {
        match fetch_file(&client, &root, pack, &directory, file).await {
            Ok(()) => kept.push(file.path.clone()),
            Err(error) => outcomes.push(DataPackImportOutcome {
                path: file.path.clone(),
                table: file.table.clone(),
                year: file.year,
                retrieved_at: file.retrieved_at.clone(),
                result: None,
                already_stored: None,
                error: Some(error.to_string()),
            }),
        }
    }
    if !kept.is_empty() {
        outcomes.extend(
            import_files(
                repository,
                directory,
                inspected_manifest_sha256,
                kept,
                pinned,
            )
            .await?,
        );
    }
    outcomes.sort_by_key(|outcome| planned.iter().position(|file| file.path == outcome.path));
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrity::sha256_hex;
    use crate::packs::PackLicence;

    const ORIGINAL: &[u8] = b"incident_num,date_time\nE1,2025-01-01 00:00:00\n";

    fn listed(path: &str, table: &str, year: Option<u16>) -> PackFile {
        PackFile {
            path: path.into(),
            table: table.into(),
            format: PackFileFormat::Original,
            source_url: format!(
                "https://seshat.datasd.org/police_calls_for_service/pd_calls_for_service_{}_datasd.csv",
                year.unwrap_or(2025)
            ),
            retrieved_at: "2026-10-06T00:00:00Z".into(),
            sha256: sha256_hex(ORIGINAL),
            bytes: ORIGINAL.len() as u64,
            rows: 1,
            excluded_rows: None,
            year,
            partial: false,
            last_modified: None,
            etag: None,
        }
    }

    fn manifest(files: Vec<PackFile>) -> PackManifest {
        PackManifest {
            pack: "san-diego-reports".into(),
            pack_version: "2026.10.7".into(),
            schema_version: "1.0".into(),
            built_at: "2026-10-07T00:00:00Z".into(),
            licence: PackLicence {
                id: "ODC-PDDL-1.0".into(),
                url: "https://opendefinition.org/licenses/odc-pddl/".into(),
                disclaimer: "Informational only.".into(),
            },
            files,
        }
    }

    /// A served file whose bytes differ from the manifest's is deleted and never put in place, so
    /// nothing can import it; the listed bytes are kept.
    #[test]
    fn a_download_is_kept_only_when_it_matches_its_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let file = listed(
            "originals/police_calls/2025.csv",
            "police_calls",
            Some(2025),
        );
        let destination = directory.path().join("2025.csv");

        let staged = directory.path().join(".served.partial");
        let mut served = ORIGINAL.to_vec();
        served[0] ^= 1;
        std::fs::write(&staged, &served).unwrap();
        let refused = keep_verified(&staged, &destination, &file).unwrap_err();
        assert!(refused.to_string().contains("SHA-256"), "{refused}");
        assert!(!staged.exists());
        assert!(!destination.exists());

        std::fs::write(&staged, ORIGINAL).unwrap();
        keep_verified(&staged, &destination, &file).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), ORIGINAL);
        assert!(!staged.exists());
    }

    /// Every address is the pack's repository plus the manifest's path, each part escaped, and a
    /// version or pack outside the closed lists is refused before any request.
    #[test]
    fn addresses_come_only_from_the_catalog_and_the_manifest() {
        let url = file_url(
            "san-diego-reports",
            Path::new("originals/police_calls/a?b#c.csv"),
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://huggingface.co/datasets/Writerslogic/onrecord-san-diego-reports/resolve/main/originals/police_calls/a%3Fb%23c.csv"
        );
        assert!(published_pack("san-diego-elsewhere").is_err());
        let root = Path::new("/packs");
        for version in ["..", "a/b", "", "/etc"] {
            assert!(
                version_directory(root, "san-diego-reports", version).is_err(),
                "{version}"
            );
        }
        assert_eq!(
            version_directory(root, "san-diego-reports", "2026.10.7").unwrap(),
            root.join("san-diego-reports/2026.10.7")
        );
    }

    /// A choice is resolved to files before the first download: every year chosen and only those,
    /// and a choice the manifest does not list refuses the whole selection.
    #[test]
    fn a_selection_downloads_only_the_chosen_tables_and_years() {
        let manifest = manifest(vec![
            listed(
                "originals/police_calls/pd_calls_for_service_2024_datasd.csv",
                "police_calls",
                Some(2024),
            ),
            listed(
                "originals/police_calls/pd_calls_for_service_2025_datasd.csv",
                "police_calls",
                Some(2025),
            ),
        ]);
        let choose = |year| PublishedPackSelection {
            table: "police_calls".into(),
            year,
        };
        let planned = plan(&manifest, &[choose(Some(2025)), choose(Some(2025))]).unwrap();
        assert_eq!(
            planned.iter().map(|file| file.year).collect::<Vec<_>>(),
            [Some(2025)]
        );
        assert!(plan(&manifest, &[choose(Some(2025)), choose(Some(2019))]).is_err());
        assert!(plan(&manifest, &[]).is_err());
    }
}
