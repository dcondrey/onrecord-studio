use super::*;

async fn setup() -> DuckDbRepository {
    let db = DuckDbRepository::new(":memory:").unwrap();
    db.run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
        .await
        .unwrap();
    db.initialize_monitoring().await.unwrap();
    db.initialize_watchlists().await.unwrap();
    let conn = db.analytics_connection().await.unwrap();
    conn.execute_batch("INSERT INTO portals(subdomain,display_name,base_url,lifecycle_state) VALUES ('city','City of San Diego','https://www.sandiego.gov','ACTIVE'); INSERT INTO agencies(id,portal_subdomain,name) VALUES (7,'city','Housing Commission'); INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at,agency_id) VALUES (42,'city','Shelter records','closed','2026-09-26','2026-09-26',7)").unwrap();
    db
}

async fn add_document(db: &DuckDbRepository, id: i64, text: &str) {
    let conn = db.analytics_connection().await.unwrap();
    conn.execute("INSERT INTO documents(id,request_id,portal_subdomain,file_name,download_url,visibility,created_at,updated_at,raw_file_sha256,extracted_text_projection_sha256,extraction_status) VALUES (?1,42,'city',?2,?3,'public','2026-09-26','2026-09-26',?4,?5,'SEARCHABLE')",
        params![id, format!("doc{id}.pdf"), format!("https://www.sandiego.gov/doc{id}.pdf"), "a".repeat(64), crate::integrity::sha256_hex(text.as_bytes())]).unwrap();
    conn.execute("INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) VALUES (?1,'city',?2,?3)", params![id, format!("doc{id}.pdf"), text]).unwrap();
}

fn watch(kind: WatchKind, target: &str) -> Watch {
    Watch {
        id: String::new(),
        kind,
        target: target.into(),
        label: String::new(),
        created_at: String::new(),
        enabled: true,
    }
}

fn dimension(check: &WatchCheck, which: WatchDimension) -> &DimensionResult {
    check
        .dimensions
        .iter()
        .find(|d| d.dimension == which)
        .unwrap()
}

#[tokio::test]
async fn first_check_is_a_baseline_then_a_new_document_is_the_only_change() {
    let db = setup().await;
    add_document(&db, 1, "Shelter contract").await;
    let saved = db
        .save_watch(watch(WatchKind::Agency, "housing commission"))
        .await
        .unwrap();

    let first = db
        .check_watches_at(None, "2026-10-01T00:00:00Z".into())
        .await
        .unwrap();
    assert_eq!(first[0].status, WatchCheckStatus::Baseline);
    assert!(first[0].new_changes.is_empty());
    assert_eq!(
        dimension(&first[0], WatchDimension::Documents).outcome,
        DimensionOutcome::BaselineRecorded
    );

    add_document(&db, 2, "Shelter amendment").await;
    let second = db
        .check_watches_at(Some(saved.id.clone()), "2026-10-02T00:00:00Z".into())
        .await
        .unwrap();
    assert_eq!(second[0].status, WatchCheckStatus::Compared);
    let changes = &second[0].new_changes;
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].change_kind, "DOCUMENT_ADDED");
    assert_eq!(changes[0].new_value.as_deref(), Some("doc2.pdf"));
    assert_eq!(
        changes[0].old_as_of.as_deref(),
        Some("2026-10-01T00:00:00Z")
    );
    assert_eq!(changes[0].new_as_of, "2026-10-02T00:00:00Z");
}

#[tokio::test]
async fn rechecking_records_nothing_twice_and_reports_unchanged_not_unmeasured() {
    let db = setup().await;
    add_document(&db, 1, "Shelter contract").await;
    db.save_watch(watch(WatchKind::Agency, "Housing Commission"))
        .await
        .unwrap();
    db.check_watches_at(None, "2026-10-01T00:00:00Z".into())
        .await
        .unwrap();
    add_document(&db, 2, "Shelter amendment").await;
    let first = db
        .check_watches_at(None, "2026-10-02T00:00:00Z".into())
        .await
        .unwrap();
    assert_eq!(first[0].new_changes.len(), 1);
    let again = db
        .check_watches_at(None, "2026-10-02T00:05:00Z".into())
        .await
        .unwrap();
    assert!(again[0].new_changes.is_empty());
    assert_eq!(
        dimension(&again[0], WatchDimension::Documents).outcome,
        DimensionOutcome::Unchanged
    );
    assert_eq!(db.list_watch_changes(None, 100).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_failed_check_leaves_the_baseline_so_the_next_one_compares_with_the_last_observation() {
    let db = setup().await;
    add_document(&db, 1, "Shelter contract").await;
    let saved = db
        .save_watch(watch(WatchKind::Entity, "Shelter"))
        .await
        .unwrap();
    db.check_watches_at(None, "2026-10-01T00:00:00Z".into())
        .await
        .unwrap();

    // Break the scope query the way a damaged or partial database would.
    let conn = db.analytics_connection().await.unwrap();
    conn.execute_batch("ALTER TABLE documents_fts RENAME TO documents_fts_away")
        .unwrap();
    let broken = db
        .check_watches_at(Some(saved.id.clone()), "2026-10-02T00:00:00Z".into())
        .await;
    assert!(
        broken.is_err(),
        "an unreadable corpus must fail the check, not report no change"
    );
    conn.execute_batch("ALTER TABLE documents_fts_away RENAME TO documents_fts")
        .unwrap();

    add_document(&db, 2, "Shelter amendment").await;
    let later = db
        .check_watches_at(Some(saved.id), "2026-10-03T00:00:00Z".into())
        .await
        .unwrap();
    assert_eq!(later[0].new_changes.len(), 1);
    assert_eq!(
        later[0].new_changes[0].old_as_of.as_deref(),
        Some("2026-10-01T00:00:00Z")
    );
}

#[tokio::test]
async fn dimensions_without_data_are_not_measured_and_never_unchanged() {
    let db = setup().await;
    add_document(&db, 1, "Shelter contract").await;
    db.save_watch(watch(WatchKind::Source, "city"))
        .await
        .unwrap();
    let check = db
        .check_watches_at(None, "2026-10-01T00:00:00Z".into())
        .await
        .unwrap();
    let check = &check[0];
    // A captured document with no figure statements is an observation of zero figures.
    assert_eq!(
        dimension(check, WatchDimension::Figures).outcome,
        DimensionOutcome::BaselineRecorded
    );
    for which in [WatchDimension::Withheld, WatchDimension::SourceHealth] {
        let d = dimension(check, which);
        assert_eq!(d.outcome, DimensionOutcome::NotMeasured, "{which:?}");
        assert!(d.reason.is_some());
    }
    let again = db
        .check_watches_at(None, "2026-10-02T00:00:00Z".into())
        .await
        .unwrap();
    assert_eq!(
        dimension(&again[0], WatchDimension::SourceHealth).outcome,
        DimensionOutcome::NotMeasured
    );
}

#[tokio::test]
async fn a_failed_sync_after_a_clean_one_reports_the_source_stopped_responding() {
    let db = setup().await;
    add_document(&db, 1, "Shelter contract").await;
    let conn = db.analytics_connection().await.unwrap();
    conn.execute_batch("INSERT INTO portal_sync_runs(run_id,portal_subdomain,started_at,completed_at,status) VALUES ('r1','city','2026-09-30T00:00:00Z','2026-09-30T00:10:00Z','COMPLETED')").unwrap();
    db.save_watch(watch(WatchKind::Source, "city"))
        .await
        .unwrap();
    let first = db
        .check_watches_at(None, "2026-10-01T00:00:00Z".into())
        .await
        .unwrap();
    assert_eq!(
        dimension(&first[0], WatchDimension::SourceHealth).outcome,
        DimensionOutcome::BaselineRecorded
    );
    let quiet = db
        .check_watches_at(None, "2026-10-01T06:00:00Z".into())
        .await
        .unwrap();
    assert_eq!(
        dimension(&quiet[0], WatchDimension::SourceHealth).outcome,
        DimensionOutcome::NotMeasured
    );

    conn.execute_batch("INSERT INTO portal_sync_runs(run_id,portal_subdomain,started_at,completed_at,status,failures) VALUES ('r2','city','2026-10-02T00:00:00Z','2026-10-02T00:10:00Z','COMPLETED_WITH_ERRORS',4)").unwrap();
    let after = db
        .check_watches_at(None, "2026-10-03T00:00:00Z".into())
        .await
        .unwrap();
    let change = after[0]
        .new_changes
        .iter()
        .find(|c| c.dimension == WatchDimension::SourceHealth)
        .unwrap();
    assert_eq!(change.change_kind, "SOURCE_STOPPED_RESPONDING");
    assert_eq!(change.old_as_of.as_deref(), Some("2026-09-30T00:10:00Z"));
}

#[tokio::test]
async fn a_restated_figure_in_a_recaptured_document_reports_old_and_new_with_capture_times() {
    let db = setup().await;
    add_document(&db, 1, "The program budget is $4 million for shelter beds.").await;
    db.save_watch(watch(WatchKind::Agency, "Housing Commission"))
        .await
        .unwrap();
    let first = db
        .check_watches_at(None, "2026-10-01T00:00:00Z".into())
        .await
        .unwrap();
    assert_eq!(
        dimension(&first[0], WatchDimension::Figures).outcome,
        DimensionOutcome::BaselineRecorded
    );

    let conn = db.analytics_connection().await.unwrap();
    conn.execute_batch("UPDATE documents SET raw_file_sha256=repeat('b',64), extracted_text_projection_sha256='changed' WHERE id=1; UPDATE documents_fts SET extracted_text='The program budget is $6 million for shelter beds.' WHERE document_id=1").unwrap();
    let second = db
        .check_watches_at(None, "2026-10-02T00:00:00Z".into())
        .await
        .unwrap();
    let change = second[0]
        .new_changes
        .iter()
        .find(|c| c.dimension == WatchDimension::Figures)
        .expect("a figure change");
    assert_eq!(change.change_kind, "FIGURE_CHANGED");
    assert!(change.old_value.as_deref().unwrap().contains("$4 million"));
    assert!(change.new_value.as_deref().unwrap().contains("$6 million"));
    assert!(change.old_as_of.is_some());
    assert!(change.new_as_of > change.old_as_of.clone().unwrap());

    let again = db
        .check_watches_at(None, "2026-10-02T00:05:00Z".into())
        .await
        .unwrap();
    assert!(again[0].new_changes.is_empty());
    assert_eq!(
        dimension(&again[0], WatchDimension::Figures).outcome,
        DimensionOutcome::Unchanged
    );
}

#[tokio::test]
async fn a_dismissed_finding_is_not_resolved_but_a_cleared_one_is() {
    let db = setup().await;
    add_document(&db, 1, "Shelter contract").await;
    let conn = db.analytics_connection().await.unwrap();
    conn.execute_batch("INSERT INTO findings(id,kind,severity,headline,explanation,discovered_at,last_seen_at,source_detector,subject_portal_key,subject_document_id) VALUES ('f1','ABSENCE_OF_RECORDS','NOTABLE','Gap in numbering','x','2026-09-30','2026-09-30','d','city',1)").unwrap();
    db.save_watch(watch(WatchKind::Source, "city"))
        .await
        .unwrap();
    db.check_watches_at(None, "2026-10-01T00:00:00Z".into())
        .await
        .unwrap();

    conn.execute_batch("UPDATE findings SET dismissed_at='2026-10-01' WHERE id='f1'")
        .unwrap();
    let dismissed = db
        .check_watches_at(None, "2026-10-02T00:00:00Z".into())
        .await
        .unwrap();
    assert!(dismissed[0].new_changes.is_empty());

    conn.execute_batch("DELETE FROM findings WHERE id='f1'")
        .unwrap();
    let cleared = db
        .check_watches_at(None, "2026-10-03T00:00:00Z".into())
        .await
        .unwrap();
    assert_eq!(cleared[0].new_changes.len(), 1);
    assert_eq!(cleared[0].new_changes[0].change_kind, "FINDING_RESOLVED");
}

#[tokio::test]
async fn a_database_written_before_watchlists_gains_them_without_touching_legacy_rows() {
    let db = DuckDbRepository::new(":memory:").unwrap();
    let conn = db.analytics_connection().await.unwrap();
    conn.execute_batch("CREATE TABLE documents(id BIGINT, title TEXT); INSERT INTO documents VALUES(9,'Existing public record'); CREATE TABLE monitor_heads(source_url TEXT PRIMARY KEY, source_sha256 TEXT NOT NULL, extraction_sha256 TEXT NOT NULL, observed_at TEXT NOT NULL, facts_zstd BLOB NOT NULL)").unwrap();
    assert!(!table_exists(&conn, "watches").unwrap());
    purge_watchlist_references(&conn, "legacy", None).unwrap();
    db.initialize_watchlists().await.unwrap();
    db.initialize_watchlists().await.unwrap();
    assert!(table_exists(&conn, "watches").unwrap());
    assert_eq!(
        conn.query_row("SELECT title FROM documents WHERE id=9", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "Existing public record"
    );
    assert!(db.list_watches().await.unwrap().is_empty());
}

#[tokio::test]
async fn purging_a_source_removes_its_baseline_entries_and_changes_without_reporting_a_withdrawal()
{
    let db = setup().await;
    add_document(&db, 1, "Shelter contract").await;
    db.save_watch(watch(WatchKind::Agency, "Housing Commission"))
        .await
        .unwrap();
    db.check_watches_at(None, "2026-10-01T00:00:00Z".into())
        .await
        .unwrap();
    add_document(&db, 2, "Shelter amendment").await;
    db.check_watches_at(None, "2026-10-02T00:00:00Z".into())
        .await
        .unwrap();
    let conn = db.analytics_connection().await.unwrap();
    purge_watchlist_references(&conn, "city", None).unwrap();
    conn.execute_batch("DELETE FROM documents; DELETE FROM documents_fts")
        .unwrap();
    assert!(db.list_watch_changes(None, 10).await.unwrap().is_empty());
    let after = db
        .check_watches_at(None, "2026-10-03T00:00:00Z".into())
        .await
        .unwrap();
    assert!(after[0].new_changes.is_empty());
    assert_eq!(
        dimension(&after[0], WatchDimension::Documents).outcome,
        DimensionOutcome::Unchanged
    );
}

#[tokio::test]
async fn purging_a_document_keeps_the_other_records_from_reading_as_new() {
    let db = setup().await;
    add_document(&db, 1, "Shelter contract").await;
    add_document(&db, 2, "Shelter amendment").await;
    db.save_watch(watch(WatchKind::Agency, "Housing Commission"))
        .await
        .unwrap();
    db.check_watches_at(None, "2026-10-01T00:00:00Z".into())
        .await
        .unwrap();
    let conn = db.analytics_connection().await.unwrap();
    purge_watchlist_references(&conn, "city", Some(2)).unwrap();
    conn.execute_batch(
        "DELETE FROM documents WHERE id=2; DELETE FROM documents_fts WHERE document_id=2",
    )
    .unwrap();
    let after = db
        .check_watches_at(None, "2026-10-02T00:00:00Z".into())
        .await
        .unwrap();
    assert!(after[0].new_changes.is_empty());
    assert_eq!(
        dimension(&after[0], WatchDimension::Documents).outcome,
        DimensionOutcome::Unchanged
    );
}

#[tokio::test]
async fn saving_validates_targets_and_refuses_duplicates() {
    let db = setup().await;
    assert!(db
        .save_watch(watch(WatchKind::TopicPack, "nope"))
        .await
        .is_err());
    assert!(db
        .save_watch(watch(WatchKind::Source, "missing"))
        .await
        .is_err());
    assert!(db.save_watch(watch(WatchKind::Query, "a")).await.is_err());
    let first = db
        .save_watch(watch(WatchKind::Query, "Sweep  Camp"))
        .await
        .unwrap();
    assert_eq!(first.target, "sweep camp");
    assert!(db
        .save_watch(watch(WatchKind::Query, "sweep camp"))
        .await
        .is_err());
    let mut renamed = first.clone();
    renamed.label = "Sweeps".into();
    renamed.enabled = false;
    assert_eq!(db.save_watch(renamed).await.unwrap().label, "Sweeps");
    let mut retargeted = first;
    retargeted.target = "other".into();
    assert!(db.save_watch(retargeted).await.is_err());
    assert!(db
        .check_watches_at(None, "2026-10-01T00:00:00Z".into())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_watch_deleted_during_a_check_is_not_recreated_by_it() {
    let db = setup().await;
    add_document(&db, 1, "Shelter contract").await;
    let saved = db
        .save_watch(watch(WatchKind::Entity, "Shelter"))
        .await
        .unwrap();
    // The check read the watch, then the operator removed it before the check's transaction ran.
    db.delete_watch(saved.id.clone()).await.unwrap();
    let stale = saved.clone();
    let outcome = db
        .query_connection("stale check", move |conn| {
            check_one(conn, &stale, Prefetch::default(), "2026-10-01T00:00:00Z")
        })
        .await
        .unwrap();
    assert!(outcome.is_none());
    let conn = db.analytics_connection().await.unwrap();
    for table in ["watch_baselines", "watch_runs", "watch_changes"] {
        let rows: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table} kept rows for a deleted watch");
    }
}

#[tokio::test]
async fn changes_with_no_owning_watch_are_not_listed() {
    let db = setup().await;
    let conn = db.analytics_connection().await.unwrap();
    conn.execute_batch("INSERT INTO watch_changes VALUES ('c1','gone','DOCUMENTS','2026-10-01T00:00:00Z',NULL,'{}')").unwrap();
    assert!(db.list_watch_changes(None, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_agency_watch_over_the_document_cap_does_not_report_findings_as_resolved() {
    let db = setup().await;
    for id in 1..=MAX_SCOPE_DOCUMENTS as i64 {
        add_document(&db, id, "Shelter contract").await;
    }
    let conn = db.analytics_connection().await.unwrap();
    conn.execute_batch("INSERT INTO findings(id,kind,severity,headline,explanation,discovered_at,last_seen_at,source_detector,subject_portal_key,subject_document_id) VALUES ('f1','ABSENCE_OF_RECORDS','NOTABLE','Gap in numbering','x','2026-09-30','2026-09-30','d','city',1)").unwrap();
    db.save_watch(watch(WatchKind::Agency, "Housing Commission"))
        .await
        .unwrap();
    // At the cap the scope is complete and the finding is reachable through its document.
    let at_cap = db
        .check_watches_at(None, "2026-10-01T00:00:00Z".into())
        .await
        .unwrap();
    assert_eq!(
        dimension(&at_cap[0], WatchDimension::Findings).outcome,
        DimensionOutcome::BaselineRecorded
    );
    // One document over the cap: the scope is unknown, so nothing is compared.
    add_document(&db, MAX_SCOPE_DOCUMENTS as i64 + 1, "Shelter contract").await;
    let over = db
        .check_watches_at(None, "2026-10-02T00:00:00Z".into())
        .await
        .unwrap();
    assert!(over[0].new_changes.is_empty());
    assert_eq!(
        dimension(&over[0], WatchDimension::Findings).outcome,
        DimensionOutcome::NotMeasured
    );
}
