use super::*;

/// A document longer than the projection cap is indexed to its prefix, and the state row says
/// how far that reached. The corpus holds 128 MB CSV exports whose tokens are identifiers and
/// coordinates; projecting them in full is millions of index rows for no readable match. A term
/// past the cap must therefore not be reported as absent-from-the-corpus by silence alone: the
/// recorded `indexed_characters` is what distinguishes "not indexed here" from "not present".
#[tokio::test]
async fn oversized_documents_are_indexed_to_a_recorded_prefix() {
    let repository = repository_with_document().await;
    let mut text =
        "alpha ".repeat(super::super::search::SEARCH_TOKEN_MAX_INDEXED_CHARACTERS / 6 + 16);
    text.push_str(" sentinelbeyondcap");
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO documents_fts (
                    document_id, portal_subdomain, file_name, extracted_text
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN, "bulk-export.csv", text],
            )
            .expect("oversized fixture must insert");
    }
    repository
        .backfill_search_token_index()
        .await
        .expect("the drain must project the oversized document");

    let connection = repository.conn.lock().await;
    let indexed_characters: Option<i64> = connection
        .query_row(
            "SELECT indexed_characters FROM document_search_index_state
             WHERE document_id = ?1 AND portal_subdomain = ?2",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            |row| row.get(0),
        )
        .expect("the state row must record the indexed prefix");
    assert_eq!(
        indexed_characters,
        Some(super::super::search::SEARCH_TOKEN_MAX_INDEXED_CHARACTERS as i64),
        "the recorded prefix length is what makes the cap visible to a reader"
    );
}

/// A database written before `indexed_characters` existed keeps its own
/// `document_search_index_state`, and `CREATE TABLE IF NOT EXISTS` leaves it alone: the
/// `ensure_column` shim is the only thing that can complete it. Without the column the first
/// projection write fails, which on an upgraded install is every search. The old shape is
/// built directly here because DuckDB refuses `DROP COLUMN` on a table with dependents.
#[tokio::test]
async fn a_database_without_indexed_characters_is_completed_before_the_projection_writes() {
    let repository = DuckDbRepository::new(Path::new(":memory:")).expect("DuckDB core must open");
    {
        let connection = repository.conn.lock().await;
        connection
            .execute_batch(
                "CREATE TABLE document_search_index_state (
                    document_id BIGINT NOT NULL,
                    portal_subdomain TEXT NOT NULL,
                    index_version BIGINT NOT NULL,
                    token_count BIGINT NOT NULL,
                    PRIMARY KEY (document_id, portal_subdomain)
                 );",
            )
            .expect("the pre-upgrade shape must be reproducible");
    }
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("migration must complete an older index state table");
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO documents_fts (
                    document_id, portal_subdomain, file_name, extracted_text
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN, "upgraded.txt", "alpha beta"],
            )
            .expect("document fixture must insert");
    }

    repository
        .backfill_search_token_index()
        .await
        .expect("the projection must write against the completed table");

    let connection = repository.conn.lock().await;
    let indexed_characters: Option<i64> = connection
        .query_row(
            "SELECT indexed_characters FROM document_search_index_state
             WHERE document_id = ?1 AND portal_subdomain = ?2",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            |row| row.get(0),
        )
        .expect("the state row must exist after the drain");
    assert_eq!(
        indexed_characters,
        Some("alpha beta".chars().count() as i64),
        "the shimmed column must hold the recorded prefix length, not be absent"
    );
}

/// Core search uses the persistent exact-token projection and requires every plain-text term.
/// Boolean words have no special meaning because the typed frontend does not offer a Boolean
/// grammar; qualifiers are passed separately to `search_requests`.
#[tokio::test]
async fn core_search_uses_persistent_exact_tokens_and_all_term_matching() {
    let repository = repository_with_document().await;
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO documents_fts (
                    document_id, portal_subdomain, file_name, extracted_text
                 ) VALUES (?1, ?2, ?3, ?4)",
                params![
                    DOCUMENT_ID,
                    PORTAL_SUBDOMAIN,
                    "release.pdf",
                    "Encampment sweep notice",
                ],
            )
            .expect("search projection fixture must insert");
    }
    repository
        .run_migrations(MIGRATION)
        .await
        .expect("migration must count the legacy projection arrears");
    // Migration counts, it does not tokenize: doing the work here ran inside the Tauri setup
    // hook and kept the window from ever appearing on a large corpus.
    assert_eq!(
        repository.pending_search_index_documents(),
        1,
        "a legacy projection row must be counted as arrears, not silently indexed at startup"
    );
    repository
        .backfill_search_token_index()
        .await
        .expect("the background drain must project the legacy row");
    assert_eq!(repository.pending_search_index_documents(), 0);

    {
        let connection = repository.conn.lock().await;
        connection.execute("UPDATE requests SET title = 'San Diego surveillance' WHERE id = ?1 AND portal_subdomain = ?2", params![REQUEST_ID, PORTAL_SUBDOMAIN]).unwrap();
    }
    assert_eq!(
        repository
            .search_requests(
                "surveillance encampment",
                Some(PORTAL_SUBDOMAIN),
                None,
                None,
                None,
                25
            )
            .await
            .unwrap()
            .len(),
        1,
        "terms can span request metadata and document text"
    );
    assert!(repository
        .search_requests(
            "surveillance absent",
            Some(PORTAL_SUBDOMAIN),
            None,
            None,
            None,
            25
        )
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn passage_search_finds_document_text_and_respects_filters_and_visibility() {
    let repository = repository_with_document().await;
    let text = format!(
        "{}Shelter capacity increased by 120 beds in the adopted budget.",
        "背景 context ".repeat(220)
    );
    {
        let mut connection = repository.conn.lock().await;
        connection.execute("UPDATE portals SET base_url = 'https://www.sandiego.gov', source_kind = 'WEB_CRAWL' WHERE subdomain = ?1", params![PORTAL_SUBDOMAIN]).unwrap();
        connection
            .execute(
                "UPDATE documents SET content_type = 'application/pdf' WHERE id = ?1",
                params![DOCUMENT_ID],
            )
            .unwrap();
        let tx = connection.transaction().unwrap();
        super::super::search::synchronize_search_tokens(&tx, DOCUMENT_ID, PORTAL_SUBDOMAIN, &text)
            .unwrap();
        tx.execute("INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text) VALUES (?1, ?2, 'budget.pdf', ?3)", params![DOCUMENT_ID, PORTAL_SUBDOMAIN, text]).unwrap();
        tx.commit().unwrap();
    }
    let filter: crate::types::HybridSearchFilter = serde_json::from_value(
        serde_json::json!({"jurisdiction":"City of San Diego", "documentType":"application/pdf"}),
    )
    .unwrap();
    let result = repository
        .search_document_passages("shelter capacity", filter.clone(), 10)
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].excerpt.contains("120 beds"));
    assert!(result[0].character_start > 1000);
    assert_eq!(
        result[0].publication_date, None,
        "crawl time must not become publication date"
    );
    assert_eq!(
        text.chars()
            .skip(result[0].character_start)
            .take(result[0].character_end - result[0].character_start)
            .collect::<String>(),
        result[0].excerpt
    );
    let mut excluded = filter.clone();
    excluded.jurisdiction = Some("San Diego County".into());
    assert!(repository
        .search_document_passages("shelter capacity", excluded, 10)
        .await
        .unwrap()
        .is_empty());
    let mut dated = filter.clone();
    dated.date_from = Some("2026-01-01".into());
    assert!(repository
        .search_document_passages("shelter capacity", dated, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(repository
        .search_document_passages("shelter absentword", filter.clone(), 10)
        .await
        .unwrap()
        .is_empty());
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "UPDATE documents SET visibility = 'private' WHERE id = ?1",
                params![DOCUMENT_ID],
            )
            .unwrap();
    }
    assert!(repository
        .search_document_passages("shelter capacity", filter, 10)
        .await
        .unwrap()
        .is_empty());
}

/// Evidence search must prepare and run against DuckDB.
///
/// It did not: the projection used `char(31)` as the separator, which SQLite accepts and
/// DuckDB parses as a type keyword, so `search_requests` failed at prepare time with
/// "Parser Error: syntax error at or near )". Every search in the evidence workspace returned
/// an error banner and no results. Nothing caught it because no test ever called this method.
#[tokio::test]
async fn evidence_search_prepares_and_returns_matching_requests() {
    let repository = repository_with_document().await;
    let results = repository
        .search_requests("", None, None, None, None, 25)
        .await
        .expect("the evidence search must prepare against DuckDB");
    assert!(
        results
            .iter()
            .any(|result| result.request_id == REQUEST_ID as u64),
        "the fixture request must be findable: {results:?}"
    );

    // The separator is what broke, so exercise the path that reads it back apart.
    let filtered = repository
        .search_requests(
            "sweep",
            Some(PORTAL_SUBDOMAIN),
            None,
            Some("closed"),
            None,
            25,
        )
        .await
        .expect("a filtered evidence search must prepare too");
    assert!(filtered.len() <= results.len());
}

/// A document is identified by its portal and id together. Excluding the redacted document by id
/// alone dropped every other portal's document that happened to share the number, and ranking
/// after the `LIMIT` let a weaker hit displace the strongest one.
#[tokio::test]
async fn redaction_context_matches_exclude_only_the_redacted_document_and_rank_before_limiting() {
    const OTHER_PORTAL: &str = "other-portal";
    let repository = repository_with_document().await;
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO portals (subdomain, display_name, base_url) VALUES (?1, ?2, ?3)",
                params![OTHER_PORTAL, "Other Portal", "https://other.example.test"],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO requests (id, portal_subdomain, status, created_at, updated_at)
                 VALUES (?1, ?2, 'open', '2026-08-24T12:00:00Z', '2026-08-24T12:00:00Z')",
                params![REQUEST_ID, OTHER_PORTAL],
            )
            .unwrap();
        for (portal, document_id, text) in [
            (
                PORTAL_SUBDOMAIN,
                DOCUMENT_ID,
                "alpha beta in the redacted release",
            ),
            (OTHER_PORTAL, DOCUMENT_ID + 1, "alpha alone"),
            (OTHER_PORTAL, DOCUMENT_ID, "alpha beta elsewhere"),
        ] {
            if portal == OTHER_PORTAL {
                connection
                    .execute(
                        "INSERT INTO documents (
                            id, request_id, portal_subdomain, file_name, visibility,
                            created_at, updated_at
                         ) VALUES (?1, ?2, ?3, 'other.pdf', 'public',
                            '2026-08-24T12:01:00Z', '2026-08-24T12:01:00Z')",
                        params![document_id, REQUEST_ID, portal],
                    )
                    .unwrap();
            }
            connection
                .execute(
                    "INSERT INTO documents_fts (
                        document_id, portal_subdomain, file_name, extracted_text
                     ) VALUES (?1, ?2, 'release.pdf', ?3)",
                    params![document_id, portal, text],
                )
                .unwrap();
        }
    }

    let matches = repository
        .find_redaction_context_matches(
            PORTAL_SUBDOMAIN,
            DOCUMENT_ID,
            vec!["alpha".to_string(), "beta".to_string()],
            1,
        )
        .await
        .expect("the lookup must succeed");
    let hits = matches
        .iter()
        .map(|(portal, id, _, matched, total)| (portal.as_str(), *id, *matched, *total))
        .collect::<Vec<_>>();
    assert_eq!(hits, vec![(OTHER_PORTAL, DOCUMENT_ID, 2, 2)]);

    // A copy of the redacted document is not independent corroboration.
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO documents (
                    id, request_id, portal_subdomain, file_name, visibility,
                    created_at, updated_at, extracted_text_sha256
                 ) VALUES (?1, ?2, ?3, 'copy.pdf', 'public',
                    '2026-08-24T12:01:00Z', '2026-08-24T12:01:00Z', ?4)",
                params![DOCUMENT_ID + 2, REQUEST_ID, OTHER_PORTAL, RAW_FILE_SHA256],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
                 VALUES (?1, ?2, 'copy.pdf', 'alpha beta in the redacted release')",
                params![DOCUMENT_ID + 2, OTHER_PORTAL],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE documents SET extracted_text_sha256 = ?3
                 WHERE id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN, RAW_FILE_SHA256],
            )
            .unwrap();
    }
    let all = repository
        .find_redaction_context_matches(
            PORTAL_SUBDOMAIN,
            DOCUMENT_ID,
            vec!["alpha".to_string(), "beta".to_string()],
            10,
        )
        .await
        .expect("the lookup must succeed")
        .into_iter()
        .map(|(portal, id, _, _, _)| (portal, id))
        .collect::<Vec<_>>();
    assert_eq!(
        all,
        vec![
            (OTHER_PORTAL.to_string(), DOCUMENT_ID),
            (OTHER_PORTAL.to_string(), DOCUMENT_ID + 1),
        ]
    );
}
