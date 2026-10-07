use onrecord_studio_lib::{
    storage::DuckDbRepository,
    types::{AnalyticalImportRequest, SqlQueryResult},
};
use serde_json::{json, Value};

fn cell<'a>(result: &'a SqlQueryResult, row: usize, name: &str) -> &'a Value {
    &result.rows[row][result.columns.iter().position(|c| c == name).unwrap()]
}

fn number(result: &SqlQueryResult, row: usize, name: &str) -> f64 {
    let value = cell(result, row, name);
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or_else(|| panic!("{name} is not a number: {value}"))
}

fn year_row(result: &SqlQueryResult, year: i64) -> usize {
    (0..result.row_count)
        .find(|row| {
            let found = cell(result, *row, "issued_year");
            found == &json!(year) || found.as_str() == Some(year.to_string().as_str())
        })
        .unwrap_or_else(|| panic!("no row for {year}"))
}

#[tokio::test]
async fn housing_check_counts_units_once_per_job_on_issued_building_permits() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/housing-leads.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("housing.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let mut tables = Vec::new();
    for dataset in fixture["schema"]["datasets"].as_array().unwrap() {
        let name = dataset["snapshotId"].as_str().unwrap();
        let path = temp.path().join(format!("{name}.csv"));
        std::fs::write(&path, fixture["csv"][name].as_str().unwrap()).unwrap();
        let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"REGIONAL_DATA", "displayName":name, "localPath":path, "sourceUri":dataset["sourceUri"], "partialDataset":true })).unwrap();
        let imported = db.import_analytical_csv(input).await.unwrap();
        assert_eq!(imported.rejected_rows, 0, "{name}");
        tables.push((
            format!("\"{}\"", dataset["tableName"].as_str().unwrap()),
            format!("\"{}\"", imported.table_name),
        ));
    }
    let run = |sql: &str| {
        let mut sql = sql.to_string();
        for (placeholder, table) in &tables {
            sql = sql.replace(placeholder, table);
        }
        sql
    };
    // JOB-100003's tier cells are blank in the file and import as NULL, not 0.
    let blank = db
        .execute_restricted_sql(
            &run("SELECT COUNT(*) AS blank FROM \"analysis_approvals_issued_2025\" WHERE job_id = 'JOB-100003' AND approval_du_extremely_low IS NULL AND approval_du_above_moderate IS NULL"),
            10,
        )
        .await
        .unwrap();
    assert_eq!(number(&blank, 0, "blank"), 1.0);
    let [query] = fixture["queries"].as_array().unwrap().as_slice() else {
        panic!("expected one query");
    };
    let raw = query["sql"].as_str().unwrap();
    for unread in [
        "analysis_approvals_prior_2025",
        "analysis_approvals_created_2025",
        "analysis_approvals_closed_2025",
    ] {
        assert!(!raw.contains(unread), "reads {unread}");
    }
    let result = db
        .execute_restricted_sql(&run(raw), 100)
        .await
        .unwrap_or_else(|e| panic!("housing_units_by_tier: {e}"));
    assert_eq!(result.row_count, 2);
    // JOB-060768's 224 units recur on its Coastal and Site Development approvals and are counted
    // nowhere. JOB-100001's two Building Permits count once; the blank job's units are in no total.
    // JOB-100005 has permits in 2024 and 2025 and is counted in 2024 only. JOB-100010 records 0 in
    // every tier: it has a tier recorded and no unit.
    let issued_2025 = year_row(&result, 2025);
    for (column, expected) in [
        ("jobs", 4.0),
        ("tiered_jobs", 3.0),
        ("unit_jobs", 2.0),
        ("multi_permit_jobs", 1.0),
        ("extremely_low", 2.0),
        ("very_low", 3.0),
        ("low", 4.0),
        ("moderate", 5.0),
        ("above_moderate", 11.0),
        ("bonus", 6.0),
        ("adu", 2.0),
        ("no_job_permits", 1.0),
    ] {
        assert_eq!(
            number(&result, issued_2025, column),
            expected,
            "2025 {column}"
        );
    }
    // Every tier cell on a 2024 job is blank: the sums are NULL, not 0.
    let issued_2024 = year_row(&result, 2024);
    assert_eq!(number(&result, issued_2024, "jobs"), 3.0);
    assert_eq!(number(&result, issued_2024, "tiered_jobs"), 0.0);
    assert_eq!(number(&result, issued_2024, "unit_jobs"), 0.0);
    for column in [
        "extremely_low",
        "very_low",
        "low",
        "moderate",
        "above_moderate",
        "bonus",
        "adu",
    ] {
        assert_eq!(
            cell(&result, issued_2024, column),
            &Value::Null,
            "2024 {column}"
        );
    }
}
