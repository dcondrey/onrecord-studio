use super::*;

/// The purge exists to be complete, and `verify_logical_relations` is the only thing that can
/// prove it: that audit runs at startup and refuses to open a core database containing an
/// orphan, so a table the purge forgets does not degrade the corpus, it makes the next launch
/// fail. Asserting on the audit rather than on a hand-written list of tables means this test
/// keeps working when a relation is added to the schema.
///
/// Proven by deleting the `documents_fts` arm from `purge_document_rows`: this test then fails
/// with "documents_fts -> documents: 1 orphan row(s)".
#[tokio::test]
async fn purging_a_document_leaves_no_orphan_for_the_startup_audit() {
    let repository = repository_with_document().await;
    let extracted_text = "responsive record";
    let extracted_text_sha256 = compute_extracted_text_sha256(extracted_text);
    repository
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: DOCUMENT_ID as u64,
            portal_subdomain: PORTAL_SUBDOMAIN,
            extracted_text,
            extracted_text_sha256: &extracted_text_sha256,
            raw_file_sha256: RAW_FILE_SHA256,
            was_ocr_processed: false,
            raw_file_size: 100,
            local_path: Path::new("/var/data/test-portal/release.pdf"),
            shannon_entropy: Some(4.0),
            entropy_sample_bytes: 100,
            suspicious_data_density: false,
        })
        .await
        .expect("the document must finalize");
    repository
        .record_document_acquisition(
            PORTAL_SUBDOMAIN,
            DOCUMENT_ID as u64,
            None,
            RAW_FILE_SHA256,
            100,
            &test_acquisition(),
        )
        .await
        .expect("the fetch must be recorded");

    let purge = repository
        .purge_document(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64)
        .await
        .expect("a finalized document must be purgeable")
        .purge;
    assert_eq!(
        purge.local_path.as_deref(),
        Some("/var/data/test-portal/release.pdf"),
        "the evidence path must be reported so the caller can unlink it"
    );
    assert!(
        purge.search_rows_removed > 0,
        "a finalized document has search projection rows and the purge must report removing them"
    );

    let connection = repository.conn.lock().await;
    verify_logical_relations(&connection)
        .expect("a purged document must leave no orphan in any audited relation");
    let remaining: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM documents WHERE id = ?1 AND portal_subdomain = ?2",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            |row| row.get(0),
        )
        .expect("the document count must be readable");
    assert_eq!(remaining, 0, "the document row itself must be gone");
}

/// Identical bytes share one content-addressed object, so a purge that released the path of a
/// file another document still references would let the caller unlink that document's
/// evidence.
#[tokio::test]
async fn purging_a_document_keeps_an_object_another_document_references() {
    const SHARED_PATH: &str = "/var/data/objects/ab/shared";
    let repository = repository_with_document().await;
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "UPDATE documents SET local_path = ?1 WHERE id = ?2",
                params![SHARED_PATH, DOCUMENT_ID],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO documents (
                    id, request_id, portal_subdomain, file_name, visibility, created_at,
                    updated_at, local_path
                 ) VALUES (?1, ?2, ?3, 'copy.pdf', 'public', ?4, ?4, ?5)",
                params![
                    DOCUMENT_ID + 1,
                    REQUEST_ID,
                    PORTAL_SUBDOMAIN,
                    "2026-08-24T12:02:00Z",
                    SHARED_PATH
                ],
            )
            .unwrap();
    }

    let first = repository
        .purge_document(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64)
        .await
        .unwrap()
        .purge;
    assert_eq!(first.local_path, None);
    let last = repository
        .purge_document(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64 + 1)
        .await
        .unwrap()
        .purge;
    assert_eq!(last.local_path.as_deref(), Some(SHARED_PATH));
}

/// Withdrawal keeps the `portals` row on purpose, which is why a withdrawn source can never be
/// added back. A purge must not: this asserts the row is actually gone, because that difference
/// is the whole reason the second command exists.
#[tokio::test]
async fn purging_a_source_removes_the_portal_row_and_leaves_no_orphan() {
    let repository = repository_with_document().await;
    let extracted_text = "responsive record";
    let extracted_text_sha256 = compute_extracted_text_sha256(extracted_text);
    repository
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: DOCUMENT_ID as u64,
            portal_subdomain: PORTAL_SUBDOMAIN,
            extracted_text,
            extracted_text_sha256: &extracted_text_sha256,
            raw_file_sha256: RAW_FILE_SHA256,
            was_ocr_processed: false,
            raw_file_size: 100,
            local_path: Path::new("/var/data/test-portal/release.pdf"),
            shannon_entropy: Some(4.0),
            entropy_sample_bytes: 100,
            suspicious_data_density: false,
        })
        .await
        .expect("the document must finalize");

    let purge = repository
        .purge_source(PORTAL_SUBDOMAIN)
        .await
        .expect("a registered source must be purgeable")
        .purge;
    assert_eq!(purge.documents_removed, 1);
    assert_eq!(purge.requests_removed, 1);
    assert_eq!(
        purge.local_paths,
        vec!["/var/data/test-portal/release.pdf".to_string()],
        "every evidence path under the source must be reported for unlinking"
    );

    let connection = repository.conn.lock().await;
    verify_logical_relations(&connection)
        .expect("a purged source must leave no orphan in any audited relation");
    let portals: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM portals WHERE subdomain = ?1",
            params![PORTAL_SUBDOMAIN],
            |row| row.get(0),
        )
        .expect("the portal count must be readable");
    assert_eq!(
        portals, 0,
        "purging must remove the portals row so the same source can be added again"
    );
}

/// The preview and the purge must count the same rows.
///
/// A confirmation dialog that names a count the purge does not honour tells the operator a false
/// thing about the evidence they are about to destroy. Bytes a pending retention receipt staged
/// are evidence files the purge unlinks too; a reclaimed receipt's are already gone.
///
/// The fixture is deliberately asymmetric: 2 documents, 4 requests, 3 evidence files. Against a
/// one-of-each fixture a preview that swapped the document and request predicates, or that
/// returned the same count three times, would still pass.
#[tokio::test]
async fn a_source_purge_preview_counts_what_the_purge_removes() {
    let repository = repository_with_document().await;
    {
        let connection = repository.conn.lock().await;
        for extra_request in [REQUEST_ID + 1, REQUEST_ID + 2, REQUEST_ID + 3] {
            connection
                .execute(
                    "INSERT INTO requests (
                        id, portal_subdomain, status, created_at, updated_at, closed_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        extra_request,
                        PORTAL_SUBDOMAIN,
                        "closed",
                        "2026-08-24T12:00:00Z",
                        "2026-08-24T11:59:00Z",
                        "2026-08-24T12:04:00Z",
                    ],
                )
                .expect("an additional request must insert");
        }
        for (receipt, state) in [
            ("prepared", "PREPARED"),
            ("referenced", "REFERENCED"),
            ("reclaimed", "RECLAIMED"),
        ] {
            connection
                .execute(
                    "INSERT INTO data_retention_receipts VALUES (
                        ?1, ?2, ?3, '/var/data/original', ?4,
                        'https://example.test/doc', ?5, 10, ?6, '2026-08-24T12:05:00Z'
                     )",
                    params![
                        receipt,
                        PORTAL_SUBDOMAIN,
                        DOCUMENT_ID + 1,
                        format!("/var/data/pending/{receipt}"),
                        RAW_FILE_SHA256,
                        state,
                    ],
                )
                .expect("a retention receipt must insert");
        }
        // No `local_path`: a document whose bytes were never retained is still a record the
        // purge destroys, and it must not be counted as an evidence file.
        connection
            .execute(
                "INSERT INTO documents (
                    id, request_id, portal_subdomain, file_name, visibility,
                    created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    DOCUMENT_ID + 1,
                    REQUEST_ID,
                    PORTAL_SUBDOMAIN,
                    "never-downloaded.pdf",
                    "public",
                    "2026-08-24T12:01:00Z",
                    "2026-08-24T12:01:00Z",
                ],
            )
            .expect("an additional document must insert");
    }
    let extracted_text = "responsive record";
    let extracted_text_sha256 = compute_extracted_text_sha256(extracted_text);
    repository
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: DOCUMENT_ID as u64,
            portal_subdomain: PORTAL_SUBDOMAIN,
            extracted_text,
            extracted_text_sha256: &extracted_text_sha256,
            raw_file_sha256: RAW_FILE_SHA256,
            was_ocr_processed: false,
            raw_file_size: 100,
            local_path: Path::new("/var/data/test-portal/release.pdf"),
            shannon_entropy: Some(4.0),
            entropy_sample_bytes: 100,
            suspicious_data_density: false,
        })
        .await
        .expect("the document must finalize");

    let preview = repository
        .preview_source_purge(PORTAL_SUBDOMAIN)
        .await
        .expect("a registered source must be previewable");
    let purge = repository
        .purge_source(PORTAL_SUBDOMAIN)
        .await
        .expect("a registered source must be purgeable")
        .purge;

    assert_eq!(
        preview.document_count, purge.documents_removed,
        "the preview promised a document count the purge did not honour"
    );
    assert_eq!(
        preview.request_count, purge.requests_removed,
        "the preview promised a request count the purge did not honour"
    );
    assert_eq!(
        preview.evidence_file_count,
        purge.local_paths.len() as u64,
        "the preview promised an evidence-file count the purge did not honour"
    );
    assert_eq!(preview.source_key, PORTAL_SUBDOMAIN);
    assert_eq!(
        (
            preview.document_count,
            preview.request_count,
            preview.evidence_file_count
        ),
        (2, 4, 3),
        "the fixture must stay asymmetric, or the three assertions above stop discriminating"
    );
}

#[tokio::test]
async fn guidance_discard_removes_only_unfinalized_documents() {
    let unfinalized = repository_with_document().await;
    assert!(unfinalized
        .delete_unextracted_document(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64)
        .await
        .expect("an unfinalized stub must be removable"));
    {
        let connection = unfinalized.conn.lock().await;
        let remaining: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                |row| row.get(0),
            )
            .expect("document count must decode");
        assert_eq!(remaining, 0);
    }

    let finalized = repository_with_document().await;
    let extracted_text = "responsive record";
    let extracted_text_sha256 = compute_extracted_text_sha256(extracted_text);
    finalized
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: DOCUMENT_ID as u64,
            portal_subdomain: PORTAL_SUBDOMAIN,
            extracted_text,
            extracted_text_sha256: &extracted_text_sha256,
            raw_file_sha256: RAW_FILE_SHA256,
            was_ocr_processed: false,
            raw_file_size: 100,
            local_path: Path::new("/var/data/test-portal/release.pdf"),
            shannon_entropy: Some(4.0),
            entropy_sample_bytes: 100,
            suspicious_data_density: false,
        })
        .await
        .expect("the document must finalize");
    assert!(!finalized
        .delete_unextracted_document(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64)
        .await
        .expect("a finalized document must be retained without error"));
    let connection = finalized.conn.lock().await;
    let counts: (i64, i64) = connection
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM documents WHERE id = ?1 AND portal_subdomain = ?2),
                (SELECT COUNT(*) FROM documents_fts WHERE document_id = ?1 AND portal_subdomain = ?2)",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("document and FTS counts must decode");
    assert_eq!(counts, (1, 1));
}

/// What a purge removes must stop being asserted anywhere else: a detector finding about the
/// purged document, and a request-level density flag only that document carried. A finding about
/// a surviving document stays until its whole source is purged.
#[tokio::test]
async fn purging_withdraws_the_findings_and_density_flags_that_rested_on_what_was_removed() {
    let repository = repository_with_document().await;
    let audit_id = format!("auto-{PORTAL_SUBDOMAIN}-{REQUEST_ID}");
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO documents (
                    id, request_id, portal_subdomain, file_name, visibility, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 'kept.pdf', 'public',
                    '2026-08-24T12:01:00Z', '2026-08-24T12:01:00Z')",
                params![DOCUMENT_ID + 1, REQUEST_ID, PORTAL_SUBDOMAIN],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE documents SET suspicious_data_density = 1
                 WHERE id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO case_audits (
                    audit_id, target_request_id, target_subdomain, auditor_notes,
                    verified_at, verification_status
                 ) VALUES (?1, ?2, ?3, 'density', '2026-08-24T12:05:00Z', 'DataAnomalyFlagged')",
                params![audit_id, REQUEST_ID, PORTAL_SUBDOMAIN],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO document_integrity_flags (audit_id, flag_type)
                 VALUES (?1, 'SUSPICIOUS_DATA_DENSITY')",
                params![audit_id],
            )
            .unwrap();
        for (finding_id, document_id) in [
            ("about-purged", DOCUMENT_ID),
            ("about-kept", DOCUMENT_ID + 1),
        ] {
            connection
                .execute(
                    "INSERT INTO findings (
                        id, kind, severity, subject_portal_key, subject_document_id, headline,
                        explanation, discovered_at, last_seen_at, source_detector
                     ) VALUES (?1, 'test', 'NOTABLE', ?2, ?3, 'headline', 'explanation',
                        '2026-08-24T12:06:00Z', '2026-08-24T12:06:00Z', 'test')",
                    params![finding_id, PORTAL_SUBDOMAIN, document_id],
                )
                .unwrap();
        }
    }
    let state = |repository: DuckDbRepository, audit_id: String| async move {
        let connection = repository.conn.lock().await;
        let findings = connection
            .prepare("SELECT id FROM findings ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<duckdb::Result<Vec<_>>>()
            .unwrap();
        let (flags, status): (i64, Option<String>) = connection
            .query_row(
                "SELECT (SELECT COUNT(*) FROM document_integrity_flags WHERE audit_id = ?1),
                        (SELECT verification_status FROM case_audits WHERE audit_id = ?1)",
                params![audit_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        (findings, flags, status)
    };

    repository
        .purge_document(PORTAL_SUBDOMAIN, DOCUMENT_ID as u64)
        .await
        .expect("the document purge must succeed");
    assert_eq!(
        state(repository.clone(), audit_id.clone()).await,
        (
            vec!["about-kept".to_string()],
            0,
            Some("Compliant".to_string())
        )
    );

    repository
        .purge_source(PORTAL_SUBDOMAIN)
        .await
        .expect("the source purge must succeed");
    assert_eq!(state(repository, audit_id).await, (Vec::new(), 0, None));
}
