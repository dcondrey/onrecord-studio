use super::*;

async fn record_manifest_claim(
    repository: &DuckDbRepository,
    file_hash_sha256: &str,
    claims_signature: bool,
) {
    let connection = repository.conn.lock().await;
    connection
        .execute(
            "INSERT INTO provenance_manifests (
                document_id, portal_subdomain, file_hash_sha256,
                is_cryptographically_signed, signature_issuer, signing_timestamp
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                DOCUMENT_ID,
                PORTAL_SUBDOMAIN,
                file_hash_sha256,
                claims_signature,
                "Example Issuer",
                "2026-08-24T12:02:00Z",
            ],
        )
        .expect("manifest fixture must insert");
}

/// Each provenance state must be reachable and distinguishable, and none of the states this
/// build can produce may present recorded claims as verified chain-of-custody evidence.
///
/// The fourth case is the one that matters: a manifest asserting a cryptographic signature,
/// over a file whose digest matches, previously reported `chain_of_custody_confirmed = true`
/// without any signature ever being checked.
#[tokio::test]
async fn provenance_states_are_distinguished_and_never_claim_verified_custody() {
    use crate::types::{ProvenanceConfidence, RecordedDigestSource, SignatureVerificationState};

    let directory = tempfile::tempdir().expect("temp dir must be created");
    let file_path = directory.path().join("release.txt");
    std::fs::write(&file_path, b"released bytes").expect("evidence file must be written");
    let actual_digest = crate::integrity::sha256_hex(b"released bytes");
    let substituted_digest = crate::integrity::sha256_hex(b"substituted bytes");
    let document_id = DOCUMENT_ID as u64;

    // 1. Nothing recorded: there is no digest to compare the stored bytes against.
    let repository = repository_with_document().await;
    set_document_file(&repository, &file_path, None).await;
    let unrecorded = repository
        .verify_document_provenance(
            document_id,
            PORTAL_SUBDOMAIN,
            ProvenanceEvidenceFile::Readable(file_path.clone()),
        )
        .await
        .expect("verification must run");
    assert_eq!(
        unrecorded.provenance_confidence,
        ProvenanceConfidence::NoRecordedDigest
    );
    assert_eq!(
        unrecorded.recorded_digest_source,
        RecordedDigestSource::None
    );
    assert!(!unrecorded.hash_matches);
    assert!(!unrecorded.manifest_present);

    // 2. The ingestion digest matches: local integrity, and nothing about provenance.
    let repository = repository_with_document().await;
    set_document_file(&repository, &file_path, Some(&actual_digest)).await;
    let local_match = repository
        .verify_document_provenance(
            document_id,
            PORTAL_SUBDOMAIN,
            ProvenanceEvidenceFile::Readable(file_path.clone()),
        )
        .await
        .expect("verification must run");
    assert_eq!(
        local_match.provenance_confidence,
        ProvenanceConfidence::LocalDigestMatch
    );
    assert_eq!(
        local_match.recorded_digest_source,
        RecordedDigestSource::IngestionRecord
    );
    assert!(local_match.hash_matches);

    // 3. The stored bytes are not the bytes that were recorded.
    let repository = repository_with_document().await;
    set_document_file(&repository, &file_path, Some(&substituted_digest)).await;
    let mismatch = repository
        .verify_document_provenance(
            document_id,
            PORTAL_SUBDOMAIN,
            ProvenanceEvidenceFile::Readable(file_path.clone()),
        )
        .await
        .expect("verification must run");
    assert_eq!(
        mismatch.provenance_confidence,
        ProvenanceConfidence::DigestMismatch
    );
    assert!(!mismatch.hash_matches);

    // 4. A stale manifest row claiming a signature, over a file that carries no embedded
    // manifest at all. Manifest state is read from the asset, so the stored claim must not
    // survive into the result: a row recorded earlier describes a file that may since have
    // been replaced, which is the whole reason verification reads the bytes in front of it.
    let repository = repository_with_document().await;
    set_document_file(&repository, &file_path, Some(&actual_digest)).await;
    record_manifest_claim(&repository, &actual_digest, true).await;
    let stale_claim = repository
        .verify_document_provenance(
            document_id,
            PORTAL_SUBDOMAIN,
            ProvenanceEvidenceFile::Readable(file_path.clone()),
        )
        .await
        .expect("verification must run");
    assert!(
        !stale_claim.manifest_present,
        "a stored manifest row must not make an asset without one report as having one"
    );
    assert!(!stale_claim.is_cryptographically_signed);
    assert_eq!(
        stale_claim.signature_verification,
        SignatureVerificationState::NotAttempted,
        "there is no manifest to check, so no signature was checked"
    );
    assert_eq!(
        stale_claim.recorded_digest_source,
        RecordedDigestSource::IngestionRecord
    );
    assert_eq!(
        stale_claim.provenance_confidence,
        ProvenanceConfidence::LocalDigestMatch
    );
    // The superseded row is gone from storage, not merely ignored in the response.
    {
        let connection = repository.conn.lock().await;
        let remaining: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM provenance_manifests
                 WHERE document_id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
                |row| row.get(0),
            )
            .expect("manifest count must read");
        assert_eq!(remaining, 0);
    }

    for result in [&unrecorded, &local_match, &mismatch, &stale_claim] {
        assert!(
            !result.chain_of_custody_confirmed,
            "no state this build can produce is independent custody evidence, but {:?} claimed it",
            result.provenance_confidence
        );
        assert!(!result.provenance_confidence.is_cryptographically_verified());
    }
}

/// A document whose evidence file cannot be read must still produce a recorded verification.
///
/// This is the case the previous implementation lost: it returned an error, so nothing reached
/// the append-only log and an investigator had no record that they looked and found the file
/// gone. The reason has to survive into the result, because "never downloaded" and "the
/// recorded path escaped the evidence store" are very different facts.
#[tokio::test]
async fn unreadable_evidence_is_recorded_rather_than_raised_as_an_error() {
    use crate::types::{ProvenanceConfidence, RecordedDigestSource};

    let repository = repository_with_document().await;
    // No local_path has been written for this document yet.
    assert_eq!(
        repository
            .resolve_document_evidence_path(DOCUMENT_ID as u64, PORTAL_SUBDOMAIN)
            .await
            .expect("path resolution must succeed"),
        None
    );

    let result = repository
        .verify_document_provenance(
            DOCUMENT_ID as u64,
            PORTAL_SUBDOMAIN,
            ProvenanceEvidenceFile::Unavailable(
                "The recorded evidence path is a symbolic link, which the managed evidence \
                 store does not permit"
                    .to_string(),
            ),
        )
        .await
        .expect("an unreadable evidence file must not fail the verification");

    assert_eq!(
        result.provenance_confidence,
        ProvenanceConfidence::EvidenceFileUnavailable
    );
    assert!(!result.evidence_file_present);
    assert!(!result.hash_matches);
    assert!(result.computed_sha256.is_empty());
    assert_eq!(result.recorded_digest_source, RecordedDigestSource::None);
    assert!(
        result
            .verification_errors
            .iter()
            .any(|message| message.contains("symbolic link")),
        "the specific reason must survive into the result, got {:?}",
        result.verification_errors
    );

    let connection = repository.conn.lock().await;
    let logged: String = connection
        .query_row(
            "SELECT provenance_confidence FROM provenance_verification_log
             WHERE document_id = ?1 AND portal_subdomain = ?2",
            params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            |row| row.get(0),
        )
        .expect("the verification must be appended to the log");
    assert_eq!(logged, "EVIDENCE_FILE_UNAVAILABLE");
}

/// Drift in the derived text must be detected, and must not implicate the asset.
///
/// `extracted_text_sha256` covers the raw extracted text, which is discarded after
/// finalization, so it can never be re-checked. The normalized projection is what search
/// returns and what a legal brief quotes, and until it carried its own digest nothing could
/// tell whether quoted text still matched the document it was drawn from.
#[tokio::test]
async fn extracted_text_drift_is_detected_without_implicating_the_asset() {
    use crate::types::ProvenanceConfidence;

    let directory = tempfile::tempdir().expect("temp dir must be created");
    let file_path = directory.path().join("release.txt");
    std::fs::write(&file_path, b"released bytes").expect("evidence file must be written");
    let file_digest = crate::integrity::sha256_hex(b"released bytes");

    let repository = repository_with_document().await;
    let extracted_text = "Encampment sweep notice\r\n[REDACTED]";
    let text_digest = compute_extracted_text_sha256(extracted_text);
    repository
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: DOCUMENT_ID as u64,
            portal_subdomain: PORTAL_SUBDOMAIN,
            extracted_text,
            extracted_text_sha256: &text_digest,
            raw_file_sha256: &file_digest,
            was_ocr_processed: false,
            raw_file_size: 14,
            local_path: &file_path,
            shannon_entropy: Some(3.5),
            entropy_sample_bytes: 14,
            suspicious_data_density: false,
        })
        .await
        .expect("finalization must commit");

    let intact = repository
        .verify_document_provenance(
            DOCUMENT_ID as u64,
            PORTAL_SUBDOMAIN,
            ProvenanceEvidenceFile::Readable(file_path.clone()),
        )
        .await
        .expect("verification must run");
    assert_eq!(
        intact.provenance_confidence,
        ProvenanceConfidence::LocalDigestMatch
    );
    assert_eq!(intact.text_projection_matches, Some(true));
    assert!(intact.text_projection_sha256.is_some());

    // Rewrite the projection while leaving the asset alone.
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "UPDATE documents_fts SET extracted_text = ?1
                 WHERE document_id = ?2 AND portal_subdomain = ?3",
                params![
                    "Encampment sweep notice with a sentence nobody released",
                    DOCUMENT_ID,
                    PORTAL_SUBDOMAIN
                ],
            )
            .expect("projection must update");
    }

    let drifted = repository
        .verify_document_provenance(
            DOCUMENT_ID as u64,
            PORTAL_SUBDOMAIN,
            ProvenanceEvidenceFile::Readable(file_path.clone()),
        )
        .await
        .expect("verification must run");
    assert!(drifted.hash_matches);
    assert_eq!(
        drifted.provenance_confidence,
        ProvenanceConfidence::LocalDigestMatch,
        "the asset is untouched, so the file verdict must not move"
    );
    assert_eq!(drifted.text_projection_matches, Some(false));
    assert!(
        drifted
            .verification_errors
            .iter()
            .any(|message| message.contains("quoted")),
        "drift must be described in terms of what it costs a citation, got {:?}",
        drifted.verification_errors
    );

    // A row imported from a legacy core has no recorded projection digest. That is reported
    // as unverifiable, never as a pass.
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "UPDATE documents SET extracted_text_projection_sha256 = NULL
                 WHERE id = ?1 AND portal_subdomain = ?2",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            )
            .expect("legacy shape must be reproducible");
    }
    let unverifiable = repository
        .verify_document_provenance(
            DOCUMENT_ID as u64,
            PORTAL_SUBDOMAIN,
            ProvenanceEvidenceFile::Readable(file_path),
        )
        .await
        .expect("verification must run");
    assert_eq!(unverifiable.text_projection_matches, None);
    assert!(unverifiable
        .verification_errors
        .iter()
        .any(|message| message.contains("cannot be verified")));

    let connection = repository.conn.lock().await;
    let drift_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM provenance_verification_log
             WHERE text_projection_matches = FALSE",
            [],
            |row| row.get(0),
        )
        .expect("the drift verdict must reach the log");
    assert_eq!(drift_rows, 1);
}

/// The append-only log must be readable, and a row written before classification existed must
/// read back as unclassified rather than as a low-confidence result.
#[tokio::test]
async fn the_verification_log_reads_back_newest_first() {
    use crate::types::{ProvenanceConfidence, RecordedDigestSource};

    let directory = tempfile::tempdir().expect("temp dir must be created");
    let file_path = directory.path().join("release.txt");
    std::fs::write(&file_path, b"released bytes").expect("evidence file must be written");
    let digest = crate::integrity::sha256_hex(b"released bytes");

    let repository = repository_with_document().await;
    set_document_file(&repository, &file_path, Some(&digest)).await;
    for _ in 0..2 {
        repository
            .verify_document_provenance(
                DOCUMENT_ID as u64,
                PORTAL_SUBDOMAIN,
                ProvenanceEvidenceFile::Readable(file_path.clone()),
            )
            .await
            .expect("verification must run");
    }
    {
        let connection = repository.conn.lock().await;
        connection
            .execute(
                "INSERT INTO provenance_verification_log (
                    verification_id, document_id, portal_subdomain, verified_at,
                    expected_sha256, computed_sha256, hash_matches, manifest_present,
                    is_cryptographically_signed, invalid_assertion_count,
                    chain_of_custody_confirmed, verification_errors_json,
                    recorded_digest_source, signature_verification, provenance_confidence
                 ) VALUES ('legacy-row', ?1, ?2, '2020-01-01T00:00:00Z', NULL, 'abc', TRUE,
                    FALSE, FALSE, 0, FALSE, '[]', 'UNCLASSIFIED_LEGACY_RECORD',
                    'UNCLASSIFIED_LEGACY_RECORD', 'UNCLASSIFIED_LEGACY_RECORD')",
                params![DOCUMENT_ID, PORTAL_SUBDOMAIN],
            )
            .expect("a pre-classification row must be insertable");
    }

    let history = repository
        .list_document_provenance_verifications(DOCUMENT_ID as u64, PORTAL_SUBDOMAIN, 50)
        .await
        .expect("history must read");
    assert_eq!(history.len(), 3);

    let current = &history[0];
    assert!(!current.predates_classification);
    assert_eq!(
        current.provenance_confidence,
        Some(ProvenanceConfidence::LocalDigestMatch)
    );
    assert_eq!(
        current.recorded_digest_source,
        Some(RecordedDigestSource::IngestionRecord)
    );
    assert_eq!(current.evidence_file_present, Some(true));
    assert_eq!(current.assertion_count, Some(0));
    assert_eq!(current.computed_sha256, digest);

    let legacy = history.last().expect("the oldest row must be present");
    assert_eq!(legacy.verification_id, "legacy-row");
    assert!(
        legacy.predates_classification,
        "a sentinel row must be marked unclassified"
    );
    assert_eq!(legacy.provenance_confidence, None);
    assert_eq!(legacy.recorded_digest_source, None);
    assert_eq!(legacy.signature_verification, None);
    assert_eq!(legacy.assertion_count, None);
    assert_eq!(legacy.evidence_file_present, None);

    let limited = repository
        .list_document_provenance_verifications(DOCUMENT_ID as u64, PORTAL_SUBDOMAIN, 1)
        .await
        .expect("history must read");
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].verification_id, current.verification_id);
}

/// A corrupt recorded digest must not be reported as a digest mismatch.
///
/// `mark_document_downloaded` normalizes on write, but a row imported from a legacy core
/// carries whatever that database held. Comparing against a truncated or non-hex value would
/// produce `DIGEST_MISMATCH` and accuse an intact file of having been substituted.
#[tokio::test]
async fn a_malformed_recorded_digest_is_not_reported_as_tampering() {
    use crate::types::ProvenanceConfidence;

    let directory = tempfile::tempdir().expect("temp dir must be created");
    let file_path = directory.path().join("release.txt");
    std::fs::write(&file_path, b"released bytes").expect("evidence file must be written");

    let repository = repository_with_document().await;
    set_document_file(&repository, &file_path, Some("not-a-sha256")).await;
    let result = repository
        .verify_document_provenance(
            DOCUMENT_ID as u64,
            PORTAL_SUBDOMAIN,
            ProvenanceEvidenceFile::Readable(file_path.clone()),
        )
        .await
        .expect("verification must run");

    assert_ne!(
        result.provenance_confidence,
        ProvenanceConfidence::DigestMismatch,
        "a corrupt record must not be presented as evidence of substitution"
    );
    assert_eq!(
        result.provenance_confidence,
        ProvenanceConfidence::NoRecordedDigest
    );
    assert!(!result.hash_matches);
    assert!(result.evidence_file_present);
    assert!(
        result
            .verification_errors
            .iter()
            .any(|message| message.contains("well-formed")),
        "the corrupt record must be named as such, got {:?}",
        result.verification_errors
    );
}
