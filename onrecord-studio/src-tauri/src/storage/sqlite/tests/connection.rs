use super::*;

/// Analytics nodes are constructed once at startup, before any document is ingested. A node
/// that opens or `ATTACH`es the core database file gets a snapshot frozen at that moment and
/// never observes a later write, which silently reported empty analytics for the whole
/// session. Both nodes must therefore read through a live sibling connection.
#[tokio::test]
async fn analytics_nodes_observe_writes_made_after_they_were_bound() {
    use crate::analysis::AnalyticsEngine;
    use crate::storage::duckdb::DuckDbWorkspace;
    use crate::types::AnalysisConfig;

    // File-backed on purpose: the frozen-snapshot failure this pins only occurs for a
    // database that lives on disk, which is what production uses.
    let directory = tempfile::tempdir().expect("temp dir must be created");
    let repository = file_repository_with_document(&directory.path().join("core.db")).await;

    // Bind both nodes first, exactly as `configure_onrecord_studio_runtime` does at startup.
    let agency_engine = AnalyticsEngine::bind_to_core(
        repository
            .analytics_connection()
            .await
            .expect("analytics connection must open"),
    );
    let correlation_engine = DuckDbWorkspace::bind_to_core(
        repository
            .analytics_connection()
            .await
            .expect("correlation connection must open"),
    );

    let config = AnalysisConfig::default();
    assert!(
        agency_engine
            .calculate_agency_outlier_profiles(&config)
            .await
            .expect("profiles must resolve against an unmapped corpus")
            .is_empty(),
        "no agency is mapped yet"
    );

    // Now write, after both nodes were bound.
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO agencies (id, portal_subdomain, name) VALUES (?1, ?2, ?3)",
                params![9_i64, PORTAL_SUBDOMAIN, "Records Division"],
            )
            .expect("agency fixture must insert");
        connection
            .execute(
                "UPDATE requests SET agency_id = ?1 WHERE id = ?2",
                params![9_i64, REQUEST_ID],
            )
            .expect("request must be mapped to the agency");
    }

    let profiles = agency_engine
        .calculate_agency_outlier_profiles(&config)
        .await
        .expect("profiles must resolve after the write");
    assert_eq!(
        profiles.len(),
        1,
        "the agency written after binding must be visible to the analytics node"
    );
    assert_eq!(profiles[0].agency_name, "Records Division");

    // The correlation workspace shares the same liveness requirement; it must at minimum
    // execute against the live catalog rather than reporting a missing attachment.
    correlation_engine
        .correlate_311_with_enforcement_requests(Some(PORTAL_SUBDOMAIN), 1, 10, 48)
        .await
        .expect("correlation must execute against the live core database");
}

/// `CHECKPOINT` only matters on a real file (an in-memory database has no WAL to flush), and
/// the real risk is a WAL flush leaving the live connection unusable afterward — so this opens
/// a file-backed repository, writes through it, checkpoints, and proves the same connection
/// still reads and writes correctly, not just that the call itself returned `Ok`.
#[tokio::test]
async fn checkpoint_flushes_the_wal_and_leaves_the_connection_usable() {
    let directory = tempfile::tempdir().expect("temp dir must be created");
    let path = directory.path().join("checkpoint-test.duckdb");
    let repository = file_repository_with_document(&path).await;

    repository
        .checkpoint()
        .await
        .expect("checkpointing a real file-backed database must succeed");

    let connection = repository.conn.lock().await;
    let document_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .expect("the connection must still serve queries after checkpointing");
    assert_eq!(
        document_count, 1,
        "the pre-checkpoint write must be visible"
    );
    drop(connection);

    assert!(
        path.exists(),
        "the checkpoint must not have replaced or removed the database file"
    );
}
