use super::*;
use crate::storage::sqlite::{SourceCheckObservation, SourceRecheckTarget};
use crate::types::SourceCheckOutcome;

async fn collected_repository() -> (DuckDbRepository, SourceRecheckTarget) {
    let repository = repository_with_document().await;
    repository
        .conn
        .lock()
        .await
        .execute(
            "UPDATE documents SET raw_file_sha256 = ?1, download_status = 'completed',
                download_url = 'https://records.example.gov/doc'
             WHERE id = ?2",
            params![RAW_FILE_SHA256, DOCUMENT_ID],
        )
        .unwrap();
    let (targets, eligible) = repository
        .source_recheck_targets(PORTAL_SUBDOMAIN, 10)
        .await
        .unwrap();
    assert_eq!(eligible, 1);
    (repository, targets.into_iter().next().expect("one target"))
}

/// Each outcome states only what was observed, and a re-fetch never repoints the collected row:
/// citations and extracted text belong to the bytes that were collected.
#[tokio::test]
async fn each_check_records_only_what_was_observed_and_never_repoints_the_collected_row() {
    let (repository, target) = collected_repository().await;
    let different = "2".repeat(64);
    let gone = crate::ingestion::client::AcquisitionRecord {
        http_status: 404,
        ..test_acquisition()
    };
    let observations = [
        (
            SourceCheckObservation::Received {
                sha256: RAW_FILE_SHA256.to_string(),
                byte_count: 10,
                acquisition: test_acquisition(),
                object_path: None,
            },
            SourceCheckOutcome::SameBytes,
        ),
        (
            SourceCheckObservation::Received {
                sha256: different.clone(),
                byte_count: 12,
                acquisition: test_acquisition(),
                object_path: None,
            },
            SourceCheckOutcome::DifferentBytes,
        ),
        (
            SourceCheckObservation::Gone { acquisition: gone },
            SourceCheckOutcome::Gone,
        ),
        (
            SourceCheckObservation::Unreachable {
                detail: "x".repeat(10_000),
            },
            SourceCheckOutcome::Unreachable,
        ),
    ];
    for (observation, expected) in observations {
        let outcome = repository
            .record_source_check("run", PORTAL_SUBDOMAIN, &target, observation)
            .await
            .unwrap();
        assert_eq!(outcome, Some(expected));
    }

    let checks = repository
        .list_source_checks(PORTAL_SUBDOMAIN, Some(DOCUMENT_ID as u64), 10)
        .await
        .unwrap();
    let by_outcome = |outcome| checks.iter().find(|c| c.outcome == outcome).unwrap();
    assert_eq!(by_outcome(SourceCheckOutcome::Gone).http_status, Some(404));
    assert_eq!(by_outcome(SourceCheckOutcome::Gone).observed_sha256, None);
    let unreachable = by_outcome(SourceCheckOutcome::Unreachable);
    assert_eq!(unreachable.http_status, None);
    assert_eq!(unreachable.detail.as_ref().unwrap().chars().count(), 2_048);
    assert_eq!(
        by_outcome(SourceCheckOutcome::DifferentBytes)
            .observed_sha256
            .as_deref(),
        Some(different.as_str())
    );

    let connection = repository.conn.lock().await;
    // Only the two fetches that received bytes join the acquisition history, each recording the
    // collected digest as the one the document held.
    let history: Vec<(String, Option<String>)> = connection
        .prepare(
            "SELECT raw_file_sha256, previous_raw_file_sha256 FROM document_acquisitions
             WHERE document_id = ?1 ORDER BY acquisition_id",
        )
        .unwrap()
        .query_map(params![DOCUMENT_ID], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        history,
        vec![
            (
                RAW_FILE_SHA256.to_string(),
                Some(RAW_FILE_SHA256.to_string())
            ),
            (different, Some(RAW_FILE_SHA256.to_string())),
        ]
    );
    let collected: String = connection
        .query_row(
            "SELECT raw_file_sha256 FROM documents WHERE id = ?1",
            params![DOCUMENT_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(collected, RAW_FILE_SHA256);
}

/// The table's own constraints refuse a row that would overstate what was observed, even from a
/// caller that bypasses the repository.
#[tokio::test]
async fn the_schema_refuses_a_gone_without_404_or_410_and_a_same_bytes_that_differs() {
    let (repository, _) = collected_repository().await;
    let connection = repository.conn.lock().await;
    let insert = |outcome: &str, status: Option<i64>, observed: Option<&str>| {
        connection.execute(
            "INSERT INTO document_source_checks (run_id, portal_subdomain, document_id,
                requested_url, checked_at, outcome, http_status, collected_sha256, observed_sha256)
             VALUES ('run', ?1, ?2, 'u', 't', ?3, ?4, ?5, ?6)",
            params![
                PORTAL_SUBDOMAIN,
                DOCUMENT_ID,
                outcome,
                status,
                RAW_FILE_SHA256,
                observed
            ],
        )
    };
    let other = "3".repeat(64);
    assert!(insert("GONE", Some(500), None).is_err());
    assert!(insert("GONE", None, None).is_err());
    assert!(insert("SAME_BYTES", Some(200), Some(&other)).is_err());
    assert!(insert("DIFFERENT_BYTES", Some(200), Some(RAW_FILE_SHA256)).is_err());
    assert!(insert("UNREACHABLE", None, Some(RAW_FILE_SHA256)).is_err());
    assert!(insert("GONE", Some(410), None).is_ok());
}

#[tokio::test]
async fn capped_runs_rotate_to_the_least_recently_checked_document() {
    let (repository, target) = collected_repository().await;
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility,
                    created_at, updated_at, raw_file_sha256, download_status, download_url)
                 SELECT ?1, request_id, portal_subdomain, 'second.pdf', visibility, created_at,
                    updated_at, raw_file_sha256, download_status, 'https://records.example.gov/two'
                 FROM documents WHERE id = ?2",
                params![DOCUMENT_ID + 1, DOCUMENT_ID],
            )
            .unwrap();
    }
    repository
        .record_source_check(
            "run",
            PORTAL_SUBDOMAIN,
            &target,
            SourceCheckObservation::Unreachable {
                detail: "timeout".to_string(),
            },
        )
        .await
        .unwrap();
    let (next, eligible) = repository
        .source_recheck_targets(PORTAL_SUBDOMAIN, 1)
        .await
        .unwrap();
    assert_eq!(eligible, 2);
    assert_eq!(next[0].document_id, (DOCUMENT_ID + 1) as u64);
}

/// A core written by an earlier release, at the library's default storage version and without
/// the table, gains it on open with its constraints intact.
#[tokio::test]
async fn a_core_written_before_source_checks_gains_the_table_on_open() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("core.duckdb");
    {
        let legacy = duckdb::Connection::open(&path).unwrap();
        legacy.execute_batch(MIGRATION).unwrap();
        legacy
            .execute_batch(
                "DROP INDEX idx_document_source_checks_document;
                 DROP TABLE document_source_checks;
                 DROP SEQUENCE document_source_check_seq;",
            )
            .unwrap();
    }
    let repository = file_repository_with_document(&path).await;
    let connection = repository.conn.lock().await;
    let insert = |status: i64| {
        connection.execute(
            "INSERT INTO document_source_checks (run_id, portal_subdomain, document_id,
                requested_url, checked_at, outcome, http_status, collected_sha256)
             VALUES ('run', ?1, ?2, 'u', 't', 'GONE', ?3, ?4)",
            params![PORTAL_SUBDOMAIN, DOCUMENT_ID, status, RAW_FILE_SHA256],
        )
    };
    assert!(insert(404).is_ok());
    assert!(insert(500).is_err());
}

fn kept(sha256: &str, path: &str) -> SourceCheckObservation {
    SourceCheckObservation::Received {
        sha256: sha256.to_string(),
        byte_count: 12,
        acquisition: test_acquisition(),
        object_path: Some(path.to_string()),
    }
}

async fn add_document(repository: &DuckDbRepository, id: i64, local_path: Option<&str>) {
    repository
        .conn
        .lock()
        .await
        .execute(
            "INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility,
                created_at, updated_at, raw_file_sha256, download_status, download_url, local_path)
             SELECT ?1, request_id, portal_subdomain, 'other.pdf', visibility, created_at,
                updated_at, raw_file_sha256, download_status, download_url, ?3
             FROM documents WHERE id = ?2",
            params![id, DOCUMENT_ID, local_path],
        )
        .unwrap();
}

/// Kept bytes are content-addressed, so one object can be another document's check copy or its
/// collected file. A purge releases an object only once nothing else references it, in either
/// direction.
#[tokio::test]
async fn a_kept_check_copy_is_released_by_purge_only_when_nothing_else_references_it() {
    const SHARED_COPY: &str = "/store/objects/22/shared-copy";
    const OTHER_ORIGINAL: &str = "/store/objects/33/other-original";
    let (repository, target) = collected_repository().await;
    add_document(&repository, DOCUMENT_ID + 1, Some(OTHER_ORIGINAL)).await;
    let other = SourceRecheckTarget {
        document_id: (DOCUMENT_ID + 1) as u64,
        ..target.clone()
    };
    for (target, observation) in [
        (&target, kept(&"2".repeat(64), SHARED_COPY)),
        (&target, kept(&"3".repeat(64), OTHER_ORIGINAL)),
        (&other, kept(&"2".repeat(64), SHARED_COPY)),
    ] {
        let outcome = repository
            .record_source_check("run", PORTAL_SUBDOMAIN, target, observation)
            .await
            .unwrap();
        assert_eq!(outcome, Some(SourceCheckOutcome::DifferentBytes));
    }

    let first = repository
        .purge_document(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64)
        .await
        .unwrap()
        .purge;
    assert!(
        first.additional_local_paths.is_empty(),
        "both copies are still referenced by the other document: {:?}",
        first.additional_local_paths
    );
    let last = repository
        .purge_document(PORTAL_SUBDOMAIN, (DOCUMENT_ID + 1) as u64)
        .await
        .unwrap()
        .purge;
    assert_eq!(last.local_path.as_deref(), Some(OTHER_ORIGINAL));
    assert_eq!(last.additional_local_paths, vec![SHARED_COPY.to_string()]);
    verify_logical_relations(&*repository.conn.lock().await).unwrap();
}

#[tokio::test]
async fn a_source_purge_releases_its_check_copies_and_its_preview_counts_them() {
    const COPY: &str = "/store/objects/22/copy";
    const ORIGINAL: &str = "/store/objects/11/original";
    let (repository, target) = collected_repository().await;
    repository
        .conn
        .lock()
        .await
        .execute(
            "UPDATE documents SET local_path = ?1 WHERE id = ?2",
            params![ORIGINAL, DOCUMENT_ID],
        )
        .unwrap();
    repository
        .record_source_check(
            "run",
            PORTAL_SUBDOMAIN,
            &target,
            kept(&"2".repeat(64), COPY),
        )
        .await
        .unwrap();
    let preview = repository
        .preview_source_purge(PORTAL_SUBDOMAIN)
        .await
        .unwrap();
    let mut released = repository
        .purge_source(PORTAL_SUBDOMAIN)
        .await
        .unwrap()
        .purge
        .local_paths;
    released.sort();
    assert_eq!(released, vec![ORIGINAL.to_string(), COPY.to_string()]);
    assert_eq!(preview.evidence_file_count, released.len() as u64);
}

/// A copy is kept only of bytes that differ, and a document purged while it was being fetched
/// gets no check row: the startup audit refuses to open a core with an orphaned check.
#[tokio::test]
async fn only_different_bytes_are_kept_and_a_purged_document_gets_no_check() {
    let (repository, target) = collected_repository().await;
    let refused = repository
        .record_source_check(
            "run",
            PORTAL_SUBDOMAIN,
            &target,
            kept(RAW_FILE_SHA256, "/store/objects/11/same"),
        )
        .await;
    assert!(refused.is_err());

    repository
        .purge_document(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64)
        .await
        .unwrap();
    let recorded = repository
        .record_source_check(
            "run",
            PORTAL_SUBDOMAIN,
            &target,
            kept(&"2".repeat(64), "/store/objects/22/late"),
        )
        .await
        .unwrap();
    assert_eq!(recorded, None);
    assert!(!repository
        .evidence_object_referenced("/store/objects/22/late")
        .await
        .unwrap());
    let connection = repository.conn.lock().await;
    verify_logical_relations(&connection).unwrap();
    let acquisitions: i64 = connection
        .query_row("SELECT COUNT(*) FROM document_acquisitions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(acquisitions, 0);
}

/// The history lists each fetch once: an acquisition a check made carries that check's id, so
/// the viewer shows it as the check rather than a second, unexplained fetch.
#[tokio::test]
async fn document_history_links_a_checks_fetch_to_the_check() {
    let (repository, target) = collected_repository().await;
    repository
        .record_source_check(
            "run",
            PORTAL_SUBDOMAIN,
            &target,
            kept(&"2".repeat(64), "/store/objects/22/copy"),
        )
        .await
        .unwrap();
    let history = repository
        .document_source_history(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64)
        .await
        .unwrap();
    let check = &history.checks[0];
    assert!(check.observed_copy_retained);
    assert_eq!(history.acquisitions.len(), 1);
    assert_eq!(
        history.acquisitions[0].source_check_id,
        Some(check.check_id)
    );
    assert_eq!(
        check.acquisition_id,
        Some(history.acquisitions[0].acquisition_id)
    );
}

/// A core whose check and recipe tables predate kept copies and schedules opens with the new
/// columns absent-valued: its earlier checks kept nothing, and no source is scheduled.
#[tokio::test]
async fn a_core_before_kept_copies_and_schedules_gains_both_columns_on_open() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("core.duckdb");
    {
        let legacy = duckdb::Connection::open(&path).unwrap();
        legacy.execute_batch(MIGRATION).unwrap();
        legacy
            .execute_batch(
                "DROP INDEX idx_document_source_checks_document;
                 DROP TABLE document_source_checks;
                 CREATE TABLE document_source_checks (
                     check_id BIGINT PRIMARY KEY DEFAULT nextval('document_source_check_seq'),
                     run_id TEXT NOT NULL,
                     portal_subdomain TEXT NOT NULL,
                     document_id BIGINT NOT NULL,
                     requested_url TEXT NOT NULL,
                     checked_at TEXT NOT NULL,
                     outcome TEXT NOT NULL
                         CHECK (outcome IN ('SAME_BYTES', 'DIFFERENT_BYTES', 'GONE', 'UNREACHABLE')),
                     http_status BIGINT CHECK (http_status IS NULL OR http_status BETWEEN 100 AND 599),
                     collected_sha256 TEXT NOT NULL CHECK (length(collected_sha256) = 64),
                     observed_sha256 TEXT CHECK (observed_sha256 IS NULL OR length(observed_sha256) = 64),
                     detail TEXT,
                     CHECK ((outcome IN ('SAME_BYTES', 'DIFFERENT_BYTES')) = (observed_sha256 IS NOT NULL)),
                     CHECK ((outcome = 'SAME_BYTES') = (observed_sha256 IS NOT DISTINCT FROM collected_sha256)),
                     CHECK (outcome <> 'GONE' OR (http_status IS NOT NULL AND http_status IN (404, 410)))
                 );
                 CREATE INDEX idx_document_source_checks_document
                     ON document_source_checks(portal_subdomain, document_id);
                 DROP TABLE source_connector_recipes;
                 CREATE TABLE source_connector_recipes (
                     source_key TEXT PRIMARY KEY,
                     root_url TEXT NOT NULL UNIQUE,
                     submitted_url TEXT,
                     recipe_version BIGINT NOT NULL DEFAULT 2,
                     publisher_pattern TEXT,
                     reuse_approved BIGINT NOT NULL DEFAULT 0 CHECK (reuse_approved IN (0, 1)),
                     crawl_policy_json TEXT NOT NULL,
                     guidance_json TEXT NOT NULL,
                     metadata_page_window_json TEXT,
                     updated_at TEXT NOT NULL DEFAULT strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
                     FOREIGN KEY (source_key) REFERENCES portals(subdomain)
                 );",
            )
            .unwrap();
    }
    let repository = file_repository_with_document(&path).await;
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "UPDATE documents SET raw_file_sha256 = ?1, download_status = 'completed',
                    download_url = 'https://records.example.gov/doc'
                 WHERE id = ?2",
                params![RAW_FILE_SHA256, DOCUMENT_ID],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO document_source_checks (run_id, portal_subdomain, document_id,
                    requested_url, checked_at, outcome, http_status, collected_sha256,
                    observed_sha256)
                 VALUES ('earlier', ?1, ?2, 'u', '2026-10-01T00:00:00Z', 'DIFFERENT_BYTES', 200,
                    ?3, ?4)",
                params![
                    PORTAL_SUBDOMAIN,
                    DOCUMENT_ID,
                    RAW_FILE_SHA256,
                    "2".repeat(64)
                ],
            )
            .unwrap();
    }
    let (targets, _) = repository
        .source_recheck_targets(PORTAL_SUBDOMAIN, 1)
        .await
        .unwrap();
    repository
        .record_source_check(
            "now",
            PORTAL_SUBDOMAIN,
            &targets[0],
            kept(&"3".repeat(64), "/store/objects/33/copy"),
        )
        .await
        .unwrap();
    let checks = repository
        .list_source_checks(PORTAL_SUBDOMAIN, Some(DOCUMENT_ID as u64), 10)
        .await
        .unwrap();
    let by_run = |run: &str| checks.iter().find(|check| check.run_id == run).unwrap();
    assert!(!by_run("earlier").observed_copy_retained);
    assert_eq!(by_run("earlier").acquisition_id, None);
    assert!(by_run("now").observed_copy_retained);

    let registration = test_registration(PORTAL_SUBDOMAIN, RECIPE_ROOT_URL);
    repository
        .insert_or_update_portal_registration(&registration)
        .await
        .unwrap();
    let interval = |sources: Vec<crate::types::SavedSource>| {
        sources
            .into_iter()
            .find(|source| source.registration.portal_key == PORTAL_SUBDOMAIN)
            .unwrap()
            .recheck_interval_days
    };
    assert_eq!(
        interval(repository.list_portal_registrations().await.unwrap()),
        None
    );
    repository
        .save_source_recheck_interval(&registration, Some(7))
        .await
        .unwrap();
    assert_eq!(
        interval(repository.list_portal_registrations().await.unwrap()),
        Some(7)
    );
}

/// The schedule is off until chosen, stays within its range, and setting it leaves the source's
/// saved crawl policy and guidance as they were.
#[tokio::test]
async fn a_check_schedule_is_saved_without_touching_the_sources_recipe() {
    let (repository, registration) = repository_with_registered_portal().await;
    let policy = CrawlPolicy {
        maximum_pages: 7,
        ..CrawlPolicy::default()
    };
    repository
        .save_source_connector_recipe(
            &registration,
            RECIPE_ROOT_URL,
            &policy,
            &SourceGuidance::default(),
        )
        .await
        .unwrap();
    for refused in [0, 366] {
        assert!(repository
            .save_source_recheck_interval(&registration, Some(refused))
            .await
            .is_err());
    }
    repository
        .save_source_recheck_interval(&registration, Some(30))
        .await
        .unwrap();
    let recipe = repository
        .load_source_connector_recipe_for_source(&registration)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recipe.crawl_policy.maximum_pages, 7);
    repository
        .save_source_recheck_interval(&registration, None)
        .await
        .unwrap();
    let saved = repository.list_portal_registrations().await.unwrap();
    assert_eq!(saved[0].recheck_interval_days, None);
}
