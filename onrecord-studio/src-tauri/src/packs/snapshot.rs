//! The snapshot pack (`docs/data-packs.md`, Snapshots): each run retrieves every file a publisher
//! overwrites in place and extends the published pack's cumulative manifest. A file whose bytes
//! equal its latest snapshot gains a check; any other is kept as a new dated snapshot with its
//! conformed Parquet copy and a change record against the snapshot before it. Published files are
//! never rewritten: a run adds files and checks, and signs the whole manifest again.

use super::builder::{
    count_rows, licence, publisher_key, require_signs, scratch_database, write_conformed_parquet,
    PublisherKey,
};
use super::catalog::{snapshot_tables, SNAPSHOT_PACK};
use super::changes::write_change_record;
use super::published::{fetch_listed_file, fetch_signed_manifest};
use super::{
    check_pack_file, latest_retrieved, pinned_pack_keys, verify_manifest, PackCheck, PackFile,
    PackFileFormat, PackManifest, PinnedPackKey, PACK_MANIFEST_FILE,
};
use crate::error::AppError;
use crate::ingestion::client::{ApiClient, HttpClientConfig};
use crate::ingestion::queue::dataset_source;
use crate::storage::tabular::MAX_STREAMED_BYTES;
use crate::types::AnalyticalDatasetKind;
use chrono::{DateTime, Datelike, Utc};
use reqwest::Url;
use std::path::{Path, PathBuf};

/// The new files of a run, one relative path per line, for the incremental upload.
const NEW_FILES: &str = "new-files.txt";
/// The tables a run could not snapshot, one per line, so the scheduled job can fail visibly
/// after publishing the others.
const NOT_SNAPSHOTTED: &str = "not-snapshotted.txt";
/// The tables kept as a new snapshot without their Parquet copy or change record, one
/// `<table>: <reason>` per line. The snapshot's original is published regardless: it is the
/// history, and the comparison can be made from it later.
const INCOMPLETE: &str = "incomplete.txt";
/// Under the work directory: downloads not yet placed, and published files fetched for a change
/// record. Neither is ever listed or uploaded.
const STAGING: &str = ".staging";
const PUBLISHED_CACHE: &str = ".published";

/// One publisher file the snapshot pack follows.
pub(super) struct SnapshotTarget {
    pub table: &'static str,
    pub url: Url,
    pub file_name: String,
    pub partial: bool,
    pub kind: AnalyticalDatasetKind,
}

/// What one retrieval received.
pub(super) struct Retrieved {
    pub retrieved_at: DateTime<Utc>,
    pub sha256: String,
    pub bytes: u64,
    pub last_modified: Option<String>,
    pub etag: Option<String>,
}

/// Where a run's bytes come from: the publishers and the published pack, or local files in tests.
pub(super) trait SnapshotSource {
    /// Retrieves `url` into `destination`, which does not exist yet.
    async fn retrieve(&self, url: &Url, destination: &Path) -> Result<Retrieved, AppError>;
    /// A published snapshot original, checked against `file`, as a local path the run may delete.
    async fn published(&self, file: &PackFile) -> Result<PathBuf, AppError>;
}

struct Network {
    client: ApiClient,
    cache: PathBuf,
    pack: String,
    /// The published manifest's version, under which [`fetch_signed_manifest`] kept it.
    pack_version: Option<String>,
}

impl SnapshotSource for Network {
    async fn retrieve(&self, url: &Url, destination: &Path) -> Result<Retrieved, AppError> {
        let downloaded = self
            .client
            .download_url_to_path(url.clone(), destination)
            .await?;
        Ok(Retrieved {
            retrieved_at: DateTime::parse_from_rfc3339(&downloaded.acquisition.retrieved_at)
                .map_err(|error| AppError::Internal(format!("Retrieval time: {error}")))?
                .with_timezone(&Utc),
            sha256: downloaded.sha256,
            bytes: downloaded.byte_count,
            last_modified: downloaded.acquisition.last_modified,
            etag: downloaded.acquisition.etag,
        })
    }

    async fn published(&self, file: &PackFile) -> Result<PathBuf, AppError> {
        let version = self.pack_version.as_deref().ok_or_else(|| {
            AppError::Internal(format!("{} is listed by no published manifest", file.path))
        })?;
        fetch_listed_file(&self.cache, &self.pack, version, file).await
    }
}

/// A file's place in time as a path part: its UTC retrieval second.
fn stamp(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H%M%SZ").to_string()
}

fn io_error(context: &'static str) -> impl Fn(std::io::Error) -> AppError {
    move |error| AppError::Storage(format!("{context}: {error}"))
}

/// The snapshot pack's publisher files.
fn targets(current_year: i32) -> Result<Vec<SnapshotTarget>, AppError> {
    snapshot_tables(current_year)
        .into_iter()
        .map(|entry| {
            let source = dataset_source(entry.dataset, None, current_year)?;
            Ok(SnapshotTarget {
                table: entry.table,
                url: Url::parse(&source.url)
                    .map_err(|error| AppError::Internal(format!("Pack source URL: {error}")))?,
                file_name: source.file_name,
                partial: source.partial,
                kind: source.kind,
            })
        })
        .collect()
}

/// Extends the published `pack` into `work`, a directory that must not exist yet: its new files,
/// the signed cumulative `manifest.json`, and `new-files.txt`. Returns the manifest's path.
pub async fn snapshot(pack: &str, work: &Path) -> Result<PathBuf, AppError> {
    if pack != SNAPSHOT_PACK {
        return Err(AppError::DataParsing(format!(
            "'{pack}' is not a snapshot pack; the snapshot pack is {SNAPSHOT_PACK}"
        )));
    }
    let key = publisher_key()?;
    require_signs(&key, pack)?;
    if work.exists() {
        return Err(AppError::Storage(format!(
            "{} already exists; a snapshot run starts in a new directory",
            work.display()
        )));
    }
    let cache = work.join(PUBLISHED_CACHE);
    tokio::fs::create_dir_all(&cache)
        .await
        .map_err(io_error("Snapshot directory"))?;
    let pinned = pinned_pack_keys();
    let previous = fetch_signed_manifest(cache.clone(), pack, pinned.clone())
        .await?
        .map(|signed| signed.manifest);
    match &previous {
        Some(manifest) => eprintln!(
            "Extending {pack} {} ({} files, {} checks)",
            manifest.pack_version,
            manifest.files.len(),
            manifest.checks.len()
        ),
        None => eprintln!("{pack} has no published manifest; starting it"),
    }
    let source = Network {
        client: ApiClient::new(HttpClientConfig {
            max_document_bytes: MAX_STREAMED_BYTES as usize,
            ..HttpClientConfig::default()
        })?,
        cache: cache.clone(),
        pack: pack.into(),
        pack_version: previous
            .as_ref()
            .map(|manifest| manifest.pack_version.clone()),
    };
    let started = Utc::now();
    let manifest = run(
        pack,
        work,
        previous,
        &targets(started.year())?,
        &source,
        &key,
        &pinned,
        started,
    )
    .await;
    let _ = tokio::fs::remove_dir_all(&cache).await;
    manifest
}

/// What one target added: a check, or new files.
enum Outcome {
    Unchanged(PackCheck),
    /// New files, and why a Parquet copy or change record is missing from them, if one is.
    Snapshot(Vec<PackFile>, Option<String>),
}

async fn snapshot_target<S: SnapshotSource>(
    work: &Path,
    manifest: &PackManifest,
    target: &SnapshotTarget,
    source: &S,
    pack_version: &str,
) -> Result<Outcome, AppError> {
    let staged = work
        .join(STAGING)
        .join(format!("{}.download", uuid::Uuid::new_v4()));
    let retrieved = source.retrieve(&target.url, &staged).await?;
    let previous = latest_retrieved(manifest.files.iter().filter(|file| {
        file.format == PackFileFormat::Original
            && file.table == target.table
            && Url::parse(&file.source_url).is_ok_and(|url| url == target.url)
    }));
    if let Some(previous) = previous.filter(|previous| {
        previous.bytes == retrieved.bytes && previous.sha256.eq_ignore_ascii_case(&retrieved.sha256)
    }) {
        tokio::fs::remove_file(&staged)
            .await
            .map_err(io_error("Staged download"))?;
        return Ok(Outcome::Unchanged(PackCheck {
            table: target.table.into(),
            source_url: target.url.to_string(),
            checked_at: retrieved.retrieved_at.to_rfc3339(),
            sha256: retrieved.sha256,
            bytes: retrieved.bytes,
            last_modified: retrieved.last_modified,
            etag: retrieved.etag,
            matches: previous.path.clone(),
        }));
    }
    let stamp = stamp(retrieved.retrieved_at);
    let relative = format!("originals/{}/{stamp}/{}", target.table, target.file_name);
    if manifest.files.iter().any(|file| file.path == relative) {
        return Err(AppError::Internal(format!(
            "{relative} is already published; a snapshot is never replaced"
        )));
    }
    let path = work.join(&relative);
    tokio::fs::create_dir_all(path.parent().expect("a file under originals"))
        .await
        .map_err(io_error("Snapshot directory"))?;
    tokio::fs::rename(&staged, &path)
        .await
        .map_err(io_error("Snapshot original"))?;
    let original = PackFile {
        path: relative,
        table: target.table.into(),
        format: PackFileFormat::Original,
        source_url: target.url.to_string(),
        retrieved_at: retrieved.retrieved_at.to_rfc3339(),
        sha256: retrieved.sha256,
        bytes: retrieved.bytes,
        rows: count_rows(&path, &target.file_name, None).await?,
        excluded_rows: None,
        year: None,
        partial: target.partial,
        last_modified: retrieved.last_modified,
        etag: retrieved.etag,
        changes: None,
        licence: None,
    };
    // From here the original is kept whatever else fails: its bytes are the history.
    let mut files = vec![original.clone()];
    // One scratch database per file, removed as soon as its records are written.
    let conformed = async {
        let (guard, scratch) = scratch_database().await?;
        let (parquet, current) = write_conformed_parquet(
            &scratch,
            work,
            &original,
            target.kind.clone(),
            None,
            pack_version,
        )
        .await?;
        Ok::<_, AppError>((guard, scratch, parquet, current))
    }
    .await;
    let (_scratch, scratch, current) = match conformed {
        Ok((guard, scratch, parquet, current)) => {
            files.push(parquet);
            (guard, scratch, current)
        }
        Err(error) => {
            let missing = format!("no Parquet copy: {error}");
            return Ok(Outcome::Snapshot(files, Some(missing)));
        }
    };
    let Some(previous) = previous else {
        return Ok(Outcome::Snapshot(files, None));
    };
    let compared = async {
        let previous_path = source.published(previous).await?;
        let change = write_change_record(
            &scratch,
            work,
            previous,
            &previous_path,
            &original,
            &current,
            target.kind.clone(),
        )
        .await;
        let _ = tokio::fs::remove_file(&previous_path).await;
        change
    }
    .await;
    Ok(match compared {
        Ok(change) => {
            files.push(change);
            Outcome::Snapshot(files, None)
        }
        Err(error) => Outcome::Snapshot(
            files,
            Some(format!(
                "no change record against {}: {error}",
                previous.path
            )),
        ),
    })
}

/// One run over `targets` against the published manifest `previous` (`None` before the first),
/// writing into `work`. A target that fails is reported and left out, so the others' history is
/// still kept; a run in which every target fails writes nothing.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run<S: SnapshotSource>(
    pack: &str,
    work: &Path,
    previous: Option<PackManifest>,
    targets: &[SnapshotTarget],
    source: &S,
    key: &PublisherKey,
    pinned: &[PinnedPackKey],
    started: DateTime<Utc>,
) -> Result<PathBuf, AppError> {
    let pack_version = stamp(started);
    let mut manifest = previous.unwrap_or_else(|| PackManifest {
        pack: pack.into(),
        pack_version: String::new(),
        schema_version: "1.0".into(),
        built_at: String::new(),
        licence: licence(),
        files: Vec::new(),
        key_id: None,
        checks: Vec::new(),
    });
    tokio::fs::create_dir_all(work.join(STAGING))
        .await
        .map_err(io_error("Snapshot directory"))?;
    let mut new_files: Vec<String> = Vec::new();
    let mut failed: Vec<&str> = Vec::new();
    let mut incomplete: Vec<String> = Vec::new();
    for target in targets {
        match snapshot_target(work, &manifest, target, source, &pack_version).await {
            Ok(Outcome::Unchanged(check)) => {
                eprintln!("{}: unchanged since {}", target.table, check.matches);
                manifest.checks.push(check);
            }
            Ok(Outcome::Snapshot(files, missing)) => {
                for file in &files {
                    eprintln!("{}: {} rows, {} bytes", file.path, file.rows, file.bytes);
                }
                if let Some(missing) = missing {
                    eprintln!("{}: snapshotted with {missing}", target.table);
                    incomplete.push(format!(
                        "{}: {}\n",
                        target.table,
                        missing.replace('\n', " ")
                    ));
                }
                new_files.extend(files.iter().map(|file| file.path.clone()));
                manifest.files.extend(files);
            }
            Err(error) => {
                failed.push(target.table);
                eprintln!("{}: not snapshotted: {error}", target.table);
            }
        }
    }
    let _ = tokio::fs::remove_dir_all(work.join(STAGING)).await;
    if failed.len() == targets.len() {
        return Err(AppError::Network(format!(
            "No file of {pack} could be snapshotted; nothing was written"
        )));
    }
    manifest.pack_version = pack_version;
    manifest.built_at = Utc::now().to_rfc3339();
    manifest.key_id = key.key_id.clone();
    let signed =
        crate::integrity::sign_package(manifest, &key.signing_key).map_err(AppError::Internal)?;
    let manifest_path = work.join(PACK_MANIFEST_FILE);
    let bytes = serde_json::to_vec_pretty(&signed)
        .map_err(|error| AppError::Internal(format!("Pack manifest: {error}")))?;
    tokio::fs::write(&manifest_path, bytes)
        .await
        .map_err(io_error("Pack manifest"))?;
    let listing = new_files
        .iter()
        .map(|path| format!("{path}\n"))
        .collect::<String>();
    tokio::fs::write(work.join(NEW_FILES), listing)
        .await
        .map_err(io_error("New file list"))?;
    let failures = failed
        .iter()
        .map(|table| format!("{table}\n"))
        .collect::<String>();
    tokio::fs::write(work.join(NOT_SNAPSHOTTED), failures)
        .await
        .map_err(io_error("Failure list"))?;
    tokio::fs::write(work.join(INCOMPLETE), incomplete.concat())
        .await
        .map_err(io_error("Incomplete list"))?;
    // What is published must be what the app accepts.
    let checked = verify_manifest(work, pinned)?;
    for path in &new_files {
        let file = checked
            .manifest
            .files
            .iter()
            .find(|file| &file.path == path)
            .ok_or_else(|| AppError::Internal(format!("{path} is missing from the manifest")))?;
        check_pack_file(&work.join(path), file)?;
    }
    Ok(manifest_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrity::sha256_hex;
    use crate::packs::import::{import_files, manifest_sha256, summarize};
    use crate::storage::DuckDbRepository;
    use ed25519_dalek::SigningKey;
    use std::cell::RefCell;

    /// Serves the next bytes queued for a publisher, and published files from a run's directory.
    struct LocalSource {
        served: RefCell<Vec<(Vec<u8>, DateTime<Utc>)>>,
        /// `None`: the published pack cannot be reached.
        published: Option<PathBuf>,
    }

    impl SnapshotSource for LocalSource {
        async fn retrieve(&self, _url: &Url, destination: &Path) -> Result<Retrieved, AppError> {
            let (bytes, retrieved_at) = self.served.borrow_mut().remove(0);
            std::fs::write(destination, &bytes).unwrap();
            Ok(Retrieved {
                retrieved_at,
                sha256: sha256_hex(&bytes),
                bytes: bytes.len() as u64,
                last_modified: None,
                etag: Some("\"served\"".into()),
            })
        }

        async fn published(&self, file: &PackFile) -> Result<PathBuf, AppError> {
            let published = self
                .published
                .as_ref()
                .ok_or_else(|| AppError::Network("HTTP 503".into()))?;
            let copy = published.join(format!("{}.copy", uuid::Uuid::new_v4()));
            std::fs::copy(published.join(&file.path), &copy).unwrap();
            check_pack_file(&copy, file)?;
            Ok(copy)
        }
    }

    fn key() -> PublisherKey {
        PublisherKey {
            key_id: Some("pack-test".into()),
            signing_key: SigningKey::from_bytes(&[7_u8; 32]),
        }
    }

    fn pinned() -> [PinnedPackKey; 1] {
        [PinnedPackKey::test(
            key().signing_key.verifying_key(),
            crate::packs::PackKeyScope::Snapshot,
        )]
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    /// One run of the call-type file, serving `bytes` retrieved at `retrieved_at`, with the
    /// published pack's files in `published` (`None`: unreachable).
    async fn run_once(
        root: &Path,
        name: &str,
        previous: Option<(PackManifest, Option<PathBuf>)>,
        bytes: &[u8],
        retrieved_at: &str,
    ) -> (PathBuf, PackManifest, Vec<String>) {
        let work = root.join(name);
        let (previous, published) = match previous {
            Some((manifest, published)) => (Some(manifest), published),
            None => (None, None),
        };
        let source = LocalSource {
            served: RefCell::new(vec![(bytes.to_vec(), at(retrieved_at))]),
            published,
        };
        let targets = targets(2026)
            .unwrap()
            .into_iter()
            .filter(|target| target.table == "call_types")
            .collect::<Vec<_>>();
        let pinned = pinned();
        run(
            SNAPSHOT_PACK,
            &work,
            previous,
            &targets,
            &source,
            &key(),
            &pinned,
            at(retrieved_at),
        )
        .await
        .unwrap();
        let manifest = verify_manifest(&work, &pinned).unwrap().manifest;
        let new_files = std::fs::read_to_string(work.join(NEW_FILES))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        (work, manifest, new_files)
    }

    const FIRST: &[u8] = b"call_type,description\nA,alpha\nB,beta\nC,gamma\n";
    const SECOND: &[u8] = b"call_type,description\nA,alpha\nB,BETA\nC,gamma\nD,delta\n";

    /// Bytes equal to the latest snapshot add a check and no file; changed bytes add a dated
    /// original, its Parquet copy and a change record. The cumulative manifest verifies, imports
    /// its originals through the app's pack import and never its change records, and the
    /// conformed view of the table holds every snapshot imported.
    #[tokio::test]
    async fn a_run_checks_unchanged_bytes_and_snapshots_changed_ones() {
        let root = tempfile::tempdir().unwrap();
        let (first, manifest, new_files) =
            run_once(root.path(), "first", None, FIRST, "2026-10-05T13:23:01Z").await;
        let original = "originals/call_types/2026-10-05T132301Z/pd_cfs_calltypes_datasd.csv";
        assert_eq!(
            new_files,
            [
                original,
                "parquet/call_types/2026-10-05T132301Z/pd_cfs_calltypes_datasd.parquet"
            ]
        );
        assert!(manifest.checks.is_empty());

        let (second, manifest, new_files) = run_once(
            root.path(),
            "second",
            Some((manifest, Some(first.clone()))),
            FIRST,
            "2026-10-06T13:23:02Z",
        )
        .await;
        assert!(new_files.is_empty());
        assert!(!second.join("originals").exists());
        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.checks.len(), 1);
        assert_eq!(manifest.checks[0].matches, original);
        assert_eq!(manifest.checks[0].sha256, sha256_hex(FIRST));
        assert_eq!(manifest.pack_version, "2026-10-06T132302Z");

        let (third, manifest, new_files) = run_once(
            root.path(),
            "third",
            Some((manifest, Some(first.clone()))),
            SECOND,
            "2026-10-07T13:23:03Z",
        )
        .await;
        let latest = "originals/call_types/2026-10-07T132303Z/pd_cfs_calltypes_datasd.csv";
        assert_eq!(new_files.len(), 3);
        assert_eq!(new_files[0], latest);
        assert_eq!(
            new_files[2],
            "changes/call_types/2026-10-07T132303Z.parquet"
        );
        let record = manifest
            .files
            .iter()
            .find(|file| file.format == PackFileFormat::Changes)
            .unwrap();
        let summary = record.changes.as_ref().unwrap();
        assert_eq!(
            (
                summary.previous.as_str(),
                summary.current.as_str(),
                summary.added,
                summary.removed,
                summary.changed
            ),
            (original, latest, 1, 0, 1)
        );
        assert_eq!(manifest.key_id.as_deref(), Some("pack-test"));
        assert_eq!(manifest.checks.len(), 1);

        // The app holds a published pack's files in one directory, whatever commit each came from.
        let earlier = third.join(original);
        std::fs::create_dir_all(earlier.parent().unwrap()).unwrap();
        std::fs::copy(first.join(original), earlier).unwrap();
        let described = summarize(manifest.clone(), "pack-test").unwrap();
        let table = &described.tables[0];
        assert_eq!(
            (
                table.table.as_str(),
                table.files.len(),
                table.unchanged_checks,
                table.last_checked_at.as_deref(),
                table.change_records
            ),
            ("call_types", 2, 1, Some("2026-10-06T13:23:02+00:00"), 1)
        );
        assert!(table.files.iter().all(|file| file.dataset_kind.is_some()));
        let repository = DuckDbRepository::new(":memory:").unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let digest = manifest_sha256(&manifest).unwrap();
        let pinned = pinned().to_vec();
        let refused = import_files(
            &repository,
            third.clone(),
            &digest,
            vec![record.path.clone()],
            pinned.clone(),
        )
        .await
        .unwrap_err();
        assert!(
            refused.to_string().contains("not an original file"),
            "{refused}"
        );
        let outcomes = import_files(
            &repository,
            third,
            &digest,
            vec![original.into(), latest.into()],
            pinned,
        )
        .await
        .unwrap();
        assert!(outcomes.iter().all(|outcome| outcome.error.is_none()));
        let view = repository
            .create_conformed_view("call_types".into())
            .await
            .unwrap()
            .unwrap();
        let rows = repository
            .execute_restricted_sql(
                &format!(
                    "SELECT count(DISTINCT source_id), count(*) FROM {view}
                     WHERE publisher_record_id = 'B'"
                ),
                10,
            )
            .await
            .unwrap()
            .rows;
        assert_eq!(rows, [[serde_json::json!(2), serde_json::json!(2)]]);
    }

    /// Changed bytes are kept even when the change record cannot be made: the snapshot is
    /// published without one, and the run names the table and why.
    #[tokio::test]
    async fn a_snapshot_is_kept_when_its_change_record_fails() {
        let root = tempfile::tempdir().unwrap();
        let (_, manifest, _) =
            run_once(root.path(), "first", None, FIRST, "2026-10-05T13:23:01Z").await;
        let (second, manifest, new_files) = run_once(
            root.path(),
            "second",
            Some((manifest, None)),
            SECOND,
            "2026-10-06T13:23:02Z",
        )
        .await;
        assert_eq!(
            new_files,
            [
                "originals/call_types/2026-10-06T132302Z/pd_cfs_calltypes_datasd.csv",
                "parquet/call_types/2026-10-06T132302Z/pd_cfs_calltypes_datasd.parquet"
            ]
        );
        assert_eq!(manifest.files.len(), 4);
        assert!(std::fs::read_to_string(second.join(NOT_SNAPSHOTTED))
            .unwrap()
            .is_empty());
        let incomplete = std::fs::read_to_string(second.join(INCOMPLETE)).unwrap();
        assert!(
            incomplete.starts_with("call_types: no change record against originals/call_types/")
                && incomplete.contains("HTTP 503"),
            "{incomplete}"
        );
    }
}
