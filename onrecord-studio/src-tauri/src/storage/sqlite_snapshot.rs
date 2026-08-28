use std::path::Path;

use duckdb::{appender_params_from_iter, types::Value as DuckValue, Connection};
use rusqlite::{types::ValueRef as SqliteValueRef, OpenFlags};

use crate::error::AppError;

const SNAPSHOT_SCHEMA: &str = "db";
const MAXIMUM_COLUMNS_PER_TABLE: usize = 256;
const MAXIMUM_SNAPSHOT_ROWS: usize = 5_000_000;
const MAXIMUM_VALUE_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_SNAPSHOT_VALUE_BYTES: usize = 512 * 1024 * 1024;

struct SnapshotColumn {
    name: String,
    duckdb_type: &'static str,
}

/// Imports a bounded, read-only snapshot of selected SQLite tables into DuckDB's `db` schema.
///
/// This deliberately avoids DuckDB's downloadable SQLite extension. The source is opened with
/// SQLite's read-only flag, only the fixed caller-provided table allowlist is read, identifiers are
/// quoted, and row/value budgets bound a hostile or unexpectedly large input. The resulting
/// in-memory schema has the same `db.table` names used by the analytical SQL.
pub(crate) fn import_sqlite_snapshot(
    duckdb: &Connection,
    sqlite_path: &Path,
    tables: &[&str],
) -> Result<(), AppError> {
    let sqlite = rusqlite::Connection::open_with_flags(
        sqlite_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        AppError::Analysis(format!("Failed to open SQLite source read-only: {error}"))
    })?;
    sqlite
        .execute_batch("PRAGMA query_only = ON; PRAGMA trusted_schema = OFF;")
        .map_err(|error| {
            AppError::Analysis(format!("Failed to restrict the SQLite source: {error}"))
        })?;

    duckdb
        .execute_batch(&format!("BEGIN; CREATE SCHEMA {SNAPSHOT_SCHEMA};"))
        .map_err(|error| {
            AppError::Analysis(format!(
                "Failed to initialize SQLite snapshot schema: {error}"
            ))
        })?;
    let result = (|| {
        let mut total_rows = 0_usize;
        let mut total_value_bytes = 0_usize;
        for table in tables {
            import_table(
                duckdb,
                &sqlite,
                table,
                &mut total_rows,
                &mut total_value_bytes,
            )?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => duckdb.execute_batch("COMMIT;").map_err(|error| {
            AppError::Analysis(format!("Failed to commit SQLite snapshot: {error}"))
        }),
        Err(error) => {
            let _ = duckdb.execute_batch("ROLLBACK;");
            Err(error)
        }
    }
}

fn import_table(
    duckdb: &Connection,
    sqlite: &rusqlite::Connection,
    table: &str,
    total_rows: &mut usize,
    total_value_bytes: &mut usize,
) -> Result<(), AppError> {
    let quoted_table = quote_identifier(table)?;
    let columns = load_columns(sqlite, table, &quoted_table)?;
    let definition = columns
        .iter()
        .map(|column| {
            Ok(format!(
                "{} {}",
                quote_identifier(&column.name)?,
                column.duckdb_type
            ))
        })
        .collect::<Result<Vec<_>, AppError>>()?
        .join(", ");
    duckdb
        .execute_batch(&format!(
            "CREATE TABLE {SNAPSHOT_SCHEMA}.{quoted_table} ({definition});"
        ))
        .map_err(|error| {
            AppError::Analysis(format!(
                "Failed to create SQLite snapshot table '{table}': {error}"
            ))
        })?;

    let mut select = sqlite
        .prepare(&format!("SELECT * FROM {quoted_table}"))
        .map_err(|error| {
            AppError::Analysis(format!("Failed to read SQLite table '{table}': {error}"))
        })?;
    let mut rows = select.query([]).map_err(|error| {
        AppError::Analysis(format!("Failed to query SQLite table '{table}': {error}"))
    })?;
    let mut appender = duckdb
        .appender_to_db(table, SNAPSHOT_SCHEMA)
        .map_err(|error| {
            AppError::Analysis(format!(
                "Failed to open snapshot appender for '{table}': {error}"
            ))
        })?;
    while let Some(row) = rows.next().map_err(|error| {
        AppError::Analysis(format!("Failed to advance SQLite table '{table}': {error}"))
    })? {
        *total_rows = total_rows.checked_add(1).ok_or_else(|| {
            AppError::Analysis("SQLite snapshot row count overflowed".to_string())
        })?;
        if *total_rows > MAXIMUM_SNAPSHOT_ROWS {
            return Err(AppError::Analysis(format!(
                "SQLite snapshot exceeds the {MAXIMUM_SNAPSHOT_ROWS}-row limit"
            )));
        }
        let mut values = Vec::with_capacity(columns.len());
        for (index, column) in columns.iter().enumerate() {
            let value = row.get_ref(index).map_err(|error| {
                AppError::Analysis(format!(
                    "Failed to decode SQLite table '{table}' column {}: {error}",
                    column.name
                ))
            })?;
            values.push(convert_value(value, total_value_bytes)?);
        }
        appender
            .append_row(appender_params_from_iter(values))
            .map_err(|error| {
                AppError::Analysis(format!("Failed to import SQLite table '{table}': {error}"))
            })?;
    }
    appender.flush().map_err(|error| {
        AppError::Analysis(format!("Failed to flush SQLite table '{table}': {error}"))
    })
}

fn load_columns(
    sqlite: &rusqlite::Connection,
    table: &str,
    quoted_table: &str,
) -> Result<Vec<SnapshotColumn>, AppError> {
    let mut statement = sqlite
        .prepare(&format!("PRAGMA table_info({quoted_table})"))
        .map_err(|error| {
            AppError::Analysis(format!("Failed to inspect SQLite table '{table}': {error}"))
        })?;
    let columns = statement
        .query_map([], |row| {
            let name = row.get::<_, String>(1)?;
            let declared_type = row.get::<_, String>(2)?;
            Ok(SnapshotColumn {
                name,
                duckdb_type: duckdb_type_for_sqlite(&declared_type),
            })
        })
        .map_err(|error| {
            AppError::Analysis(format!("Failed to query SQLite table '{table}': {error}"))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| {
            AppError::Analysis(format!("Failed to decode SQLite table '{table}': {error}"))
        })?;
    if columns.is_empty() {
        return Err(AppError::Analysis(format!(
            "Required SQLite table '{table}' is missing or has no columns"
        )));
    }
    if columns.len() > MAXIMUM_COLUMNS_PER_TABLE {
        return Err(AppError::Analysis(format!(
            "SQLite table '{table}' exceeds the {MAXIMUM_COLUMNS_PER_TABLE}-column limit"
        )));
    }
    Ok(columns)
}

fn convert_value(
    value: SqliteValueRef<'_>,
    total_value_bytes: &mut usize,
) -> Result<DuckValue, AppError> {
    let (value, byte_count) = match value {
        SqliteValueRef::Null => (DuckValue::Null, 0),
        SqliteValueRef::Integer(value) => (DuckValue::BigInt(value), size_of::<i64>()),
        SqliteValueRef::Real(value) => (DuckValue::Double(value), size_of::<f64>()),
        SqliteValueRef::Text(value) => {
            if value.len() > MAXIMUM_VALUE_BYTES {
                return Err(AppError::Analysis(format!(
                    "SQLite text value exceeds the {MAXIMUM_VALUE_BYTES}-byte limit"
                )));
            }
            let text = std::str::from_utf8(value).map_err(|error| {
                AppError::Analysis(format!("SQLite text is not valid UTF-8: {error}"))
            })?;
            (DuckValue::Text(text.to_string()), value.len())
        }
        SqliteValueRef::Blob(value) => {
            if value.len() > MAXIMUM_VALUE_BYTES {
                return Err(AppError::Analysis(format!(
                    "SQLite blob value exceeds the {MAXIMUM_VALUE_BYTES}-byte limit"
                )));
            }
            (DuckValue::Blob(value.to_vec()), value.len())
        }
    };
    *total_value_bytes = total_value_bytes
        .checked_add(byte_count)
        .ok_or_else(|| AppError::Analysis("SQLite snapshot byte count overflowed".to_string()))?;
    if *total_value_bytes > MAXIMUM_SNAPSHOT_VALUE_BYTES {
        return Err(AppError::Analysis(format!(
            "SQLite snapshot exceeds the {MAXIMUM_SNAPSHOT_VALUE_BYTES}-byte value limit"
        )));
    }
    Ok(value)
}

fn duckdb_type_for_sqlite(declared_type: &str) -> &'static str {
    let declared = declared_type.to_ascii_uppercase();
    if declared.contains("INT") {
        "BIGINT"
    } else if declared.contains("REAL") || declared.contains("FLOA") || declared.contains("DOUB") {
        "DOUBLE"
    } else if declared.contains("BLOB") {
        "BLOB"
    } else {
        "TEXT"
    }
}

fn quote_identifier(value: &str) -> Result<String, AppError> {
    if value.is_empty() || value.len() > 256 || value.bytes().any(|byte| byte == 0) {
        return Err(AppError::Analysis(
            "SQLite schema contains an invalid identifier".to_string(),
        ));
    }
    Ok(format!("\"{}\"", value.replace('"', "\"\"")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_an_offline_read_only_snapshot_without_duckdb_extensions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.sqlite");
        let sqlite = rusqlite::Connection::open(&path).unwrap();
        sqlite
            .execute_batch(
                "CREATE TABLE requests (id INTEGER, title TEXT, score REAL, payload BLOB);
                 INSERT INTO requests VALUES (7, 'Evidence', 1.5, X'0102');",
            )
            .unwrap();
        drop(sqlite);

        let duckdb = Connection::open_in_memory().unwrap();
        import_sqlite_snapshot(&duckdb, &path, &["requests"]).unwrap();
        let row: (i64, String, f64, Vec<u8>) = duckdb
            .query_row(
                "SELECT id, title, score, payload FROM db.requests",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row, (7, "Evidence".to_string(), 1.5, vec![1, 2]));
    }
}
