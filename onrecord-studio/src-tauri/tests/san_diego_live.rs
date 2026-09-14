use onrecord_studio_lib::ingestion::client::{ApiClient, HttpClientConfig};
use onrecord_studio_lib::ingestion::queue::{IngestionProgress, QueueManager};
use onrecord_studio_lib::ingestion::RateLimiter;
use onrecord_studio_lib::storage::corpus::CorpusIndex;
use onrecord_studio_lib::storage::sqlite::DuckDbRepository;
use onrecord_studio_lib::types::{CrawlPolicy, RateLimitPolicy, SourceGuidance};
use std::time::Duration;

#[tokio::test]
#[ignore = "Inspects two live SDPD catalog pages"]
async fn sdpd_pagination_exposes_additional_documents() {
    let client = ApiClient::new(HttpClientConfig {
        max_retries: 0,
        request_timeout_seconds: 20,
        host_request_budget: 10,
        ..Default::default()
    })
    .unwrap();
    let url = "https://www.sandiego.gov/police/data-transparency/policies-procedures/procedures";
    client.mark_user_directed(url);
    let first = client.inspect_public_resource(url).await.unwrap();
    let next = first
        .links
        .iter()
        .find(|link| link.url.ends_with("?page=1"))
        .expect("SDPD procedures must expose the next page");
    assert!(next.follow_candidate && !next.navigation_chrome);
    let second = client.inspect_public_resource(&next.url).await.unwrap();
    let additional = second
        .links
        .iter()
        .filter(|link| {
            link.download_candidate && !first.links.iter().any(|existing| existing.url == link.url)
        })
        .count();
    assert!(
        additional > 0,
        "the next page must expose additional policy documents"
    );
    println!("Verified SDPD pagination: {additional} additional documents on page 2");
}

// Opt-in because the City's content and availability change. Everything acquired stays in a
// temporary workspace; a successful run must reach searchable text, not merely an HTTP 200.
#[tokio::test]
#[ignore = "Fetches the live City CPP calendar and its first two linked documents"]
async fn cpp_agenda_reaches_search_and_document_view() {
    let temporary = tempfile::tempdir().unwrap();
    let repository = DuckDbRepository::new(temporary.path().join("core.db")).unwrap();
    repository
        .run_migrations(include_str!("../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    let corpus = CorpusIndex::open(temporary.path().join("corpus.db")).unwrap();
    let client = ApiClient::new(HttpClientConfig {
        max_retries: 0,
        request_timeout_seconds: 20,
        max_document_bytes: 20 * 1024 * 1024,
        host_request_budget: 20,
        ..Default::default()
    })
    .unwrap();
    client.mark_user_directed("https://www.sandiego.gov/cpp/meetingsagendas");
    let calendar = client
        .inspect_public_resource("https://www.sandiego.gov/cpp/meetingsagendas")
        .await
        .unwrap();
    let agenda = calendar
        .links
        .iter()
        .find(|link| link.download_candidate && link.label.starts_with("CPP Meeting Agenda"))
        .expect("the current CPP calendar must expose a meeting agenda");
    println!("Live CPP agenda: {} ({})", agenda.label, agenda.url);
    let (sender, mut events) = tokio::sync::mpsc::channel(2048);
    let manager = QueueManager::new(
        client,
        repository.clone(),
        corpus,
        RateLimiter::new(1).unwrap(),
        sender,
        temporary.path().join("documents"),
    )
    .unwrap();
    let start = manager
        .discover_and_start_public_source(
            "https://www.sandiego.gov/cpp/meetingsagendas",
            CrawlPolicy {
                maximum_pages: 1,
                maximum_depth: 0,
                // The preserved HTML calendar itself occupies one resource slot.
                maximum_resources: 3,
                follow_supported_external_platforms: false,
            },
            SourceGuidance {
                allowed_hostnames: vec!["www.sandiego.gov".to_string()],
                include_url_prefixes: vec![
                    "https://www.sandiego.gov/cpp/meetingsagendas".to_string(),
                    "https://www.sandiego.gov/sites/default/files/".to_string(),
                ],
                restrict_downloads_to_source_host: true,
                ..Default::default()
            },
            RateLimitPolicy::default(),
        )
        .await
        .unwrap();
    let mut failures = Vec::new();
    tokio::time::timeout(Duration::from_secs(90), async {
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
    .expect("one agenda should finish within 90 seconds");
    assert!(failures.is_empty(), "ingestion failures: {failures:?}");
    let records = repository
        .search_requests(
            "commission",
            Some(&start.source.portal_key),
            None,
            None,
            None,
            500,
        )
        .await
        .unwrap();
    assert!(!records.is_empty(), "ingested agenda should be searchable");
    let mut documents = Vec::new();
    for record in &records {
        documents.extend(
            repository
                .list_request_documents_for_view(record.request_id, &start.source.portal_key)
                .await
                .unwrap()
                .into_iter()
                .filter(|document| agenda.url.ends_with(&document.file_name)),
        );
    }
    assert_eq!(documents.len(), 1);
    let chunk = repository
        .load_document_text_chunk(
            documents[0].document_id,
            &start.source.portal_key,
            0,
            100000,
        )
        .await
        .unwrap();
    assert!(chunk.returned_character_count > 500);
    println!(
        "Verified: {} searchable records; {} extracted characters; {}",
        records.len(),
        chunk.total_character_count,
        documents[0].file_name
    );
}
