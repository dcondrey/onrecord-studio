use onrecord_studio_lib::{
    storage::DuckDbRepository,
    types::{AnalyticalImportRequest, SqlQueryResult},
};
use serde_json::{json, Value};

fn cell<'a>(result: &'a SqlQueryResult, row: usize, name: &str) -> &'a Value {
    &result.rows[row][result.columns.iter().position(|c| c == name).unwrap()]
}

fn count(result: &SqlQueryResult, row: usize, name: &str) -> i64 {
    let value = cell(result, row, name);
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or_else(|| panic!("{name} is not a count: {value}"))
}

fn row_of(result: &SqlQueryResult, pairs: &[(&str, Value)]) -> usize {
    (0..result.row_count)
        .find(|row| {
            pairs.iter().all(|(column, value)| {
                let found = cell(result, *row, column);
                found == value || found.as_str() == Some(value.to_string().as_str())
            })
        })
        .unwrap_or_else(|| panic!("no row with {pairs:?}"))
}

#[tokio::test]
async fn call_checks_count_each_incident_once_from_the_latest_copy_of_each_year() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/call-leads.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("calls.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let mut tables = Vec::new();
    for dataset in fixture["schema"]["datasets"].as_array().unwrap() {
        let name = dataset["snapshotId"].as_str().unwrap();
        let path = temp.path().join(format!("{name}.csv"));
        std::fs::write(&path, fixture["csv"][name].as_str().unwrap()).unwrap();
        let kind = match dataset["datasetKind"].as_str().unwrap() {
            "CALLS_FOR_SERVICE" => "CALLS_FOR_SERVICE",
            _ if name == "cfs_beats" => "REGIONAL_DATA",
            _ => "CALLS_CODEBOOK",
        };
        let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":kind, "displayName":name, "localPath":path, "sourceUri":dataset["sourceUri"], "partialDataset":true })).unwrap();
        let imported = db.import_analytical_csv(input).await.unwrap();
        assert_eq!(imported.rejected_rows, 0, "{name}");
        tables.push((
            format!("\"{}\"", dataset["tableName"].as_str().unwrap()),
            format!("\"{}\"", imported.table_name),
        ));
    }
    let run = |sql: String| {
        let mut sql = sql;
        for (placeholder, table) in &tables {
            sql = sql.replace(placeholder, table);
        }
        sql
    };
    // The call-type list starts with a BOM and ends its header with three blank names.
    let types = db
        .execute_restricted_sql(
            &run(
                "SELECT description FROM \"analysis_cfs_types\" WHERE call_type = 'SLEEPER'".into(),
            ),
            100,
        )
        .await
        .unwrap();
    assert_eq!(cell(&types, 0, "description"), &json!("SLEEPER"));
    for query in fixture["queries"].as_array().unwrap() {
        let id = query["id"].as_str().unwrap();
        let raw = query["sql"].as_str().unwrap();
        assert!(
            !raw.contains("analysis_cfs_prior_2026"),
            "{id} reads the earlier copy of 2026"
        );
        let sql = run(raw.to_string());
        let result = db
            .execute_restricted_sql(&sql, 100)
            .await
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        match id {
            "calls_outcomes" => {
                assert_eq!(result.row_count, 5);
                // E25001 is entered twice and counted once, as its later arrest; the lower-case
                // sleeper is a SLEEPER call. AHR is an arrest and a Crisis Response Team call.
                // E25011's two entries share a time but not a disposition: conflicting, not A.
                let sleeper_2025 = row_of(
                    &result,
                    &[("call_year", json!(2025)), ("call_type", json!("SLEEPER"))],
                );
                for (column, expected) in [
                    ("calls", 7),
                    ("cancelled_duplicate", 1),
                    ("blank", 1),
                    ("conflicting", 1),
                    ("arrest", 2),
                    ("report", 0),
                    ("no_report", 2),
                    ("unfounded", 0),
                    ("other", 0),
                    ("crisis_team", 2),
                ] {
                    assert_eq!(
                        count(&result, sleeper_2025, column),
                        expected,
                        "2025 {column}"
                    );
                }
                // Only the latest copy of 2026: E26001 as re-coded K, E26099 gone from the file.
                let sleeper_2026 = row_of(
                    &result,
                    &[("call_year", json!(2026)), ("call_type", json!("SLEEPER"))],
                );
                for (column, expected) in [
                    ("calls", 6),
                    ("cancelled_duplicate", 2),
                    ("blank", 0),
                    ("conflicting", 0),
                    ("arrest", 0),
                    ("report", 1),
                    ("no_report", 2),
                    ("other", 1),
                    ("crisis_team", 0),
                ] {
                    assert_eq!(
                        count(&result, sleeper_2026, column),
                        expected,
                        "2026 {column}"
                    );
                }
                assert_eq!(
                    cell(&result, sleeper_2026, "other_codes"),
                    &json!("O (OTHER)")
                );
                // 5150V and 415 are not read; CW, 602 and 647F are.
                for (year, call_type) in [(2025, "CW"), (2025, "602"), (2026, "647F")] {
                    let row = row_of(
                        &result,
                        &[("call_year", json!(year)), ("call_type", json!(call_type))],
                    );
                    assert_eq!(count(&result, row, "calls"), 1, "{year} {call_type}");
                }
            }
            "calls_sleeper_beats" => {
                assert_eq!(result.row_count, 3);
                assert_eq!(cell(&result, 0, "beat"), &json!("524"));
                assert_eq!(cell(&result, 0, "neighborhood"), &json!("Core-Columbia"));
                assert_eq!(count(&result, 0, "calls"), 4);
                assert_eq!(count(&result, 1, "calls"), 3);
                // A beat missing from the list keeps its calls and has no neighborhood.
                assert_eq!(cell(&result, 2, "beat"), &json!("999"));
                assert!(cell(&result, 2, "neighborhood").is_null());
                assert_eq!(count(&result, 2, "calls"), 2);
                assert_eq!(count(&result, 0, "citywide_calls"), 10);
                assert_eq!(count(&result, 0, "no_beat"), 1);
            }
            _ => panic!("Unverified query {id}"),
        }
    }
}
