//! Imports a verified pack's original files through the analytical import, one file per import,
//! and records the pack beside each snapshot (`docs/data-packs.md`, build order step 4).

use super::catalog::original_kind;
use super::{
    check_pack_file, verify_manifest, verify_pack, PackFile, PackFileFormat, PackManifest,
    SignedManifest,
};
use crate::error::AppError;
use crate::integrity::sha256_hex;
use crate::storage::sqlite::DuckDbRepository;
use crate::types::{
    AnalyticalDatasetKind, AnalyticalImportRequest, DataPackFileSummary, DataPackImportOutcome,
    DataPackSummary, DataPackTable,
};
use chrono::{DateTime, Datelike, SecondsFormat, Utc};
use ed25519_dalek::VerifyingKey;
use std::path::PathBuf;

/// The data pack a snapshot's bytes came from, recorded beside the snapshot.
#[derive(Debug, Clone)]
pub(crate) struct PackOrigin {
    pub pack: String,
    pub pack_version: String,
    pub built_at: String,
    /// Lowercase hex, as `pack_origins` keys it.
    pub signer_key_id: String,
    pub file_path: String,
    /// Lowercase hex.
    pub file_sha256: String,
    /// When the pack builder retrieved the file, normalized to UTC.
    pub retrieved_at: String,
}

async fn verify(directory: PathBuf, pinned: Vec<VerifyingKey>) -> Result<SignedManifest, AppError> {
    crate::blocking::spawn("Verify data pack", move || verify_pack(&directory, &pinned)).await?
}

/// The manifest in `directory`, unchanged since inspection, and exactly the `selected` originals,
/// each checked once; files the selection leaves out are not read.
async fn verify_chosen(
    directory: PathBuf,
    inspected_manifest_sha256: String,
    selected: Vec<String>,
    pinned: Vec<VerifyingKey>,
) -> Result<(SignedManifest, Vec<PackFile>), AppError> {
    crate::blocking::spawn("Verify data pack", move || {
        let signed = verify_manifest(&directory, &pinned)?;
        if !manifest_sha256(&signed.manifest)?.eq_ignore_ascii_case(&inspected_manifest_sha256) {
            return Err(AppError::Provenance(
                "Data pack refused: its manifest has changed since it was inspected; inspect it again"
                    .into(),
            ));
        }
        let mut chosen: Vec<PackFile> = Vec::new();
        for path in &selected {
            if chosen.iter().any(|file| &file.path == path) {
                continue;
            }
            let file = originals(&signed.manifest)
                .find(|file| &file.path == path)
                .ok_or_else(|| {
                    AppError::DataParsing(format!("{path} is not an original file this pack lists"))
                })?;
            chosen.push(file.clone());
        }
        for file in &chosen {
            check_pack_file(&directory.join(&file.path), file)?;
        }
        Ok((signed, chosen))
    })
    .await?
}

/// SHA-256 of the manifest's canonical bytes: verification has checked they re-encode to the
/// signed bytes, so this is the signed manifest's digest.
pub(super) fn manifest_sha256(manifest: &PackManifest) -> Result<String, AppError> {
    serde_json::to_vec(manifest)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|error| AppError::Internal(format!("Pack manifest: {error}")))
}

fn originals(manifest: &PackManifest) -> impl Iterator<Item = &PackFile> {
    manifest
        .files
        .iter()
        .filter(|file| file.format == PackFileFormat::Original)
}

pub(super) fn file_kind(manifest: &PackManifest, file: &PackFile) -> Option<AnalyticalDatasetKind> {
    let file_name = file.path.rsplit('/').next()?;
    original_kind(
        &manifest.pack,
        &file.table,
        file.year,
        file_name,
        &file.source_url,
        Utc::now().year(),
    )
}

/// Describes a verified manifest's original files by table.
pub(crate) fn summarize(
    manifest: PackManifest,
    signer_key_id: &str,
) -> Result<DataPackSummary, AppError> {
    let manifest_sha256 = manifest_sha256(&manifest)?;
    let mut tables: Vec<DataPackTable> = Vec::new();
    for file in originals(&manifest) {
        let summary = DataPackFileSummary {
            path: file.path.clone(),
            year: file.year,
            rows: file.rows,
            bytes: file.bytes,
            partial: file.partial,
            source_url: file.source_url.clone(),
            retrieved_at: file.retrieved_at.clone(),
            dataset_kind: file_kind(&manifest, file),
        };
        match tables.iter_mut().find(|table| table.table == file.table) {
            Some(table) => table.files.push(summary),
            None => tables.push(DataPackTable {
                table: file.table.clone(),
                files: vec![summary],
            }),
        }
    }
    Ok(DataPackSummary {
        pack: manifest.pack,
        pack_version: manifest.pack_version,
        schema_version: manifest.schema_version,
        built_at: manifest.built_at,
        signer_key_id: signer_key_id.to_ascii_lowercase(),
        licence_id: manifest.licence.id,
        licence_url: manifest.licence.url,
        disclaimer: manifest.licence.disclaimer,
        manifest_sha256,
        tables,
    })
}

/// Verifies the pack in `directory` against `pinned` and describes what it holds.
pub(crate) async fn inspect(
    directory: PathBuf,
    pinned: Vec<VerifyingKey>,
) -> Result<DataPackSummary, AppError> {
    let pack = verify(directory, pinned).await?;
    summarize(pack.manifest, &pack.signer_key_id)
}

/// The builder's retrieval time in the form the app stores its own.
fn retrieved_at(file: &PackFile) -> Result<String, AppError> {
    DateTime::parse_from_rfc3339(&file.retrieved_at)
        .map(|at| {
            at.with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Micros, true)
        })
        .map_err(|_| {
            AppError::DataParsing(format!(
                "{} lists a retrieval time that is not RFC 3339",
                file.path
            ))
        })
}

fn display_name(manifest: &PackManifest, file: &PackFile, retrieved_at: &str) -> String {
    let year = file.year.map(|year| format!(" {year}")).unwrap_or_default();
    format!(
        "{}{year} ({} {}), retrieved by the pack builder on {}",
        file.table,
        manifest.pack,
        manifest.pack_version,
        &retrieved_at[..10]
    )
}

/// Imports each of `selected` (paths as the manifest lists them) from the pack in `directory`,
/// only while its manifest is the one inspected (`inspected_manifest_sha256`). The manifest and
/// every selected file, and only those, are checked before the first import; files the pack lists
/// but the selection leaves out may be absent. Each file is then its own import, so one file's
/// failure is reported beside the others' results.
pub(crate) async fn import_files(
    repository: &DuckDbRepository,
    directory: PathBuf,
    inspected_manifest_sha256: &str,
    selected: Vec<String>,
    pinned: Vec<VerifyingKey>,
) -> Result<Vec<DataPackImportOutcome>, AppError> {
    // Verified again, not trusted from the inspection: the folder may have changed since.
    let (signed, files) = verify_chosen(
        directory.clone(),
        inspected_manifest_sha256.to_owned(),
        selected,
        pinned,
    )
    .await?;
    let manifest = &signed.manifest;
    let mut chosen: Vec<(&PackFile, AnalyticalDatasetKind, String)> = Vec::new();
    for file in &files {
        let kind = file_kind(manifest, file).ok_or_else(|| {
            AppError::DataParsing(format!("This app cannot import {}", file.path))
        })?;
        chosen.push((file, kind, retrieved_at(file)?));
    }
    if chosen.is_empty() {
        return Err(AppError::DataParsing(
            "Choose at least one file to import".into(),
        ));
    }
    let mut outcomes = Vec::with_capacity(chosen.len());
    for (file, kind, retrieved_at) in chosen {
        let request = AnalyticalImportRequest {
            dataset_kind: kind,
            display_name: display_name(manifest, file, &retrieved_at),
            source_uri: Some(file.source_url.clone()),
            local_path: directory.join(&file.path).to_string_lossy().into_owned(),
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
            signer_key_id: signed.signer_key_id.to_ascii_lowercase(),
            file_path: file.path.clone(),
            file_sha256: file.sha256.to_ascii_lowercase(),
            retrieved_at,
        };
        let imported = repository
            .import_analytical_csv_from_pack(request, origin)
            .await;
        let (result, already_stored, error) = match imported {
            Ok((result, reused)) => (Some(result), Some(reused), None),
            Err(error) => (None, None, Some(error.to_string())),
        };
        outcomes.push(DataPackImportOutcome {
            path: file.path.clone(),
            table: file.table.clone(),
            year: file.year,
            retrieved_at: file.retrieved_at.clone(),
            result,
            already_stored,
            error,
        });
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrity::sign_package;
    use crate::packs::{PackLicence, PACK_MANIFEST_FILE};
    use ed25519_dalek::SigningKey;
    use std::path::Path;

    const ACTUALS_URI: &str =
        "https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv";
    const ACTUALS: &[u8] = b"amount,report_fy,dept_name\n100.5,2024,Police\n7,2025,Library\n";
    const FILE: &str = "originals/budget_actuals/actuals_operating_datasd.csv";
    const BUILT_AT: &str = "2026-10-06T22:59:25.459212+00:00";
    /// A day before `BUILT_AT` in UTC, and written with an offset, so a test tells the file's
    /// retrieval from the build and sees it normalized.
    const RETRIEVED_AT: &str = "2026-10-05T16:59:15-07:00";
    const RETRIEVED_AT_UTC: &str = "2026-10-05T23:59:15.000000Z";

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[9_u8; 32])
    }

    fn manifest() -> PackManifest {
        PackManifest {
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
                retrieved_at: RETRIEVED_AT.into(),
                sha256: sha256_hex(ACTUALS),
                bytes: ACTUALS.len() as u64,
                rows: 2,
                excluded_rows: None,
                year: None,
                partial: true,
                last_modified: None,
                etag: None,
            }],
        }
    }

    /// Writes `manifest` signed, its signer key id in upper case: verification accepts either
    /// case, so storage has to normalize it.
    fn sign_into(directory: &Path, manifest: PackManifest) {
        let mut package = sign_package(manifest, &key()).unwrap();
        package.signature.signer_key_id = package.signature.signer_key_id.to_ascii_uppercase();
        std::fs::write(
            directory.join(PACK_MANIFEST_FILE),
            serde_json::to_vec_pretty(&package).unwrap(),
        )
        .unwrap();
    }

    /// A signed pack holding one `budget_actuals` original, with its URL as the builder records
    /// it (parsed and re-serialized).
    fn signed_pack() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, ACTUALS).unwrap();
        sign_into(directory.path(), manifest());
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

    async fn import_as_inspected(
        repository: &DuckDbRepository,
        directory: &Path,
        manifest_sha256: &str,
    ) -> Result<Vec<DataPackImportOutcome>, AppError> {
        import_files(
            repository,
            directory.to_path_buf(),
            manifest_sha256,
            vec![FILE.into()],
            vec![key().verifying_key()],
        )
        .await
    }

    /// Inspects the pack, as the app does before an import, and imports what it showed.
    async fn import(
        repository: &DuckDbRepository,
        directory: &Path,
    ) -> Result<Vec<DataPackImportOutcome>, AppError> {
        let inspected = inspect(directory.to_path_buf(), vec![key().verifying_key()])
            .await?
            .manifest_sha256;
        import_as_inspected(repository, directory, &inspected).await
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
    async fn a_pack_whose_signed_manifest_changed_writes_nothing() {
        let directory = signed_pack();
        let manifest = directory.path().join(PACK_MANIFEST_FILE);
        let text = std::fs::read_to_string(&manifest).unwrap();
        std::fs::write(&manifest, text.replacen("\"rows\": 2", "\"rows\": 3", 1)).unwrap();
        let repository = repository().await;
        // As if inspected after the change, so only verification stands in the way.
        let mut changed = self::manifest();
        changed.files[0].rows = 3;
        let digest = manifest_sha256(&changed).unwrap();
        let refused = import_as_inspected(&repository, directory.path(), &digest)
            .await
            .unwrap_err();
        assert!(refused.to_string().contains("refused"), "{refused}");
        assert_eq!(written(&repository).await, (0, 0));
    }

    #[tokio::test]
    async fn a_pack_with_one_changed_file_byte_writes_nothing() {
        let directory = signed_pack();
        let file = directory.path().join(FILE);
        let mut bytes = std::fs::read(&file).unwrap();
        bytes[0] ^= 1;
        std::fs::write(&file, bytes).unwrap();
        let repository = repository().await;
        let digest = manifest_sha256(&manifest()).unwrap();
        // Refused as a whole by verification, not reported as one file's failed import.
        let refused = import_as_inspected(&repository, directory.path(), &digest)
            .await
            .unwrap_err();
        assert!(refused.to_string().contains("refused"), "{refused}");
        assert!(refused.to_string().contains("SHA-256"), "{refused}");
        assert_eq!(written(&repository).await, (0, 0));
    }

    /// A published pack is downloaded only as far as the user chose, so an import checks the
    /// manifest and the files it imports, not files it leaves out.
    #[tokio::test]
    async fn an_import_verifies_only_the_files_it_imports() {
        let directory = signed_pack();
        let mut listed = manifest();
        let mut absent = listed.files[0].clone();
        absent.path = "originals/budget_adopted/budget_operating_datasd.csv".into();
        absent.table = "budget_adopted".into();
        listed.files.push(absent);
        sign_into(directory.path(), listed.clone());
        let repository = repository().await;
        let digest = manifest_sha256(&listed).unwrap();
        let outcomes = import_as_inspected(&repository, directory.path(), &digest)
            .await
            .unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].error, None);
        assert_eq!(written(&repository).await, (1, 1));
    }

    #[tokio::test]
    async fn a_manifest_signed_again_after_inspection_is_refused() {
        let directory = signed_pack();
        let inspected = inspect(directory.path().to_path_buf(), vec![key().verifying_key()])
            .await
            .unwrap()
            .manifest_sha256;
        // A different manifest, validly signed by the same key, now sits in the folder.
        let mut changed = manifest();
        changed.pack_version = "2026.10.7".into();
        sign_into(directory.path(), changed);
        let repository = repository().await;
        let refused = import_as_inspected(&repository, directory.path(), &inspected)
            .await
            .unwrap_err();
        assert!(
            refused.to_string().contains("since it was inspected"),
            "{refused}"
        );
        assert_eq!(written(&repository).await, (0, 0));
    }

    /// Shaped as `fetch_city_dataset` shapes OperatingActuals.
    fn fetched(directory: &Path) -> AnalyticalImportRequest {
        AnalyticalImportRequest {
            dataset_kind: AnalyticalDatasetKind::RegionalData,
            display_name: "City operating actuals (all published years)".into(),
            source_uri: Some(ACTUALS_URI.into()),
            local_path: directory.join(FILE).to_string_lossy().into_owned(),
            observation_period_start: None,
            observation_period_end: None,
            partial_dataset: true,
            worksheet: None,
            source_file_name: Some("actuals_operating_datasd.csv".into()),
            expected_source_sha256: Some(sha256_hex(ACTUALS)),
            derived_subset: None,
        }
    }

    /// The snapshot row's name and retrieval time, and the pack origin beside it.
    async fn recorded(repository: &DuckDbRepository) -> (String, String, Vec<String>, bool) {
        repository
            .query_connection("test", |conn| {
                conn.query_row(
                    "SELECT s.display_name, s.retrieved_at, p.pack, p.pack_version, p.built_at,
                            p.signer_key_id, p.file_path, p.file_sha256, p.retrieved_at,
                            p.describes_snapshot
                     FROM analytical_snapshots s JOIN pack_origins p USING (snapshot_id)",
                    [],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            (2..9)
                                .map(|column| row.get::<_, String>(column))
                                .collect::<Result<Vec<_>, _>>()?,
                            row.get(9)?,
                        ))
                    },
                )
                .map_err(|error| AppError::Storage(error.to_string()))
            })
            .await
            .unwrap()
    }

    fn expected_origin() -> Vec<String> {
        vec![
            "san-diego-money".into(),
            "2026.10.6".into(),
            BUILT_AT.into(),
            sha256_hex(key().verifying_key().as_bytes()),
            FILE.into(),
            sha256_hex(ACTUALS),
            RETRIEVED_AT_UTC.into(),
        ]
    }

    /// The same bytes fetched by the user and imported from a pack, in either order, are one
    /// snapshot whose row is the user's acquisition, with the pack recorded beside it.
    #[tokio::test]
    async fn the_same_bytes_fetched_and_from_a_pack_are_one_snapshot_with_both_origins() {
        // The user's fetch first.
        let directory = signed_pack();
        let repository = repository().await;
        let user = repository
            .import_analytical_csv(fetched(directory.path()))
            .await
            .unwrap();
        let outcomes = import(&repository, directory.path()).await.unwrap();
        assert_eq!(outcomes[0].error, None);
        assert_eq!(outcomes[0].already_stored, Some(true));
        let from_pack = outcomes[0].result.as_ref().unwrap();
        assert_eq!(from_pack.snapshot_id, user.snapshot_id);
        assert_eq!(from_pack.display_name, user.display_name);
        assert_eq!(written(&repository).await, (1, 1));
        let (name, retrieved, origin, describes) = recorded(&repository).await;
        assert_eq!(name, user.display_name);
        assert_ne!(retrieved, RETRIEVED_AT_UTC);
        assert_eq!(origin, expected_origin());
        assert!(!describes);

        // The pack first: its row states the builder's retrieval until the user's fetch.
        let directory = signed_pack();
        let repository = self::repository().await;
        let outcomes = import(&repository, directory.path()).await.unwrap();
        assert_eq!(outcomes[0].already_stored, Some(false));
        let (name, retrieved, origin, describes) = recorded(&repository).await;
        assert!(
            name.ends_with("retrieved by the pack builder on 2026-10-05"),
            "{name}"
        );
        assert_eq!(retrieved, RETRIEVED_AT_UTC);
        assert_eq!(origin, expected_origin());
        assert!(describes);
        let user = repository
            .import_analytical_csv(fetched(directory.path()))
            .await
            .unwrap();
        assert_eq!(user.display_name, fetched(directory.path()).display_name);
        assert_eq!(written(&repository).await, (1, 1));
        let (name, retrieved, origin, describes) = recorded(&repository).await;
        assert_eq!(name, user.display_name);
        assert_ne!(retrieved, RETRIEVED_AT_UTC);
        assert_eq!(origin, expected_origin());
        assert!(!describes);
    }

    /// A core from 1.0.0, before pack import, has no `pack_origins`; opening it must create the
    /// table. The analytical tables are built here in their 1.0.0 shape.
    #[tokio::test]
    async fn a_core_from_before_pack_import_records_pack_origins() {
        let repository = DuckDbRepository::new(":memory:").unwrap();
        repository
            .query_connection("1.0.0 analytical tables", |conn| {
                conn.execute_batch(
                    "CREATE TABLE analytical_snapshots (
                        snapshot_id TEXT PRIMARY KEY, dataset_kind TEXT NOT NULL,
                        display_name TEXT NOT NULL, table_name TEXT NOT NULL, source_uri TEXT,
                        local_path TEXT NOT NULL, source_sha256 TEXT NOT NULL,
                        retrieved_at TEXT NOT NULL, observation_period_start TEXT,
                        observation_period_end TEXT, partial_dataset BIGINT NOT NULL DEFAULT 0,
                        imported_rows BIGINT NOT NULL DEFAULT 0,
                        rejected_rows BIGINT NOT NULL DEFAULT 0,
                        malformed_rows BIGINT NOT NULL DEFAULT 0,
                        duplicate_key_rows BIGINT NOT NULL DEFAULT 0,
                        validation_summary_json TEXT NOT NULL,
                        CHECK (length(source_sha256) = 64), CHECK (partial_dataset IN (0, 1)));
                     CREATE TABLE analytical_columns (
                        snapshot_id TEXT NOT NULL, table_name TEXT NOT NULL,
                        column_name TEXT NOT NULL, original_name TEXT,
                        logical_type TEXT NOT NULL, nullable BIGINT NOT NULL DEFAULT 1,
                        dictionary TEXT, ordinal BIGINT NOT NULL,
                        PRIMARY KEY (snapshot_id, column_name),
                        FOREIGN KEY (snapshot_id) REFERENCES analytical_snapshots(snapshot_id));
                     CREATE TABLE analytical_row_ledger (
                        snapshot_id TEXT NOT NULL, row_number BIGINT NOT NULL,
                        row_hash TEXT NOT NULL, source_payload_json TEXT NOT NULL,
                        normalized_payload_json TEXT NOT NULL,
                        malformed BIGINT NOT NULL DEFAULT 0, validation_errors_json TEXT NOT NULL,
                        PRIMARY KEY (snapshot_id, row_number),
                        FOREIGN KEY (snapshot_id) REFERENCES analytical_snapshots(snapshot_id));
                     CREATE TABLE data_maintenance_origins (
                        snapshot_id TEXT NOT NULL, portal_key TEXT NOT NULL,
                        document_id BIGINT NOT NULL,
                        PRIMARY KEY (snapshot_id, portal_key, document_id));",
                )
                .map_err(|error| AppError::Storage(error.to_string()))
            })
            .await
            .unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .expect("a 1.0.0 core must open");
        let directory = signed_pack();
        let outcomes = import(&repository, directory.path()).await.unwrap();
        assert_eq!(outcomes[0].error, None);
        assert_eq!(written(&repository).await, (1, 1));
    }

    /// Imports every original of a pack the builder produced (`ONRECORD_PACK_DIR`), verified
    /// against the key this app pins.
    #[tokio::test]
    #[ignore = "needs ONRECORD_PACK_DIR"]
    async fn a_built_pack_imports_against_the_pinned_key() {
        let directory = PathBuf::from(std::env::var("ONRECORD_PACK_DIR").unwrap());
        let summary = inspect(directory.clone(), crate::packs::pinned_pack_keys())
            .await
            .unwrap();
        let files = summary
            .tables
            .iter()
            .flat_map(|table| table.files.iter())
            .filter(|file| file.dataset_kind.is_some())
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let repository = repository().await;
        let outcomes = import_files(
            &repository,
            directory,
            &summary.manifest_sha256,
            files,
            crate::packs::pinned_pack_keys(),
        )
        .await
        .unwrap();
        for outcome in &outcomes {
            assert_eq!(outcome.error, None, "{}", outcome.path);
        }
        assert_eq!(
            written(&repository).await,
            (outcomes.len() as i64, outcomes.len() as i64)
        );
    }
}
