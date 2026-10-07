//! Imports a verified pack's original files through the analytical import, one file per import,
//! and records the pack beside each snapshot (`docs/data-packs.md`, build order step 4).

use super::{verify_pack, PackFile, PackFileFormat, PackManifest, VerifiedPack};
use crate::error::AppError;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{
    AnalyticalDatasetKind, AnalyticalImportRequest, DataPackFileSummary, DataPackImportOutcome,
    DataPackSummary, DataPackTable,
};
use ed25519_dalek::VerifyingKey;
use std::path::PathBuf;

/// The data pack a snapshot's bytes came from, recorded beside the snapshot.
#[derive(Debug, Clone)]
pub(crate) struct PackOrigin {
    pub pack: String,
    pub pack_version: String,
    pub built_at: String,
    pub signer_key_id: String,
    pub file_path: String,
    pub file_sha256: String,
}

/// The kind `ingestion::queue::city_datasets` imports the same publisher file as. The snapshot id
/// is prefixed by its kind, so any other kind would make the same bytes a second snapshot.
fn dataset_kind(table: &str) -> Option<AnalyticalDatasetKind> {
    match table {
        "budget_adopted" => Some(AnalyticalDatasetKind::OperatingBudget),
        "police_calls" => Some(AnalyticalDatasetKind::CallsForService),
        "budget_actuals" | "service_reports" => Some(AnalyticalDatasetKind::RegionalData),
        _ => None,
    }
}

async fn verify(directory: PathBuf, pinned: Vec<VerifyingKey>) -> Result<VerifiedPack, AppError> {
    crate::blocking::spawn("Verify data pack", move || verify_pack(&directory, &pinned)).await?
}

fn originals(manifest: &PackManifest) -> impl Iterator<Item = &PackFile> {
    manifest
        .files
        .iter()
        .filter(|file| file.format == PackFileFormat::Original)
}

fn summarize(pack: VerifiedPack) -> DataPackSummary {
    let mut tables: Vec<DataPackTable> = Vec::new();
    for file in originals(&pack.manifest) {
        let summary = DataPackFileSummary {
            path: file.path.clone(),
            year: file.year,
            rows: file.rows,
            bytes: file.bytes,
            partial: file.partial,
            source_url: file.source_url.clone(),
            retrieved_at: file.retrieved_at.clone(),
        };
        match tables.iter_mut().find(|table| table.table == file.table) {
            Some(table) => table.files.push(summary),
            None => tables.push(DataPackTable {
                table: file.table.clone(),
                dataset_kind: dataset_kind(&file.table),
                files: vec![summary],
            }),
        }
    }
    let manifest = pack.manifest;
    DataPackSummary {
        pack: manifest.pack,
        pack_version: manifest.pack_version,
        schema_version: manifest.schema_version,
        built_at: manifest.built_at,
        signer_key_id: pack.signer_key_id,
        licence_id: manifest.licence.id,
        licence_url: manifest.licence.url,
        disclaimer: manifest.licence.disclaimer,
        tables,
    }
}

/// Verifies the pack in `directory` against `pinned` and describes what it holds.
pub(crate) async fn inspect(
    directory: PathBuf,
    pinned: Vec<VerifyingKey>,
) -> Result<DataPackSummary, AppError> {
    verify(directory, pinned).await.map(summarize)
}

fn display_name(manifest: &PackManifest, file: &PackFile) -> String {
    let year = file.year.map(|year| format!(" {year}")).unwrap_or_default();
    let built_on = manifest.built_at.get(..10).unwrap_or(&manifest.built_at);
    format!(
        "{}{year} ({} {}), retrieved by the pack builder on {built_on}",
        file.table, manifest.pack, manifest.pack_version
    )
}

/// Imports each of `selected` (paths as the manifest lists them) from the pack in `directory`.
/// Every selection is checked before the first import; each file is then its own import, so one
/// file's failure is reported beside the others' results.
pub(crate) async fn import_files(
    repository: &DuckDbRepository,
    directory: PathBuf,
    selected: Vec<String>,
    pinned: Vec<VerifyingKey>,
) -> Result<Vec<DataPackImportOutcome>, AppError> {
    // Verified again, not trusted from the inspection: the folder may have changed since.
    let pack = verify(directory, pinned).await?;
    let mut chosen: Vec<(&PackFile, AnalyticalDatasetKind)> = Vec::new();
    for path in &selected {
        if chosen.iter().any(|(file, _)| &file.path == path) {
            continue;
        }
        let file = originals(&pack.manifest)
            .find(|file| &file.path == path)
            .ok_or_else(|| {
                AppError::DataParsing(format!("{path} is not an original file this pack lists"))
            })?;
        let kind = dataset_kind(&file.table).ok_or_else(|| {
            AppError::DataParsing(format!("This app cannot import {} files", file.table))
        })?;
        chosen.push((file, kind));
    }
    if chosen.is_empty() {
        return Err(AppError::DataParsing(
            "Choose at least one file to import".into(),
        ));
    }
    let manifest = &pack.manifest;
    let mut outcomes = Vec::with_capacity(chosen.len());
    for (file, kind) in chosen {
        let request = AnalyticalImportRequest {
            dataset_kind: kind,
            display_name: display_name(manifest, file),
            source_uri: Some(file.source_url.clone()),
            local_path: pack.path_of(file).to_string_lossy().into_owned(),
            observation_period_start: None,
            observation_period_end: None,
            partial_dataset: file.partial,
            worksheet: None,
            source_file_name: file.path.rsplit('/').next().map(str::to_owned),
            // Catches a file replaced between verification and the import's own read.
            expected_source_sha256: Some(file.sha256.clone()),
            derived_subset: None,
        };
        let origin = PackOrigin {
            pack: manifest.pack.clone(),
            pack_version: manifest.pack_version.clone(),
            built_at: manifest.built_at.clone(),
            signer_key_id: pack.signer_key_id.clone(),
            file_path: file.path.clone(),
            file_sha256: file.sha256.to_ascii_lowercase(),
        };
        let imported = repository
            .import_analytical_csv_from_pack(request, origin)
            .await;
        let (result, error) = match imported {
            Ok(result) => (Some(result), None),
            Err(error) => (None, Some(error.to_string())),
        };
        outcomes.push(DataPackImportOutcome {
            path: file.path.clone(),
            table: file.table.clone(),
            year: file.year,
            result,
            error,
        });
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrity::{sha256_hex, sign_package};
    use crate::packs::{PackLicence, PACK_MANIFEST_FILE};
    use ed25519_dalek::SigningKey;
    use std::path::Path;

    const ACTUALS_URI: &str =
        "https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv";
    const ACTUALS: &[u8] = b"amount,report_fy,dept_name\n100.5,2024,Police\n7,2025,Library\n";
    const FILE: &str = "originals/budget_actuals/actuals_operating_datasd.csv";
    const BUILT_AT: &str = "2026-10-06T22:59:25.459212+00:00";

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[9_u8; 32])
    }

    /// A signed pack holding one `budget_actuals` original, with its URL as the builder records
    /// it (parsed and re-serialized).
    fn signed_pack() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, ACTUALS).unwrap();
        let manifest = PackManifest {
            pack: "san-diego-money".into(),
            pack_version: "2026.10.6".into(),
            schema_version: "1.0".into(),
            built_at: BUILT_AT.into(),
            licence: PackLicence {
                id: "ODC-PDDL-1.0".into(),
                url: "https://opendefinition.org/licenses/odc-pddl/".into(),
                disclaimer: "Informational only.".into(),
            },
            files: vec![PackFile {
                path: FILE.into(),
                table: "budget_actuals".into(),
                format: PackFileFormat::Original,
                source_url: reqwest::Url::parse(ACTUALS_URI).unwrap().to_string(),
                retrieved_at: "2026-10-06T22:59:15Z".into(),
                sha256: sha256_hex(ACTUALS),
                bytes: ACTUALS.len() as u64,
                rows: 2,
                year: None,
                partial: true,
                last_modified: None,
                etag: None,
            }],
        };
        let package = sign_package(manifest, &key()).unwrap();
        std::fs::write(
            directory.path().join(PACK_MANIFEST_FILE),
            serde_json::to_vec_pretty(&package).unwrap(),
        )
        .unwrap();
        directory
    }

    async fn repository() -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        repository
    }

    async fn import(
        repository: &DuckDbRepository,
        directory: &Path,
    ) -> Result<Vec<DataPackImportOutcome>, AppError> {
        import_files(
            repository,
            directory.to_path_buf(),
            vec![FILE.into()],
            vec![key().verifying_key()],
        )
        .await
    }

    async fn count(repository: &DuckDbRepository, sql: &'static str) -> i64 {
        repository
            .query_connection("test", move |conn| {
                conn.query_row(sql, [], |row| row.get(0))
                    .map_err(|error| AppError::Storage(error.to_string()))
            })
            .await
            .unwrap()
    }

    async fn written(repository: &DuckDbRepository) -> (i64, i64) {
        (
            count(repository, "SELECT COUNT(*) FROM analytical_snapshots").await,
            count(repository, "SELECT COUNT(*) FROM pack_origins").await,
        )
    }

    #[tokio::test]
    async fn a_tampered_pack_writes_nothing() {
        // The signed manifest changed: a listed row count.
        let directory = signed_pack();
        let manifest = directory.path().join(PACK_MANIFEST_FILE);
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(&manifest, text.replacen("\"rows\": 2", "\"rows\": 3", 1)).unwrap();
        let repository = repository().await;
        let refused = import(&repository, directory.path()).await.unwrap_err();
        assert!(refused.to_string().contains("refused"), "{refused}");
        assert_eq!(written(&repository).await, (0, 0));

        // One byte of the original changed.
        let directory = signed_pack();
        let file = directory.path().join(FILE);
        let mut bytes = std::fs::read(&file).unwrap();
        bytes[0] ^= 1;
        std::fs::write(&file, bytes).unwrap();
        assert!(import(&repository, directory.path()).await.is_err());
        assert_eq!(written(&repository).await, (0, 0));
    }

    /// The same bytes fetched by the user and imported from a pack are one snapshot; the pack is
    /// recorded beside it and survives the user's later re-fetch of the same bytes.
    #[tokio::test]
    async fn the_same_bytes_fetched_and_from_a_pack_are_one_snapshot_with_the_pack_recorded() {
        let directory = signed_pack();
        let repository = repository().await;
        // Shaped as `fetch_city_dataset` shapes OperatingActuals.
        let fetched = || AnalyticalImportRequest {
            dataset_kind: AnalyticalDatasetKind::RegionalData,
            display_name: "City operating actuals (all published years)".into(),
            source_uri: Some(ACTUALS_URI.into()),
            local_path: directory.path().join(FILE).to_string_lossy().into_owned(),
            observation_period_start: None,
            observation_period_end: None,
            partial_dataset: true,
            worksheet: None,
            source_file_name: Some("actuals_operating_datasd.csv".into()),
            expected_source_sha256: Some(sha256_hex(ACTUALS)),
            derived_subset: None,
        };
        let outcomes = import(&repository, directory.path()).await.unwrap();
        let from_pack = outcomes[0].result.as_ref().unwrap();
        assert!(from_pack
            .display_name
            .ends_with("retrieved by the pack builder on 2026-10-06"));
        let user = repository.import_analytical_csv(fetched()).await.unwrap();
        assert_eq!(user.snapshot_id, from_pack.snapshot_id);
        let again = import(&repository, directory.path()).await.unwrap();
        assert_eq!(again[0].error, None);
        assert_eq!(written(&repository).await, (1, 1));
        let snapshot = user.snapshot_id.clone();
        let origin = repository
            .query_connection("test", move |conn| {
                conn.query_row(
                    "SELECT pack, pack_version, built_at, signer_key_id, file_path, file_sha256
                     FROM pack_origins WHERE snapshot_id = ?1",
                    [snapshot],
                    |row| {
                        (0..6)
                            .map(|column| row.get::<_, String>(column))
                            .collect::<Result<Vec<_>, _>>()
                    },
                )
                .map_err(|error| AppError::Storage(error.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(
            origin,
            [
                "san-diego-money".to_string(),
                "2026.10.6".into(),
                BUILT_AT.into(),
                sha256_hex(key().verifying_key().as_bytes()),
                FILE.into(),
                sha256_hex(ACTUALS),
            ]
        );
    }

    /// A core written before pack import has no `pack_origins`; opening it must create the table.
    #[tokio::test]
    async fn a_core_from_before_pack_import_records_pack_origins() {
        let repository = repository().await;
        repository
            .query_connection("test", |conn| {
                conn.execute_batch("DROP TABLE pack_origins")
                    .map_err(|error| AppError::Storage(error.to_string()))
            })
            .await
            .unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .expect("an older core must open");
        let directory = signed_pack();
        let outcomes = import(&repository, directory.path()).await.unwrap();
        assert_eq!(outcomes[0].error, None);
        assert_eq!(written(&repository).await, (1, 1));
    }
}
