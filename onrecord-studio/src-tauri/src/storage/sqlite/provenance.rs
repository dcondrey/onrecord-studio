//! Document provenance verification and its append-only log.

use super::integrity::is_well_formed_sha256;
use super::*;

impl DuckDbRepository {
    /// Resolves the evidence path recorded for a document, for the command layer to gate.
    ///
    /// Visibility-agnostic on purpose. `resolve_public_document_path` restricts to public
    /// documents because it feeds parsers that render content; an integrity check has to be able
    /// to confirm a staff-only document is still the file that was ingested.
    pub async fn resolve_document_evidence_path(
        &self,
        document_id: u64,
        portal_key: &str,
    ) -> Result<Option<PathBuf>, AppError> {
        let portal_key = portal_key.to_string();
        let error_portal_key = portal_key.clone();
        self.query_connection("Document evidence path lookup", move |conn| {
            conn.query_row(
                "SELECT local_path
                 FROM documents
                 WHERE id = ?1 AND portal_subdomain = ?2",
                params![document_id, portal_key],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|error| match error {
                duckdb::Error::QueryReturnedNoRows => AppError::Provenance(format!(
                    "Document {error_portal_key}/{document_id} was not found"
                )),
                other => AppError::Provenance(format!(
                    "Failed to resolve the evidence path for {error_portal_key}/{document_id}: {other}"
                )),
            })
            .map(|path| {
                path.filter(|value| !value.trim().is_empty())
                    .map(PathBuf::from)
            })
        })
        .await
    }

    pub async fn verify_document_provenance(
        &self,
        document_id: u64,
        portal_key: &str,
        evidence: ProvenanceEvidenceFile,
    ) -> Result<DocumentProvenanceVerification, AppError> {
        let portal_key_owned = portal_key.to_string();
        let (context, current_text_projection_sha256) = self
            .query_connection("Load document provenance state", move |conn| {
                let portal_key = portal_key_owned.as_str();
                let context = conn
                    .query_row(
                        // Only the ingestion digest and the projection digest are read here. Manifest
                        // state used to come from `provenance_manifests` and is now validated live from
                        // the asset, because a stored row describes what was true when it was written and
                        // a verification has to describe the file in front of it.
                        //
                        // `expected_sha256` was `COALESCE(provenance_manifests.file_hash_sha256,
                        // documents.raw_file_sha256)`. That was inert while nothing wrote manifests; once
                        // they are written it would silently move every comparison off the digest recorded
                        // at download onto one observed later, which is the wrong side of a substitution.
                        "SELECT documents.raw_file_sha256,
                    documents.extracted_text_projection_sha256
                 FROM documents
                 WHERE documents.id = ?1 AND documents.portal_subdomain = ?2",
                        params![document_id, portal_key],
                        map_provenance_row,
                    )
                    .map_err(|error| match error {
                        duckdb::Error::QueryReturnedNoRows => AppError::Provenance(format!(
                            "Document {portal_key}/{document_id} was not found"
                        )),
                        other => AppError::Provenance(format!(
                            "Failed to load provenance state: {other}"
                        )),
                    })?;
                // Hashed inside the engine rather than in process: a large text projection should not
                // have to be materialized here just to be digested.
                let current_projection = conn
                    .query_row(
                        "SELECT sha256(extracted_text)
                     FROM documents_fts
                     WHERE document_id = ?1 AND portal_subdomain = ?2",
                        params![document_id, portal_key],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()
                    .map_err(|error| {
                        AppError::Provenance(format!(
                            "Failed to digest the extracted-text projection: {error}"
                        ))
                    })?
                    .flatten();
                Ok((context, current_projection))
            })
            .await?;
        // A document whose evidence file cannot be read is still verified and still logged. The
        // previous implementation returned an error here, so the one fact an investigator most
        // needs on the record -- that the file was gone when they checked -- was the one fact the
        // append-only log never captured.
        let (computed_sha256, asset_provenance, evidence_file_present, mut verification_errors) =
            match evidence {
                ProvenanceEvidenceFile::Readable(path) => {
                    // One worker makes both passes over the file: the digest, and the C2PA read.
                    let (digest, provenance) =
                        spawn_with("Provenance worker", AppError::Provenance, move || {
                            let digest = sha256_file(&path).map_err(|error| {
                                AppError::Provenance(format!(
                                    "Failed to hash '{}': {error}",
                                    path.display()
                                ))
                            })?;
                            let provenance = read_asset_provenance(&path)?;
                            Ok((digest, provenance))
                        })
                        .await??;
                    let findings = provenance.findings.clone();
                    (digest, provenance, true, findings)
                }
                ProvenanceEvidenceFile::Unavailable(reason) => (
                    String::new(),
                    AssetProvenance::default(),
                    false,
                    vec![reason],
                ),
            };

        // A recorded digest that is not a well-formed SHA-256 is a corrupt record, not evidence of
        // substitution. Comparing against it would produce a mismatch and accuse the file.
        let recorded_digest = context
            .expected_sha256
            .as_deref()
            .filter(|expected| is_well_formed_sha256(expected));
        if context.expected_sha256.is_some() && recorded_digest.is_none() {
            verification_errors.push(
                "The recorded digest is not a well-formed SHA-256 value, so the stored bytes \
                 could not be compared against it"
                    .to_string(),
            );
        }
        let hash_matches = evidence_file_present
            && recorded_digest
                .is_some_and(|expected| constant_time_ascii_equal(expected, &computed_sha256));

        // The derived-text check is a separate axis from the asset check. A document can be
        // byte-identical to what was ingested while the text quoted from it in a brief has drifted,
        // and the reverse is equally possible, so neither verdict is allowed to stand in for the
        // other.
        let recorded_projection = context
            .recorded_text_projection_sha256
            .as_deref()
            .filter(|digest| is_well_formed_sha256(digest));
        let text_projection_matches = match (
            recorded_projection,
            current_text_projection_sha256.as_deref(),
        ) {
            (Some(recorded), Some(current)) => Some(constant_time_ascii_equal(recorded, current)),
            _ => None,
        };
        if text_projection_matches == Some(false) {
            verification_errors.push(
                "The extracted-text projection no longer matches the digest recorded when the \
                 document was extracted, so text quoted from this document may not reflect what \
                 was released"
                    .to_string(),
            );
        } else if recorded_projection.is_some() && current_text_projection_sha256.is_none() {
            verification_errors.push(
                "A digest was recorded for the extracted-text projection, but the projection is \
                 missing"
                    .to_string(),
            );
        } else if recorded_projection.is_none() && context.recorded_text_projection_sha256.is_some()
        {
            verification_errors.push(
                "The digest recorded for the extracted-text projection is not a well-formed \
                 SHA-256 value, so text quoted from this document cannot be verified"
                    .to_string(),
            );
        } else if recorded_projection.is_none() && current_text_projection_sha256.is_some() {
            verification_errors.push(
                "No digest was recorded for the extracted-text projection, so text quoted from \
                 this document cannot be verified"
                    .to_string(),
            );
        }
        let assertion_count = asset_provenance.assertions.len();
        let invalid_assertion_count = asset_provenance.invalid_assertion_count();
        if context.expected_sha256.is_none() {
            verification_errors.push("No expected raw-file SHA-256 is recorded".to_string());
        } else if evidence_file_present && recorded_digest.is_some() && !hash_matches {
            verification_errors
                .push("Raw-file SHA-256 does not match the recorded digest".to_string());
        }
        if evidence_file_present && !asset_provenance.manifest_present {
            verification_errors
                .push("The document carries no embedded C2PA provenance manifest".to_string());
        }
        if invalid_assertion_count > 0 {
            verification_errors.push(format!(
                "{invalid_assertion_count} manifest assertions are not covered by a valid \
                 signature and cannot be relied on"
            ));
        }

        // Which digest was compared decides what a match is worth. A manifest digest is a third
        // party's statement about the asset; an ingestion digest is this application's own note of
        // what it downloaded, so matching it proves only that local storage is intact.
        let recorded_digest_source = if context.expected_sha256.is_some() {
            RecordedDigestSource::IngestionRecord
        } else {
            RecordedDigestSource::None
        };
        let signature_verification = asset_provenance.signature_verification();
        let provenance_confidence = classify_confidence(
            evidence_file_present,
            recorded_digest.is_some(),
            hash_matches,
            asset_provenance.manifest_present,
            asset_provenance.validation,
        );
        // Always false today, by construction rather than by accident: the only state that earns
        // this label requires a verified signature chaining to a trusted anchor.
        let chain_of_custody_confirmed = provenance_confidence.is_independent_custody_evidence();
        let result = DocumentProvenanceVerification {
            document_id,
            portal_key: portal_key.to_string(),
            expected_sha256: context.expected_sha256,
            computed_sha256,
            evidence_file_present,
            hash_matches,
            recorded_digest_source,
            manifest_present: asset_provenance.manifest_present,
            is_cryptographically_signed: asset_provenance.claims_signature(),
            signature_verification,
            signature_issuer: asset_provenance.signature_issuer.clone(),
            signing_certificate_serial_number: asset_provenance
                .signing_certificate_serial_number
                .clone(),
            signing_timestamp: asset_provenance.signing_timestamp.clone(),
            assertion_count,
            invalid_assertion_count,
            claim_graph: asset_provenance.claim_graph.clone(),
            text_projection_sha256: context.recorded_text_projection_sha256.clone(),
            text_projection_matches,
            provenance_confidence,
            chain_of_custody_confirmed,
            verification_errors,
        };
        let errors_json = serde_json::to_string(&result.verification_errors).map_err(|error| {
            AppError::Provenance(format!("Failed to encode verification errors: {error}"))
        })?;
        let portal_key_owned = portal_key.to_string();
        self.query_connection("Record document provenance verification", move |conn| {
            let portal_key = portal_key_owned.as_str();
            // The manifest tables record what the asset carried at this verification, and the log
            // records the verification itself. They are written together so the log can never point at
            // manifest state that was not committed.
            let transaction = conn.transaction().map_err(|error| {
                AppError::Provenance(format!(
                    "Failed to open the provenance transaction: {error}"
                ))
            })?;
            record_asset_manifest(
                &transaction,
                document_id,
                portal_key,
                &result.computed_sha256,
                &asset_provenance,
            )?;
            transaction
                .execute(
                    "INSERT INTO provenance_verification_log (
                verification_id, document_id, portal_subdomain, verified_at,
                expected_sha256, computed_sha256, hash_matches, manifest_present,
                is_cryptographically_signed, invalid_assertion_count,
                chain_of_custody_confirmed, verification_errors_json,
                recorded_digest_source, signature_verification, provenance_confidence,
                text_projection_matches, evidence_file_present, assertion_count,
                signature_issuer, signing_timestamp, text_projection_sha256,
                signing_certificate_serial_number
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                       ?17, ?18, ?19, ?20, ?21, ?22)",
                    params![
                        uuid::Uuid::new_v4().to_string(),
                        document_id,
                        portal_key,
                        chrono::Utc::now().to_rfc3339(),
                        result.expected_sha256,
                        result.computed_sha256,
                        result.hash_matches,
                        result.manifest_present,
                        result.is_cryptographically_signed,
                        result.invalid_assertion_count,
                        result.chain_of_custody_confirmed,
                        errors_json,
                        result.recorded_digest_source.as_str(),
                        result.signature_verification.as_str(),
                        result.provenance_confidence.as_str(),
                        result.text_projection_matches,
                        result.evidence_file_present,
                        i64::try_from(result.assertion_count).map_err(|_| {
                            AppError::Provenance(
                                "Assertion count exceeds storage capacity".to_string(),
                            )
                        })?,
                        result.signature_issuer,
                        result.signing_timestamp,
                        result.text_projection_sha256,
                        result.signing_certificate_serial_number,
                    ],
                )
                .map_err(|error| {
                    AppError::Provenance(format!("Failed to append verification log: {error}"))
                })?;
            transaction.commit().map_err(|error| {
                AppError::Provenance(format!("Failed to commit the provenance record: {error}"))
            })?;
            Ok(result)
        })
        .await
    }

    /// Reads a document's verification history, newest first.
    ///
    /// The log is the evidentiary artifact this subsystem produces: it shows that a document was
    /// checked, when, and what the check found. Until this existed the log was written and never
    /// read, so none of that was reachable from the application.
    pub async fn list_document_provenance_verifications(
        &self,
        document_id: u64,
        portal_key: &str,
        limit: usize,
    ) -> Result<Vec<ProvenanceVerificationRecord>, AppError> {
        let portal_key = portal_key.to_string();
        let limit = i64::try_from(limit).map_err(|_| {
            AppError::DataParsing("Verification history limit exceeds capacity".to_string())
        })?;
        self.query_connection("Provenance verification history", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT verification_id, document_id, portal_subdomain, verified_at,
                        expected_sha256, computed_sha256, hash_matches, manifest_present,
                        is_cryptographically_signed, invalid_assertion_count,
                        chain_of_custody_confirmed, verification_errors_json,
                        recorded_digest_source, signature_verification, provenance_confidence,
                        text_projection_matches, evidence_file_present, assertion_count,
                        signature_issuer, signing_timestamp, text_projection_sha256,
                        signing_certificate_serial_number
                     FROM provenance_verification_log
                     WHERE document_id = ?1 AND portal_subdomain = ?2
                     ORDER BY verified_at DESC, verification_id DESC
                     LIMIT ?3",
                )
                .map_err(|error| {
                    AppError::Provenance(format!(
                        "Failed to prepare the verification history query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(
                    params![document_id, portal_key, limit],
                    map_provenance_verification_record,
                )
                .map_err(|error| {
                    AppError::Provenance(format!(
                        "Failed to read the verification history: {error}"
                    ))
                })?;
            rows.map(|row| {
                convert_provenance_verification_record(row.map_err(|error| {
                    AppError::Provenance(format!("Failed to decode a verification row: {error}"))
                })?)
            })
            .collect()
        })
        .await
    }
}

struct RawProvenanceVerificationRecord {
    verification_id: String,
    document_id: i64,
    portal_key: String,
    verified_at: String,
    expected_sha256: Option<String>,
    computed_sha256: String,
    hash_matches: bool,
    manifest_present: bool,
    is_cryptographically_signed: bool,
    invalid_assertion_count: i64,
    chain_of_custody_confirmed: bool,
    verification_errors_json: String,
    recorded_digest_source: Option<String>,
    signature_verification: Option<String>,
    provenance_confidence: Option<String>,
    text_projection_matches: Option<bool>,
    evidence_file_present: Option<bool>,
    assertion_count: Option<i64>,
    signature_issuer: Option<String>,
    signing_timestamp: Option<String>,
    text_projection_sha256: Option<String>,
    signing_certificate_serial_number: Option<String>,
}

/// Replaces the recorded manifest and assertions for a document with what the asset carries now.
///
/// A full replace rather than a merge: these rows describe one observation of one file, so leaving
/// a stale assertion behind would blend two different readings of the document into one record.
/// `file_hash_sha256` is the digest of the bytes the manifest was read from, which is what ties the
/// recorded claims to a specific state of the file.
fn record_asset_manifest(
    tx: &Transaction<'_>,
    document_id: u64,
    portal_key: &str,
    computed_sha256: &str,
    provenance: &AssetProvenance,
) -> Result<(), AppError> {
    tx.execute(
        "DELETE FROM provenance_assertions
         WHERE document_id = ?1 AND portal_subdomain = ?2",
        params![document_id, portal_key],
    )
    .map_err(|error| AppError::Provenance(format!("Failed to clear prior assertions: {error}")))?;
    tx.execute(
        "DELETE FROM provenance_manifests
         WHERE document_id = ?1 AND portal_subdomain = ?2",
        params![document_id, portal_key],
    )
    .map_err(|error| {
        AppError::Provenance(format!("Failed to clear the prior manifest: {error}"))
    })?;

    if !provenance.manifest_present || computed_sha256.is_empty() {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO provenance_manifests (
            document_id, portal_subdomain, file_hash_sha256,
            is_cryptographically_signed, signature_issuer, signing_timestamp,
            signing_certificate_serial_number
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            document_id,
            portal_key,
            computed_sha256,
            provenance.claims_signature(),
            provenance.signature_issuer,
            provenance.signing_timestamp,
            provenance.signing_certificate_serial_number,
        ],
    )
    .map_err(|error| AppError::Provenance(format!("Failed to record the manifest: {error}")))?;

    let mut insert = tx
        .prepare_cached(
            "INSERT INTO provenance_assertions (
                document_id, portal_subdomain, assertion_type, actor_identifier, is_tamper_evident
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .map_err(|error| {
            AppError::Provenance(format!("Failed to prepare assertion insert: {error}"))
        })?;
    for assertion in &provenance.assertions {
        insert
            .execute(params![
                document_id,
                portal_key,
                assertion.assertion_type,
                assertion.actor_identifier,
                assertion.is_tamper_evident,
            ])
            .map_err(|error| {
                AppError::Provenance(format!("Failed to record a manifest assertion: {error}"))
            })?;
    }
    Ok(())
}

fn map_provenance_verification_record(
    row: &Row<'_>,
) -> duckdb::Result<RawProvenanceVerificationRecord> {
    Ok(RawProvenanceVerificationRecord {
        verification_id: row.get(0)?,
        document_id: row.get(1)?,
        portal_key: row.get(2)?,
        verified_at: row.get(3)?,
        expected_sha256: row.get(4)?,
        computed_sha256: row.get(5)?,
        hash_matches: row.get(6)?,
        manifest_present: row.get(7)?,
        is_cryptographically_signed: row.get(8)?,
        invalid_assertion_count: row.get(9)?,
        chain_of_custody_confirmed: row.get(10)?,
        verification_errors_json: row.get(11)?,
        recorded_digest_source: row.get(12)?,
        signature_verification: row.get(13)?,
        provenance_confidence: row.get(14)?,
        text_projection_matches: row.get(15)?,
        evidence_file_present: row.get(16)?,
        assertion_count: row.get(17)?,
        signature_issuer: row.get(18)?,
        signing_timestamp: row.get(19)?,
        text_projection_sha256: row.get(20)?,
        signing_certificate_serial_number: row.get(21)?,
    })
}

fn convert_provenance_verification_record(
    raw: RawProvenanceVerificationRecord,
) -> Result<ProvenanceVerificationRecord, AppError> {
    let provenance_confidence = raw
        .provenance_confidence
        .as_deref()
        .and_then(ProvenanceConfidence::from_stored);
    Ok(ProvenanceVerificationRecord {
        verification_id: raw.verification_id,
        document_id: u64::try_from(raw.document_id).map_err(|_| {
            AppError::Provenance(format!("Invalid logged document id {}", raw.document_id))
        })?,
        portal_key: raw.portal_key,
        verified_at: raw.verified_at,
        expected_sha256: raw.expected_sha256,
        computed_sha256: raw.computed_sha256,
        evidence_file_present: raw.evidence_file_present,
        hash_matches: raw.hash_matches,
        manifest_present: raw.manifest_present,
        is_cryptographically_signed: raw.is_cryptographically_signed,
        signature_issuer: raw.signature_issuer,
        signing_certificate_serial_number: raw.signing_certificate_serial_number,
        signing_timestamp: raw.signing_timestamp,
        assertion_count: raw
            .assertion_count
            .map(|value| {
                usize::try_from(value).map_err(|_| {
                    AppError::Provenance(format!("Invalid logged assertion count {value}"))
                })
            })
            .transpose()?,
        invalid_assertion_count: usize::try_from(raw.invalid_assertion_count).map_err(|_| {
            AppError::Provenance("Logged invalid-assertion count was negative".to_string())
        })?,
        recorded_digest_source: raw
            .recorded_digest_source
            .as_deref()
            .and_then(RecordedDigestSource::from_stored),
        signature_verification: raw
            .signature_verification
            .as_deref()
            .and_then(SignatureVerificationState::from_stored),
        // A row whose confidence does not parse predates classification. That is not a
        // low-confidence result and must never be shown as one.
        predates_classification: provenance_confidence.is_none(),
        provenance_confidence,
        text_projection_sha256: raw.text_projection_sha256,
        text_projection_matches: raw.text_projection_matches,
        chain_of_custody_confirmed: raw.chain_of_custody_confirmed,
        verification_errors: serde_json::from_str(&raw.verification_errors_json).map_err(
            |error| {
                AppError::Provenance(format!("Logged verification errors are invalid: {error}"))
            },
        )?,
    })
}

fn map_provenance_row(row: &Row<'_>) -> duckdb::Result<ProvenanceRow> {
    Ok(ProvenanceRow {
        expected_sha256: row.get(0)?,
        recorded_text_projection_sha256: row.get(1)?,
    })
}

fn constant_time_ascii_equal(left: &str, right: &str) -> bool {
    let left = left.trim().to_ascii_lowercase();
    let right = right.trim().to_ascii_lowercase();
    constant_time_equal(left.as_bytes(), right.as_bytes())
}
