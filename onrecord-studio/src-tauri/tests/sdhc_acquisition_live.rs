//! Opt-in production acquisition proof: a fresh database and default-guidance board discovery.
//! No model calls. The temporary originals/database are removed at the end of the test.
use onrecord_studio_lib::{
    ingestion::{
        client::{ApiClient, HttpClientConfig},
        queue::{IngestionProgress, QueueManager},
        RateLimiter,
    },
    storage::{corpus::CorpusIndex, sqlite::DuckDbRepository},
    types::{CrawlPolicy, RateLimitPolicy, SourceGuidance},
};
use std::time::Duration;

#[tokio::test]
#[ignore = "Downloads the public SDHC board index and its first eight automatically selected reports; no model calls"]
async fn extensionless_shelter_reports_reach_search_with_a_single_page_budget() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("documents")).unwrap();
    let db = DuckDbRepository::new(root.path().join("core.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let client = ApiClient::new(HttpClientConfig {
        max_retries: 0,
        request_timeout_seconds: 40,
        max_document_bytes: 20 * 1024 * 1024,
        host_concurrency: 1,
        host_request_budget: 20,
        ..Default::default()
    })
    .unwrap();
    let index =
        "https://sdhc.org/governance-legislative-affairs/sdhc-board-of-commissioners/meetings/";
    let expected = [
        (
            "HCR26-014",
            "https://sdhc.org/103_hcr26-014-imperial-shelter_final/",
            "3,523,195",
        ),
        (
            "HCR26-015",
            "https://sdhc.org/104_hcr26-015-harm-reduction-shelter_final/",
            "1,970,278",
        ),
        (
            "HCR26-016",
            "https://sdhc.org/101_hcr26-016-single-adults-shelter_final/",
            "2,865,640",
        ),
    ];
    let (_, inspected) = client.discover_public_source(index).await.unwrap();
    let automatically_selected = inspected
        .links
        .iter()
        .filter(|link| link.download_candidate)
        .take(8)
        .collect::<Vec<_>>();
    assert_eq!(automatically_selected.len(), 8);
    for (_, url, _) in expected {
        assert!(automatically_selected.iter().any(|link|link.url==url), "The default bounded collection must reach the shelter reports without a supplied document URL");
    }
    assert!(automatically_selected.iter().all(|link| !link.navigation_chrome && link.label.starts_with("HCR")), "Automatically selected resources must be substantive board reports, not navigation or presentation labels");
    let selection_trace = automatically_selected
        .iter()
        .map(|link| serde_json::json!({"title":link.label,"url":link.url}))
        .collect::<Vec<_>>();
    let (sender, mut events) = tokio::sync::mpsc::channel(2048);
    let manager = QueueManager::new(
        client,
        db.clone(),
        CorpusIndex::open(root.path().join("corpus.db")).unwrap(),
        RateLimiter::new(1).unwrap(),
        sender,
        root.path().join("documents"),
    )
    .unwrap();
    let start = manager
        .discover_and_start_public_source(
            index,
            CrawlPolicy {
                maximum_pages: 1,
                maximum_depth: 0,
                maximum_resources: 9,
                follow_supported_external_platforms: false,
            },
            SourceGuidance::default(),
            RateLimitPolicy::default(),
        )
        .await
        .unwrap();
    let mut failures = Vec::new();
    tokio::time::timeout(Duration::from_secs(150), async {
        while let Some(event) = events.recv().await {
            match event {
                IngestionProgress::TaskFailed { error_msg, .. } => failures.push(error_msg),
                IngestionProgress::SyncStopped { portal_key, .. }
                    if portal_key == start.source.portal_key =>
                {
                    break
                }
                _ => {}
            }
        }
    })
    .await
    .expect("The small shelter acquisition should complete within150seconds");
    assert!(failures.is_empty(), "{failures:?}");
    let conn = db.analytics_connection().await.unwrap();
    let mut records = Vec::new();
    for (report, url, amount) in expected {
        let (id, sha, text, status, bytes):(u64,String,String,String,u64) = conn.query_row(
            "SELECT d.id,d.raw_file_sha256,f.extracted_text,d.extraction_status,d.file_size FROM documents d JOIN documents_fts f ON f.document_id=d.id AND f.portal_subdomain=d.portal_subdomain WHERE d.download_url=?1", [url],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(status, "SEARCHABLE");
        let compact_identifier_text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            compact_identifier_text.contains(report) && text.contains(amount),
            "{report} lacks its report identity or appropriation"
        );
        assert!(text.contains("2027") && text.contains("FISCAL CONSIDERATIONS"));
        assert!(
            text.chars().count() > 7000,
            "An index/summary cannot stand in for the report body"
        );
        assert_eq!(sha.len(), 64);
        let position = text.find(amount).unwrap();
        let quote_start = text[..position].chars().count().saturating_sub(120);
        let quote: String = text.chars().skip(quote_start).take(550).collect();
        records.push(serde_json::json!({"report":report,"url":url,"documentId":id,"sha256":sha,"downloadedBytes":bytes,"extractedCharacters":text.chars().count(),"amountQuote":quote}));
    }
    let retrieved = db
        .research_search("Imperial shelter operating agreement")
        .await
        .unwrap();
    assert!(retrieved.iter().any(|e| e.text.contains("3,523,195") && e.source_url.as_deref() == Some(expected[0].1)), "Autonomous research must retrieve the report's actual funding evidence");
    let output = serde_json::json!({"checkedAt":chrono::Utc::now().to_rfc3339(),"index":index,"pageBudget":1,"resourceBudget":9,"guidance":"default — no supplied document URLs, selectors or topic filters","automaticallySelectedReports":selection_trace,"records":records,"researchExcerpts":retrieved.len(),"limits":"These are published approval reports, not executed agreements or paid invoices. Published capacity is not occupancy or housing outcomes. The bounded default collection is verified against three current shelter reports and does not claim full archive coverage."});
    std::fs::write(
        std::env::temp_dir().join("onrecord-sdhc-acquisition-validation.json"),
        serde_json::to_vec_pretty(&output).unwrap(),
    )
    .unwrap();
    println!("{output}");
}
