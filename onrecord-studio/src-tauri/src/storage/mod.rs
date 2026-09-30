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
pub mod sqlite;
#[cfg(test)]
pub(crate) mod sqlite_snapshot;
pub(crate) mod tabular;
pub mod vector;

pub use duckdb::DuckDbWorkspace;
pub use pii::{mask_json_pii, mask_pii, PiiFinding, PiiKind, PiiMaskResult};
pub use sqlite::{DocumentExtractionFinalization, DuckDbRepository};
pub use vector::VectorIndex;

pub mod review;

pub mod financial;

pub mod facilities;

pub mod casework;

pub mod monitoring;

pub mod watchlists;

pub mod findings;
