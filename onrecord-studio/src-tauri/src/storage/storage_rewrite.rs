//! One-time rewrite of a DuckDB file into the storage version [`super::open_duckdb`] pins.
//!
//! A DuckDB file keeps the storage version recorded in its header, so an installation created
//! before the pin stays at `v0.10.2`, where long text is never compressed, until something copies
//! it into a new file. `COPY FROM DATABASE` cannot: it copies tables out of foreign-key order and
//! fails on the real core. This module copies the catalog verbatim instead, in dependency order,
//! verifies the copy, and swaps it into place before anything else opens the file.
//!
//! The copy reproduces each object from the old file's own catalog SQL, never from the current
//! schema script, because the stored definitions are what existing rows, foreign keys and upgrade
//! shims bind against: a column the old file stores as `BIGINT` stays `BIGINT`, and every default,
//! `CHECK`, `NOT NULL`, key, index, view and sequence counter is carried as stored.
//!
//! # Crash states
//!
//! | step | on disk if the process dies here | next attempt |
//! |---|---|---|
//! | 1. open the original, `CHECKPOINT`, close | the original, its WAL possibly unfolded | retried |
//! | 2. build and verify `<db>.storage-rewrite` | the original and a partial sibling | sibling discarded, retried |
//! | 3. hard-link the original as `<db>.storage-rewrite.bak` | the original under two names, the verified sibling | link and sibling discarded, retried |
//! | 4. rename the sibling over the original | the rewritten file and the backup | nothing to do |
//!
//! The original is never moved or written after step 1, and step 4 is a single `rename`, which
//! replaces the directory entry atomically. So the path the application opens names a complete
//! database at every instant; there is no state in which it is missing and startup would create a
//! fresh, empty one. The backup is removed by [`remove_retained_backup`] once a later launch has
//! completed startup on the rewritten file.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use duckdb::Connection;

use super::legacy_import::{
    foreign_key_dependencies, order_by_foreign_keys, quote_sql_literal, remove_if_present,
    sibling_path,
};
use super::quote_ident;
use crate::error::AppError;

/// Suffix of the file being built; it is only ever published by the final rename.
const REWRITE_SUFFIX: &str = "storage-rewrite";
/// Suffix of the retained original.
const BACKUP_SUFFIX: &str = "storage-rewrite.bak";
/// Catalog alias under which the original is attached, read-only, while it is copied.
const SOURCE: &str = "rewrite_source";

/// What [`rewrite_to_current_storage`] did.
#[derive(Debug)]
pub enum StorageRewrite {
    /// No file exists yet, or it is already at the pinned storage version.
    NotNeeded,
    Rewritten(StorageRewriteReport),
}

/// Measurements of a completed rewrite.
#[derive(Debug)]
pub struct StorageRewriteReport {
    /// The original's `storage_version` tag, e.g. `v1.0.0+` for a `v0.10.2` file.
    pub previous_storage: String,
    pub backup_path: PathBuf,
    pub bytes_before: u64,
    pub bytes_after: u64,
    /// Every table with its row count, in copy order. Each count and content digest was verified
    /// equal in the original and the copy before the swap.
    pub tables: Vec<(String, u64)>,
    pub elapsed: Duration,
}

impl StorageRewriteReport {
    pub fn total_rows(&self) -> u64 {
        self.tables
            .iter()
            .map(|(_, rows)| *rows)
            .fold(0_u64, u64::saturating_add)
    }
}

/// Rewrites the DuckDB file at `path` into the pinned storage version if it is older.
///
/// Must run before any connection to `path` is open. On any error the original is left in place,
/// unchanged and launchable, at its current storage version, and nothing half-built remains.
pub fn rewrite_to_current_storage(path: &Path) -> Result<StorageRewrite, AppError> {
    rewrite_with(path, available_bytes, &mut |_, _| Ok(()))
}

/// Deletes the original retained by an earlier rewrite. Call only after startup has succeeded on
/// a launch that did not itself perform the rewrite.
pub fn remove_retained_backup(path: &Path) -> Result<bool, AppError> {
    let backup = sibling_path(path, BACKUP_SUFFIX);
    if !exists(&backup)? || same_file(path, &backup)? {
        // A link to the live file is an interrupted publish, not a backup; the next rewrite
        // attempt releases it.
        return Ok(false);
    }
    remove_if_present(&backup)?;
    Ok(true)
}

/// `after_table_copy` runs on the build connection after each table's rows are copied; tests use
/// it to fail or tamper mid-copy.
fn rewrite_with(
    path: &Path,
    available_bytes: fn(&Path) -> Result<Option<u64>, AppError>,
    after_table_copy: &mut dyn FnMut(&Connection, &str) -> Result<(), AppError>,
) -> Result<StorageRewrite, AppError> {
    let started = Instant::now();
    let rewrite_path = sibling_path(path, REWRITE_SUFFIX);
    let backup_path = sibling_path(path, BACKUP_SUFFIX);

    // Only the final rename publishes the sibling, so one that exists now was never published.
    discard_database_files(&rewrite_path)?;
    if !exists(path)? {
        return Ok(StorageRewrite::NotNeeded);
    }
    let Some(previous_storage) = checkpoint_if_outdated(path)? else {
        return Ok(StorageRewrite::NotNeeded);
    };
    if exists(&wal_path(path))? {
        return Err(AppError::Storage(format!(
            "'{}' still has a write-ahead log after a checkpoint; it was left at storage {previous_storage}.",
            path.display()
        )));
    }
    release_interrupted_publish(path, &backup_path)?;

    let bytes_before = file_len(path)?;
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    // The new file is no larger than the original, and the same again covers its write-ahead log
    // and spill while it is built. The original itself is retained by a hard link, not a copy.
    let required = bytes_before.saturating_mul(2);
    match available_bytes(directory)? {
        Some(free) if free >= required => {}
        Some(free) => {
            return Err(AppError::Storage(format!(
                "Rewriting '{}' to compressed storage needs {} free and only {} is available; it \
                 was left at storage {previous_storage}. Free space and relaunch to compress it.",
                path.display(),
                megabytes(required),
                megabytes(free)
            )))
        }
        None => {
            return Err(AppError::Storage(format!(
                "Free space cannot be measured on this platform, so '{}' was left at storage \
                 {previous_storage}.",
                path.display()
            )))
        }
    }

    let built = build_verified_copy(path, &rewrite_path, after_table_copy)
        .and_then(|tables| publish(path, &rewrite_path, &backup_path).map(|()| tables));
    let tables = match built {
        Ok(tables) => tables,
        Err(error) => {
            // The failure is the cause the caller needs; a leftover sibling costs only disk and is
            // discarded by the next attempt.
            if let Err(cleanup) = discard_database_files(&rewrite_path) {
                eprintln!("Failed to discard the partial storage rewrite: {cleanup}");
            }
            return Err(error);
        }
    };

    Ok(StorageRewrite::Rewritten(StorageRewriteReport {
        previous_storage,
        backup_path,
        bytes_before,
        bytes_after: file_len(path)?,
        tables,
        elapsed: started.elapsed(),
    }))
}

/// Returns the file's storage tag when it is older than the pinned version, after folding its
/// write-ahead log into the file so the original is a single self-contained file.
fn checkpoint_if_outdated(path: &Path) -> Result<Option<String>, AppError> {
    let connection = super::open_duckdb(Some(path))
        .map_err(|error| storage_error(&format!("open '{}'", path.display()), error))?;
    let tag = storage_tag(&connection)?;
    if !older_than_pinned(&tag)? {
        return Ok(None);
    }
    connection
        .execute_batch("CHECKPOINT;")
        .map_err(|error| storage_error("checkpoint the original", error))?;
    Ok(Some(tag))
}

/// Builds `target` from `source` and verifies it, returning each table's verified row count.
fn build_verified_copy(
    source: &Path,
    target: &Path,
    after_table_copy: &mut dyn FnMut(&Connection, &str) -> Result<(), AppError>,
) -> Result<Vec<(String, u64)>, AppError> {
    let connection = super::open_duckdb(Some(target))
        .map_err(|error| storage_error(&format!("create '{}'", target.display()), error))?;
    connection
        .execute_batch(&format!(
            "ATTACH {} AS {SOURCE} (READ_ONLY);",
            quote_sql_literal(source)?
        ))
        .map_err(|error| storage_error("attach the original", error))?;
    let target_catalog: String = connection
        .query_row("SELECT current_database()", [], |row| row.get(0))
        .map_err(|error| storage_error("resolve the new catalog", error))?;

    reject_unreproduced_objects(&connection)?;

    // Sequences first: table defaults bind `nextval`. Their catalog SQL carries the live counter
    // as `START`, so the copy resumes numbering where the original stopped.
    for (_, sql) in catalog(&connection, SOURCE, CatalogKind::Sequence)? {
        execute(&connection, &sql)?;
    }
    let tables = catalog(&connection, SOURCE, CatalogKind::Table)?;
    // Foreign keys are enforced while rows are inserted, so a child copied before its parent fails.
    let order = order_by_foreign_keys(
        tables.iter().map(|(name, _)| name.clone()).collect(),
        &foreign_key_dependencies(&connection, SOURCE)?,
    );
    for table in &order {
        if let Some((_, sql)) = tables.iter().find(|(name, _)| name == table) {
            execute(&connection, sql)?;
        }
    }
    for table in &order {
        let quoted = quote_ident(table);
        execute(
            &connection,
            &format!("INSERT INTO main.{quoted} SELECT * FROM {SOURCE}.main.{quoted};"),
        )?;
        after_table_copy(&connection, table)?;
    }
    // Indexes after the rows, so each is built once rather than maintained per insert.
    for (_, sql) in catalog(&connection, SOURCE, CatalogKind::Index)? {
        execute(&connection, &sql)?;
    }
    for (_, sql) in catalog(&connection, SOURCE, CatalogKind::View)? {
        execute(&connection, &sql)?;
    }

    for kind in CatalogKind::ALL {
        let mut original = catalog(&connection, SOURCE, kind)?;
        let mut copied = catalog(&connection, &target_catalog, kind)?;
        original.sort();
        copied.sort();
        if original != copied {
            return Err(AppError::Storage(format!(
                "The rewritten database's {} definitions differ from the original's; the original \
                 was not modified.",
                kind.label()
            )));
        }
    }
    let mut verified = Vec::with_capacity(order.len());
    for table in &order {
        let original = table_digest(&connection, SOURCE, table)?;
        let copied = table_digest(&connection, &target_catalog, table)?;
        if original != copied {
            return Err(AppError::Storage(format!(
                "Rewrite verification failed for table '{table}': the original holds {} rows \
                 (digest {}) and the copy {} rows (digest {}). The original was not modified.",
                original.0, original.1, copied.0, copied.1
            )));
        }
        verified.push((table.clone(), original.0));
    }

    execute(&connection, &format!("DETACH {SOURCE}; CHECKPOINT;"))?;
    let tag = storage_tag(&connection)?;
    if older_than_pinned(&tag)? {
        return Err(AppError::Storage(format!(
            "The rewritten database reports storage {tag}, not {}; the original was not modified.",
            super::DUCKDB_STORAGE_VERSION
        )));
    }
    drop(connection);
    if exists(&wal_path(target))? {
        return Err(AppError::Storage(
            "The rewritten database kept a write-ahead log after closing; the original was not \
             modified."
                .to_string(),
        ));
    }
    fs::File::open(target)
        .and_then(|file| file.sync_all())
        .map_err(|error| AppError::Storage(format!("Failed to flush the rewrite: {error}")))?;
    Ok(verified)
}

/// Steps 3 and 4 of the module table.
fn publish(path: &Path, rewritten: &Path, backup: &Path) -> Result<(), AppError> {
    // DuckDB replays `<db>.wal` into whatever file sits at `<db>`; the original's log must never
    // be found beside the rewritten file.
    if exists(&wal_path(path))? {
        return Err(AppError::Storage(format!(
            "'{}' gained a write-ahead log during the rewrite; another process opened it. The \
             original was not modified.",
            path.display()
        )));
    }
    fs::hard_link(path, backup).map_err(|error| {
        AppError::Storage(format!(
            "Failed to retain the original as '{}': {error}",
            backup.display()
        ))
    })?;
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    if let Err(error) = sync_directory(directory).and_then(|()| {
        fs::rename(rewritten, path).map_err(|error| {
            AppError::Storage(format!("Failed to install the rewritten database: {error}"))
        })
    }) {
        // The original is still at `path`; the link is only a second name for it.
        let _ = remove_if_present(backup);
        return Err(error);
    }
    // The rename has happened; a failed flush of the directory entry affects only durability of
    // the swap across a power loss, and either outcome of that is a complete database.
    if let Err(error) = sync_directory(directory) {
        eprintln!("Failed to flush the storage rewrite's directory entry: {error}");
    }
    Ok(())
}

/// A backup path left by step 3 of an interrupted attempt is only a second name for the live
/// file and can go. Anything else there is a real earlier backup, which is never overwritten.
fn release_interrupted_publish(path: &Path, backup: &Path) -> Result<(), AppError> {
    if !exists(backup)? {
        return Ok(());
    }
    if same_file(path, backup)? {
        return remove_if_present(backup);
    }
    Err(AppError::Storage(format!(
        "An earlier storage rewrite's backup is still at '{}'; '{}' stays at its current storage \
         version until that backup is removed by a successful launch or moved aside.",
        backup.display(),
        path.display()
    )))
}

/// Refuses a file holding catalog objects this rewrite does not copy, rather than dropping them.
fn reject_unreproduced_objects(connection: &Connection) -> Result<(), AppError> {
    for (label, query) in [
        (
            "schemas other than main",
            "SELECT count(*) FROM duckdb_schemas()
             WHERE database_name = ?1 AND NOT internal AND schema_name <> 'main'",
        ),
        (
            "user-defined types",
            "SELECT count(*) FROM duckdb_types() WHERE database_name = ?1 AND NOT internal",
        ),
        (
            "macros",
            "SELECT count(*) FROM duckdb_functions() WHERE database_name = ?1 AND NOT internal",
        ),
    ] {
        let count: i64 = connection
            .query_row(query, [SOURCE], |row| row.get(0))
            .map_err(|error| storage_error(&format!("inventory {label}"), error))?;
        if count != 0 {
            return Err(AppError::Storage(format!(
                "The database holds {count} {label}, which the storage rewrite does not \
                 reproduce; it was left at its current storage version."
            )));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum CatalogKind {
    Sequence,
    Table,
    Index,
    View,
}

impl CatalogKind {
    const ALL: [Self; 4] = [Self::Sequence, Self::Table, Self::Index, Self::View];

    fn label(self) -> &'static str {
        match self {
            Self::Sequence => "sequence",
            Self::Table => "table",
            Self::Index => "index",
            Self::View => "view",
        }
    }

    /// `(name, sql)` in creation order. Views must follow the views they select from, which
    /// creation order guarantees.
    fn query(self) -> &'static str {
        match self {
            Self::Sequence => {
                "SELECT sequence_name, sql FROM duckdb_sequences()
                 WHERE database_name = ?1 AND schema_name = 'main' AND NOT temporary
                 ORDER BY sequence_oid"
            }
            Self::Table => {
                "SELECT table_name, sql FROM duckdb_tables()
                 WHERE database_name = ?1 AND schema_name = 'main' AND NOT internal
                   AND NOT temporary
                 ORDER BY table_oid"
            }
            Self::Index => {
                "SELECT index_name, sql FROM duckdb_indexes()
                 WHERE database_name = ?1 AND schema_name = 'main'
                 ORDER BY index_oid"
            }
            Self::View => {
                "SELECT view_name, sql FROM duckdb_views()
                 WHERE database_name = ?1 AND schema_name = 'main' AND NOT internal
                   AND NOT temporary
                 ORDER BY view_oid"
            }
        }
    }
}

/// One kind of object in one catalog, as DuckDB reconstructs its definition from what is stored.
fn catalog(
    connection: &Connection,
    database: &str,
    kind: CatalogKind,
) -> Result<Vec<(String, String)>, AppError> {
    let context = format!("read {} definitions", kind.label());
    let mut statement = connection
        .prepare(kind.query())
        .map_err(|error| storage_error(&context, error))?;
    let rows = statement
        .query_map([database], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|error| storage_error(&context, error))?;
    let mut objects = Vec::new();
    for row in rows {
        let (name, sql) = row.map_err(|error| storage_error(&context, error))?;
        let sql = sql.ok_or_else(|| {
            AppError::Storage(format!(
                "The {} '{name}' has no reproducible definition; the database was left at its \
                 current storage version.",
                kind.label()
            ))
        })?;
        objects.push((name, sql));
    }
    Ok(objects)
}

/// Row count and an order-independent content digest: the sum of every row's hash, which, unlike
/// an XOR, does not cancel duplicate rows.
fn table_digest(
    connection: &Connection,
    database: &str,
    table: &str,
) -> Result<(u64, String), AppError> {
    let context = format!("digest '{database}.{table}'");
    let mut statement = connection
        .prepare(
            "SELECT column_name FROM duckdb_columns()
             WHERE database_name = ?1 AND schema_name = 'main' AND table_name = ?2
             ORDER BY column_index",
        )
        .map_err(|error| storage_error(&context, error))?;
    let columns = statement
        .query_map([database, table], |row| row.get::<_, String>(0))
        .map_err(|error| storage_error(&context, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| storage_error(&context, error))?;
    if columns.is_empty() {
        return Err(AppError::Storage(format!(
            "Table '{database}.{table}' reports no columns"
        )));
    }
    let projection = columns
        .iter()
        .map(|column| quote_ident(column))
        .collect::<Vec<_>>()
        .join(", ");
    let (rows, digest): (i64, String) = connection
        .query_row(
            &format!(
                "SELECT count(*), COALESCE(CAST(sum(CAST(hash({projection}) AS HUGEINT)) AS VARCHAR), '')
                 FROM {}.main.{}",
                quote_ident(database),
                quote_ident(table)
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| storage_error(&context, error))?;
    let rows = u64::try_from(rows)
        .map_err(|_| AppError::Storage(format!("{context}: negative row count")))?;
    Ok((rows, digest))
}

/// The `storage_version` tag of the connection's own file.
fn storage_tag(connection: &Connection) -> Result<String, AppError> {
    let tag: Option<String> = connection
        .query_row(
            "SELECT tags['storage_version'] FROM duckdb_databases()
             WHERE database_name = current_database()",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("read the storage version", error))?;
    tag.ok_or_else(|| AppError::Storage("The database reports no storage version".to_string()))
}

/// Whether a tag such as `v1.0.0+` names a storage version older than the pinned one.
fn older_than_pinned(tag: &str) -> Result<bool, AppError> {
    let pinned = parse_version(super::DUCKDB_STORAGE_VERSION).ok_or_else(|| {
        AppError::Internal("The pinned DuckDB storage version is malformed".to_string())
    })?;
    let found = parse_version(tag)
        .ok_or_else(|| AppError::Storage(format!("Unrecognized DuckDB storage version '{tag}'")))?;
    Ok(found < pinned)
}

fn parse_version(tag: &str) -> Option<(u32, u32, u32)> {
    let mut parts = tag
        .strip_prefix('v')?
        .trim_end_matches('+')
        .split('.')
        .map(|part| part.parse::<u32>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

fn execute(connection: &Connection, sql: &str) -> Result<(), AppError> {
    connection
        .execute_batch(sql)
        .map_err(|error| storage_error(&format!("run `{sql}`"), error))
}

fn storage_error(context: &str, error: duckdb::Error) -> AppError {
    AppError::Storage(format!("Storage rewrite failed to {context}: {error}"))
}

/// DuckDB's write-ahead log, kept beside the database as `<file>.wal`.
pub(super) fn wal_path(path: &Path) -> PathBuf {
    sibling_path(path, "wal")
}

/// Removes a database file with its write-ahead log and DuckDB's spill directory.
fn discard_database_files(path: &Path) -> Result<(), AppError> {
    remove_if_present(path)?;
    remove_if_present(&wal_path(path))?;
    let spill = sibling_path(path, "tmp");
    match fs::remove_dir_all(&spill) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Storage(format!(
            "Failed to remove '{}': {error}",
            spill.display()
        ))),
    }
}

fn exists(path: &Path) -> Result<bool, AppError> {
    path.try_exists().map_err(|error| {
        AppError::Storage(format!("Failed to inspect '{}': {error}", path.display()))
    })
}

fn file_len(path: &Path) -> Result<u64, AppError> {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| {
            AppError::Storage(format!("Failed to inspect '{}': {error}", path.display()))
        })
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

#[cfg(unix)]
fn same_file(left: &Path, right: &Path) -> Result<bool, AppError> {
    use std::os::unix::fs::MetadataExt;
    let identity = |path: &Path| match fs::metadata(path) {
        Ok(metadata) => Ok(Some((metadata.dev(), metadata.ino()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(AppError::Storage(format!(
            "Failed to inspect '{}': {error}",
            path.display()
        ))),
    };
    Ok(matches!((identity(left)?, identity(right)?), (Some(a), Some(b)) if a == b))
}

#[cfg(not(unix))]
fn same_file(_: &Path, _: &Path) -> Result<bool, AppError> {
    // Unreachable in practice: without a free-space measurement no backup is ever created here.
    Ok(false)
}

#[cfg(unix)]
fn available_bytes(directory: &Path) -> Result<Option<u64>, AppError> {
    let stats = rustix::fs::statvfs(directory).map_err(|error| {
        AppError::Storage(format!(
            "Failed to measure free space in '{}': {error}",
            directory.display()
        ))
    })?;
    Ok(Some(stats.f_bavail.saturating_mul(stats.f_frsize)))
}

#[cfg(not(unix))]
fn available_bytes(_: &Path) -> Result<Option<u64>, AppError> {
    Ok(None)
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<(), AppError> {
    fs::File::open(directory)
        .and_then(|handle| handle.sync_all())
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to flush directory '{}': {error}",
                directory.display()
            ))
        })
}

#[cfg(not(unix))]
fn sync_directory(_: &Path) -> Result<(), AppError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `analytical_rows` sorts before its parent, as on the real core where `COPY FROM DATABASE`
    /// failed; keys are declared `BIGINT`, as the legacy import stored them.
    const LEGACY_FIXTURE: &str = "
        CREATE SEQUENCE analytical_row_seq START 1;
        CREATE TABLE analytical_snapshots(
            snapshot_id BIGINT PRIMARY KEY,
            label VARCHAR NOT NULL CHECK (length(label) > 0)
        );
        CREATE TABLE analytical_rows(
            row_id BIGINT DEFAULT nextval('analytical_row_seq') PRIMARY KEY,
            snapshot_id BIGINT NOT NULL REFERENCES analytical_snapshots(snapshot_id),
            row_count INTEGER NOT NULL CHECK (row_count >= 0),
            body VARCHAR NOT NULL
        );
        CREATE INDEX idx_analytical_rows_snapshot ON analytical_rows(snapshot_id);
        CREATE VIEW analytical_row_bodies AS SELECT row_id, body FROM analytical_rows;
        INSERT INTO analytical_snapshots VALUES (1, 'first'), (2, 'second');
        INSERT INTO analytical_rows(snapshot_id, row_count, body)
            SELECT 1 + i % 2, i, repeat('records request responsive document ' || i::VARCHAR, 4000)
            FROM range(64) r(i);";

    type Snapshot = (String, Vec<Vec<(String, String)>>, Vec<(u64, String)>);

    fn legacy_file(directory: &Path) -> PathBuf {
        let path = directory.join("core.db");
        let connection = Connection::open(&path).expect("legacy file must be created");
        connection
            .execute_batch(LEGACY_FIXTURE)
            .expect("legacy fixture must load");
        path
    }

    fn plenty(_: &Path) -> Result<Option<u64>, AppError> {
        Ok(Some(u64::MAX))
    }

    fn rewrite(
        path: &Path,
        hook: &mut dyn FnMut(&Connection, &str) -> Result<(), AppError>,
    ) -> Result<StorageRewrite, AppError> {
        rewrite_with(path, plenty, hook)
    }

    /// Everything a reader can observe: storage tag, catalog definitions, and per-table digests.
    fn snapshot(path: &Path) -> Snapshot {
        let connection = super::super::open_duckdb(Some(path)).expect("database must open");
        let catalog_name: String = connection
            .query_row("SELECT current_database()", [], |row| row.get(0))
            .unwrap();
        let definitions = CatalogKind::ALL
            .iter()
            .map(|kind| {
                let mut objects = catalog(&connection, &catalog_name, *kind).unwrap();
                objects.sort();
                objects
            })
            .collect();
        let digests = ["analytical_rows", "analytical_snapshots"]
            .iter()
            .map(|table| table_digest(&connection, &catalog_name, table).unwrap())
            .collect();
        (storage_tag(&connection).unwrap(), definitions, digests)
    }

    fn body_compression(connection: &Connection) -> Vec<String> {
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT compression FROM pragma_storage_info('analytical_rows')
                 WHERE column_name = 'body' AND segment_type = 'VARCHAR'",
            )
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn assert_untouched(path: &Path, before: &Snapshot) {
        assert!(!sibling_path(path, REWRITE_SUFFIX).exists());
        assert!(!sibling_path(path, BACKUP_SUFFIX).exists());
        assert_eq!(&snapshot(path), before, "the original changed");
    }

    #[test]
    fn rewrites_an_outdated_file_preserving_schema_rows_and_sequence() {
        let directory = tempfile::tempdir().unwrap();
        let path = legacy_file(directory.path());
        let before = snapshot(&path);
        assert_eq!(before.0, "v1.0.0+");
        assert_eq!(
            body_compression(&super::super::open_duckdb(Some(&path)).unwrap()),
            vec!["Uncompressed"]
        );

        let StorageRewrite::Rewritten(report) = rewrite(&path, &mut |_, _| Ok(())).unwrap() else {
            panic!("an outdated file must be rewritten");
        };
        assert_eq!(report.total_rows(), 66);
        assert_eq!(report.tables[0].0, "analytical_snapshots");

        let after = snapshot(&path);
        assert_eq!(after.0, "v1.5.0+");
        assert_eq!(after.1, before.1, "definitions must be reproduced exactly");
        assert_eq!(after.2, before.2, "rows must be reproduced exactly");
        assert_eq!(
            snapshot(&report.backup_path),
            before,
            "the backup is the original"
        );

        let connection = super::super::open_duckdb(Some(&path)).unwrap();
        // Before any insert: a fresh, uncheckpointed row lives in its own uncompressed segment.
        let compression = body_compression(&connection);
        assert!(
            !compression.is_empty() && !compression.iter().any(|c| c == "Uncompressed"),
            "long text after the rewrite stored as {compression:?}"
        );
        let next: i64 = connection
            .query_row(
                "INSERT INTO analytical_rows(snapshot_id, row_count, body) VALUES (1, 0, 'x')
                 RETURNING row_id",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            next, 65,
            "the sequence must continue where the original stopped"
        );
        let orphan = connection.execute(
            "INSERT INTO analytical_rows(snapshot_id, row_count, body) VALUES (9, 0, 'x')",
            [],
        );
        assert!(orphan.is_err(), "the foreign key must still be enforced");
        let negative = connection.execute(
            "INSERT INTO analytical_rows(snapshot_id, row_count, body) VALUES (1, -1, 'x')",
            [],
        );
        assert!(negative.is_err(), "the CHECK must still be enforced");
        drop(connection);

        assert!(matches!(
            rewrite(&path, &mut |_, _| Ok(())).unwrap(),
            StorageRewrite::NotNeeded
        ));
        assert!(remove_retained_backup(&path).unwrap());
        assert!(!report.backup_path.exists());
    }

    #[test]
    fn a_failure_mid_copy_leaves_the_original_intact_and_launchable() {
        let directory = tempfile::tempdir().unwrap();
        let path = legacy_file(directory.path());
        let before = snapshot(&path);
        let result = rewrite(&path, &mut |_, table| {
            Err(AppError::Storage(format!("injected failure after {table}")))
        });
        assert!(result.unwrap_err().to_string().contains("injected failure"));
        assert_untouched(&path, &before);
    }

    #[test]
    fn a_copy_that_differs_from_the_original_is_never_swapped_in() {
        let directory = tempfile::tempdir().unwrap();
        let path = legacy_file(directory.path());
        let before = snapshot(&path);
        let result = rewrite(&path, &mut |connection, table| {
            if table == "analytical_rows" {
                connection
                    .execute_batch(
                        "UPDATE main.analytical_rows SET body = 'altered' WHERE row_id = 7;",
                    )
                    .map_err(|error| AppError::Storage(error.to_string()))?;
            }
            Ok(())
        });
        let message = result.unwrap_err().to_string();
        assert!(message.contains("analytical_rows"), "{message}");
        assert_untouched(&path, &before);
    }

    #[test]
    fn free_space_below_twice_the_file_refuses_before_writing_anything() {
        fn one_byte_short(directory: &Path) -> Result<Option<u64>, AppError> {
            Ok(Some(
                fs::metadata(directory.join("core.db")).unwrap().len() * 2 - 1,
            ))
        }
        fn exactly_enough(directory: &Path) -> Result<Option<u64>, AppError> {
            Ok(Some(
                fs::metadata(directory.join("core.db")).unwrap().len() * 2,
            ))
        }
        let directory = tempfile::tempdir().unwrap();
        let path = legacy_file(directory.path());
        let before = snapshot(&path);
        let message = rewrite_with(&path, one_byte_short, &mut |_, _| Ok(()))
            .unwrap_err()
            .to_string();
        assert!(message.contains("free"), "{message}");
        assert_untouched(&path, &before);
        assert!(matches!(
            rewrite_with(&path, exactly_enough, &mut |_, _| Ok(())).unwrap(),
            StorageRewrite::Rewritten(_)
        ));
    }

    #[test]
    fn an_attempt_interrupted_before_the_swap_is_retried_cleanly() {
        let directory = tempfile::tempdir().unwrap();
        let path = legacy_file(directory.path());
        let before = snapshot(&path);
        // What a crash between steps 3 and 4 leaves: a sibling and the original linked under the
        // backup name. A partial sibling (a crash in step 2) is the same file with less in it.
        let sibling = sibling_path(&path, REWRITE_SUFFIX);
        fs::write(&sibling, b"partial").unwrap();
        fs::write(wal_path(&sibling), b"partial").unwrap();
        fs::hard_link(&path, sibling_path(&path, BACKUP_SUFFIX)).unwrap();
        assert!(
            !remove_retained_backup(&path).unwrap(),
            "a link to the live file is no backup"
        );
        assert_eq!(snapshot(&path), before, "the interrupted state must open");

        let StorageRewrite::Rewritten(report) = rewrite(&path, &mut |_, _| Ok(())).unwrap() else {
            panic!("the retry must rewrite");
        };
        assert_eq!(snapshot(&path).2, before.2);
        assert_eq!(snapshot(&report.backup_path), before);
        assert!(!sibling.exists() && !wal_path(&sibling).exists());
    }

    #[test]
    fn a_distinct_earlier_backup_is_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = legacy_file(directory.path());
        let before = snapshot(&path);
        let backup = sibling_path(&path, BACKUP_SUFFIX);
        fs::write(&backup, b"an earlier original").unwrap();
        assert!(rewrite(&path, &mut |_, _| Ok(())).is_err());
        assert_eq!(fs::read(&backup).unwrap(), b"an earlier original");
        assert_eq!(snapshot(&path), before);
    }

    #[test]
    fn a_catalog_object_the_copy_cannot_reproduce_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = legacy_file(directory.path());
        Connection::open(&path)
            .unwrap()
            .execute_batch("CREATE MACRO doubled(x) AS x * 2;")
            .unwrap();
        let before = snapshot(&path);
        let message = rewrite(&path, &mut |_, _| Ok(())).unwrap_err().to_string();
        assert!(message.contains("macros"), "{message}");
        assert_untouched(&path, &before);
    }

    #[test]
    fn storage_tags_compare_by_version_not_text() {
        assert!(older_than_pinned("v1.0.0+").unwrap());
        assert!(older_than_pinned("v1.4.9+").unwrap());
        assert!(!older_than_pinned("v1.5.0+").unwrap());
        assert!(!older_than_pinned("v1.10.0+").unwrap());
        assert!(older_than_pinned("latest").is_err());
    }
}
