//! The data-pack builder: retrieves a pack's City files through the app's own source list and
//! downloader, records what the publisher served, and signs the manifest with the publisher key,
//! kept in the macOS Keychain or, on the CI runner, given as an environment seed. Built only with
//! the `pack-builder` feature.

use super::catalog::{pack_tables, SNAPSHOT_PACK};
use super::conformed::{conformed_select, literal, ConformedOrigin};
use super::{
    decode_hex, PackFile, PackFileFormat, PackLicence, PackManifest, PACK_MANIFEST_FILE,
    PACK_PUBLIC_KEYS,
};
use crate::error::AppError;
use crate::ingestion::client::{ApiClient, HttpClientConfig};
use crate::ingestion::queue::dataset_source;
use crate::storage::tabular::{TableSource, MAX_STREAMED_BYTES};
use crate::storage::DuckDbRepository;
use crate::types::{AnalyticalDatasetKind, AnalyticalImportRequest, CityDataset};
use chrono::{Datelike, Utc};
use ed25519_dalek::SigningKey;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "com.onrecord.pack-publisher";
#[cfg(target_os = "macos")]
const KEYCHAIN_ACCOUNT: &str = "publisher-seed-v1";
/// A publisher seed as 64 hex characters, which takes the place of the Keychain key.
const SEED_VARIABLE: &str = "ONRECORD_PACK_SEED";
/// The `PACK_PUBLIC_KEYS` name the seed's key must be pinned under.
const KEY_ID_VARIABLE: &str = "ONRECORD_PACK_KEY_ID";
/// The earliest year asked of a table split by year; the source list refuses years before its
/// publisher's series, and those are skipped.
const EARLIEST_YEAR: u16 = 2000;

pub(super) fn licence() -> PackLicence {
    PackLicence {
        id: "ODC-PDDL-1.0".into(),
        url: "https://opendefinition.org/licenses/odc-pddl/".into(),
        disclaimer: "City of San Diego Public Data Sets are provided for informational purposes \
            only. The City does not warrant their completeness, accuracy, content, or fitness for \
            any particular purpose or use, and is not liable for any deficiency in them (City of \
            San Diego Open Data Policy, s. 2.7)."
            .into(),
    }
}

fn key_error(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("Pack publisher key: {error}"))
}

/// Creates the publisher key and stores its seed in the Keychain; refuses if one exists. Returns
/// the public key as hex, for `PACK_PUBLIC_KEYS`.
#[cfg(target_os = "macos")]
pub fn init_key() -> Result<String, AppError> {
    use security_framework::passwords::{get_generic_password, set_generic_password};
    if get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).is_ok() {
        return Err(AppError::Internal(
            "A pack publisher key already exists in the Keychain; it is never replaced here".into(),
        ));
    }
    let seed = crate::integrity::generate_signing_key_seed();
    set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, &seed).map_err(key_error)?;
    Ok(crate::integrity::digest_hex(
        SigningKey::from_bytes(&seed).verifying_key().to_bytes(),
    ))
}

/// A publisher signing key and the `PACK_PUBLIC_KEYS` name it is pinned under, if it is pinned.
pub(super) struct PublisherKey {
    pub key_id: Option<String>,
    pub signing_key: SigningKey,
}

/// The key `seed_hex` names, accepted only when `pinned` lists exactly `(key_id, its public
/// key)`. No error repeats the seed.
fn pinned_seed_key(
    seed_hex: &str,
    key_id: &str,
    pinned: &[(&str, &str)],
) -> Result<PublisherKey, AppError> {
    let seed: [u8; 32] = (seed_hex.len() == 64 && seed_hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| decode_hex(seed_hex))
        .flatten()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| key_error(format!("{SEED_VARIABLE} is not 64 hex characters")))?;
    let signing_key = SigningKey::from_bytes(&seed);
    let public = crate::integrity::digest_hex(signing_key.verifying_key().to_bytes());
    if !pinned
        .iter()
        .any(|(kid, hex)| *kid == key_id && hex.eq_ignore_ascii_case(&public))
    {
        return Err(key_error(format!(
            "the key {SEED_VARIABLE} names is not pinned in PACK_PUBLIC_KEYS as '{key_id}'"
        )));
    }
    Ok(PublisherKey {
        key_id: Some(key_id.to_owned()),
        signing_key,
    })
}

/// The publisher key: from [`SEED_VARIABLE`] when it is set, else from the Keychain (macOS only).
pub(super) fn publisher_key() -> Result<PublisherKey, AppError> {
    if let Some(seed) = std::env::var_os(SEED_VARIABLE) {
        let seed = seed
            .into_string()
            .map_err(|_| key_error(format!("{SEED_VARIABLE} is not 64 hex characters")))?;
        let key_id = std::env::var(KEY_ID_VARIABLE)
            .map_err(|_| key_error(format!("{SEED_VARIABLE} is set without {KEY_ID_VARIABLE}")))?;
        return pinned_seed_key(&seed, &key_id, PACK_PUBLIC_KEYS);
    }
    keychain_key()
}

#[cfg(target_os = "macos")]
fn keychain_key() -> Result<PublisherKey, AppError> {
    let seed =
        security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
            .map_err(key_error)?;
    let seed: [u8; 32] = seed
        .as_slice()
        .try_into()
        .map_err(|_| key_error("the stored seed is not 32 bytes"))?;
    let signing_key = SigningKey::from_bytes(&seed);
    let public = crate::integrity::digest_hex(signing_key.verifying_key().to_bytes());
    Ok(PublisherKey {
        key_id: PACK_PUBLIC_KEYS
            .iter()
            .find(|(_, hex)| hex.eq_ignore_ascii_case(&public))
            .map(|(kid, _)| (*kid).to_owned()),
        signing_key,
    })
}

#[cfg(not(target_os = "macos"))]
fn keychain_key() -> Result<PublisherKey, AppError> {
    Err(key_error(format!(
        "set {SEED_VARIABLE} and {KEY_ID_VARIABLE}; the Keychain key source is macOS only"
    )))
}

/// Rows in the original at `path` as the analytical import's reader counts them.
pub(super) async fn count_rows(path: &Path, file_name: &str) -> Result<u64, AppError> {
    let (source, name) = (path.to_path_buf(), file_name.to_owned());
    Ok(tauri::async_runtime::spawn_blocking(move || {
        TableSource::open(&source, &name, None)?.read(|_| Ok(()))
    })
    .await
    .map_err(|error| AppError::Internal(format!("Pack row count: {error}")))??
    .rows as u64)
}

/// The years a table is published for, or `[None]` for a table published as one file.
fn published_years(dataset: CityDataset, current_year: i32) -> Vec<Option<u16>> {
    if dataset_source(dataset, None, current_year).is_ok() {
        return vec![None];
    }
    (EARLIEST_YEAR..=current_year as u16)
        .filter(|year| dataset_source(dataset, Some(*year), current_year).is_ok())
        .map(Some)
        .collect()
}

/// Retrieves every file of `pack` into `out` and writes its signed manifest there. `out` must not
/// exist yet, so a pack directory only ever holds one build.
pub async fn build(pack: &str, out: &Path) -> Result<PathBuf, AppError> {
    if pack == SNAPSHOT_PACK {
        return Err(AppError::DataParsing(format!(
            "{SNAPSHOT_PACK} is extended by `snapshot`, never built anew"
        )));
    }
    let tables = pack_tables(pack)?;
    let key = publisher_key()?.signing_key;
    if out.exists() {
        return Err(AppError::Storage(format!(
            "{} already exists; a pack is built into a new directory",
            out.display()
        )));
    }
    let client = ApiClient::new(HttpClientConfig {
        max_document_bytes: MAX_STREAMED_BYTES as usize,
        ..HttpClientConfig::default()
    })?;
    let now = Utc::now();
    let pack_version = format!("{}.{}.{}", now.year(), now.month(), now.day());
    let (_scratch, repository) = scratch_database().await?;
    let mut files = Vec::new();
    for table in tables {
        for year in published_years(table.dataset, now.year()) {
            let published = dataset_source(table.dataset, year, now.year())?;
            let (url, file_name, partial, kind) = (
                published.url,
                published.file_name,
                published.partial,
                published.kind,
            );
            let relative = format!("originals/{}/{file_name}", table.table);
            let path = out.join(&relative);
            tokio::fs::create_dir_all(path.parent().expect("a file under originals"))
                .await
                .map_err(|error| AppError::Storage(format!("Pack directory: {error}")))?;
            let url = reqwest::Url::parse(&url)
                .map_err(|error| AppError::Internal(format!("Pack source URL: {error}")))?;
            let downloaded = client.download_url_to_path(url.clone(), &path).await?;
            let rows = count_rows(&path, &file_name).await?;
            eprintln!("{relative}: {rows} rows, {} bytes", downloaded.byte_count);
            let original = PackFile {
                path: relative,
                table: table.table.into(),
                format: PackFileFormat::Original,
                source_url: url.to_string(),
                retrieved_at: downloaded.acquisition.retrieved_at,
                sha256: downloaded.sha256,
                bytes: downloaded.byte_count,
                rows,
                excluded_rows: None,
                year,
                partial,
                last_modified: downloaded.acquisition.last_modified,
                etag: downloaded.acquisition.etag,
                changes: None,
            };
            let (parquet, _) =
                write_conformed_parquet(&repository, out, &original, kind.clone(), &pack_version)
                    .await?;
            eprintln!(
                "{}: {} rows, {} bytes",
                parquet.path, parquet.rows, parquet.bytes
            );
            files.extend([original, parquet]);
        }
    }
    let manifest = PackManifest {
        pack: pack.into(),
        pack_version,
        schema_version: "1.0".into(),
        built_at: Utc::now().to_rfc3339(),
        licence: licence(),
        files,
        key_id: None,
        checks: Vec::new(),
    };
    let signed = crate::integrity::sign_package(manifest, &key).map_err(AppError::Internal)?;
    let manifest_path = out.join(PACK_MANIFEST_FILE);
    let bytes = serde_json::to_vec_pretty(&signed)
        .map_err(|error| AppError::Internal(format!("Pack manifest: {error}")))?;
    tokio::fs::write(&manifest_path, bytes)
        .await
        .map_err(|error| AppError::Storage(format!("Pack manifest: {error}")))?;
    Ok(manifest_path)
}

/// The DuckDB file the Parquet copies are made in, removed with its WAL when dropped.
pub(super) struct ScratchDatabase(PathBuf);

/// A new scratch DuckDB file with the app's schema, removed when the guard is dropped.
pub(super) async fn scratch_database() -> Result<(ScratchDatabase, DuckDbRepository), AppError> {
    let scratch = ScratchDatabase(
        std::env::temp_dir().join(format!("onrecord-pack-{}.duckdb", uuid::Uuid::new_v4())),
    );
    let repository = DuckDbRepository::new(&scratch.0)?;
    repository
        .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
        .await?;
    Ok((scratch, repository))
}

impl Drop for ScratchDatabase {
    fn drop(&mut self) {
        let mut wal = self.0.clone().into_os_string();
        wal.push(".wal");
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(wal);
    }
}

/// Imports `original` from `out` through the app's analytical import into `scratch`, writes its
/// conformed rows in file order to the Parquet path mirroring the original's
/// (`originals/<dir>/<name>` to `parquet/<dir>/<stem>.parquet`), and returns that file's manifest
/// entry and the scratch snapshot. Its rows are the rows the import kept; the rows it rejected as
/// unreadable are not in the copy, and the entry counts them.
pub(super) async fn write_conformed_parquet(
    scratch: &DuckDbRepository,
    out: &Path,
    original: &PackFile,
    kind: AnalyticalDatasetKind,
    pack_version: &str,
) -> Result<(PackFile, String), AppError> {
    let source = Path::new(&original.path);
    let (Some(file_name), Some(stem), Some(directory)) = (
        source.file_name().and_then(|name| name.to_str()),
        source.file_stem().and_then(|stem| stem.to_str()),
        source
            .parent()
            .and_then(|parent| parent.strip_prefix("originals").ok())
            .and_then(|directory| directory.to_str())
            .filter(|directory| !directory.is_empty()),
    ) else {
        return Err(AppError::Internal(format!(
            "Pack original {} is not originals/<table>/.../<file name>",
            original.path
        )));
    };
    let imported = scratch
        .import_analytical_csv(AnalyticalImportRequest {
            dataset_kind: kind,
            display_name: original.path.clone(),
            source_uri: Some(original.source_url.clone()),
            local_path: out.join(source).to_string_lossy().into_owned(),
            observation_period_start: None,
            observation_period_end: None,
            partial_dataset: original.partial,
            worksheet: None,
            source_file_name: Some(file_name.into()),
            expected_source_sha256: Some(original.sha256.clone()),
            derived_subset: None,
        })
        .await?;
    let relative = format!("parquet/{directory}/{stem}.parquet");
    let path = out.join(&relative);
    tokio::fs::create_dir_all(path.parent().expect("a file under parquet"))
        .await
        .map_err(|error| AppError::Storage(format!("Pack directory: {error}")))?;
    let target = path
        .to_str()
        .ok_or_else(|| AppError::Storage(format!("{} is not UTF-8", path.display())))?
        .to_owned();
    let (table, snapshot_id) = (original.table.clone(), imported.snapshot_id.clone());
    let origin = ConformedOrigin {
        retrieved_at: Some(original.retrieved_at.clone()),
        pack_version: Some(pack_version.into()),
    };
    let rows = scratch
        .query_connection("Parquet copy", move |conn| {
            let select = conformed_select(conn, &table, &snapshot_id, &origin)?;
            let copy_failed =
                |error: duckdb::Error| AppError::Storage(format!("Parquet copy: {error}"));
            conn.execute_batch(&format!(
                "COPY (SELECT * FROM ({select}) ORDER BY source_row) TO {} (FORMAT PARQUET)",
                literal(Some(&target))
            ))
            .map_err(copy_failed)?;
            conn.query_row(
                "SELECT count(*) FROM read_parquet(?1)",
                duckdb::params![target],
                |row| row.get::<_, u64>(0),
            )
            .map_err(copy_failed)
        })
        .await?;
    let bytes = std::fs::metadata(&path)
        .map_err(|error| AppError::Storage(format!("Parquet copy: {error}")))?
        .len();
    let sha256 = crate::integrity::sha256_file(&path)
        .map_err(|error| AppError::Storage(format!("Parquet copy: {error}")))?;
    Ok((
        PackFile {
            path: relative,
            format: PackFileFormat::Parquet,
            sha256,
            bytes,
            rows,
            excluded_rows: Some(imported.rejected_rows as u64),
            ..original.clone()
        },
        imported.snapshot_id,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public_hex(seed: u8) -> String {
        crate::integrity::digest_hex(
            SigningKey::from_bytes(&[seed; 32])
                .verifying_key()
                .to_bytes(),
        )
    }

    /// An environment seed signs only as the exact `(kid, public key)` pair the app pins, and no
    /// refusal repeats the seed.
    #[test]
    fn an_environment_seed_signs_only_as_the_key_it_is_pinned_under() {
        let (ours, other) = (public_hex(0x11), public_hex(0x22));
        let pinned = [("pack-other", other.as_str()), ("pack-test", ours.as_str())];
        let seed = "11".repeat(32);
        let key = pinned_seed_key(&seed, "pack-test", &pinned).unwrap();
        assert_eq!(key.key_id.as_deref(), Some("pack-test"));
        assert_eq!(public_hex(0x11), {
            crate::integrity::digest_hex(key.signing_key.verifying_key().to_bytes())
        });

        let refusal = |seed: &str, kid: &str, pinned: &[(&str, &str)]| {
            pinned_seed_key(seed, kid, pinned)
                .err()
                .expect("refused")
                .to_string()
        };
        // Pinned, but under another name.
        assert!(refusal(&seed, "pack-other", &pinned).contains("not pinned"));
        // Not pinned at all, including by the app's own table.
        assert!(refusal(&"33".repeat(32), "pack-test", &pinned).contains("not pinned"));
        assert!(refusal(&seed, "pack-ci-2026-1", PACK_PUBLIC_KEYS).contains("not pinned"));
        for malformed in [String::new(), "11".into(), "zz".repeat(32), "1".repeat(65)] {
            let error = refusal(&malformed, "pack-test", &pinned);
            assert!(error.contains("not 64 hex characters"), "{error}");
            assert!(
                malformed.len() < 2 || !error.contains(&malformed),
                "{error}"
            );
        }
    }

    #[tokio::test]
    async fn a_parquet_copy_is_listed_as_the_file_written_and_holds_the_conformed_rows() {
        let out = tempfile::tempdir().unwrap();
        let original_bytes: &[u8] =
            b"amount,report_fy,budget_cycle\n10,23,adopted\n20,24,proposed\n30,24,\n40,25,adopted,x\n";
        let path = out.path().join("originals/budget_adopted/budget.csv");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, original_bytes).unwrap();
        let original = PackFile {
            path: "originals/budget_adopted/budget.csv".into(),
            table: "budget_adopted".into(),
            format: PackFileFormat::Original,
            source_url: "https://seshat.datasd.org/operating_budget/budget.csv".into(),
            retrieved_at: "2026-10-06T00:00:00Z".into(),
            sha256: crate::integrity::sha256_hex(original_bytes),
            bytes: original_bytes.len() as u64,
            // Not the copy's count, so a copy listing the original's would be caught.
            rows: 99,
            excluded_rows: None,
            year: None,
            partial: false,
            last_modified: None,
            etag: None,
            changes: None,
        };
        let scratch = DuckDbRepository::new(":memory:").unwrap();
        scratch
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let (copy, _) = write_conformed_parquet(
            &scratch,
            out.path(),
            &original,
            AnalyticalDatasetKind::OperatingBudget,
            "2026.10.6",
        )
        .await
        .unwrap();
        let written = out.path().join(&copy.path);
        assert_eq!(copy.path, "parquet/budget_adopted/budget.parquet");
        assert_eq!(copy.format, PackFileFormat::Parquet);
        assert_eq!(copy.bytes, std::fs::metadata(&written).unwrap().len());
        assert_eq!(
            copy.sha256,
            crate::integrity::sha256_file(&written).unwrap()
        );
        assert_eq!((copy.rows, copy.excluded_rows), (3, Some(1)));
        let target = written.to_str().unwrap().to_owned();
        let rows = scratch
            .query_connection("read copy", move |conn| {
                let mut statement = conn
                    .prepare(
                        "SELECT fiscal_year, retrieved_at, pack_version, budget_cycle
                         FROM read_parquet(?1)
                         ORDER BY source_row",
                    )
                    .unwrap();
                Ok(statement
                    .query_map(duckdb::params![target], |row| {
                        Ok((
                            row.get::<_, i32>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap())
            })
            .await
            .unwrap();
        let stamp = ("2026-10-06T00:00:00Z".to_owned(), "2026.10.6".to_owned());
        let cycle = |cycle: Option<&str>| cycle.map(str::to_owned);
        assert_eq!(
            rows,
            [
                (
                    2023,
                    stamp.0.clone(),
                    stamp.1.clone(),
                    cycle(Some("adopted"))
                ),
                (
                    2024,
                    stamp.0.clone(),
                    stamp.1.clone(),
                    cycle(Some("proposed"))
                ),
                (2024, stamp.0, stamp.1, cycle(None)),
            ]
        );
    }
}
