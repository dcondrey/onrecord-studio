//! Helpers shared by unit tests and integration tests. Compiled only under `cfg(test)` or the
//! `test-support` feature, which the crate's own dev-dependency enables for `tests/`.

use crate::{error::AppError, storage::DuckDbRepository};
use duckdb::params;

/// Registers an existing analytics table as an imported dataset: one `analytical_snapshots`
/// row under `snapshot_id` with the given source URI, and one `analytical_columns` row per
/// column read back from `information_schema`, DECIMAL columns typed as such and the rest as
/// TEXT. The source hash is a fixed fill of `hash_fill`, so two registrations can be told apart.
pub async fn register_analytical_table(
    repository: &DuckDbRepository,
    snapshot_id: &str,
    table: &str,
    source_uri: &str,
    hash_fill: char,
) -> Result<(), AppError> {
    let conn = repository.analytics_connection().await?;
    let storage = |error: duckdb::Error| AppError::Storage(format!("test registration: {error}"));
    let rows: i64 = conn
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .map_err(storage)?;
    conn.execute(
        "INSERT INTO analytical_snapshots(snapshot_id,dataset_kind,display_name,table_name,source_uri,local_path,source_sha256,retrieved_at,validation_summary_json,imported_rows) VALUES (?1,'OPERATING_BUDGET',?1,?2,?3,'/fixture',repeat(?4,64),'2026-09-26','{}',?5)",
        params![snapshot_id, table, source_uri, hash_fill.to_string(), rows],
    )
    .map_err(storage)?;
    let mut statement = conn
        .prepare("SELECT column_name, data_type, ordinal_position FROM information_schema.columns WHERE table_name = ?1 ORDER BY ordinal_position")
        .map_err(storage)?;
    let columns = statement
        .query_map(params![table], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    for (name, kind, ordinal) in columns {
        let logical = if kind.starts_with("DECIMAL") {
            "DECIMAL"
        } else {
            "TEXT"
        };
        conn.execute(
            "INSERT INTO analytical_columns(snapshot_id,table_name,column_name,logical_type,ordinal) VALUES (?1,?2,?3,?4,?5)",
            params![snapshot_id, table, name, logical, ordinal],
        )
        .map_err(storage)?;
    }
    Ok(())
}
