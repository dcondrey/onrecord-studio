//! Opt-in end-to-end check of the actual City finance exports, not a sampled fixture.
use onrecord_studio_lib::{
    ingestion::{
        client::{ApiClient, HttpClientConfig},
        queue::{IngestionProgress, QueueManager},
        RateLimiter,
    },
    research,
    storage::{corpus::CorpusIndex, sqlite::DuckDbRepository},
    types::*,
};
use std::time::Duration;

#[tokio::test]
#[ignore = "Downloads about 130 MB of official City finance exports and runs four billed model calls"]
async fn actual_city_finance_exports_produce_reproducible_investigative_leads() {
    let root = tempfile::tempdir().unwrap();
    let documents = root.path().join("documents");
    std::fs::create_dir_all(&documents).unwrap();
    let db = DuckDbRepository::new(root.path().join("core.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    db.initialize_research().await.unwrap();
    let mut policy = db.research_policy().await.unwrap();
    policy.research_passes = 2;
    policy.discover_sources = false;
    db.save_research_policy(policy).await.unwrap();
    let client = ApiClient::new(HttpClientConfig {
        max_retries: 1,
        request_timeout_seconds: 90,
        max_document_bytes: 96 * 1024 * 1024,
        host_request_budget: 20,
        ..Default::default()
    })
    .unwrap();
    let (sender, mut events) = tokio::sync::mpsc::channel(2048);
    let manager = QueueManager::new(
        client,
        db.clone(),
        CorpusIndex::open(root.path().join("corpus.db")).unwrap(),
        RateLimiter::new(1).unwrap(),
        sender,
        documents.clone(),
    )
    .unwrap();
    for url in [
        "https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv",
        "https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv",
    ] {
        let start = manager
            .discover_and_start_public_source(
                url,
                CrawlPolicy {
                    maximum_pages: 1,
                    maximum_depth: 0,
                    maximum_resources: 1,
                    follow_supported_external_platforms: false,
                },
                SourceGuidance::default(),
                RateLimitPolicy::default(),
            )
            .await
            .unwrap();
        let mut failures = vec![];
        tokio::time::timeout(Duration::from_secs(300), async {
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
        .unwrap();
        assert!(failures.is_empty(), "{failures:?}");
        println!("Acquired full City export: {url}");
        assert!(db.maintain_one_table().await.unwrap());
        let schema = db.analytical_schema().await.unwrap();
        assert!(
            schema
                .datasets
                .iter()
                .any(|d| d.source_uri.as_deref() == Some(url)),
            "Automatic table import failed: schema={schema:?}; {:?}",
            db.maintenance_status().await.unwrap()
        );
    }
    let schema = db.analytical_schema().await.unwrap();
    assert_eq!(schema.datasets.len(), 2);
    assert!(schema
        .datasets
        .iter()
        .all(|d| d.rejected_rows == 0 && d.imported_rows > 100_000));
    let (evidence, log) = research::budget::seed(&db, &schema, "police").await;
    assert_eq!(evidence.len(), 5, "{log:?}");
    assert!(evidence.iter().any(|e| e.source_references.len() == 2));
    for item in &evidence {
        println!("{}\n{}", item.label, item.text);
    }
    std::fs::write(
        std::env::temp_dir().join("onrecord-finance-comparisons.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    let investigation = research::investigate(&db, "police", true);
    tokio::pin!(investigation);
    loop {
        tokio::select! {
            result=&mut investigation=>{let status=db.research_status().await.unwrap();assert!(result.is_ok(),"{}; {} calls; {:?}",status.phase,status.calls_today,result.err());break;},
            _=tokio::time::sleep(Duration::from_secs(30))=>{let status=db.research_status().await.unwrap();println!("Research: {} ({} calls)",status.phase,status.calls_today);}
        }
    }
    let status = db.research_status().await.unwrap();
    let report = &status.reports[0];
    std::fs::write(
        std::env::temp_dir().join("onrecord-live-finance-dossier.json"),
        serde_json::to_vec_pretty(report).unwrap(),
    )
    .unwrap();
    assert!(!report.synthesis.findings.is_empty());
    assert!(!report.synthesis.requests.is_empty());
    assert!(report
        .synthesis
        .findings
        .iter()
        .any(|f| f.citations.iter().any(|c| report
            .evidence
            .iter()
            .any(|e| e.id == c.evidence_id && e.sql.is_some()))));
    for _ in 0..8 {
        if !db
            .compact_one_document(&documents, "1970-01-01T00:00:00.000Z")
            .await
            .unwrap()
        {
            break;
        }
    }
    let storage = db.maintenance_status().await.unwrap();
    assert_eq!(
        storage.referenced_documents, 2,
        "Both fully imported CSV originals should be released: {storage:?}"
    );
    let (after, _) = research::budget::seed(&db, &schema, "police").await;
    assert_eq!(after.len(), 5);
    std::fs::write(
        std::env::temp_dir().join("onrecord-live-finance-dossier.json"),
        serde_json::to_vec_pretty(report).unwrap(),
    )
    .unwrap();
    println!(
        "{}; {} findings, {} requests; both originals released",
        report.synthesis.title,
        report.synthesis.findings.len(),
        report.synthesis.requests.len()
    );
}
