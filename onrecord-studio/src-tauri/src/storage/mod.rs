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
//! * [`fts`] carries a SQLite FTS5 query sanitizer that no core path calls; see its module docs.
//!
//! The core search projection, `documents_fts`, is a plain DuckDB table maintained transactionally
//! by the repository. A versioned companion token table provides indexed all-term matching; it is
//! not FTS5 and does not claim stemming or relevance ranking.
//!
//! See `docs/storage-and-evidence-architecture.md`.

pub mod corpus;
pub mod cross_agency;
pub(crate) mod discovery_search;
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
pub(crate) mod tabular;
pub mod vector;

pub use duckdb::DuckDbWorkspace;

/// Opens a DuckDB connection (a file, or in memory when `path` is `None`) that never installs or
/// loads an extension at runtime. The bundled library defaults both to on, which downloads unsigned
/// native code from extensions.duckdb.org the first time a query needs one: App Review rejects
/// downloaded executable code, and library validation in a signed build refuses to load it. Every
/// extension the app needs is compiled in through Cargo features instead.
pub(crate) fn open_duckdb(
    path: Option<&std::path::Path>,
) -> ::duckdb::Result<::duckdb::Connection> {
    let config = ::duckdb::Config::default()
        .with("autoload_known_extensions", "false")?
        .with("autoinstall_known_extensions", "false")?;
    match path {
        Some(path) => ::duckdb::Connection::open_with_flags(path, config),
        None => ::duckdb::Connection::open_in_memory_with_flags(config),
    }
}
pub use pii::{mask_json_pii, mask_pii, PiiFinding, PiiKind, PiiMaskResult};
pub use sqlite::{DocumentExtractionFinalization, DuckDbRepository};
pub use vector::VectorIndex;

pub mod review;

pub mod financial;

pub mod facilities;

pub mod bundle;
pub mod case_scope;
pub mod casework;
pub mod casework_operations;

pub mod monitoring;

pub mod watchlists;

pub mod findings;

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
}
