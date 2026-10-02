use super::*;

#[tokio::test]
async fn legal_authority_versions_are_listed_and_loaded_without_losing_text() {
    let repository = DuckDbRepository::new(":memory:").expect("DuckDB core must open");
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("migration must run");
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO legal_statutes (
                statute_id, jurisdiction, statute_citation, title, authority_kind, full_text,
                current_version_id, content_sha256, imported_at, updated_at
             ) VALUES ('policy-1', 'Test', 'Policy 1', 'Test policy', 'AGENCY_POLICY',
                'Second text', 'version-2', ?1, '2026-02-01T00:00:00Z', '2026-02-01T00:00:00Z')",
                params!["b".repeat(64)],
            )
            .expect("authority must insert");
        connection.execute(
            "INSERT INTO legal_statute_versions (
                version_id, statute_id, full_text, content_sha256, effective_from, imported_at
             ) VALUES ('version-1', 'policy-1', 'First text', ?1, '2026-01-01', '2026-01-01T00:00:00Z'),
                      ('version-2', 'policy-1', 'Second text', ?2, '2026-02-01', '2026-02-01T00:00:00Z')",
            params!["a".repeat(64), "b".repeat(64)],
        ).expect("versions must insert");
    }
    let versions = repository
        .list_legal_authority_versions("policy-1")
        .await
        .unwrap();
    assert_eq!(
        versions
            .iter()
            .map(|version| version.version_id.as_str())
            .collect::<Vec<_>>(),
        vec!["version-2", "version-1"]
    );
    let first = repository
        .get_legal_authority_version("policy-1", "version-1")
        .await
        .unwrap();
    assert_eq!(first.full_text, "First text");
    assert_eq!(first.version.effective_from.as_deref(), Some("2026-01-01"));
}
