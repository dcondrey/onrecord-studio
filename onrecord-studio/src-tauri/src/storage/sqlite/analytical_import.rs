//! Turns a table file into an analytical snapshot with one read of the source. That read hashes
//! the bytes, stores every record's original fields in the snapshot's column store and profiles
//! each column; the typed table is then built from the stored originals. So the stored cells come
//! from exactly the bytes that were hashed, and no format is parsed twice. Everything happens in
//! one transaction.
//!
//! A row is rejected only when the file itself makes it unreadable: the parser failed, or its
//! field count differs from the header's (beyond empty trailing fields). A cell that does not
//! convert to its column's type is stored as NULL in the typed table, keeps its original text in
//! the row ledger, and is counted per column. Counting it is what keeps a NULL from reading as an
//! empty source cell.

use super::analytical::{
    analytical_counting_key, duckdb_type, logical_type_for, normalize_csv_value, quote_ident,
    schema_warnings_for, slash_date_fields, unique_column_alias, validation_notes_for, DateOrder,
    ANALYTICAL_PARSER_VERSION,
};
use super::row_store;
use super::*;
use crate::storage::tabular::{TableSource, TypeEvidence};
use crate::types::{AnalyticalImportRequest, AnalyticalImportResult};
use duckdb::arrow::array::{
    Array, ArrayBuilder, ArrayRef, AsArray, BooleanBuilder, Float64Builder, Int32Builder,
    Int64Builder, StringBuilder,
};
use duckdb::arrow::datatypes::{Field, Int64Type, Schema};
use duckdb::arrow::record_batch::RecordBatch;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

/// What the read learned about one column.
#[derive(Default)]
struct ColumnProfile {
    evidence: TypeEvidence,
    non_empty: usize,
    slash_dates: usize,
    first_over_12: bool,
    second_over_12: bool,
    /// Whether any value converts to the column's documented type, under each [`DateOrder::ALL`].
    /// One converting value settles a slot, so later values are not converted again.
    converts: [bool; 3],
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
    aliases: Vec<String>,
    profiles: &[ColumnProfile],
    warnings: &mut Vec<String>,
) -> Vec<ColumnPlan> {
    headers
        .iter()
        .zip(aliases)
        .zip(profiles)
        .map(|((header, alias), profile)| {
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
                if profile.non_empty > 0 && !converted {
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

/// Converted values are built from the stored originals this many rows at a time.
const CONVERT_BATCH_ROWS: i64 = 50_000;

fn storage(context: &str) -> impl Fn(duckdb::Error) -> AppError + '_ {
    move |error| AppError::Storage(format!("{context}: {error}"))
}

/// What the read stored and measured, before column types are known.
struct ReadOutcome {
    headers: Vec<String>,
    aliases: Vec<String>,
    profiles: Vec<ColumnProfile>,
    sha256: String,
    rows: usize,
    malformed_rows: usize,
    duplicate_key_rows: usize,
    repaired_trailing_rows: usize,
    repair_row_numbers: Vec<usize>,
}

fn column_aliases(headers: &[String]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    headers
        .iter()
        .enumerate()
        .map(|(index, header)| {
            let base = unique_column_alias(header, index, headers);
            let mut alias = base.clone();
            let mut suffix = 2;
            while !seen.insert(alias.clone()) {
                alias = format!("{base}_{suffix}");
                suffix += 1;
            }
            alias
        })
        .collect()
}

fn create_originals(tx: &Connection, table: &str, aliases: &[String]) -> Result<(), AppError> {
    let cells = aliases
        .iter()
        .map(|alias| format!("{} VARCHAR", quote_ident(alias)))
        .collect::<Vec<_>>()
        .join(", ");
    tx.execute_batch(&format!(
        "CREATE TABLE {} (_row BIGINT NOT NULL, _row_hash VARCHAR NOT NULL,
             _field_count INTEGER NOT NULL, _malformed BOOLEAN NOT NULL, _parse_error VARCHAR,
             _messages VARCHAR NOT NULL, {cells}, _surplus VARCHAR)",
        quote_ident(table)
    ))
    .map_err(storage("Failed to create the original-cell store"))
}

/// Rows gathered per batch before one bulk append: an appender call per cell is what dominated
/// the read.
const APPEND_BATCH_ROWS: usize = 16_384;

fn bulk_append(
    appender: &mut duckdb::Appender<'_>,
    columns: Vec<ArrayRef>,
) -> Result<(), AppError> {
    let schema = Schema::new(
        columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                Field::new(format!("c{index}"), column.data_type().clone(), true)
            })
            .collect::<Vec<_>>(),
    );
    let batch = RecordBatch::try_new(Arc::new(schema), columns)
        .map_err(|error| AppError::Internal(format!("Analytical import batch: {error}")))?;
    appender.append_record_batch(batch).map_err(read_failed)
}

/// The original-cell store's columns, built a batch at a time.
struct OriginalsBatch {
    row: Int64Builder,
    hash: StringBuilder,
    field_count: Int32Builder,
    malformed: BooleanBuilder,
    parse_error: StringBuilder,
    messages: StringBuilder,
    cells: Vec<StringBuilder>,
    surplus: StringBuilder,
}

impl OriginalsBatch {
    fn new(width: usize) -> Self {
        Self {
            row: Int64Builder::new(),
            hash: StringBuilder::new(),
            field_count: Int32Builder::new(),
            malformed: BooleanBuilder::new(),
            parse_error: StringBuilder::new(),
            messages: StringBuilder::new(),
            cells: (0..width).map(|_| StringBuilder::new()).collect(),
            surplus: StringBuilder::new(),
        }
    }

    fn len(&self) -> usize {
        self.row.len()
    }

    fn append(&mut self, appender: &mut duckdb::Appender<'_>) -> Result<(), AppError> {
        let mut columns: Vec<ArrayRef> = vec![
            Arc::new(self.row.finish()),
            Arc::new(self.hash.finish()),
            Arc::new(self.field_count.finish()),
            Arc::new(self.malformed.finish()),
            Arc::new(self.parse_error.finish()),
            Arc::new(self.messages.finish()),
        ];
        columns.extend(
            self.cells
                .iter_mut()
                .map(|cell| Arc::new(cell.finish()) as ArrayRef),
        );
        columns.push(Arc::new(self.surplus.finish()));
        bulk_append(appender, columns)
    }
}

/// The one read of the source: every record's original fields go to `staging` as text while each
/// column is profiled. Whether a row is readable is known here; how its cells convert is not, since
/// that depends on types the whole column decides.
fn read_once(
    tx: &Connection,
    source: &TableSource,
    kind: &AnalyticalDatasetKind,
    staging: &str,
) -> Result<ReadOutcome, AppError> {
    let mut originals: Option<(duckdb::Appender<'_>, OriginalsBatch)> = None;
    let (mut aliases, mut hints, mut profiles) = (Vec::new(), Vec::new(), Vec::new());
    let (mut malformed_rows, mut duplicate_key_rows, mut repaired_trailing_rows) = (0, 0, 0);
    let mut repair_row_numbers = Vec::new();
    let mut seen_keys = HashSet::new();
    let documented = *kind != AnalyticalDatasetKind::RegionalData;
    let read = source.read(|headers, row_index, record| {
        if originals.is_none() {
            aliases = column_aliases(headers);
            hints = headers
                .iter()
                .map(|header| {
                    Some(logical_type_for(kind, header)).filter(|logical| {
                        *kind != AnalyticalDatasetKind::RegionalData && !is_text(logical)
                    })
                })
                .collect();
            profiles = headers.iter().map(|_| ColumnProfile::default()).collect();
            create_originals(tx, staging, &aliases)?;
            originals = Some((
                tx.appender(staging).map_err(read_failed)?,
                OriginalsBatch::new(aliases.len()),
            ));
        }
        let width = aliases.len();
        let row_number = row_index + 1;
        let fields = record.ok();
        let mut errors = Vec::new();
        match record {
            Err(error) => errors.push(format!("CSV row parse failed: {error}")),
            Ok(record) => {
                for (index, profile) in profiles.iter_mut().enumerate() {
                    let raw = record.get(index).unwrap_or("").trim();
                    // Only an undocumented table's types are inferred from its values.
                    if !documented {
                        profile.evidence.observe(raw);
                    }
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
                            if !profile.converts[slot] {
                                profile.converts[slot] =
                                    normalize_csv_value(raw, hint, *order).is_ok();
                            }
                        }
                    }
                }
                if record.len() > width
                    && record.iter().skip(width).all(|cell| cell.trim().is_empty())
                {
                    repaired_trailing_rows += 1;
                    if repair_row_numbers.len() < 1000 {
                        repair_row_numbers.push(row_number);
                    }
                } else if record.len() != width {
                    errors.push(format!("Expected {width} fields, found {}", record.len()));
                }
                if let Some(key) = analytical_counting_key(kind, headers, record) {
                    if !seen_keys.insert(key_digest(&key)) {
                        duplicate_key_rows += 1;
                    }
                }
            }
        }
        let malformed = !errors.is_empty();
        malformed_rows += usize::from(malformed);
        let (appender, batch) = originals.as_mut().expect("created on the first record");
        batch.row.append_value(row_number as i64);
        batch.hash.append_value(row_store::row_hash(fields));
        batch
            .field_count
            .append_value(fields.map_or(0, csv::StringRecord::len) as i32);
        batch.malformed.append_value(malformed);
        batch.parse_error.append_option(record.err());
        batch
            .messages
            .append_value(serde_json::to_string(&errors).expect("strings always serialize"));
        for (index, cell) in batch.cells.iter_mut().enumerate() {
            cell.append_option(fields.and_then(|record| record.get(index)));
        }
        // A JSON array: the store holds text, and the row view casts it back.
        batch
            .surplus
            .append_option(fields.filter(|record| record.len() > width).map(|record| {
                serde_json::to_string(&record.iter().skip(width).collect::<Vec<_>>())
                    .expect("strings always serialize")
            }));
        if batch.len() == APPEND_BATCH_ROWS {
            batch.append(appender)?;
        }
        Ok(())
    })?;
    match originals {
        Some((mut appender, mut batch)) => {
            if batch.len() > 0 {
                batch.append(&mut appender)?;
            }
            appender.flush().map_err(read_failed)?
        }
        // A header-only table never reached the callback.
        None => {
            aliases = column_aliases(&read.headers);
            profiles = read
                .headers
                .iter()
                .map(|_| ColumnProfile::default())
                .collect();
            create_originals(tx, staging, &aliases)?;
        }
    }
    Ok(ReadOutcome {
        headers: read.headers,
        aliases,
        profiles,
        sha256: read.sha256,
        rows: read.rows,
        malformed_rows,
        duplicate_key_rows,
        repaired_trailing_rows,
        repair_row_numbers,
    })
}

/// A typed column's values, built a batch at a time in the Arrow type that matches what
/// `normalize_csv_value` produces for it; DuckDB casts each to the column's declared type.
enum TypedColumn {
    Integer(Int64Builder),
    Float(Float64Builder),
    Text(StringBuilder),
}

impl TypedColumn {
    fn for_logical(logical: &str) -> Self {
        match logical {
            "INTEGER" => Self::Integer(Int64Builder::new()),
            "FLOAT" | "DECIMAL" => Self::Float(Float64Builder::new()),
            _ => Self::Text(StringBuilder::new()),
        }
    }

    fn append(&mut self, value: &serde_json::Value) -> Result<(), AppError> {
        match (self, value) {
            (Self::Integer(column), serde_json::Value::Null) => column.append_null(),
            (Self::Float(column), serde_json::Value::Null) => column.append_null(),
            (Self::Text(column), serde_json::Value::Null) => column.append_null(),
            (Self::Integer(column), serde_json::Value::Number(number)) if number.is_i64() => {
                column.append_option(number.as_i64())
            }
            (Self::Float(column), serde_json::Value::Number(number)) => {
                column.append_option(number.as_f64())
            }
            (Self::Text(column), serde_json::Value::String(text)) => column.append_value(text),
            (_, value) => {
                return Err(AppError::Internal(format!(
                    "Analytical import: converted value {value} does not fit its column"
                )))
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> ArrayRef {
        match self {
            Self::Integer(column) => Arc::new(column.finish()),
            Self::Float(column) => Arc::new(column.finish()),
            Self::Text(column) => Arc::new(column.finish()),
        }
    }
}

/// Converts every readable row from the stored originals into the typed table, in bounded
/// batches, and returns per-column empty and unconverted counts. A row whose cells did not all
/// convert gets the reasons recorded beside its originals.
fn convert_rows(
    tx: &Connection,
    originals: &str,
    typed: &str,
    plans: &[ColumnPlan],
    rows: usize,
    token: &str,
) -> Result<(Vec<usize>, Vec<usize>), AppError> {
    let mut null_counts = vec![0usize; plans.len()];
    let mut unconverted = vec![0usize; plans.len()];
    let notes_table = format!("analysis_src_notes_{token}");
    tx.execute_batch(&format!(
        "CREATE TABLE {} (_row BIGINT NOT NULL, _messages VARCHAR NOT NULL)",
        quote_ident(&notes_table)
    ))
    .map_err(storage("Failed to stage conversion notes"))?;
    let cells = plans
        .iter()
        .map(|plan| quote_ident(&plan.alias))
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = tx
        .prepare(&format!(
            "SELECT _row, {cells} FROM {} WHERE NOT _malformed AND _row > ?1 AND _row <= ?2 ORDER BY _row",
            quote_ident(originals)
        ))
        .map_err(read_failed)?;
    let mut typed_rows = tx.appender(typed).map_err(read_failed)?;
    let mut notes_rows = tx.appender(&notes_table).map_err(read_failed)?;
    let unreadable =
        || AppError::Internal("Analytical import: stored originals are not text".into());
    // `_row` numbers every source row from 1, malformed ones included, so a fixed range scans one
    // batch of the store rather than sorting everything after the previous batch.
    for after in (0..rows as i64).step_by(CONVERT_BATCH_ROWS as usize) {
        statement
            .execute(params![after, after + CONVERT_BATCH_ROWS])
            .map_err(read_failed)?;
        while let Some(chunk) = statement.step().map_err(read_failed)? {
            let source_rows = chunk
                .column(0)
                .as_primitive_opt::<Int64Type>()
                .ok_or_else(unreadable)?;
            let stored = (1..=plans.len())
                .map(|index| {
                    chunk
                        .column(index)
                        .as_string_opt::<i32>()
                        .ok_or_else(unreadable)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut columns = plans
                .iter()
                .map(|plan| TypedColumn::for_logical(plan.logical))
                .collect::<Vec<_>>();
            for position in 0..chunk.len() {
                let row = source_rows.value(position);
                let mut notes = Vec::new();
                for (index, plan) in plans.iter().enumerate() {
                    let cell = &stored[index];
                    let raw = if cell.is_null(position) {
                        ""
                    } else {
                        cell.value(position).trim()
                    };
                    let value = match normalize_csv_value(raw, plan.logical, plan.order) {
                        Ok(value) => {
                            null_counts[index] += usize::from(value.is_null());
                            value
                        }
                        Err(error) => {
                            unconverted[index] += 1;
                            notes.push(format!("{}: {error}", plan.header));
                            serde_json::Value::Null
                        }
                    };
                    columns[index].append(&value)?;
                }
                if !notes.is_empty() {
                    notes_rows
                        .append_row(params![
                            row,
                            serde_json::to_string(&notes).expect("strings always serialize")
                        ])
                        .map_err(read_failed)?;
                }
            }
            let mut arrays = columns
                .iter_mut()
                .map(TypedColumn::finish)
                .collect::<Vec<_>>();
            arrays.push(chunk.column(0).clone());
            bulk_append(&mut typed_rows, arrays)?;
        }
    }
    typed_rows.flush().map_err(read_failed)?;
    notes_rows.flush().map_err(read_failed)?;
    drop((typed_rows, notes_rows, statement));
    tx.execute_batch(&format!(
        "UPDATE {originals} SET _messages = n._messages FROM {notes} n WHERE {originals}._row = n._row;
         DROP TABLE {notes};",
        originals = quote_ident(originals),
        notes = quote_ident(&notes_table),
    ))
    .map_err(storage("Failed to record conversion notes"))?;
    Ok((null_counts, unconverted))
}

impl DuckDbRepository {
    pub(crate) async fn import_analytical_table(
        &self,
        request: AnalyticalImportRequest,
        automatic_origin: Option<(String, u64)>,
    ) -> Result<AnalyticalImportResult, AppError> {
        self.query_connection("Analytical table import", move |conn| {
            import_table(conn, request, automatic_origin)
        })
        .await
    }
}

fn import_table(
    conn: &mut Connection,
    request: AnalyticalImportRequest,
    origin: Option<(String, u64)>,
) -> Result<AnalyticalImportResult, AppError> {
    let kind = request.dataset_kind.clone();
    let format_name = request
        .source_file_name
        .clone()
        .unwrap_or_else(|| request.local_path.clone());
    let source = TableSource::open(
        std::path::Path::new(&request.local_path),
        &format_name,
        request.worksheet.as_deref(),
    )?;
    // Every write below commits together or not at all, staging tables included.
    let tx = conn
        .transaction()
        .map_err(storage("Failed to begin analytical import"))?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let staging = format!("analysis_src_import_{token}");
    let read = read_once(&tx, &source, &kind, &staging)?;
    if request
        .expected_source_sha256
        .as_ref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&read.sha256))
    {
        return Err(AppError::DataParsing(
            "The acquired file no longer matches its recorded SHA-256; import stopped.".into(),
        ));
    }
    // Equal bytes from different publishers or worksheets are distinct acquisitions.
    let identity_hash = sha256_hex(
        serde_json::json!([
            ANALYTICAL_PARSER_VERSION,
            read.sha256,
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
    let originals = format!("analysis_src_{}", &identity_hash[..24]);
    let latest_view = if kind == AnalyticalDatasetKind::RegionalData {
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
    let mut warnings = schema_warnings_for(&read.headers, &kind);
    warnings.extend(source.warnings().iter().cloned());
    if read
        .headers
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != read.headers.len()
    {
        warnings.push("Duplicate column headers were assigned distinct SQL names; raw rows are retained by column position.".into());
    }
    let plans = plan_columns(
        &kind,
        &read.headers,
        read.aliases,
        &read.profiles,
        &mut warnings,
    );
    let declares_partial = source.declares_partial();
    if declares_partial {
        warnings.push(
            "The source declares additional pages or worksheets; this snapshot is partial.".into(),
        );
    }
    let partial = request.partial_dataset || declares_partial;
    let (table, store) = (quote_ident(&table_name), quote_ident(&originals));
    let create_columns = plans
        .iter()
        .map(|plan| format!("{} {}", quote_ident(&plan.alias), duckdb_type(plan.logical)))
        .collect::<Vec<_>>()
        .join(", ");
    tx.execute_batch(&format!(
        "DROP TABLE IF EXISTS {table};
         DROP TABLE IF EXISTS {store};
         ALTER TABLE {} RENAME TO {store};
         CREATE TABLE {table} ({create_columns}, _source_row BIGINT NOT NULL);",
        quote_ident(&staging)
    ))
    .map_err(storage("Failed to create analytical table"))?;
    for statement in [
        "DELETE FROM analytical_row_ledger WHERE snapshot_id = ?1",
        "DELETE FROM analytical_columns WHERE snapshot_id = ?1",
        "DELETE FROM analytical_snapshots WHERE snapshot_id = ?1",
    ] {
        tx.execute(statement, params![snapshot_id])
            .map_err(storage("Failed to replace snapshot"))?;
    }
    let (null_counts, unconverted) =
        convert_rows(&tx, &originals, &table_name, &plans, read.rows, &token)?;
    let unconverted_cells = unconverted.iter().sum::<usize>();
    if read.repaired_trailing_rows > 0 {
        warnings.push(format!("Format repair: ignored only empty surplus trailing fields in {} rows; all original field values remain in the row ledger.", read.repaired_trailing_rows));
    }
    if unconverted_cells > 0 {
        warnings.push(format!("{unconverted_cells} cells did not convert to their column's type; they are NULL in the table and kept as written in the row ledger."));
    }
    let validation_summary = serde_json::json!({
        "malformedRows": read.malformed_rows,
        "duplicateKeyRows": read.duplicate_key_rows,
        "rejectedRows": read.malformed_rows,
        "unconvertedCells": unconverted_cells,
        "unconvertedByColumn": plans.iter().zip(&unconverted).filter(|(_, n)| **n > 0).map(|(plan, n)| (plan.alias.clone(), *n)).collect::<BTreeMap<_, _>>(),
        "dateOrder": plans.iter().filter(|plan| matches!(plan.logical, "DATE" | "TIMESTAMP")).map(|plan| (plan.alias.clone(), plan.order.as_str())).collect::<BTreeMap<_, _>>(),
        "repairs": { "emptyTrailingFields": { "rowCount": read.repaired_trailing_rows, "rowNumbers": read.repair_row_numbers, "rowNumbersTruncated": read.repaired_trailing_rows > 1000 } },
        "parserVersion": ANALYTICAL_PARSER_VERSION,
        "sourceFormat": source.format(),
        "sourceDeclaresPartial": declares_partial,
        "worksheet": request.worksheet,
        "warnings": warnings,
        "nullCounts": plans.iter().zip(&null_counts).map(|(plan, n)| (plan.alias.clone(), *n)).collect::<BTreeMap<_, _>>(),
        "notes": validation_notes_for(&kind),
        (row_store::ROW_STORE_KEY): originals,
        "derivedSubset": request.derived_subset.as_ref().map(|subset| serde_json::json!({
            "publisherSha256": subset.publisher_sha256,
            "publisherRows": subset.publisher_rows,
            "filter": subset.filter,
            "note": "sourceSha256 is the digest of this filtered subset, not of the publisher's file.",
        })),
    });
    let imported_rows = read.rows - read.malformed_rows;
    tx.execute(
        "INSERT INTO analytical_snapshots (
            snapshot_id, dataset_kind, display_name, table_name, source_uri, local_path,
            source_sha256, retrieved_at, observation_period_start, observation_period_end,
            partial_dataset, imported_rows, rejected_rows, malformed_rows,
            duplicate_key_rows, validation_summary_json
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7,
            strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'), ?8, ?9, ?10, ?11, ?12, ?12, ?13, ?14)",
        params![
            snapshot_id,
            kind.as_str(),
            request.display_name,
            table_name,
            request.source_uri,
            request.local_path,
            read.sha256,
            request.observation_period_start,
            request.observation_period_end,
            i64::from(partial),
            imported_rows as i64,
            read.malformed_rows as i64,
            read.duplicate_key_rows as i64,
            validation_summary.to_string(),
        ],
    )
    .map_err(storage("Failed to record analytical snapshot"))?;
    for (ordinal, plan) in plans.iter().enumerate() {
        tx.execute(
            "INSERT INTO analytical_columns (
                snapshot_id, table_name, column_name, original_name, logical_type,
                nullable, dictionary, ordinal
             ) VALUES (?1, ?2, ?3, ?4, ?5, 1, NULL, ?6)",
            params![
                snapshot_id,
                table_name,
                plan.alias,
                plan.header,
                plan.logical,
                ordinal as i64
            ],
        )
        .map_err(storage("Failed to record analytical column"))?;
    }
    tx.execute_batch(&format!(
        "CREATE OR REPLACE VIEW {} AS SELECT * EXCLUDE (_source_row) FROM {table}",
        quote_ident(&latest_view)
    ))
    .map_err(storage("Failed to refresh latest analytical view"))?;
    row_store::rebuild_row_view(&tx)?;
    if let Some((portal_key, document_id)) = &origin {
        tx.execute(
            "INSERT INTO data_maintenance_origins VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
            params![snapshot_id, portal_key, document_id],
        )
        .map_err(storage(
            "Failed to record the automatic-import origin receipt",
        ))?;
    }
    tx.commit()
        .map_err(storage("Failed to commit analytical import"))?;
    Ok(AnalyticalImportResult {
        snapshot_id,
        dataset_kind: kind,
        display_name: request.display_name,
        table_name,
        imported_rows,
        rejected_rows: read.malformed_rows,
        malformed_rows: read.malformed_rows,
        duplicate_key_rows: read.duplicate_key_rows,
        unconverted_cells,
        schema_warnings: warnings,
        source_sha256: read.sha256,
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
    async fn every_readable_row_reaches_the_typed_table_across_conversion_batches() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nibrs.csv");
        let batch = CONVERT_BATCH_ROWS as usize;
        let mut csv = String::from("nibrs_uniq,case_number,ibr_offense,occured_on\n");
        for row in 1..=batch + 1 {
            if row == batch - 1 {
                csv.push_str("unreadable\n");
            } else {
                csv.push_str(&format!("{row},{row},90Z,2025-01-02\n"));
            }
        }
        std::fs::write(&path, csv).unwrap();
        let repo = repository().await;
        let result = repo
            .import_analytical_csv(request(&path, AnalyticalDatasetKind::NibrsOffenses))
            .await
            .unwrap();
        assert_eq!((result.imported_rows, result.rejected_rows), (batch, 1));
        let rows = query(
            &repo,
            &format!(
                "SELECT count(*), count(DISTINCT nibrs_uniq), min(nibrs_uniq::BIGINT),
                        max(nibrs_uniq::BIGINT),
                        count(*) FILTER (WHERE nibrs_uniq IN ('{}', '{batch}', '{}'))
                 FROM analysis_nibrs_offenses_latest",
                batch - 2,
                batch + 1
            ),
        )
        .await;
        assert_eq!(
            rows[0],
            [batch, batch, 1, batch + 1, 3].map(|n| serde_json::json!(n))
        );
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
    async fn an_import_refused_after_its_read_leaves_nothing_behind() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rows.csv");
        std::fs::write(&path, "a,b\n1,2\n").unwrap();
        let repo = repository().await;
        let mut input = request(&path, AnalyticalDatasetKind::RegionalData);
        input.expected_source_sha256 = Some("0".repeat(64));
        let error = repo.import_analytical_csv(input).await.unwrap_err();
        assert!(error.to_string().contains("no longer matches"), "{error}");
        let left = repo
            .query_connection("test", |conn| {
                conn.query_row(
                    "SELECT string_agg(table_name, ',') FROM (SELECT table_name FROM information_schema.tables WHERE table_name LIKE 'analysis_src%' OR table_name LIKE 'analysis_regional%' UNION ALL SELECT snapshot_id FROM analytical_snapshots)",
                    [],
                    |row| row.get::<_, Option<String>>(0),
                )
                .map_err(|error| AppError::Storage(error.to_string()))
            })
            .await
            .unwrap();
        assert_eq!(left, None);
    }
}
