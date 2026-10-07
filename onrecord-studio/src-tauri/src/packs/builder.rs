//! The data-pack builder: retrieves a pack's City files through the app's own source list and
//! downloader, records what the publisher served, and signs the manifest with the publisher key
//! kept in the macOS Keychain. Built only with the `pack-builder` feature.

use super::catalog::pack_tables;
use super::conformed::{conformed_select, literal, ConformedOrigin};
use super::{PackFile, PackFileFormat, PackLicence, PackManifest, PACK_MANIFEST_FILE};
use crate::error::AppError;
use crate::ingestion::client::{ApiClient, HttpClientConfig};
use crate::ingestion::queue::dataset_source;
use crate::storage::tabular::{TableSource, MAX_STREAMED_BYTES};
use crate::storage::DuckDbRepository;
use crate::types::{AnalyticalDatasetKind, AnalyticalImportRequest, CityDataset};
use chrono::{Datelike, Utc};
use ed25519_dalek::SigningKey;
use std::path::{Path, PathBuf};

const KEYCHAIN_SERVICE: &str = "com.onrecord.pack-publisher";
const KEYCHAIN_ACCOUNT: &str = "publisher-seed-v1";
/// The earliest year asked of a table split by year; the source list refuses years before its
/// publisher's series, and those are skipped.
const EARLIEST_YEAR: u16 = 2000;

fn licence() -> PackLicence {
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

fn keychain_error(error: impl std::fmt::Display) -> AppError {
    AppError::Internal(format!("Pack publisher key: {error}"))
}

/// Creates the publisher key and stores its seed in the Keychain; refuses if one exists. Returns
/// the public key as hex, for `PACK_PUBLIC_KEYS`.
pub fn init_key() -> Result<String, AppError> {
    use security_framework::passwords::{get_generic_password, set_generic_password};
    if get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).is_ok() {
        return Err(AppError::Internal(
            "A pack publisher key already exists in the Keychain; it is never replaced here".into(),
        ));
    }
    let seed = crate::integrity::generate_signing_key_seed();
    set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, &seed).map_err(keychain_error)?;
    Ok(crate::integrity::digest_hex(
        SigningKey::from_bytes(&seed).verifying_key().to_bytes(),
    ))
}

fn publisher_key() -> Result<SigningKey, AppError> {
    let seed =
        security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
            .map_err(keychain_error)?;
    let seed: [u8; 32] = seed
        .as_slice()
        .try_into()
        .map_err(|_| keychain_error("the stored seed is not 32 bytes"))?;
    Ok(SigningKey::from_bytes(&seed))
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
    let tables = pack_tables(pack)?;
    let key = publisher_key()?;
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
    let scratch = ScratchDatabase(
        std::env::temp_dir().join(format!("onrecord-pack-{}.duckdb", uuid::Uuid::new_v4())),
    );
    let repository = DuckDbRepository::new(&scratch.0)?;
    repository
        .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
        .await?;
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
            let (source, name) = (path.clone(), file_name.clone());
            let rows = tauri::async_runtime::spawn_blocking(move || {
                TableSource::open(&source, &name, None)?.read(|_| Ok(()))
            })
            .await
            .map_err(|error| AppError::Internal(format!("Pack row count: {error}")))??
            .rows as u64;
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
            };
            let parquet =
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

/// The DuckDB file the Parquet copies are made in, removed with its WAL when the build ends.
struct ScratchDatabase(PathBuf);

impl Drop for ScratchDatabase {
    fn drop(&mut self) {
        let mut wal = self.0.clone().into_os_string();
        wal.push(".wal");
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(wal);
    }
}

/// Imports `original` from `out` through the app's analytical import into `scratch`, writes its
/// conformed rows in file order to `parquet/<table>/<file stem>.parquet`, and returns that file's
/// manifest entry. Its rows are the rows the import kept; the rows it rejected as unreadable are
/// not in the copy, and the entry counts them.
async fn write_conformed_parquet(
    scratch: &DuckDbRepository,
    out: &Path,
    original: &PackFile,
    kind: AnalyticalDatasetKind,
    pack_version: &str,
) -> Result<PackFile, AppError> {
    let source = Path::new(&original.path);
    let (Some(file_name), Some(stem)) = (
        source.file_name().and_then(|name| name.to_str()),
        source.file_stem().and_then(|stem| stem.to_str()),
    ) else {
        return Err(AppError::Internal(format!(
            "Pack original {} has no file name",
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
    let relative = format!("parquet/{}/{stem}.parquet", original.table);
    let path = out.join(&relative);
    tokio::fs::create_dir_all(path.parent().expect("a file under parquet"))
        .await
        .map_err(|error| AppError::Storage(format!("Pack directory: {error}")))?;
    let target = path
        .to_str()
        .ok_or_else(|| AppError::Storage(format!("{} is not UTF-8", path.display())))?
        .to_owned();
    let (table, snapshot_id) = (original.table.clone(), imported.snapshot_id);
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
    Ok(PackFile {
        path: relative,
        format: PackFileFormat::Parquet,
        sha256,
        bytes,
        rows,
        excluded_rows: Some(imported.rejected_rows as u64),
        ..original.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let scratch = DuckDbRepository::new(":memory:").unwrap();
        scratch
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let copy = write_conformed_parquet(
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
