use super::*;

/// Every other test here opens a fresh database, so nothing about an upgraded one is covered.
/// This one writes the rows an older build would have left behind.
///
/// `lease_expires_at` is the only timestamp this repository compares rather than merely stores,
/// and the format change moved both sides of that comparison at once. A row written by an older
/// build reads `2020-01-01T09:41:123456Z`: microseconds in the seconds field, and local
/// wall-clock time labelled `Z`. The comparison against a new `%S.%f` UTC string is lexical, so
/// what has to hold is that the date and hour still decide it. An expired legacy lease must be
/// reclaimed rather than stranding its task `LEASED` forever, and a live one must not be
/// reclaimed out from under a running worker.
///
/// What this does not cover, because no deterministic test can: two timestamps inside the same
/// minute, where the old format's microseconds occupy the seconds field and the ordering is
/// meaningless. Leases are ten minutes long, so that window is not reachable in practice, and
/// the seconds an old row lost cannot be recovered by any backfill.
#[tokio::test]
async fn legacy_format_leases_are_reclaimed_when_expired_and_left_alone_when_live() {
    let repository = repository_with_document().await;
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO portal_sync_runs (run_id, portal_subdomain, started_at, status)
                 VALUES (?1, ?2, ?3, 'RUNNING')",
                params!["run-legacy", PORTAL_SUBDOMAIN, "2026-08-25T09:00:00Z"],
            )
            .expect("sync run fixture must insert");
        for (target, lease) in [
            ("target-expired", "2020-01-01T09:41:123456Z"),
            ("target-live", "2099-01-01T09:41:123456Z"),
        ] {
            connection
                .execute(
                    "INSERT INTO ingestion_task_frontier (
                        run_id, source_key, target_id, payload_json, status,
                        lease_expires_at, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, 'LEASED', ?5, ?6, ?6)",
                    params![
                        "run-legacy",
                        PORTAL_SUBDOMAIN,
                        target,
                        "{}",
                        lease,
                        "2026-08-25T09:00:00Z",
                    ],
                )
                .expect("legacy frontier row must insert");
        }
    }

    let claimed = repository
        .claim_ingestion_frontier_task("run-legacy", PORTAL_SUBDOMAIN)
        .await
        .expect("claiming against legacy leases must not error");
    let claimed = claimed.expect("the expired legacy lease must be recovered");

    let connection = repository.conn.lock().await;
    let claimed_target: String = connection
        .query_row(
            "SELECT target_id FROM ingestion_task_frontier WHERE frontier_id = ?1",
            params![claimed.frontier_id],
            |row| row.get(0),
        )
        .expect("the claimed frontier row must be readable");
    assert_eq!(
        claimed_target, "target-expired",
        "only the expired lease may be recovered"
    );
    let live_status: String = connection
        .query_row(
            "SELECT status FROM ingestion_task_frontier WHERE target_id = ?1",
            params!["target-live"],
            |row| row.get(0),
        )
        .expect("the live frontier row must be readable");
    assert_eq!(
        live_status, "LEASED",
        "a legacy lease that has not expired must not be reclaimed from a running worker"
    );
}

/// `record_source_task_failure` upserts, and its `DO UPDATE SET` clause set `last_occurred_at`
/// from `current_timestamp`. DuckDB binds a bare identifier there against the target table, so
/// the statement never bound and no ingestion failure was ever recorded, on the first
/// occurrence or the repeat. Nothing in the guided-repair surface can work without this row, so
/// the assertion is both halves: the row appears, and a second failure increments it rather
/// than failing to bind.
#[tokio::test]
async fn a_repeated_ingestion_failure_records_once_and_counts_up() {
    let repository = repository_with_document().await;
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO portal_sync_runs (run_id, portal_subdomain, started_at, status)
                 VALUES (?1, ?2, ?3, 'RUNNING')",
                params!["run-1", PORTAL_SUBDOMAIN, "2026-08-25T09:00:00Z"],
            )
            .expect("sync run fixture must insert");
    }
    let failure = AppError::Storage("evidence store unavailable".to_string());
    for _ in 0..2 {
        repository
            .record_source_task_failure("run-1", PORTAL_SUBDOMAIN, "target-7", &failure)
            .await
            .expect("an ingestion failure must be recordable");
    }

    let connection = repository.conn.lock().await;
    let (rows, occurrences, last_occurred_at): (i64, i64, String) = connection
        .query_row(
            "SELECT COUNT(*), MAX(occurrences), MAX(last_occurred_at)
             FROM source_ingestion_failures WHERE run_id = ?1",
            params!["run-1"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("the failure row must be readable");
    assert_eq!(rows, 1, "the same failure must collapse onto one row");
    assert_eq!(occurrences, 2, "the repeat must increment the count");
    chrono::DateTime::parse_from_rfc3339(&last_occurred_at).unwrap_or_else(|error| {
        panic!("last_occurred_at '{last_occurred_at}' must be RFC 3339: {error}")
    });
}

// The resume path asks this query whether a source's request-index walk was interrupted, and
// acts on the answer by not re-seeding the index. A false negative silently restores the
// every-launch re-fetch it exists to close, and a false positive strands a source that had no
// pending page, so both directions are pinned here. `%` in a source key is deliberate: under
// `LIKE` it would be a wildcard and this row would answer for a prefix it does not carry.
#[tokio::test]
async fn the_frontier_reports_a_surviving_page_task_by_prefix_only() {
    let repository = repository_with_document().await;
    repository
        .begin_portal_sync_run("run-prefix", PORTAL_SUBDOMAIN)
        .await
        .expect("sync run should begin");
    assert!(!repository
        .frontier_holds_target_prefix("run-prefix", PORTAL_SUBDOMAIN, "test-portal-page-")
        .await
        .expect("an empty frontier should answer"));

    repository
        .persist_ingestion_frontier_task(
            "run-prefix",
            PORTAL_SUBDOMAIN,
            "test-portal-document-77",
            r#"{"taskKind":"downloadBinaryPayload"}"#,
        )
        .await
        .expect("a document task should persist");
    assert!(
        !repository
            .frontier_holds_target_prefix("run-prefix", PORTAL_SUBDOMAIN, "test-portal-page-")
            .await
            .expect("a document-only frontier should answer"),
        "pending downloads are not an interrupted walk; the walk ended and its documents did not"
    );

    repository
        .persist_ingestion_frontier_task(
            "run-prefix",
            PORTAL_SUBDOMAIN,
            "test-portal-page-901",
            r#"{"taskKind":"fetchMetadataPage"}"#,
        )
        .await
        .expect("a page task should persist");
    assert!(repository
        .frontier_holds_target_prefix("run-prefix", PORTAL_SUBDOMAIN, "test-portal-page-")
        .await
        .expect("a page-bearing frontier should answer"));
    assert!(
        !repository
            .frontier_holds_target_prefix("another-run", PORTAL_SUBDOMAIN, "test-portal-page-")
            .await
            .expect("a foreign run should answer"),
        "a row belongs to the run that owns it, and adoption is what moves it"
    );
    assert!(
        !repository
            .frontier_holds_target_prefix("run-prefix", PORTAL_SUBDOMAIN, "%-page-")
            .await
            .expect("a wildcard-shaped prefix should answer"),
        "the match is a prefix and not a pattern"
    );
}

#[tokio::test]
async fn durable_frontier_leases_and_acknowledges_tasks_exactly_once() {
    let repository = repository_with_document().await;
    repository
        .begin_portal_sync_run("run-frontier", PORTAL_SUBDOMAIN)
        .await
        .expect("sync run should begin");
    assert!(repository
        .persist_ingestion_frontier_task(
            "run-frontier",
            PORTAL_SUBDOMAIN,
            "target-1",
            r#"{"taskKind":"fetchMetadataPage"}"#,
        )
        .await
        .expect("frontier task should persist"));
    assert!(!repository
        .persist_ingestion_frontier_task(
            "run-frontier",
            PORTAL_SUBDOMAIN,
            "target-1",
            r#"{"taskKind":"fetchMetadataPage"}"#,
        )
        .await
        .expect("duplicate frontier task should be ignored"));

    let claimed = repository
        .claim_ingestion_frontier_task("run-frontier", PORTAL_SUBDOMAIN)
        .await
        .expect("frontier claim should succeed")
        .expect("frontier task should be available");
    assert!(repository
        .claim_ingestion_frontier_task("run-frontier", PORTAL_SUBDOMAIN)
        .await
        .expect("second frontier claim should succeed")
        .is_none());
    repository
        .complete_ingestion_frontier_task(claimed.frontier_id)
        .await
        .expect("frontier acknowledgement should succeed");
    assert!(repository
        .claim_ingestion_frontier_task("run-frontier", PORTAL_SUBDOMAIN)
        .await
        .expect("empty frontier claim should succeed")
        .is_none());
}

#[tokio::test]
async fn interrupted_frontier_is_recovered_into_a_new_run() {
    let repository = repository_with_document().await;
    repository
        .begin_portal_sync_run("run-before-crash", PORTAL_SUBDOMAIN)
        .await
        .expect("first sync run should begin");
    for target in ["leased-at-crash", "pending-at-crash"] {
        repository
            .persist_ingestion_frontier_task(
                "run-before-crash",
                PORTAL_SUBDOMAIN,
                target,
                r#"{"taskKind":"fetchMetadataPage","subdomain":"test-portal","page":1,"perPage":50}"#,
            )
            .await
            .expect("recovery fixture should persist");
    }
    repository
        .claim_ingestion_frontier_task("run-before-crash", PORTAL_SUBDOMAIN)
        .await
        .expect("frontier lease should succeed")
        .expect("first task should be leased");
    assert_eq!(
        repository
            .reserve_ingestion_crawl_page("run-before-crash", PORTAL_SUBDOMAIN, 2)
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        repository
            .reserve_ingestion_crawl_page("run-before-crash", PORTAL_SUBDOMAIN, 2)
            .await
            .unwrap(),
        Some(2)
    );
    assert_eq!(
        repository
            .reserve_ingestion_resource("run-before-crash", PORTAL_SUBDOMAIN, 1)
            .await
            .unwrap(),
        Some(1)
    );

    let interrupted = repository
        .recover_interrupted_source_keys()
        .await
        .expect("interrupted run scan should succeed");
    assert_eq!(interrupted, vec![PORTAL_SUBDOMAIN.to_string()]);

    repository
        .begin_portal_sync_run("run-after-restart", PORTAL_SUBDOMAIN)
        .await
        .expect("replacement sync run should begin");
    assert_eq!(
        repository
            .adopt_orphaned_frontier_tasks(PORTAL_SUBDOMAIN, "run-after-restart")
            .await
            .expect("interrupted tasks should be adopted"),
        2
    );
    assert_eq!(
        repository
            .ingestion_scheduler_counts("run-after-restart", PORTAL_SUBDOMAIN)
            .await
            .unwrap(),
        IngestionSchedulerCounts {
            crawl_pages_started: 2,
            resources_started: 1,
        }
    );
    assert_eq!(
        repository
            .reserve_ingestion_crawl_page("run-after-restart", PORTAL_SUBDOMAIN, 2)
            .await
            .unwrap(),
        None,
        "restart must not reset the configured page budget"
    );
    for _ in 0..2 {
        let task = repository
            .claim_ingestion_frontier_task("run-after-restart", PORTAL_SUBDOMAIN)
            .await
            .expect("adopted task claim should succeed")
            .expect("both interrupted tasks must be recoverable");
        repository
            .complete_ingestion_frontier_task(task.frontier_id)
            .await
            .expect("adopted task should acknowledge");
    }
    assert!(repository
        .claim_ingestion_frontier_task("run-after-restart", PORTAL_SUBDOMAIN)
        .await
        .expect("empty recovered frontier query should succeed")
        .is_none());
}

#[tokio::test]
async fn startup_recovery_failure_is_exposed_through_saved_source_health() {
    let repository = repository_with_document().await;
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "UPDATE portals SET requests_endpoint = '/requests' WHERE subdomain = ?1",
                params![PORTAL_SUBDOMAIN],
            )
            .expect("source fixture must be listable");
    }
    repository
        .record_source_recovery_failure(
            PORTAL_SUBDOMAIN,
            "connector recipe loading",
            &AppError::DataParsing("recipe bytes are invalid".to_string()),
        )
        .await
        .expect("recovery failure must persist");

    let saved = repository
        .list_portal_registrations()
        .await
        .expect("saved sources must load");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].health_status, "degraded");
    let last_error = saved[0]
        .last_error
        .as_deref()
        .expect("durable error must be exposed");
    assert!(last_error.contains("connector recipe loading"));
    assert!(last_error.contains("recipe bytes are invalid"));
}

/// A run the operator stopped ends `CANCELLED`, not `INTERRUPTED`, yet its frontier is just as
/// resumable: the reason a run stopped says nothing about whether its queued work survives.
/// Before this was fixed, adoption matched `INTERRUPTED` alone, so a stopped source's frontier
/// rows were never adopted and never deleted. They kept the source in the startup recovery
/// list forever while every relaunch re-crawled from page one.
#[tokio::test]
async fn a_cancelled_runs_frontier_is_adopted_by_the_run_that_replaces_it() {
    let repository = repository_with_document().await;
    repository
        .begin_portal_sync_run("run-before-stop", PORTAL_SUBDOMAIN)
        .await
        .expect("first sync run should begin");
    for target in ["queued-at-stop", "leased-at-stop"] {
        repository
            .persist_ingestion_frontier_task(
                "run-before-stop",
                PORTAL_SUBDOMAIN,
                target,
                r#"{"taskKind":"fetchMetadataPage","subdomain":"test-portal","page":1,"perPage":50}"#,
            )
            .await
            .expect("stop fixture should persist");
    }
    repository
        .claim_ingestion_frontier_task("run-before-stop", PORTAL_SUBDOMAIN)
        .await
        .expect("frontier lease should succeed")
        .expect("first task should be leased");
    repository
        .finish_portal_sync_run(PortalSyncRunFinalization {
            run_id: "run-before-stop",
            portal_key: PORTAL_SUBDOMAIN,
            status: "CANCELLED",
            pages_processed: 0,
            requests_processed: 0,
            documents_processed: 0,
            failures: 0,
        })
        .await
        .expect("an operator stop should finalize the run");

    repository
        .begin_portal_sync_run("run-after-resume", PORTAL_SUBDOMAIN)
        .await
        .expect("replacement sync run should begin");
    assert_eq!(
        repository
            .adopt_orphaned_frontier_tasks(PORTAL_SUBDOMAIN, "run-after-resume")
            .await
            .expect("stopped tasks should be adopted"),
        2,
        "a resumed source must inherit the frontier its stopped run left behind"
    );
    for _ in 0..2 {
        let task = repository
            .claim_ingestion_frontier_task("run-after-resume", PORTAL_SUBDOMAIN)
            .await
            .expect("adopted task claim should succeed")
            .expect("both stopped tasks must be recoverable");
        repository
            .complete_ingestion_frontier_task(task.frontier_id)
            .await
            .expect("adopted task should acknowledge");
    }
}

/// Pins the upgrade path and the guard it exists to support, together.
///
/// The lifecycle marker has to be durable, because before it existed a stopped source was
/// only stopped in memory: the next launch saw its frontier rows, read them as an interrupted
/// crawl, and resumed a source the operator had deliberately halted. Nothing on a fresh
/// database can catch that, so this builds the pre-lifecycle `portals` shape directly and
/// upgrades it, the way an existing installation does.
#[tokio::test]
async fn a_halted_source_survives_the_upgrade_and_is_not_resumed_at_startup() {
    const LEGACY_PORTALS: &str = "CREATE TABLE portals (
        subdomain TEXT PRIMARY KEY,
        display_name TEXT NOT NULL,
        base_url TEXT NOT NULL,
        origin_url TEXT NOT NULL DEFAULT '',
        requests_endpoint TEXT NOT NULL DEFAULT '',
        requests_pointer TEXT NOT NULL DEFAULT '/requests',
        metadata_pointer TEXT,
        page_parameter TEXT NOT NULL DEFAULT 'page',
        page_size_parameter TEXT NOT NULL DEFAULT 'per_page',
        verified_at TEXT,
        health_status TEXT NOT NULL DEFAULT 'verified',
        last_error TEXT,
        last_synced_at TEXT,
        source_kind TEXT NOT NULL DEFAULT 'NEXT_REQUEST',
        created_at TEXT DEFAULT strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
    );";

    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    {
        let connection = repository.conn.lock().await;
        connection
            .execute_batch(LEGACY_PORTALS)
            .expect("the pre-lifecycle portal shape must be creatable");
        connection
            .execute(
                "INSERT INTO portals (subdomain, display_name, base_url, requests_endpoint)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    PORTAL_SUBDOMAIN,
                    "Test Portal",
                    "https://example.test",
                    "/api/requests",
                ],
            )
            .expect("a source registered before the upgrade must insert");
    }
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("the upgrade path must add the lifecycle column to a populated table");

    assert_eq!(
        repository
            .source_lifecycle_state(PORTAL_SUBDOMAIN)
            .await
            .expect("lifecycle lookup must succeed"),
        Some(SourceLifecycleState::Active),
        "a source that predates the column was never halted, and must read as active"
    );

    repository
        .begin_portal_sync_run("run-before-pause", PORTAL_SUBDOMAIN)
        .await
        .expect("sync run should begin");
    repository
        .persist_ingestion_frontier_task(
            "run-before-pause",
            PORTAL_SUBDOMAIN,
            "queued-at-pause",
            r#"{"taskKind":"fetchMetadataPage","subdomain":"test-portal","page":1,"perPage":50}"#,
        )
        .await
        .expect("frontier fixture should persist");

    assert_eq!(
        repository
            .recover_interrupted_source_keys()
            .await
            .expect("startup recovery scan must succeed"),
        vec![PORTAL_SUBDOMAIN.to_string()],
        "an active source with queued work is exactly what startup recovery is for"
    );

    repository
        .set_source_lifecycle_state(PORTAL_SUBDOMAIN, SourceLifecycleState::Paused)
        .await
        .expect("pausing must record");
    assert!(
        repository
            .recover_interrupted_source_keys()
            .await
            .expect("startup recovery scan must succeed")
            .is_empty(),
        "a paused source must not be resumed at startup by the frontier it deliberately kept"
    );
    assert_eq!(
        repository
            .list_portal_registrations()
            .await
            .expect("listing must succeed")
            .len(),
        1,
        "a paused source is still the operator's source and stays listed"
    );

    repository
        .set_source_lifecycle_state(PORTAL_SUBDOMAIN, SourceLifecycleState::Removed)
        .await
        .expect("withdrawal must record");
    assert!(
        repository
            .list_portal_registrations()
            .await
            .expect("listing must succeed")
            .is_empty(),
        "a withdrawn source leaves the listing"
    );
    assert!(
        repository
            .recover_interrupted_source_keys()
            .await
            .expect("startup recovery scan must succeed")
            .is_empty(),
        "a withdrawn source must not be resurrected by startup recovery"
    );
}
