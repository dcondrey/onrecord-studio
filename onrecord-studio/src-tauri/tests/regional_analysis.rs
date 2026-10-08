use onrecord_studio_lib::storage::sqlite::DuckDbRepository;
use onrecord_studio_lib::types::{AnalyticalDatasetKind, AnalyticalImportRequest};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::Path;

async fn database(root: &Path) -> DuckDbRepository {
    let repository = DuckDbRepository::new(root.join("regional.db")).unwrap();
    repository
        .run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    repository
}

fn request(path: &Path, url: &str) -> AnalyticalImportRequest {
    serde_json::from_value(json!({
        "datasetKind":"REGIONAL_DATA", "displayName":"Regional fixture", "localPath":path.to_str().unwrap(),
        "sourceUri":url, "observationPeriodStart":"2026-01-01", "observationPeriodEnd":"2026-01-31", "partialDataset":false
    })).unwrap()
}

#[tokio::test]
async fn budget_import_retains_cycles_and_separates_expenses_revenue_and_fiscal_years() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("budget.csv");
    std::fs::write(&path, "amount,report_fy,budget_cycle,fund_number,dept_name,account_number\n100.01,25,adopted,000100,Housing,511000\n20.02,25,Adopted,000100,Housing,511000\n700,25,adopted,000100,Housing,411000\n999,25,proposed,000100,Housing,511000\n5.01,26,,000100,Housing,511000\n*,26,adopted,000100,Housing,511000\n").unwrap();
    let db = database(temp.path()).await;
    let mut input = request(
        &path,
        "https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv",
    );
    input.dataset_kind = AnalyticalDatasetKind::OperatingBudget;
    let imported = db.import_analytical_csv(input).await.unwrap();
    // An unreadable amount keeps its row: the cell is stored as NULL and counted, not dropped.
    assert_eq!(
        (
            imported.imported_rows,
            imported.rejected_rows,
            imported.unconverted_cells
        ),
        (6, 0, 1)
    );
    let schema = db.analytical_schema().await.unwrap();
    let comparison = schema
        .templates
        .iter()
        .find(|t| t.template_id == "budget-cycle-comparison")
        .unwrap();
    let result = db
        .execute_restricted_sql(&comparison.sql, 20)
        .await
        .unwrap();
    assert_eq!(result.row_count, 3);
    // FY26's only adopted expense amount was unreadable: its total is absent, never 0.
    assert!(result
        .rows
        .iter()
        .any(|r| r[0] == json!("26") && r[4] == json!("Expense") && r[5].is_null()));
    let expense = result
        .rows
        .iter()
        .find(|r| r[0] == json!("25") && r[4] == json!("Expense"))
        .unwrap();
    assert_eq!(expense[3], json!("000100"));
    assert_eq!(expense[5], json!("120.0300000000"));
    let coverage = schema
        .templates
        .iter()
        .find(|t| t.template_id.ends_with("-budget-coverage"))
        .unwrap();
    let result = db.execute_restricted_sql(&coverage.sql, 20).await.unwrap();
    assert!(result
        .rows
        .iter()
        .any(|r| r[0] == json!("26") && r[1] == json!("UNSPECIFIED") && r[2] == json!(1)));
}

#[tokio::test]
#[ignore = "Set ONRECORD_BUDGET_EXPORT to a downloaded City operating-budget CSV; imports into a temporary database"]
async fn full_city_budget_export_reaches_sql_without_truncation() {
    let path = std::env::var("ONRECORD_BUDGET_EXPORT")
        .expect("ONRECORD_BUDGET_EXPORT must name a downloaded CSV");
    let expected_rows = csv::Reader::from_path(&path).unwrap().records().count();
    assert!(
        expected_rows > 500_000,
        "Use the complete multi-year export"
    );
    let temp = tempfile::tempdir().unwrap();
    let db = database(temp.path()).await;
    let mut input = request(
        Path::new(&path),
        "https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv",
    );
    input.dataset_kind = AnalyticalDatasetKind::OperatingBudget;
    input.observation_period_start = None;
    input.observation_period_end = None;
    let imported = db.import_analytical_csv(input).await.unwrap();
    assert_eq!(imported.imported_rows, expected_rows);
    assert_eq!(imported.rejected_rows, 0);
    let count = db
        .execute_restricted_sql(&format!("SELECT COUNT(*) FROM {}", imported.table_name), 1)
        .await
        .unwrap();
    assert_eq!(count.rows[0][0], json!(expected_rows));
    let ledger = db
        .execute_restricted_sql("SELECT COUNT(*) FROM analytical_rows", 1)
        .await
        .unwrap();
    assert_eq!(ledger.rows[0][0], json!(expected_rows));
    println!(
        "City operating budget: {} rows imported and retained in the ledger; SHA256 {}",
        imported.imported_rows, imported.source_sha256
    );
}

#[tokio::test]
#[ignore = "Set ONRECORD_ACTUALS_EXPORT to a downloaded City operating-actuals CSV; uses a temporary database"]
async fn full_city_actuals_export_preserves_observed_fiscal_years() {
    let path = std::env::var("ONRECORD_ACTUALS_EXPORT")
        .expect("ONRECORD_ACTUALS_EXPORT must name a downloaded CSV");
    let mut reader = csv::Reader::from_path(&path).unwrap();
    let headers = reader.headers().unwrap().clone();
    let fy = headers.iter().position(|h| h == "report_fy").unwrap();
    let mut expected_rows = 0;
    let mut years = std::collections::BTreeSet::new();
    for row in reader.records() {
        let row = row.unwrap();
        years.insert(row.get(fy).unwrap().to_string());
        expected_rows += 1;
    }
    assert!(expected_rows > 600_000);
    let temp = tempfile::tempdir().unwrap();
    let db = database(temp.path()).await;
    let mut input = request(
        Path::new(&path),
        "https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv",
    );
    input.observation_period_start = None;
    input.observation_period_end = None;
    let imported = db.import_analytical_csv(input).await.unwrap();
    assert_eq!(
        (imported.imported_rows, imported.rejected_rows),
        (expected_rows, 0)
    );
    let schema = db.analytical_schema().await.unwrap();
    let coverage = schema
        .templates
        .iter()
        .find(|t| t.template_id.ends_with("-fiscal-year-coverage"))
        .unwrap();
    let result = db.execute_restricted_sql(&coverage.sql, 100).await.unwrap();
    let sql_years = result
        .rows
        .iter()
        .map(|r| match &r[0] {
            serde_json::Value::String(s) => s.clone(),
            v => v.to_string(),
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(sql_years, years);
    println!(
        "City operating actuals: {} rows, fiscal-year codes {:?}, SHA256 {}",
        expected_rows, years, imported.source_sha256
    );
}

#[tokio::test]
async fn regional_csv_reaches_exact_sql_and_snapshot_profiles() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("health.csv");
    std::fs::write(&path, "zip,count,rate,estimate,observed,large\n00123,15,0.000001,*,2026-01-01,9007199254740993\n92101,20,1.123456,12,2026-01-02,1\n").unwrap();
    let db = database(temp.path()).await;
    let imported = db
        .import_analytical_csv(request(&path, "https://example.test/health.csv"))
        .await
        .unwrap();
    assert_eq!(imported.imported_rows, 2);
    let result = db
        .execute_restricted_sql(
            &format!("SELECT SUM(count), SUM(rate) FROM {}", imported.table_name),
            10,
        )
        .await
        .unwrap();
    assert_eq!(result.rows[0], vec![json!("35"), json!("1.1234570000")]);
    let result = db
        .execute_restricted_sql(
            &format!(
                "SELECT zip, estimate, observed, large FROM {} ORDER BY zip",
                imported.table_name
            ),
            10,
        )
        .await
        .unwrap();
    assert_eq!(
        result.rows[0],
        vec![
            json!("00123"),
            json!("*"),
            json!("2026-01-01"),
            json!("9007199254740993")
        ]
    );
    let schema = db.analytical_schema().await.unwrap();
    assert_eq!(
        schema.datasets[0].dataset_kind,
        AnalyticalDatasetKind::RegionalData
    );
    assert!(!schema.datasets[0].partial_dataset);
    assert!(schema.datasets[0].validation_summary.contains("nullCounts"));
    let profile = schema
        .templates
        .iter()
        .find(|t| t.template_id == format!("{}-profile", imported.snapshot_id))
        .unwrap();
    assert_eq!(
        db.execute_restricted_sql(&profile.sql, 100)
            .await
            .unwrap()
            .row_count,
        6
    );
}

#[tokio::test]
async fn malformed_and_duplicate_header_rows_are_accounted_for_without_overwriting_fields() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("duplicate.csv");
    std::fs::write(&path, "a,a,a_2,\n1,2,3,4\n5,6,7,8,EXTRA\n").unwrap();
    let db = database(temp.path()).await;
    let result = db
        .import_analytical_csv(request(&path, "https://example.test/duplicate.csv"))
        .await
        .unwrap();
    assert_eq!(
        (
            result.imported_rows,
            result.rejected_rows,
            result.malformed_rows
        ),
        (1, 1, 1)
    );
    assert!(!result.schema_warnings.is_empty());
    let data = db
        .execute_restricted_sql(&format!("SELECT * FROM {}", result.table_name), 10)
        .await
        .unwrap();
    assert_eq!(
        data.columns,
        ["a", "a_2", "a_2_2", "field_4", "_source_row"]
    );
    assert_eq!(
        data.rows[0],
        vec![json!(1), json!(2), json!(3), json!(4), json!(1)]
    );
    let ledger = db
        .execute_restricted_sql("SELECT rejected_rows FROM analytical_snapshots", 10)
        .await
        .unwrap();
    assert_eq!(ledger.rows[0][0], json!(1));
    let schema = db.analytical_schema().await.unwrap();
    let rejected = schema
        .templates
        .iter()
        .find(|t| t.template_id.ends_with("-rejected"))
        .unwrap();
    let rows = db.execute_restricted_sql(&rejected.sql, 10).await.unwrap();
    assert!(rows.rows[0][1].as_str().unwrap().contains("EXTRA"));
}

#[tokio::test]
async fn feature_exports_keep_geometry_and_force_declared_truncation_to_partial() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("layer.geojson");
    std::fs::write(&path, r#"{"type":"FeatureCollection","exceededTransferLimit":true,"features":[{"type":"Feature","properties":{"geoid":"06073000100","count":7},"geometry":{"type":"Point","coordinates":[-117.1,32.7]}}]}"#).unwrap();
    let db = database(temp.path()).await;
    let imported = db
        .import_analytical_csv(request(
            &path,
            "https://services.arcgis.com/example/FeatureServer/0/query",
        ))
        .await
        .unwrap();
    let schema = db.analytical_schema().await.unwrap();
    assert!(schema.datasets[0].partial_dataset);
    let result = db
        .execute_restricted_sql(
            &format!("SELECT geoid, geometry FROM {}", imported.table_name),
            10,
        )
        .await
        .unwrap();
    assert_eq!(result.rows[0][0], json!("06073000100"));
    assert!(result.rows[0][1].as_str().unwrap().contains("coordinates"));
}

#[tokio::test]
async fn distinct_publishers_and_changed_evidence_cannot_replace_a_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("counts.csv");
    std::fs::write(&path, "count\n1\n").unwrap();
    let db = database(temp.path()).await;
    let a = db
        .import_analytical_csv(request(&path, "https://city.test/counts.csv"))
        .await
        .unwrap();
    let b = db
        .import_analytical_csv(request(&path, "https://county.test/counts.csv"))
        .await
        .unwrap();
    assert_ne!(a.snapshot_id, b.snapshot_id);
    assert_eq!(a.source_sha256, b.source_sha256);
    assert_eq!(db.analytical_schema().await.unwrap().datasets.len(), 2);
    let mut changed = request(&path, "https://city.test/counts.csv");
    changed.expected_source_sha256 = Some(a.source_sha256);
    std::fs::write(&path, "count\n2\n").unwrap();
    assert!(db
        .import_analytical_csv(changed)
        .await
        .unwrap_err()
        .to_string()
        .contains("SHA-256"));
    assert_eq!(db.analytical_schema().await.unwrap().datasets.len(), 2);
}

#[tokio::test]
async fn ingestion_downloads_are_offered_to_analysis_with_original_provenance() {
    use onrecord_studio_lib::storage::sqlite::DocumentDownloadFinalization;
    use onrecord_studio_lib::types::{PortalRegistration, PublicRequest};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("acquired.csv");
    let bytes = b"count\n5\n";
    std::fs::write(&path, bytes).unwrap();
    let digest = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let db = database(temp.path()).await;
    let portal: PortalRegistration = serde_json::from_value(json!({"portalKey":"county", "displayName":"County", "canonicalUrl":"https://example.test", "originUrl":"https://example.test", "verifiedAt":"2026-09-13T00:00:00Z", "sourceKind":"WEB_CRAWL", "apiLayout":{"requestsEndpoint":"https://example.test", "requestsPointer":"/resources", "pageParameter":"page", "pageSizeParameter":"limit"}})).unwrap();
    db.insert_or_update_portal_registration(&portal)
        .await
        .unwrap();
    let record: PublicRequest = serde_json::from_value(json!({"id":42, "title":"Data", "status":"open", "created_at":"2026-09-13T00:00:00Z", "updated_at":"2026-09-13T00:00:00Z", "documents":[{"id":7,"file_name":"acquired.csv","content_type":"text/csv", "download_url":"https://example.test/acquired.csv", "visibility":"public", "created_at":"2026-09-13T00:00:00Z", "updated_at":"2026-09-13T00:00:00Z"}]})).unwrap();
    db.insert_or_update_request("county", &record)
        .await
        .unwrap();
    db.mark_document_downloaded(DocumentDownloadFinalization {
        document_id: 7,
        portal_subdomain: "county",
        raw_file_sha256: &digest,
        raw_file_size: bytes.len() as u64,
        local_path: &path,
        shannon_entropy: None,
        entropy_sample_bytes: 0,
        suspicious_data_density: false,
    })
    .await
    .unwrap();
    let candidates = db.analytical_import_candidates().await.unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].source_uri.as_deref(),
        Some("https://example.test/acquired.csv")
    );
    assert_eq!(
        candidates[0].source_sha256.as_deref(),
        Some(digest.as_str())
    );
    // Exercise the exact source query used by semantic indexing without downloading a model.
    let connection = db.analytics_connection().await.unwrap();
    connection.execute("UPDATE portals SET base_url = 'https://pra.sandiegocounty.gov' WHERE subdomain = 'county'", []).unwrap();
    connection.execute("UPDATE documents SET extracted_text_sha256 = ?1 WHERE id = 7 AND portal_subdomain = 'county'", [&digest]).unwrap();
    connection.execute("INSERT INTO documents_fts(document_id, portal_subdomain, file_name, extracted_text) VALUES (7, 'county', 'acquired.csv', 'county health count')", []).unwrap();
    let vector_sources = db
        .list_vector_source_documents_page(None, 10)
        .await
        .unwrap();
    assert_eq!(vector_sources.len(), 1);
    assert_eq!(vector_sources[0].jurisdiction, "San Diego County");
    assert!(vector_sources[0].publication_date.is_none());
    assert!(vector_sources[0].observation_date.is_none());
}

#[tokio::test]
async fn workbook_sheets_are_selected_explicitly_and_never_overwrite_each_other() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("environment.xlsx");
    std::fs::write(
        &path,
        include_bytes!("../fixtures/open-data/regional-workbook.xlsx"),
    )
    .unwrap();
    let db = database(temp.path()).await;
    let mut input = request(&path, "https://example.test/environment.xlsx");
    let error = db
        .import_analytical_csv(input.clone())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Air, Water"), "{error}");
    input.worksheet = Some("Air".into());
    let air = db.import_analytical_csv(input.clone()).await.unwrap();
    input.worksheet = Some("Water".into());
    let water = db.import_analytical_csv(input).await.unwrap();
    assert_ne!(air.snapshot_id, water.snapshot_id);
    assert_eq!(air.source_sha256, water.source_sha256);
    let result = db
        .execute_restricted_sql(
            &format!("SELECT station_code, measurement FROM {}", water.table_name),
            10,
        )
        .await
        .unwrap();
    assert_eq!(result.rows[0], vec![json!("001"), json!(34)]);
    assert!(db
        .analytical_schema()
        .await
        .unwrap()
        .datasets
        .iter()
        .all(|dataset| dataset.partial_dataset));
}
