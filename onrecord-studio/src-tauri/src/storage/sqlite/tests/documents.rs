use super::*;

/// A re-fetch that returns different bytes must leave the earlier digest on record, linked to
/// the one that replaced it.
#[tokio::test]
async fn refetching_changed_content_keeps_every_received_digest() {
    let repository = repository_with_document().await;
    let first = "a".repeat(64);
    let second = "b".repeat(64);
    for digest in [&first, &second] {
        repository
            .record_document_acquisition(
                PORTAL_SUBDOMAIN,
                DOCUMENT_ID as u64,
                Some("run"),
                digest,
                10,
                &test_acquisition(),
            )
            .await
            .unwrap();
        repository
            .conn
            .lock()
            .await
            .execute(
                "UPDATE documents SET raw_file_sha256 = ?1 WHERE id = ?2",
                params![digest, DOCUMENT_ID],
            )
            .unwrap();
    }
    let connection = repository.conn.lock().await;
    let mut statement = connection
        .prepare(
            "SELECT raw_file_sha256, previous_raw_file_sha256 FROM document_acquisitions
             WHERE document_id = ?1 ORDER BY acquisition_id",
        )
        .unwrap();
    let history = statement
        .query_map(params![DOCUMENT_ID], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(history, vec![(first.clone(), None), (second, Some(first))]);
}

#[tokio::test]
async fn extraction_finalization_commits_metadata_audits_flags_and_fts_atomically() {
    let repository = repository_with_document().await;
    let extracted_text = "sweep\r\n[REDACTED]\0 [REDACTED]";
    let uppercase_hash = compute_extracted_text_sha256(extracted_text).to_ascii_uppercase();

    repository
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: DOCUMENT_ID as u64,
            portal_subdomain: PORTAL_SUBDOMAIN,
            extracted_text,
            extracted_text_sha256: &uppercase_hash,
            raw_file_sha256: RAW_FILE_SHA256,
            was_ocr_processed: true,
            raw_file_size: 9_876,
            local_path: Path::new("/var/data/test-portal/release.pdf"),
            shannon_entropy: Some(7.875),
            entropy_sample_bytes: 9_876,
            suspicious_data_density: true,
        })
        .await
        .expect("finalization must commit");

    {
        let connection = repository.conn.lock().await;
        let document_state: (i64, String, String, i64, f64, i64, i64, String, String) = connection
            .query_row(
                "SELECT
                    file_size,
                    local_path,
                    extracted_text_sha256,
                    was_ocr_processed,
                    shannon_entropy,
                    entropy_sample_bytes,
                    suspicious_data_density,
                    download_status,
                    extraction_status
                 FROM documents
                 WHERE id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                    ))
                },
            )
            .expect("document state must exist");
        assert_eq!(
            document_state,
            (
                9_876,
                "/var/data/test-portal/release.pdf".to_string(),
                uppercase_hash.to_ascii_lowercase(),
                1,
                7.875,
                9_876,
                1,
                "completed".to_string(),
                "SEARCHABLE".to_string(),
            )
        );

        let audit_status: String = connection
            .query_row(
                "SELECT verification_status
                 FROM case_audits
                 WHERE audit_id = ?1",
                params![format!("auto-{}-{}", PORTAL_SUBDOMAIN, REQUEST_ID)],
                |row| row.get(0),
            )
            .expect("automatic audit must exist");
        assert_eq!(audit_status, "DataAnomalyFlagged");

        let mut flag_statement = connection
            .prepare(
                "SELECT flag_type
                 FROM document_integrity_flags
                 WHERE audit_id = ?1
                 ORDER BY flag_type",
            )
            .expect("flag query must prepare");
        let flags = flag_statement
            .query_map(
                params![format!("auto-{}-{}", PORTAL_SUBDOMAIN, REQUEST_ID)],
                |row| row.get::<_, String>(0),
            )
            .expect("flag query must execute")
            .collect::<duckdb::Result<Vec<_>>>()
            .expect("flags must decode");
        assert_eq!(
            flags,
            vec![
                "HEAVY_REDACTION_DENSITY".to_string(),
                "SUSPICIOUSLY_FAST_CLOSURE".to_string(),
                "SUSPICIOUS_DATA_DENSITY".to_string(),
                "UPDATE_BEFORE_CREATION".to_string(),
            ]
        );

        let indexed_row: (i64, String) = connection
            .query_row(
                "SELECT COUNT(*), MIN(extracted_text)
                 FROM documents_fts
                 WHERE document_id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("FTS state must exist");
        assert_eq!(indexed_row.0, 1);
        assert_eq!(indexed_row.1, "sweep [REDACTED] [REDACTED]");
    }
}

#[tokio::test]
async fn repeated_finalization_updates_the_single_fts_row_and_refreshes_flags() {
    let repository = repository_with_document().await;
    let path = Path::new("/var/data/test-portal/release.pdf");
    let first_text = "sweep [REDACTED] [REDACTED]";
    let first_hash = compute_extracted_text_sha256(first_text);
    let second_text = "ordinary retention schedule";
    let second_hash = compute_extracted_text_sha256(second_text);

    repository
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: DOCUMENT_ID as u64,
            portal_subdomain: PORTAL_SUBDOMAIN,
            extracted_text: first_text,
            extracted_text_sha256: &first_hash,
            raw_file_sha256: RAW_FILE_SHA256,
            was_ocr_processed: false,
            raw_file_size: 100,
            local_path: path,
            shannon_entropy: Some(4.5),
            entropy_sample_bytes: 100,
            suspicious_data_density: false,
        })
        .await
        .expect("initial finalization must succeed");
    repository
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: DOCUMENT_ID as u64,
            portal_subdomain: PORTAL_SUBDOMAIN,
            extracted_text: second_text,
            extracted_text_sha256: &second_hash,
            raw_file_sha256: RAW_FILE_SHA256,
            was_ocr_processed: false,
            raw_file_size: 120,
            local_path: path,
            shannon_entropy: Some(4.25),
            entropy_sample_bytes: 120,
            suspicious_data_density: false,
        })
        .await
        .expect("re-finalization must succeed");

    {
        let connection = repository.conn.lock().await;
        let indexed_row: (i64, String) = connection
            .query_row(
                "SELECT COUNT(*), MIN(extracted_text)
                 FROM documents_fts
                 WHERE document_id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("one refreshed FTS row must exist");
        assert_eq!(indexed_row, (1, "ordinary retention schedule".to_string()));

        let heavy_flag_count: i64 = connection
            .query_row(
                "SELECT COUNT(*)
                 FROM document_integrity_flags
                 WHERE audit_id = ?1 AND flag_type = ?2",
                params![
                    format!("auto-{}-{}", PORTAL_SUBDOMAIN, REQUEST_ID),
                    "HEAVY_REDACTION_DENSITY",
                ],
                |row| row.get(0),
            )
            .expect("heavy flag count must decode");
        assert_eq!(heavy_flag_count, 0);
    }
}

/// A portal identifier does not fit in 32 bits, and DuckDB's `INTEGER` is 32 bits.
///
/// This schema was written against SQLite, where `INTEGER` is 64-bit. Under DuckDB a real
/// source produced request id 5614575451713354 -- 53 bits -- and the entire ingestion
/// transaction aborted with "Conversion Error: Type UINT64 with value 5614575451713354 can't
/// be cast because the value is out of range for the destination type INT32", so the record
/// was never stored at all. Every other test used identifiers small enough to fit.
#[tokio::test]
async fn a_portal_identifier_wider_than_thirty_two_bits_is_stored() {
    const WIDE_REQUEST_ID: u64 = 5_614_575_451_713_354;
    assert!(WIDE_REQUEST_ID > i64::from(i32::MAX) as u64);

    let repository = repository_with_document().await;
    let request = PublicRequest {
        id: WIDE_REQUEST_ID,
        rpid: None,
        title: Some("Nonprofit Explorer filing".to_string()),
        description: None,
        status: "open".to_string(),
        current_status_id: None,
        created_at: "2026-08-25T09:00:00Z".to_string(),
        updated_at: "2026-08-25T09:00:00Z".to_string(),
        closed_at: None,
        due_date: None,
        determination_at: None,
        determination_source: None,
        neighborhood_block: None,
        statutory_extension: None,
        documents: Vec::new(),
        agency: None,
        category: None,
        timeline_anomalies: Vec::new(),
    };
    repository
        .insert_or_update_request(PORTAL_SUBDOMAIN, &request)
        .await
        .expect("a 53-bit portal identifier must be storable");

    let stored: i64 = {
        let connection = repository.conn.lock().await;
        connection
            .query_row(
                "SELECT id FROM requests WHERE id = ?1 AND portal_subdomain = ?2",
                params![WIDE_REQUEST_ID as i64, PORTAL_SUBDOMAIN],
                |row| row.get(0),
            )
            .expect("the wide identifier must round trip")
    };
    assert_eq!(stored as u64, WIDE_REQUEST_ID);
}

/// 0.0 Shannon entropy is a measurement -- every byte in the sample identical -- and the
/// strongest uniformity signal the assessment can produce. A document nothing ever assessed
/// must not read as that. With no DEFAULT on the column the two are separate values, and the
/// validator refuses to mix them.
#[tokio::test]
async fn never_computed_entropy_reads_absent_while_a_measured_zero_reads_zero() {
    let repository = repository_with_document().await;

    async fn recorded_entropy(repository: &DuckDbRepository) -> Option<f64> {
        let connection = repository.conn.lock().await;
        connection
            .query_row(
                "SELECT shannon_entropy FROM documents
                 WHERE id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                |row| row.get(0),
            )
            .expect("the fixture document must exist")
    }

    assert_eq!(
        recorded_entropy(&repository).await,
        None,
        "a document written before its entropy was computed must read as absent"
    );

    // A zero-byte evidence file: `calculate_shannon_entropy` scores an empty sample 0.0, so
    // this is a genuine measurement of zero and not a stand-in for absence.
    repository
        .mark_document_downloaded(DocumentDownloadFinalization {
            document_id: DOCUMENT_ID as u64,
            portal_subdomain: PORTAL_SUBDOMAIN,
            raw_file_size: 0,
            raw_file_sha256: RAW_FILE_SHA256,
            local_path: Path::new("/var/data/test-portal/empty.pdf"),
            shannon_entropy: Some(0.0),
            entropy_sample_bytes: 0,
            suspicious_data_density: false,
        })
        .await
        .expect("a zero-byte evidence file must finalize");

    assert_eq!(
        recorded_entropy(&repository).await,
        Some(0.0),
        "a measured zero must survive the round trip as a value, not collapse into absence"
    );

    // The two cannot be mixed: bytes that were read produced a score.
    assert!(matches!(
        validate_entropy_metadata(None, 64, 128, false),
        Err(crate::error::AppError::DataParsing(_))
    ));
}

#[tokio::test]
async fn hash_mismatch_rejects_finalization_without_persisting_partial_state() {
    let repository = repository_with_document().await;
    let result = repository
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: DOCUMENT_ID as u64,
            portal_subdomain: PORTAL_SUBDOMAIN,
            extracted_text: "responsive text",
            extracted_text_sha256:
                "0000000000000000000000000000000000000000000000000000000000000000",
            raw_file_sha256: RAW_FILE_SHA256,
            was_ocr_processed: true,
            raw_file_size: 500,
            local_path: Path::new("/var/data/test-portal/release.pdf"),
            shannon_entropy: Some(4.0),
            entropy_sample_bytes: 500,
            suspicious_data_density: false,
        })
        .await;
    assert!(matches!(
        result,
        Err(crate::error::AppError::DataParsing(_))
    ));

    let connection = repository.conn.lock().await;
    let document_state: (Option<i64>, Option<String>, Option<String>, String) = connection
        .query_row(
            "SELECT file_size, local_path, extracted_text_sha256, download_status
             FROM documents
             WHERE id = ?1 AND portal_subdomain = ?2",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("unfinalized document must still exist");
    assert_eq!(document_state, (None, None, None, "pending".to_string()));

    let audit_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM case_audits", [], |row| row.get(0))
        .expect("audit count must decode");
    let fts_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM documents_fts", [], |row| row.get(0))
        .expect("FTS count must decode");
    assert_eq!((audit_count, fts_count), (0, 0));
}

#[tokio::test]
async fn fts_failure_rolls_back_document_and_audit_updates() {
    let repository = repository_with_document().await;
    let extracted_text = "responsive text";
    let extracted_text_sha256 = compute_extracted_text_sha256(extracted_text);
    {
        let connection = repository.conn.lock().await;
        connection
            .execute("DROP TABLE documents_fts", [])
            .expect("FTS table must drop for the failure fixture");
    }

    let result = repository
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: DOCUMENT_ID as u64,
            portal_subdomain: PORTAL_SUBDOMAIN,
            extracted_text,
            extracted_text_sha256: &extracted_text_sha256,
            raw_file_sha256: RAW_FILE_SHA256,
            was_ocr_processed: false,
            raw_file_size: 500,
            local_path: Path::new("/var/data/test-portal/release.pdf"),
            shannon_entropy: Some(4.0),
            entropy_sample_bytes: 500,
            suspicious_data_density: false,
        })
        .await;
    assert!(result.is_err());

    let connection = repository.conn.lock().await;
    let document_state: (Option<i64>, Option<String>, Option<String>, String) = connection
        .query_row(
            "SELECT file_size, local_path, extracted_text_sha256, download_status
             FROM documents
             WHERE id = ?1 AND portal_subdomain = ?2",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("rolled-back document must still exist");
    assert_eq!(document_state, (None, None, None, "pending".to_string()));

    let audit_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM case_audits WHERE audit_id = ?1",
            params![format!("auto-{}-{}", PORTAL_SUBDOMAIN, REQUEST_ID)],
            |row| row.get(0),
        )
        .expect("audit count must decode");
    assert_eq!(audit_count, 0);
}

/// The embedding rebuild reads its sources one keyset page at a time, so every eligible document
/// must appear exactly once across pages, in order, including across a portal boundary.
#[tokio::test]
async fn vector_source_pages_visit_every_document_once_in_key_order() {
    const OTHER_PORTAL: &str = "a-portal";
    let repository = repository_with_document().await;
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO portals (subdomain, display_name, base_url) VALUES (?1, ?2, ?3)",
                params![OTHER_PORTAL, "A Portal", "https://a.example.test"],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO requests (id, portal_subdomain, status, created_at, updated_at)
                 VALUES (?1, ?2, 'open', '2026-08-24T12:00:00Z', '2026-08-24T12:00:00Z')",
                params![REQUEST_ID, OTHER_PORTAL],
            )
            .unwrap();
        for (portal, document_id) in [
            (OTHER_PORTAL, DOCUMENT_ID),
            (PORTAL_SUBDOMAIN, DOCUMENT_ID + 1),
            (OTHER_PORTAL, DOCUMENT_ID + 2),
        ] {
            connection
                .execute(
                    "INSERT INTO documents (
                        id, request_id, portal_subdomain, file_name, visibility,
                        created_at, updated_at
                     ) VALUES (?1, ?2, ?3, 'page.pdf', 'public',
                        '2026-08-24T12:01:00Z', '2026-08-24T12:01:00Z')",
                    params![document_id, REQUEST_ID, portal],
                )
                .unwrap();
        }
        for (portal, document_id) in [
            (OTHER_PORTAL, DOCUMENT_ID),
            (PORTAL_SUBDOMAIN, DOCUMENT_ID),
            (PORTAL_SUBDOMAIN, DOCUMENT_ID + 1),
            (OTHER_PORTAL, DOCUMENT_ID + 2),
        ] {
            connection
                .execute(
                    "UPDATE documents SET extracted_text_sha256 = ?3
                     WHERE id = ?1 AND portal_subdomain = ?2",
                    params![document_id, portal, RAW_FILE_SHA256],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO documents_fts (
                        document_id, portal_subdomain, file_name, extracted_text
                     ) VALUES (?1, ?2, 'page.pdf', 'text')",
                    params![document_id, portal],
                )
                .unwrap();
        }
    }

    let mut visited = Vec::new();
    let mut after = None;
    loop {
        let page = repository
            .list_vector_source_documents_page(after.take(), 1)
            .await
            .expect("a vector source page must load");
        let Some(last) = page.last() else {
            break;
        };
        after = Some((last.portal_key.clone(), last.document_id));
        visited.extend(
            page.iter()
                .map(|document| (document.portal_key.clone(), document.document_id)),
        );
    }
    let document = |portal: &str, id: i64| (portal.to_string(), id as u64);
    assert_eq!(
        visited,
        vec![
            document(OTHER_PORTAL, DOCUMENT_ID),
            document(OTHER_PORTAL, DOCUMENT_ID + 2),
            document(PORTAL_SUBDOMAIN, DOCUMENT_ID),
            document(PORTAL_SUBDOMAIN, DOCUMENT_ID + 1),
        ]
    );
}
