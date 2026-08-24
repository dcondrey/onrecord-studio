//! Persistence layer orchestration tree registration.

pub mod sqlite;
pub mod duckdb;
pub mod fts;

pub use sqlite::SqliteRepository;
pub use duckdb::DuckDbWorkspace;
