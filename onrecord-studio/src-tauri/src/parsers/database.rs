use std::path::Path;

// NOTE(import-compatibility): rusqlite is intentionally retained only to inspect arbitrary
// user-supplied SQLite files; it is not used by the application's core database.
use rusqlite::config::DbConfig;
use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use rusqlite::{params, Connection, OpenFlags};
use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::integrity::digest_hex;
use crate::normalization::normalize_delimited_field as sanitize_field;
use crate::parsers::source::inspect_regular_file;

const MAX_DATABASE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_TABLES: usize = 10_000;
const MAX_ROWS_PER_TABLE: usize = 100_000;
const MAX_TOTAL_ROWS: usize = 1_000_000;
const MAX_OUTPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_VALUE_BYTES: i32 = 16 * 1024 * 1024;

// FEATURE(database): Add bounded read-only extractors for DuckDB, Access, and database dump
// formats, preserving the source engine and schema provenance in the normalized output.
pub fn extract_sqlite_text<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    let path = file_path.as_ref();
    inspect_regular_file(path, "SQLITE", MAX_DATABASE_BYTES)?;

    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        AppError::DataParsing(format!(
            "SQLITE_OPEN_FAILED: failed to open '{}': {error}",
            path.display()
        ))
    })?;
    harden_untrusted_connection(&connection, path)?;
    let mut table_statement = connection
        .prepare(
            "SELECT name, COALESCE(sql, '')
             FROM sqlite_schema
             WHERE type = ?1 AND substr(name, 1, 7) <> ?2
             ORDER BY name
             LIMIT ?3",
        )
        .map_err(|error| {
            AppError::DataParsing(format!(
                "SQLITE_SCHEMA_READ_FAILED: failed to inspect '{}': {error}",
                path.display()
            ))
        })?;
    let table_limit = i64::try_from(MAX_TABLES + 1)
        .map_err(|error| AppError::Internal(format!("SQLITE_LIMIT_INVALID: {error}")))?;
    let tables = table_statement
        .query_map(params!["table", "sqlite_", table_limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| {
            AppError::DataParsing(format!(
                "SQLITE_SCHEMA_READ_FAILED: failed to inspect '{}': {error}",
                path.display()
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| {
            AppError::DataParsing(format!(
                "SQLITE_SCHEMA_READ_FAILED: failed to decode '{}': {error}",
                path.display()
            ))
        })?;
    if tables.len() > MAX_TABLES {
        return Err(AppError::DataParsing(format!(
            "SQLITE_TABLE_COUNT_EXCEEDED: '{}' exceeds {MAX_TABLES} tables",
            path.display()
        )));
    }

    let mut output = String::new();
    let mut total_rows = 0_usize;
    for (table_name, schema) in tables {
        append(
            &mut output,
            &format!(
                "--- Table: {} ---\nSchema: {}\n",
                sanitize_field(&table_name),
                sanitize_field(&schema)
            ),
            path,
        )?;
        let sql = format!("SELECT * FROM {} LIMIT ?1", quote_identifier(&table_name));
        let mut statement = connection.prepare(&sql).map_err(|error| {
            AppError::DataParsing(format!(
                "SQLITE_TABLE_READ_FAILED: failed to prepare table '{}': {error}",
                table_name
            ))
        })?;
        let column_names = statement
            .column_names()
            .into_iter()
            .map(sanitize_field)
            .collect::<Vec<_>>();
        append(&mut output, &column_names.join(","), path)?;
        append(&mut output, "\n", path)?;

        let row_limit = i64::try_from(MAX_ROWS_PER_TABLE)
            .map_err(|error| AppError::Internal(format!("SQLITE_LIMIT_INVALID: {error}")))?;
        let column_count = statement.column_count();
        let mut rows = statement.query(params![row_limit]).map_err(|error| {
            AppError::DataParsing(format!(
                "SQLITE_TABLE_READ_FAILED: failed to query table '{}': {error}",
                table_name
            ))
        })?;
        while let Some(row) = rows.next().map_err(|error| {
            AppError::DataParsing(format!(
                "SQLITE_TABLE_READ_FAILED: failed to read table '{}': {error}",
                table_name
            ))
        })? {
            total_rows = total_rows.checked_add(1).ok_or_else(|| {
                AppError::DataParsing("SQLITE_ROW_COUNT_OVERFLOW: row count overflow".to_string())
            })?;
            if total_rows > MAX_TOTAL_ROWS {
                return Err(AppError::DataParsing(format!(
                    "SQLITE_ROW_COUNT_EXCEEDED: '{}' exceeds {MAX_TOTAL_ROWS} extracted rows",
                    path.display()
                )));
            }
            for column in 0..column_count {
                if column > 0 {
                    append(&mut output, ",", path)?;
                }
                let value = row.get_ref(column).map_err(|error| {
                    AppError::DataParsing(format!(
                        "SQLITE_VALUE_READ_FAILED: table '{}' column {}: {error}",
                        table_name, column
                    ))
                })?;
                // PERF(database): Write normalized cell values directly into `output`; allocating
                // a temporary String per cell amplifies allocator traffic at the million-row cap.
                append(&mut output, &value_to_text(value), path)?;
            }
            append(&mut output, "\n", path)?;
        }
        append(&mut output, "\n", path)?;
    }

    let output = output.trim().to_string();
    if output.is_empty() {
        return Err(AppError::DataParsing(format!(
            "SQLITE_OUTPUT_EMPTY: '{}' contains no extractable user tables",
            path.display()
        )));
    }
    Ok(output)
}

fn harden_untrusted_connection(connection: &Connection, path: &Path) -> Result<(), AppError> {
    connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, MAX_VALUE_BYTES);
    connection.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0);
    connection.set_limit(Limit::SQLITE_LIMIT_WORKER_THREADS, 0);
    let defensive = connection
        .set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)
        .map_err(|error| {
            AppError::DataParsing(format!(
                "SQLITE_HARDENING_FAILED: failed to enable defensive mode for '{}': {error}",
                path.display()
            ))
        })?;
    if !defensive {
        return Err(AppError::DataParsing(format!(
            "SQLITE_HARDENING_FAILED: defensive mode did not activate for '{}'",
            path.display()
        )));
    }
    connection
        .execute_batch(
            "PRAGMA query_only = ON;
             PRAGMA trusted_schema = OFF;
             PRAGMA cell_size_check = ON;",
        )
        .map_err(|error| {
            AppError::DataParsing(format!(
                "SQLITE_HARDENING_FAILED: failed to configure '{}': {error}",
                path.display()
            ))
        })
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn value_to_text(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Null => String::new(),
        ValueRef::Integer(value) => value.to_string(),
        ValueRef::Real(value) => value.to_string(),
        ValueRef::Text(value) => sanitize_field(&String::from_utf8_lossy(value)),
        ValueRef::Blob(value) => format!(
            "[BLOB bytes={} sha256={}]",
            value.len(),
            digest_hex(Sha256::digest(value))
        ),
    }
}

fn append(output: &mut String, value: &str, path: &Path) -> Result<(), AppError> {
    if output
        .len()
        .checked_add(value.len())
        .is_none_or(|size| size > MAX_OUTPUT_BYTES)
    {
        return Err(AppError::DataParsing(format!(
            "SQLITE_OUTPUT_SIZE_EXCEEDED: '{}' exceeds {MAX_OUTPUT_BYTES} extracted bytes",
            path.display()
        )));
    }
    output.push_str(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_sqlite_connections_enable_every_hardening_control() {
        let connection = Connection::open_in_memory().expect("fixture connection must open");
        harden_untrusted_connection(&connection, Path::new("untrusted.sqlite"))
            .expect("hardening must succeed");

        assert!(connection
            .db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE)
            .expect("defensive state must be readable"));
        for pragma in ["query_only", "cell_size_check"] {
            let enabled: i64 = connection
                .pragma_query_value(None, pragma, |row| row.get(0))
                .expect("hardening pragma must be readable");
            assert_eq!(enabled, 1, "{pragma} must be enabled");
        }
        let trusted_schema: i64 = connection
            .pragma_query_value(None, "trusted_schema", |row| row.get(0))
            .expect("trusted_schema must be readable");
        assert_eq!(trusted_schema, 0);
        assert_eq!(
            connection.limit(Limit::SQLITE_LIMIT_LENGTH),
            MAX_VALUE_BYTES
        );
        assert_eq!(connection.limit(Limit::SQLITE_LIMIT_ATTACHED), 0);
        assert_eq!(connection.limit(Limit::SQLITE_LIMIT_WORKER_THREADS), 0);
    }
}
