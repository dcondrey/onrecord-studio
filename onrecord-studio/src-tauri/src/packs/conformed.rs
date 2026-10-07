//! The conformed layer of `docs/data-packs.md`: one SELECT over an imported analytical snapshot
//! that keeps every typed publisher column and adds the standard Identity, Provenance and Time
//! columns. The pack builder copies it to Parquet; the app creates `analysis_conformed_<table>`
//! from it. Place waits for the pre-2021 council district lines.
//!
//! Standard columns are read from the snapshot's stored original cells, not its typed columns: a
//! regional table's types are inferred per file, while the original text is what the publisher
//! recorded. Absent or unreadable values are NULL, and a derived value is NULL whenever its basis
//! is.

use crate::error::AppError;
use crate::storage::DuckDbRepository;
use duckdb::{params, Connection, OptionalExt};

/// Where a conformed table's time comes from.
enum TimeColumn {
    /// A wall-clock reading. The City publishes Pacific local time with no offset, so it stays a
    /// zone-less TIMESTAMP: a value that carries an offset is not read rather than shifted.
    RecordedAt(&'static str),
    /// A City fiscal year published as its last two digits (`23` is the year ending June 2023).
    /// It names no day, so `occurred_at`, `occurred_on` and `calendar_year` stay NULL.
    FiscalYear(&'static str),
}

struct ConformedSpec {
    table: &'static str,
    /// The publisher's record key, matched to its header without regard to case.
    key: Option<&'static str>,
    time: TimeColumn,
}

/// The first slice's tables; headers as the publisher's files spell them.
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
        table: "service_reports",
        key: Some("service_request_id"),
        time: TimeColumn::RecordedAt("date_requested"),
    },
    ConformedSpec {
        table: "police_calls",
        key: Some("incident_num"),
        time: TimeColumn::RecordedAt("date_time"),
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

fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
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

fn spec(table: &str) -> Result<&'static ConformedSpec, AppError> {
    SPECS
        .iter()
        .find(|spec| spec.table == table)
        .ok_or_else(|| AppError::DataParsing(format!("'{table}' has no conformed definition")))
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

/// A SELECT of every typed column of `snapshot_id` plus the standard columns, for `table`.
pub fn conformed_select(
    conn: &Connection,
    table: &str,
    snapshot_id: &str,
    origin: &ConformedOrigin,
) -> Result<String, AppError> {
    let spec = spec(table)?;
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
    let clashes = aliases
        .query_map(params![snapshot_id], |row| row.get::<_, String>(0))
        .map_err(storage("Conformed columns"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage("Conformed columns"))?
        .into_iter()
        .filter(|alias| STANDARD_COLUMNS.contains(&alias.as_str()) || alias.starts_with("__"))
        .collect::<Vec<_>>();
    if !clashes.is_empty() {
        return Err(AppError::DataParsing(format!(
            "Snapshot {snapshot_id} has publisher columns named like standard columns: {}",
            clashes.join(", ")
        )));
    }
    let key = match spec.key {
        Some(header) => format!(
            "NULLIF(trim(s.{}), '')",
            ident(&column_alias(conn, snapshot_id, header)?)
        ),
        None => "CAST(NULL AS VARCHAR)".into(),
    };
    let (time_header, occurred_at, occurred_on, fiscal_year) = match spec.time {
        TimeColumn::RecordedAt(header) => (
            header,
            "try_strptime(__time, ['%Y-%m-%d %H:%M:%S.%g', '%Y-%m-%d %H:%M:%S'])",
            "COALESCE(CAST(occurred_at AS DATE), CAST(try_strptime(__time, '%Y-%m-%d') AS DATE))",
            "CAST(year(occurred_on) + CASE WHEN month(occurred_on) >= 7 THEN 1 ELSE 0 END AS INTEGER)",
        ),
        TimeColumn::FiscalYear(header) => (
            header,
            "CAST(NULL AS TIMESTAMP)",
            "CAST(NULL AS DATE)",
            "CASE WHEN regexp_full_match(__time, '[0-9]{2}') THEN 2000 + CAST(__time AS INTEGER) END",
        ),
    };
    let time_alias = column_alias(conn, snapshot_id, time_header)?;
    Ok(format!(
        "SELECT * EXCLUDE (_source_row, __row_hash, __key, __time),
            COALESCE(__key, 'row:' || __row_hash) AS record_id,
            __key AS publisher_record_id,
            {source_id} AS source_id,
            {source_url} AS source_url,
            {retrieved_at} AS retrieved_at,
            {source_sha256} AS source_sha256,
            _source_row AS source_row,
            {pack_version} AS pack_version,
            {occurred_at} AS occurred_at,
            {occurred_on} AS occurred_on,
            CAST(year(occurred_on) AS INTEGER) AS calendar_year,
            CAST({fiscal_year} AS INTEGER) AS fiscal_year,
            CASE WHEN fiscal_year IS NOT NULL THEN {time_basis} END AS time_basis
         FROM (
            SELECT t.*, s._row_hash AS __row_hash, {key} AS __key,
                   NULLIF(trim(s.{time}), '') AS __time
            FROM {typed} t JOIN {store} s ON s._row = t._source_row
         )",
        source_id = literal(Some(snapshot_id)),
        source_url = literal(source_url.as_deref()),
        retrieved_at = literal(origin.retrieved_at.as_deref()),
        source_sha256 = literal(Some(&source_sha256)),
        pack_version = literal(origin.pack_version.as_deref()),
        time_basis = literal(Some(time_header)),
        time = ident(&time_alias),
        typed = ident(&typed),
        store = ident(&store),
    ))
}

impl DuckDbRepository {
    /// Creates or replaces `analysis_conformed_<table>` over the imported snapshot
    /// `snapshot_id`, and returns the view's name.
    pub async fn create_conformed_view(
        &self,
        table: String,
        snapshot_id: String,
        origin: ConformedOrigin,
    ) -> Result<String, AppError> {
        self.query_connection("Conformed view", move |conn| {
            let select = conformed_select(conn, &table, &snapshot_id, &origin)?;
            let view = format!("analysis_conformed_{}", spec(&table)?.table);
            conn.execute_batch(&format!(
                "CREATE OR REPLACE VIEW {} AS {select}",
                ident(&view)
            ))
            .map_err(storage("Conformed view"))?;
            Ok(view)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AnalyticalDatasetKind, AnalyticalImportRequest};
    use serde_json::{json, Value};

    /// Imports `csv` as `kind` through the analytical import, creates the conformed view of
    /// `table`, and returns `columns` of it in file order.
    async fn conformed(
        kind: AnalyticalDatasetKind,
        table: &str,
        csv: &str,
        columns: &str,
    ) -> (DuckDbRepository, Vec<Vec<Value>>) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.csv");
        std::fs::write(&path, csv).unwrap();
        let repo = DuckDbRepository::new(":memory:").unwrap();
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let imported = repo
            .import_analytical_csv(AnalyticalImportRequest {
                dataset_kind: kind,
                display_name: "fixture".into(),
                source_uri: Some("https://seshat.datasd.org/fixture.csv".into()),
                local_path: path.to_string_lossy().into_owned(),
                observation_period_start: None,
                observation_period_end: None,
                partial_dataset: false,
                worksheet: None,
                source_file_name: None,
                expected_source_sha256: None,
                derived_subset: None,
            })
            .await
            .unwrap();
        let view = repo
            .create_conformed_view(
                table.into(),
                imported.snapshot_id,
                ConformedOrigin::default(),
            )
            .await
            .unwrap();
        let rows = repo
            .execute_restricted_sql(
                &format!("SELECT {columns} FROM {view} ORDER BY source_row"),
                100,
            )
            .await
            .unwrap()
            .rows;
        (repo, rows)
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
            "amount,report_fy,budget_cycle\n1,23,adopted\n",
            "occurred_on, calendar_year, fiscal_year, time_basis",
        )
        .await;
        assert_eq!(
            budget,
            [[json!(null), json!(null), json!(2023), json!("report_fy")]]
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

    #[tokio::test]
    async fn the_record_id_is_the_publisher_key_when_one_is_recorded() {
        let (repo, rows) = conformed(
            AnalyticalDatasetKind::CallsForService,
            "police_calls",
            "INCIDENT_NUM,DATE_TIME\nE1,2025-01-01 00:00:00.000\n,2025-01-01 00:00:00.000\n",
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
                        &format!("SELECT _row_hash FROM {} WHERE _row = 2", ident(&store)),
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
                [json!(format!("row:{stored_hash}")), json!(null)],
            ]
        );
    }
}
