use super::*;
use crate::storage::sqlite::{ReleaseEvent, RELEASED};

fn release(released_at: &str, files: &[&str]) -> ReleaseEvent {
    ReleaseEvent {
        timeline_event_id: None,
        event_name: RELEASED.to_string(),
        released_at: released_at.to_string(),
        listed_file_names: files.iter().map(|name| (*name).to_string()).collect(),
        source_byline: "June 19, 2022,  9:45am by Staff".to_string(),
    }
}

/// A core written before release events existed, at the library's default storage version and
/// without the table, gains it on open with its constraints intact, and a re-crawl replaces a
/// request's events rather than appending to them.
#[tokio::test]
async fn a_core_written_before_release_events_gains_the_table_on_open() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("core.duckdb");
    {
        let legacy = duckdb::Connection::open(&path).unwrap();
        legacy.execute_batch(MIGRATION).unwrap();
        legacy
            .execute_batch("DROP TABLE request_release_events;")
            .unwrap();
    }
    let repository = file_repository_with_document(&path).await;
    for _ in 0..2 {
        repository
            .replace_release_events(
                PORTAL_SUBDOMAIN,
                REQUEST_ID as u64,
                vec![
                    release("2022-06-19T09:45:00-07:00", &["a.pdf", "b.pdf"]),
                    release("2022-07-10T11:16:00-07:00", &[]),
                ],
            )
            .await
            .unwrap();
    }
    let histories = repository.request_release_histories(1, 10).await.unwrap();
    assert_eq!(histories.len(), 1);
    let counts: Vec<Option<u32>> = histories[0]
        .releases
        .iter()
        .map(|release| release.listed_document_count)
        .collect();
    assert_eq!(counts, vec![Some(2), None]);

    let connection = repository.conn.lock().await;
    let zero_listed = connection.execute(
        "INSERT INTO request_release_events (portal_subdomain, request_id, event_ordinal,
            event_name, released_at, listed_document_count, listed_file_names, source_byline)
         VALUES (?1, ?2, 9, 'Document(s) Released', 't', 0, '', 'b')",
        params![PORTAL_SUBDOMAIN, REQUEST_ID],
    );
    assert!(zero_listed.is_err(), "a listed count of zero is refused");
}

/// The relation to `requests` is logical, so the write path refuses events for a request that is
/// not stored instead of leaving an orphan the startup audit would refuse to open.
#[tokio::test]
async fn release_events_for_an_unstored_request_are_refused() {
    let repository = repository_with_document().await;
    let refused = repository
        .replace_release_events(
            PORTAL_SUBDOMAIN,
            (REQUEST_ID + 1) as u64,
            vec![release("2022-06-19T09:45:00-07:00", &["a.pdf"])],
        )
        .await;
    assert!(refused.is_err());
    let connection = repository.conn.lock().await;
    verify_logical_relations(&connection).unwrap();
}
