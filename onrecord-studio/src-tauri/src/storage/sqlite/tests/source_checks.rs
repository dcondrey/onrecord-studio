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
            },
            SourceCheckOutcome::SameBytes,
        ),
        (
            SourceCheckObservation::Received {
                sha256: different.clone(),
                byte_count: 12,
                acquisition: test_acquisition(),
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
        assert_eq!(outcome, expected);
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
