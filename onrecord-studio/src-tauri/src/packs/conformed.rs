//! The conformed layer of `docs/data-packs.md`: one SELECT over an imported analytical snapshot
//! that keeps every typed publisher column and adds the standard Identity, Provenance and Time
//! columns. The pack builder copies it to Parquet; the app creates `analysis_conformed_<table>`
//! over every snapshot imported from a pack file of that table. Place waits for the pre-2021
//! council district lines.
//!
//! Standard columns are read from the snapshot's stored original cells, not its typed columns: a
//! regional table's types are inferred per file, while the original text is what the publisher
//! recorded. Absent or unreadable values are NULL, and a derived value is NULL whenever its basis
//! is.

use super::sources::RegistryTimeKind;
use crate::error::AppError;
use crate::storage::sqlite::quote_ident;
use crate::storage::DuckDbRepository;
use duckdb::{params, Connection, OptionalExt};

/// Where a conformed table's time comes from.
#[derive(Clone, Copy)]
enum TimeColumn {
    /// A wall-clock reading. The City publishes Pacific local time with no offset, so it stays a
    /// zone-less TIMESTAMP: a value that carries an offset is not read rather than shifted.
    RecordedAt(&'static str),
    /// A fiscal year named by the June it ends in: the City's last two digits (`23`) or the State
    /// Controller's four (`2024` is 2023-24, per its dataset description). It names no day, so `occurred_at`, `occurred_on` and `calendar_year` stay NULL.
    FiscalYear(&'static str),
    /// A four-digit calendar year. It names no day, so `occurred_at` and `occurred_on` stay NULL,
    /// and no fiscal year, since a calendar year spans two.
    CalendarYear(&'static str),
    /// A reference list or a holdings snapshot: its rows record no event time.
    Undated,
}

#[derive(Clone, Copy)]
struct ConformedSpec {
    table: &'static str,
    /// The publisher's record key, matched to its header without regard to case.
    key: Option<&'static str>,
    time: TimeColumn,
}

/// Every pack table; headers as the publisher's files spell them, matched without regard to case.
const SPECS: &[ConformedSpec] = &[
    ConformedSpec {
        table: "budget_adopted",
        key: None,
        time: TimeColumn::FiscalYear("report_fy"),
    },
    ConformedSpec {
        table: "budget_actuals",
        key: None,
        time: TimeColumn::FiscalYear("report_fy"),
    },
    ConformedSpec {
        table: "capital_budget",
        key: None,
        time: TimeColumn::FiscalYear("report_fy"),
    },
    ConformedSpec {
        table: "capital_actuals",
        key: None,
        time: TimeColumn::FiscalYear("report_fy"),
    },
    ConformedSpec {
        table: "budget_accounts",
        key: Some("account_number"),
        time: TimeColumn::Undated,
    },
    ConformedSpec {
        table: "budget_departments",
        key: None,
        time: TimeColumn::Undated,
    },
    ConformedSpec {
        table: "budget_funds",
        key: Some("fund_number"),
        time: TimeColumn::Undated,
    },
    ConformedSpec {
        table: "city_properties",
        key: None,
        time: TimeColumn::Undated,
    },
    ConformedSpec {
        table: "city_parcels",
        key: None,
        time: TimeColumn::Undated,
    },
    ConformedSpec {
        table: "city_leases",
        key: None,
        time: TimeColumn::RecordedAt("date_effective"),
    },
    ConformedSpec {
        table: "service_reports",
        key: Some("service_request_id"),
        time: TimeColumn::RecordedAt("date_requested"),
    },
    ConformedSpec {
        table: "police_calls",
        key: Some("incident_num"),
        time: TimeColumn::RecordedAt("date_time"),
    },
    ConformedSpec {
        table: "call_types",
        key: Some("call_type"),
        time: TimeColumn::Undated,
    },
    ConformedSpec {
        table: "call_dispositions",
        key: Some("dispo_code"),
        time: TimeColumn::Undated,
    },
    ConformedSpec {
        table: "police_beats",
        key: Some("beat"),
        time: TimeColumn::Undated,
    },
    ConformedSpec {
        table: "fire_ems_incidents",
        key: None,
        time: TimeColumn::RecordedAt("date_response"),
    },
    ConformedSpec {
        table: "police_stops",
        key: None,
        time: TimeColumn::RecordedAt("stopdate"),
    },
    ConformedSpec {
        table: "police_stop_reasons",
        key: None,
        time: TimeColumn::Undated,
    },
    ConformedSpec {
        table: "police_stop_results",
        key: None,
        time: TimeColumn::Undated,
    },
    ConformedSpec {
        table: "police_stop_seizure_basis",
        key: None,
        time: TimeColumn::Undated,
    },
    ConformedSpec {
        table: "police_offenses",
        key: Some("nibrs_uniq"),
        time: TimeColumn::RecordedAt("occured_on"),
    },
    ConformedSpec {
        table: "parking_citations",
        key: Some("citation_id"),
        time: TimeColumn::RecordedAt("date_issue"),
    },
    ConformedSpec {
        table: "traffic_collisions",
        key: Some("report_id"),
        time: TimeColumn::RecordedAt("date_time"),
    },
    ConformedSpec {
        table: "traffic_collision_people",
        key: None,
        time: TimeColumn::RecordedAt("date_time"),
    },
    ConformedSpec {
        table: "hate_crimes",
        key: None,
        time: TimeColumn::RecordedAt("date"),
    },
    ConformedSpec {
        table: "approvals_created",
        key: Some("approval_id"),
        time: TimeColumn::RecordedAt("approval_create_date"),
    },
    ConformedSpec {
        table: "approvals_issued",
        key: Some("approval_id"),
        time: TimeColumn::RecordedAt("approval_issue_date"),
    },
    ConformedSpec {
        table: "approvals_closed",
        key: Some("approval_id"),
        time: TimeColumn::RecordedAt("approval_close_date"),
    },
];

const STANDARD_COLUMNS: &[&str] = &[
    "record_id",
    "publisher_record_id",
    "source_id",
    "source_url",
    "retrieved_at",
    "source_sha256",
    "source_row",
    "pack_version",
    "occurred_at",
    "occurred_on",
    "calendar_year",
    "fiscal_year",
    "time_basis",
];

/// What the snapshot does not record about where its file came from. `analytical_snapshots`
/// holds the import time, not the retrieval time, so retrieval is the caller's to state or leave
/// absent.
#[derive(Debug, Clone, Default)]
pub struct ConformedOrigin {
    /// When the file was retrieved from its publisher, as recorded (RFC 3339).
    pub retrieved_at: Option<String>,
    /// The data pack the file came from, if it came from one.
    pub pack_version: Option<String>,
}

pub(super) fn literal(value: Option<&str>) -> String {
    value.map_or_else(
        || "CAST(NULL AS VARCHAR)".into(),
        |text| format!("'{}'", text.replace('\'', "''")),
    )
}

fn storage(context: &'static str) -> impl Fn(duckdb::Error) -> AppError {
    move |error| AppError::Storage(format!("{context}: {error}"))
}

#[cfg(test)]
pub(super) fn has_definition(table: &str) -> bool {
    spec(table).is_ok()
}

/// The City packs' conformed tables, whose names a registry entry may not reuse.
pub(super) fn city_tables() -> Vec<&'static str> {
    SPECS.iter().map(|spec| spec.table).collect()
}

/// A City table's definition, or a registry table's from its entry.
fn spec(table: &str) -> Result<ConformedSpec, AppError> {
    if let Some(spec) = SPECS.iter().find(|spec| spec.table == table) {
        return Ok(*spec);
    }
    let entry = super::sources::registry()
        .iter()
        .find(|entry| entry.table == table)
        .ok_or_else(|| AppError::DataParsing(format!("'{table}' has no conformed definition")))?;
    let column = entry.time.column.as_deref().unwrap_or_default();
    Ok(ConformedSpec {
        table: &entry.table,
        key: entry.key.as_deref(),
        time: match entry.time.kind {
            RegistryTimeKind::RecordedAt => TimeColumn::RecordedAt(column),
            RegistryTimeKind::FiscalYear => TimeColumn::FiscalYear(column),
            RegistryTimeKind::CalendarYear => TimeColumn::CalendarYear(column),
            RegistryTimeKind::Undated => TimeColumn::Undated,
        },
    })
}

/// The publisher's record key header of `table`, if it has one.
#[cfg_attr(not(feature = "pack-builder"), allow(dead_code))]
pub(super) fn record_key(table: &str) -> Result<Option<&'static str>, AppError> {
    spec(table).map(|spec| spec.key)
}

/// The SQL alias the import gave the publisher column `header` in `snapshot_id`.
fn column_alias(conn: &Connection, snapshot_id: &str, header: &str) -> Result<String, AppError> {
    conn.query_row(
        "SELECT column_name FROM analytical_columns
         WHERE snapshot_id = ?1 AND lower(original_name) = lower(?2)",
        params![snapshot_id, header],
        |row| row.get(0),
    )
    .optional()
    .map_err(storage("Conformed columns"))?
    .ok_or_else(|| {
        AppError::DataParsing(format!(
            "Snapshot {snapshot_id} has no '{header}' column, which its conformed table needs"
        ))
    })
}

/// The publisher columns whose alias is a standard column's name, each with the name it keeps:
/// the alias followed by `_as_published` as many times as it takes to name no other column. The
/// rename happens before the standard columns are derived, since their expressions refer to one
/// another by name and would otherwise read the publisher's column.
fn renamed_publisher_columns(aliases: &[String]) -> Vec<(String, String)> {
    let mut taken = aliases
        .iter()
        .map(|alias| alias.to_ascii_lowercase())
        .chain(STANDARD_COLUMNS.iter().map(|column| (*column).to_owned()))
        .collect::<std::collections::BTreeSet<_>>();
    aliases
        .iter()
        .filter(|alias| STANDARD_COLUMNS.contains(&alias.to_ascii_lowercase().as_str()))
        .map(|alias| {
            let mut kept = format!("{alias}_as_published");
            while !taken.insert(kept.to_ascii_lowercase()) {
                kept.push_str("_as_published");
            }
            (alias.clone(), kept)
        })
        .collect()
}

/// A SELECT of every typed column of `snapshot_id` plus the standard columns, for `table`.
pub fn conformed_select(
    conn: &Connection,
    table: &str,
    snapshot_id: &str,
    origin: &ConformedOrigin,
) -> Result<String, AppError> {
    select_for(conn, spec(table)?, snapshot_id, origin)
}

fn select_for(
    conn: &Connection,
    spec: ConformedSpec,
    snapshot_id: &str,
    origin: &ConformedOrigin,
) -> Result<String, AppError> {
    let (typed, store, source_url, source_sha256): (
        String,
        Option<String>,
        Option<String>,
        String,
    ) = conn
        .query_row(
            "SELECT table_name, json_extract_string(validation_summary_json, '$.rowStore'),
                    source_uri, source_sha256
             FROM analytical_snapshots WHERE snapshot_id = ?1",
            params![snapshot_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(storage("Conformed snapshot"))?
        .ok_or_else(|| AppError::DataParsing(format!("No analytical snapshot {snapshot_id}")))?;
    let store = store.ok_or_else(|| {
        AppError::DataParsing(format!(
            "Snapshot {snapshot_id} predates stored original cells; import its file again"
        ))
    })?;
    let mut aliases = conn
        .prepare("SELECT column_name FROM analytical_columns WHERE snapshot_id = ?1")
        .map_err(storage("Conformed columns"))?;
    let aliases = aliases
        .query_map(params![snapshot_id], |row| row.get::<_, String>(0))
        .map_err(storage("Conformed columns"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage("Conformed columns"))?;
    let internal = aliases
        .iter()
        .filter(|alias| alias.starts_with("__"))
        .map(String::as_str)
        .collect::<Vec<_>>();
    if !internal.is_empty() {
        return Err(AppError::DataParsing(format!(
            "Snapshot {snapshot_id} has publisher columns named like internal columns: {}",
            internal.join(", ")
        )));
    }
    let renamed = renamed_publisher_columns(&aliases);
    let publisher_columns = if renamed.is_empty() {
        "t.*".to_owned()
    } else {
        format!(
            "t.* EXCLUDE ({}), {}",
            renamed
                .iter()
                .map(|(alias, _)| quote_ident(alias))
                .collect::<Vec<_>>()
                .join(", "),
            renamed
                .iter()
                .map(|(alias, kept)| format!("t.{} AS {}", quote_ident(alias), quote_ident(kept)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let key = match spec.key {
        Some(header) => format!(
            "NULLIF(trim(s.{}), '')",
            quote_ident(&column_alias(conn, snapshot_id, header)?)
        ),
        None => "CAST(NULL AS VARCHAR)".into(),
    };
    let year_of_day = "year(occurred_on)";
    let (time_header, occurred_at, occurred_on, calendar_year, fiscal_year) = match spec.time {
        TimeColumn::RecordedAt(header) => (
            Some(header),
            "try_strptime(__time, ['%Y-%m-%d %H:%M:%S.%g', '%Y-%m-%d %H:%M:%S'])",
            "COALESCE(CAST(occurred_at AS DATE),
                CAST(try_strptime(__time, ['%Y-%m-%d', '%m/%d/%Y']) AS DATE))",
            year_of_day,
            "CAST(year(occurred_on) + CASE WHEN month(occurred_on) >= 7 THEN 1 ELSE 0 END AS INTEGER)",
        ),
        TimeColumn::FiscalYear(header) => (
            Some(header),
            "CAST(NULL AS TIMESTAMP)",
            "CAST(NULL AS DATE)",
            year_of_day,
            "CASE WHEN regexp_full_match(__time, '[0-9]{2}') THEN 2000 + CAST(__time AS INTEGER)
                  WHEN regexp_full_match(__time, '[0-9]{4}') THEN CAST(__time AS INTEGER) END",
        ),
        TimeColumn::CalendarYear(header) => (
            Some(header),
            "CAST(NULL AS TIMESTAMP)",
            "CAST(NULL AS DATE)",
            "CASE WHEN regexp_full_match(__time, '[0-9]{4}') THEN CAST(__time AS INTEGER) END",
            "NULL",
        ),
        TimeColumn::Undated => (
            None,
            "CAST(NULL AS TIMESTAMP)",
            "CAST(NULL AS DATE)",
            year_of_day,
            "NULL",
        ),
    };
    let time = match time_header {
        Some(header) => format!(
            "NULLIF(trim(s.{}), '')",
            quote_ident(&column_alias(conn, snapshot_id, header)?)
        ),
        None => "CAST(NULL AS VARCHAR)".into(),
    };
    // A keyless record is named by its row's hash and its place among the file's identical rows,
    // numbered over every stored row so the name depends on the file's bytes alone. Publishers
    // repeat their own keys (a call listed twice, an approval on two jobs), so a key that recurs in
    // the file is numbered by file order.
    Ok(format!(
        "SELECT * EXCLUDE (_source_row, __row_hash, __row_ordinal, __key, __time),
            CASE
                WHEN __key IS NULL THEN 'row:' || __row_hash || ':' || __row_ordinal
                WHEN count(*) OVER (PARTITION BY __key) = 1 THEN __key
                ELSE __key || ':' || row_number() OVER (PARTITION BY __key ORDER BY _source_row)
            END AS record_id,
            __key AS publisher_record_id,
            {source_id} AS source_id,
            {source_url} AS source_url,
            {retrieved_at} AS retrieved_at,
            {source_sha256} AS source_sha256,
            _source_row AS source_row,
            {pack_version} AS pack_version,
            {occurred_at} AS occurred_at,
            {occurred_on} AS occurred_on,
            CAST({calendar_year} AS INTEGER) AS calendar_year,
            CAST({fiscal_year} AS INTEGER) AS fiscal_year,
            CASE WHEN COALESCE(fiscal_year, calendar_year) IS NOT NULL THEN {time_basis} END
                AS time_basis
         FROM (
            SELECT {publisher_columns}, s._row_hash AS __row_hash, s.__row_ordinal, {key} AS __key,
                   {time} AS __time
            FROM {typed} t JOIN (
                SELECT *, row_number() OVER (PARTITION BY _row_hash ORDER BY _row) AS __row_ordinal
                FROM {store}
            ) s ON s._row = t._source_row
         )",
        source_id = literal(Some(snapshot_id)),
        source_url = literal(source_url.as_deref()),
        retrieved_at = literal(origin.retrieved_at.as_deref()),
        source_sha256 = literal(Some(&source_sha256)),
        pack_version = literal(origin.pack_version.as_deref()),
        time_basis = literal(time_header),
        typed = quote_ident(&typed),
        store = quote_ident(&store),
    ))
}

/// Recreates `analysis_conformed_<table>` over every live snapshot imported from a pack file of
/// `table`, or drops it when there is none, and returns the view's name if one exists. A pack
/// file's table is the second segment of its `originals/<table>/<file>` path. Each pack path
/// contributes the snapshot of its latest build, so a refreshed file replaces its earlier copy
/// rather than repeating its rows. `pack_origins` does not record retrieval, so `retrieved_at` is
/// NULL here.
pub(crate) fn rebuild_conformed_view(
    conn: &Connection,
    table: &str,
) -> Result<Option<String>, AppError> {
    let view = quote_ident(&format!("analysis_conformed_{}", spec(table)?.table));
    let members = conn
        .prepare(
            "SELECT o.snapshot_id, o.pack_version
             FROM pack_origins o JOIN analytical_snapshots s USING (snapshot_id)
             WHERE split_part(o.file_path, '/', 1) = 'originals'
                 AND split_part(o.file_path, '/', 2) = ?1
                 AND EXISTS (SELECT 1 FROM duckdb_tables() t WHERE t.schema_name = 'main'
                         AND t.table_name = s.table_name)
             QUALIFY row_number() OVER (
                 PARTITION BY o.file_path ORDER BY o.built_at DESC, o.snapshot_id) = 1
             ORDER BY o.file_path",
        )
        .map_err(storage("Conformed view members"))?
        .query_map(params![table], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(storage("Conformed view members"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage("Conformed view members"))?;
    let mut selects: Vec<String> = Vec::with_capacity(members.len());
    let mut seen = std::collections::BTreeSet::new();
    for (snapshot_id, pack_version) in members {
        if !seen.insert(snapshot_id.clone()) {
            continue;
        }
        let origin = ConformedOrigin {
            retrieved_at: None,
            pack_version: Some(pack_version),
        };
        selects.push(format!(
            "({})",
            conformed_select(conn, table, &snapshot_id, &origin)?
        ));
    }
    if selects.is_empty() {
        conn.execute_batch(&format!("DROP VIEW IF EXISTS {view}"))
            .map_err(storage("Conformed view"))?;
        return Ok(None);
    }
    // By name: each file's columns are typed and ordered as that file has them.
    conn.execute_batch(&format!(
        "CREATE OR REPLACE VIEW {view} AS {}",
        selects.join(" UNION ALL BY NAME ")
    ))
    .map_err(storage("Conformed view"))?;
    Ok(Some(format!("analysis_conformed_{table}")))
}

impl DuckDbRepository {
    /// [`rebuild_conformed_view`] on its own connection.
    pub async fn create_conformed_view(&self, table: String) -> Result<Option<String>, AppError> {
        self.query_connection("Conformed view", move |conn| {
            rebuild_conformed_view(conn, &table)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::import::PackOrigin;
    use crate::types::{AnalyticalDatasetKind, AnalyticalImportRequest};
    use serde_json::{json, Value};

    async fn repository() -> DuckDbRepository {
        let repo = DuckDbRepository::new(":memory:").unwrap();
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        repo
    }

    /// Imports `csv` as `kind` from a pack file `originals/<table>/<file_name>` built at
    /// `built_at`, and returns its snapshot id.
    async fn import(
        repo: &DuckDbRepository,
        kind: AnalyticalDatasetKind,
        table: &str,
        file_name: &str,
        built_at: &str,
        csv: &str,
    ) -> String {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(file_name);
        std::fs::write(&path, csv).unwrap();
        let file_path = format!("originals/{table}/{file_name}");
        repo.import_analytical_csv_from_pack(
            AnalyticalImportRequest {
                dataset_kind: kind,
                display_name: file_path.clone(),
                source_uri: Some(format!("https://seshat.datasd.org/{file_name}")),
                local_path: path.to_string_lossy().into_owned(),
                observation_period_start: None,
                observation_period_end: None,
                partial_dataset: false,
                worksheet: None,
                source_file_name: Some(file_name.into()),
                expected_source_sha256: None,
                derived_subset: None,
            },
            PackOrigin {
                pack: "san-diego-reports".into(),
                pack_version: built_at[..10].into(),
                built_at: built_at.into(),
                signer_key_id: "pack-test".into(),
                file_path,
                file_sha256: crate::integrity::sha256_hex(csv.as_bytes()),
                retrieved_at: built_at.into(),
            },
        )
        .await
        .unwrap()
        .0
        .snapshot_id
    }

    async fn rows(repo: &DuckDbRepository, table: &str, columns: &str) -> Vec<Vec<Value>> {
        let view = repo
            .create_conformed_view(table.into())
            .await
            .unwrap()
            .unwrap();
        repo.execute_restricted_sql(
            &format!("SELECT {columns} FROM {view} ORDER BY source_id, source_row"),
            100,
        )
        .await
        .unwrap()
        .rows
    }

    /// Imports `csv` as the only file of `table` and returns `columns` of its conformed rows.
    async fn conformed(
        kind: AnalyticalDatasetKind,
        table: &str,
        csv: &str,
        columns: &str,
    ) -> (DuckDbRepository, Vec<Vec<Value>>) {
        let repo = repository().await;
        import(
            &repo,
            kind,
            table,
            "source.csv",
            "2026-10-06T00:00:00Z",
            csv,
        )
        .await;
        let rows = rows(&repo, table, columns).await;
        (repo, rows)
    }

    async fn refusal(table: &str, csv: &str, tamper: &'static str) -> String {
        let repo = repository().await;
        import(
            &repo,
            AnalyticalDatasetKind::CallsForService,
            table,
            "source.csv",
            "2026-10-06T00:00:00Z",
            csv,
        )
        .await;
        repo.query_connection("tamper", move |conn| {
            conn.execute_batch(tamper).unwrap();
            Ok(())
        })
        .await
        .unwrap();
        repo.create_conformed_view(table.into())
            .await
            .unwrap_err()
            .to_string()
    }

    #[tokio::test]
    async fn the_fiscal_year_is_named_by_the_june_it_ends_in() {
        let (_, calls) = conformed(
            AnalyticalDatasetKind::CallsForService,
            "police_calls",
            "INCIDENT_NUM,DATE_TIME\nE1,2025-06-30 23:59:59.000\nE2,2025-07-01 00:00:00.000\n",
            "CAST(occurred_at AS VARCHAR), calendar_year, fiscal_year, time_basis",
        )
        .await;
        assert_eq!(
            calls,
            [
                [
                    json!("2025-06-30 23:59:59"),
                    json!(2025),
                    json!(2025),
                    json!("date_time")
                ],
                [
                    json!("2025-07-01 00:00:00"),
                    json!(2025),
                    json!(2026),
                    json!("date_time")
                ],
            ]
        );
        let (_, budget) = conformed(
            AnalyticalDatasetKind::OperatingBudget,
            "budget_adopted",
            "amount,report_fy,budget_cycle\n1,23,adopted\n2,2024,adopted\n",
            "occurred_on, calendar_year, fiscal_year, time_basis",
        )
        .await;
        assert_eq!(
            budget,
            [
                [json!(null), json!(null), json!(2023), json!("report_fy")],
                [json!(null), json!(null), json!(2024), json!("report_fy")],
            ]
        );
    }

    /// The City's lease file writes dates month first without padding.
    /// A calendar year names no day and no fiscal year, and is read only as four digits.
    #[tokio::test]
    async fn a_calendar_year_fills_only_the_calendar_year_and_its_basis() {
        let repo = repository().await;
        let snapshot = import(
            &repo,
            AnalyticalDatasetKind::RegionalData,
            "counts",
            "counts.csv",
            "2026-10-06T00:00:00Z",
            "year,count\n2024,5\n24,6\n,7\n",
        )
        .await;
        let spec = ConformedSpec {
            table: "counts",
            key: None,
            time: TimeColumn::CalendarYear("year"),
        };
        let rows = repo
            .query_connection("calendar year", move |conn| {
                let select = select_for(conn, spec, &snapshot, &ConformedOrigin::default())?;
                let mut statement = conn
                    .prepare(&format!(
                        "SELECT calendar_year, fiscal_year, CAST(occurred_on AS VARCHAR),
                                CAST(occurred_at AS VARCHAR), time_basis
                         FROM ({select}) ORDER BY source_row"
                    ))
                    .unwrap();
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, Option<i32>>(0)?,
                            row.get::<_, Option<i32>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ))
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                Ok(rows)
            })
            .await
            .unwrap();
        assert_eq!(
            rows,
            [
                (Some(2024), None, None, None, Some("year".to_owned())),
                (None, None, None, None, None),
                (None, None, None, None, None),
            ]
        );
    }

    #[tokio::test]
    async fn a_month_first_date_names_its_day_and_fiscal_year() {
        let (_, leases) = conformed(
            AnalyticalDatasetKind::RegionalData,
            "city_leases",
            "site_code,date_effective\nA001WR,4/18/2019\nA002,7/1/2019\n",
            "CAST(occurred_on AS VARCHAR), fiscal_year, time_basis",
        )
        .await;
        assert_eq!(
            leases,
            [
                [json!("2019-04-18"), json!(2019), json!("date_effective")],
                [json!("2019-07-01"), json!(2020), json!("date_effective")],
            ]
        );
    }

    #[tokio::test]
    async fn an_absent_time_leaves_every_time_column_and_its_basis_null() {
        let (_, rows) = conformed(
            AnalyticalDatasetKind::CallsForService,
            "police_calls",
            "INCIDENT_NUM,DATE_TIME\nE1,\n",
            "occurred_at, occurred_on, calendar_year, fiscal_year, time_basis",
        )
        .await;
        assert_eq!(rows, [vec![Value::Null; 5]]);
    }

    /// The City's times are zone-less local readings; one that carries an offset is not shifted
    /// into a local time it was not recorded as.
    #[tokio::test]
    async fn a_time_with_an_offset_is_not_read_rather_than_shifted() {
        let (_, rows) = conformed(
            AnalyticalDatasetKind::CallsForService,
            "police_calls",
            "INCIDENT_NUM,DATE_TIME\nE1,2025-01-01 00:00:00-08:00\n",
            "occurred_at, occurred_on, calendar_year, fiscal_year, time_basis",
        )
        .await;
        assert_eq!(rows, [vec![Value::Null; 5]]);
    }

    #[tokio::test]
    async fn the_record_id_is_the_publisher_key_when_one_is_recorded() {
        let (repo, rows) = conformed(
            AnalyticalDatasetKind::CallsForService,
            "police_calls",
            "INCIDENT_NUM,DATE_TIME\nE1,2025-01-01 00:00:00.000\n,2025-01-01 00:00:00.000\n\
             E2,2025-01-01 00:00:00.000\nE2,2025-01-01 00:00:01.000\n",
            "record_id, publisher_record_id",
        )
        .await;
        let stored_hash = repo
            .query_connection("row hash", |conn| {
                let store: String = conn
                    .query_row(
                        "SELECT json_extract_string(validation_summary_json, '$.rowStore')
                         FROM analytical_snapshots",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                Ok(conn
                    .query_row(
                        &format!(
                            "SELECT _row_hash FROM {} WHERE _row = 2",
                            quote_ident(&store)
                        ),
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap())
            })
            .await
            .unwrap();
        assert_eq!(
            rows,
            [
                [json!("E1"), json!("E1")],
                [json!(format!("row:{stored_hash}:1")), json!(null)],
                // The publisher repeats a key: each repeat is numbered in file order.
                [json!("E2:1"), json!("E2")],
                [json!("E2:2"), json!("E2")],
            ]
        );
    }

    #[tokio::test]
    async fn identical_keyless_rows_keep_distinct_record_ids() {
        let (_, rows) = conformed(
            AnalyticalDatasetKind::OperatingBudget,
            "budget_adopted",
            "amount,report_fy,budget_cycle\n1,23,adopted\n1,23,adopted\n",
            "record_id",
        )
        .await;
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0], rows[1]);
    }

    #[tokio::test]
    async fn the_view_covers_every_year_imported_and_drops_a_retired_one() {
        let repo = repository().await;
        let kind = AnalyticalDatasetKind::CallsForService;
        let earlier = import(
            &repo,
            kind.clone(),
            "police_calls",
            "pd_calls_for_service_2024_datasd.csv",
            "2026-10-06T00:00:00Z",
            "INCIDENT_NUM,DATE_TIME\nE24,2024-03-01 08:00:00.000\n",
        )
        .await;
        import(
            &repo,
            kind,
            "police_calls",
            "pd_calls_for_service_2025_datasd.csv",
            "2026-10-06T00:00:00Z",
            "DATE_TIME,PRIORITY,INCIDENT_NUM\n2025-03-01 08:00:00.000,2,E25\n",
        )
        .await;
        let mut both = rows(
            &repo,
            "police_calls",
            "publisher_record_id, priority, fiscal_year",
        )
        .await;
        both.sort_by_key(|row| row[0].to_string());
        assert_eq!(
            both,
            [
                [json!("E24"), json!(null), json!(2024)],
                [json!("E25"), json!("2"), json!(2025)],
            ]
        );
        repo.query_connection("retire", move |conn| {
            let typed: String = conn
                .query_row(
                    "SELECT table_name FROM analytical_snapshots WHERE snapshot_id = ?1",
                    params![earlier],
                    |row| row.get(0),
                )
                .unwrap();
            conn.execute_batch(&format!("DROP TABLE {}", quote_ident(&typed)))
                .unwrap();
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(
            rows(&repo, "police_calls", "publisher_record_id").await,
            [[json!("E25")]]
        );
    }

    #[tokio::test]
    async fn a_refreshed_pack_file_replaces_its_earlier_build() {
        let repo = repository().await;
        for (built_at, csv) in [
            (
                "2026-09-30T00:00:00Z",
                "INCIDENT_NUM,DATE_TIME\nE1,2026-01-01 00:00:00.000\n",
            ),
            (
                "2026-10-06T00:00:00Z",
                "INCIDENT_NUM,DATE_TIME\nE1,2026-01-01 00:00:00.000\nE2,2026-02-01 00:00:00.000\n",
            ),
        ] {
            import(
                &repo,
                AnalyticalDatasetKind::CallsForService,
                "police_calls",
                "pd_calls_for_service_2026_datasd.csv",
                built_at,
                csv,
            )
            .await;
        }
        assert_eq!(
            rows(&repo, "police_calls", "publisher_record_id, pack_version").await,
            [
                [json!("E1"), json!("2026-10-06")],
                [json!("E2"), json!("2026-10-06")]
            ]
        );
    }

    #[tokio::test]
    async fn a_snapshot_without_stored_original_cells_is_refused() {
        let error = refusal(
            "police_calls",
            "INCIDENT_NUM,DATE_TIME\nE1,2025-01-01 00:00:00.000\n",
            "UPDATE analytical_snapshots SET validation_summary_json = '{}'",
        )
        .await;
        assert!(error.contains("predates stored original cells"), "{error}");
    }

    /// The publisher's own `calendar_year` keeps its values under another name, and the standard
    /// `calendar_year` comes from the time column alone: empty where the time is.
    #[tokio::test]
    async fn a_publisher_column_named_like_a_standard_column_is_kept_under_another_name() {
        let (_, rows) = conformed(
            AnalyticalDatasetKind::CallsForService,
            "police_calls",
            "INCIDENT_NUM,DATE_TIME,calendar_year,calendar_year_as_published\n\
             E1,2025-01-01 00:00:00.000,1999,a\nE2,,2020,b\n",
            "calendar_year, time_basis,
             CAST(calendar_year_as_published_as_published AS VARCHAR),
             CAST(calendar_year_as_published AS VARCHAR)",
        )
        .await;
        assert_eq!(
            rows,
            [
                [json!(2025), json!("date_time"), json!("1999"), json!("a")],
                [json!(null), json!(null), json!("2020"), json!("b")],
            ]
        );
    }
}
