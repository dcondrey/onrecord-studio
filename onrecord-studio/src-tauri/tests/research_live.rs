//! Explicitly opt-in: reads public City records and uses a configured OpenRouter or OpenAI key from ~/.env.
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
#[ignore = "Downloads real San Diego funding/shelter records and uses up to seven billed model/search calls"]
async fn real_homelessness_records_produce_a_grounded_investigation() {
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    db.initialize_research().await.unwrap();
    let mut policy = db.research_policy().await.unwrap();
    policy.research_passes = 3;
    policy.discover_sources = true;
    db.save_research_policy(policy).await.unwrap();
    let corpus = CorpusIndex::open(temp.path().join("corpus.db")).unwrap();
    let client = ApiClient::new(HttpClientConfig {
        max_retries: 0,
        request_timeout_seconds: 30,
        max_document_bytes: 30 * 1024 * 1024,
        host_request_budget: 30,
        ..Default::default()
    })
    .unwrap();
    let (sender, mut events) = tokio::sync::mpsc::channel(2048);
    std::fs::create_dir_all(temp.path().join("documents")).unwrap();
    let manager = std::sync::Arc::new(
        QueueManager::new(
            client,
            db.clone(),
            corpus,
            RateLimiter::new(1).unwrap(),
            sender,
            temp.path().join("documents"),
        )
        .unwrap(),
    );
    let sources = [
        "https://www.sandiego.gov/sites/default/files/23-21_fy_2024_homelessness_programs_and_funding.pdf",
        "https://www.sandiego.gov/sites/default/files/2024-09/24-24-rev-fy-2025-homelessness-programs-and-funding_rev.pdf",
        "https://information.auditor.ca.gov/reports/2023-102.2/index.html",
        "https://www.sandiego.gov/sites/default/files/2026-09/26-19-fy-2027-homelessness-programs-and-funding_rev.pdf",
        "https://www.sandiego.gov/sites/default/files/2026-09/9-28-26-special.pdf",
        "https://sdhc.org/103_hcr26-014-imperial-shelter_final/",
        "https://sdhc.org/104_hcr26-015-harm-reduction-shelter_final/",
        "https://sdhc.org/101_hcr26-016-single-adults-shelter_final/",
        "https://www.sandiego.gov/sites/default/files/fy23-year-end-performance-report.pdf",
        "https://www.sandiego.gov/sites/default/files/2026-05/fy2026-third-quarter-budget-monitoring-report.pdf",
    ];
    for url in sources {
        let start = manager
            .discover_and_start_public_source(
                url,
                CrawlPolicy {
                    maximum_pages: 1,
                    maximum_depth: 0,
                    maximum_resources: 1,
                    follow_supported_external_platforms: false,
                },
                SourceGuidance {
                    link_selector: Some("main a[href]".into()),
                    ..Default::default()
                },
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
        .unwrap();
        assert!(failures.is_empty(), "{failures:?}");
        println!("Acquired {url}");
    }
    let evidence = db.research_search("homelessness").await.unwrap();
    assert!(
        evidence.len() >= 2,
        "Multiple published sources must reach searchable extraction"
    );
    let agendas = onrecord_studio_lib::research::agenda::evidence(&db, "homelessness")
        .await
        .unwrap();
    assert!(agendas.candidates.iter().any(|item|item.date=="2026-09-28" && item.agenda_item.replace(' ',"").eq_ignore_ascii_case("Item605")),
        "The native extraction must recover the complete riverbed amendment item before any billed model calls");
    let investigation = onrecord_studio_lib::research::investigate_with_collection(
        &db,
        Some(&manager),
        "homelessness",
        true,
    );
    tokio::pin!(investigation);
    loop {
        tokio::select! {
            result = &mut investigation => {
                let progress = db.research_status().await.unwrap();
                assert!(result.is_ok(), "Phase: {}; calls: {}; error: {:?}", progress.phase, progress.calls_today, result.err());
                break;
            }
            _ = tokio::time::sleep(Duration::from_secs(30)) => {
                let progress = db.research_status().await.unwrap();
                println!("Research: {} ({} calls)", progress.phase, progress.calls_today);
            }
        }
    }
    let status = db.research_status().await.unwrap();
    assert_eq!(status.reports.len(), 1);
    let report = &status.reports[0];
    std::fs::write(
        std::env::temp_dir().join("onrecord-live-research-dossier.json"),
        serde_json::to_vec_pretty(report).unwrap(),
    )
    .unwrap();
    assert!(
        !report.synthesis.findings.is_empty(),
        "The live records should yield a specific investigative lead"
    );
    assert!(!report.synthesis.requests.is_empty());
    assert!(
        report
            .investigation_log
            .iter()
            .any(|line| line.starts_with("Independent evidence challenge:")),
        "A real disproof retrieval stage must run before editorial review"
    );
    assert!(
        !report
            .investigation_log
            .iter()
            .any(|line| line.contains("Binder Error")),
        "A polished report must not hide a broken deeper-read path"
    );
    assert!(
        report
            .investigation_log
            .iter()
            .any(|line| line.starts_with("Read ")),
        "The live investigation must actually reach deeper retained source context"
    );
    assert!(
        report
            .synthesis
            .meetings
            .iter()
            .any(|meeting| meeting.date == "2026-09-28"),
        "The dated Council agenda includes an actionable encampment grant amendment: {:?}",
        report.synthesis
    );
    assert!(
        report.synthesis.findings.iter().any(|f| f
            .citations
            .iter()
            .filter_map(|c| report.evidence.iter().find(|e| e.id == c.evidence_id))
            .filter_map(|e| e.source_url.as_deref())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            >= 2),
        "Must establish at least one cross-record connection"
    );
    for _ in 0..64 {
        if !db
            .compact_one_document(&temp.path().join("documents"), "1970-01-01T00:00:00.000Z")
            .await
            .unwrap()
        {
            break;
        }
    }
    assert!(
        !db.research_search("homelessness").await.unwrap().is_empty(),
        "Evidence remains searchable after original release"
    );
    let storage = db.maintenance_status().await.unwrap();
    assert!(
        storage.referenced_documents >= sources.len() as u64,
        "Every supplied stable public source should release its processed original: {} released",
        storage.referenced_documents
    );
    let output = std::env::temp_dir().join("onrecord-live-research-dossier.json");
    std::fs::write(&output, serde_json::to_vec_pretty(report).unwrap()).unwrap();
    println!(
        "Dossier: {}\n{}\nFindings: {}; requests: {}; originals released: {}",
        output.display(),
        report.synthesis.title,
        report.synthesis.findings.len(),
        report.synthesis.requests.len(),
        storage.referenced_documents
    );
    for finding in &report.synthesis.findings {
        println!("Finding: {}", finding.title);
    }
}

#[tokio::test]
#[ignore = "Uses one billed model/search call to verify primary-source discovery"]
async fn primary_source_discovery_returns_tool_attested_urls() {
    let temp = tempfile::tempdir().unwrap();
    let db = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
    db.run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    db.initialize_research().await.unwrap();
    let policy = db.research_policy().await.unwrap();
    let result = onrecord_studio_lib::research::model::discover(&db, &policy, &serde_json::json!({"topic":"homelessness", "questions":["Find the newest San Diego IBA analysis of homelessness program funding and the current Council agenda where shelter spending is discussed."], "today":chrono::Utc::now().format("%Y-%m-%d").to_string()})).await.unwrap();
    let accepted = result.value["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["url"].as_str())
        .filter(|url| {
            onrecord_studio_lib::research::trusted_discovery_url(url)
                && result.searched_urls.iter().any(|seen| seen == url)
        })
        .collect::<Vec<_>>();
    assert!(!accepted.is_empty(), "Discovery must return official URLs independently attested by the search tool; model URLs alone do not count");
    for url in accepted {
        println!("Verified discovery: {url}");
    }
}
