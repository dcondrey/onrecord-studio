//! Change records of the snapshot pack: what differs between two snapshots of one publisher file,
//! read from the original cell text the analytical import stores, never from typed values, so a
//! change of inferred type cannot pose as a change of what the publisher wrote.
//!
//! A table with a publisher key compares records by (key, occurrence of the key in file order), so
//! a key the publisher repeats is compared repeat by repeat rather than multiplied. Rows without a
//! key, and every row of a keyless table, compare by their cells in the columns both snapshots
//! have, with multiplicity: a row present twice before and once after is one removal.

use super::conformed::{literal, record_key};
use super::{ChangeSummary, PackFile, PackFileFormat};
use crate::error::AppError;
use crate::storage::sqlite::quote_ident;
use crate::storage::DuckDbRepository;
use crate::types::{AnalyticalDatasetKind, AnalyticalImportRequest};
use duckdb::{params, Connection};
use std::path::Path;

/// Stated on every row of a record that lists changed records whole.
const WHOLE_RECORDS_NOTE: &str = "More than half the records changed, so each changed record is \
    listed as removed and added rather than field by field";
/// The field name for cells past the last header.
const SURPLUS_FIELD: &str = "(fields past the header)";

fn storage(error: duckdb::Error) -> AppError {
    AppError::Storage(format!("Change record: {error}"))
}

/// One snapshot's original-cell store and its publisher columns as `(alias, header)`, in order.
struct Side {
    store: String,
    columns: Vec<(String, String)>,
}

fn side(conn: &Connection, snapshot_id: &str) -> Result<Side, AppError> {
    let store: Option<String> = conn
        .query_row(
            "SELECT json_extract_string(validation_summary_json, '$.rowStore')
             FROM analytical_snapshots WHERE snapshot_id = ?1",
            params![snapshot_id],
            |row| row.get(0),
        )
        .map_err(storage)?;
    let store = store.ok_or_else(|| {
        AppError::DataParsing(format!(
            "Snapshot {snapshot_id} has no stored original cells"
        ))
    })?;
    let columns = conn
        .prepare(
            "SELECT c.column_name, c.original_name FROM analytical_columns c
             JOIN duckdb_columns() d ON d.schema_name = 'main' AND d.table_name = ?2
                 AND d.column_name = c.column_name
             WHERE c.snapshot_id = ?1 AND c.original_name IS NOT NULL
             ORDER BY c.ordinal",
        )
        .map_err(storage)?
        .query_map(params![snapshot_id, store], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(Side { store, columns })
}

impl Side {
    fn alias(&self, header: &str) -> Option<&str> {
        self.columns
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(header))
            .map(|(alias, _)| alias.as_str())
    }

    /// Every row as `_row`, its key, the shared fields as `__f<i>`, the cells past the header,
    /// the shared cells as one text (`__shared`, a row's identity without a key), and the row's
    /// own cells as a JSON object of header to text.
    fn rows(&self, key: Option<&str>, shared: &[(String, &str, &str)], pick: usize) -> String {
        let key = key
            .and_then(|header| self.alias(header))
            .map_or("CAST(NULL AS VARCHAR)".to_owned(), |alias| {
                format!("NULLIF(trim(s.{}), '')", quote_ident(alias))
            });
        let cells_shared = shared
            .iter()
            .map(|field| {
                format!(
                    "s.{}",
                    quote_ident(if pick == 0 { field.1 } else { field.2 })
                )
            })
            .chain(["s._surplus".to_owned()])
            .collect::<Vec<_>>();
        let fields = cells_shared[..shared.len()]
            .iter()
            .enumerate()
            .map(|(index, cell)| format!("{cell} AS __f{index}"))
            .collect::<Vec<_>>();
        let cells = self
            .columns
            .iter()
            .map(|(alias, header)| format!("{}, s.{}", literal(Some(header)), quote_ident(alias)))
            .collect::<Vec<_>>()
            .join(", ");
        let row = if cells.is_empty() {
            "'{}'".to_owned()
        } else {
            format!("json_object({cells})::VARCHAR")
        };
        let fields = if fields.is_empty() {
            String::new()
        } else {
            format!("{}, ", fields.join(", "))
        };
        format!(
            "SELECT s._row, {key} AS __key, {fields}s._surplus AS __surplus,
                 to_json([{}])::VARCHAR AS __shared, {row} AS __json
             FROM {} s",
            cells_shared.join(", "),
            quote_ident(&self.store)
        )
    }
}

/// How many rows of each kind a change record holds.
struct Counts {
    rows: u64,
    added: u64,
    removed: u64,
    changed: u64,
    whole_records: bool,
}

/// Writes the change record from snapshot `before` to `after` of `table` to `target` as Parquet.
fn compare(
    conn: &Connection,
    table: &str,
    before: &str,
    after: &str,
    target: &str,
) -> Result<Counts, AppError> {
    let (previous, current) = (side(conn, before)?, side(conn, after)?);
    // Compared only where both snapshots have the column: a header on one side is reported once,
    // not as a value its absence would stand for in every row.
    let shared: Vec<(String, &str, &str)> = previous
        .columns
        .iter()
        .filter_map(|(alias, header)| {
            current
                .alias(header)
                .map(|other| (header.clone(), alias.as_str(), other))
        })
        .collect();
    let key = record_key(table)?
        .filter(|header| previous.alias(header).is_some() && current.alias(header).is_some());
    let mut sql = format!(
        "CREATE OR REPLACE TEMP TABLE __before AS {};
         CREATE OR REPLACE TEMP TABLE __after AS {};
         CREATE OR REPLACE TEMP TABLE __changes (change VARCHAR, publisher_record_id VARCHAR,
             key_occurrence BIGINT, field VARCHAR, before VARCHAR, after VARCHAR, note VARCHAR);",
        previous.rows(key, &shared, 0),
        current.rows(key, &shared, 1),
    );
    for (kind, from, other) in [
        ("column_removed", &previous, &current),
        ("column_added", &current, &previous),
    ] {
        for (_, header) in &from.columns {
            if other.alias(header).is_none() {
                sql.push_str(&format!(
                    "INSERT INTO __changes (change, field) VALUES ('{kind}', {});",
                    literal(Some(header))
                ));
            }
        }
    }
    // Rows with no key, or every row of a keyless table: by their shared cells, one row per copy
    // gained or lost.
    sql.push_str(
        "INSERT INTO __changes (change, before, after)
         SELECT change, CASE WHEN change = 'removed' THEN __removed END,
                CASE WHEN change = 'added' THEN __added END
         FROM (
             SELECT CASE WHEN COALESCE(a.copies, 0) > COALESCE(b.copies, 0)
                         THEN 'added' ELSE 'removed' END AS change,
                    b.__json AS __removed, a.__json AS __added,
                    unnest(range(abs(COALESCE(a.copies, 0) - COALESCE(b.copies, 0)))) AS __copy
             FROM (SELECT __shared, count(*) AS copies, arg_min(__json, _row) AS __json
                   FROM __before WHERE __key IS NULL GROUP BY __shared) b
             FULL JOIN (SELECT __shared, count(*) AS copies, arg_min(__json, _row) AS __json
                   FROM __after WHERE __key IS NULL GROUP BY __shared) a USING (__shared)
             WHERE COALESCE(a.copies, 0) <> COALESCE(b.copies, 0)
         );",
    );
    let mut whole_records = false;
    if key.is_some() {
        let mut differences = shared
            .iter()
            .enumerate()
            .map(|(index, (header, _, _))| {
                format!(
                    "{{'f': {}, 'b': b.__f{index}, 'a': a.__f{index}}}",
                    literal(Some(header))
                )
            })
            .collect::<Vec<_>>();
        differences.push(format!(
            "{{'f': {}, 'b': b.__surplus, 'a': a.__surplus}}",
            literal(Some(SURPLUS_FIELD))
        ));
        sql.push_str(&format!(
            "CREATE OR REPLACE TEMP TABLE __kb AS SELECT *,
                 row_number() OVER (PARTITION BY __key ORDER BY _row) AS __n
             FROM __before WHERE __key IS NOT NULL;
             CREATE OR REPLACE TEMP TABLE __ka AS SELECT *,
                 row_number() OVER (PARTITION BY __key ORDER BY _row) AS __n
             FROM __after WHERE __key IS NOT NULL;
             CREATE OR REPLACE TEMP TABLE __fields AS
             SELECT __key, __n, d.f AS field, d.b AS before, d.a AS after FROM (
                 SELECT b.__key, b.__n, unnest([{}]) AS d
                 FROM __kb b JOIN __ka a ON a.__key = b.__key AND a.__n = b.__n
                 WHERE a.__shared <> b.__shared
             ) WHERE d.b IS DISTINCT FROM d.a;
             INSERT INTO __changes (change, publisher_record_id, key_occurrence, before)
             SELECT 'removed', __key, __n, __json FROM __kb b
             WHERE NOT EXISTS (SELECT 1 FROM __ka a WHERE a.__key = b.__key AND a.__n = b.__n);
             INSERT INTO __changes (change, publisher_record_id, key_occurrence, after)
             SELECT 'added', __key, __n, __json FROM __ka a
             WHERE NOT EXISTS (SELECT 1 FROM __kb b WHERE a.__key = b.__key AND a.__n = b.__n);",
            differences.join(", ")
        ));
        conn.execute_batch(&sql).map_err(storage)?;
        sql.clear();
        let (changed, records): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT count(*) FROM (SELECT DISTINCT __key, __n FROM __fields)),
                        greatest((SELECT count(*) FROM __kb), (SELECT count(*) FROM __ka))",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(storage)?;
        // Field by field, a whole-file rewrite would be every record times every column.
        whole_records = changed * 2 > records;
        sql.push_str(if whole_records {
            "INSERT INTO __changes (change, publisher_record_id, key_occurrence, before)
             SELECT 'removed', b.__key, b.__n, b.__json FROM __kb b WHERE EXISTS (
                 SELECT 1 FROM __fields f WHERE f.__key = b.__key AND f.__n = b.__n);
             INSERT INTO __changes (change, publisher_record_id, key_occurrence, after)
             SELECT 'added', a.__key, a.__n, a.__json FROM __ka a WHERE EXISTS (
                 SELECT 1 FROM __fields f WHERE f.__key = a.__key AND f.__n = a.__n);"
        } else {
            "INSERT INTO __changes (change, publisher_record_id, key_occurrence, field, before,
                 after)
             SELECT 'changed', __key, __n, field, before, after FROM __fields;"
        });
    }
    let note = if whole_records {
        format!(
            "UPDATE __changes SET note = {};",
            literal(Some(WHOLE_RECORDS_NOTE))
        )
    } else {
        String::new()
    };
    sql.push_str(&format!(
        "{note}
         COPY (SELECT * FROM __changes
               ORDER BY change, publisher_record_id, key_occurrence, field, before, after)
         TO {} (FORMAT PARQUET);",
        literal(Some(target))
    ));
    conn.execute_batch(&sql).map_err(storage)?;
    let count = |kind: &str| -> Result<u64, AppError> {
        conn.query_row(
            "SELECT count(*) FROM __changes WHERE change = ?1",
            params![kind],
            |row| row.get::<_, u64>(0),
        )
        .map_err(storage)
    };
    let counts = Counts {
        rows: conn
            .query_row("SELECT count(*) FROM __changes", [], |row| {
                row.get::<_, u64>(0)
            })
            .map_err(storage)?,
        added: count("added")?,
        removed: count("removed")?,
        changed: count("changed")?,
        whole_records,
    };
    conn.execute_batch(
        "DROP TABLE IF EXISTS __before; DROP TABLE IF EXISTS __after;
         DROP TABLE IF EXISTS __changes; DROP TABLE IF EXISTS __kb; DROP TABLE IF EXISTS __ka;
         DROP TABLE IF EXISTS __fields;",
    )
    .map_err(storage)?;
    Ok(counts)
}

/// Imports the earlier snapshot `previous` (its original at `previous_path`) into `scratch`
/// beside `current` (`originals/<table>/<stamp>/<file>`), already imported there as
/// `current_snapshot`, writes their change record to `changes/<table>/<stamp>.parquet` under
/// `out`, and returns its manifest entry.
pub(super) async fn write_change_record(
    scratch: &DuckDbRepository,
    out: &Path,
    previous: &PackFile,
    previous_path: &Path,
    current: &PackFile,
    current_snapshot: &str,
    kind: AnalyticalDatasetKind,
) -> Result<PackFile, AppError> {
    let stamp = Path::new(&current.path)
        .parent()
        .and_then(|directory| directory.file_name())
        .and_then(|stamp| stamp.to_str())
        .ok_or_else(|| AppError::Internal(format!("{} is not a dated snapshot", current.path)))?;
    let before = scratch
        .import_analytical_csv(AnalyticalImportRequest {
            dataset_kind: kind,
            display_name: previous.path.clone(),
            source_uri: Some(previous.source_url.clone()),
            local_path: previous_path.to_string_lossy().into_owned(),
            observation_period_start: None,
            observation_period_end: None,
            partial_dataset: previous.partial,
            worksheet: None,
            source_file_name: previous.path.rsplit('/').next().map(str::to_owned),
            expected_source_sha256: Some(previous.sha256.clone()),
            derived_subset: None,
        })
        .await?
        .snapshot_id;
    let relative = format!("changes/{}/{stamp}.parquet", current.table);
    let path = out.join(&relative);
    tokio::fs::create_dir_all(path.parent().expect("a file under changes"))
        .await
        .map_err(|error| AppError::Storage(format!("Pack directory: {error}")))?;
    let target = path
        .to_str()
        .ok_or_else(|| AppError::Storage(format!("{} is not UTF-8", path.display())))?
        .to_owned();
    let (table, after) = (current.table.clone(), current_snapshot.to_owned());
    let counts = scratch
        .query_connection("Change record", move |conn| {
            compare(conn, &table, &before, &after, &target)
        })
        .await?;
    let bytes = std::fs::metadata(&path)
        .map_err(|error| AppError::Storage(format!("Change record: {error}")))?
        .len();
    let sha256 = crate::integrity::sha256_file(&path)
        .map_err(|error| AppError::Storage(format!("Change record: {error}")))?;
    Ok(PackFile {
        path: relative,
        format: PackFileFormat::Changes,
        sha256,
        bytes,
        rows: counts.rows,
        excluded_rows: None,
        year: None,
        changes: Some(ChangeSummary {
            previous: previous.path.clone(),
            current: current.path.clone(),
            added: counts.added,
            removed: counts.removed,
            changed: counts.changed,
            whole_records: counts.whole_records,
        }),
        ..current.clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    type Row = (
        String,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );

    fn listed(table: &str, stamp: &str, bytes: &[u8]) -> PackFile {
        PackFile {
            path: format!("originals/{table}/{stamp}/file.csv"),
            table: table.into(),
            format: PackFileFormat::Original,
            source_url: "https://seshat.datasd.org/test/file.csv".into(),
            retrieved_at: "2026-10-07T00:00:00Z".into(),
            sha256: crate::integrity::sha256_hex(bytes),
            bytes: bytes.len() as u64,
            rows: 0,
            excluded_rows: None,
            year: None,
            partial: false,
            last_modified: None,
            etag: None,
            changes: None,
            licence: None,
        }
    }

    /// The change record from `before` to `after`, both files of `table`, and its rows.
    async fn change_record(table: &str, before: &str, after: &str) -> (ChangeSummary, Vec<Row>) {
        let out = tempfile::tempdir().unwrap();
        let (previous, current) = (
            listed(table, "A", before.as_bytes()),
            listed(table, "B", after.as_bytes()),
        );
        for (file, text) in [(&previous, before), (&current, after)] {
            let path = out.path().join(&file.path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let scratch = DuckDbRepository::new(":memory:").unwrap();
        scratch
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let imported = scratch
            .import_analytical_csv(AnalyticalImportRequest {
                dataset_kind: AnalyticalDatasetKind::RegionalData,
                display_name: current.path.clone(),
                source_uri: Some(current.source_url.clone()),
                local_path: out.path().join(&current.path).to_string_lossy().into(),
                observation_period_start: None,
                observation_period_end: None,
                partial_dataset: false,
                worksheet: None,
                source_file_name: Some("file.csv".into()),
                expected_source_sha256: None,
                derived_subset: None,
            })
            .await
            .unwrap();
        let record = write_change_record(
            &scratch,
            out.path(),
            &previous,
            &out.path().join(&previous.path),
            &current,
            &imported.snapshot_id,
            AnalyticalDatasetKind::RegionalData,
        )
        .await
        .unwrap();
        assert_eq!(record.path, format!("changes/{table}/B.parquet"));
        assert_eq!(record.format, PackFileFormat::Changes);
        let target = out.path().join(&record.path).to_string_lossy().into_owned();
        let rows = scratch
            .query_connection("read change record", move |conn| {
                let mut statement = conn
                    .prepare(
                        "SELECT change, publisher_record_id, key_occurrence, field, before, after,
                                note
                         FROM read_parquet(?1)",
                    )
                    .unwrap();
                Ok(statement
                    .query_map(params![target], |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                        ))
                    })
                    .unwrap()
                    .collect::<Result<Vec<Row>, _>>()
                    .unwrap())
            })
            .await
            .unwrap();
        assert_eq!(record.rows, rows.len() as u64);
        (record.changes.unwrap(), rows)
    }

    fn of_kind<'a>(rows: &'a [Row], kind: &str) -> Vec<&'a Row> {
        rows.iter().filter(|row| row.0 == kind).collect()
    }

    fn text(value: &str) -> Option<String> {
        Some(value.to_owned())
    }

    /// Records are compared by key and the key's repeat in file order, on the publisher's text:
    /// `01` to `1` is a change though both read as the same number, a repeated key is compared
    /// repeat by repeat rather than multiplied, and a column on one side is reported once rather
    /// than as a change in every row.
    #[tokio::test]
    async fn a_keyed_record_lists_each_difference_once_on_the_publishers_text() {
        let (summary, rows) = change_record(
            "call_types",
            "call_type,description,priority\nA,alpha,01\nB,beta,2\nB,beta2,2\nC,gamma,3\n\
             K,kappa,4\n,blank,5\n",
            "call_type,description,priority,extra\nA,alpha,1,x\nB,beta,2,x\nB,BETA2,2,x\n\
             B,beta3,2,x\nK,kappa,4,x\nK,kappa2,4,x\nD,delta,6,x\n,blank,5,x\n",
        )
        .await;
        assert_eq!(
            of_kind(&rows, "changed"),
            [
                &(
                    "changed".into(),
                    text("A"),
                    Some(1),
                    text("priority"),
                    text("01"),
                    text("1"),
                    None
                ),
                &(
                    "changed".into(),
                    text("B"),
                    Some(2),
                    text("description"),
                    text("beta2"),
                    text("BETA2"),
                    None
                ),
            ]
        );
        let keys = |kind| {
            of_kind(&rows, kind)
                .iter()
                .map(|row| (row.1.clone().unwrap(), row.2.unwrap()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            keys("added"),
            [
                ("B".to_owned(), 3),
                ("D".to_owned(), 1),
                ("K".to_owned(), 2)
            ]
        );
        assert_eq!(keys("removed"), [("C".to_owned(), 1)]);
        assert!(of_kind(&rows, "removed")[0]
            .4
            .as_deref()
            .unwrap()
            .contains("gamma"));
        assert_eq!(
            of_kind(&rows, "column_added"),
            [&(
                "column_added".into(),
                None,
                None,
                text("extra"),
                None,
                None,
                None
            )]
        );
        assert_eq!(rows.len(), 7);
        assert_eq!(
            (
                summary.added,
                summary.removed,
                summary.changed,
                summary.whole_records
            ),
            (3, 1, 2, false)
        );
    }

    /// Without a key, rows compare by their cells in the shared columns with multiplicity: a row
    /// present twice before and once after is one removal, shown as the earlier file held it.
    #[tokio::test]
    async fn a_keyless_record_counts_each_copy_gained_or_lost() {
        let (summary, rows) = change_record(
            "budget_departments",
            "dept,name\nX,a\nX,a\nY,b\n",
            "dept,name,extra\nX,a,1\nZ,c,2\n",
        )
        .await;
        let removed = of_kind(&rows, "removed")
            .iter()
            .map(|row| row.4.clone().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            removed,
            [r#"{"dept":"X","name":"a"}"#, r#"{"dept":"Y","name":"b"}"#]
        );
        assert_eq!(
            of_kind(&rows, "added")[0].5,
            text(r#"{"dept":"Z","name":"c","extra":"2"}"#)
        );
        assert_eq!(of_kind(&rows, "column_added")[0].3, text("extra"));
        assert_eq!((summary.added, summary.removed, summary.changed), (1, 2, 0));
    }

    /// Up to half the records changed is listed field by field; past half, each changed record is
    /// listed whole as removed and added, and every row says so.
    #[tokio::test]
    async fn past_half_the_records_changed_they_are_listed_whole() {
        let before = "call_type,description\nA,a\nB,b\nC,c\nD,d\n";
        let (half, rows) = change_record(
            "call_types",
            before,
            "call_type,description\nA,1\nB,2\nC,c\nD,d\n",
        )
        .await;
        assert_eq!((half.changed, half.whole_records), (2, false));
        assert!(rows.iter().all(|row| row.6.is_none()));

        let (most, rows) = change_record(
            "call_types",
            before,
            "call_type,description\nA,1\nB,2\nC,3\nD,d\n",
        )
        .await;
        assert_eq!(
            (most.added, most.removed, most.changed, most.whole_records),
            (3, 3, 0, true)
        );
        assert!(rows
            .iter()
            .all(|row| row.6.as_deref() == Some(WHOLE_RECORDS_NOTE)));

        // Shown to the app, the whole-records pair has no changed count rather than a 0.
        assert_eq!(most.exposed(None, None).changed, None);
        assert_eq!(half.exposed(None, None).changed, Some(2));
    }
}
