//! Policy-mandate evaluation and review of the legal findings it stores.

use super::*;
use super::{
    connection::execute_transaction,
    integrity::{update_automatic_audit_status, upsert_automatic_case_audit},
};

impl DuckDbRepository {
    pub async fn evaluate_request_legal_compliance(
        &self,
        request_id: u64,
        portal_key: &str,
    ) -> Result<LegalEvaluationResult, AppError> {
        let portal_key_owned = portal_key.to_string();
        let state = self
            .query_connection("Evaluate request legal compliance", move |conn| {
                execute_transaction(conn, "policy evaluation", |tx| {
                    synchronize_policy_evidence_gaps(tx, request_id, &portal_key_owned)
                })
            })
            .await?;
        let findings = self
            .list_request_legal_findings(request_id, portal_key)
            .await?;
        Ok(LegalEvaluationResult {
            request_id,
            portal_key: portal_key.to_string(),
            evaluated_mandate_count: state.evaluated_mandate_count,
            applicable_mandate_count: state.applicable_mandate_count,
            corpus_character_count: state.corpus_character_count,
            evaluation_complete: state.evaluation_complete,
            warnings: state.warnings,
            findings,
        })
    }

    pub async fn list_request_legal_findings(
        &self,
        request_id: u64,
        portal_key: &str,
    ) -> Result<Vec<LegalComplianceFinding>, AppError> {
        let portal_key = portal_key.to_string();
        self.query_connection("List request legal findings", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT
                    statutory_violations.violation_id, statutory_violations.request_id,
                    statutory_violations.portal_subdomain,
                    statutory_violations.matched_statute_id,
                    statutory_violations.matched_version_id,
                    statutory_violations.mandate_id,
                    legal_statutes.statute_citation, legal_statutes.title,
                    legal_mandates.title, statutory_violations.classification,
                    statutory_violations.calculated_variance_metric,
                    statutory_violations.variance_days,
                    statutory_violations.confidence_basis_points,
                    statutory_violations.evidence_summary,
                    statutory_violations.supporting_document_ids_json,
                    statutory_violations.missing_evidence_json,
                    statutory_violations.review_status,
                    statutory_violations.first_detected_at,
                    statutory_violations.evaluated_at
                 FROM statutory_violations
                 INNER JOIN legal_statutes
                    ON legal_statutes.statute_id = statutory_violations.matched_statute_id
                 LEFT JOIN legal_mandates
                    ON legal_mandates.mandate_id = statutory_violations.mandate_id
                 WHERE statutory_violations.request_id = ?1
                   AND statutory_violations.portal_subdomain = ?2
                 ORDER BY
                    CASE statutory_violations.review_status
                        WHEN 'OPEN' THEN 0 WHEN 'CONFIRMED' THEN 1
                        WHEN 'RESOLVED_BY_NEW_EVIDENCE' THEN 2 ELSE 3
                    END,
                    statutory_violations.confidence_basis_points DESC,
                    statutory_violations.evaluated_at DESC",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare legal findings query: {error}"))
                })?;
            let rows = statement
                .query_map(params![request_id, portal_key], map_legal_finding_row)
                .map_err(|error| {
                    AppError::Storage(format!("Failed to query legal findings: {error}"))
                })?;
            rows.map(|row| {
                row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode legal finding: {error}"))
                })
                .and_then(convert_legal_finding)
            })
            .collect()
        })
        .await
    }

    pub async fn review_legal_finding(
        &self,
        violation_id: &str,
        request_id: u64,
        portal_key: &str,
        review_status: LegalFindingReviewStatus,
        reviewer_notes: Option<&str>,
    ) -> Result<Vec<LegalComplianceFinding>, AppError> {
        let violation_id = violation_id.to_string();
        let portal_key_owned = portal_key.to_string();
        let reviewer_notes = reviewer_notes.map(str::to_string);
        self.query_connection("Review legal finding", move |conn| {
            execute_transaction(conn, "legal finding review", |tx| {
                let classification = tx
                    .query_row(
                        "SELECT classification
                     FROM statutory_violations
                     WHERE violation_id = ?1
                       AND request_id = ?2
                       AND portal_subdomain = ?3",
                        params![violation_id, request_id, portal_key_owned],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|error| match error {
                        duckdb::Error::QueryReturnedNoRows => AppError::Storage(format!(
                        "Legal finding '{violation_id}' was not found for {portal_key_owned}/{request_id}"
                    )),
                        other => AppError::Storage(format!(
                            "Failed to load the legal finding for review: {other}"
                        )),
                    })?;
                let updated = tx
                    .execute(
                        "UPDATE statutory_violations
                     SET review_status = ?1,
                         reviewed_at = CASE
                            WHEN ?1 = 'OPEN' THEN NULL
                            ELSE strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
                         END,
                         reviewer_notes = ?2
                     WHERE violation_id = ?3
                       AND request_id = ?4
                       AND portal_subdomain = ?5",
                        params![
                            review_status.as_str(),
                            reviewer_notes,
                            violation_id,
                            request_id,
                            portal_key_owned,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to record legal finding review: {error}"))
                    })?;
                if updated != 1 {
                    return Err(AppError::Storage(format!(
                        "Legal finding review affected {updated} records"
                    )));
                }
                if classification == FLAG_POTENTIAL_POLICY_VIOLATION {
                    synchronize_policy_audit_alert(tx, request_id, &portal_key_owned)?;
                }
                Ok(())
            })
        })
        .await?;
        self.list_request_legal_findings(request_id, portal_key)
            .await
    }
}

fn map_raw_stored_legal_mandate(row: &Row<'_>) -> duckdb::Result<RawStoredLegalMandate> {
    Ok(RawStoredLegalMandate {
        mandate_id: row.get(0)?,
        statute_id: row.get(1)?,
        version_id: row.get(2)?,
        title: row.get(3)?,
        applicability_json: row.get(4)?,
        evidence_json: row.get(5)?,
    })
}

fn map_legal_finding_row(row: &Row<'_>) -> duckdb::Result<RawLegalFinding> {
    Ok(RawLegalFinding {
        violation_id: row.get(0)?,
        request_id: row.get(1)?,
        portal_key: row.get(2)?,
        matched_statute_id: row.get(3)?,
        matched_version_id: row.get(4)?,
        mandate_id: row.get(5)?,
        statute_citation: row.get(6)?,
        authority_title: row.get(7)?,
        mandate_title: row.get(8)?,
        classification: row.get(9)?,
        calculated_variance_metric: row.get(10)?,
        variance_days: row.get(11)?,
        confidence_basis_points: row.get(12)?,
        evidence_summary: row.get(13)?,
        supporting_document_ids_json: row.get(14)?,
        missing_evidence_json: row.get(15)?,
        review_status: row.get(16)?,
        first_detected_at: row.get(17)?,
        evaluated_at: row.get(18)?,
    })
}

fn convert_legal_finding(row: RawLegalFinding) -> Result<LegalComplianceFinding, AppError> {
    let request_id = u64::try_from(row.request_id).map_err(|_| {
        AppError::Storage(format!(
            "Legal finding '{}' has an invalid request identifier",
            row.violation_id
        ))
    })?;
    let confidence_basis_points = u16::try_from(row.confidence_basis_points).map_err(|_| {
        AppError::Storage(format!(
            "Legal finding '{}' has an invalid confidence score",
            row.violation_id
        ))
    })?;
    if confidence_basis_points > 10_000 {
        return Err(AppError::Storage(format!(
            "Legal finding '{}' exceeds the confidence scale",
            row.violation_id
        )));
    }
    let supporting_document_ids =
        serde_json::from_str::<Vec<u64>>(&row.supporting_document_ids_json).map_err(|error| {
            AppError::Storage(format!(
                "Legal finding '{}' has invalid supporting evidence: {error}",
                row.violation_id
            ))
        })?;
    let missing_evidence_phrases = serde_json::from_str::<Vec<String>>(&row.missing_evidence_json)
        .map_err(|error| {
            AppError::Storage(format!(
                "Legal finding '{}' has invalid missing-evidence data: {error}",
                row.violation_id
            ))
        })?;
    Ok(LegalComplianceFinding {
        violation_id: row.violation_id,
        request_id,
        portal_key: row.portal_key,
        matched_statute_id: row.matched_statute_id,
        matched_version_id: row.matched_version_id,
        mandate_id: row.mandate_id,
        statute_citation: row.statute_citation,
        authority_title: row.authority_title,
        mandate_title: row.mandate_title,
        classification: row.classification,
        calculated_variance_metric: row.calculated_variance_metric,
        variance_days: row.variance_days,
        confidence_basis_points,
        evidence_summary: row.evidence_summary,
        supporting_document_ids,
        missing_evidence_phrases,
        review_status: row.review_status,
        first_detected_at: row.first_detected_at,
        evaluated_at: row.evaluated_at,
    })
}

pub(super) fn synchronize_policy_evidence_gaps(
    tx: &Transaction<'_>,
    request_id: u64,
    portal_key: &str,
) -> Result<PolicyEvaluationState, AppError> {
    let (title, description) = tx
        .query_row(
            "SELECT COALESCE(title, ''), COALESCE(description, '')
             FROM requests
             WHERE id = ?1 AND portal_subdomain = ?2",
            params![request_id, portal_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| match error {
            duckdb::Error::QueryReturnedNoRows => {
                AppError::Storage(format!("Request {portal_key}/{request_id} was not found"))
            }
            other => AppError::Storage(format!("Failed to load request policy context: {other}")),
        })?;
    let (public_document_count, indexed_document_count) = tx
        .query_row(
            "SELECT
                COUNT(*) FILTER (WHERE LOWER(TRIM(documents.visibility)) = 'public'),
                COUNT(*) FILTER (
                    WHERE LOWER(TRIM(documents.visibility)) = 'public'
                      AND EXISTS (
                        SELECT 1 FROM documents_fts
                        WHERE documents_fts.document_id = documents.id
                          AND documents_fts.portal_subdomain = documents.portal_subdomain
                      )
                )
             FROM documents
             WHERE documents.request_id = ?1
               AND documents.portal_subdomain = ?2",
            params![request_id, portal_key],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to inspect policy evidence completeness: {error}"
            ))
        })?;
    let public_document_count = usize::try_from(public_document_count).map_err(|_| {
        AppError::Storage("Public document count was negative or too large".to_string())
    })?;
    let indexed_document_count = usize::try_from(indexed_document_count).map_err(|_| {
        AppError::Storage("Indexed document count was negative or too large".to_string())
    })?;
    if public_document_count == 0 || public_document_count != indexed_document_count {
        let warning = if public_document_count == 0 {
            "Policy evaluation deferred because the request has no public document evidence"
                .to_string()
        } else {
            format!(
                "Policy evaluation deferred because only {indexed_document_count} of {public_document_count} public documents have searchable text"
            )
        };
        return Ok(PolicyEvaluationState {
            evaluated_mandate_count: 0,
            applicable_mandate_count: 0,
            corpus_character_count: 0,
            evaluation_complete: false,
            warnings: vec![warning],
        });
    }

    let metadata = collapse_whitespace(&format!("{title} {description}"));
    let mut corpus_bytes = metadata.len();
    let mut corpus_character_count = metadata.chars().count();
    let mut sources = vec![PolicyCorpusSource {
        document_id: None,
        tokens: policy_tokens(&metadata),
    }];
    {
        let mut statement = tx
            .prepare(
                "SELECT documents.id,
                    (SELECT documents_fts.extracted_text
                     FROM documents_fts
                     WHERE documents_fts.document_id = documents.id
                       AND documents_fts.portal_subdomain = documents.portal_subdomain
                     ORDER BY documents_fts.document_id
                     LIMIT 1)
                 FROM documents
                 WHERE documents.request_id = ?1
                   AND documents.portal_subdomain = ?2
                   AND LOWER(TRIM(documents.visibility)) = 'public'
                   AND EXISTS (
                        SELECT 1 FROM documents_fts
                        WHERE documents_fts.document_id = documents.id
                          AND documents_fts.portal_subdomain = documents.portal_subdomain
                   )
                 ORDER BY documents.id",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to prepare policy evidence scan: {error}"))
            })?;
        let rows = statement
            .query_map(params![request_id, portal_key], |row| {
                Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| {
                AppError::Storage(format!("Failed to query policy evidence: {error}"))
            })?;
        for row in rows {
            let (document_id, text) = row.map_err(|error| {
                AppError::Storage(format!("Failed to decode policy evidence: {error}"))
            })?;
            corpus_bytes = corpus_bytes.checked_add(text.len()).ok_or_else(|| {
                AppError::Storage("Policy evidence corpus size overflowed".to_string())
            })?;
            corpus_character_count = corpus_character_count
                .checked_add(text.chars().count())
                .ok_or_else(|| {
                    AppError::Storage("Policy evidence character count overflowed".to_string())
                })?;
            if corpus_bytes > MAXIMUM_POLICY_CORPUS_BYTES {
                return Ok(PolicyEvaluationState {
                    evaluated_mandate_count: 0,
                    applicable_mandate_count: 0,
                    corpus_character_count,
                    evaluation_complete: false,
                    warnings: vec![format!(
                        "Policy evaluation deferred because the searchable corpus exceeds the {} MiB safety limit",
                        MAXIMUM_POLICY_CORPUS_BYTES / (1024 * 1024)
                    )],
                });
            }
            sources.push(PolicyCorpusSource {
                document_id: Some(document_id),
                tokens: policy_tokens(&text),
            });
        }
    }

    let mandates = {
        let mut statement = tx
            .prepare(
                "SELECT legal_mandates.mandate_id, legal_mandates.statute_id,
                    legal_mandates.version_id, legal_mandates.title,
                    legal_mandates.applicability_phrases_json,
                    legal_mandates.required_evidence_phrases_json
                 FROM legal_mandates
                 INNER JOIN legal_statutes
                    ON legal_statutes.statute_id = legal_mandates.statute_id
                   AND legal_statutes.current_version_id = legal_mandates.version_id
                 WHERE legal_mandates.enabled = 1
                 ORDER BY legal_mandates.statute_id, legal_mandates.mandate_id",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to prepare legal mandate scan: {error}"))
            })?;
        let rows = statement
            .query_map([], map_raw_stored_legal_mandate)
            .map_err(|error| {
                AppError::Storage(format!("Failed to query legal mandates: {error}"))
            })?;
        let mut mandates = Vec::new();
        for row in rows {
            let row = row.map_err(|error| {
                AppError::Storage(format!("Failed to decode legal mandate: {error}"))
            })?;
            let applicability_phrases =
                serde_json::from_str::<Vec<String>>(&row.applicability_json).map_err(|error| {
                    AppError::Storage(format!(
                        "Mandate '{}' has invalid applicability phrases: {error}",
                        row.mandate_id
                    ))
                })?;
            let required_evidence_phrases = serde_json::from_str::<Vec<String>>(&row.evidence_json)
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Mandate '{}' has invalid evidence phrases: {error}",
                        row.mandate_id
                    ))
                })?;
            if applicability_phrases.is_empty() || required_evidence_phrases.is_empty() {
                return Err(AppError::Storage(format!(
                    "Mandate '{}' has an empty phrase set",
                    row.mandate_id
                )));
            }
            mandates.push(StoredLegalMandate {
                mandate_id: row.mandate_id,
                statute_id: row.statute_id,
                version_id: row.version_id,
                title: row.title,
                applicability_phrases,
                required_evidence_phrases,
            });
        }
        mandates
    };

    tx.execute(
        "UPDATE statutory_violations
         SET review_status = 'RESOLVED_BY_NEW_EVIDENCE',
             evaluated_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
         WHERE request_id = ?1
           AND portal_subdomain = ?2
           AND classification = ?3
           AND review_status = 'OPEN'",
        params![request_id, portal_key, FLAG_POTENTIAL_POLICY_VIOLATION],
    )
    .map_err(|error| {
        AppError::Storage(format!("Failed to retire stale policy findings: {error}"))
    })?;

    let mut applicable_mandate_count = 0_usize;
    for mandate in &mandates {
        let applicability_tokens = mandate
            .applicability_phrases
            .iter()
            .map(|phrase| policy_tokens(phrase))
            .collect::<Vec<_>>();
        let evidence_tokens = mandate
            .required_evidence_phrases
            .iter()
            .map(|phrase| policy_tokens(phrase))
            .collect::<Vec<_>>();
        let mut applicability_match_count = 0_usize;
        let mut supporting_document_ids = std::collections::BTreeSet::new();
        for source in &sources {
            let source_match_count =
                applicability_tokens
                    .iter()
                    .try_fold(0_usize, |total, phrase| {
                        total
                            .checked_add(token_sequence_match_count(&source.tokens, phrase))
                            .ok_or_else(|| {
                                AppError::Analysis(
                                    "Policy source applicability count overflowed".to_string(),
                                )
                            })
                    })?;
            applicability_match_count = applicability_match_count
                .checked_add(source_match_count)
                .ok_or_else(|| {
                AppError::Analysis("Policy applicability count overflowed".to_string())
            })?;
            if source_match_count > 0 {
                if let Some(document_id) = source.document_id {
                    supporting_document_ids.insert(document_id);
                }
            }
        }
        if applicability_match_count == 0 {
            continue;
        }
        applicable_mandate_count = applicable_mandate_count
            .checked_add(1)
            .ok_or_else(|| AppError::Analysis("Applicable mandate count overflowed".to_string()))?;
        let missing_evidence_phrases = mandate
            .required_evidence_phrases
            .iter()
            .zip(evidence_tokens.iter())
            .filter(|(_, phrase)| {
                !sources
                    .iter()
                    .any(|source| token_sequence_match_count(&source.tokens, phrase) > 0)
            })
            .map(|(phrase, _)| phrase.clone())
            .collect::<Vec<_>>();
        if missing_evidence_phrases.is_empty() {
            continue;
        }
        let identity = format!(
            "{portal_key}\0{request_id}\0{}\0{}",
            mandate.mandate_id, FLAG_POTENTIAL_POLICY_VIOLATION
        );
        let digest = sha256_hex(identity.as_bytes());
        let violation_id = format!("policy-gap-{}", &digest[..32]);
        let longest_applicability_phrase =
            applicability_tokens.iter().map(Vec::len).max().unwrap_or(0);
        let match_signal = applicability_match_count.saturating_mul(350).min(1_400);
        let specificity_signal = longest_applicability_phrase.saturating_mul(150).min(1_200);
        let confidence_basis_points = 6_200_usize
            .saturating_add(match_signal)
            .saturating_add(specificity_signal)
            .min(8_800);
        let supporting_document_ids = supporting_document_ids.into_iter().collect::<Vec<_>>();
        let supporting_json = serde_json::to_string(&supporting_document_ids).map_err(|error| {
            AppError::Storage(format!("Failed to encode supporting evidence: {error}"))
        })?;
        let missing_json = serde_json::to_string(&missing_evidence_phrases).map_err(|error| {
            AppError::Storage(format!("Failed to encode missing policy evidence: {error}"))
        })?;
        let variance_metric = format!(
            "{} of {} configured evidence phrases found across {} indexed evidence sources",
            mandate.required_evidence_phrases.len() - missing_evidence_phrases.len(),
            mandate.required_evidence_phrases.len(),
            sources.len()
        );
        let evidence_summary = format!(
            "Applicability language for '{}' matched {} times, while {} configured evidence indicators were absent from the complete indexed corpus. This is an investigative evidence gap, not a legal conclusion.",
            mandate.title,
            applicability_match_count,
            missing_evidence_phrases.len()
        );
        tx.execute(
            "INSERT INTO statutory_violations (
                violation_id, portal_subdomain, request_id, matched_statute_id,
                matched_version_id, mandate_id, classification,
                calculated_variance_metric, variance_days, confidence_basis_points,
                evidence_summary, supporting_document_ids_json, missing_evidence_json,
                classifier_version, review_status, first_detected_at, evaluated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10, ?11, ?12,
                ?13, 'OPEN', strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
                strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
             )
             ON CONFLICT(violation_id) DO UPDATE SET
                calculated_variance_metric = excluded.calculated_variance_metric,
                confidence_basis_points = excluded.confidence_basis_points,
                evidence_summary = excluded.evidence_summary,
                supporting_document_ids_json = excluded.supporting_document_ids_json,
                missing_evidence_json = excluded.missing_evidence_json,
                classifier_version = excluded.classifier_version,
                review_status = CASE
                    WHEN statutory_violations.review_status IN ('CONFIRMED', 'DISMISSED')
                    THEN statutory_violations.review_status
                    ELSE 'OPEN'
                END,
                evaluated_at = excluded.evaluated_at",
            params![
                violation_id,
                portal_key,
                request_id,
                mandate.statute_id,
                mandate.version_id,
                mandate.mandate_id,
                FLAG_POTENTIAL_POLICY_VIOLATION,
                variance_metric,
                confidence_basis_points,
                evidence_summary,
                supporting_json,
                missing_json,
                POLICY_CLASSIFIER_VERSION,
            ],
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to persist policy evidence gap: {error}"))
        })?;
    }

    synchronize_policy_audit_alert(tx, request_id, portal_key)?;

    Ok(PolicyEvaluationState {
        evaluated_mandate_count: mandates.len(),
        applicable_mandate_count,
        corpus_character_count,
        evaluation_complete: true,
        warnings: Vec::new(),
    })
}

fn synchronize_policy_audit_alert(
    tx: &Transaction<'_>,
    request_id: u64,
    portal_key: &str,
) -> Result<(), AppError> {
    let active_finding_count = tx
        .query_row(
            "SELECT COUNT(*)
             FROM statutory_violations
             WHERE request_id = ?1
               AND portal_subdomain = ?2
               AND classification = ?3
               AND review_status IN ('OPEN', 'CONFIRMED')",
            params![request_id, portal_key, FLAG_POTENTIAL_POLICY_VIOLATION],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to aggregate policy findings: {error}"))
        })?;
    let audit_id = format!("auto-{portal_key}-{request_id}");
    let audit_exists = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM case_audits WHERE audit_id = ?1)",
            params![audit_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to inspect policy audit state: {error}"))
        })?;
    if active_finding_count > 0 || audit_exists {
        upsert_automatic_case_audit(
            tx,
            &audit_id,
            request_id,
            portal_key,
            "Automatically derived from versioned policy evidence rules",
            if active_finding_count > 0 {
                VERIFICATION_STATUS_ANOMALY
            } else {
                VERIFICATION_STATUS_COMPLIANT
            },
            false,
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to synchronize policy audit: {error}"))
        })?;
        tx.execute(
            "DELETE FROM document_integrity_flags
             WHERE audit_id = ?1 AND flag_type = ?2",
            params![audit_id, FLAG_POTENTIAL_POLICY_VIOLATION],
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to clear stale policy alert: {error}"))
        })?;
        if active_finding_count > 0 {
            tx.execute(
                "INSERT INTO document_integrity_flags (audit_id, flag_type)
                 VALUES (?1, ?2)",
                params![audit_id, FLAG_POTENTIAL_POLICY_VIOLATION],
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to synchronize policy alert: {error}"))
            })?;
        }
        update_automatic_audit_status(tx, &audit_id)?;
    }
    Ok(())
}
