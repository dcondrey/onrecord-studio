//! Analytical CSV imports and the restricted SQL surface over them.

use super::*;

impl DuckDbRepository {
    pub async fn encampment_ordinance_report(
        &self,
    ) -> Result<crate::types::EncampmentOrdinanceReport, AppError> {
        self.query_connection("Encampment ordinance observations", |conn| {
            crate::analysis::encampment_ordinance::report(conn)
        })
        .await
    }

    pub async fn import_analytical_csv(
        &self,
        request: AnalyticalImportRequest,
    ) -> Result<AnalyticalImportResult, AppError> {
        self.import_analytical_table(request, None)
            .await
            .map(|(result, _)| result)
    }

    /// As [`Self::import_analytical_csv`], and also records the `data_maintenance_origins`
    /// receipt in the same transaction as the snapshot write. The two have to commit or roll back
    /// together: a crash between two separate transactions would leave a snapshot on disk with
    /// no origin receipt, which is exactly what later maintenance and purge logic uses to tell an
    /// automatic import from an intentionally partial manual one.
    pub(crate) async fn import_analytical_csv_recording_origin(
        &self,
        request: AnalyticalImportRequest,
        automatic_origin: Option<(String, u64)>,
    ) -> Result<AnalyticalImportResult, AppError> {
        let origin = automatic_origin.map(|(portal_key, document_id)| {
            super::analytical_import::ImportOrigin::Maintenance {
                portal_key,
                document_id,
            }
        });
        self.import_analytical_table(request, origin)
            .await
            .map(|(result, _)| result)
    }

    /// As [`Self::import_analytical_csv`], recording the data pack the file came from beside the
    /// snapshot in the same transaction. Also returns whether the snapshot was already stored.
    pub(crate) async fn import_analytical_csv_from_pack(
        &self,
        request: AnalyticalImportRequest,
        origin: crate::packs::import::PackOrigin,
    ) -> Result<(AnalyticalImportResult, bool), AppError> {
        self.import_analytical_table(
            request,
            Some(super::analytical_import::ImportOrigin::Pack(origin)),
        )
        .await
    }

    pub async fn analytical_import_candidates(
        &self,
    ) -> Result<Vec<crate::types::AnalyticalImportCandidate>, AppError> {
        self.query_connection("Acquired analytical tables", move |conn| {
            let mut statement = conn.prepare("SELECT id, portal_subdomain, COALESCE(file_name, 'Dataset'), local_path, download_url, raw_file_sha256
                FROM documents WHERE local_path IS NOT NULL AND download_status = 'completed'
                AND LOWER(TRIM(visibility)) = 'public'
                AND (regexp_matches(lower(COALESCE(file_name, '')), '\\.(csv|tsv|json|jsonl|ndjson|geojson|xlsx|xls|ods)$')
                OR content_type IN ('text/csv', 'text/tab-separated-values', 'application/json', 'application/geo+json'))
                ORDER BY updated_at DESC, portal_subdomain, id LIMIT 500")
                .map_err(|e| AppError::Storage(format!("Failed to list acquired tables: {e}")))?;
            let rows = statement.query_map([], |row| Ok(crate::types::AnalyticalImportCandidate {
                document_id: row.get::<_, i64>(0)? as u64, portal_key: row.get(1)?, file_name: row.get(2)?,
                local_path: row.get(3)?, source_uri: row.get(4)?, source_sha256: row.get(5)?,
            })).map_err(|e| AppError::Storage(format!("Failed to query acquired tables: {e}")))?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|e| AppError::Storage(format!("Failed to decode acquired tables: {e}")))
        }).await
    }

    /// The distinct non-empty values of `column` across every imported copy of `source_uri`.
    pub async fn imported_column_values(
        &self,
        source_uri: String,
        column: String,
    ) -> Result<std::collections::HashSet<String>, AppError> {
        self.query_connection("Imported column values", move |conn| {
            let storage = |error: duckdb::Error| {
                AppError::Storage(format!("Failed to read imported column values: {error}"))
            };
            let tables = conn
                .prepare(
                    "SELECT s.table_name FROM analytical_snapshots s
                     JOIN analytical_columns c ON c.snapshot_id = s.snapshot_id
                     WHERE s.source_uri = ?1 AND c.column_name = ?2",
                )
                .map_err(storage)?
                .query_map(params![source_uri, column], |row| row.get::<_, String>(0))
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            let mut values = std::collections::HashSet::new();
            for table in tables {
                let column = quote_ident(&column);
                let mut statement = conn
                    .prepare(&format!(
                        "SELECT DISTINCT TRIM(CAST({column} AS VARCHAR)) FROM {} WHERE NULLIF(TRIM(CAST({column} AS VARCHAR)), '') IS NOT NULL",
                        quote_ident(&table)
                    ))
                    .map_err(storage)?;
                for value in statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(storage)?
                {
                    values.insert(value.map_err(storage)?);
                }
            }
            Ok(values)
        })
        .await
    }

    pub async fn analytical_schema(&self) -> Result<AnalyticalSchema, AppError> {
        self.query_connection("Analytical schema", move |conn| {
            let mut datasets_stmt = conn.prepare(
                "SELECT snapshot_id, dataset_kind, display_name, table_name, source_uri,
                        source_sha256, retrieved_at, observation_period_start,
                        observation_period_end, partial_dataset, imported_rows,
                        rejected_rows, validation_summary_json
                 FROM analytical_snapshots
                 ORDER BY retrieved_at DESC"
            ).map_err(|error| AppError::Storage(format!("Failed to prepare analytical datasets: {error}")))?;
            let datasets = datasets_stmt.query_map([], |row| {
                Ok(AnalyticalDatasetSummary {
                    snapshot_id: row.get(0)?,
                    dataset_kind: dataset_kind_from_str(&row.get::<_, String>(1)?),
                    display_name: row.get(2)?,
                    table_name: row.get(3)?,
                    source_uri: row.get(4)?,
                    source_sha256: row.get(5)?,
                    retrieved_at: row.get(6)?,
                    observation_period_start: row.get(7)?,
                    observation_period_end: row.get(8)?,
                    partial_dataset: row.get::<_, i64>(9)? != 0,
                    imported_rows: row.get::<_, i64>(10)?.max(0) as usize,
                    rejected_rows: row.get::<_, i64>(11)?.max(0) as usize,
                    unconverted_by_column: unconverted_by_column(&row.get::<_, String>(12)?),
                    validation_summary: row.get(12)?,
                })
            }).map_err(|error| AppError::Storage(format!("Failed to query analytical datasets: {error}")))?
              .collect::<Result<Vec<_>, _>>()
              .map_err(|error| AppError::Storage(format!("Failed to decode analytical dataset: {error}")))?;
            let mut columns_stmt = conn.prepare(
                "SELECT table_name, column_name, logical_type, original_name, nullable, dictionary
                 FROM analytical_columns
                 ORDER BY table_name, ordinal"
            ).map_err(|error| AppError::Storage(format!("Failed to prepare analytical columns: {error}")))?;
            let columns = columns_stmt.query_map([], |row| {
                Ok(AnalyticalColumnSummary {
                    table_name: row.get(0)?,
                    column_name: row.get(1)?,
                    logical_type: row.get(2)?,
                    original_name: row.get(3)?,
                    nullable: row.get::<_, i64>(4)? != 0,
                    dictionary: row.get(5)?,
                })
            }).map_err(|error| AppError::Storage(format!("Failed to query analytical columns: {error}")))?
              .collect::<Result<Vec<_>, _>>()
              .map_err(|error| AppError::Storage(format!("Failed to decode analytical column: {error}")))?;
            let mut templates = sql_templates();
            for dataset in &datasets {
                let dataset_columns = columns.iter().filter(|column| column.table_name == dataset.table_name).collect::<Vec<_>>();
                if dataset_columns.iter().any(|c| c.column_name == "report_fy") && !dataset_columns.iter().any(|c| c.column_name == "budget_cycle") {
                    templates.push(SqlTemplate {
                        template_id: format!("{}-fiscal-year-coverage", dataset.snapshot_id),
                        title: format!("Fiscal-year coverage: {}", dataset.display_name),
                        description: "Inspect the fiscal-year codes actually represented in this snapshot. A recent publication date does not establish recent observations.".into(),
                        sql: format!("SELECT report_fy, COUNT(*) AS imported_rows FROM {} GROUP BY 1 ORDER BY 1", quote_ident(&dataset.table_name)),
                        denominator: "Rows in the selected snapshot, grouped by its own fiscal-year code.".into(),
                    });
                }
                if ["report_fy", "budget_cycle", "amount"].iter().all(|name| dataset_columns.iter().any(|c| c.column_name == *name))
                    && dataset_columns.iter().any(|c| c.column_name == "amount" && matches!(c.logical_type.as_str(), "NUMBER" | "INTEGER" | "DECIMAL")) {
                    templates.push(SqlTemplate {
                        template_id: format!("{}-budget-coverage", dataset.snapshot_id),
                        title: format!("Budget year and cycle coverage: {}", dataset.display_name),
                        description: "Inspect fiscal years, reported budget cycles, and missing amounts before selecting appropriations. Blank cycles remain unspecified.".into(),
                        sql: format!("SELECT report_fy, COALESCE(NULLIF(LOWER(TRIM(budget_cycle)), ''), 'UNSPECIFIED') AS reported_cycle, COUNT(*) AS allocation_rows, COUNT(*) - COUNT(amount) AS missing_amounts FROM {} GROUP BY 1, 2 ORDER BY 1, 2", quote_ident(&dataset.table_name)),
                        denominator: "Imported budget rows by the publisher's fiscal-year and cycle fields. Missing status is not assumed to be adopted.".into(),
                    });
                }
                let profile_sql = columns.iter().filter(|column| column.table_name == dataset.table_name).take(64).map(|column| {
                    let name = quote_ident(&column.column_name);
                    let label = column.column_name.replace('\'', "''");
                    format!("SELECT '{label}' AS column_name, COUNT(*) AS rows, COUNT(*) - COUNT({name}) AS missing, COUNT(DISTINCT {name}) AS distinct_values, CAST(MIN({name}) AS VARCHAR) AS minimum, CAST(MAX({name}) AS VARCHAR) AS maximum FROM {}", quote_ident(&dataset.table_name))
                }).collect::<Vec<_>>().join(" UNION ALL ");
                templates.push(SqlTemplate {
                    template_id: format!("{}-profile", dataset.snapshot_id),
                    title: format!("Profile {}", dataset.display_name),
                    description: "Missing values, distinct values, and minimum/maximum for the first 64 columns; text bounds are lexical.".into(),
                    sql: profile_sql,
                    denominator: "Imported rows in this snapshot; rejected rows and partial coverage remain in the snapshot ledger.".into(),
                });
                templates.push(SqlTemplate {
                    template_id: format!("{}-count", dataset.snapshot_id),
                    title: format!("Count {}", dataset.display_name),
                    description: "Count imported rows without assuming that rows represent unique people, events, or properties.".into(),
                    sql: format!("SELECT COUNT(*) AS imported_rows FROM {}", quote_ident(&dataset.table_name)),
                    denominator: "One row of the selected snapshot.".into(),
                });
                if dataset.rejected_rows > 0 {
                    templates.push(SqlTemplate {
                        template_id: format!("{}-rejected", dataset.snapshot_id),
                        title: format!("Review rejected rows: {}", dataset.display_name),
                        description: "Inspect original field positions and validation errors for rows excluded from SQL counts.".into(),
                        sql: format!("SELECT row_number, source_payload_json, validation_errors_json FROM analytical_rows WHERE snapshot_id = '{}' AND malformed = 1 ORDER BY row_number", dataset.snapshot_id.replace('\'', "''")),
                        denominator: "Rejected rows in the selected snapshot.".into(),
                    });
                }
            }
            Ok(AnalyticalSchema { datasets, columns, templates })
        }).await
    }

    pub async fn execute_restricted_sql(
        &self,
        sql: &str,
        limit: usize,
    ) -> Result<SqlQueryResult, AppError> {
        if !(1..=10_000).contains(&limit) {
            return Err(AppError::DataParsing(
                "SQL row limit must be between 1 and 10000".to_string(),
            ));
        }
        let sql = sql.trim().trim_end_matches(';').to_string();
        let started = std::time::Instant::now();
        self.query_connection("Restricted analytical SQL", move |conn| {
            let _deadline = crate::analysis::sql::QueryDeadline::start(
                conn,
                std::time::Duration::from_secs(30),
            )?;
            let tables = {
                let mut statement = conn
                    .prepare("SELECT table_name FROM information_schema.tables")
                    .map_err(|e| AppError::Storage(e.to_string()))?;
                let names = statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|e| AppError::Storage(e.to_string()))?;
                names
                    .collect::<Result<std::collections::BTreeSet<_>, _>>()
                    .map_err(|e| AppError::Storage(e.to_string()))?
            };
            let referenced = crate::analysis::sql::validate(&sql, &tables)?;
            conn.execute_batch(
                "SET enable_external_access = false; SET enable_progress_bar = false;",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to restrict SQL engine: {error}"))
            })?;
            let wrapped = format!(
                "SELECT * FROM ({sql}) AS onrecord_user_query LIMIT {}",
                limit + 1
            );
            let mut statement = conn.prepare(&wrapped).map_err(|error| {
                AppError::DataParsing(format!("SQL could not be prepared: {error}"))
            })?;
            // DuckDB exposes result columns after execution; asking a merely prepared statement
            // for them panics, unlike SQLite. Empty result sets still carry the executed schema.
            let mut rows = statement.query([]).map_err(|error| {
                AppError::DataParsing(format!(
                    "SQL execution failed (analytical work has a 30-second deadline): {error}"
                ))
            })?;
            let column_names = rows
                .as_ref()
                .expect("executed query retains its statement")
                .column_names();
            let column_count = column_names.len();
            let mut output = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|error| AppError::Storage(format!("Failed to read SQL row: {error}")))?
            {
                let values = (0..column_count)
                    .map(|index| duckdb_value_to_json(row, index))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to decode SQL row: {error}"))
                    })?;
                output.push(values);
            }
            let truncated = output.len() > limit;
            output.truncate(limit);
            Ok(SqlQueryResult {
                columns: column_names,
                row_count: output.len(),
                rows: output,
                truncated,
                elapsed_ms: started.elapsed().as_millis(),
                provenance: analytical_query_provenance(conn, &referenced)?,
            })
        })
        .await
    }

    pub async fn export_restricted_sql_csv(
        &self,
        sql: &str,
        output_path: &str,
        limit: usize,
    ) -> Result<SqlExportResult, AppError> {
        let result = self.execute_restricted_sql(sql, limit).await?;
        // A symlink at the target path would redirect this write to a location the caller never
        // actually chose — the real frontend only ever populates this path from a native Save
        // dialog, but the IPC boundary itself enforces nothing, so a compromised or scripted
        // caller could point it at a symlink planted somewhere reachable to overwrite an
        // arbitrary file (e.g. a shell profile or LaunchAgents entry) through an indirection the
        // user never saw. Overwriting an existing *regular* file the user re-selected is a
        // legitimate re-export and is intentionally still allowed. The IPC command already runs
        // this same check before the request reaches here; it is repeated at the point of the
        // actual write, through the one shared implementation, rather than trusted from upstream.
        crate::commands::validation::validate_filesystem_target_path(
            output_path,
            "Export path",
            crate::commands::validation::MAXIMUM_PATH_BYTES,
        )?;
        let mut writer = csv::Writer::from_path(output_path)
            .map_err(|error| AppError::Storage(format!("Failed to create CSV export: {error}")))?;
        writer
            .write_record(&result.columns)
            .map_err(|error| AppError::Storage(format!("Failed to write CSV header: {error}")))?;
        for row in &result.rows {
            writer
                .write_record(row.iter().map(json_cell_to_csv))
                .map_err(|error| AppError::Storage(format!("Failed to write CSV row: {error}")))?;
        }
        writer
            .flush()
            .map_err(|error| AppError::Storage(format!("Failed to flush CSV export: {error}")))?;
        let digest = crate::integrity::sha256_file(std::path::Path::new(output_path))
            .map_err(|error| AppError::Storage(format!("Failed to hash CSV export: {error}")))?;
        Ok(SqlExportResult {
            output_path: output_path.to_string(),
            row_count: result.row_count,
            sha256: digest,
        })
    }
}

pub(crate) fn unconverted_by_column(
    validation_summary: &str,
) -> std::collections::BTreeMap<String, usize> {
    serde_json::from_str::<serde_json::Value>(validation_summary)
        .ok()
        .and_then(|summary| summary.get("unconvertedByColumn").cloned())
        .and_then(|counts| serde_json::from_value(counts).ok())
        .unwrap_or_default()
}

pub(crate) fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub(super) fn unique_column_alias(header: &str, index: usize, headers: &[String]) -> String {
    let mut alias = header
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if alias.is_empty() || alias.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        alias = format!("field_{}", index + 1);
    }
    let duplicates_before = headers[..index]
        .iter()
        .filter(|prior| {
            prior
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() {
                        ch.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
                .trim_matches('_')
                == alias
        })
        .count();
    if duplicates_before > 0 {
        format!("{alias}_{}", duplicates_before + 1)
    } else {
        alias
    }
}

pub(crate) const ANALYTICAL_PARSER_VERSION: &str = "regional-tables-v4";

pub(super) fn logical_type_for(kind: &AnalyticalDatasetKind, header: &str) -> &'static str {
    let header = header.to_ascii_lowercase();
    if matches!(kind, AnalyticalDatasetKind::OperatingBudget) {
        return match header.as_str() {
            "amount" => "NUMBER",
            "report_fy" | "budget_cycle" => "CATEGORY",
            name if name.ends_with("_number") => "IDENTIFIER",
            _ => "CATEGORY",
        };
    }
    if matches!(
        header.as_str(),
        "latitude" | "longitude" | "lat" | "lon" | "lng"
    ) {
        return "FLOAT";
    }
    // CFS `DATE_TIME` is a Pacific wall-clock reading with no zone; stored zone-less so it is
    // never asserted to be UTC.
    if header.contains("date") && header.contains("time") {
        return "LOCAL_TIMESTAMP";
    }
    // NIBRS `approved_on` carries a wall-clock time: kept as recorded text, since a DATE refuses
    // every published row and a TIMESTAMP would stamp Pacific time as UTC.
    if header.contains("date") || header == "occured_on" {
        return "DATE";
    }
    if header.contains("time") {
        return "TIMESTAMP";
    }
    if header.contains("amount") || header.contains("budget") {
        return "DECIMAL";
    }
    if matches!(
        header.as_str(),
        "uid" | "id" | "pid" | "incident_num" | "case_number" | "nibrs_uniq"
    ) {
        return "IDENTIFIER";
    }
    if matches!(kind, AnalyticalDatasetKind::OperatingBudget) && header.ends_with("_number") {
        return "IDENTIFIER";
    }
    "CATEGORY"
}

pub(super) fn duckdb_type(logical: &str) -> &'static str {
    match logical {
        "INTEGER" => "BIGINT",
        "NUMBER" => "DECIMAL(38,10)",
        "FLOAT" => "DOUBLE",
        "DATE" => "DATE",
        "TIMESTAMP" => "TIMESTAMP",
        "LOCAL_TIMESTAMP" => "TIMESTAMP",
        "DECIMAL" => "DECIMAL(18,2)",
        _ => "TEXT",
    }
}

/// Which field of an `n/n/yyyy` date is the month, decided per column from its own values. When
/// no field ever exceeds 12, or both do, the order is not decidable and slash dates stay
/// unconverted: guessing would put a real date on the wrong day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DateOrder {
    MonthFirst,
    DayFirst,
    Undetermined,
}

impl DateOrder {
    pub(super) const ALL: [Self; 3] = [Self::MonthFirst, Self::DayFirst, Self::Undetermined];

    pub(super) fn decided_by(first_over_12: bool, second_over_12: bool) -> Self {
        match (first_over_12, second_over_12) {
            (true, false) => Self::DayFirst,
            (false, true) => Self::MonthFirst,
            _ => Self::Undetermined,
        }
    }

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::MonthFirst => "MONTH_FIRST",
            Self::DayFirst => "DAY_FIRST",
            Self::Undetermined => "UNDETERMINED",
        }
    }
}

/// A cell converted to its column's type. Text borrows the source cell where it can.
#[derive(Debug, PartialEq)]
pub(super) enum Normalized<'a> {
    Null,
    Integer(i64),
    Float(f64),
    Text(std::borrow::Cow<'a, str>),
}

pub(super) fn normalize_csv_value<'a>(
    raw: &'a str,
    logical: &str,
    order: DateOrder,
) -> Result<Normalized<'a>, String> {
    use std::borrow::Cow;
    if raw.is_empty() {
        return Ok(Normalized::Null);
    }
    let finite = |value: f64| Some(value).filter(|value| value.is_finite());
    match logical {
        "INTEGER" => raw
            .parse::<i64>()
            .map(Normalized::Integer)
            .map_err(|_| format!("could not parse integer '{raw}'")),
        // Bind decimal strings directly, avoiding a binary floating-point round trip.
        "NUMBER" if crate::storage::tabular::exact_decimal(raw) => {
            Ok(Normalized::Text(Cow::Borrowed(raw)))
        }
        "NUMBER" => Err(format!("could not parse exact decimal '{raw}'")),
        // Approximate by declaration: the original text stays in the snapshot's column store.
        "FLOAT" if crate::storage::tabular::plain_decimal(raw) => raw
            .parse::<f64>()
            .ok()
            .and_then(finite)
            .map(Normalized::Float)
            .ok_or_else(|| format!("could not parse decimal '{raw}'")),
        "FLOAT" => Err(format!("could not parse decimal '{raw}'")),
        "DATE" => normalize_date(raw, order)
            .map(|date| Normalized::Text(Cow::Owned(date)))
            .ok_or_else(|| format!("could not parse date '{raw}'")),
        "LOCAL_TIMESTAMP" => normalize_local_timestamp(raw)
            .map(|at| Normalized::Text(Cow::Owned(at)))
            .ok_or_else(|| format!("could not parse date and time '{raw}'")),
        "TIMESTAMP" => normalize_timestamp(raw, order)
            .map(|at| Normalized::Text(Cow::Owned(at)))
            .ok_or_else(|| format!("could not parse timestamp '{raw}'")),
        "DECIMAL" => raw
            .replace(',', "")
            .parse::<f64>()
            .ok()
            .and_then(finite)
            .map(Normalized::Float)
            .ok_or_else(|| format!("could not parse amount '{raw}'")),
        _ => Ok(Normalized::Text(Cow::Borrowed(raw))),
    }
}

fn normalize_date(raw: &str, order: DateOrder) -> Option<String> {
    let slash: &[&str] = match order {
        DateOrder::MonthFirst => &["%m/%d/%Y", "%-m/%-d/%Y"],
        DateOrder::DayFirst => &["%d/%m/%Y", "%-d/%-m/%Y"],
        DateOrder::Undetermined => &[],
    };
    std::iter::once("%Y-%m-%d")
        .chain(slash.iter().copied())
        .find_map(|format| chrono::NaiveDate::parse_from_str(raw, format).ok())
        .map(|date| date.format("%Y-%m-%d").to_string())
}

/// The two leading numeric fields of a `n/n/yyyy` date, when `raw` is one.
pub(super) fn slash_date_fields(raw: &str) -> Option<(u32, u32)> {
    let mut parts = raw.split('/');
    let (first, second, year) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || year.len() != 4
        || !year.bytes().all(|b| b.is_ascii_digit())
        || !(1..=2).contains(&first.len())
        || !(1..=2).contains(&second.len())
    {
        return None;
    }
    Some((first.parse().ok()?, second.parse().ok()?))
}

fn normalize_local_timestamp(raw: &str) -> Option<String> {
    [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
    ]
    .iter()
    .find_map(|format| chrono::NaiveDateTime::parse_from_str(raw.trim(), format).ok())
    .map(|at| at.format("%Y-%m-%d %H:%M:%S%.f").to_string())
}

/// A value that states its offset keeps it. One that does not is a wall-clock reading and is kept
/// without a zone: appending `Z` would assert UTC for a time the publisher never placed in UTC.
fn normalize_timestamp(raw: &str, order: DateOrder) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|date| date.to_rfc3339())
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
                .map(|date| date.format("%Y-%m-%dT%H:%M:%S").to_string())
                .ok()
        })
        .or_else(|| normalize_date(raw, order).map(|date| format!("{date}T00:00:00")))
}

fn dataset_kind_from_str(value: &str) -> AnalyticalDatasetKind {
    match value {
        "REGIONAL_DATA" => AnalyticalDatasetKind::RegionalData,
        "RIPA_STOPS" => AnalyticalDatasetKind::RipaStops,
        "RIPA_SEARCH_BASIS" => AnalyticalDatasetKind::RipaSearchBasis,
        "CALLS_FOR_SERVICE" => AnalyticalDatasetKind::CallsForService,
        "CALLS_CODEBOOK" => AnalyticalDatasetKind::CallsCodebook,
        "NIBRS_OFFENSES" => AnalyticalDatasetKind::NibrsOffenses,
        "OPERATING_BUDGET" => AnalyticalDatasetKind::OperatingBudget,
        _ => AnalyticalDatasetKind::BudgetDimension,
    }
}

pub(super) fn analytical_counting_key(
    kind: &AnalyticalDatasetKind,
    headers: &[String],
    record: &csv::StringRecord,
) -> Option<String> {
    let key_fields: &[&str] = match kind {
        AnalyticalDatasetKind::RipaStops => &["UID", "ID", "PID"],
        AnalyticalDatasetKind::CallsForService => &["INCIDENT_NUM"],
        AnalyticalDatasetKind::NibrsOffenses => &["nibrs_uniq"],
        AnalyticalDatasetKind::OperatingBudget => &[
            "report_fy",
            "budget_cycle",
            "funds_center_number",
            "account_number",
        ],
        _ => return None,
    };
    let mut parts = Vec::new();
    for key in key_fields {
        let index = headers
            .iter()
            .position(|header| header.eq_ignore_ascii_case(key))?;
        parts.push(record.get(index).unwrap_or("").to_string());
    }
    Some(parts.join("\u{1f}"))
}

pub(super) fn validation_notes_for(kind: &AnalyticalDatasetKind) -> Vec<&'static str> {
    match kind {
        AnalyticalDatasetKind::RipaStops => vec![
            "RIPA UID/ID values can repeat across current and historical collections.",
            "PID identifies a person within one stop, not a persistent person across encounters.",
        ],
        AnalyticalDatasetKind::RipaSearchBasis => vec![
            "Companion tables can multiply rows; aggregate to the intended counting unit before joining.",
        ],
        AnalyticalDatasetKind::CallsForService => vec![
            "Public calls-for-service data excludes publisher-documented sensitive categories.",
            "Calls are not crimes, cases, offenses, or stopped-person records.",
        ],
        AnalyticalDatasetKind::NibrsOffenses => vec![
            "Rows are offenses; distinct cases require an explicit case-number grouping rule.",
        ],
        AnalyticalDatasetKind::OperatingBudget => vec![
            "Budget fiscal year and budget cycle must both be retained.",
            "Budget allocations are not actual expenditures.",
        ],
        _ => vec!["Preserve source dictionary and table family before joining."],
    }
}

pub(super) fn schema_warnings_for(headers: &[String], kind: &AnalyticalDatasetKind) -> Vec<String> {
    let required: &[&str] = match kind {
        AnalyticalDatasetKind::RipaStops => &["UID", "ID", "PID", "STOPDATE"],
        AnalyticalDatasetKind::CallsForService => &["INCIDENT_NUM", "DATE_TIME", "CALL_TYPE"],
        AnalyticalDatasetKind::NibrsOffenses => &["nibrs_uniq", "case_number", "ibr_offense"],
        AnalyticalDatasetKind::OperatingBudget => &["amount", "report_fy", "budget_cycle"],
        _ => &[],
    };
    required
        .iter()
        .filter(|required| {
            !headers
                .iter()
                .any(|header| header.eq_ignore_ascii_case(required))
        })
        .map(|required| format!("Expected column '{required}' was not present"))
        .collect()
}

fn sql_templates() -> Vec<SqlTemplate> {
    vec![
        SqlTemplate {
            template_id: "ripa-stop-search-rate".to_string(),
            title: "RIPA stops and searches".to_string(),
            description: "Counts stopped-person records and records with a search flag when that column exists.".to_string(),
            denominator: "Distinct stopped-person records within the selected RIPA snapshot.".to_string(),
            sql: "SELECT stopdate, COUNT(*) AS stopped_person_rows, COUNT(*) FILTER (WHERE lower(coalesce(searchbasis, '')) <> '') AS rows_with_search_basis FROM analysis_ripa_stops_latest GROUP BY stopdate ORDER BY stopdate".to_string(),
        },
        SqlTemplate {
            template_id: "calls-by-category-period".to_string(),
            title: "Calls by category and period".to_string(),
            description: "Counts public calls for service by call type and month.".to_string(),
            denominator: "Published public CFS rows; excluded categories remain outside the denominator.".to_string(),
            sql: "SELECT date_trunc('month', date_time) AS month, call_type, COUNT(*) AS calls FROM analysis_calls_for_service_latest GROUP BY 1, 2 ORDER BY 1, 2".to_string(),
        },
        SqlTemplate {
            template_id: "nibrs-offenses-vs-cases".to_string(),
            title: "NIBRS offenses versus cases".to_string(),
            description: "Compares offense rows to distinct case numbers.".to_string(),
            denominator: "Offense rows and distinct case_number values are different units.".to_string(),
            sql: "SELECT year, COUNT(*) AS offense_rows, COUNT(DISTINCT case_number) AS distinct_cases FROM analysis_nibrs_offenses_latest GROUP BY year ORDER BY year".to_string(),
        },
        SqlTemplate {
            template_id: "budget-cycle-comparison".to_string(),
            title: "Budget comparison by cycle".to_string(),
            description: "Compare adopted allocations by fiscal year, department, fund, and revenue/expense account class.".to_string(),
            denominator: "Budget allocation rows for the selected fiscal year and budget cycle, not actual expenditures.".to_string(),
            sql: "SELECT report_fy, LOWER(TRIM(budget_cycle)) AS budget_cycle, dept_name, fund_number, CASE WHEN account_number LIKE '4%' THEN 'Revenue' WHEN account_number LIKE '5%' THEN 'Expense' ELSE 'Other' END AS account_class, SUM(amount) AS allocated_amount FROM analysis_operating_budget_latest WHERE LOWER(TRIM(budget_cycle)) = 'adopted' GROUP BY 1, 2, 3, 4, 5 ORDER BY report_fy, allocated_amount DESC".to_string(),
        },
    ]
}

fn duckdb_value_to_json(row: &duckdb::Row<'_>, index: usize) -> duckdb::Result<serde_json::Value> {
    use duckdb::types::ValueRef;
    Ok(match row.get_ref(index)? {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Boolean(value) => serde_json::Value::Bool(value),
        ValueRef::TinyInt(value) => serde_json::json!(value),
        ValueRef::SmallInt(value) => serde_json::json!(value),
        ValueRef::Int(value) => serde_json::json!(value),
        ValueRef::BigInt(value)
            if !(-9_007_199_254_740_991..=9_007_199_254_740_991).contains(&value) =>
        {
            serde_json::json!(value.to_string())
        }
        ValueRef::BigInt(value) => serde_json::json!(value),
        ValueRef::HugeInt(value) => serde_json::json!(value.to_string()),
        ValueRef::UHugeInt(value) => serde_json::json!(value.to_string()),
        ValueRef::UTinyInt(value) => serde_json::json!(value),
        ValueRef::USmallInt(value) => serde_json::json!(value),
        ValueRef::UInt(value) => serde_json::json!(value),
        ValueRef::UBigInt(value) if value > 9_007_199_254_740_991 => {
            serde_json::json!(value.to_string())
        }
        ValueRef::UBigInt(value) => serde_json::json!(value),
        ValueRef::Date32(days) => serde_json::json!(chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
            .and_then(|epoch| epoch.checked_add_signed(chrono::Duration::days(i64::from(days))))
            .map(|date| date.to_string())),
        ValueRef::Float(value) => serde_json::json!(value),
        ValueRef::Double(value) => serde_json::json!(value),
        ValueRef::Decimal(value) => serde_json::json!(value.to_string()),
        ValueRef::Text(value) => {
            serde_json::Value::String(String::from_utf8_lossy(value).to_string())
        }
        ValueRef::Blob(value) | ValueRef::Geometry(value) => {
            serde_json::Value::String(format!("[binary:{} bytes]", value.len()))
        }
        other => serde_json::Value::String(format!("{other:?}")),
    })
}

fn approved_analysis_tables(conn: &Connection) -> Result<Vec<String>, AppError> {
    let mut statement = conn.prepare("SELECT table_name || '@' || snapshot_id FROM analytical_snapshots ORDER BY retrieved_at DESC")
        .map_err(|error| AppError::Storage(format!("Failed to prepare provenance table list: {error}")))?;
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| {
            AppError::Storage(format!("Failed to query provenance table list: {error}"))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            AppError::Storage(format!("Failed to read provenance table list: {error}"))
        })
}

pub(crate) fn analytical_query_provenance(
    conn: &Connection,
    referenced: &std::collections::BTreeSet<String>,
) -> Result<Vec<String>, AppError> {
    let mut dependencies = referenced.clone();
    let mut statement = conn
        .prepare("SELECT view_name, sql FROM duckdb_views() WHERE schema_name = 'main'")
        .map_err(|e| AppError::Storage(e.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|e| AppError::Storage(e.to_string()))?;
    for row in rows {
        let (view, sql) = row.map_err(|e| AppError::Storage(e.to_string()))?;
        if referenced.contains(&view) {
            // Application-created latest views contain a direct SELECT of one immutable table.
            // Resolve that actual target rather than attributing every historical snapshot.
            if let Some(sql) = sql {
                dependencies.extend(
                    sql.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                        .filter(|word| word.starts_with("analysis_"))
                        .map(str::to_string),
                );
            }
        }
    }
    Ok(approved_analysis_tables(conn)?
        .into_iter()
        .filter(|entry| {
            entry
                .split_once('@')
                .is_some_and(|(table, _)| dependencies.contains(table))
        })
        .collect())
}

fn json_cell_to_csv(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(value) => sanitize_csv_string_cell(value),
        other => other.to_string(),
    }
}

/// Neutralizes spreadsheet formula/DDE injection: a cell value beginning with `=`, `+`, `-`, `@`,
/// tab, or carriage return is interpreted as a formula by Excel/Sheets/LibreOffice, not as literal
/// text, when the CSV is opened by a human reviewer. This application treats exported cell content
/// as externally supplied evidence (portal-sourced request titles/descriptions, imported analytical
/// data), not as internally generated text, so a crafted record could otherwise execute a formula
/// or DDE command in the reviewer's spreadsheet application. A leading apostrophe forces text
/// interpretation in every mainstream spreadsheet application and is invisible in the rendered
/// cell.
///
/// A plain signed decimal is exempt. It cannot carry a formula, and DECIMAL, HUGEINT, and BIGINT
/// beyond 2^53 reach this function as strings (see `duckdb_value_to_json`), so quoting a negative
/// one's leading `-` would corrupt a financial figure into a text cell.
fn sanitize_csv_string_cell(value: &str) -> String {
    if value.starts_with(['=', '+', '-', '@', '\t', '\r']) && !is_plain_decimal(value) {
        format!("'{value}")
    } else {
        value.to_string()
    }
}

fn is_plain_decimal(value: &str) -> bool {
    let unsigned = value.strip_prefix(['-', '+']).unwrap_or(value);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    !whole.is_empty()
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod semantic_analysis_tests {
    use super::*;

    #[test]
    fn ripa_key_includes_person_within_stop() {
        let headers = vec!["UID".to_string(), "ID".to_string(), "PID".to_string()];
        let one = csv::StringRecord::from(vec!["1", "STOP-A", "1"]);
        let two = csv::StringRecord::from(vec!["1", "STOP-A", "2"]);
        assert_ne!(
            analytical_counting_key(&AnalyticalDatasetKind::RipaStops, &headers, &one),
            analytical_counting_key(&AnalyticalDatasetKind::RipaStops, &headers, &two)
        );
    }

    #[test]
    fn budget_cycle_template_keeps_cycle_filter_explicit() {
        let template = sql_templates()
            .into_iter()
            .find(|template| template.template_id == "budget-cycle-comparison")
            .unwrap();
        assert!(template.sql.contains("budget_cycle"));
        assert!(template.denominator.contains("budget cycle"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Header and two rows as published in pd_calls_for_service_2025_datasd.csv.
    const CALLS: &str = "\"INCIDENT_NUM\",\"DATE_TIME\",\"DAY_OF_WEEK\",\"ADDRESS_NUMBER_PRIMARY\",\"ADDRESS_DIR_PRIMARY\",\"ADDRESS_ROAD_PRIMARY\",\"ADDRESS_SFX_PRIMARY\",\"ADDRESS_DIR_INTERSECTING\",\"ADDRESS_ROAD_INTERSECTING\",\"ADDRESS_SFX_INTERSECTING\",\"CALL_TYPE\",\"DISPOSITION\",\"BEAT\",\"PRIORITY\"
\"E25070031255\",\"2025-07-21 15:38:39.000\",2,\"0\",,\"HAZARD CENTER\",\"DR\",,\"FRAZEE\",,\"1183\",\"R\",\"315\",\"0\"
\"E25070031348\",\"2025-07-21 16:53:49.000\",2,\"0\",,\"PARK\",\"BLV\",,\"BROADWAY\",,\"FOOT\",\"O\",\"521\",\"3\"
";

    #[tokio::test]
    async fn published_calls_import_whole_with_their_recorded_wall_clock_time() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join("pd_calls_for_service_2025_datasd.csv");
        std::fs::write(&path, CALLS).unwrap();
        let repo = DuckDbRepository::new(":memory:").unwrap();
        repo.run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let result = repo
            .import_analytical_csv(AnalyticalImportRequest {
                dataset_kind: AnalyticalDatasetKind::CallsForService,
                display_name: "calls".into(),
                source_uri: None,
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
        assert_eq!((result.imported_rows, result.rejected_rows), (2, 0));
        let times = repo
            .execute_restricted_sql(
                "SELECT CAST(date_time AS VARCHAR) AS t, CAST(date_trunc('month', date_time) AS VARCHAR) AS m FROM analysis_calls_for_service_latest ORDER BY t",
                10,
            )
            .await
            .unwrap();
        let text = serde_json::to_string(&times.rows).unwrap();
        assert!(text.contains("2025-07-21 15:38:39"), "{text}");
        assert!(text.contains("2025-07-01"), "{text}");
    }
}
