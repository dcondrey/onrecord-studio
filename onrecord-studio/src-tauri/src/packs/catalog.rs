//! The packs' tables and the City dataset each is built from: the one list the builder retrieves
//! by and the app imports by (`docs/data-packs.md`).

use crate::error::AppError;
use crate::ingestion::queue::dataset_source;
use crate::types::{AnalyticalDatasetKind, CityDataset};

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

/// The City open-data tables of `pack`.
pub(crate) fn pack_tables(pack: &str) -> Result<&'static [PackTable], AppError> {
    match pack {
        "san-diego-money" => Ok(MONEY),
        "san-diego-reports" => Ok(REPORTS),
        other => Err(AppError::DataParsing(format!(
            "Unknown pack '{other}'; packs are san-diego-money and san-diego-reports"
        ))),
    }
}

/// The kind the app imports a pack's original file as: that of the source-list dataset that
/// publishes `file_name` at `source_url` for `table` and `year`. The snapshot id is prefixed by
/// its kind, so only the kind the user's own fetch uses keeps the same bytes one snapshot. `None`
/// when the file is not one the source list publishes.
pub(crate) fn original_kind(
    pack: &str,
    table: &str,
    year: Option<u16>,
    file_name: &str,
    source_url: &str,
    current_year: i32,
) -> Option<AnalyticalDatasetKind> {
    let source_url = reqwest::Url::parse(source_url).ok()?;
    pack_tables(pack)
        .ok()?
        .iter()
        .filter(|entry| entry.table == table)
        .filter_map(|entry| dataset_source(entry.dataset, year, current_year).ok())
        .find(|source| {
            source.file_name == file_name
                && reqwest::Url::parse(&source.url).is_ok_and(|url| url == source_url)
        })
        .map(|source| source.kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTUALS: &str =
        "https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv";

    #[test]
    fn an_original_takes_the_kind_of_the_dataset_that_publishes_it() {
        let kind = |pack, table, year, name, url| original_kind(pack, table, year, name, url, 2026);
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
}
