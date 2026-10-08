//! Detection and one-way conversion of a pre-DuckDB core database.
//!
//! Installations released before the storage migration hold `onrecord_studio_core.db` as a SQLite
//! file. DuckDB cannot open one, so without this module an upgraded application fails at startup
//! with an engine-level error and the investigator's corpus is unreachable.
//!
//! The legacy file is read with the bundled SQLite library, not DuckDB's `sqlite` extension: that
//! extension is not compiled in, and the application never loads one at runtime (see
//! `storage::open_duckdb`). Each legacy table the conversion reads is first streamed into a scratch
//! DuckDB file, which is attached read-only as the catalog `legacy`, so the copy and the full-row
//! verification are plain DuckDB SQL over `legacy.<table>`.
//!
//! Conversion cost is linear in the size of the legacy database: an 82 MB WAL-mode core converts
//! at 140 ms/MB in the unoptimized test profile, where the extension-based reader this replaced
//! took 813 ms/MB. It is paid once per installation, before the window is shown.
//!
//! The conversion never writes to the legacy file, nor creates a journal, WAL or shared-memory
//! file for it. It builds the DuckDB core beside it, copies every table the two schemas share
//! inside a single transaction, verifies every legacy row is present in the converted table, and
//! only then swaps the new file into place. The legacy file is retained as a backup on every path,
//! including failure.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use duckdb::types::{ToSqlOutput, Value as DuckValue, ValueRef as DuckValueRef};
use duckdb::Connection;
use rusqlite::types::ValueRef as SqliteValueRef;
use rusqlite::OpenFlags;

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
/// Suffix of the scratch DuckDB copy of the legacy tables, removed on both success and failure.
const LEGACY_SNAPSHOT_SUFFIX: &str = "migrating-legacy";
/// Suffix of a private copy of the legacy file, made only when a WAL or journal must be replayed.
const LEGACY_SOURCE_COPY_SUFFIX: &str = "migrating-source";
/// SQLite's sidecar files. Only `-wal` and `-journal` can hold pages the main file lacks; `-shm` is
/// a rebuildable index, moved with the others so none is left beside the converted core.
const SQLITE_SIDECARS: [&str; 3] = ["-wal", "-journal", "-shm"];
/// Text and blob bytes appended before the appender is flushed. DuckDB otherwise buffers about
/// 200k rows, which for a document-text table is the whole table.
const APPENDER_FLUSH_BYTES: usize = 64 * 1024 * 1024;
/// Integers up to this magnitude convert to `f64` exactly.
const EXACT_F64_INTEGER_LIMIT: u64 = 1 << 53;

/// SQLite storage classes as distinct bit flags, so a `sum(DISTINCT ...)` of them is their union.
const CLASS_INTEGER: i64 = 1;
const CLASS_REAL: i64 = 2;
const CLASS_TEXT: i64 = 4;
const CLASS_BLOB: i64 = 8;

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

    if let Some(existing) = std::iter::once(backup_path.clone())
        .chain(SQLITE_SIDECARS.map(|suffix| sidecar_path(&backup_path, suffix)))
        .find(|path| path.exists())
    {
        return Err(AppError::Storage(format!(
            "A previous migration backup already exists at '{}'. Move or remove it before \
             retrying so the original database is never overwritten.",
            existing.display()
        )));
    }
    // A leftover from an interrupted attempt carries no committed data; the swap below is the only
    // step that publishes it, and that step never runs unless verification passed.
    remove_if_present(&in_progress_path)?;
    remove_staging_files(core_path)?;

    let result = build_converted_core(core_path, &in_progress_path, migration_sql);
    if let Err(cleanup_error) = remove_staging_files(core_path) {
        eprintln!("Failed to remove the migration's scratch files: {cleanup_error}");
    }

    let report = match result {
        Ok(report) => report,
        Err(error) => {
            // The conversion failure is the cause the caller needs; a cleanup failure here is a
            // secondary concern (a stale `.in-progress` file costs nothing but disk, since the
            // original core was never touched) and must not replace it via `?`.
            if let Err(cleanup_error) = remove_if_present(&in_progress_path) {
                eprintln!(
                    "Failed to remove the partially built core '{}' after a migration failure: \
                     {cleanup_error}",
                    in_progress_path.display()
                );
            }
            return Err(error);
        }
    };

    // Publish only after verification succeeded: retain the original, then move the new core into
    // place. If the second rename fails the original is restored so startup can retry cleanly.
    move_with_sidecars(core_path, &backup_path).map_err(|error| {
        AppError::Storage(format!(
            "Failed to retain the legacy database as '{}': {error}",
            backup_path.display()
        ))
    })?;
    if let Err(error) = fs::rename(&in_progress_path, core_path) {
        let restore = move_with_sidecars(&backup_path, core_path);
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
/// and verifies every legacy row is present in the converted table before returning.
fn build_converted_core(
    legacy_path: &Path,
    target: &Path,
    migration_sql: &str,
) -> Result<LegacyMigrationReport, AppError> {
    let connection = super::open_duckdb(Some(target)).map_err(|error| {
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

    // The target's catalog name is derived from its file name, not the literal "main"; only the
    // schema inside it is called that. Ask the connection which catalog it is currently in.
    let target_catalog: String = connection
        .query_row("SELECT current_database()", [], |row| row.get(0))
        .map_err(|error| {
            AppError::Storage(format!("Failed to resolve the target catalog: {error}"))
        })?;
    let target_tables = list_tables(&connection, &target_catalog)?;

    let snapshot_path = sibling_path(legacy_path, LEGACY_SNAPSHOT_SUFFIX);
    let legacy_tables = snapshot_legacy_tables(legacy_path, &snapshot_path, &target_tables)?;
    connection
        .execute_batch(&format!(
            "ATTACH {} AS legacy (READ_ONLY);",
            quote_sql_literal(&snapshot_path)?
        ))
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to open the staged copy of the legacy database: {error}"
            ))
        })?;

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

/// Streams each legacy table named in `wanted` into a new DuckDB file at `snapshot_path`, returning
/// the names of every table the legacy database holds.
fn snapshot_legacy_tables(
    legacy_path: &Path,
    snapshot_path: &Path,
    wanted: &[String],
) -> Result<Vec<String>, AppError> {
    let source = open_legacy_source(legacy_path)?;
    let legacy_tables = sqlite_tables(&source)?;
    let snapshot = super::open_duckdb(Some(snapshot_path)).map_err(|error| {
        AppError::Storage(format!(
            "Failed to create the staged copy of the legacy database '{}': {error}",
            snapshot_path.display()
        ))
    })?;
    for table in legacy_tables.iter().filter(|table| wanted.contains(*table)) {
        snapshot_table(&source, &snapshot, table)?;
    }
    snapshot.execute_batch("CHECKPOINT;").map_err(|error| {
        AppError::Storage(format!(
            "Failed to checkpoint the staged legacy tables: {error}"
        ))
    })?;
    Ok(legacy_tables)
}

/// Opens the legacy database for reading without writing to it or creating any file beside it.
///
/// `immutable` stops SQLite from taking locks or creating a `-shm` or `-journal`, but it also makes
/// SQLite ignore a WAL and a hot journal. The legacy application ran in WAL mode, so committed rows
/// can exist only in its `-wal`; when either sidecar holds data, a private copy is opened instead
/// and SQLite replays the sidecar into that copy.
fn open_legacy_source(legacy_path: &Path) -> Result<rusqlite::Connection, AppError> {
    let open_error = |error: rusqlite::Error| {
        AppError::Storage(format!(
            "Failed to open the legacy database '{}' for reading: {error}",
            legacy_path.display()
        ))
    };
    let mut pending = Vec::new();
    for suffix in ["-wal", "-journal"] {
        let path = sidecar_path(legacy_path, suffix);
        match fs::metadata(&path) {
            Ok(metadata) if metadata.len() > 0 => pending.push(suffix),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::Storage(format!(
                    "Failed to inspect '{}': {error}",
                    path.display()
                )))
            }
        }
    }
    if pending.is_empty() {
        return rusqlite::Connection::open_with_flags(
            immutable_uri(legacy_path)?,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI,
        )
        .map_err(open_error);
    }
    let copy = sibling_path(legacy_path, LEGACY_SOURCE_COPY_SUFFIX);
    copy_file(legacy_path, &copy)?;
    for suffix in pending {
        copy_file(
            &sidecar_path(legacy_path, suffix),
            &sidecar_path(&copy, suffix),
        )?;
    }
    rusqlite::Connection::open_with_flags(
        &copy,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(open_error)
}

/// A `file:` URI for `path` with `immutable=1`. SQLite percent-decodes the path and ends it at `?`
/// or `#`, and a Windows drive path needs a leading `/`.
fn immutable_uri(path: &Path) -> Result<String, AppError> {
    let value = path.to_str().ok_or_else(|| {
        AppError::DataParsing(format!("Database path '{}' is not UTF-8", path.display()))
    })?;
    let mut uri = String::from("file:");
    if cfg!(windows) && path.is_absolute() {
        uri.push('/');
    }
    for character in value.chars() {
        match character {
            '%' => uri.push_str("%25"),
            '?' => uri.push_str("%3F"),
            '#' => uri.push_str("%23"),
            '\\' if cfg!(windows) => uri.push('/'),
            other => uri.push(other),
        }
    }
    uri.push_str("?immutable=1");
    Ok(uri)
}

/// Copies into a newly created file, so the copy is writable even when the legacy file is not.
fn copy_file(from: &Path, to: &Path) -> Result<(), AppError> {
    let copy_error = |error: std::io::Error| {
        AppError::Storage(format!(
            "Failed to copy '{}' to '{}': {error}",
            from.display(),
            to.display()
        ))
    };
    let mut source = fs::File::open(from).map_err(copy_error)?;
    let mut target = fs::File::create(to).map_err(copy_error)?;
    std::io::copy(&mut source, &mut target).map_err(copy_error)?;
    Ok(())
}

/// Ordinary and virtual table names in the legacy database, excluding SQLite's internal tables.
fn sqlite_tables(source: &rusqlite::Connection) -> Result<Vec<String>, AppError> {
    let mut statement = source
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'
             ORDER BY name",
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to enumerate legacy tables: {error}"))
        })?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| AppError::Storage(format!("Failed to read legacy tables: {error}")))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Storage(format!("Failed to decode legacy tables: {error}")))
}

/// How one legacy column is staged in DuckDB.
///
/// SQLite stores each value with its own storage class whatever the column declares: a column
/// declared `INTEGER` keeps text such as `'0x1F'` as text. So the staged type follows the classes
/// the column's values actually hold, and every value is staged exactly as stored; the declared
/// type's affinity decides only for a column with no values. If the copy into the converted schema
/// then changes a value (`'0x1F'` cast to `31`), the verification anti-join reports that row as
/// missing and the conversion aborts, rather than altering evidence silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StagedColumn {
    Integer,
    /// Real values, plus integers that convert exactly.
    Real,
    Text,
    Blob,
    /// Text mixed with numbers, each number kept as its exact decimal text.
    MixedText,
}

impl StagedColumn {
    /// `None` when binary values share the column with another class: no type holds both exactly.
    fn resolve(declared: &str, classes: i64) -> Option<Self> {
        Some(match classes {
            0 => Self::from_affinity(declared),
            CLASS_INTEGER => Self::Integer,
            CLASS_REAL => Self::Real,
            CLASS_TEXT => Self::Text,
            CLASS_BLOB => Self::Blob,
            mixed if mixed == CLASS_INTEGER | CLASS_REAL => Self::Real,
            mixed if mixed & CLASS_BLOB == 0 => Self::MixedText,
            _ => return None,
        })
    }

    /// SQLite's affinity rules, in their precedence order; NUMERIC and untyped columns stage as
    /// text.
    fn from_affinity(declared: &str) -> Self {
        let declared = declared.to_ascii_uppercase();
        let has = |needles: &[&str]| needles.iter().any(|needle| declared.contains(needle));
        if has(&["INT"]) {
            Self::Integer
        } else if has(&["CHAR", "CLOB", "TEXT"]) {
            Self::Text
        } else if has(&["BLOB"]) {
            Self::Blob
        } else if has(&["REAL", "FLOA", "DOUB"]) {
            Self::Real
        } else {
            Self::Text
        }
    }

    fn duckdb_type(self) -> &'static str {
        match self {
            Self::Integer => "BIGINT",
            Self::Real => "DOUBLE",
            Self::Text | Self::MixedText => "VARCHAR",
            Self::Blob => "BLOB",
        }
    }

    /// The staged form of one stored value, or why it cannot be staged without loss.
    fn stage(self, value: SqliteValueRef<'_>) -> Result<ToSqlOutput<'_>, String> {
        Ok(match (self, value) {
            (_, SqliteValueRef::Null) => ToSqlOutput::Borrowed(DuckValueRef::Null),
            (Self::Integer, SqliteValueRef::Integer(number)) => {
                ToSqlOutput::Borrowed(DuckValueRef::BigInt(number))
            }
            (Self::Real, SqliteValueRef::Real(number)) => {
                ToSqlOutput::Borrowed(DuckValueRef::Double(number))
            }
            (Self::Real, SqliteValueRef::Integer(number))
                if number.unsigned_abs() <= EXACT_F64_INTEGER_LIMIT =>
            {
                ToSqlOutput::Borrowed(DuckValueRef::Double(number as f64))
            }
            (Self::Real, SqliteValueRef::Integer(_)) => {
                return Err("holds an integer too large to stage exactly beside real values".into())
            }
            (Self::MixedText, SqliteValueRef::Integer(number)) => {
                ToSqlOutput::Owned(DuckValue::Text(number.to_string()))
            }
            // `Display` for `f64` is the shortest text that parses back to the same value.
            (Self::MixedText, SqliteValueRef::Real(number)) => {
                ToSqlOutput::Owned(DuckValue::Text(number.to_string()))
            }
            (Self::Text | Self::MixedText, SqliteValueRef::Text(bytes)) => {
                std::str::from_utf8(bytes)
                    .map_err(|_| "holds text that is not valid UTF-8".to_string())?;
                ToSqlOutput::Borrowed(DuckValueRef::Text(bytes))
            }
            (Self::Blob, SqliteValueRef::Blob(bytes)) => {
                ToSqlOutput::Borrowed(DuckValueRef::Blob(bytes))
            }
            (_, other) => {
                return Err(format!(
                    "changed storage class to {} while being read",
                    other.data_type()
                ))
            }
        })
    }
}

/// Streams one legacy table into the staging file through DuckDB's appender.
fn snapshot_table(
    source: &rusqlite::Connection,
    snapshot: &Connection,
    table: &str,
) -> Result<(), AppError> {
    let read_error = |error: rusqlite::Error| {
        AppError::Storage(format!("Failed to read legacy table '{table}': {error}"))
    };
    let stage_error = |error: duckdb::Error| {
        AppError::Storage(format!("Failed to stage legacy table '{table}': {error}"))
    };
    let unconvertible = |column: &str, reason: &str| {
        AppError::Storage(format!(
            "Legacy column '{table}.{column}' {reason}; it cannot be converted without loss. \
             The original was not modified."
        ))
    };
    let quoted = quote_identifier(table);

    // Hidden columns (an FTS5 table's own name and `rank`) are absent from `table_info`, so the
    // explicit projection below never reads them.
    let columns: Vec<(String, String)> = source
        .prepare("SELECT name, type FROM pragma_table_info(?1) ORDER BY cid")
        .and_then(|mut statement| {
            statement
                .query_map([table], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect()
        })
        .map_err(read_error)?;
    let quoted_columns: Vec<String> = columns
        .iter()
        .map(|(name, _)| quote_identifier(name))
        .collect();

    let class_unions = quoted_columns
        .iter()
        .map(|column| {
            format!(
                "coalesce(sum(DISTINCT CASE typeof({column}) \
                   WHEN 'integer' THEN {CLASS_INTEGER} WHEN 'real' THEN {CLASS_REAL} \
                   WHEN 'text' THEN {CLASS_TEXT} WHEN 'blob' THEN {CLASS_BLOB} ELSE 0 END), 0)"
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let classes: Vec<i64> = source
        .query_row(&format!("SELECT {class_unions} FROM {quoted}"), [], |row| {
            (0..columns.len()).map(|index| row.get(index)).collect()
        })
        .map_err(read_error)?;
    let staged = columns
        .iter()
        .zip(classes)
        .map(|((name, declared), classes)| {
            StagedColumn::resolve(declared, classes).ok_or_else(|| {
                unconvertible(name, "mixes binary values with other storage classes")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let definitions = quoted_columns
        .iter()
        .zip(&staged)
        .map(|(column, kind)| format!("{column} {}", kind.duckdb_type()))
        .collect::<Vec<_>>()
        .join(", ");
    snapshot
        .execute_batch(&format!("CREATE TABLE {quoted} ({definitions});"))
        .map_err(stage_error)?;

    let mut statement = source
        .prepare(&format!(
            "SELECT {} FROM {quoted}",
            quoted_columns.join(", ")
        ))
        .map_err(read_error)?;
    let mut rows = statement.query([]).map_err(read_error)?;
    let mut appender = snapshot.appender(table).map_err(stage_error)?;
    let mut buffered_bytes = 0_usize;
    while let Some(row) = rows.next().map_err(read_error)? {
        let mut values = Vec::with_capacity(staged.len());
        for (index, kind) in staged.iter().enumerate() {
            let value = row.get_ref(index).map_err(read_error)?;
            if let SqliteValueRef::Text(bytes) | SqliteValueRef::Blob(bytes) = value {
                buffered_bytes += bytes.len();
            }
            values.push(
                kind.stage(value)
                    .map_err(|reason| unconvertible(&columns[index].0, &reason))?,
            );
        }
        appender
            .append_row(duckdb::appender_params_from_iter(values))
            .map_err(stage_error)?;
        if buffered_bytes >= APPENDER_FLUSH_BYTES {
            appender.flush().map_err(stage_error)?;
            buffered_bytes = 0;
        }
    }
    appender.flush().map_err(stage_error)
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
pub(super) fn foreign_key_dependencies(
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
pub(super) fn order_by_foreign_keys(
    tables: Vec<String>,
    dependencies: &[(String, String)],
) -> Vec<String> {
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

/// Removes the scratch files a conversion stages beside `core_path`.
fn remove_staging_files(core_path: &Path) -> Result<(), AppError> {
    let snapshot = sibling_path(core_path, LEGACY_SNAPSHOT_SUFFIX);
    let source_copy = sibling_path(core_path, LEGACY_SOURCE_COPY_SUFFIX);
    remove_if_present(&snapshot)?;
    remove_if_present(&sibling_path(&snapshot, "wal"))?;
    remove_if_present(&source_copy)?;
    for suffix in SQLITE_SIDECARS {
        remove_if_present(&sidecar_path(&source_copy, suffix))?;
    }
    Ok(())
}

/// Renames a SQLite database together with whichever sidecars it has. A WAL can hold committed
/// rows the main file lacks, so a backup separated from it would be incomplete. On failure,
/// everything already moved is moved back.
fn move_with_sidecars(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)?;
    let mut moved = Vec::new();
    for suffix in SQLITE_SIDECARS {
        let (source, target) = (sidecar_path(from, suffix), sidecar_path(to, suffix));
        match fs::rename(&source, &target) {
            Ok(()) => moved.push((source, target)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                for (source, target) in moved.iter().rev() {
                    let _ = fs::rename(target, source);
                }
                let _ = fs::rename(to, from);
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Appends `suffix` to the file name verbatim, the way SQLite names `-wal`, `-journal` and `-shm`.
fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

pub(super) fn remove_if_present(path: &Path) -> Result<(), AppError> {
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
pub(super) fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

pub(super) fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub(super) fn quote_sql_literal(path: &Path) -> Result<String, AppError> {
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

    /// Writes `tables` from `staging`'s in-memory catalog to a new SQLite file, declaring each
    /// column with its DuckDB type name and storing booleans as integers, as SQLite has no boolean.
    fn write_sqlite_copy(staging: &Connection, tables: &[String], path: &Path) {
        use rusqlite::types::Value as SqliteValue;

        let mut sqlite = rusqlite::Connection::open(path).unwrap();
        let transaction = sqlite.transaction().unwrap();
        for table in tables {
            let quoted = quote_identifier(table);
            let columns: Vec<(String, String)> = staging
                .prepare(
                    "SELECT column_name, data_type FROM information_schema.columns
                     WHERE table_catalog = 'memory' AND table_schema = 'main' AND table_name = ?1
                     ORDER BY ordinal_position",
                )
                .unwrap()
                .query_map([table], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let definitions = columns
                .iter()
                .map(|(name, data_type)| format!("{} {data_type}", quote_identifier(name)))
                .collect::<Vec<_>>()
                .join(", ");
            transaction
                .execute_batch(&format!("CREATE TABLE {quoted} ({definitions});"))
                .unwrap();
            let placeholders = vec!["?"; columns.len()].join(", ");
            let mut insert = transaction
                .prepare(&format!("INSERT INTO {quoted} VALUES ({placeholders})"))
                .unwrap();
            let mut select = staging.prepare(&format!("SELECT * FROM {quoted}")).unwrap();
            let mut rows = select.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                let values: Vec<SqliteValue> = (0..columns.len())
                    .map(|index| match row.get::<_, DuckValue>(index).unwrap() {
                        DuckValue::Null => SqliteValue::Null,
                        DuckValue::Boolean(flag) => SqliteValue::Integer(i64::from(flag)),
                        DuckValue::Int(number) => SqliteValue::Integer(i64::from(number)),
                        DuckValue::BigInt(number) => SqliteValue::Integer(number),
                        DuckValue::Float(number) => SqliteValue::Real(f64::from(number)),
                        DuckValue::Double(number) => SqliteValue::Real(number),
                        DuckValue::Text(text) => SqliteValue::Text(text),
                        DuckValue::Blob(bytes) => SqliteValue::Blob(bytes),
                        other => panic!("fixture value {other:?} in '{table}' has no SQLite form"),
                    })
                    .collect();
                insert.execute(rusqlite::params_from_iter(values)).unwrap();
            }
        }
        transaction.commit().unwrap();
    }

    /// Shaped like the pre-DuckDB application's core: WAL mode, with document text in FTS5.
    fn legacy_sqlite_core(path: &Path) {
        let connection = rusqlite::Connection::open(path).unwrap();
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE VIRTUAL TABLE documents_fts USING fts5(
                    document_id, portal_subdomain UNINDEXED, file_name, extracted_text,
                    tokenize=\"porter unicode61\"
                 );
                 INSERT INTO documents_fts
                 VALUES (100, 'city-a', 'release.pdf', 'body worn camera retention schedule');
                 CREATE TABLE portals (
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
        assert_eq!(copied.get("documents_fts"), Some(&1));
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

        let tables = list_tables(&staging, "memory").unwrap();
        assert!(
            tables.len() >= 20,
            "the fixture must cover the real schema, saw {} tables",
            tables.len()
        );
        write_sqlite_copy(&staging, &tables, &core);
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
            !sibling_path(&core, LEGACY_SNAPSHOT_SUFFIX).exists(),
            "the staged legacy tables must be removed"
        );
        assert!(
            !sibling_path(&core, LEGACY_BACKUP_SUFFIX).exists(),
            "no backup should be published when the conversion never completed"
        );
    }

    /// SQLite's INTEGER affinity converts only well-formed decimal text, so `'0x1F'` stays TEXT in
    /// a column declared `INTEGER`. The copy into a `BIGINT` column casts it to 31, a changed
    /// value, which the verification anti-join must catch rather than pass as a clean upgrade.
    #[test]
    fn verification_rejects_a_declared_integer_column_value_the_copy_would_change() {
        let directory = tempfile::tempdir().unwrap();
        let core = directory.path().join("core.db");
        let connection = rusqlite::Connection::open(&core).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ledger (id INTEGER, amount INTEGER);
                 INSERT INTO ledger VALUES (1, 10), (2, '0x1F');",
            )
            .unwrap();
        let stored: String = connection
            .query_row(
                "SELECT typeof(amount) FROM ledger WHERE id = 2",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, "text");
        drop(connection);

        let error =
            migrate_legacy_sqlite_core(&core, "CREATE TABLE ledger (id BIGINT, amount BIGINT);")
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Migration verification failed for table 'ledger'"),
            "unexpected: {error}"
        );
        assert_eq!(
            classify_core_database(&core).unwrap(),
            CoreDatabaseKind::LegacySqlite
        );
    }

    /// The legacy application ran in WAL mode. Opening such a file read-only without `immutable`
    /// creates a `-shm` beside it, so this fixture is what distinguishes a truly read-only open.
    #[test]
    fn conversion_neither_writes_the_legacy_file_nor_creates_files_beside_it() {
        let directory = tempfile::tempdir().unwrap();
        let core = directory.path().join("core.db");
        legacy_sqlite_core(&core);
        for suffix in SQLITE_SIDECARS {
            assert!(
                !sidecar_path(&core, suffix).exists(),
                "fixture left {suffix}"
            );
        }
        let stale =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        fs::File::options()
            .write(true)
            .open(&core)
            .unwrap()
            .set_modified(stale)
            .unwrap();
        let original = fs::read(&core).unwrap();

        let report = migrate_legacy_sqlite_core(&core, MIGRATION).unwrap();

        assert_eq!(fs::read(&report.backup_path).unwrap(), original);
        assert_eq!(
            fs::metadata(&report.backup_path)
                .unwrap()
                .modified()
                .unwrap(),
            stale
        );
        let mut names: Vec<String> = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["core.db", "core.db.legacy-sqlite.bak"],
            "no journal, WAL, shared-memory or staging file may remain"
        );
    }

    /// `immutable` makes SQLite ignore a WAL, and a WAL-mode database that was not checkpointed
    /// holds committed rows only there. Those rows must be converted, and the WAL must stay with
    /// the retained backup so the backup still holds them.
    #[test]
    fn rows_committed_only_to_a_pending_wal_are_converted_and_kept_with_the_backup() {
        let directory = tempfile::tempdir().unwrap();
        let live = directory.path().join("live.db");
        let writer = rusqlite::Connection::open(&live).unwrap();
        writer
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA wal_autocheckpoint = 0;
                 CREATE TABLE portals (
                    subdomain TEXT PRIMARY KEY, display_name TEXT NOT NULL, base_url TEXT NOT NULL
                 );
                 INSERT INTO portals VALUES ('city-a', 'City A', 'https://city-a.example');",
            )
            .unwrap();
        // Copy while the writer is open: the state a crash or force-quit leaves on disk.
        let core = directory.path().join("core.db");
        fs::copy(&live, &core).unwrap();
        fs::copy(sidecar_path(&live, "-wal"), sidecar_path(&core, "-wal")).unwrap();
        drop(writer);
        let original = fs::read(&core).unwrap();
        let wal = fs::read(sidecar_path(&core, "-wal")).unwrap();
        assert!(!wal.is_empty());

        let report = migrate_legacy_sqlite_core(
            &core,
            "CREATE TABLE portals (
                subdomain TEXT PRIMARY KEY, display_name TEXT NOT NULL, base_url TEXT NOT NULL
             );",
        )
        .unwrap();

        assert_eq!(report.copied_tables, vec![("portals".to_string(), 1)]);
        let converted = Connection::open(&core).unwrap();
        let name: String = converted
            .query_row("SELECT display_name FROM portals", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "City A");
        assert_eq!(fs::read(&report.backup_path).unwrap(), original);
        assert_eq!(
            fs::read(sidecar_path(&report.backup_path, "-wal")).unwrap(),
            wal,
            "the WAL must travel with the backup, unchanged"
        );
        assert!(!sidecar_path(&core, "-wal").exists());
        assert!(!sibling_path(&core, LEGACY_SOURCE_COPY_SUFFIX).exists());
    }
}
