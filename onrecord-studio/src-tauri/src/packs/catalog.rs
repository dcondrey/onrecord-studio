//! The packs' tables and the files each is built from, a City dataset or a source registry entry:
//! the one list the builder retrieves by and the app imports by (`docs/data-packs.md`).

use super::sources::{file_name, registry, registry_packs};
use crate::error::AppError;
use crate::ingestion::queue::dataset_source;
use crate::types::{AnalyticalDatasetKind, CityDataset, DataPackFileLicence};
use chrono::Datelike;

/// One conformed table of a pack and the City dataset it is built from.
pub(crate) struct PackTable {
    pub table: &'static str,
    pub dataset: CityDataset,
}

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
        table: "budget_accounts",
        dataset: CityDataset::BudgetAccounts,
    },
    PackTable {
        table: "budget_departments",
        dataset: CityDataset::BudgetDepartments,
    },
    PackTable {
        table: "budget_funds",
        dataset: CityDataset::BudgetFunds,
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
    PackTable {
        table: "call_types",
        dataset: CityDataset::CallTypes,
    },
    PackTable {
        table: "call_dispositions",
        dataset: CityDataset::DispositionCodes,
    },
    PackTable {
        table: "police_beats",
        dataset: CityDataset::BeatCodes,
    },
    PackTable {
        table: "fire_ems_incidents",
        dataset: CityDataset::FireEmsIncidents,
    },
];

const ENFORCEMENT: &[PackTable] = &[
    PackTable {
        table: "police_stops",
        dataset: CityDataset::RipaStops,
    },
    PackTable {
        table: "police_stop_reasons",
        dataset: CityDataset::RipaStopReasons,
    },
    PackTable {
        table: "police_stop_results",
        dataset: CityDataset::RipaStopResults,
    },
    PackTable {
        table: "police_stop_seizure_basis",
        dataset: CityDataset::RipaPropertySeizureBasis,
    },
    PackTable {
        table: "police_offenses",
        dataset: CityDataset::NibrsOffenses,
    },
    PackTable {
        table: "parking_citations",
        dataset: CityDataset::ParkingCitationsFirstHalf,
    },
    PackTable {
        table: "parking_citations",
        dataset: CityDataset::ParkingCitationsSecondHalf,
    },
    PackTable {
        table: "traffic_collisions",
        dataset: CityDataset::TrafficCollisions,
    },
    PackTable {
        table: "traffic_collision_people",
        dataset: CityDataset::TrafficCollisionPeople,
    },
    PackTable {
        table: "hate_crimes",
        dataset: CityDataset::HateCrimes,
    },
];

const CAPACITY: &[PackTable] = &[
    PackTable {
        table: "approvals_created",
        dataset: CityDataset::ApprovalsCreated,
    },
    PackTable {
        table: "approvals_issued",
        dataset: CityDataset::ApprovalsIssued,
    },
    PackTable {
        table: "approvals_closed",
        dataset: CityDataset::ApprovalsClosed,
    },
];

/// Every pack by name. Each holds only City open data published under ODC-PDDL.
const PACKS: &[(&str, &[PackTable])] = &[
    ("san-diego-money", MONEY),
    ("san-diego-reports", REPORTS),
    ("san-diego-enforcement", ENFORCEMENT),
    ("san-diego-capacity", CAPACITY),
];

/// The pack a scheduled job extends every day with dated snapshots of the files publishers
/// overwrite in place (`docs/data-packs.md`, Snapshots).
pub(crate) const SNAPSHOT_PACK: &str = "san-diego-snapshots";

/// The City packs' names, the snapshot pack's among them.
pub(crate) fn city_pack_names() -> impl Iterator<Item = &'static str> {
    PACKS.iter().map(|(name, _)| *name).chain([SNAPSHOT_PACK])
}

/// Every pack's name: the City packs, then the registry packs.
pub(crate) fn pack_names() -> impl Iterator<Item = &'static str> {
    city_pack_names().chain(registry_packs())
}

/// The earliest year asked of a City table split by year; the source list refuses years before
/// its publisher's series, and those are skipped.
const EARLIEST_YEAR: u16 = 2000;

/// The years a City table is published for, or `[None]` for a table published as one file.
fn published_years(dataset: CityDataset, current_year: i32) -> Vec<Option<u16>> {
    if dataset_source(dataset, None, current_year).is_ok() {
        return vec![None];
    }
    (EARLIEST_YEAR..=current_year as u16)
        .filter(|year| dataset_source(dataset, Some(*year), current_year).is_ok())
        .map(Some)
        .collect()
}

/// One file a pack is built from, and how the app imports it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PublishedFile {
    pub table: &'static str,
    pub year: Option<u16>,
    pub url: String,
    pub file_name: String,
    pub partial: bool,
    pub kind: AnalyticalDatasetKind,
    pub worksheet: Option<&'static str>,
    /// The file's own licence; `None` for a City file, which the pack's licence covers.
    pub licence: Option<DataPackFileLicence>,
}

/// Every file `pack` is built from: its City datasets' files, or its registry entries'.
pub(crate) fn published_files(
    pack: &str,
    current_year: i32,
) -> Result<Vec<PublishedFile>, AppError> {
    let entries = registry()
        .iter()
        .filter(|entry| entry.pack == pack)
        .collect::<Vec<_>>();
    if !entries.is_empty() {
        let mut files = Vec::new();
        for entry in entries {
            for year in entry.published_years() {
                let url = entry.url_for(year);
                files.push(PublishedFile {
                    table: &entry.table,
                    year,
                    file_name: file_name(&url).ok_or_else(|| {
                        AppError::Internal(format!("Registry URL {url} has no file name"))
                    })?,
                    url,
                    partial: entry.partial_for(year),
                    kind: AnalyticalDatasetKind::RegionalData,
                    worksheet: entry.worksheet.as_deref(),
                    licence: Some(entry.file_licence()),
                });
            }
        }
        return Ok(files);
    }
    let mut files = Vec::new();
    for table in pack_tables(pack)? {
        for year in published_years(table.dataset, current_year) {
            let source = dataset_source(table.dataset, year, current_year)?;
            files.push(PublishedFile {
                table: table.table,
                year,
                url: source.url,
                file_name: source.file_name,
                partial: source.partial,
                kind: source.kind,
                worksheet: None,
                licence: None,
            });
        }
    }
    Ok(files)
}

/// Whether `pack` is built from the source registry, so its files carry their own licences.
#[cfg_attr(not(feature = "pack-builder"), allow(dead_code))]
pub(crate) fn is_registry_pack(pack: &str) -> bool {
    registry_packs().contains(&pack)
}

/// The snapshot pack's tables: every catalog table published as one file rather than by year,
/// since that one file is what its publisher overwrites.
pub(crate) fn snapshot_tables(current_year: i32) -> Vec<&'static PackTable> {
    let mut tables: Vec<&'static PackTable> = Vec::new();
    for entry in PACKS.iter().flat_map(|(_, tables)| tables.iter()) {
        if dataset_source(entry.dataset, None, current_year).is_ok()
            && !tables.iter().any(|kept| kept.dataset == entry.dataset)
        {
            tables.push(entry);
        }
    }
    tables
}

/// The City open-data tables of `pack`.
pub(crate) fn pack_tables(pack: &str) -> Result<Vec<&'static PackTable>, AppError> {
    if pack == SNAPSHOT_PACK {
        return Ok(snapshot_tables(chrono::Utc::now().year()));
    }
    PACKS
        .iter()
        .find(|(name, _)| *name == pack)
        .map(|(_, tables)| tables.iter().collect())
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "Unknown pack '{pack}'; packs are {}",
                pack_names().collect::<Vec<_>>().join(", ")
            ))
        })
}

/// How the app imports a pack's original file: as the City dataset or registry entry of `pack`
/// that publishes `file_name` at `source_url` for `table` and `year`. The snapshot id is prefixed
/// by its kind, so only the kind the user's own fetch uses keeps the same bytes one snapshot.
/// `None` when no source list publishes the file.
pub(crate) fn original_source(
    pack: &str,
    table: &str,
    year: Option<u16>,
    file_name: &str,
    source_url: &str,
    current_year: i32,
) -> Option<PublishedFile> {
    let source_url = reqwest::Url::parse(source_url).ok()?;
    published_files(pack, current_year)
        .ok()?
        .into_iter()
        .find(|file| {
            file.table == table
                && file.year == year
                && file.file_name == file_name
                && reqwest::Url::parse(&file.url).is_ok_and(|url| url == source_url)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table the builder cannot conform, or a dataset with no published file, fails a build only
    /// after every earlier file has been downloaded.
    #[test]
    fn every_pack_table_is_published_and_has_a_conformed_definition() {
        for (pack, tables) in PACKS {
            for entry in *tables {
                assert!(
                    super::super::conformed::has_definition(entry.table),
                    "{pack} {}",
                    entry.table
                );
                assert!(
                    dataset_source(entry.dataset, None, 2026).is_ok()
                        || dataset_source(entry.dataset, Some(2025), 2026).is_ok(),
                    "{pack} {:?}",
                    entry.dataset
                );
            }
        }
    }

    /// Each snapshot table is one publisher file under its own name, so its dated snapshots and
    /// change records never share a path with another file's.
    #[test]
    fn the_snapshot_tables_are_the_catalog_tables_published_as_one_file() {
        let tables = snapshot_tables(2026)
            .iter()
            .map(|entry| entry.table)
            .collect::<Vec<_>>();
        assert_eq!(
            tables,
            [
                "budget_adopted",
                "budget_actuals",
                "capital_budget",
                "capital_actuals",
                "budget_accounts",
                "budget_departments",
                "budget_funds",
                "city_properties",
                "city_parcels",
                "city_leases",
                "service_reports",
                "call_types",
                "call_dispositions",
                "police_beats",
                "police_stops",
                "police_stop_reasons",
                "police_stop_results",
                "police_stop_seizure_basis",
                "traffic_collisions",
                "traffic_collision_people",
                "hate_crimes",
            ]
        );
        assert_eq!(
            snapshot_tables(2026)
                .iter()
                .find(|entry| entry.table == "service_reports")
                .map(|entry| entry.dataset),
            Some(CityDataset::GetItDoneOpen)
        );
    }

    const ACTUALS: &str =
        "https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv";

    #[test]
    fn an_original_takes_the_kind_of_the_dataset_that_publishes_it() {
        let kind = |pack, table, year, name, url| {
            original_source(pack, table, year, name, url, 2026).map(|file| file.kind)
        };
        assert_eq!(
            kind(
                "san-diego-reports",
                "police_calls",
                Some(2025),
                "pd_calls_for_service_2025_datasd.csv",
                "https://seshat.datasd.org/police_calls_for_service/pd_calls_for_service_2025_datasd.csv"
            ),
            Some(AnalyticalDatasetKind::CallsForService)
        );
        let actuals = "actuals_operating_datasd.csv";
        assert_eq!(
            kind("san-diego-money", "budget_actuals", None, actuals, ACTUALS),
            Some(AnalyticalDatasetKind::RegionalData)
        );
        // Not what the source list publishes: another name, address, table or pack.
        for (pack, table, name, url) in [
            ("san-diego-money", "budget_actuals", "renamed.csv", ACTUALS),
            (
                "san-diego-money",
                "budget_actuals",
                actuals,
                "https://example.test/actuals_operating_datasd.csv",
            ),
            ("san-diego-money", "budget_adopted", actuals, ACTUALS),
            ("san-diego-reports", "budget_actuals", actuals, ACTUALS),
        ] {
            assert_eq!(
                kind(pack, table, None, name, url),
                None,
                "{table} {name} {url}"
            );
        }
    }

    /// Every registry pack is a curated release: the owner's key signs it, the snapshot key never.
    #[test]
    fn a_registry_pack_resolves_its_files_and_is_signed_only_by_the_curated_key() {
        let pinned = super::super::pinned_pack_keys();
        let hex = |kid: &str| {
            super::super::PACK_PUBLIC_KEYS
                .iter()
                .find(|(name, _, _)| *name == kid)
                .unwrap()
                .1
        };
        for pack in registry_packs() {
            assert!(super::super::pinned_signer(&pinned, hex("pack-2026-1"), pack).is_ok());
            assert!(super::super::pinned_signer(&pinned, hex("pack-ci-2026-1"), pack).is_err());
            for file in published_files(pack, 2026).unwrap() {
                assert_eq!(file.kind, AnalyticalDatasetKind::RegionalData);
                assert!(file.licence.is_some(), "{}", file.url);
                assert_eq!(
                    original_source(
                        pack,
                        file.table,
                        file.year,
                        &file.file_name,
                        &file.url,
                        2026
                    ),
                    Some(file.clone())
                );
            }
        }
        let entry = &registry()[0];
        let files = published_files(&entry.pack, 2026).unwrap();
        assert_eq!(files[0].url, entry.url);
        assert_eq!(files[0].file_name, file_name(&entry.url).unwrap());
    }
}
