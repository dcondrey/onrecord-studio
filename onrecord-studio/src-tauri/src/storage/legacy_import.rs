//! Detection and one-way conversion of a pre-DuckDB core database.
//!
//! Installations released before the storage migration hold `onrecord_studio_core.db` as a SQLite
//! file. DuckDB cannot open one, so without this module an upgraded application fails at startup
//! with an engine-level error and the investigator's corpus is unreachable.
//!
//! Conversion cost is linear at roughly 37 ms per MB of legacy database, dominated by the
//! full-row verification anti-join; see `tests/bench_migration.rs` for measurements. It is paid
//! once per installation, before the window is shown.
//!
//! The conversion never writes to the legacy file. It builds the DuckDB core beside it, copies
//! every table the two schemas share inside a single transaction, verifies row counts, and only
//! then swaps the new file into place. The legacy file is retained as a backup on every path,
//! including failure.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use duckdb::Connection;

use crate::error::AppError;

/// First 16 bytes of every SQLite 3 database file.
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// DuckDB writes `DUCK` at offset 8 of the first and second storage headers.
const DUCKDB_MAGIC: &[u8; 4] = b"DUCK";
const DUCKDB_MAGIC_OFFSET: usize = 8;
const HEADER_PROBE_BYTES: usize = 16;

/// Suffix appended to the retained pre-migration file.
const LEGACY_BACKUP_SUFFIX: &str = "legacy-sqlite.bak";
/// Suffix of the half-built core, removed on both success and failure.
const IN_PROGRESS_SUFFIX: &str = "migrating";

/// What, if anything, currently occupies the core database path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreDatabaseKind {
    /// Nothing exists yet; a fresh DuckDB core will be created.
    Missing,
    /// An existing DuckDB core that opens directly.
    DuckDb,
    /// A pre-migration SQLite core that must be converted first.
    LegacySqlite,
    /// A file whose header matches neither engine. Never overwritten.
    Unrecognized,
}

/// Outcome of a completed conversion, surfaced to startup telemetry.
#[derive(Debug, Clone)]
pub struct LegacyMigrationReport {
    /// Path of the retained pre-migration SQLite file.
    pub backup_path: PathBuf,
    /// Table name paired with the number of rows copied, in copy order.
    pub copied_tables: Vec<(String, u64)>,
    /// Tables present in the DuckDB schema that the legacy database did not have.
    pub absent_from_legacy: Vec<String>,
    /// Tables present in the legacy database that the DuckDB schema no longer defines.
    pub dropped_from_legacy: Vec<String>,
}

impl LegacyMigrationReport {
    /// Total rows carried across, used for the startup telemetry summary.
    pub fn total_rows(&self) -> u64 {
        self.copied_tables
            .iter()
            .map(|(_, rows)| *rows)
            .fold(0_u64, u64::saturating_add)
    }
}

/// Classifies the file at `path` by its header bytes, without opening a database engine.
pub fn classify_core_database(path: &Path) -> Result<CoreDatabaseKind, AppError> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CoreDatabaseKind::Missing)
        }
        Err(error) => {
            return Err(AppError::Storage(format!(
                "Failed to inspect core database '{}': {error}",
                path.display()
            )))
        }
    };

    let mut header = [0_u8; HEADER_PROBE_BYTES];
    let mut filled = 0;
    while filled < header.len() {
        match file.read(&mut header[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(AppError::Storage(format!(
                    "Failed to read core database header '{}': {error}",
                    path.display()
                )))
            }
        }
    }

    // A zero-length file is what a crashed first run leaves behind; treat it as absent so startup
    // recreates it rather than reporting an unrecognized engine.
    if filled == 0 {
        return Ok(CoreDatabaseKind::Missing);
    }
    if filled >= SQLITE_MAGIC.len() && &header[..SQLITE_MAGIC.len()] == SQLITE_MAGIC {
        return Ok(CoreDatabaseKind::LegacySqlite);
    }
    if filled >= DUCKDB_MAGIC_OFFSET + DUCKDB_MAGIC.len()
        && &header[DUCKDB_MAGIC_OFFSET..DUCKDB_MAGIC_OFFSET + DUCKDB_MAGIC.len()] == DUCKDB_MAGIC
    {
        return Ok(CoreDatabaseKind::DuckDb);
    }
    Ok(CoreDatabaseKind::Unrecognized)
}

/// Converts the legacy SQLite core at `core_path` into a DuckDB core at the same path.
///
/// On success the pre-migration file remains at `<core_path>.legacy-sqlite.bak`. On any failure the
/// original file is left exactly where it was and the partially built core is removed, so the
/// operation is safe to retry and never destroys evidence.
pub fn migrate_legacy_sqlite_core(
    core_path: &Path,
    migration_sql: &str,
) -> Result<LegacyMigrationReport, AppError> {
    let backup_path = sibling_path(core_path, LEGACY_BACKUP_SUFFIX);
    let in_progress_path = sibling_path(core_path, IN_PROGRESS_SUFFIX);

    if backup_path.exists() {
        return Err(AppError::Storage(format!(
            "A previous migration backup already exists at '{}'. Move or remove it before \
             retrying so the original database is never overwritten.",
            backup_path.display()
        )));
    }
    // A leftover from an interrupted attempt carries no committed data; the swap below is the only
    // step that publishes it, and that step never runs unless verification passed.
    remove_if_present(&in_progress_path)?;

    let result = build_converted_core(core_path, &in_progress_path, migration_sql);

    let report = match result {
        Ok(report) => report,
        Err(error) => {
            remove_if_present(&in_progress_path)?;
            return Err(error);
        }
    };

    // Publish only after verification succeeded: retain the original, then move the new core into
    // place. If the second rename fails the original is restored so startup can retry cleanly.
    fs::rename(core_path, &backup_path).map_err(|error| {
        AppError::Storage(format!(
            "Failed to retain the legacy database as '{}': {error}",
            backup_path.display()
        ))
    })?;
    if let Err(error) = fs::rename(&in_progress_path, core_path) {
        let restore = fs::rename(&backup_path, core_path);
        let _ = remove_if_present(&in_progress_path);
        return Err(AppError::Storage(match restore {
            Ok(()) => format!(
                "Failed to install the converted database at '{}': {error}. The original SQLite \
                 database was restored; no data was lost.",
                core_path.display()
            ),
            Err(restore_error) => format!(
                "Failed to install the converted database at '{}': {error}. Restoring the \
                 original also failed ({restore_error}); it remains at '{}'.",
                core_path.display(),
                backup_path.display()
            ),
        }));
    }

    Ok(LegacyMigrationReport {
        backup_path,
        ..report
    })
}

/// Creates the DuckDB schema at `target`, copies the shared tables from the legacy SQLite database,
/// and verifies every copied row count before returning.
fn build_converted_core(
    legacy_path: &Path,
    target: &Path,
    migration_sql: &str,
) -> Result<LegacyMigrationReport, AppError> {
    let connection = Connection::open(target).map_err(|error| {
        AppError::Storage(format!(
            "Failed to create the converted database '{}': {error}",
            target.display()
        ))
    })?;
    // The migration both defines the schema and seeds built-in rows (the CPRA statute and its
    // provisions). A legacy database from a prior installation already holds those rows, so seeding
    // before the copy guarantees a duplicate-key abort. They cannot simply be deleted afterwards
    // either: `legal_statutes` and `legal_statute_versions` reference each other, so no delete
    // order exists. Apply the schema now and the seeds after the copy -- all four seed statements
    // are idempotent (`OR IGNORE`, or guarded by `NOT EXISTS`), so they no-op when the legacy
    // database already supplied them.
    let (schema_sql, seed_sql) = split_schema_from_seeds(migration_sql);
    connection.execute_batch(&schema_sql).map_err(|error| {
        AppError::Storage(format!("Failed to build the DuckDB schema: {error}"))
    })?;

    load_sqlite_extension(&connection)?;
    connection
        .execute_batch(&format!(
            "ATTACH {} AS legacy (TYPE SQLITE, READ_ONLY);",
            quote_sql_literal(legacy_path)?
        ))
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to open the legacy database '{}' for reading: {error}",
                legacy_path.display()
            ))
        })?;

    // The target's catalog name is derived from its file name, not the literal "main"; only the
    // schema inside it is called that. Ask the connection which catalog it is currently in.
    let target_catalog: String = connection
        .query_row("SELECT current_database()", [], |row| row.get(0))
        .map_err(|error| {
            AppError::Storage(format!("Failed to resolve the target catalog: {error}"))
        })?;
    let target_tables = list_tables(&connection, &target_catalog)?;
    let legacy_tables = list_tables(&connection, "legacy")?;

    // Foreign keys are enforced during the copy, so a child table inserted before its parent
    // aborts the whole transaction. Order the copy by the target schema's own dependency graph.
    let shared = order_by_foreign_keys(
        target_tables
            .iter()
            .filter(|table| legacy_tables.contains(*table))
            .cloned()
            .collect(),
        &foreign_key_dependencies(&connection, &target_catalog)?,
    );
    let absent_from_legacy = target_tables
        .iter()
        .filter(|table| !legacy_tables.contains(*table))
        .cloned()
        .collect();
    let dropped_from_legacy = legacy_tables
        .iter()
        .filter(|table| !target_tables.contains(*table))
        .cloned()
        .collect();

    // One transaction for the whole corpus: a failure part-way leaves the target empty rather than
    // half-populated, and the target is discarded by the caller regardless.
    connection
        .execute_batch("BEGIN TRANSACTION;")
        .map_err(|error| AppError::Storage(format!("Failed to begin the copy: {error}")))?;

    let mut copied_tables = Vec::with_capacity(shared.len());
    for table in &shared {
        match copy_table(&connection, table) {
            Ok(rows) => copied_tables.push((table.clone(), rows)),
            Err(error) => {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
    }

    // Verify inside the transaction, before commit, so a mismatch can still be rolled back.
    for (table, _) in &copied_tables {
        if let Err(error) = verify_table_transferred(&connection, table) {
            let _ = connection.execute_batch("ROLLBACK;");
            return Err(error);
        }
    }

    if let Err(error) = connection.execute_batch(&seed_sql) {
        let _ = connection.execute_batch("ROLLBACK;");
        return Err(AppError::Storage(format!(
            "Failed to apply built-in reference data after the copy: {error}"
        )));
    }

    connection
        .execute_batch("COMMIT;")
        .map_err(|error| AppError::Storage(format!("Failed to commit the copy: {error}")))?;
    connection
        .execute_batch("DETACH legacy;")
        .map_err(|error| {
            AppError::Storage(format!("Failed to close the legacy source: {error}"))
        })?;
    // Force the pages out before the caller renames the file into place.
    connection
        .execute_batch("CHECKPOINT;")
        .map_err(|error| AppError::Storage(format!("Failed to checkpoint the copy: {error}")))?;
    drop(connection);

    Ok(LegacyMigrationReport {
        backup_path: PathBuf::new(),
        copied_tables,
        absent_from_legacy,
        dropped_from_legacy,
    })
}

/// Copies one table by column name, returning the number of rows read from the legacy database.
///
/// `BY NAME` matches on column name rather than position, so a legacy table whose columns were
/// added in a different order, or which lacks a column added after it was written, still
/// transfers; columns the legacy table does not have take their schema default.
///
fn copy_table(connection: &Connection, table: &str) -> Result<u64, AppError> {
    let quoted = quote_identifier(table);
    let expected = count_rows(connection, "legacy", table)?;
    connection
        .execute_batch(&format!(
            "INSERT INTO main.{quoted} BY NAME SELECT * FROM legacy.{quoted};"
        ))
        .map_err(|error| AppError::Storage(format!("Failed to copy table '{table}': {error}")))?;
    Ok(expected)
}

/// Confirms every row in the legacy table is present in the converted table.
///
/// This is checked instead of comparing row counts, because the converted database legitimately
/// holds seeded rows the legacy database may not have. Presence of every legacy row is the
/// guarantee that actually matters: no record the investigator collected was dropped.
fn verify_table_transferred(connection: &Connection, table: &str) -> Result<(), AppError> {
    let quoted = quote_identifier(table);
    let columns = legacy_columns(connection, table)?;
    if columns.is_empty() {
        return Ok(());
    }
    let projection = columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let missing: i64 = connection
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM (
                    SELECT {projection} FROM legacy.{quoted}
                    EXCEPT
                    SELECT {projection} FROM main.{quoted}
                 )"
            ),
            [],
            |row| row.get(0),
        )
        .map_err(|error| AppError::Storage(format!("Failed to verify table '{table}': {error}")))?;
    if missing != 0 {
        return Err(AppError::Storage(format!(
            "Migration verification failed for table '{table}': {missing} distinct rows from the \
             original database are missing from the converted one. The original was not modified."
        )));
    }
    Ok(())
}

/// Column names of one legacy table, in declaration order.
fn legacy_columns(connection: &Connection, table: &str) -> Result<Vec<String>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT column_name
             FROM information_schema.columns
             WHERE table_catalog = 'legacy' AND table_schema = 'main' AND table_name = ?1
             ORDER BY ordinal_position",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to read columns of '{table}': {error}"))
        })?;
    let rows = statement
        .query_map([table], |row| row.get::<_, String>(0))
        .map_err(|error| {
            AppError::Storage(format!("Failed to list columns of '{table}': {error}"))
        })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
        AppError::Storage(format!("Failed to decode columns of '{table}': {error}"))
    })
}

fn count_rows(connection: &Connection, schema: &str, table: &str) -> Result<u64, AppError> {
    let quoted = quote_identifier(table);
    let count: i64 = connection
        .query_row(
            &format!("SELECT COUNT(*) FROM {schema}.{quoted}"),
            [],
            |row| row.get(0),
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to count rows in '{schema}.{table}': {error}"
            ))
        })?;
    u64::try_from(count).map_err(|_| {
        AppError::Storage(format!(
            "Table '{schema}.{table}' reported a negative row count"
        ))
    })
}

/// Reads `(table, referenced_table)` pairs for every foreign key defined in one catalog.
fn foreign_key_dependencies(
    connection: &Connection,
    database: &str,
) -> Result<Vec<(String, String)>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT table_name, referenced_table
             FROM duckdb_constraints()
             WHERE database_name = ?1
               AND schema_name = 'main'
               AND constraint_type = 'FOREIGN KEY'
               AND referenced_table IS NOT NULL",
        )
        .map_err(|error| AppError::Storage(format!("Failed to enumerate foreign keys: {error}")))?;
    let rows = statement
        .query_map([database], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| AppError::Storage(format!("Failed to read foreign keys: {error}")))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Storage(format!("Failed to decode foreign keys: {error}")))
}

/// Orders `tables` so every table follows the tables it references.
///
/// A self-reference is ignored: DuckDB validates a foreign key at statement end, so a table whose
/// parent rows arrive in the same `INSERT` is satisfied. Tables left over from a reference cycle
/// are appended in their original order; the copy then surfaces a genuine constraint error, which
/// the caller rolls back without touching the original database.
fn order_by_foreign_keys(tables: Vec<String>, dependencies: &[(String, String)]) -> Vec<String> {
    let mut remaining = tables;
    let mut ordered: Vec<String> = Vec::with_capacity(remaining.len());

    while !remaining.is_empty() {
        let ready: Vec<String> = remaining
            .iter()
            .filter(|table| {
                dependencies.iter().all(|(child, parent)| {
                    child != *table || parent == *table || !remaining.contains(parent)
                })
            })
            .cloned()
            .collect();
        if ready.is_empty() {
            ordered.append(&mut remaining);
            break;
        }
        remaining.retain(|table| !ready.contains(table));
        ordered.extend(ready);
    }
    ordered
}

/// Base-table names in the `main` schema of one attached catalog, excluding views.
fn list_tables(connection: &Connection, database: &str) -> Result<Vec<String>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT table_name
             FROM information_schema.tables
             WHERE table_catalog = ?1
               AND table_schema = 'main'
               AND table_type = 'BASE TABLE'
             ORDER BY table_name",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to enumerate '{database}' tables: {error}"))
        })?;
    let rows = statement
        .query_map([database], |row| row.get::<_, String>(0))
        .map_err(|error| {
            AppError::Storage(format!("Failed to read '{database}' tables: {error}"))
        })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
        AppError::Storage(format!("Failed to decode '{database}' tables: {error}"))
    })
}

/// Splits a migration script into its schema statements and its seed `INSERT`s.
///
/// Statement boundaries respect single-quoted literals and `--` line comments, so a semicolon
/// inside a string or a comment does not split a statement. Anything that is not an `INSERT` at
/// the top level is treated as schema.
fn split_schema_from_seeds(migration_sql: &str) -> (String, String) {
    let mut schema = String::with_capacity(migration_sql.len());
    let mut seeds = String::new();
    let mut statement = String::new();
    let mut in_string = false;
    let mut in_line_comment = false;
    let mut characters = migration_sql.chars().peekable();

    while let Some(character) = characters.next() {
        if in_line_comment {
            statement.push(character);
            if character == '\n' {
                in_line_comment = false;
            }
            continue;
        }
        if in_string {
            statement.push(character);
            if character == '\'' {
                // A doubled quote is an escaped quote, not the end of the literal.
                if characters.peek() == Some(&'\'') {
                    statement.push(characters.next().unwrap_or('\''));
                } else {
                    in_string = false;
                }
            }
            continue;
        }
        match character {
            '\'' => {
                in_string = true;
                statement.push(character);
            }
            '-' if characters.peek() == Some(&'-') => {
                in_line_comment = true;
                statement.push(character);
            }
            ';' => {
                statement.push(character);
                push_statement(&mut statement, &mut schema, &mut seeds);
            }
            _ => statement.push(character),
        }
    }
    push_statement(&mut statement, &mut schema, &mut seeds);
    (schema, seeds)
}

/// Routes one completed statement to the schema or seed script, then clears the buffer.
fn push_statement(statement: &mut String, schema: &mut String, seeds: &mut String) {
    let is_seed = statement
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("--"))
        .is_some_and(|line| line.len() >= 6 && line[..6].eq_ignore_ascii_case("INSERT"));
    if is_seed {
        seeds.push_str(statement);
    } else {
        schema.push_str(statement);
    }
    statement.clear();
}

fn load_sqlite_extension(connection: &Connection) -> Result<(), AppError> {
    if connection.execute_batch("LOAD sqlite;").is_ok() {
        return Ok(());
    }
    connection
        .execute_batch("INSTALL sqlite; LOAD sqlite;")
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to load the DuckDB SQLite reader required to upgrade the existing \
                 database: {error}"
            ))
        })
}

fn remove_if_present(path: &Path) -> Result<(), AppError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Storage(format!(
            "Failed to remove '{}': {error}",
            path.display()
        ))),
    }
}

/// Appends `suffix` to the file name so the sibling shares the core's directory and permissions.
fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_sql_literal(path: &Path) -> Result<String, AppError> {
    let value = path.to_str().ok_or_else(|| {
        AppError::DataParsing(format!("Database path '{}' is not UTF-8", path.display()))
    })?;
    if value.contains('\0') {
        return Err(AppError::DataParsing(
            "Database path contains a NUL character".to_string(),
        ));
    }
    Ok(format!("'{}'", value.replace('\'', "''")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIGRATION: &str = include_str!("../../migrations/0001_initial_schema.sql");

    fn legacy_sqlite_core(path: &Path) {
        let connection = rusqlite::Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE portals (
                    subdomain TEXT PRIMARY KEY, display_name TEXT NOT NULL, base_url TEXT NOT NULL
                 );
                 CREATE TABLE agencies (
                    id INTEGER, portal_subdomain TEXT, name TEXT NOT NULL,
                    PRIMARY KEY (id, portal_subdomain)
                 );
                 CREATE TABLE requests (
                    id INTEGER, portal_subdomain TEXT, title TEXT, description TEXT,
                    status TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                    closed_at TEXT, agency_id INTEGER,
                    PRIMARY KEY (id, portal_subdomain)
                 );
                 -- A table the current schema no longer defines; it must be reported, not copied.
                 CREATE TABLE retired_scratch (id INTEGER PRIMARY KEY, note TEXT);
                 INSERT INTO portals VALUES ('city-a', 'City A', 'https://city-a.example');
                 INSERT INTO agencies VALUES (1, 'city-a', 'Records Division');
                 INSERT INTO requests VALUES
                    (10, 'city-a', 'Body camera policy', 'desc', 'closed',
                     '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', '2026-01-03T00:00:00Z', 1),
                    (11, 'city-a', 'Overtime ledger', 'desc', 'open',
                     '2026-02-01T00:00:00Z', '2026-02-02T00:00:00Z', NULL, 1);
                 INSERT INTO retired_scratch VALUES (1, 'obsolete');",
            )
            .unwrap();
    }

    #[test]
    fn classifies_sqlite_duckdb_missing_and_unrecognized_headers() {
        let directory = tempfile::tempdir().unwrap();

        let missing = directory.path().join("absent.db");
        assert_eq!(
            classify_core_database(&missing).unwrap(),
            CoreDatabaseKind::Missing
        );

        let empty = directory.path().join("empty.db");
        fs::write(&empty, b"").unwrap();
        assert_eq!(
            classify_core_database(&empty).unwrap(),
            CoreDatabaseKind::Missing,
            "a zero-length file from a crashed first run must be recreated, not rejected"
        );

        let legacy = directory.path().join("legacy.db");
        legacy_sqlite_core(&legacy);
        assert_eq!(
            classify_core_database(&legacy).unwrap(),
            CoreDatabaseKind::LegacySqlite
        );

        let duck = directory.path().join("duck.db");
        let connection = Connection::open(&duck).unwrap();
        connection
            .execute_batch("CREATE TABLE t(x INTEGER);")
            .unwrap();
        drop(connection);
        assert_eq!(
            classify_core_database(&duck).unwrap(),
            CoreDatabaseKind::DuckDb
        );

        let junk = directory.path().join("junk.db");
        fs::write(&junk, b"this is not a database file at all").unwrap();
        assert_eq!(
            classify_core_database(&junk).unwrap(),
            CoreDatabaseKind::Unrecognized,
            "an unrecognized file must never be treated as convertible"
        );
    }

    #[test]
    fn converts_a_legacy_core_and_retains_the_original() {
        let directory = tempfile::tempdir().unwrap();
        let core = directory.path().join("onrecord_studio_core.db");
        legacy_sqlite_core(&core);

        let report = migrate_legacy_sqlite_core(&core, MIGRATION).unwrap();

        assert_eq!(
            classify_core_database(&core).unwrap(),
            CoreDatabaseKind::DuckDb,
            "the core path must hold a DuckDB database after conversion"
        );
        assert_eq!(
            classify_core_database(&report.backup_path).unwrap(),
            CoreDatabaseKind::LegacySqlite,
            "the pre-migration database must be retained unmodified"
        );

        let copied: std::collections::HashMap<_, _> =
            report.copied_tables.iter().cloned().collect();
        assert_eq!(copied.get("requests"), Some(&2));
        assert_eq!(copied.get("agencies"), Some(&1));
        assert_eq!(copied.get("portals"), Some(&1));
        assert!(
            report
                .dropped_from_legacy
                .contains(&"retired_scratch".to_string()),
            "a legacy-only table must be reported rather than silently discarded"
        );
        assert!(
            report.absent_from_legacy.contains(&"documents".to_string()),
            "a table the legacy database never had must be reported as newly created"
        );

        // The converted rows must be readable through the ordinary repository connection.
        let connection = Connection::open(&core).unwrap();
        let titles: Vec<String> = connection
            .prepare("SELECT title FROM requests ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(titles, vec!["Body camera policy", "Overtime ledger"]);
    }

    /// The three-table fixture above cannot exercise the real dependency graph: the production
    /// schema declares 28 foreign-key references across 27 tables. This builds a legacy database
    /// holding the *whole* schema, populated along the deepest reference chain
    /// (`portals -> agencies -> requests -> documents -> provenance_manifests ->
    /// provenance_assertions`), and asserts the conversion completes.
    ///
    /// The failure this guards is fatal rather than cosmetic: `migrate_legacy_sqlite_core(...)?`
    /// propagates out of `configure_onrecord_studio_runtime`, so a copy-ordering or constraint
    /// error means the upgraded application refuses to start at all.
    #[test]
    fn converts_a_legacy_core_holding_the_complete_production_schema() {
        let directory = tempfile::tempdir().unwrap();
        let core = directory.path().join("core.db");

        // Build the fixture through DuckDB so the legacy file carries the real schema's table set
        // and column names rather than a hand-transcribed approximation, then write it out as a
        // genuine SQLite file.
        let staging = Connection::open_in_memory().unwrap();
        staging.execute_batch(MIGRATION).unwrap();
        staging
            .execute_batch(
                "INSERT INTO portals (subdomain, display_name, base_url)
                 VALUES ('city-a', 'City A', 'https://city-a.example');
                 INSERT INTO agencies (id, portal_subdomain, name)
                 VALUES (1, 'city-a', 'Records Division');
                 INSERT INTO categories (id, portal_subdomain, name)
                 VALUES (1, 'city-a', 'Enforcement');
                 INSERT INTO requests (
                    id, portal_subdomain, title, description, status,
                    created_at, updated_at, closed_at, agency_id, category_id
                 ) VALUES
                    (10, 'city-a', 'Body camera policy', 'd', 'closed',
                     '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', '2026-01-03T00:00:00Z', 1, 1),
                    -- A row with NULLs in every nullable column: a legacy database that predates a
                    -- later NOT NULL would fail the copy here rather than in the field.
                    (11, 'city-a', NULL, NULL, 'open',
                     '2026-02-01T00:00:00Z', '2026-02-02T00:00:00Z', NULL, NULL, NULL);
                 INSERT INTO documents (
                    id, request_id, portal_subdomain, file_name, visibility, created_at, updated_at
                 ) VALUES (100, 10, 'city-a', 'release.pdf', 'public',
                           '2026-01-02T00:00:00Z', '2026-01-02T00:00:00Z');
                 INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
                 VALUES (100, 'city-a', 'release.pdf', 'body worn camera retention schedule');",
            )
            .unwrap();

        load_sqlite_extension(&staging).unwrap();
        staging
            .execute_batch(&format!(
                "ATTACH {} AS legacy (TYPE SQLITE);",
                quote_sql_literal(&core).unwrap()
            ))
            .unwrap();
        let tables = list_tables(&staging, "memory").unwrap();
        assert!(
            tables.len() >= 20,
            "the fixture must cover the real schema, saw {} tables",
            tables.len()
        );
        for table in &tables {
            let quoted = quote_identifier(table);
            staging
                .execute_batch(&format!(
                    "CREATE TABLE legacy.{quoted} AS SELECT * FROM main.{quoted};"
                ))
                .unwrap_or_else(|error| panic!("fixture table '{table}' must transfer: {error}"));
        }
        staging.execute_batch("DETACH legacy;").unwrap();
        drop(staging);

        assert_eq!(
            classify_core_database(&core).unwrap(),
            CoreDatabaseKind::LegacySqlite
        );

        let report = migrate_legacy_sqlite_core(&core, MIGRATION).unwrap();

        assert!(
            report.dropped_from_legacy.is_empty(),
            "the fixture was built from the current schema, so nothing should be dropped: {:?}",
            report.dropped_from_legacy
        );
        assert_eq!(
            report.copied_tables.len(),
            tables.len(),
            "every table in the legacy database must be copied"
        );

        // `requests` must not precede `portals`/`agencies`/`categories` in the copy order, and
        // `documents` must not precede `requests`.
        let position = |name: &str| {
            report
                .copied_tables
                .iter()
                .position(|(table, _)| table == name)
                .unwrap_or_else(|| panic!("'{name}' must have been copied"))
        };
        // Only single-column foreign keys constrain the copy order. DuckDB catalogues all 28 of
        // this schema's references but does not enforce the composite ones -- an orphan row in
        // `documents(request_id, portal_subdomain)` inserts without error -- so asserting an order
        // for those would pin behaviour the engine does not actually provide.
        assert!(position("portals") < position("agencies"));
        assert!(position("legal_statutes") < position("legal_mandates"));

        let converted = Connection::open(&core).unwrap();
        let requests: i64 = converted
            .query_row("SELECT COUNT(*) FROM requests", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            requests, 2,
            "both rows, including the all-NULL one, survive"
        );
        let text: String = converted
            .query_row(
                "SELECT extracted_text FROM documents_fts WHERE document_id = 100",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(text, "body worn camera retention schedule");
    }

    #[test]
    fn refuses_to_run_when_a_previous_backup_would_be_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let core = directory.path().join("core.db");
        legacy_sqlite_core(&core);
        let backup = sibling_path(&core, LEGACY_BACKUP_SUFFIX);
        fs::write(&backup, b"an earlier backup").unwrap();

        let error = migrate_legacy_sqlite_core(&core, MIGRATION).unwrap_err();
        assert!(
            error.to_string().contains("already exists"),
            "unexpected error: {error}"
        );
        assert_eq!(
            fs::read(&backup).unwrap(),
            b"an earlier backup",
            "the existing backup must not be touched"
        );
        assert_eq!(
            classify_core_database(&core).unwrap(),
            CoreDatabaseKind::LegacySqlite,
            "the original database must be left in place when the migration refuses to run"
        );
    }

    #[test]
    fn leaves_the_original_in_place_when_the_copy_fails() {
        let directory = tempfile::tempdir().unwrap();
        let core = directory.path().join("core.db");
        legacy_sqlite_core(&core);

        // A schema whose column type cannot accept the legacy values forces the copy to fail after
        // the target has been created, exercising the rollback path.
        let error = migrate_legacy_sqlite_core(
            &core,
            "CREATE TABLE requests (id BIGINT, portal_subdomain BIGINT);",
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("requests"),
            "unexpected: {error}"
        );

        assert_eq!(
            classify_core_database(&core).unwrap(),
            CoreDatabaseKind::LegacySqlite,
            "a failed conversion must leave the original database exactly where it was"
        );
        assert!(
            !sibling_path(&core, IN_PROGRESS_SUFFIX).exists(),
            "the half-built core must be removed"
        );
        assert!(
            !sibling_path(&core, LEGACY_BACKUP_SUFFIX).exists(),
            "no backup should be published when the conversion never completed"
        );
    }
}
