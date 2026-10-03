use super::super::frontier::{enqueue_task, pool_seed_tasks, resume_initial_tasks};
use super::*;

// The seed is what decides where a resumed portal re-enters its request index whenever one is
// enqueued at all: `start_task_pool` drops it only when the adopted frontier proves the walk
// was interrupted mid-pagination (see `pool_seed_tasks`), and otherwise adds it alongside
// every adopted row. Before the window was persisted this was hardcoded to page 1 at size 50,
// which meant an operator who started at page 400 got the first 399 pages crawled on the next
// launch.
#[test]
fn a_resumed_portal_seeds_the_page_window_the_operator_saved() {
    let source = PortalRegistration {
        portal_key: "records".to_string(),
        display_name: "Records".to_string(),
        canonical_url: "https://records.example.test".to_string(),
        origin_url: "https://records.example.test".to_string(),
        api_layout: PortalApiLayout {
            requests_endpoint: "https://records.example.test/client/requests".to_string(),
            requests_pointer: "/requests".to_string(),
            metadata_pointer: None,
            page_parameter: "page".to_string(),
            page_size_parameter: "per_page".to_string(),
        },
        verified_at: "2026-08-29T00:00:00Z".to_string(),
        source_kind: SourceKind::NextRequest,
    };

    let saved = resume_initial_tasks(
        &source,
        MetadataPageWindow {
            start_page: 400,
            page_size: 25,
        },
    );
    assert!(
        matches!(
            saved.as_slice(),
            [IngestionTask::FetchMetadataPage {
                page: 400,
                per_page: 25,
                ..
            }]
        ),
        "a resume must re-enter at the saved window, got: {saved:?}"
    );

    let unset = resume_initial_tasks(&source, MetadataPageWindow::default());
    assert!(
        matches!(
            unset.as_slice(),
            [IngestionTask::FetchMetadataPage {
                page: 1,
                per_page: 50,
                ..
            }]
        ),
        "a source with no saved window must resume exactly as it did before, got: {unset:?}"
    );
}

// REGRESSION(queue): a resume enqueued its start-page seed alongside the adopted frontier
// unconditionally, so a NextRequest source paused at page 900 re-fetched pages 1..900 on every
// launch while page 901 sat pending in its own frontier. Proven by removing the
// `walk_interrupted` arm from `pool_seed_tasks` and confirming the first case below fails.
//
// The two cases that keep the seed are the point of the fix as much as the case that drops it:
// an operator start is an instruction, and a walk with no surviving page row finished, so
// re-entering at the start page is the only way it ever sees records filed since.
#[test]
fn a_resume_defers_to_an_interrupted_walk_and_re_seeds_every_other_start() {
    let seed = || {
        vec![
            IngestionTask::FetchMetadataPage {
                subdomain: "records".to_string(),
                page: 400,
                per_page: 25,
                search: None,
            },
            IngestionTask::CrawlPublicResource {
                subdomain: "records".to_string(),
                url: "https://records.example.test".to_string(),
                depth: 0,
            },
        ]
    };

    let resumed = pool_seed_tasks(seed(), PoolSeedPolicy::ContinueInterruptedWalk, true);
    assert!(
        !resumed
            .iter()
            .any(|task| matches!(task, IngestionTask::FetchMetadataPage { .. })),
        "a resume whose frontier still owes a page must not re-seed the index: {resumed:?}"
    );
    assert_eq!(
        resumed.len(),
        1,
        "only the request-index seed is dropped; a crawl seed is a separate question: {resumed:?}"
    );

    assert_eq!(
        pool_seed_tasks(seed(), PoolSeedPolicy::ContinueInterruptedWalk, false).len(),
        2,
        "a walk that left no page row finished, and a resume must re-enter at its start page"
    );
    assert_eq!(
        pool_seed_tasks(seed(), PoolSeedPolicy::OperatorRequested, true).len(),
        2,
        "an operator who just chose a start page outranks an inherited frontier"
    );
}

// The prefix and the identifier are two constructions of the same string, and the frontier
// query that decides the above matches rows by it. A drift between them would not fail any
// test that only exercised `pool_seed_tasks`: the query would simply never match, and every
// resume would silently re-seed again.
#[test]
fn the_metadata_page_prefix_matches_the_identifier_such_a_task_is_stored_under() {
    let prefix = IngestionTask::metadata_page_target_prefix("records");
    let page = IngestionTask::FetchMetadataPage {
        subdomain: "records".to_string(),
        page: 901,
        per_page: 50,
        search: None,
    };
    assert!(
        page.target_id().starts_with(&prefix),
        "a stored page task must be findable by the prefix, got '{}' against '{prefix}'",
        page.target_id()
    );
    assert!(
        !IngestionTask::CrawlPublicResource {
            subdomain: "records".to_string(),
            url: "https://records.example.test".to_string(),
            depth: 0,
        }
        .target_id()
        .starts_with(&prefix),
        "no other task kind may answer for the request index"
    );
    assert!(
        !IngestionTask::metadata_page_target_prefix("records-2").starts_with(&prefix),
        "one source key must not carry another's prefix"
    );
}

#[test]
fn preserves_queue_statistics_when_backpressure_rejects_a_task() {
    let statistics = PortalStatistics::default();
    let (sender, _receiver) = mpsc::channel(1);
    enqueue_task(&sender, &statistics, metadata_task(1)).unwrap();
    assert_eq!(statistics.queued_tasks.load(Ordering::Acquire), 1);

    assert!(enqueue_task(&sender, &statistics, metadata_task(2)).is_err());
    assert_eq!(statistics.queued_tasks.load(Ordering::Acquire), 1);
}

#[test]
fn preserves_queue_statistics_when_the_receiver_is_closed() {
    let statistics = PortalStatistics::default();
    let (sender, receiver) = mpsc::channel(1);
    drop(receiver);

    assert!(enqueue_task(&sender, &statistics, metadata_task(1)).is_err());
    assert_eq!(statistics.queued_tasks.load(Ordering::Acquire), 0);
}

#[test]
fn durable_frontier_round_trip_preserves_paths_and_ignores_the_legacy_msg_backstop() {
    let task = IngestionTask::DownloadBinaryPayload {
        subdomain: "records".to_string(),
        request_id: 41,
        document: Box::new(DocumentMetadata {
            id: 73,
            file_name: Some("evidence.pdf".to_string()),
            file_size: Some(128),
            description: None,
            content_type: Some("application/pdf".to_string()),
            download_url: None,
            created_at: "2026-08-24T00:00:00Z".to_string(),
            updated_at: "2026-08-24T00:00:00Z".to_string(),
            visibility: "public".to_string(),
            local_path: Some("/managed/objects/abc".to_string()),
            extracted_text_sha256: None,
            was_ocr_processed: false,
        }),
        output_directory: "/managed/records".to_string(),
        container_depth: 0,
        container_root_directory: Some("/managed/root_contents".to_string()),
        crawl_depth: 5,
        structured_depth: 3,
    };
    let encoded = serde_json::to_string(&task).unwrap();
    let decoded: IngestionTask = serde_json::from_str(&encoded).unwrap();
    let IngestionTask::DownloadBinaryPayload {
        document,
        container_root_directory,
        crawl_depth,
        structured_depth,
        ..
    } = decoded
    else {
        panic!("durable task kind changed during round trip");
    };
    assert_eq!(document.local_path.as_deref(), Some("/managed/objects/abc"));
    assert_eq!(
        container_root_directory.as_deref(),
        Some("/managed/root_contents")
    );
    assert_eq!(crawl_depth, 5);
    assert_eq!(structured_depth, 3);

    // Rows written before `.msg` expansion became format- and depth-driven carry this field.
    // Serde's default unknown-field behavior must let those durable tasks resume after upgrade.
    let mut legacy_encoded = serde_json::to_value(&task).unwrap();
    legacy_encoded.as_object_mut().unwrap().insert(
        "skip_msg_attachment_expansion".to_string(),
        serde_json::Value::Bool(true),
    );
    let legacy_decoded: IngestionTask = serde_json::from_value(legacy_encoded).unwrap();
    let IngestionTask::DownloadBinaryPayload {
        document,
        container_depth,
        ..
    } = legacy_decoded
    else {
        panic!("legacy durable task kind changed during resume");
    };
    assert_eq!(document.id, 73);
    assert_eq!(container_depth, 0);
}

#[test]
fn a_frontier_row_written_before_the_depth_fields_existed_resumes_at_depth_zero() {
    let task = IngestionTask::DownloadBinaryPayload {
        subdomain: "records".to_string(),
        request_id: 41,
        document: Box::new(DocumentMetadata {
            id: 73,
            file_name: Some("evidence.pdf".to_string()),
            file_size: Some(128),
            description: None,
            content_type: Some("application/pdf".to_string()),
            download_url: None,
            created_at: "2026-08-24T00:00:00Z".to_string(),
            updated_at: "2026-08-24T00:00:00Z".to_string(),
            visibility: "public".to_string(),
            local_path: Some("/managed/objects/abc".to_string()),
            extracted_text_sha256: None,
            was_ocr_processed: false,
        }),
        output_directory: "/managed/records".to_string(),
        container_depth: 0,
        container_root_directory: Some("/managed/root_contents".to_string()),
        crawl_depth: 5,
        structured_depth: 3,
    };
    let mut encoded = serde_json::to_value(&task).unwrap();
    let object = encoded.as_object_mut().unwrap();
    for field in [
        "crawl_depth",
        "structured_depth",
        "container_root_directory",
    ] {
        assert!(
            object.remove(field).is_some(),
            "the durable payload must carry {field}"
        );
    }

    let decoded: IngestionTask = serde_json::from_value(encoded).unwrap();
    let IngestionTask::DownloadBinaryPayload {
        container_root_directory,
        crawl_depth,
        structured_depth,
        ..
    } = decoded
    else {
        panic!("durable task kind changed during round trip");
    };
    assert_eq!(crawl_depth, 0);
    assert_eq!(structured_depth, 0);
    assert_eq!(container_root_directory, None);
}
