//! Where each imported row's original cells live, and the `analytical_rows` view every reader
//! queries.
//!
//! Snapshots imported before the column store keep their rows in `analytical_row_ledger`, one JSON
//! document per row. A newer snapshot keeps its original cells in its own `analysis_src_*` table,
//! one text column per source column, which DuckDB compresses column by column. `analytical_rows`
//! is a view over both that builds the legacy row shape on read; a `snapshot_id` filter prunes
//! every other snapshot's branch, so a reader of one snapshot scans only that snapshot.

use super::*;
use crate::storage::{quote_ident, sql_literal};

/// Validation-summary key naming a snapshot's column store.
pub(super) const ROW_STORE_KEY: &str = "rowStore";

/// The digest of a row's original fields, in order, including surplus fields. A row the parser
/// could not read has no fields and hashes as JSON `null`.
pub(super) fn row_hash(fields: Option<&csv::StringRecord>) -> String {
    /// The JSON is written straight into the digest rather than built as a string first.
    struct Digesting(sha2::Sha256);
    impl std::io::Write for Digesting {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            sha2::Digest::update(&mut self.0, bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    struct Fields<'a>(&'a csv::StringRecord);
    impl serde::Serialize for Fields<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.collect_seq(self.0.iter())
        }
    }
    let mut digest = Digesting(<sha2::Sha256 as sha2::Digest>::new());
    match fields {
        Some(record) => serde_json::to_writer(&mut digest, &Fields(record)),
        None => serde_json::to_writer(&mut digest, &serde_json::Value::Null),
    }
    .expect("strings always serialize");
    crate::integrity::digest_hex(sha2::Digest::finalize(digest.0))
}

fn storage(error: impl std::fmt::Display) -> AppError {
    AppError::Storage(format!("Analytical row store: {error}"))
}

/// Renames a pre-column-store `analytical_rows` table to `analytical_row_ledger`, so the name can
/// become the view. DuckDB refuses to rename a table an index depends on, so the secondary index
/// is dropped first; the base schema recreates it on the new name. Runs before the base schema.
pub(super) fn move_legacy_ledger(conn: &mut Connection) -> Result<(), AppError> {
    let legacy: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM duckdb_tables() WHERE schema_name = 'main' AND table_name = 'analytical_rows')",
            [],
            |row| row.get(0),
        )
        .map_err(storage)?;
    if !legacy {
        return Ok(());
    }
    let indexes = conn
        .prepare("SELECT index_name FROM duckdb_indexes() WHERE schema_name = 'main' AND table_name = 'analytical_rows'")
        .map_err(storage)?
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let tx = conn.transaction().map_err(storage)?;
    for index in indexes {
        tx.execute_batch(&format!("DROP INDEX {}", quote_ident(&index)))
            .map_err(storage)?;
    }
    tx.execute_batch("ALTER TABLE analytical_rows RENAME TO analytical_row_ledger")
        .map_err(storage)?;
    tx.commit().map_err(storage)
}

/// Recreates `analytical_rows` from the ledger and every snapshot whose column store and typed
/// table both still exist; a retired snapshot keeps its catalog row after its tables are dropped.
/// Called after the base schema at startup and inside every transaction that adds or removes a
/// snapshot, so the view never names a dropped table once that transaction commits.
pub(crate) fn rebuild_row_view(conn: &Connection) -> Result<(), AppError> {
    let stores = conn
        .prepare(&format!(
            "SELECT s.snapshot_id, s.table_name, json_extract_string(s.validation_summary_json, '$.{ROW_STORE_KEY}')
             FROM analytical_snapshots s
             WHERE EXISTS (SELECT 1 FROM duckdb_tables() t WHERE t.schema_name = 'main'
                     AND t.table_name = json_extract_string(s.validation_summary_json, '$.{ROW_STORE_KEY}'))
                 AND EXISTS (SELECT 1 FROM duckdb_tables() t WHERE t.schema_name = 'main' AND t.table_name = s.table_name)
             ORDER BY s.snapshot_id"
        ))
        .map_err(storage)?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let mut branches = vec![
        "SELECT snapshot_id, row_number, row_hash, source_payload_json, normalized_payload_json, malformed, validation_errors_json FROM analytical_row_ledger".to_string(),
    ];
    for (snapshot, typed, source) in stores {
        let columns = conn
            .prepare("SELECT column_name, COALESCE(original_name, column_name) FROM analytical_columns WHERE snapshot_id = ?1 ORDER BY ordinal")
            .map_err(storage)?
            .query_map(params![snapshot], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        branches.push(branch_sql(&snapshot, &typed, &source, &columns));
    }
    conn.execute_batch(&format!(
        "CREATE OR REPLACE VIEW analytical_rows AS {}",
        branches.join(" UNION ALL ")
    ))
    .map_err(storage)
}

/// One snapshot's rows in the ledger's shape. Original values are the row's fields in order,
/// surplus fields included and absent ones left out; converted values come from the typed table,
/// which holds only rows that were not rejected.
fn branch_sql(snapshot: &str, typed: &str, source: &str, columns: &[(String, String)]) -> String {
    let headers = serde_json::to_string(&columns.iter().map(|(_, h)| h).collect::<Vec<_>>())
        .expect("strings always serialize");
    let cells = columns
        .iter()
        .map(|(alias, _)| format!("s.{}", quote_ident(alias)))
        .collect::<Vec<_>>()
        .join(", ");
    let converted = columns
        .iter()
        .map(|(alias, _)| format!("{}, t.{}", sql_literal(alias), quote_ident(alias)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT {snapshot} AS snapshot_id, s._row AS row_number, s._row_hash AS row_hash,
            CASE WHEN s._parse_error IS NOT NULL THEN 'null'
                 ELSE json_object('headers', {headers}::JSON,
                      'values', to_json(list_slice(list_concat([{cells}], COALESCE(CAST(s._surplus::JSON AS VARCHAR[]), []::VARCHAR[])), 1, s._field_count)))::VARCHAR
            END AS source_payload_json,
            CASE WHEN s._malformed THEN '{{}}' ELSE json_object({converted})::VARCHAR END AS normalized_payload_json,
            CAST(s._malformed AS BIGINT) AS malformed, s._messages AS validation_errors_json
         FROM {source_table} s LEFT JOIN {typed_table} t ON t._source_row = s._row",
        snapshot = sql_literal(snapshot),
        headers = sql_literal(&headers),
        source_table = quote_ident(source),
        typed_table = quote_ident(typed),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AnalyticalDatasetKind, AnalyticalImportRequest};

    const SCHEMA: &str = include_str!("../../../migrations/0001_initial_schema.sql");

    fn request(path: &std::path::Path) -> AnalyticalImportRequest {
        AnalyticalImportRequest {
            dataset_kind: AnalyticalDatasetKind::NibrsOffenses,
            display_name: "rows".into(),
            source_uri: None,
            local_path: path.to_string_lossy().into_owned(),
            observation_period_start: None,
            observation_period_end: None,
            partial_dataset: false,
            worksheet: None,
            source_file_name: None,
            expected_source_sha256: None,
            derived_subset: None,
        }
    }

    async fn rows(repo: &DuckDbRepository, snapshot: &str) -> Vec<Vec<serde_json::Value>> {
        repo.execute_restricted_sql(
            &format!("SELECT row_number, row_hash, source_payload_json, normalized_payload_json, malformed, validation_errors_json FROM analytical_rows WHERE snapshot_id = '{snapshot}' ORDER BY row_number"),
            100,
        )
        .await
        .unwrap()
        .rows
    }

    /// Values recorded when the column store shipped. Duplicate detection and row citations
    /// compare these digests, so the bytes hashed must not change.
    #[test]
    fn row_hashes_match_those_already_stored() {
        let record = csv::StringRecord::from(vec!["1", " padded ", "", "O'Brien, Jr."]);
        assert_eq!(
            row_hash(Some(&record)),
            "aaf032ad070fb9f2b6a51d5852b4d351c576af881b322e5ade74961c486346eb"
        );
        assert_eq!(
            row_hash(None),
            "74234e98afe7498fb5daf1f36ac2d78acc339464f950703b8c019892f982b90b"
        );
    }

    /// A snapshot's id is its identity across re-imports: the same file must land on the same id.
    #[tokio::test]
    async fn a_snapshot_id_matches_the_one_already_stored_for_the_same_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rows.csv");
        std::fs::write(&path, "a,b\n1,2\n").unwrap();
        let repo = DuckDbRepository::new(":memory:").unwrap();
        repo.run_migrations(SCHEMA).await.unwrap();
        let mut input = request(&path);
        input.dataset_kind = AnalyticalDatasetKind::RegionalData;
        let result = repo.import_analytical_csv(input).await.unwrap();
        assert_eq!(
            result.source_sha256,
            "492d5ea496056f1a6a6592241032fab764c321596317930b4fa0e1e8bc3b7470"
        );
        assert_eq!(result.snapshot_id, "regional-data-30c4330af2925520decf8b0b");
    }

    /// Every field of every row, rejected or not, comes back from the column store exactly as the
    /// source wrote it: that is what lets a released source file be represented by its snapshot.
    #[tokio::test]
    async fn the_view_returns_every_original_field_of_every_row() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rows.csv");
        let text = "nibrs_uniq,case_number,occured_on\n1, padded ,2025-01-02\n2,x\n3,y,2025-01-03,,\n4,z,2025-01-04,extra\n5,w,soon\n";
        std::fs::write(&path, text).unwrap();
        let repo = DuckDbRepository::new(":memory:").unwrap();
        repo.run_migrations(SCHEMA).await.unwrap();
        let result = repo.import_analytical_csv(request(&path)).await.unwrap();
        let stored = rows(&repo, &result.snapshot_id).await;
        let mut reader = csv::ReaderBuilder::new()
            .flexible(true)
            .from_reader(text.as_bytes());
        let expected = reader.records().map(Result::unwrap).collect::<Vec<_>>();
        assert_eq!(stored.len(), expected.len());
        for (row, record) in stored.iter().zip(&expected) {
            let source: serde_json::Value = serde_json::from_str(row[2].as_str().unwrap()).unwrap();
            assert_eq!(
                source["values"],
                serde_json::json!(record.iter().collect::<Vec<_>>())
            );
            assert_eq!(
                source["headers"],
                serde_json::json!(["nibrs_uniq", "case_number", "occured_on"])
            );
            assert_eq!(row[1], serde_json::json!(row_hash(Some(record))));
        }
        let malformed = stored
            .iter()
            .map(|row| row[4].as_i64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(malformed, vec![0, 1, 0, 1, 0]);
        let converted: serde_json::Value =
            serde_json::from_str(stored[4][3].as_str().unwrap()).unwrap();
        assert_eq!(converted["occured_on"], serde_json::Value::Null);
        assert!(stored[4][5]
            .as_str()
            .unwrap()
            .contains("could not parse date 'soon'"));
        assert!(stored[1][5]
            .as_str()
            .unwrap()
            .contains("Expected 3 fields, found 2"));
    }

    #[tokio::test]
    async fn an_upgraded_core_keeps_its_ledger_rows_beside_new_column_stores() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("core.db");
        {
            // The pre-column-store schema, in a file at the storage version older cores carry.
            let legacy = SCHEMA
                .replace(
                    "analytical_row_ledger_snapshot_hash",
                    "analytical_rows_snapshot_hash",
                )
                .replace("analytical_row_ledger", "analytical_rows");
            let conn = duckdb::Connection::open(&path).unwrap();
            conn.execute_batch(&legacy).unwrap();
            conn.execute_batch(
                "INSERT INTO analytical_snapshots (snapshot_id, dataset_kind, display_name, table_name, local_path, source_sha256, retrieved_at, validation_summary_json)
                 VALUES ('old', 'REGIONAL_DATA', 'old', 'analysis_old', '/old', repeat('a', 64), '2026-01-01', '{}');
                 INSERT INTO analytical_rows VALUES ('old', 1, 'h', '{\"values\":[\"1\"]}', '{\"a\":1}', 0, '[]');",
            )
            .unwrap();
        }
        let repo = DuckDbRepository::new(&path).unwrap();
        repo.run_migrations(SCHEMA).await.unwrap();
        let csv = directory.path().join("rows.csv");
        std::fs::write(&csv, "nibrs_uniq,case_number\n1,10\n").unwrap();
        let result = repo.import_analytical_csv(request(&csv)).await.unwrap();
        assert_eq!(rows(&repo, "old").await[0][1], serde_json::json!("h"));
        assert_eq!(rows(&repo, &result.snapshot_id).await.len(), 1);
        drop(repo);
        // A second start finds the ledger already moved and leaves both readable.
        let repo = DuckDbRepository::new(&path).unwrap();
        repo.run_migrations(SCHEMA).await.unwrap();
        assert_eq!(rows(&repo, "old").await.len(), 1);
        assert_eq!(rows(&repo, &result.snapshot_id).await.len(), 1);
        let indexed = repo
            .query_connection("indexes", |conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM duckdb_indexes() WHERE table_name = 'analytical_row_ledger'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(storage)
            })
            .await
            .unwrap();
        assert_eq!(indexed, 1);
    }
}
