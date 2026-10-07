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

fn row_of(result: &SqlQueryResult, column: &str, value: &str) -> usize {
    (0..result.row_count)
        .find(|row| cell(result, *row, column) == &json!(value))
        .unwrap_or_else(|| panic!("no row with {column} = {value}"))
}

#[tokio::test]
async fn parking_checks_count_each_citation_once_by_issuing_unit() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/parking-leads.json")).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("parking.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let dataset = &fixture["schema"]["datasets"][0];
    let name = dataset["snapshotId"].as_str().unwrap();
    let path = temp.path().join(format!("{name}.csv"));
    std::fs::write(&path, fixture["csv"][name].as_str().unwrap()).unwrap();
    let input: AnalyticalImportRequest = serde_json::from_value(json!({ "datasetKind":"REGIONAL_DATA", "displayName":name, "localPath":path, "sourceUri":dataset["sourceUri"], "partialDataset":true })).unwrap();
    let imported = db.import_analytical_csv(input).await.unwrap();
    assert_eq!(imported.rejected_rows, 0);
    let placeholder = format!("\"{}\"", dataset["tableName"].as_str().unwrap());
    let table = format!("\"{}\"", imported.table_name);
    for query in fixture["queries"].as_array().unwrap() {
        let id = query["id"].as_str().unwrap();
        let sql = query["sql"].as_str().unwrap().replace(&placeholder, &table);
        let result = db
            .execute_restricted_sql(&sql, 100)
            .await
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        match id {
            "parking_vehicle_sections" => {
                assert_eq!(result.row_count, 3);
                // 86.0139(A), 86.0139 (A) and 86.0139(A) SDMC are one code; 86.0139(A)_W is a
                // warning and 86.0139(B) is not counted. 70000004 is entered twice and counted
                // once, from its later entry by police. CSD I PARKING ENF is parking enforcement;
                // the blank unit is not recorded, not parking enforcement.
                let oversized = row_of(&result, "section", "oversized");
                for (column, expected) in [
                    ("citations", 7.0),
                    ("by_police", 3.0),
                    ("by_parking", 2.0),
                    ("other_sector", 1.0),
                    ("no_sector", 1.0),
                    ("conflicting", 0.0),
                    ("unread_fines", 1.0),
                    ("warnings", 1.0),
                    // Six readable fines of 112.50: the repeated citation's fine is summed once.
                    ("fines", 675.0),
                ] {
                    assert_eq!(
                        number(&result, oversized, column),
                        expected,
                        "oversized {column}"
                    );
                }
                // SDMC 86.0137(G) and 86.0137(G)S DMC are 86.0137(G). 70000013's two entries
                // share a time but not a unit: conflicting, under no unit and in no fine total.
                let unmoved = row_of(&result, "section", "unmoved");
                for (column, expected) in [
                    ("citations", 4.0),
                    ("by_police", 1.0),
                    ("by_parking", 2.0),
                    ("other_sector", 0.0),
                    ("no_sector", 0.0),
                    ("conflicting", 1.0),
                    ("unread_fines", 0.0),
                    ("fines", 160.5),
                ] {
                    assert_eq!(
                        number(&result, unmoved, column),
                        expected,
                        "unmoved {column}"
                    );
                }
                let habitation = row_of(&result, "section", "habitation");
                assert_eq!(number(&result, habitation, "citations"), 1.0);
                assert_eq!(number(&result, oversized, "all_citations"), 15.0);
                assert_eq!(
                    cell(&result, oversized, "first_issue"),
                    &json!("2022-07-01")
                );
                assert_eq!(cell(&result, oversized, "last_issue"), &json!("2022-12-31"));
            }
            "parking_vehicle_blocks" => {
                // The earlier, misspelt place of 70000004 is not read, and 4000 HARBOR DR is
                // matched across case and surrounding spaces.
                assert_eq!(result.row_count, 4);
                assert_eq!(cell(&result, 0, "block"), &json!("3500 SPORTS ARENA BLVD"));
                assert_eq!(number(&result, 0, "citations"), 4.0);
                assert_eq!(number(&result, 0, "by_police"), 3.0);
                assert_eq!(cell(&result, 1, "block"), &json!("4000 HARBOR DR"));
                assert_eq!(number(&result, 1, "citations"), 3.0);
                assert_eq!(number(&result, 0, "section_citations"), 11.0);
                assert_eq!(number(&result, 0, "no_block"), 1.0);
            }
            _ => panic!("Unverified query {id}"),
        }
    }
}
