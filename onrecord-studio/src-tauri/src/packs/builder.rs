//! The data-pack builder: retrieves a pack's City files through the app's own source list and
//! downloader, records what the publisher served, and signs the manifest with the publisher key
//! kept in the macOS Keychain. Built only with the `pack-builder` feature.

use super::{PackFile, PackFileFormat, PackLicence, PackManifest, PACK_MANIFEST_FILE};
use crate::error::AppError;
use crate::ingestion::client::{ApiClient, HttpClientConfig};
use crate::ingestion::queue::dataset_source;
use crate::storage::tabular::{TableSource, MAX_STREAMED_BYTES};
use crate::types::CityDataset;
use chrono::{Datelike, Utc};
use ed25519_dalek::SigningKey;
use std::path::{Path, PathBuf};

const KEYCHAIN_SERVICE: &str = "com.onrecord.pack-publisher";
const KEYCHAIN_ACCOUNT: &str = "publisher-seed-v1";
/// The earliest year asked of a table split by year; the source list refuses years before its
/// publisher's series, and those are skipped.
const EARLIEST_YEAR: u16 = 2000;

/// One conformed table of a pack and the City dataset it is built from.
struct PackTable {
    table: &'static str,
    dataset: CityDataset,
}

/// The packs' City open-data tables (`docs/data-packs.md`).
fn pack_tables(pack: &str) -> Result<&'static [PackTable], AppError> {
    const MONEY: &[PackTable] = &[
        PackTable {
            table: "budget_adopted",
            dataset: CityDataset::OperatingBudget,
        },
        PackTable {
            table: "budget_actuals",
            dataset: CityDataset::OperatingActuals,
        },
        PackTable {
            table: "capital_budget",
            dataset: CityDataset::CapitalBudget,
        },
        PackTable {
            table: "capital_actuals",
            dataset: CityDataset::CapitalActuals,
        },
        PackTable {
            table: "city_properties",
            dataset: CityDataset::CityProperties,
        },
        PackTable {
            table: "city_parcels",
            dataset: CityDataset::CityParcels,
        },
        PackTable {
            table: "city_leases",
            dataset: CityDataset::CityLeases,
        },
    ];
    const REPORTS: &[PackTable] = &[
        PackTable {
            table: "service_reports",
            dataset: CityDataset::GetItDoneEncampments,
        },
        PackTable {
            table: "service_reports",
            dataset: CityDataset::GetItDoneOpen,
        },
        PackTable {
            table: "police_calls",
            dataset: CityDataset::CallsForService,
        },
    ];
    match pack {
        "san-diego-money" => Ok(MONEY),
        "san-diego-reports" => Ok(REPORTS),
        other => Err(AppError::DataParsing(format!(
            "Unknown pack '{other}'; packs are san-diego-money and san-diego-reports"
        ))),
    }
}

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
    let mut files = Vec::new();
    for table in tables {
        for year in published_years(table.dataset, now.year()) {
            let (url, file_name, partial) = dataset_source(table.dataset, year, now.year())?;
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
            files.push(PackFile {
                path: relative,
                table: table.table.into(),
                format: PackFileFormat::Original,
                source_url: url.to_string(),
                retrieved_at: downloaded.acquisition.retrieved_at,
                sha256: downloaded.sha256,
                bytes: downloaded.byte_count,
                rows,
                year,
                partial,
                last_modified: downloaded.acquisition.last_modified,
                etag: downloaded.acquisition.etag,
            });
        }
    }
    let manifest = PackManifest {
        pack: pack.into(),
        pack_version: format!("{}.{}.{}", now.year(), now.month(), now.day()),
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
