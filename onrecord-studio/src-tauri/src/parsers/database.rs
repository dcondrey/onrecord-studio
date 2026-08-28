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
use crate::parsers::source::{inspect_regular_file, read_regular_file};

const MAX_DATABASE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_TABLES: usize = 10_000;
const MAX_ROWS_PER_TABLE: usize = 100_000;
const MAX_TOTAL_ROWS: usize = 1_000_000;
const MAX_OUTPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_VALUE_BYTES: i32 = 16 * 1024 * 1024;
const MAX_SQL_DUMP_BYTES: u64 = 64 * 1024 * 1024;

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
        append_output(
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
            .map(|name| sanitize_field(&name))
            .collect::<Vec<_>>();
        append_output(&mut output, &column_names.join(","), path)?;
        append_output(&mut output, "\n", path)?;

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
                    append_output(&mut output, ",", path)?;
                }
                let value = row.get_ref(column).map_err(|error| {
                    AppError::DataParsing(format!(
                        "SQLITE_VALUE_READ_FAILED: table '{}' column {}: {error}",
                        table_name, column
                    ))
                })?;
                append_value(&mut output, value, path)?;
            }
            append_output(&mut output, "\n", path)?;
        }
        append_output(&mut output, "\n", path)?;
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

pub fn extract_duckdb_text<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    use duckdb::types::ValueRef as DuckValueRef;
    use duckdb::{AccessMode, Config, Connection as DuckConnection};

    let path = file_path.as_ref();
    inspect_regular_file(path, "DUCKDB", MAX_DATABASE_BYTES)?;
    let config = Config::default()
        .access_mode(AccessMode::ReadOnly)
        .and_then(|config| config.enable_external_access(false))
        .and_then(|config| config.max_memory("512MB"))
        .and_then(|config| config.threads(1))
        .and_then(|config| config.with("autoload_known_extensions", "false"))
        .and_then(|config| config.with("autoinstall_known_extensions", "false"))
        .map_err(|error| AppError::Internal(format!("DUCKDB_READ_ONLY_CONFIG_FAILED:{error}")))?;
    let connection = DuckConnection::open_with_flags(path, config).map_err(|error| {
        AppError::DataParsing(format!(
            "DUCKDB_OPEN_FAILED: failed to open '{}': {error}",
            path.display()
        ))
    })?;
    let mut table_statement = connection
        .prepare(
            "SELECT schema_name, table_name, COALESCE(sql, '')
             FROM duckdb_tables()
             WHERE NOT internal
             ORDER BY schema_name, table_name
             LIMIT ?",
        )
        .map_err(|error| AppError::DataParsing(format!("DUCKDB_SCHEMA_READ_FAILED:{error}")))?;
    let tables = table_statement
        .query_map([i64::try_from(MAX_TABLES + 1).unwrap_or(i64::MAX)], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| AppError::DataParsing(format!("DUCKDB_SCHEMA_READ_FAILED:{error}")))?
        .collect::<duckdb::Result<Vec<_>>>()
        .map_err(|error| AppError::DataParsing(format!("DUCKDB_SCHEMA_READ_FAILED:{error}")))?;
    if tables.len() > MAX_TABLES {
        return Err(AppError::DataParsing(format!(
            "DUCKDB_TABLE_COUNT_EXCEEDED:{MAX_TABLES}"
        )));
    }
    let mut output = "--- Database Engine: DuckDB ---\n--- Access Mode: READ_ONLY; External Access: DISABLED ---\n".to_string();
    let mut total_rows = 0_usize;
    for (schema_name, table_name, schema) in tables {
        append_output(
            &mut output,
            &format!(
                "--- Table: {}.{} ---\nSchema: {}\n",
                sanitize_field(&schema_name),
                sanitize_field(&table_name),
                sanitize_field(&schema)
            ),
            path,
        )?;
        let mut column_statement = connection
            .prepare(
                "SELECT column_name FROM duckdb_columns()
                 WHERE schema_name = ? AND table_name = ?
                 ORDER BY column_index",
            )
            .map_err(|error| AppError::DataParsing(format!("DUCKDB_COLUMN_READ_FAILED:{error}")))?;
        let column_names = column_statement
            .query_map([schema_name.as_str(), table_name.as_str()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| AppError::DataParsing(format!("DUCKDB_COLUMN_READ_FAILED:{error}")))?
            .collect::<duckdb::Result<Vec<_>>>()
            .map_err(|error| AppError::DataParsing(format!("DUCKDB_COLUMN_READ_FAILED:{error}")))?
            .into_iter()
            .map(|name| sanitize_field(&name))
            .collect::<Vec<_>>();
        append_output(&mut output, &column_names.join(","), path)?;
        append_output(&mut output, "\n", path)?;
        let sql = format!(
            "SELECT * FROM {}.{} LIMIT {}",
            quote_identifier(&schema_name),
            quote_identifier(&table_name),
            MAX_ROWS_PER_TABLE
        );
        let mut statement = connection.prepare(&sql).map_err(|error| {
            AppError::DataParsing(format!(
                "DUCKDB_TABLE_READ_FAILED:{}.{}:{error}",
                schema_name, table_name
            ))
        })?;
        let column_count = column_names.len();
        let mut rows = statement
            .query([])
            .map_err(|error| AppError::DataParsing(format!("DUCKDB_TABLE_READ_FAILED:{error}")))?;
        while let Some(row) = rows
            .next()
            .map_err(|error| AppError::DataParsing(format!("DUCKDB_TABLE_READ_FAILED:{error}")))?
        {
            total_rows = total_rows
                .checked_add(1)
                .ok_or_else(|| AppError::DataParsing("DUCKDB_ROW_COUNT_OVERFLOW".to_string()))?;
            if total_rows > MAX_TOTAL_ROWS {
                return Err(AppError::DataParsing(format!(
                    "DUCKDB_ROW_COUNT_EXCEEDED:{MAX_TOTAL_ROWS}"
                )));
            }
            for column in 0..column_count {
                if column > 0 {
                    append_output(&mut output, ",", path)?;
                }
                let value = row.get_ref(column).map_err(|error| {
                    AppError::DataParsing(format!("DUCKDB_VALUE_READ_FAILED:{error}"))
                })?;
                match value {
                    DuckValueRef::Null => {}
                    DuckValueRef::Boolean(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::TinyInt(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::SmallInt(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::Int(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::BigInt(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::HugeInt(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::UHugeInt(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::UTinyInt(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::USmallInt(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::UInt(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::UBigInt(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::Float(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::Double(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::Decimal(value) => {
                        append_output(&mut output, &value.to_string(), path)?
                    }
                    DuckValueRef::Text(value) => append_output(
                        &mut output,
                        &sanitize_field(&String::from_utf8_lossy(value)),
                        path,
                    )?,
                    DuckValueRef::Blob(value) | DuckValueRef::Geometry(value) => {
                        append_output(
                            &mut output,
                            &format!(
                                "[BINARY bytes={} sha256={}]",
                                value.len(),
                                digest_hex(Sha256::digest(value))
                            ),
                            path,
                        )?;
                    }
                    other => {
                        append_output(&mut output, &sanitize_field(&format!("{other:?}")), path)?
                    }
                }
            }
            append_output(&mut output, "\n", path)?;
        }
        append_output(&mut output, "\n", path)?;
    }
    if total_rows == 0 && output.lines().count() <= 2 {
        return Err(AppError::DataParsing(format!(
            "DUCKDB_OUTPUT_EMPTY: '{}' contains no extractable user tables",
            path.display()
        )));
    }
    Ok(output.trim().to_string())
}

pub fn extract_sql_dump_text<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    let path = file_path.as_ref();
    let bytes = read_regular_file(path, "SQL_DUMP", MAX_SQL_DUMP_BYTES)?;
    let text = std::str::from_utf8(&bytes).map_err(|error| {
        AppError::DataParsing(format!(
            "SQL_DUMP_UTF8_REQUIRED: '{}': {error}",
            path.display()
        ))
    })?;
    if text.contains('\0') {
        return Err(AppError::DataParsing(
            "SQL_DUMP_NUL_BYTE_REJECTED".to_string(),
        ));
    }
    let mut output = format!(
        "--- Database Engine: SQL dump (never executed) ---\n--- Source SHA-256: {} ---\n",
        digest_hex(Sha256::digest(&bytes))
    );
    for line in text.lines() {
        let line = line
            .chars()
            .map(|character| {
                if character == '\t' || !character.is_control() {
                    character
                } else {
                    ' '
                }
            })
            .collect::<String>();
        append_output(&mut output, &line, path)?;
        append_output(&mut output, "\n", path)?;
    }
    if !text.chars().any(|character| character.is_alphabetic()) {
        return Err(AppError::DataParsing("SQL_DUMP_TEXT_EMPTY".to_string()));
    }
    Ok(output.trim().to_string())
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

fn append_value(output: &mut String, value: ValueRef<'_>, path: &Path) -> Result<(), AppError> {
    match value {
        ValueRef::Null => Ok(()),
        ValueRef::Integer(value) => append_output(output, &value.to_string(), path),
        ValueRef::Real(value) => append_output(output, &value.to_string(), path),
        ValueRef::Text(value) => {
            let value = String::from_utf8_lossy(value);
            let normalized = sanitize_field(&value);
            append_output(output, &normalized, path)
        }
        ValueRef::Blob(value) => {
            append_output(output, "[BLOB bytes=", path)?;
            append_output(output, &value.len().to_string(), path)?;
            append_output(output, " sha256=", path)?;
            append_output(output, &digest_hex(Sha256::digest(value)), path)?;
            append_output(output, "]", path)
        }
    }
}

fn append_output(output: &mut String, value: &str, path: &Path) -> Result<(), AppError> {
    if output
        .len()
        .checked_add(value.len())
        .is_none_or(|size| size > MAX_OUTPUT_BYTES)
    {
        return Err(AppError::DataParsing(format!(
            "DATABASE_OUTPUT_SIZE_EXCEEDED: '{}' exceeds {MAX_OUTPUT_BYTES} extracted bytes",
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

    #[test]
    fn duckdb_extraction_is_read_only_and_preserves_engine_schema_and_values() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("fixture.duckdb");
        {
            let connection = duckdb::Connection::open(&path).expect("create DuckDB");
            connection
                .execute_batch(
                    "CREATE TABLE records(id BIGINT, label VARCHAR, payload BLOB);
                     INSERT INTO records VALUES (7, 'budget hearing', blob 'A1B2');",
                )
                .expect("fixture schema");
        }
        let before = std::fs::read(&path).expect("read before");
        assert_eq!(&before[8..12], b"DUCK", "DuckDB storage magic moved");
        let output = extract_duckdb_text(&path).expect("extract");
        let after = std::fs::read(&path).expect("read after");
        assert_eq!(
            before, after,
            "read-only extraction changed the source file"
        );
        assert!(output.contains("Database Engine: DuckDB"), "{output}");
        assert!(output.contains("Table: main.records"), "{output}");
        assert!(output.contains("budget hearing"), "{output}");
        assert!(output.contains("[BINARY bytes=4 sha256="), "{output}");
    }

    #[test]
    fn sql_dump_is_preserved_as_text_and_never_executed() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("fixture.sql");
        let source = "CREATE TABLE records(id INTEGER);\nINSERT INTO records VALUES (7);\n";
        std::fs::write(&path, source).expect("write dump");
        let output = extract_sql_dump_text(&path).expect("extract");
        assert!(output.contains("SQL dump (never executed)"));
        assert!(output.contains("CREATE TABLE records"));
        assert!(output.contains(&digest_hex(Sha256::digest(source.as_bytes()))));
    }
}
