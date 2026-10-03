use super::super::{
    frontier::resume_initial_tasks,
    objects::{commit_staged_object, release_object_link},
    worker::retire_portal_if_idle,
};
use super::*;

/// A purge that released an object must not unlink it while a sync that just linked the same
/// bytes has yet to commit the row referencing it.
#[tokio::test]
async fn purge_leaves_an_object_a_running_sync_just_linked() {
    let root = tempdir().unwrap();
    let staging = root.path().join("staged");
    tokio::fs::write(&staging, b"shared evidence")
        .await
        .unwrap();
    let object = commit_staged_object(root.path(), &staging, &sha256_hex(b"shared evidence"))
        .await
        .unwrap();
    let path = object.to_string_lossy().into_owned();
    assert_eq!(
        remove_released_evidence_files(std::slice::from_ref(&path)).await,
        vec![path.clone()]
    );
    assert!(object.exists());

    // Once a row references it, the purge-time reference check is the guard instead.
    release_object_link(&object).await;
    assert!(remove_released_evidence_files(&[path]).await.is_empty());
    assert!(!object.exists());
}

// The input is the canonical registration returned AFTER redirect/platform discovery,
// while submitted_url deliberately names a different alias. No network is needed to
// exercise the mutation boundary: disabling its identity guard must alter the stored
// lifecycle/recipe and fail these assertions.
#[tokio::test]
async fn automatic_discovery_preserves_resolved_source_lifecycle_and_recipe() {
    let directory = tempdir().unwrap();
    let (manager, repository, source, _telemetry) = manager_with_source(&directory).await;
    manager.global_semaphore.close(); // Mutants must never issue an actual network request.
    let saved_guidance = SourceGuidance {
        link_selector: Some("main a[href]".into()),
        ..Default::default()
    };
    repository
        .save_source_connector_recipe(
            &source,
            "https://records.example.test/original-alias",
            &CrawlPolicy::default(),
            &saved_guidance,
        )
        .await
        .unwrap();
    let original_recipe = repository
        .load_source_connector_recipe_for_source(&source)
        .await
        .unwrap()
        .unwrap();
    for state in [
        SourceLifecycleState::Paused,
        SourceLifecycleState::Removed,
        SourceLifecycleState::Active,
    ] {
        repository
            .set_source_lifecycle_state(&source.portal_key, state)
            .await
            .unwrap();
        for reclassified in [false, true] {
            let mut resolved = source.clone();
            resolved.display_name = "Unwanted automatic replacement".into();
            if reclassified {
                // A changed platform classification can change the derived key;
                // the canonical publication still belongs to the saved source.
                resolved.portal_key = "source-reclassified".into();
                resolved.source_kind = SourceKind::WebCrawl;
                resolved.canonical_url.push('/');
            }
            let result = manager
                .start_task_pool(
                    resolved.clone(),
                    RateLimitPolicy::default(),
                    resume_initial_tasks(&resolved, MetadataPageWindow::default()),
                    CrawlPolicy {
                        maximum_pages: 2,
                        maximum_depth: 1,
                        maximum_resources: 8,
                        follow_supported_external_platforms: false,
                    },
                    SourceGuidance::default(),
                    PoolSeedPolicy::AutomaticDiscovery {
                        submitted_url: "https://records.example.test/redirecting-alias".into(),
                    },
                )
                .await;
            assert!(
                matches!(result, Err(AppError::DataParsing(ref message)) if message.contains("existing source")),
                "Automatic discovery must reject the resolved registered identity: {result:?}"
            );
            assert_eq!(
                repository
                    .source_lifecycle_state(&source.portal_key)
                    .await
                    .unwrap(),
                Some(state)
            );
            let retained = repository
                .load_source_connector_recipe_for_source(&source)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                (
                    &retained.crawl_policy,
                    &retained.guidance,
                    &retained.metadata_page_window
                ),
                (
                    &original_recipe.crawl_policy,
                    &original_recipe.guidance,
                    &original_recipe.metadata_page_window
                ),
                "Rejected discovery must not replace the owner's collection recipe"
            );
            if reclassified {
                assert_eq!(
                    repository
                        .source_lifecycle_state(&resolved.portal_key)
                        .await
                        .unwrap(),
                    None
                );
            }
        }
    }
    let conn = repository.analytics_connection().await.unwrap();
    let (registrations, runs, label): (i64, i64, String) = conn.query_row(
        "SELECT (SELECT count(*) FROM portals), (SELECT count(*) FROM portal_sync_runs), display_name FROM portals WHERE subdomain = ?1",
        [&source.portal_key], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!((registrations, runs, label), (1, 0, source.display_name));
}

#[tokio::test]
async fn automatic_discovery_rechecks_identity_after_a_concurrent_pause() {
    let directory = tempdir().unwrap();
    let (manager, repository, source, _telemetry) = manager_with_source(&directory).await;
    manager.global_semaphore.close();
    let held = manager.provisioning_lock.lock().await;
    let (ready, waiting) = tokio::sync::oneshot::channel();
    let pause_manager = Arc::clone(&manager);
    let pause_key = source.portal_key.clone();
    let pause = tokio::spawn(async move {
        ready.send(()).unwrap();
        pause_manager.pause_source(&pause_key).await
    });
    waiting.await.unwrap();
    // Tokio's mutex is FIFO: pause has reached the lock before discovery is polled.
    let start_manager = Arc::clone(&manager);
    let resolved = source.clone();
    let start = tokio::spawn(async move {
        start_manager
            .start_task_pool(
                resolved.clone(),
                RateLimitPolicy::default(),
                resume_initial_tasks(&resolved, MetadataPageWindow::default()),
                CrawlPolicy::default(),
                SourceGuidance::default(),
                PoolSeedPolicy::AutomaticDiscovery {
                    submitted_url: "https://records.example.test/alias".into(),
                },
            )
            .await
    });
    tokio::task::yield_now().await;
    assert!(
        !start.is_finished(),
        "The identity check must share the lifecycle mutation lock"
    );
    drop(held);
    pause.await.unwrap().unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), start)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.is_err(),
        "A discovery resolved before pause cannot reactivate its source"
    );
    assert_eq!(
        repository
            .source_lifecycle_state(&source.portal_key)
            .await
            .unwrap(),
        Some(SourceLifecycleState::Paused)
    );
    let conn = repository.analytics_connection().await.unwrap();
    let runs: i64 = conn
        .query_row("SELECT count(*) FROM portal_sync_runs", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        runs, 0,
        "Rejected discovery cannot leave even a cancelled automatic run"
    );
}

#[tokio::test]
async fn finishing_worker_observes_cancellation_while_lifecycle_lock_is_held() {
    let directory = tempdir().unwrap();
    let context = crawl_worker_context(&directory, SourceGuidance::default()).await;
    // A real worker_loop owns this receiver independently of retirement. Keep
    // it alive so the negative control reaches the lock timeout, not SendError.
    let _worker_cancellation = context.cancellation.subscribe();
    let lifecycle = context.provisioning_lock.lock().await;
    let worker = context.clone();
    let (ready, waiting) = tokio::sync::oneshot::channel();
    let retirement = tokio::spawn(async move {
        ready.send(()).unwrap();
        retire_portal_if_idle(&worker).await;
    });
    waiting.await.unwrap();
    tokio::task::yield_now().await;
    assert!(
        !retirement.is_finished(),
        "An uncancelled worker must wait for normal lifecycle finalization"
    );
    context
        .statistics
        .stop_requested
        .store(true, Ordering::Release);
    context.cancellation.send(true).unwrap();
    // Keep the lifecycle lock held, exactly as stop_source_pool_locked does
    // while joining this worker. Cancellation must break the lock cycle.
    tokio::time::timeout(std::time::Duration::from_secs(3), retirement)
        .await
        .expect("A stopped worker must not wait on the lifecycle lock held by its joiner")
        .unwrap();
    assert!(context.provisioning_lock.try_lock().is_err());
    drop(lifecycle);
    // Cancellation sent before retirement subscribes must also be honored.
    let held_again = context.provisioning_lock.lock().await;
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        retire_portal_if_idle(&context),
    )
    .await
    .expect("Already-cancelled retirement must return without acquiring the lifecycle lock");
    drop(held_again);
}

#[tokio::test]
async fn automatic_discovery_accepts_new_sources_and_operator_starts_can_resume_them() {
    let directory = tempdir().unwrap();
    let (manager, repository, _source, _telemetry) = manager_with_source(&directory).await;
    manager.global_semaphore.close(); // Starts are real; payload workers remain network-free.
    let fresh = second_source("new-automatic-source");
    let crawl = CrawlPolicy {
        maximum_pages: 2,
        maximum_depth: 1,
        maximum_resources: 8,
        follow_supported_external_platforms: false,
    };
    let alias = "https://second.example.test/alias";
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        manager.start_task_pool(
            fresh.clone(),
            RateLimitPolicy::default(),
            resume_initial_tasks(&fresh, MetadataPageWindow::default()),
            crawl.clone(),
            SourceGuidance::default(),
            PoolSeedPolicy::AutomaticDiscovery {
                submitted_url: alias.into(),
            },
        ),
    )
    .await
    .expect("A new automatic source must finish provisioning")
    .unwrap();
    assert_eq!(
        repository
            .source_lifecycle_state(&fresh.portal_key)
            .await
            .unwrap(),
        Some(SourceLifecycleState::Active)
    );
    let recipe = repository
        .load_source_connector_recipe(alias)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recipe.crawl_policy, crawl);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        manager.pause_source(&fresh.portal_key),
    )
    .await
    .expect(
        "Pausing the new automatic source must join its workers without a lifecycle-lock deadlock",
    )
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        manager.start_task_pool(
            fresh.clone(),
            RateLimitPolicy::default(),
            resume_initial_tasks(&fresh, MetadataPageWindow::default()),
            CrawlPolicy::default(),
            SourceGuidance::default(),
            PoolSeedPolicy::OperatorRequested,
        ),
    )
    .await
    .expect("An explicit operator resume must finish provisioning")
    .unwrap();
    assert_eq!(
        repository
            .source_lifecycle_state(&fresh.portal_key)
            .await
            .unwrap(),
        Some(SourceLifecycleState::Active)
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        manager.pause_source(&fresh.portal_key),
    )
    .await
    .expect("Pausing the resumed source must join its workers without a lifecycle-lock deadlock")
    .unwrap();
}

/// Covers the lifecycle writes as one sequence, and the lock discipline underneath them.
///
/// `pause_source` and `remove_source` both take `provisioning_lock` and then call the shared
/// teardown. That lock is a plain `tokio::sync::Mutex`, so re-entering it deadlocks rather
/// than erroring, and a deadlock is invisible to review and fatal in the app. This test hangs
/// instead of failing if either grows a second acquisition.
#[tokio::test]
async fn automatic_refresh_never_reactivates_paused_removed_or_purged_sources() {
    let directory = tempdir().unwrap();
    let (manager, repository, source, _telemetry) = manager_with_source(&directory).await;
    manager.pause_source(&source.portal_key).await.unwrap();
    assert!(manager
        .refresh_registered_source(
            source.clone(),
            CrawlPolicy::default(),
            SourceGuidance::default(),
            None
        )
        .await
        .is_err());
    assert_eq!(
        repository
            .source_lifecycle_state(&source.portal_key)
            .await
            .unwrap(),
        Some(SourceLifecycleState::Paused)
    );
    manager.remove_source(&source.portal_key).await.unwrap();
    assert!(manager
        .refresh_registered_source(
            source.clone(),
            CrawlPolicy::default(),
            SourceGuidance::default(),
            None
        )
        .await
        .is_err());
    assert_eq!(
        repository
            .source_lifecycle_state(&source.portal_key)
            .await
            .unwrap(),
        Some(SourceLifecycleState::Removed)
    );
    manager.purge_source(&source.portal_key).await.unwrap();
    assert!(manager
        .refresh_registered_source(
            source.clone(),
            CrawlPolicy::default(),
            SourceGuidance::default(),
            None
        )
        .await
        .is_err());
    assert_eq!(
        repository
            .source_lifecycle_state(&source.portal_key)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn pausing_keeps_the_queue_and_removing_discards_it() {
    let directory = tempdir().expect("temp dir must be created");
    let (manager, repository, source, _telemetry) = manager_with_source(&directory).await;
    repository
        .begin_portal_sync_run("run-1", &source.portal_key)
        .await
        .expect("sync run must begin");
    repository
        .persist_ingestion_frontier_task(
            "run-1",
            &source.portal_key,
            "page-2",
            r#"{"taskKind":"fetchMetadataPage","subdomain":"records","page":2,"perPage":50}"#,
        )
        .await
        .expect("queued work must persist");

    assert!(
        !manager
            .pause_source(&source.portal_key)
            .await
            .expect("pausing an idle source must succeed"),
        "no pool was running, and saying otherwise would be a false report"
    );
    assert_eq!(
        repository
            .source_lifecycle_state(&source.portal_key)
            .await
            .expect("lifecycle lookup must succeed"),
        Some(SourceLifecycleState::Paused)
    );
    assert!(
        repository
            .recover_interrupted_source_keys()
            .await
            .expect("startup recovery scan must succeed")
            .is_empty(),
        "a paused source must not be resumed by the next launch"
    );

    let removal = manager
        .remove_source(&source.portal_key)
        .await
        .expect("removal must succeed");
    assert_eq!(removal.source_key, source.portal_key);
    assert_eq!(
        removal.discarded_frontier_tasks, 1,
        "the queued page was the only frontier row, and removal discards it"
    );
    assert_eq!(
        removal.retained_documents, 0,
        "nothing was ingested, and the count must report what is actually kept"
    );
    assert!(
        repository
            .list_portal_registrations()
            .await
            .expect("listing must succeed")
            .is_empty(),
        "a removed source leaves the list"
    );
    assert!(
        manager.pause_source(&source.portal_key).await.is_err(),
        "pausing a removed source would put it back on the list"
    );

    // Re-adding is the same write every start path performs, and it must restore the source
    // rather than leave it invisible forever.
    repository
        .insert_or_update_portal_registration(&source)
        .await
        .expect("re-registration must persist");
    assert_eq!(
        repository
            .source_lifecycle_state(&source.portal_key)
            .await
            .expect("lifecycle lookup must succeed"),
        Some(SourceLifecycleState::Active)
    );
}

// REGRESSION(queue): a start whose initial enqueue failed joined its workers before signalling
// them, so the join never returned while `provisioning_lock` was held and every later start,
// pause and removal blocked. To prove it, make `start_task_pool`'s failure branch join before
// signalling again: the start below then exceeds the timeout.
#[tokio::test]
async fn a_failed_initial_enqueue_stops_its_workers_and_releases_the_lifecycle_lock() {
    let directory = tempdir().expect("temp dir must be created");
    let (manager, repository, source, _telemetry) = manager_with_source(&directory).await;
    manager.global_semaphore.close(); // Keep any dispatched task network-free.
    repository
        .begin_portal_sync_run("run-1", &source.portal_key)
        .await
        .expect("sync run must begin");
    repository
        .persist_ingestion_frontier_task("run-1", &source.portal_key, "corrupt", "{not json")
        .await
        .expect("queued work must persist");
    repository
        .finish_portal_sync_run(PortalSyncRunFinalization {
            run_id: "run-1",
            portal_key: &source.portal_key,
            status: "CANCELLED",
            pages_processed: 0,
            requests_processed: 0,
            documents_processed: 0,
            failures: 0,
        })
        .await
        .expect("interrupted run must finalize");

    // The corrupt row is adopted, claimed first during the start's own refill, and refused as
    // invalid: an error from inside the enqueue step, after the workers are already running.
    let started = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        manager.start_verified_portal(
            source.clone(),
            MetadataPageWindow::default(),
            RateLimitPolicy::default(),
            SourceGuidance::default(),
        ),
    )
    .await
    .expect("a failed start must return instead of waiting on workers nobody cancelled");
    assert!(
        started.is_err(),
        "the invalid frontier row must fail the start"
    );
    assert!(!manager.is_source_running(&source.portal_key).await);
    assert!(
        manager.provisioning_lock.try_lock().is_ok(),
        "a failed start must not leave the lifecycle lock held"
    );
}

fn second_source(portal_key: &str) -> PortalRegistration {
    PortalRegistration {
        portal_key: portal_key.to_string(),
        display_name: "Second".to_string(),
        canonical_url: "https://second.example.test".to_string(),
        origin_url: "https://second.example.test".to_string(),
        api_layout: PortalApiLayout {
            requests_endpoint: "https://second.example.test/client/requests".to_string(),
            requests_pointer: "/requests".to_string(),
            metadata_pointer: None,
            page_parameter: "page".to_string(),
            page_size_parameter: "per_page".to_string(),
        },
        verified_at: "2026-08-26T00:00:00Z".to_string(),
        source_kind: SourceKind::NextRequest,
    }
}

fn placeholder_pool(portal_key: &str) -> PortalPool {
    // No worker handles and no real network activity: this pool exists only to occupy a slot
    // in `manager.pools` deterministically, so the park/promote test below is not racing a
    // real worker's `retire_portal_if_idle` self-teardown (which the first version of this
    // test did, and which flaked under load: the seeded fetch task can fail fast against a
    // fake host and the pool retires itself before the test observes the parked state).
    PortalPool {
        portal: second_source(portal_key),
        cancellation: watch::channel(false).0,
        statistics: Arc::new(PortalStatistics::default()),
        sync_run_id: "placeholder-run".to_string(),
        worker_handles: Vec::new(),
        crawl: Arc::new(CrawlState {
            policy: CrawlPolicy::default(),
            guidance: SourceGuidance::default(),
            subject: None,
            visited_urls: Mutex::new(HashSet::new()),
            resource_urls: Mutex::new(HashSet::new()),
            crawl_path_claims: Mutex::new(HashSet::new()),
            announced_host_stops: Mutex::new(HashSet::new()),
            frontier_exclusions: Mutex::new(BTreeMap::new()),
            pages_started: AtomicUsize::new(0),
            resources_started: AtomicUsize::new(0),
            frontier_refill_lock: Mutex::new(()),
            arcgis_page_hashes: Mutex::new(HashSet::new()),
            compiled_guidance: CompiledGuidancePatterns::compile(&SourceGuidance::default()),
        }),
    }
}

// REGRESSION(queue): registration used to be capped by MAXIMUM_ACTIVE_PORTALS, so a source
// beyond the concurrency ceiling was refused outright rather than parked. Proven by reverting
// `start_task_pool`'s park loop to an immediate `Err(AppError::PortalCapacity(..))` on a full
// pool map and confirming the parked start below returns that error instead of completing once
// the occupying pool is removed.
#[tokio::test]
async fn a_start_beyond_the_concurrency_cap_parks_and_is_promoted_when_a_slot_frees() {
    let directory = tempdir().expect("temp dir must be created");
    let (manager, _repository, _first, _telemetry) = manager_with_source(&directory).await;
    let second = second_source("second-records");
    _repository
        .insert_or_update_portal_registration(&second)
        .await
        .expect("second source registration must persist");

    manager
        .set_max_active_portals(1)
        .expect("a positive cap must be accepted");
    let placeholder = second_source("placeholder");
    _repository
        .insert_or_update_portal_registration(&placeholder)
        .await
        .expect("placeholder source registration must persist");
    _repository
        .begin_portal_sync_run("placeholder-run", "placeholder")
        .await
        .expect("placeholder sync run must begin");
    manager
        .pools
        .write()
        .await
        .insert("placeholder".to_string(), placeholder_pool("placeholder"));

    let parked_manager = Arc::clone(&manager);
    let parked_second = second.clone();
    let parked = tokio::spawn(async move {
        parked_manager
            .start_verified_portal(
                parked_second,
                MetadataPageWindow::default(),
                RateLimitPolicy::default(),
                SourceGuidance::default(),
            )
            .await
    });

    // Give the parked start a moment to actually reach the park point rather than merely
    // asserting it eventually succeeds, which would also pass if it never parked at all. The
    // placeholder pool above never tears itself down, so there is nothing racing this check.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !parked.is_finished(),
        "a start over the cap must park rather than resolve immediately"
    );

    // Through the real removal path, not a hand-fired signal: a regression where a removal
    // site forgets to signal `slot_freed` (this happened once during development, on the
    // natural-completion path) must fail this test, not merely the park mechanism in
    // isolation.
    let removal_guard = manager.provisioning_lock.lock().await;
    manager
        .stop_source_pool_locked("placeholder")
        .await
        .expect("stopping the placeholder pool must succeed");
    drop(removal_guard);

    let promoted = tokio::time::timeout(std::time::Duration::from_secs(5), parked)
        .await
        .expect("the parked start must be promoted once the slot frees")
        .expect("the spawned task must not panic");
    assert!(
        promoted.is_ok(),
        "a start parked on a full cap must succeed once a slot frees, not be refused: {:?}",
        promoted.err()
    );
}
