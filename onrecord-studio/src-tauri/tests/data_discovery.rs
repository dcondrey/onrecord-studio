use onrecord_studio_lib::{
    storage::DuckDbRepository,
    types::{AnalyticalImportRequest, SqlQueryResult},
};
use serde_json::{json, Value};

fn cell<'a>(result: &'a SqlQueryResult, row: usize, name: &str) -> &'a Value {
    &result.rows[row][result.columns.iter().position(|c| c == name).unwrap()]
}

#[tokio::test]
async fn generated_discovery_queries_run_through_the_native_boundary() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/discovery.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("analysis.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let path = temp.path().join("fixture.csv");
    std::fs::write(&path, fixture["csv"].as_str().unwrap()).unwrap();
    let input: AnalyticalImportRequest = serde_json::from_value(json!({
        "datasetKind":"REGIONAL_DATA", "displayName":"Discovery fixture", "localPath":path,
        "sourceUri":"https://example.test/fixture.csv", "partialDataset":true
    }))
    .unwrap();
    let imported = db.import_analytical_csv(input.clone()).await.unwrap();
    // Another snapshot must not be cited by a query that never reads it.
    let mut other = input;
    other.source_uri = Some("https://example.test/unrelated.csv".into());
    db.import_analytical_csv(other).await.unwrap();
    for query in fixture["queries"].as_array().unwrap() {
        let id = query["id"].as_str().unwrap();
        let sql = query["sql"]
            .as_str()
            .unwrap()
            .replace("analysis_fixture", &imported.table_name)
            .replace("'fixture'", &format!("'{}'", imported.snapshot_id));
        let result = db
            .execute_restricted_sql(&sql, 100)
            .await
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        if id != "duplicates" {
            assert_eq!(result.provenance.len(), 1, "{id} provenance");
        }
        match id {
            "coverage" => {
                assert_eq!(cell(&result, 0, "source_rows"), &json!(27));
                assert_eq!(cell(&result, 0, "missing_values"), &json!(1));
                assert_eq!(cell(&result, 0, "unusable_period_rows"), &json!(1));
                assert_eq!(cell(&result, 0, "last_period"), &json!(25));
            }
            "quality" => assert_eq!(result.row_count, 4),
            "groups" => {
                assert_eq!(cell(&result, 0, "group_value"), &json!("A"));
                assert_eq!(cell(&result, 0, "observed_value"), &json!("445"));
                assert!(
                    result.rows.iter().any(|row| row[0].is_null()),
                    "missing groups stay in denominator"
                );
            }
            "changes" => {
                assert_eq!(cell(&result, 0, "change"), &json!("-45"));
                for i in 0..result.row_count {
                    match (
                        cell(&result, i, "group_value").as_str(),
                        cell(&result, i, "discovery_period").as_i64(),
                    ) {
                        (Some("A"), Some(25)) => {
                            assert_eq!(
                                cell(&result, i, "comparison_status"),
                                &json!("Missing intervening periods")
                            );
                            assert!(cell(&result, i, "change").is_null());
                        }
                        (Some("B"), Some(23)) => {
                            assert!(cell(&result, i, "change_percent").is_null())
                        }
                        (Some("C"), Some(23)) => {
                            assert_eq!(
                                cell(&result, i, "comparison_status"),
                                &json!("Incomplete measurements")
                            );
                            assert!(cell(&result, i, "change").is_null());
                        }
                        _ => {}
                    }
                }
            }
            "outliers" => {
                assert_eq!(result.row_count, 1);
                assert_eq!(cell(&result, 0, "source: amount"), &json!(100));
                assert_eq!(cell(&result, 0, "total_flagged_rows"), &json!(1));
            }
            "duplicates" => {
                assert_eq!(result.row_count, 1);
                assert_eq!(cell(&result, 0, "occurrences"), &json!(10));
                assert!(cell(&result, 0, "source_cells")
                    .as_str()
                    .unwrap()
                    .contains("adopted"));
            }
            _ => panic!("Unverified query {id}"),
        }
    }
    // User-written CTE names work, while nested reads of unrelated evidence tables fail.
    assert!(db
        .execute_restricted_sql(
            &format!(
                "WITH budget AS (SELECT * FROM {}) SELECT COUNT(*) FROM budget",
                imported.table_name
            ),
            1
        )
        .await
        .is_ok());
    assert!(db
        .execute_restricted_sql(
            "WITH analysis_hidden AS (SELECT * FROM documents) SELECT * FROM analysis_hidden",
            1
        )
        .await
        .is_err());
}

#[tokio::test]
async fn calendar_discovery_preserves_gaps_and_applies_limits_after_aggregation() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/discovery.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("calendar.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let path = temp.path().join("calendar.csv");
    std::fs::write(&path, fixture["calendar"]["csv"].as_str().unwrap()).unwrap();
    let input = serde_json::from_value(json!({"datasetKind":"REGIONAL_DATA", "displayName":"Calendar", "localPath":path, "partialDataset":true})).unwrap();
    let imported = db.import_analytical_csv(input).await.unwrap();
    let query = |id| {
        fixture["calendar"]["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == id)
            .unwrap()["sql"]
            .as_str()
            .unwrap()
            .replace("analysis_fixture", &imported.table_name)
    };
    let changes = db
        .execute_restricted_sql(&query("changes"), 100)
        .await
        .unwrap();
    assert_eq!(cell(&changes, 0, "change"), &json!("-45"));
    assert_eq!(cell(&changes, 0, "discovery_period"), &json!("2026-02-01"));
    assert!(changes
        .rows
        .iter()
        .enumerate()
        .any(|(i, _)| cell(&changes, i, "comparison_status")
            == &json!("Missing intervening periods")));
    let groups = db
        .execute_restricted_sql(&query("groups"), 1)
        .await
        .unwrap();
    assert!(groups.truncated);
    assert_eq!(cell(&groups, 0, "source_rows"), &json!(21));
    assert_eq!(cell(&groups, 0, "observed_value"), &json!("445"));
    assert_eq!(
        cell(&groups, 0, "record_share_percent"),
        &json!(77.78),
        "denominator includes all 27 records even with a one-group display limit"
    );
}

#[tokio::test]
async fn automatic_startup_queries_run_on_real_imported_rows() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/automatic-discovery.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("automatic.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let path = temp.path().join("automatic.csv");
    std::fs::write(&path, fixture["csv"].as_str().unwrap()).unwrap();
    let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"REGIONAL_DATA", "displayName":"Automatic fixture", "localPath":path, "sourceUri":"https://example.test/automatic.csv", "partialDataset":true })).unwrap();
    let imported = db.import_analytical_csv(input).await.unwrap();
    for query in fixture["queries"].as_array().unwrap() {
        let sql = query["sql"]
            .as_str()
            .unwrap()
            .replace("analysis_fixture", &imported.table_name)
            .replace("'fixture'", &format!("'{}'", imported.snapshot_id));
        let result = db.execute_restricted_sql(&sql, 100).await.unwrap();
        assert_eq!(result.provenance.len(), 1);
        match query["id"].as_str().unwrap() {
            "quality" => {
                assert_eq!(result.row_count, 4);
                assert_eq!(cell(&result, 0, "source_rows"), &json!(28));
            }
            "coverage" => {
                assert_eq!(cell(&result, 0, "source_rows"), &json!(28));
                assert_eq!(cell(&result, 0, "last_period"), &json!(25));
            }
            "groups" => {
                assert_eq!(cell(&result, 0, "group_value"), &json!("A"));
                assert_eq!(cell(&result, 0, "source_rows"), &json!(22));
                assert_eq!(cell(&result, 0, "observed_value"), &json!(22));
            }
            "changes" => assert!(result.rows.iter().enumerate().any(|(i, _)| cell(
                &result,
                i,
                "comparison_status"
            )
                == "Missing intervening periods")),
            other => panic!("Unexpected automatic query {other}"),
        }
    }
}

#[tokio::test]
async fn service_report_checks_run_on_real_imported_rows() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/service-accountability.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("service.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let mut tables = Vec::new();
    for (csv, uri) in [
        ("earlierCsv", "https://example.test/service.csv"),
        ("csv", "https://example.test/service.csv"),
        ("closedCsv", "https://example.test/closed.csv"),
    ] {
        let path = temp.path().join(format!("{csv}.csv"));
        std::fs::write(&path, fixture[csv].as_str().unwrap()).unwrap();
        let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"REGIONAL_DATA", "displayName":"Service fixture", "localPath":path, "sourceUri":uri, "partialDataset":true })).unwrap();
        tables.push(db.import_analytical_csv(input).await.unwrap().table_name);
    }
    let budget_path = temp.path().join("budget.csv");
    std::fs::write(&budget_path, fixture["budgetCsv"].as_str().unwrap()).unwrap();
    let budget: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"OPERATING_BUDGET", "displayName":"Budget fixture", "localPath":budget_path, "sourceUri":"https://example.test/budget.csv", "partialDataset":false })).unwrap();
    let budget_table = db.import_analytical_csv(budget).await.unwrap().table_name;
    // A later copy of one source is a second table; it does not replace the earlier copy.
    assert_ne!(tables[0], tables[1]);
    let copies = db.analytical_schema().await.unwrap().datasets;
    assert_eq!(
        copies
            .iter()
            .filter(
                |dataset| dataset.source_uri.as_deref() == Some("https://example.test/service.csv")
            )
            .count(),
        2
    );
    let seen = db
        .imported_column_values(
            "https://example.test/service.csv".into(),
            "service_request_id".into(),
        )
        .await
        .unwrap();
    // 107 reports in the later copy plus the five that appear only in the earlier one.
    assert_eq!(seen.len(), 112);
    assert!(seen.contains("205") && !seen.contains("900"));
    let number = |result: &SqlQueryResult, row: usize, name: &str| -> f64 {
        let value = cell(result, row, name);
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .unwrap_or_else(|| panic!("{name} is not numeric: {value}"))
    };
    for query in fixture["queries"].as_array().unwrap() {
        let id = query["id"].as_str().unwrap();
        let sql = query["sql"]
            .as_str()
            .unwrap()
            .replace("analysis_budget_fixture", &budget_table)
            .replace("analysis_earlier_fixture", &tables[0])
            .replace("analysis_closed_fixture", &tables[2])
            .replace("analysis_fixture", &tables[1]);
        let result = db
            .execute_restricted_sql(&sql, 100)
            .await
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        match id {
            "service_backlog" => {
                // The five referred reports have no close date but are not an open backlog.
                assert_eq!(result.row_count, 1);
                assert_eq!(
                    cell(&result, 0, "category"),
                    &json!("Street Light Maintenance")
                );
                assert_eq!(number(&result, 0, "all_open_reports"), 12.0);
                assert_eq!(number(&result, 0, "all_open_over_one_year"), 12.0);
                assert_eq!(cell(&result, 0, "earliest_opened"), &json!("2024-01-10"));
                // One of the twelve sits under another staff group.
                assert_eq!(cell(&result, 0, "staff_group"), &json!("TSW"));
                assert_eq!(number(&result, 0, "staff_group_reports"), 11.0);
            }
            "service_budget" => {
                // Revenue accounts, the proposed cycle, the year with no cycle and departments
                // no staff group is read as are all left out.
                let rows: Vec<(String, f64, f64)> = (0..result.row_count)
                    .map(|row| {
                        (
                            cell(&result, row, "department")
                                .as_str()
                                .unwrap()
                                .to_string(),
                            number(&result, row, "fiscal_year"),
                            number(&result, row, "adopted_expense"),
                        )
                    })
                    .collect();
                assert_eq!(
                    rows,
                    [
                        ("Stormwater".to_string(), 25.0, 30.0),
                        ("Transportation".to_string(), 25.0, 150.0),
                        ("Transportation".to_string(), 24.0, 80.0),
                    ]
                );
            }
            "service_wait_area:council_district" => {
                assert_eq!(result.row_count, 1);
                assert_eq!(cell(&result, 0, "area"), &json!("3"));
                assert_eq!(number(&result, 0, "median_days"), 40.0);
                assert_eq!(number(&result, 0, "all_median_days"), 5.0);
            }
            "service_wait_area:comm_plan_name" => {
                // "GAMMA" and "Gamma" are one community; apart, neither reaches 30 reports.
                assert_eq!(result.row_count, 1);
                assert_eq!(cell(&result, 0, "area"), &json!("GAMMA"));
                assert_eq!(number(&result, 0, "closed_reports"), 30.0);
            }
            "service_backlog_area:council_district" | "service_backlog_area:comm_plan_name" => {
                assert_eq!(result.row_count, 0, "{id}: one area has open reports")
            }
            "service_repeat_places" => {
                assert_eq!(result.row_count, 1);
                assert_eq!(cell(&result, 0, "place"), &json!("1 Main St"));
                assert_eq!(number(&result, 0, "open_reports"), 12.0);
            }
            "service_referred" => {
                // A -999 age is an unrecorded age, not a reason to drop the report.
                assert_eq!(result.row_count, 1);
                assert_eq!(cell(&result, 0, "category"), &json!("Other"));
                assert_eq!(number(&result, 0, "referred_reports"), 5.0);
                assert_eq!(number(&result, 0, "all_reports"), 107.0);
            }
            "service_movement" => {
                // Report 1 is closed in the later copy itself; 205 is in no imported table.
                assert_eq!(result.row_count, 3);
                assert_eq!(cell(&result, 0, "category"), &json!("Pothole"));
                assert_eq!(number(&result, 0, "left_reports"), 5.0);
                assert_eq!(number(&result, 0, "left_over_one_year"), 3.0);
                assert_eq!(number(&result, 0, "closed_on_record"), 4.0);
                assert_eq!(number(&result, 0, "referred_on_record"), 1.0);
                assert_eq!(cell(&result, 1, "category"), &json!("Graffiti"));
                assert_eq!(number(&result, 1, "no_outcome_record"), 1.0);
                assert_eq!(
                    cell(&result, 2, "category"),
                    &json!("Street Light Maintenance")
                );
                assert_eq!(number(&result, 2, "arrived_reports"), 2.0);
                assert_eq!(number(&result, 0, "all_left_reports"), 6.0);
                assert_eq!(number(&result, 0, "all_no_outcome_record"), 1.0);
                assert_eq!(number(&result, 0, "all_arrived_reports"), 2.0);
            }
            _ => panic!("Unverified query {id}"),
        }
    }
}

#[tokio::test]
async fn stop_record_checks_run_on_imported_tables_with_the_publishers_headers() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/stop-leads.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("stops.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let mut tables = Vec::new();
    for name in ["stops", "results", "reasons", "property"] {
        let path = temp.path().join(format!("{name}.csv"));
        std::fs::write(&path, fixture["csv"][name].as_str().unwrap()).unwrap();
        let kind = if name == "stops" {
            "RIPA_STOPS"
        } else {
            "REGIONAL_DATA"
        };
        let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":kind, "displayName":name, "localPath":path, "sourceUri":format!("https://example.test/{name}.csv"), "partialDataset":true })).unwrap();
        let imported = db.import_analytical_csv(input).await.unwrap();
        assert_eq!(imported.rejected_rows, 0, "{name}");
        tables.push((format!("analysis_stop_{name}"), imported.table_name));
    }
    let count = |result: &SqlQueryResult, row: usize, name: &str| -> i64 {
        let value = cell(result, row, name);
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .unwrap_or_else(|| panic!("{name} is not a count: {value}"))
    };
    let row_of = |result: &SqlQueryResult, column: &str, value: Value| -> usize {
        (0..result.row_count)
            .find(|row| cell(result, *row, column) == &value)
            .unwrap_or_else(|| panic!("no row with {column} = {value}"))
    };
    for query in fixture["queries"].as_array().unwrap() {
        let id = query["id"].as_str().unwrap();
        let mut sql = query["sql"].as_str().unwrap().to_string();
        for (placeholder, table) in &tables {
            sql = sql.replace(placeholder, table);
        }
        let result = db
            .execute_restricted_sql(&sql, 100)
            .await
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        match id {
            "stops_unhoused_outcomes" => {
                assert_eq!(result.row_count, 3);
                let unhoused = row_of(&result, "perceived", json!("unhoused"));
                // One warrant arrest, one arrest without a warrant whose person was also cited.
                for (column, expected) in [
                    ("persons", 3),
                    ("arrested", 2),
                    ("arrested_on_warrant", 1),
                    ("arrested_without_warrant", 1),
                    ("cited", 1),
                    ("no_result_row", 0),
                    ("property_seized", 1),
                ] {
                    assert_eq!(count(&result, unhoused, column), expected, "{column}");
                }
                // The year-23 stop date is not the first stop.
                assert_eq!(cell(&result, unhoused, "first_stop"), &json!("2025-02-01"));
                let others = row_of(&result, "perceived", json!("not unhoused"));
                // Two persons in one stop are two persons; one has no result row.
                for (column, expected) in [
                    ("persons", 3),
                    ("arrested", 0),
                    ("cited", 2),
                    ("no_result_row", 1),
                    ("property_seized", 1),
                ] {
                    assert_eq!(count(&result, others, column), expected, "{column}");
                }
                // The older "0" entry is neither group.
                assert_eq!(
                    count(
                        &result,
                        row_of(&result, "perceived", Value::Null),
                        "persons"
                    ),
                    1
                );
            }
            "stops_unhoused_reasons" => {
                assert_eq!(result.row_count, 2);
                assert!(cell(&result, 0, "reason")
                    .as_str()
                    .unwrap()
                    .starts_with("647(E)"));
                assert_eq!(count(&result, 0, "unhoused_persons"), 2);
                // The warrant arrest is not an arrest without a warrant.
                assert_eq!(count(&result, 0, "unhoused_arrested"), 1);
                assert_eq!(count(&result, 0, "other_persons"), 1);
                assert_eq!(count(&result, 0, "other_arrested"), 0);
            }
            "stops_unhoused_property" => {
                // The seizure from a person not perceived as unhoused is not listed.
                assert_eq!(result.row_count, 2);
                assert_eq!(cell(&result, 0, "basis"), &json!("Evidence"));
                assert_eq!(count(&result, 0, "persons"), 1);
            }
            _ => panic!("Unverified query {id}"),
        }
    }
}

#[tokio::test]
async fn abatement_logs_pair_one_notice_with_one_abatement_in_the_released_format() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/abatement-leads.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("abatement.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let mut sql = fixture["queries"][0]["sql"].as_str().unwrap().to_string();
    for name in ["posted", "completed"] {
        let path = temp.path().join(format!("{name}.csv"));
        std::fs::write(&path, fixture["csv"][name].as_str().unwrap()).unwrap();
        let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"REGIONAL_DATA", "displayName":name, "localPath":path, "sourceUri":format!("https://example.test/{name}.csv"), "partialDataset":true })).unwrap();
        let imported = db.import_analytical_csv(input).await.unwrap();
        // "7/1/2025, 8:00 AM" is kept as recorded rather than refused as an unreadable date.
        assert_eq!(
            (imported.imported_rows, imported.rejected_rows),
            (7, 0),
            "{name}"
        );
        sql = sql.replace(&format!("analysis_abatement_{name}"), &imported.table_name);
    }
    let result = db.execute_restricted_sql(&sql, 100).await.unwrap();
    assert_eq!(result.row_count, 1);
    let number = |name: &str| -> f64 {
        let value = cell(&result, 0, name);
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .unwrap_or_else(|| panic!("{name} is not numeric: {value}"))
    };
    for (column, expected) in [
        ("posted_notices", 7.0),
        ("completed_abatements", 7.0),
        // The notice with no coordinates.
        ("notices_not_matchable", 1.0),
        // The place with two notices is not paired with its one abatement.
        ("paired_places", 4.0),
        ("completed_before_posted", 1.0),
        ("under_3_hours", 1.0),
        ("under_24_hours", 1.0),
        ("from_24_to_72_hours", 1.0),
        ("over_72_hours", 1.0),
        ("median_hours", 30.0),
        // The abatement after the last notice is outside the notice log's period.
        ("abatements_in_notice_period", 6.0),
        ("abatements_without_notice_in_log", 1.0),
    ] {
        assert_eq!(number(column), expected, "{column}");
    }
    assert_eq!(cell(&result, 0, "last_posted"), &json!("2025-07-10"));
}

#[tokio::test]
async fn reports_around_abatements_are_counted_once_per_report_inside_the_complete_period() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/abatement-reports-nearby.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("nearby.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let mut repeat_sql = fixture["queries"][0]["sql"].as_str().unwrap().to_string();
    let mut sql = fixture["queries"][1]["sql"].as_str().unwrap().to_string();
    for name in ["completed", "closed2025", "open"] {
        let path = temp.path().join(format!("{name}.csv"));
        std::fs::write(&path, fixture["csv"][name].as_str().unwrap()).unwrap();
        let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"REGIONAL_DATA", "displayName":name, "localPath":path, "sourceUri":format!("https://example.test/{name}.csv"), "partialDataset":true })).unwrap();
        let imported = db.import_analytical_csv(input).await.unwrap();
        assert_eq!(imported.rejected_rows, 0, "{name}");
        sql = sql.replace(&format!("analysis_nearby_{name}"), &imported.table_name);
        repeat_sql = repeat_sql.replace(&format!("analysis_nearby_{name}"), &imported.table_name);
    }
    // Three placed abatements at three places, one day each; the fourth row has no coordinates.
    let repeats = db.execute_restricted_sql(&repeat_sql, 100).await.unwrap();
    assert_eq!(repeats.row_count, 3);
    for (column, expected) in [
        ("days", json!(1)),
        ("places", json!(3)),
        ("places_abated_once", json!(3)),
        ("repeat_places", json!(0)),
        ("rows_not_placed", json!(1)),
    ] {
        let value = cell(&repeats, 0, column);
        assert_eq!(
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok())),
            expected.as_i64(),
            "{column}"
        );
    }
    let result = db.execute_restricted_sql(&sql, 100).await.unwrap();
    assert_eq!(result.row_count, 1);
    let count = |name: &str| -> i64 {
        let value = cell(&result, 0, name);
        value
            .as_i64()
            .or_else(|| value.as_f64().map(|number| number as i64))
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .unwrap_or_else(|| panic!("{name} is not a count: {value}"))
    };
    for (column, expected) in [
        ("abatement_place_days", 4),
        // One abatement has no coordinates; one is within 14 days of the reports' last day.
        ("counted_place_days", 2),
        // Thirty-five Encampment reports; the one held by both tables is one report; the Pothole is not read.
        ("reports_in_period", 35),
        ("reports_not_placed", 1),
        ("near_before", 2),
        ("near_same_day", 1),
        // The report 15 days after is outside the window.
        ("near_after", 2),
        ("ring_before", 11),
        // The report 2 km away is in neither distance band.
        ("ring_after", 16),
        // Only the abatement with ten or more ring reports beforehand is compared: 11 before, 15 after.
        ("ring_compared", 1),
        ("ring_more_after", 1),
        ("ring_fewer_after", 0),
        // 15 after against 11 before is under half again; 17 would be the first to count.
        ("ring_rose_by_half", 0),
        ("ring_fell_by_third", 0),
        // Both abatements have their earlier 29 days inside the period; no report falls in them.
        ("earlier_place_days", 2),
        ("earlier_near_before", 0),
        ("earlier_ring_after", 0),
    ] {
        assert_eq!(count(column), expected, "{column}");
    }
}

#[tokio::test]
async fn agency_report_export_counts_reports_by_form_and_by_named_official() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/disclosure-leads.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("reports.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let path = temp.path().join("agency_reports.csv");
    std::fs::write(&path, fixture["csv"].as_str().unwrap()).unwrap();
    let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"REGIONAL_DATA", "displayName":"agency reports", "localPath":path, "partialDataset":true })).unwrap();
    let imported = db.import_analytical_csv(input).await.unwrap();
    assert_eq!((imported.imported_rows, imported.rejected_rows), (8, 0));
    let count = |result: &SqlQueryResult, row: usize, name: &str| -> i64 {
        let value = cell(result, row, name);
        value
            .as_i64()
            .or_else(|| value.as_f64().map(|number| number as i64))
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .unwrap_or_else(|| panic!("{name} is not a count: {value}"))
    };
    let mut results = Vec::new();
    for query in fixture["queries"].as_array().unwrap() {
        let sql = query["sql"]
            .as_str()
            .unwrap()
            .replace("analysis_agency_reports", &imported.table_name);
        results.push(db.execute_restricted_sql(&sql, 100).await.unwrap());
    }
    let forms: Vec<(String, i64, i64)> = (0..results[0].row_count)
        .map(|row| {
            (
                cell(&results[0], row, "form").as_str().unwrap().to_string(),
                count(&results[0], row, "reports"),
                count(&results[0], row, "undated"),
            )
        })
        .collect();
    // The type "FPPC 802; Not electronically filed." is Form 802; the undated report is counted.
    assert_eq!(
        forms,
        vec![
            ("FPPC 803".to_string(), 6, 0),
            ("FPPC 801".to_string(), 1, 1),
            ("FPPC 802".to_string(), 1, 0),
        ]
    );
    let officials = &results[1];
    assert_eq!(officials.row_count, 3);
    assert_eq!(cell(officials, 0, "official"), &json!("Official A"));
    for (column, expected) in [
        ("reports", 4),
        ("all_reports", 6),
        ("officials_named", 2),
        // The report with an empty official cell is counted apart, under no name.
        ("unnamed_reports", 1),
    ] {
        assert_eq!(count(officials, 0, column), expected, "{column}");
    }
    assert_eq!(cell(officials, 0, "first_filed"), &json!("2026-01-05"));
    assert_eq!(cell(officials, 0, "last_filed"), &json!("2026-08-28"));
}

#[tokio::test]
async fn charge_log_checks_count_arrests_and_citations_not_charge_rows() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/charge-leads.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("charges.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let path = temp.path().join("charges.csv");
    std::fs::write(&path, fixture["csv"].as_str().unwrap()).unwrap();
    let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"REGIONAL_DATA", "displayName":"charges", "localPath":path, "sourceUri":"https://example.test/charges.csv", "partialDataset":true })).unwrap();
    let imported = db.import_analytical_csv(input).await.unwrap();
    assert_eq!((imported.imported_rows, imported.rejected_rows), (9, 0));
    let run = |index: usize| {
        let sql = fixture["queries"][index]["sql"]
            .as_str()
            .unwrap()
            .replace("analysis_charge_log", &imported.table_name);
        let db = &db;
        async move { db.execute_restricted_sql(&sql, 100).await.unwrap() }
    };
    let count = |result: &SqlQueryResult, row: usize, name: &str| -> i64 {
        let value = cell(result, row, name);
        value
            .as_i64()
            .or_else(|| value.as_f64().map(|number| number as i64))
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .unwrap_or_else(|| panic!("{name} is not a count: {value}"))
    };
    let counts = run(0).await;
    assert_eq!(counts.row_count, 1);
    for (column, expected) in [
        // Nine charge rows are eight arrests and citations.
        ("activities", 8),
        ("counted_activities", 5),
        ("arrests", 2),
        ("citations", 3),
        // The arrest that also cites battery is not on these sections alone.
        ("arrests_on_these_alone", 1),
        ("encampment", 1),
        // 54.011 and 54.0110 are one section.
        ("encroachment", 2),
        // 647(F) is not 647(E).
        ("lodging", 1),
        ("urination", 1),
    ] {
        assert_eq!(count(&counts, 0, column), expected, "{column}");
    }
    assert_eq!(cell(&counts, 0, "first_date"), &json!("2025-01-01"));
    assert_eq!(cell(&counts, 0, "last_date"), &json!("2025-02-02"));
    let races = run(1).await;
    // The citation whose race is the publisher's "NULL" is in neither column.
    let rows: Vec<(String, i64, i64, i64, i64)> = (0..races.row_count)
        .map(|row| {
            (
                cell(&races, row, "race").as_str().unwrap().to_string(),
                count(&races, row, "counted_activities"),
                count(&races, row, "activities"),
                count(&races, row, "all_counted_activities"),
                count(&races, row, "all_activities"),
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            ("BLACK".to_string(), 2, 2, 4, 7),
            ("WHITE".to_string(), 2, 3, 4, 7),
            ("HISPANIC".to_string(), 0, 2, 4, 7),
        ]
    );
}

/// The shelter inventory as GeoJSON, the operating budget and the Get It Done open and closed-year
/// files, each with the publisher's own header, read together by the comparison's SQL.
#[tokio::test]
async fn capacity_comparison_counts_each_series_per_fiscal_year_and_keeps_absence_apart() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/capacity-leads.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("capacity.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let mut sql = fixture["queries"][0]["sql"].as_str().unwrap().to_string();
    let mut files = vec![
        (
            "shelter".to_string(),
            "hic.geojson".to_string(),
            &fixture["inventory"],
        ),
        (
            "budget".to_string(),
            "budget.csv".to_string(),
            &fixture["budget"],
        ),
    ];
    for (name, text) in fixture["reports"].as_object().unwrap() {
        let snapshot = if name == "open" {
            name.clone()
        } else {
            format!("closed_{name}")
        };
        files.push((snapshot, format!("reports_{name}.csv"), text));
    }
    for (snapshot, file_name, text) in files {
        let path = temp.path().join(&file_name);
        std::fs::write(&path, text.as_str().unwrap()).unwrap();
        let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"REGIONAL_DATA", "displayName":snapshot, "localPath":path, "sourceUri":format!("https://example.test/{file_name}"), "partialDataset":true })).unwrap();
        let imported = db.import_analytical_csv(input).await.unwrap();
        assert_eq!(imported.rejected_rows, 0, "{snapshot}");
        sql = sql.replace(
            &format!("\"analysis_{snapshot}\""),
            &format!("\"{}\"", imported.table_name),
        );
    }
    let result = db.execute_restricted_sql(&sql, 100).await.unwrap();
    let number = |row: usize, name: &str| -> Option<i64> {
        let value = cell(&result, row, name);
        value
            .as_i64()
            .or_else(|| value.as_f64().map(|number| number as i64))
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
    };
    let rows: Vec<[Option<i64>; 9]> = (0..result.row_count)
        .map(|row| {
            [
                "fiscal_year",
                "adopted_rows",
                "department_rows",
                "adopted_expense",
                "encampment_reports",
                "year_round_beds",
                "total_beds",
                "shelter_projects",
                "projects_without_year_round",
            ]
            .map(|name| number(row, name))
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            // Adopted rows exist for fiscal 2023 and none names the department: no amount, not
            // zero. Its reports are not counted because the 2022 closed file is not imported.
            [
                Some(2023),
                Some(1),
                Some(0),
                None,
                None,
                Some(80),
                Some(85),
                Some(1),
                Some(0)
            ],
            // Revenue and proposed lines are left out of 1,500. Three reports were filed from
            // 1 July 2023 through 30 June 2024, the last at 23:50 on 30 June.
            [
                Some(2024),
                Some(4),
                Some(3),
                Some(1500),
                Some(3),
                Some(150),
                Some(170),
                Some(2),
                Some(0)
            ],
            // Report 9001 is in the open file and the 2025 closed file and is counted once;
            // Homeless Outreach is another service. One project records no year-round count.
            [
                Some(2025),
                Some(1),
                Some(1),
                Some(2000),
                Some(4),
                Some(120),
                Some(170),
                Some(2),
                Some(1)
            ],
            // Fiscal 2026 rows carry no budget cycle, so no row is adopted.
            [
                Some(2026),
                None,
                None,
                None,
                Some(1),
                Some(90),
                Some(95),
                Some(1),
                Some(0)
            ],
        ]
    );
    assert_eq!(
        cell(&result, 1, "departments"),
        &json!("Homelessness Strategies & Solutions")
    );
}

/// The two logs as the City released them (PRA 25-5806 notices, 26-772 abatements), read from a
/// collection's object store the way Data upkeep reads them: no extension on disk, the format
/// taken from the released file name.
#[tokio::test]
#[ignore = "Needs ONRECORD_ENCAMPMENT_COLLECTION, a directory holding the 2026-10-03 encampment collection"]
async fn released_abatement_spreadsheets_import_and_reproduce_the_measured_hours() {
    let collection = std::path::PathBuf::from(
        std::env::var("ONRECORD_ENCAMPMENT_COLLECTION").expect("ONRECORD_ENCAMPMENT_COLLECTION"),
    );
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/abatement-leads.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("abatement.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let mut sql = fixture["queries"][0]["sql"].as_str().unwrap().to_string();
    for (name, file_name, sha256, rows) in [
        (
            "posted",
            "PRA #25-5806 - ESD.xlsx",
            "ed2b8da8521d761ddc03403a0452b22e6c5df223a7356448ab328c52bd9c74e2",
            11_090,
        ),
        (
            "completed",
            "Copy of Abatements Completed 7.31.23 to 1.28.26.xlsx",
            "ba47308323e283fef3a1f2c4174059793eddde98a34bcecef9cf1bb98004a428",
            14_000,
        ),
    ] {
        let path = collection
            .join("downloads/objects")
            .join(&sha256[..2])
            .join(sha256);
        let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"REGIONAL_DATA", "displayName":file_name, "localPath":path, "sourceUri":format!("https://example.test/{name}"), "partialDataset":true, "sourceFileName":file_name, "expectedSourceSha256":sha256 })).unwrap();
        let imported = db.import_analytical_csv(input).await.unwrap();
        assert_eq!(
            (imported.imported_rows, imported.rejected_rows),
            (rows, 0),
            "{name}"
        );
        sql = sql.replace(&format!("analysis_abatement_{name}"), &imported.table_name);
    }
    let result = db.execute_restricted_sql(&sql, 100).await.unwrap();
    let number = |name: &str| -> f64 {
        let value = cell(&result, 0, name);
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .unwrap_or_else(|| panic!("{name} is not numeric: {value}"))
    };
    for (column, expected) in [
        ("posted_notices", 11_090.0),
        ("completed_abatements", 14_000.0),
        ("paired_places", 2_067.0),
        ("completed_before_posted", 11.0),
        ("under_3_hours", 22.0),
        ("under_24_hours", 101.0),
        ("from_24_to_72_hours", 1_811.0),
        ("over_72_hours", 144.0),
        ("median_hours", 29.3),
        ("abatements_in_notice_period", 11_104.0),
        ("abatements_without_notice_in_log", 214.0),
    ] {
        assert_eq!(number(column), expected, "{column}");
    }
}

/// The operating budget, the operating actuals and the State Controller's city pay file, each with
/// the publisher's own header, read by the spending leads' SQL.
#[tokio::test]
async fn spending_checks_keep_no_rows_apart_from_zero_and_tie_a_budget_to_one_employer() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/spending-leads.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("spending.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let mut tables = Vec::new();
    for snapshot in ["budget", "actuals", "pay"] {
        let path = temp.path().join(format!("{snapshot}.csv"));
        std::fs::write(&path, fixture[snapshot].as_str().unwrap()).unwrap();
        let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"REGIONAL_DATA", "displayName":snapshot, "localPath":path, "sourceUri":format!("https://example.test/{snapshot}.csv"), "partialDataset":true })).unwrap();
        let imported = db.import_analytical_csv(input).await.unwrap();
        assert_eq!(imported.rejected_rows, 0, "{snapshot}");
        tables.push((format!("\"analysis_{snapshot}\""), imported.table_name));
    }
    let mut results = Vec::new();
    for query in fixture["queries"].as_array().unwrap() {
        let mut sql = query["sql"].as_str().unwrap().to_string();
        for (placeholder, table_name) in &tables {
            sql = sql.replace(placeholder, &format!("\"{table_name}\""));
        }
        let result = db.execute_restricted_sql(&sql, 100).await.unwrap();
        assert!(!result.truncated, "{}", query["id"]);
        results.push(result);
    }
    let [compared, settled, paid] = &results[..] else {
        panic!("three spending queries");
    };
    let number = |result: &SqlQueryResult, row: usize, name: &str| -> Option<i64> {
        let value = cell(result, row, name);
        value
            .as_i64()
            .or_else(|| value.as_f64().map(|number| number as i64))
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
    };
    let rows =
        |result: &SqlQueryResult, label: &str, names: &[&str]| -> Vec<(String, Vec<Option<i64>>)> {
            (0..result.row_count)
                .map(|row| {
                    (
                        cell(result, row, label).as_str().unwrap_or("").to_string(),
                        names.iter().map(|name| number(result, row, name)).collect(),
                    )
                })
                .collect()
        };
    let line = |label: &str, values: &[Option<i64>]| (label.to_string(), values.to_vec());

    // Fiscal 2024 is adopted and not in the actuals file, so only 2022 and 2023 are read. Revenue
    // and proposed lines are in neither sum. A department-year with no actual rows has no count
    // and no amount, not zero.
    assert_eq!(
        rows(
            compared,
            "department",
            &[
                "fiscal_year",
                "adopted_expense",
                "actual_rows",
                "actual_expense"
            ]
        ),
        vec![
            line(
                "Business Office",
                &[Some(2022), Some(1_000_000), None, None]
            ),
            line(
                "Environmental Services",
                &[Some(2022), Some(50_000_000), Some(1), Some(49_000_000)]
            ),
            line(
                "Environmental Services",
                &[Some(2023), Some(51_000_000), None, None]
            ),
            line(
                "Homelessness Strategies & Solutions",
                &[Some(2022), Some(18_000_000), Some(2), Some(9_000_000)]
            ),
            line(
                "Homelessness Strategies & Solutions",
                &[Some(2023), Some(25_000_000), Some(2), Some(22_650_000)]
            ),
            line(
                "Police",
                &[Some(2022), Some(100_000_000), Some(1), Some(101_000_000)]
            ),
            line(
                "Police",
                &[Some(2023), Some(100_000_000), Some(3), Some(90_000_000)]
            ),
        ]
    );
    assert_eq!(
        [
            "first_covered_year",
            "last_covered_year",
            "department_years"
        ]
        .map(|name| number(compared, 0, name)),
        [Some(2022), Some(2023), Some(8)]
    );

    // Fiscal 2021 has rows and none in a settlement account. The lease settlement is left out.
    assert_eq!(
        rows(
            settled,
            "department",
            &[
                "fiscal_year",
                "departments_in_table",
                "departments_reporting",
                "year_amount",
                "department_amount"
            ]
        ),
        vec![
            line(
                "Police",
                &[
                    Some(2023),
                    Some(3),
                    Some(3),
                    Some(2_450_000),
                    Some(2_100_000)
                ]
            ),
            line(
                "Library",
                &[Some(2023), Some(3), Some(3), Some(2_450_000), Some(300_000)]
            ),
            line(
                "Homelessness Strategies & Solutions",
                &[Some(2023), Some(3), Some(3), Some(2_450_000), Some(50_000)]
            ),
            line(
                "Library",
                &[Some(2022), Some(4), Some(1), Some(-5_000), Some(-5_000)]
            ),
            line("", &[Some(2021), Some(1), None, None, None]),
        ]
    );

    // Smallville's 99 positions are one short; Oceanside has exactly 100. Overtime of exactly
    // 100,000 is counted and 99,999 is not. Oceanside's Police is not set beside this budget.
    let pay_columns = [
        "positions",
        "overtime_pay",
        "high_overtime_positions",
        "positions_without_overtime",
        "budget_departments_matched",
        "department_overtime_pay",
        "adopted_overtime_same_year",
        "adopted_overtime_next_year",
    ];
    let san_diego = |overtime: i64, same: Option<i64>, next: Option<i64>| {
        vec![
            Some(113),
            Some(2_921_999),
            Some(2),
            Some(1),
            Some(20),
            Some(overtime),
            same,
            next,
        ]
    };
    let oceanside = |overtime: i64| {
        vec![
            Some(100),
            Some(640_000),
            Some(0),
            Some(0),
            Some(1),
            Some(overtime),
            None,
            None,
        ]
    };
    assert_eq!(
        (0..paid.row_count)
            .map(|row| (
                cell(paid, row, "employer").as_str().unwrap().to_string(),
                cell(paid, row, "department").as_str().unwrap().to_string(),
                pay_columns
                    .iter()
                    .map(|name| number(paid, row, name))
                    .collect::<Vec<_>>()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "San Diego".to_string(),
                "Fire-Rescue".to_string(),
                san_diego(1_500_000, None, None)
            ),
            (
                "San Diego".to_string(),
                "Police".to_string(),
                san_diego(1_419_999, Some(49_000_000), Some(46_000_000))
            ),
            (
                "San Diego".to_string(),
                "Homelessness Strategies & Solutions".to_string(),
                san_diego(2_000, None, None)
            ),
            (
                "San Diego".to_string(),
                "City Attorney".to_string(),
                san_diego(0, None, None)
            ),
            (
                "San Diego".to_string(),
                "City Auditor".to_string(),
                san_diego(0, None, None)
            ),
            (
                "Oceanside".to_string(),
                "Police".to_string(),
                oceanside(600_000)
            ),
            (
                "Oceanside".to_string(),
                "Harbor".to_string(),
                oceanside(40_000)
            ),
        ]
    );
    assert_eq!(
        [
            cell(paid, 0, "budget_city"),
            cell(paid, 5, "budget_city"),
            cell(paid, 0, "overtime_departments_same_year")
        ],
        [&json!(true), &json!(false), &json!(1)]
    );
}
