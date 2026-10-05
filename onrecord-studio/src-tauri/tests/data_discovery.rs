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
