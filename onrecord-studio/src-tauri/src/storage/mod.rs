//! Persistence layer orchestration tree registration.
//!
//! The application core is DuckDB. [`sqlite::DuckDbRepository`] owns the single durable writer and
//! every canonical table. The module retains its historical `sqlite` filename to avoid a noisy
//! source-file move; its public repository terminology reflects the actual engine.
//!
//! SQLite appears only at boundaries, never in the core:
//!
//! * [`legacy_import`] upgrades a pre-DuckDB core written by an earlier release.
//! * [`duckdb::DuckDbWorkspace`] and `analysis::anomaly` can `ATTACH` a read-only SQLite file an
//!   investigator supplies, so it can be analyzed without being imported.
//! * [`fts`] carries a SQLite FTS5 query sanitizer that no core path calls; see its module docs.
//!
//! The core search projection, `documents_fts`, is a plain DuckDB table maintained transactionally
//! by the repository. It is not an FTS5 virtual table and provides no tokenization or ranking.
//!
//! See `docs/storage-and-evidence-architecture.md`.

pub mod corpus;
pub mod duckdb;
pub mod fts;
pub mod legacy_import;
pub mod pii;
pub mod sqlite;

pub use duckdb::DuckDbWorkspace;
pub use pii::{mask_json_pii, mask_pii, PiiFinding, PiiKind, PiiMaskResult};
pub use sqlite::{DocumentExtractionFinalization, DuckDbRepository};
