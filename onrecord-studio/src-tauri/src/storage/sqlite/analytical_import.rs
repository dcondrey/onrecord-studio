//! Turns a table file into an analytical snapshot in two reads of the source. The first profiles
//! every column outside the database lock; the second converts and writes rows inside one
//! transaction. Neither read keeps more than one record of delimited text in memory, and the two
//! must agree on the source's SHA-256 or nothing is committed.
//!
//! A row is rejected only when the file itself makes it unreadable: the parser failed, or its
//! field count differs from the header's (beyond empty trailing fields). A cell that does not
//! convert to its column's type is stored as NULL in the typed table, keeps its original text in
//! the row ledger, and is counted per column. Counting it is what keeps a NULL from reading as an
//! empty source cell.

use super::analytical::{
    analytical_counting_key, duckdb_type, json_to_sql_value, logical_type_for, normalize_csv_value,
    quote_ident, schema_warnings_for, slash_date_fields, unique_column_alias, validation_notes_for,
    DateOrder, ANALYTICAL_PARSER_VERSION,
};
use super::*;
use crate::storage::tabular::{TableSource, TypeEvidence};
use crate::types::{AnalyticalImportRequest, AnalyticalImportResult};
use std::collections::{BTreeMap, HashSet};

/// What the first read learned about one column.
#[derive(Default)]
struct ColumnProfile {
    evidence: TypeEvidence,
    non_empty: usize,
    slash_dates: usize,
    first_over_12: bool,
    second_over_12: bool,
    /// Values the column's documented type converts, under each [`DateOrder::ALL`].
    converts: [usize; 3],
}

#[derive(Debug, Clone)]
struct ColumnPlan {
    header: String,
    alias: String,
    logical: &'static str,
    order: DateOrder,
}

fn is_text(logical: &str) -> bool {
    matches!(logical, "CATEGORY" | "IDENTIFIER")
}

/// Regional tables are typed only by what every value allows. A documented dataset keeps the
/// type its publisher's column implies unless no value converts to it, which means the hint is
/// wrong for this file rather than that every value is bad.
fn plan_columns(
    kind: &AnalyticalDatasetKind,
    headers: &[String],
    profiles: &[ColumnProfile],
    warnings: &mut Vec<String>,
) -> Vec<ColumnPlan> {
    let mut aliases = std::collections::BTreeSet::new();
    headers
        .iter()
        .zip(profiles)
        .enumerate()
        .map(|(index, (header, profile))| {
            let base = unique_column_alias(header, index, headers);
            let mut alias = base.clone();
            let mut suffix = 2;
            while !aliases.insert(alias.clone()) {
                alias = format!("{base}_{suffix}");
                suffix += 1;
            }
            let order = DateOrder::decided_by(profile.first_over_12, profile.second_over_12);
            let documented = *kind != AnalyticalDatasetKind::RegionalData;
            let mut logical = if documented {
                logical_type_for(kind, header)
            } else {
                profile.evidence.inferred(header)
            };
            // An inferred type already fits every value; only a documented one can be wrong.
            if documented && !is_text(logical) {
                let converted = profile.converts[DateOrder::ALL
                    .iter()
                    .position(|candidate| *candidate == order)
                    .unwrap_or(2)];
                if profile.non_empty > 0 && converted == 0 {
                    warnings.push(format!(
                        "{header}: none of its {} values is a {}; kept as text.",
                        profile.non_empty,
                        logical.to_ascii_lowercase().replace('_', " ")
                    ));
                    logical = "CATEGORY";
                } else if matches!(logical, "DATE" | "TIMESTAMP")
                    && profile.slash_dates > 0
                    && order == DateOrder::Undetermined
                {
                    warnings.push(format!(
                        "{header}: {} n/n/yyyy dates left unconverted because their day and month order cannot be determined from the column.",
                        profile.slash_dates
                    ));
                }
            }
            ColumnPlan {
                header: header.clone(),
                alias,
                logical,
                order,
            }
        })
        .collect()
}

fn read_failed(error: impl std::fmt::Display) -> AppError {
    AppError::Storage(format!("Failed to write analytical rows: {error}"))
}

/// A 128-bit digest of a counting key: duplicate detection then holds 16 bytes per row however
/// long the key, and a false match needs a SHA-256 prefix collision.
fn key_digest(key: &str) -> u128 {
    let digest = <sha2::Sha256 as sha2::Digest>::digest(key.as_bytes());
    u128::from_be_bytes(
        digest[..16]
            .try_into()
            .expect("a SHA-256 digest has 32 bytes"),
    )
}

impl DuckDbRepository {
    pub(crate) async fn import_analytical_table(
        &self,
        request: AnalyticalImportRequest,
        automatic_origin: Option<(String, u64)>,
    ) -> Result<AnalyticalImportResult, AppError> {
        let kind = request.dataset_kind.clone();
        let format_name = request
            .source_file_name
            .clone()
            .unwrap_or_else(|| request.local_path.clone());
        let (path, worksheet) = (request.local_path.clone(), request.worksheet.clone());
        let profile_kind = kind.clone();
        let (source, first_read, profiles) = crate::blocking::spawn("Profile table", move || {
            let source = TableSource::open(
                std::path::Path::new(&path),
                &format_name,
                worksheet.as_deref(),
            )?;
            let hints_for = |headers: &[String]| -> Vec<Option<&'static str>> {
                headers
                    .iter()
                    .map(|header| {
                        Some(logical_type_for(&profile_kind, header)).filter(|logical| {
                            profile_kind != AnalyticalDatasetKind::RegionalData && !is_text(logical)
                        })
                    })
                    .collect()
            };
            let mut hints = Vec::new();
            let mut profiles: Vec<ColumnProfile> = Vec::new();
            let read = source.read(|headers, _, record| {
                if hints.is_empty() {
                    hints = hints_for(headers);
                    profiles = headers.iter().map(|_| ColumnProfile::default()).collect();
                }
                let Ok(record) = record else { return Ok(()) };
                for (index, profile) in profiles.iter_mut().enumerate() {
                    let raw = record.get(index).unwrap_or("").trim();
                    profile.evidence.observe(raw);
                    if raw.is_empty() {
                        continue;
                    }
                    profile.non_empty += 1;
                    if let Some((first, second)) = slash_date_fields(raw) {
                        profile.slash_dates += 1;
                        profile.first_over_12 |= first > 12;
                        profile.second_over_12 |= second > 12;
                    }
                    if let Some(hint) = hints[index] {
                        for (slot, order) in DateOrder::ALL.iter().enumerate() {
                            if normalize_csv_value(raw, hint, *order).is_ok() {
                                profile.converts[slot] += 1;
                            }
                        }
                    }
                }
                Ok(())
            })?;
            // A header-only table never reached the callback.
            profiles.resize_with(read.headers.len(), ColumnProfile::default);
            Ok((source, read, profiles))
        })
        .await??;
        if request
            .expected_source_sha256
            .as_ref()
            .is_some_and(|expected| !expected.eq_ignore_ascii_case(&first_read.sha256))
        {
            return Err(AppError::DataParsing(
                "The acquired file no longer matches its recorded SHA-256; import stopped.".into(),
            ));
        }
        let source_sha256 = first_read.sha256.clone();
        // Equal bytes from different publishers or worksheets are distinct acquisitions.
        let identity_hash = sha256_hex(
            serde_json::json!([
                ANALYTICAL_PARSER_VERSION,
                source_sha256,
                request.source_uri,
                request.worksheet,
                request.observation_period_start,
                request.observation_period_end,
                request.partial_dataset
            ])
            .to_string()
            .as_bytes(),
        );
        let snapshot_id = format!(
            "{}-{}",
            kind.as_str().to_ascii_lowercase().replace('_', "-"),
            &identity_hash[..24]
        );
        let table_name = format!(
            "analysis_{}_{}",
            kind.as_str().to_ascii_lowercase(),
            &identity_hash[..24]
        );
        let latest_view_name = if kind == AnalyticalDatasetKind::RegionalData {
            let source_identity = request.source_uri.as_deref().unwrap_or(&request.local_path);
            let key = sha256_hex(
                serde_json::json!([source_identity, request.worksheet])
                    .to_string()
                    .as_bytes(),
            );
            format!("analysis_regional_{}_latest", &key[..16])
        } else {
            format!("analysis_{}_latest", kind.as_str().to_ascii_lowercase())
        };
        let headers = first_read.headers;
        let mut schema_warnings = schema_warnings_for(&headers, &kind);
        schema_warnings.extend(source.warnings().iter().cloned());
        if headers
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != headers.len()
        {
            schema_warnings.push("Duplicate column headers were assigned distinct SQL names; raw rows are retained by column position.".into());
        }
        let plans = plan_columns(&kind, &headers, &profiles, &mut schema_warnings);
        let declares_partial = source.declares_partial();
        if declares_partial {
            schema_warnings.push(
                "The source declares additional pages or worksheets; this snapshot is partial."
                    .into(),
            );
        }
        let partial = request.partial_dataset || declares_partial;
        let expected_rows = first_read.rows;
        let request_for_tx = request.clone();
        let (snapshot, table, view) = (
            snapshot_id.clone(),
            table_name.clone(),
            latest_view_name.clone(),
        );
        let outcome = self
            .query_connection("Analytical table import", move |conn| {
                write_snapshot(
                    conn,
                    &source,
                    &plans,
                    &headers,
                    WriteTarget {
                        snapshot_id: &snapshot,
                        table_name: &table,
                        latest_view: &view,
                        source_sha256: &source_sha256,
                        expected_rows,
                        partial,
                        format: source.format(),
                        declares_partial,
                        warnings: schema_warnings,
                        request: &request_for_tx,
                        origin: automatic_origin.as_ref(),
                    },
                )
            })
            .await?;
        Ok(AnalyticalImportResult {
            snapshot_id,
            dataset_kind: kind,
            display_name: request.display_name,
            table_name,
            imported_rows: outcome.imported_rows,
            rejected_rows: outcome.malformed_rows,
            malformed_rows: outcome.malformed_rows,
            duplicate_key_rows: outcome.duplicate_key_rows,
            unconverted_cells: outcome.unconverted_cells,
            schema_warnings: outcome.warnings,
            source_sha256: first_read.sha256,
        })
    }
}

struct WriteTarget<'a> {
    snapshot_id: &'a str,
    table_name: &'a str,
    latest_view: &'a str,
    source_sha256: &'a str,
    expected_rows: usize,
    partial: bool,
    format: &'static str,
    declares_partial: bool,
    warnings: Vec<String>,
    request: &'a AnalyticalImportRequest,
    origin: Option<&'a (String, u64)>,
}

struct WriteOutcome {
    imported_rows: usize,
    malformed_rows: usize,
    duplicate_key_rows: usize,
    unconverted_cells: usize,
    warnings: Vec<String>,
}

/// The second read. The snapshot row is written first so the row ledger's foreign key holds while
/// rows are appended; its counts and summary are filled in once the read has finished.
fn write_snapshot(
    conn: &mut Connection,
    source: &TableSource,
    plans: &[ColumnPlan],
    first_headers: &[String],
    target: WriteTarget<'_>,
) -> Result<WriteOutcome, AppError> {
    let tx = conn.transaction().map_err(|error| {
        AppError::Storage(format!("Failed to begin analytical import: {error}"))
    })?;
    let table = quote_ident(target.table_name);
    let create_columns = plans
        .iter()
        .map(|plan| format!("{} {}", quote_ident(&plan.alias), duckdb_type(plan.logical)))
        .collect::<Vec<_>>()
        .join(", ");
    tx.execute_batch(&format!(
        "DROP TABLE IF EXISTS {table};
         CREATE TABLE {table} ({create_columns});"
    ))
    .map_err(|error| AppError::Storage(format!("Failed to create analytical table: {error}")))?;
    let request = target.request;
    for statement in [
        "DELETE FROM analytical_rows WHERE snapshot_id = ?1",
        "DELETE FROM analytical_columns WHERE snapshot_id = ?1",
        "DELETE FROM analytical_snapshots WHERE snapshot_id = ?1",
    ] {
        tx.execute(statement, params![target.snapshot_id])
            .map_err(|error| AppError::Storage(format!("Failed to replace snapshot: {error}")))?;
    }
    tx.execute(
        "INSERT INTO analytical_snapshots (
            snapshot_id, dataset_kind, display_name, table_name, source_uri, local_path,
            source_sha256, retrieved_at, observation_period_start, observation_period_end,
            partial_dataset, validation_summary_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7,
            strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'), ?8, ?9, ?10, '{}')",
        params![
            target.snapshot_id,
            request.dataset_kind.as_str(),
            request.display_name,
            target.table_name,
            request.source_uri,
            request.local_path,
            target.source_sha256,
            request.observation_period_start,
            request.observation_period_end,
            i64::from(target.partial),
        ],
    )
    .map_err(|error| AppError::Storage(format!("Failed to record analytical snapshot: {error}")))?;
    for (ordinal, plan) in plans.iter().enumerate() {
        tx.execute(
            "INSERT INTO analytical_columns (
                snapshot_id, table_name, column_name, original_name, logical_type,
                nullable, dictionary, ordinal
             ) VALUES (?1, ?2, ?3, ?4, ?5, 1, NULL, ?6)",
            params![
                target.snapshot_id,
                target.table_name,
                plan.alias,
                plan.header,
                plan.logical,
                ordinal as i64
            ],
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to record analytical column: {error}"))
        })?;
    }
    let mut typed = tx.appender(target.table_name).map_err(read_failed)?;
    let mut ledger = tx.appender("analytical_rows").map_err(read_failed)?;
    let mut malformed_rows = 0usize;
    let mut repaired_trailing_rows = 0usize;
    let mut repair_row_numbers = Vec::new();
    let mut duplicate_key_rows = 0usize;
    let mut seen_keys = HashSet::new();
    let mut null_counts = vec![0usize; plans.len()];
    let mut unconverted = vec![0usize; plans.len()];
    let second_read = source.read(|_, row_index, record| {
        let row_number = row_index + 1;
        let (original, values, errors, notes) = match record {
            Err(error) => (
                serde_json::Value::Null,
                Vec::new(),
                vec![format!("CSV row parse failed: {error}")],
                Vec::new(),
            ),
            Ok(record) => {
                // Position retains duplicate headers and surplus fields in malformed rows.
                let original = serde_json::json!(record.iter().collect::<Vec<_>>());
                let mut values = Vec::with_capacity(plans.len());
                let mut notes = Vec::new();
                for (index, plan) in plans.iter().enumerate() {
                    let raw = record.get(index).unwrap_or("").trim();
                    match normalize_csv_value(raw, plan.logical, plan.order) {
                        Ok(value) => {
                            if value.is_null() {
                                null_counts[index] += 1;
                            }
                            values.push(value);
                        }
                        Err(error) => {
                            unconverted[index] += 1;
                            notes.push(format!("{}: {error}", plan.header));
                            values.push(serde_json::Value::Null);
                        }
                    }
                }
                let mut errors = Vec::new();
                if record.len() > plans.len()
                    && record
                        .iter()
                        .skip(plans.len())
                        .all(|cell| cell.trim().is_empty())
                {
                    repaired_trailing_rows += 1;
                    if repair_row_numbers.len() < 1000 {
                        repair_row_numbers.push(row_number);
                    }
                } else if record.len() != plans.len() {
                    errors.push(format!(
                        "Expected {} fields, found {}",
                        plans.len(),
                        record.len()
                    ));
                }
                if let Some(key) =
                    analytical_counting_key(&target.request.dataset_kind, first_headers, record)
                {
                    if !seen_keys.insert(key_digest(&key)) {
                        duplicate_key_rows += 1;
                    }
                }
                (original, values, errors, notes)
            }
        };
        let malformed = !errors.is_empty();
        if malformed {
            malformed_rows += 1;
        } else {
            typed
                .append_row(duckdb::appender_params_from_iter(
                    values.iter().map(json_to_sql_value),
                ))
                .map_err(read_failed)?;
        }
        let normalized = serde_json::Value::Object(
            plans
                .iter()
                .zip(values)
                .map(|(plan, value)| (plan.alias.clone(), value))
                .collect(),
        );
        let payload = if original.is_null() {
            original.to_string()
        } else {
            serde_json::json!({"headers": first_headers, "values": original}).to_string()
        };
        let normalized_payload = normalized.to_string();
        let messages = if malformed { errors } else { notes };
        let messages_json = serde_json::Value::Array(
            messages
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        )
        .to_string();
        let row_hash = sha256_hex(format!("{payload}\n{normalized_payload}").as_bytes());
        ledger
            .append_row(duckdb::params![
                target.snapshot_id,
                row_number as i64,
                row_hash,
                payload,
                normalized_payload,
                i64::from(malformed),
                messages_json,
            ])
            .map_err(read_failed)
    })?;
    typed.flush().map_err(read_failed)?;
    ledger.flush().map_err(read_failed)?;
    drop((typed, ledger));
    if second_read.sha256 != target.source_sha256
        || second_read.rows != target.expected_rows
        || second_read.headers != first_headers
    {
        return Err(AppError::DataParsing(
            "The source changed between its two reads; import stopped and nothing was stored."
                .into(),
        ));
    }
    let rows = second_read.rows;
    let unconverted_cells = unconverted.iter().sum::<usize>();
    let mut warnings = target.warnings;
    if repaired_trailing_rows > 0 {
        warnings.push(format!("Format repair: ignored only empty surplus trailing fields in {repaired_trailing_rows} rows; all original field values remain in the row ledger."));
    }
    if unconverted_cells > 0 {
        warnings.push(format!("{unconverted_cells} cells did not convert to their column's type; they are NULL in the table and kept as written in the row ledger."));
    }
    let validation_summary = serde_json::json!({
        "malformedRows": malformed_rows,
        "duplicateKeyRows": duplicate_key_rows,
        "rejectedRows": malformed_rows,
        "unconvertedCells": unconverted_cells,
        "unconvertedByColumn": plans.iter().zip(&unconverted).filter(|(_, n)| **n > 0).map(|(plan, n)| (plan.alias.clone(), *n)).collect::<BTreeMap<_, _>>(),
        "dateOrder": plans.iter().filter(|plan| matches!(plan.logical, "DATE" | "TIMESTAMP")).map(|plan| (plan.alias.clone(), plan.order.as_str())).collect::<BTreeMap<_, _>>(),
        "repairs": { "emptyTrailingFields": { "rowCount": repaired_trailing_rows, "rowNumbers": repair_row_numbers, "rowNumbersTruncated": repaired_trailing_rows > 1000 } },
        "parserVersion": ANALYTICAL_PARSER_VERSION,
        "sourceFormat": target.format,
        "sourceDeclaresPartial": target.declares_partial,
        "worksheet": request.worksheet,
        "warnings": warnings,
        "nullCounts": plans.iter().zip(&null_counts).map(|(plan, n)| (plan.alias.clone(), *n)).collect::<BTreeMap<_, _>>(),
        "notes": validation_notes_for(&request.dataset_kind),
        "derivedSubset": request.derived_subset.as_ref().map(|subset| serde_json::json!({
            "publisherSha256": subset.publisher_sha256,
            "publisherRows": subset.publisher_rows,
            "filter": subset.filter,
            "note": "sourceSha256 is the digest of this filtered subset, not of the publisher's file.",
        })),
    });
    let imported_rows = rows - malformed_rows;
    tx.execute(
        &format!(
            "CREATE OR REPLACE VIEW {} AS SELECT * FROM {table}",
            quote_ident(target.latest_view)
        ),
        [],
    )
    .map_err(|error| {
        AppError::Storage(format!("Failed to refresh latest analytical view: {error}"))
    })?;
    tx.execute(
        "UPDATE analytical_snapshots SET imported_rows = ?2, rejected_rows = ?3, malformed_rows = ?3,
            duplicate_key_rows = ?4, validation_summary_json = ?5 WHERE snapshot_id = ?1",
        params![
            target.snapshot_id,
            imported_rows as i64,
            malformed_rows as i64,
            duplicate_key_rows as i64,
            validation_summary.to_string(),
        ],
    )
    .map_err(|error| AppError::Storage(format!("Failed to record analytical snapshot: {error}")))?;
    if let Some((portal_key, document_id)) = target.origin {
        tx.execute(
            "INSERT INTO data_maintenance_origins VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
            params![target.snapshot_id, portal_key, document_id],
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to record the automatic-import origin receipt: {error}"
            ))
        })?;
    }
    tx.commit().map_err(|error| {
        AppError::Storage(format!("Failed to commit analytical import: {error}"))
    })?;
    Ok(WriteOutcome {
        imported_rows,
        malformed_rows,
        duplicate_key_rows,
        unconverted_cells,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repository() -> DuckDbRepository {
        let repo = DuckDbRepository::new(":memory:").unwrap();
        repo.run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        repo
    }

    fn request(path: &std::path::Path, kind: AnalyticalDatasetKind) -> AnalyticalImportRequest {
        AnalyticalImportRequest {
            dataset_kind: kind,
            display_name: "fixture".into(),
            source_uri: None,
            local_path: path.to_string_lossy().into_owned(),
            observation_period_start: None,
            observation_period_end: None,
            partial_dataset: false,
            worksheet: None,
            source_file_name: None,
            expected_source_sha256: None,
            derived_subset: None,
        }
    }

    async fn query(repo: &DuckDbRepository, sql: &str) -> Vec<Vec<serde_json::Value>> {
        repo.execute_restricted_sql(sql, 100).await.unwrap().rows
    }

    #[tokio::test]
    async fn a_cell_that_does_not_convert_keeps_its_row_and_its_text() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nibrs.csv");
        std::fs::write(
            &path,
            "nibrs_uniq,case_number,ibr_offense,occured_on\n1,10,90Z,2025-01-02\n2,11,90Z,unknown\n",
        )
        .unwrap();
        let repo = repository().await;
        let result = repo
            .import_analytical_csv(request(&path, AnalyticalDatasetKind::NibrsOffenses))
            .await
            .unwrap();
        assert_eq!(
            (
                result.imported_rows,
                result.rejected_rows,
                result.unconverted_cells
            ),
            (2, 0, 1)
        );
        let rows = query(
            &repo,
            "SELECT CAST(occured_on AS VARCHAR) FROM analysis_nibrs_offenses_latest ORDER BY nibrs_uniq",
        )
        .await;
        assert_eq!(
            rows,
            vec![
                vec![serde_json::json!("2025-01-02")],
                vec![serde_json::Value::Null]
            ]
        );
        let ledger = query(
            &repo,
            &format!("SELECT malformed, source_payload_json, validation_errors_json FROM analytical_rows WHERE snapshot_id = '{}' AND row_number = 2", result.snapshot_id),
        )
        .await;
        assert_eq!(ledger[0][0], serde_json::json!(0));
        assert!(ledger[0][1].as_str().unwrap().contains("\"unknown\""));
        assert!(ledger[0][2].as_str().unwrap().contains("occured_on"));
        let schema = repo.analytical_schema().await.unwrap();
        assert_eq!(schema.datasets[0].unconverted_in(["occured_on"]), 1);
        assert_eq!(schema.datasets[0].unconverted_in(["case_number"]), 0);
    }

    #[tokio::test]
    async fn only_an_unreadable_row_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rows.csv");
        std::fs::write(&path, "a,b\n1,2\n3\n4,5,,\n6,7,8\n").unwrap();
        let repo = repository().await;
        let result = repo
            .import_analytical_csv(request(&path, AnalyticalDatasetKind::RegionalData))
            .await
            .unwrap();
        // Row 3 has empty surplus fields, which are repaired; rows 2 and 4 have the wrong width.
        assert_eq!((result.imported_rows, result.rejected_rows), (2, 2));
        let schema = repo.analytical_schema().await.unwrap();
        assert!(
            schema.columns.iter().all(|c| c.logical_type == "INTEGER"),
            "{:?}",
            schema.columns
        );
    }

    #[tokio::test]
    async fn a_documented_type_no_value_fits_is_kept_as_text() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("calls.csv");
        std::fs::write(
            &path,
            "INCIDENT_NUM,DATE_TIME,CALL_TYPE\nE1,Monday,X\nE2,Tuesday,Y\n",
        )
        .unwrap();
        let repo = repository().await;
        let result = repo
            .import_analytical_csv(request(&path, AnalyticalDatasetKind::CallsForService))
            .await
            .unwrap();
        assert_eq!((result.rejected_rows, result.unconverted_cells), (0, 0));
        assert!(result
            .schema_warnings
            .iter()
            .any(|w| w.starts_with("DATE_TIME: none of its 2")));
        let schema = repo.analytical_schema().await.unwrap();
        let column = schema
            .columns
            .iter()
            .find(|c| c.column_name == "date_time")
            .unwrap();
        assert_eq!(column.logical_type, "CATEGORY");
    }

    #[tokio::test]
    async fn slash_dates_convert_only_when_the_column_settles_day_and_month() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dates.csv");
        std::fs::write(
            &path,
            "nibrs_uniq,day_first_date,month_first_date,unknown_date\n1,25/12/2025,12/25/2025,01/02/2025\n2,01/02/2025,01/02/2025,03/04/2025\n3,,,2025-05-06\n",
        )
        .unwrap();
        let repo = repository().await;
        let result = repo
            .import_analytical_csv(request(&path, AnalyticalDatasetKind::NibrsOffenses))
            .await
            .unwrap();
        let rows = query(
            &repo,
            "SELECT CAST(day_first_date AS VARCHAR), CAST(month_first_date AS VARCHAR), CAST(unknown_date AS VARCHAR) FROM analysis_nibrs_offenses_latest ORDER BY nibrs_uniq",
        )
        .await;
        let null = serde_json::Value::Null;
        assert_eq!(
            rows,
            vec![
                vec![
                    serde_json::json!("2025-12-25"),
                    serde_json::json!("2025-12-25"),
                    null.clone()
                ],
                vec![
                    serde_json::json!("2025-02-01"),
                    serde_json::json!("2025-01-02"),
                    null.clone()
                ],
                vec![null.clone(), null, serde_json::json!("2025-05-06")],
            ]
        );
        assert_eq!(result.unconverted_cells, 2);
        assert!(result
            .schema_warnings
            .iter()
            .any(|w| w.starts_with("unknown_date: 2 n/n/yyyy dates")));
    }

    #[tokio::test]
    async fn nothing_is_stored_when_the_two_reads_disagree() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rows.csv");
        std::fs::write(&path, "a,b\n1,2\n").unwrap();
        let source = TableSource::open(&path, "rows.csv", None).unwrap();
        let repo = repository().await;
        let request = request(&path, AnalyticalDatasetKind::RegionalData);
        let plans = vec![
            ColumnPlan {
                header: "a".into(),
                alias: "a".into(),
                logical: "INTEGER",
                order: DateOrder::Undetermined,
            },
            ColumnPlan {
                header: "b".into(),
                alias: "b".into(),
                logical: "INTEGER",
                order: DateOrder::Undetermined,
            },
        ];
        let error = repo
            .query_connection("test", move |conn| {
                write_snapshot(
                    conn,
                    &source,
                    &plans,
                    &["a".into(), "b".into()],
                    WriteTarget {
                        snapshot_id: "s",
                        table_name: "analysis_regional_data_s",
                        latest_view: "analysis_regional_s_latest",
                        source_sha256: &"0".repeat(64),
                        expected_rows: 1,
                        partial: false,
                        format: "CSV",
                        declares_partial: false,
                        warnings: Vec::new(),
                        request: &request,
                        origin: None,
                    },
                )
                .map(|_| ())
            })
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("changed between its two reads"),
            "{error}"
        );
        let left = repo
            .query_connection("test", |conn| {
                conn.query_row(
                    "SELECT (SELECT COUNT(*) FROM analytical_snapshots) + (SELECT COUNT(*) FROM analytical_rows) + (SELECT COUNT(*) FROM information_schema.tables WHERE table_name LIKE 'analysis_regional%' OR table_name LIKE 'import_ledger%')",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| AppError::Storage(error.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(left, 0);
    }
}
