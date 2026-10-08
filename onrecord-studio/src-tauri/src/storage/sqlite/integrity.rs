//! Digest validation, derived integrity flags, and the automatic case audits they feed.

use super::*;

pub(super) fn replace_managed_integrity_flags(
    tx: &Transaction<'_>,
    audit_id: &str,
    managed_flags: &[&str],
    replacement_flags: &[&str],
) -> Result<(), AppError> {
    if managed_flags.is_empty()
        || replacement_flags
            .iter()
            .any(|flag| !managed_flags.contains(flag))
    {
        return Err(AppError::Internal(
            "Integrity-flag synchronization received an invalid managed set".to_string(),
        ));
    }
    let placeholders = (2..=managed_flags.len() + 1)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let delete_sql = format!(
        "DELETE FROM document_integrity_flags \
         WHERE audit_id = ?1 AND flag_type IN ({placeholders})"
    );
    tx.execute(
        &delete_sql,
        params_from_iter(std::iter::once(audit_id).chain(managed_flags.iter().copied())),
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "Failed to clear stale managed integrity flags: {error}"
        ))
    })?;

    let mut insert = tx
        .prepare_cached(
            "INSERT INTO document_integrity_flags (audit_id, flag_type) VALUES (?1, ?2)",
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to prepare integrity-flag synchronization: {error}"
            ))
        })?;
    for flag in replacement_flags {
        insert.execute(params![audit_id, flag]).map_err(|error| {
            AppError::Internal(format!(
                "Failed to synchronize managed integrity flag '{flag}': {error}"
            ))
        })?;
    }
    Ok(())
}

pub(super) fn update_automatic_audit_status(
    tx: &Transaction<'_>,
    audit_id: &str,
) -> Result<(), AppError> {
    let updated = tx
        .execute(
            "UPDATE case_audits
             SET
                verified_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
                verification_status = CASE
                    WHEN EXISTS (
                        SELECT 1
                        FROM document_integrity_flags
                        WHERE document_integrity_flags.audit_id = case_audits.audit_id
                          AND document_integrity_flags.flag_type = ?4
                    ) THEN ?5
                    WHEN EXISTS (
                        SELECT 1
                        FROM document_integrity_flags
                        WHERE document_integrity_flags.audit_id = case_audits.audit_id
                    ) THEN ?2
                    ELSE ?3
                END
             WHERE audit_id = ?1",
            params![
                audit_id,
                VERIFICATION_STATUS_ANOMALY,
                VERIFICATION_STATUS_COMPLIANT,
                FLAG_STATUTORY_LATENCY_VIOLATION,
                VERIFICATION_STATUS_STATUTORY,
            ],
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to refresh the automatic audit verification status: {error}"
            ))
        })?;

    if updated != 1 {
        return Err(AppError::Internal(format!(
            "Automatic audit status refresh affected {updated} rows for audit '{audit_id}'"
        )));
    }

    Ok(())
}

pub(super) fn normalize_sha256(sha256_hash: &str) -> Result<String, AppError> {
    let trimmed_hash = sha256_hash.trim();
    if !crate::integrity::is_sha256_hex(trimmed_hash) {
        return Err(AppError::DataParsing(
            "Document SHA-256 signature must contain exactly 64 hexadecimal characters".to_string(),
        ));
    }

    Ok(trimmed_hash.to_ascii_lowercase())
}

pub(super) fn validate_extracted_text_sha256(
    extracted_text: &str,
    supplied_sha256: &str,
) -> Result<String, AppError> {
    let normalized_sha256 = normalize_sha256(supplied_sha256)?;
    let computed_sha256 = compute_extracted_text_sha256(extracted_text);
    if normalized_sha256 != computed_sha256 {
        return Err(AppError::DataParsing(format!(
            "Extracted text SHA-256 mismatch: expected {computed_sha256}, received {normalized_sha256}"
        )));
    }

    Ok(normalized_sha256)
}

pub(super) fn compute_extracted_text_sha256(extracted_text: &str) -> String {
    sha256_hex(extracted_text.as_bytes())
}

pub(super) fn derive_integrity_flags(
    analytical_state: &RequestAnalyticalState,
    request_has_heavy_redaction: bool,
    request_has_suspicious_data_density: bool,
) -> Vec<&'static str> {
    let mut flags = Vec::with_capacity(DERIVED_DOCUMENT_FLAG_TYPES.len());
    let created_at = chrono::DateTime::parse_from_rfc3339(&analytical_state.created_at).ok();
    let updated_at = chrono::DateTime::parse_from_rfc3339(&analytical_state.updated_at).ok();
    let closed_at = analytical_state
        .closed_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());

    if matches!((&created_at, &updated_at), (Some(created), Some(updated)) if updated < created) {
        flags.push(FLAG_UPDATE_BEFORE_CREATION);
    }

    if let (Some(created), Some(closed)) = (&created_at, &closed_at) {
        let lifespan = closed.signed_duration_since(*created);
        if lifespan < chrono::TimeDelta::zero() {
            flags.push(FLAG_CLOSE_BEFORE_CREATION);
        } else if lifespan <= chrono::TimeDelta::seconds(SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS)
            && analytical_state.status.eq_ignore_ascii_case("closed")
        {
            flags.push(FLAG_SUSPICIOUSLY_FAST_CLOSURE);
        }
    }

    if request_has_heavy_redaction {
        flags.push(FLAG_HEAVY_REDACTION_DENSITY);
    }
    if request_has_suspicious_data_density {
        flags.push(FLAG_SUSPICIOUS_DATA_DENSITY);
    }

    flags
}

pub(super) fn validate_entropy_metadata(
    shannon_entropy: Option<f64>,
    entropy_sample_bytes: u64,
    raw_file_size: u64,
    suspicious_data_density: bool,
) -> Result<(i64, i64), AppError> {
    match shannon_entropy {
        Some(score) => {
            if !score.is_finite() || !(0.0..=8.0).contains(&score) {
                return Err(AppError::DataParsing(format!(
                    "Shannon entropy must be finite and between 0.0 and 8.0, received {score}"
                )));
            }
            if entropy_sample_bytes == 0 && score != 0.0 {
                return Err(AppError::DataParsing(
                    "A zero-byte entropy sample must have an entropy score of 0.0".to_string(),
                ));
            }
        }
        None => {
            if entropy_sample_bytes != 0 {
                return Err(AppError::DataParsing(format!(
                    "Entropy was sampled over {entropy_sample_bytes} bytes but no score was \
                     computed; absence and measurement cannot be mixed"
                )));
            }
        }
    }
    if entropy_sample_bytes > raw_file_size {
        return Err(AppError::DataParsing(format!(
            "Entropy sample size {entropy_sample_bytes} exceeds raw file size {raw_file_size}"
        )));
    }
    let entropy_sample_bytes = i64::try_from(entropy_sample_bytes).map_err(|error| {
        AppError::DataParsing(format!("Entropy sample size cannot be persisted: {error}"))
    })?;
    Ok((
        entropy_sample_bytes,
        if suspicious_data_density { 1 } else { 0 },
    ))
}

pub(super) fn request_has_suspicious_density(
    tx: &Transaction<'_>,
    request_id: i64,
    subdomain: &str,
) -> Result<bool, AppError> {
    tx.query_row(
        "SELECT EXISTS(
            SELECT 1
            FROM documents
            WHERE request_id = ?1
              AND portal_subdomain = ?2
              AND suspicious_data_density = 1
         )",
        params![request_id, subdomain],
        |row| row.get::<_, bool>(0),
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "Failed to aggregate request entropy state: {error}"
        ))
    })
}

/// Whether any indexed document of the request, other than `excluding`, is heavily redacted.
pub(super) fn request_has_heavily_redacted_document(
    tx: &Transaction<'_>,
    request_id: i64,
    subdomain: &str,
    excluding: Option<i64>,
) -> Result<bool, AppError> {
    let mut statement = tx
        .prepare(
            "SELECT documents_fts.extracted_text
             FROM documents
             INNER JOIN documents_fts
                ON documents_fts.document_id = documents.id
               AND documents_fts.portal_subdomain = documents.portal_subdomain
             WHERE documents.request_id = ?1
               AND documents.portal_subdomain = ?2
               AND documents.id IS DISTINCT FROM ?3",
        )
        .map_err(|e| {
            AppError::Internal(format!("Failed to prepare request redaction scan: {e}"))
        })?;
    let indexed_text_rows = statement
        .query_map(params![request_id, subdomain, excluding], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|e| {
            AppError::Internal(format!("Failed to execute request redaction scan: {e}"))
        })?;
    for indexed_text in indexed_text_rows {
        let indexed_text = indexed_text.map_err(|e| {
            AppError::Internal(format!(
                "Failed to decode indexed text during redaction scan: {e}"
            ))
        })?;
        if has_heavy_redaction_density(&indexed_text)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `EXISTS` is BOOLEAN in DuckDB; decoding it as an integer fails the whole transaction.
pub(super) fn automatic_audit_exists(tx: &Transaction<'_>, audit_id: &str) -> duckdb::Result<bool> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM case_audits WHERE audit_id = ?1)",
        params![audit_id],
        |row| row.get::<_, bool>(0),
    )
}

/// Re-derives the request-level density flags after a document leaves the request, so the
/// automatic audit stops asserting an anomaly that only the removed document carried.
pub(super) fn resynchronize_request_density_flags(
    tx: &Transaction<'_>,
    request_id: i64,
    subdomain: &str,
) -> Result<(), AppError> {
    let audit_id = format!("auto-{subdomain}-{request_id}");
    let audit_exists = automatic_audit_exists(tx, &audit_id).map_err(|error| {
        AppError::Internal(format!("Failed to inspect the request audit: {error}"))
    })?;
    if !audit_exists {
        return Ok(());
    }
    let mut flags = Vec::with_capacity(2);
    if request_has_heavily_redacted_document(tx, request_id, subdomain, None)? {
        flags.push(FLAG_HEAVY_REDACTION_DENSITY);
    }
    if request_has_suspicious_density(tx, request_id, subdomain)? {
        flags.push(FLAG_SUSPICIOUS_DATA_DENSITY);
    }
    replace_managed_integrity_flags(
        tx,
        &audit_id,
        &[FLAG_HEAVY_REDACTION_DENSITY, FLAG_SUSPICIOUS_DATA_DENSITY],
        &flags,
    )?;
    update_automatic_audit_status(tx, &audit_id)
}

pub(super) fn upsert_automatic_case_audit<T: ToSql>(
    tx: &Transaction<'_>,
    audit_id: &str,
    request_id: T,
    subdomain: &str,
    notes: &str,
    verification_status: &str,
    replace_conclusion: bool,
) -> duckdb::Result<()> {
    if automatic_audit_exists(tx, audit_id)? {
        tx.execute(
            "UPDATE case_audits
             SET target_request_id = ?2,
                 target_subdomain = ?3,
                 auditor_notes = CASE WHEN ?6 = 1 THEN ?4 ELSE auditor_notes END,
                 verified_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
                 verification_status = CASE WHEN ?6 = 1 THEN ?5 ELSE verification_status END
             WHERE audit_id = ?1",
            params![
                audit_id,
                request_id,
                subdomain,
                notes,
                verification_status,
                if replace_conclusion { 1_i64 } else { 0_i64 }
            ],
        )?;
    } else {
        tx.execute(
            "INSERT INTO case_audits (
                audit_id, target_request_id, target_subdomain, auditor_notes,
                verified_at, verification_status
             ) VALUES (?1, ?2, ?3, ?4, strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'), ?5)",
            params![audit_id, request_id, subdomain, notes, verification_status],
        )?;
    }
    Ok(())
}

/// Makes `flags` the complete set of `managed` flags on a request's automatic audit, then
/// re-derives the audit's status from every flag it carries, including those other writers
/// manage. The audit is created only when there is a flag to record; an existing one keeps its
/// notes.
pub(super) fn synchronize_automatic_audit_flags<T: ToSql>(
    tx: &Transaction<'_>,
    audit_id: &str,
    request_id: T,
    subdomain: &str,
    notes: &str,
    managed: &[&str],
    flags: &[&str],
) -> Result<(), AppError> {
    let failed = |error: duckdb::Error| {
        AppError::Internal(format!(
            "Failed to synchronize automatic audit '{audit_id}': {error}"
        ))
    };
    if flags.is_empty() && !automatic_audit_exists(tx, audit_id).map_err(failed)? {
        return Ok(());
    }
    // A new audit's status is provisional; `update_automatic_audit_status` settles it below.
    upsert_automatic_case_audit(
        tx,
        audit_id,
        request_id,
        subdomain,
        notes,
        VERIFICATION_STATUS_COMPLIANT,
        false,
    )
    .map_err(failed)?;
    replace_managed_integrity_flags(tx, audit_id, managed, flags)?;
    update_automatic_audit_status(tx, audit_id)
}

pub(super) fn synchronize_density_audit(
    tx: &Transaction<'_>,
    document_id: i64,
    subdomain: &str,
) -> Result<(), AppError> {
    let request_id = tx
        .query_row(
            "SELECT request_id
             FROM documents
             WHERE id = ?1 AND portal_subdomain = ?2",
            params![document_id, subdomain],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to load document request for entropy synchronization: {error}"
            ))
        })?;
    let flags: &[&str] = if request_has_suspicious_density(tx, request_id, subdomain)? {
        &[FLAG_SUSPICIOUS_DATA_DENSITY]
    } else {
        &[]
    };
    synchronize_automatic_audit_flags(
        tx,
        &format!("auto-{subdomain}-{request_id}"),
        request_id,
        subdomain,
        "Automatically derived during document entropy synchronization",
        &[FLAG_SUSPICIOUS_DATA_DENSITY],
        flags,
    )
}

pub(super) fn has_heavy_redaction_density(extracted_text: &str) -> Result<bool, AppError> {
    Ok(analyze_redaction_density(extracted_text)?
        .alert_states
        .contains(&ClusterAlertState::HeavyRedactionDensity))
}
