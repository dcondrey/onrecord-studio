//! Persistence layer orchestration tree registration.
//!
//! The application core is DuckDB. [`sqlite::DuckDbRepository`] owns the single durable writer and
//! every canonical table. The module retains its historical `sqlite` filename to avoid a noisy
//! source-file move; its public repository terminology reflects the actual engine.
//!
//! SQLite appears only at boundaries, never in the core:
//!
//! * [`legacy_import`] upgrades a pre-DuckDB core written by an earlier release.
//! * [`duckdb::DuckDbWorkspace`] and `analysis::anomaly` can take a bounded read-only snapshot of
//!   a SQLite file an investigator supplies, so it can be analyzed without changing the source or
//!   downloading a platform-specific DuckDB extension.
//!
//! The core search projection, `documents_fts`, is a plain DuckDB table maintained transactionally
//! by the repository. A versioned companion token table provides indexed all-term matching; it is
//! not FTS5 and does not claim stemming or relevance ranking.
//!
//! See `docs/storage-and-evidence-architecture.md`.

pub mod corpus;
pub mod cross_agency;
pub(crate) mod discovery_search;
pub mod document_origin;
pub mod duckdb;
pub mod fts;
pub mod legacy_import;
pub mod maintenance;
pub mod pii;
pub mod reconstruction;
pub mod research;
pub mod resolution_source;
pub mod search_paging;
pub mod snapshot_export;
pub mod sqlite;
#[cfg(test)]
pub(crate) mod sqlite_snapshot;
pub mod standing_queries;
pub mod storage_rewrite;
pub(crate) mod tabular;
pub mod vector;

pub use duckdb::DuckDbWorkspace;

/// Opens a DuckDB connection (a file, or in memory when `path` is `None`) that never installs or
/// loads an extension at runtime. The bundled library defaults both to on, which downloads unsigned
/// native code from extensions.duckdb.org the first time a query needs one: App Review rejects
/// downloaded executable code, and library validation in a signed build refuses to load it. Every
/// extension the app needs is compiled in through Cargo features instead.
///
/// A file this creates is written at [`DUCKDB_STORAGE_VERSION`]. The library default is v0.10.2,
/// under which DuckDB never compresses long text: measured on a real installation, 365 MB of
/// extracted text stored `Uncompressed` there and ZSTD at 89 MB under v1.5.0. An existing file
/// keeps the version in its header; this setting does not upgrade it. [`storage_rewrite`] does,
/// once, at startup.
pub(crate) fn open_duckdb(
    path: Option<&std::path::Path>,
) -> ::duckdb::Result<::duckdb::Connection> {
    let config = ::duckdb::Config::default()
        .with("autoload_known_extensions", "false")?
        .with("autoinstall_known_extensions", "false")?
        .with("storage_compatibility_version", DUCKDB_STORAGE_VERSION)?;
    match path {
        Some(path) => ::duckdb::Connection::open_with_flags(path, config),
        None => ::duckdb::Connection::open_in_memory_with_flags(config),
    }
}

/// Pinned rather than `latest` so a library upgrade cannot silently write files an older release
/// cannot open; raising it is a deliberate change covered by the release rollback procedure.
const DUCKDB_STORAGE_VERSION: &str = "v1.5.0";
pub use pii::{mask_json_pii, mask_pii, PiiFinding, PiiKind, PiiMaskResult};
pub use sqlite::{DocumentExtractionFinalization, DuckDbRepository};
pub use vector::VectorIndex;

pub mod bundle;
pub mod case_scope;
pub mod casework;
pub mod casework_operations;
pub mod facilities;
pub mod financial;
pub mod findings;
pub mod monitoring;
pub mod organisations;
pub(crate) mod redaction_sweep;
pub mod review;
pub mod watchlists;

/// A DuckDB identifier in double quotes, embedded quotes doubled.
pub(crate) fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// A DuckDB string literal in single quotes, embedded quotes doubled.
pub(crate) fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// The current UTC instant as RFC 3339 at whole-second precision with a `Z` suffix.
pub(crate) fn now_stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn json_functions_work_without_installing_an_extension() {
        let connection = super::open_duckdb(None).expect("in-memory DuckDB must open");
        let (value, autoinstall): (String, bool) = connection
            .query_row(
                "SELECT json_extract_string('{\"a\":\"b\"}', '$.a'), current_setting('autoinstall_known_extensions')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("json_extract_string must be compiled in");
        assert_eq!(value, "b");
        assert!(!autoinstall);
    }

    #[test]
    fn timestamps_render_in_utc_without_a_session_time_zone() {
        // Stored timestamps carry a literal `Z`; that is only honest while no ICU extension can
        // give the session a local zone.
        let connection = super::open_duckdb(None).expect("in-memory DuckDB must open");
        let rendered: String = connection
            .query_row(
                "SELECT strftime(TIMESTAMPTZ '2026-01-01 12:00:00-08:00', '%Y-%m-%dT%H:%M')",
                [],
                |row| row.get(0),
            )
            .expect("strftime over TIMESTAMPTZ must work without ICU");
        assert_eq!(rendered, "2026-01-01T20:00");
    }

    fn text_column_compression(connection: &::duckdb::Connection) -> Vec<String> {
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT compression FROM pragma_storage_info('t')
                 WHERE column_name = 'body' AND segment_type = 'VARCHAR'",
            )
            .expect("storage info must be readable");
        statement
            .query_map([], |row| row.get(0))
            .expect("storage info must query")
            .collect::<Result<_, _>>()
            .expect("storage info rows must read")
    }

    const POPULATE_LONG_TEXT: &str = "CREATE TABLE t AS
        SELECT repeat('records request responsive document ' || i::VARCHAR, 4000) AS body
        FROM range(64) r(i);
        CHECKPOINT;";

    #[test]
    fn a_new_file_compresses_long_text() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let connection = super::open_duckdb(Some(&directory.path().join("fresh.db")))
            .expect("a new file must open");
        connection
            .execute_batch(POPULATE_LONG_TEXT)
            .expect("text must persist");
        let compression = text_column_compression(&connection);
        assert!(
            !compression.is_empty() && !compression.iter().any(|c| c == "Uncompressed"),
            "long text in a new file stored as {compression:?}"
        );
    }

    #[test]
    fn a_file_written_at_the_library_default_version_still_opens_and_writes() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("legacy.db");
        {
            let legacy = ::duckdb::Connection::open(&path).expect("legacy file must be created");
            legacy
                .execute_batch("CREATE TABLE kept (id BIGINT); INSERT INTO kept VALUES (1);")
                .expect("legacy rows must persist");
        }
        let connection = super::open_duckdb(Some(&path)).expect("a legacy file must open");
        connection
            .execute_batch(&format!(
                "INSERT INTO kept VALUES (2); {POPULATE_LONG_TEXT}"
            ))
            .expect("a legacy file must accept writes");
        let kept: i64 = connection
            .query_row("SELECT count(*) FROM kept", [], |row| row.get(0))
            .expect("rows must read");
        assert_eq!(kept, 2);
        // The header's version governs, so this file still stores text uncompressed. Asserting it
        // keeps the doc comment on `open_duckdb` honest if a library upgrade starts migrating files.
        assert_eq!(text_column_compression(&connection), vec!["Uncompressed"]);
    }
}
