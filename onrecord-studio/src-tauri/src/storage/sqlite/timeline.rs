//! Request timeline audits, statutory extensions, CPRA findings, and cross-agency alerts.

use super::*;
use super::{
    connection::execute_transaction,
    integrity::{
        replace_managed_integrity_flags, update_automatic_audit_status, upsert_automatic_case_audit,
    },
};

impl DuckDbRepository {
    pub async fn record_statutory_extension(
        &self,
        request_id: u64,
        portal_key: &str,
        extension: &StatutoryExtensionRecord,
    ) -> Result<(), AppError> {
        chrono::DateTime::parse_from_rfc3339(&extension.notice_at).map_err(|error| {
            AppError::DataParsing(format!("Invalid extension notice timestamp: {error}"))
        })?;
        chrono::DateTime::parse_from_rfc3339(&extension.expected_determination_at).map_err(
            |error| {
                AppError::DataParsing(format!("Invalid expected determination timestamp: {error}"))
            },
        )?;
        if extension.notice_source.trim().is_empty() {
            return Err(AppError::DataParsing(
                "Statutory extension evidence must identify its notice source".to_string(),
            ));
        }
        let portal_key = portal_key.to_string();
        let extension = extension.clone();
        self.query_connection("Record statutory extension", move |conn| {
            upsert_statutory_extension(conn, request_id, &portal_key, &extension)
        })
        .await
    }

    /// Reads back the one extension recorded for a request, if any.
    ///
    /// The write is an upsert on `(request_id, portal_subdomain)`, so an operator who cannot see
    /// the existing row would silently replace recorded evidence. `None` here means no extension
    /// was ever recorded, which is a different statement from an extension the statute rejects:
    /// validity is [`crate::analysis`]'s verdict on a row that exists, not the absence of one.
    pub async fn load_statutory_extension(
        &self,
        request_id: u64,
        portal_key: &str,
    ) -> Result<Option<StatutoryExtensionRecord>, AppError> {
        let portal_key = portal_key.to_string();
        let row = self
            .query_connection("Load statutory extension", move |conn| {
                conn.query_row(
                    "SELECT notice_at, expected_determination_at, reason, notice_source
                 FROM statutory_extensions
                 WHERE request_id = ?1 AND portal_subdomain = ?2",
                    params![request_id, portal_key],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to read statutory extension evidence: {error}"
                    ))
                })
            })
            .await?;
        row.map(
            |(notice_at, expected_determination_at, reason, notice_source)| {
                Ok(StatutoryExtensionRecord {
                    notice_at,
                    expected_determination_at,
                    reason: parse_extension_reason(&reason)?,
                    notice_source,
                })
            },
        )
        .transpose()
    }

    pub async fn refresh_request_timeline_audit(
        &self,
        request_id: u64,
        portal_key: &str,
    ) -> Result<Vec<String>, AppError> {
        let evidence = self
            .load_legal_brief_evidence(request_id, portal_key)
            .await?;
        let operational = crate::analysis::calculate_operational_latency_for_status(
            request_id,
            &evidence.status,
            &evidence.created_at,
            evidence.closed_at.as_deref(),
        )?;
        let statutory = crate::analysis::evaluate_statutory_compliance_with_determination_at(
            request_id,
            &evidence.created_at,
            Some(&evidence.updated_at),
            evidence.closed_at.as_deref(),
            evidence.determination_at.as_deref(),
            evidence.determination_source.as_deref(),
            evidence.extension.as_ref(),
            &chrono::Utc::now().to_rfc3339(),
        )?;
        let mut flags = operational
            .anomaly_markers
            .into_iter()
            .chain(statutory.anomaly_markers.iter().copied())
            .map(|marker| marker.as_str().to_string())
            .collect::<Vec<_>>();
        let created =
            chrono::DateTime::parse_from_rfc3339(&evidence.created_at).map_err(|error| {
                AppError::DataParsing(format!("Invalid request creation timestamp: {error}"))
            })?;
        let updated =
            chrono::DateTime::parse_from_rfc3339(&evidence.updated_at).map_err(|error| {
                AppError::DataParsing(format!("Invalid request update timestamp: {error}"))
            })?;
        if updated < created {
            flags.push(FLAG_UPDATE_BEFORE_CREATION.to_string());
        }
        flags.sort();
        flags.dedup();
        let known_flags = flags
            .iter()
            .map(|flag| flag.as_str().to_string())
            .collect::<Vec<_>>();
        let portal_key_owned = portal_key.to_string();
        self.query_connection("Refresh request timeline audit", move |conn| {
            execute_transaction(conn, "timeline audit refresh", |tx| {
                let known_flags = known_flags.iter().map(String::as_str).collect::<Vec<_>>();
                synchronize_timeline_audit(
                    tx,
                    &format!("auto-{portal_key_owned}-{request_id}"),
                    request_id,
                    &portal_key_owned,
                    &known_flags,
                )?;
                synchronize_cpra_finding(tx, request_id, &portal_key_owned, &statutory)
            })
        })
        .await?;
        Ok(flags)
    }

    /// Recomputes every stored CPRA finding once after the deadline rule changes, so findings the
    /// old rule produced do not keep standing until each request happens to sync again. A request
    /// whose dates cannot be evaluated is skipped, as it would be on sync; the version is recorded
    /// only after every request was attempted, so an interrupted pass resumes on the next launch.
    pub async fn recompute_cpra_findings_for_rule_change(&self) -> Result<usize, AppError> {
        const RULE_KEY: &str = "cpra_deadline";
        let current = crate::analysis::statute::CPRA_RULE_VERSION;
        let stale_requests = self
            .query_connection("Find requests under an older CPRA rule", move |conn| {
                let recorded = conn
                    .query_row(
                        "SELECT version FROM analysis_rule_versions WHERE rule_key = ?1",
                        params![RULE_KEY],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()
                    .map_err(|error| AppError::Storage(error.to_string()))?;
                if recorded.is_some_and(|version| version >= current) {
                    return Ok(None);
                }
                let mut statement = conn
                    .prepare(
                        "SELECT id, portal_subdomain FROM requests ORDER BY portal_subdomain, id",
                    )
                    .map_err(|error| AppError::Storage(error.to_string()))?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|error| AppError::Storage(error.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| AppError::Storage(error.to_string()))?;
                Ok(Some(rows))
            })
            .await?;
        let Some(stale_requests) = stale_requests else {
            return Ok(0);
        };
        let mut recomputed = 0;
        for (request_id, portal_key) in stale_requests {
            if self
                .refresh_request_timeline_audit(request_id, &portal_key)
                .await
                .is_ok()
            {
                recomputed += 1;
            }
        }
        self.query_connection("Record the CPRA rule version", move |conn| {
            conn.execute(
                "INSERT INTO analysis_rule_versions (rule_key, version) VALUES (?1, ?2)
                 ON CONFLICT (rule_key) DO UPDATE SET version = excluded.version",
                params![RULE_KEY, current],
            )
            .map_err(|error| AppError::Storage(error.to_string()))?;
            Ok(())
        })
        .await?;
        Ok(recomputed)
    }

    pub async fn import_auxiliary_events(
        &self,
        portal_key: Option<&str>,
        source_kind: AuxiliaryStreamKind,
        display_name: &str,
        source_uri: Option<&str>,
        events: &[AuxiliaryEventInput],
        source_sha256: &str,
    ) -> Result<AuxiliaryImportResult, AppError> {
        if display_name.trim().is_empty() || events.is_empty() {
            return Err(AppError::DataParsing(
                "Auxiliary imports require a display name and at least one event".to_string(),
            ));
        }
        let identity = format!(
            "{}|{}|{}|{}",
            portal_key.unwrap_or("global"),
            source_kind.as_str(),
            source_uri.unwrap_or("local-import"),
            source_sha256
        );
        let identity_sha256 = sha256_hex(identity.as_bytes());
        let source_id = format!("aux-{}", &identity_sha256[..32]);
        let imported_at = chrono::Utc::now().to_rfc3339();
        let mut accepted = Vec::with_capacity(events.len());
        let mut rejected_count = 0_usize;
        for event in events {
            if validate_auxiliary_event(event).is_ok() {
                accepted.push(event.clone());
            } else {
                rejected_count += 1;
            }
        }
        if accepted.is_empty() {
            return Err(AppError::DataParsing(
                "Every auxiliary event failed structural validation".to_string(),
            ));
        }

        let portal_key = portal_key.map(str::to_string);
        let display_name = display_name.to_string();
        let source_uri = source_uri.map(str::to_string);
        let source_sha256 = source_sha256.to_string();
        self.query_connection("Import auxiliary events", move |conn| {
            execute_transaction(conn, "auxiliary import", |tx| {
                tx.execute(
                    "INSERT INTO auxiliary_data_sources (
                source_id, portal_subdomain, source_kind, display_name, source_uri,
                source_sha256, imported_at, record_count
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(source_id) DO UPDATE SET
                display_name = excluded.display_name,
                source_uri = excluded.source_uri,
                imported_at = excluded.imported_at,
                record_count = excluded.record_count",
                    params![
                        source_id,
                        portal_key,
                        source_kind.as_str(),
                        display_name.trim(),
                        source_uri,
                        source_sha256,
                        imported_at,
                        accepted.len(),
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to persist auxiliary source: {error}"))
                })?;

                let mut imported_count = 0_usize;
                let mut updated_count = 0_usize;
                for event in accepted {
                    let existed = tx
                        .query_row(
                            "SELECT EXISTS(
                        SELECT 1 FROM auxiliary_events WHERE source_id = ?1 AND external_id = ?2
                     )",
                            params![source_id, event.external_id],
                            |row| row.get::<_, bool>(0),
                        )
                        .map_err(|error| {
                            AppError::Storage(format!("Failed to inspect auxiliary event: {error}"))
                        })?;
                    let source_payload =
                        serde_json::to_string(&event.source_payload).map_err(|error| {
                            AppError::DataParsing(format!(
                                "Failed to encode auxiliary payload: {error}"
                            ))
                        })?;
                    tx.execute(
                        "INSERT INTO auxiliary_events (
                    source_id, external_id, occurred_at, neighborhood_block, event_type,
                    description, latitude, longitude, source_payload_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(source_id, external_id) DO UPDATE SET
                    occurred_at = excluded.occurred_at,
                    neighborhood_block = excluded.neighborhood_block,
                    event_type = excluded.event_type,
                    description = excluded.description,
                    latitude = excluded.latitude,
                    longitude = excluded.longitude,
                    source_payload_json = excluded.source_payload_json,
                    imported_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')",
                        params![
                            source_id,
                            event.external_id,
                            event.occurred_at,
                            normalize_neighborhood(&event.neighborhood_block),
                            event.event_type.trim(),
                            event.description,
                            event.latitude,
                            event.longitude,
                            source_payload,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to persist auxiliary event: {error}"))
                    })?;
                    if existed {
                        updated_count += 1;
                    } else {
                        imported_count += 1;
                    }
                }
                Ok(AuxiliaryImportResult {
                    source_id,
                    imported_count,
                    updated_count,
                    rejected_count,
                    source_sha256: source_sha256.to_string(),
                })
            })
        })
        .await
    }

    /// Records the correlations one evaluation produced over `scope_portal_key` (every portal
    /// when `None`).
    ///
    /// When `evaluation_complete`, the set is the whole answer for that scope, so an alert it no
    /// longer contains is withdrawn along with its audit flag: a request that correlated only under
    /// looser settings is not still correlated. A truncated evaluation withdraws nothing, because
    /// an absent request may simply have been cut off by the limit.
    pub async fn synchronize_cross_correlation_alerts(
        &self,
        scope_portal_key: Option<&str>,
        evaluation_complete: bool,
        correlations: &[CrossAgencyCorrelation],
    ) -> Result<(), AppError> {
        if correlations.is_empty() && !evaluation_complete {
            return Ok(());
        }
        let mut identities = std::collections::BTreeSet::new();
        for correlation in correlations {
            if !identities.insert((correlation.portal_key.clone(), correlation.request_id)) {
                return Err(AppError::Analysis(format!(
                    "Duplicate correlation identity {}/{}",
                    correlation.portal_key, correlation.request_id
                )));
            }
            if !correlation.correlation_density.is_finite() || correlation.correlation_density < 0.0
            {
                return Err(AppError::Analysis(
                    "Correlation density must be finite and non-negative".to_string(),
                ));
            }
        }
        let evaluated_at = chrono::Utc::now().to_rfc3339();
        let correlations = correlations.to_vec();
        let scope_portal_key = scope_portal_key.map(str::to_string);
        self.query_connection("Synchronize cross correlation alerts", move |conn| {
        execute_transaction(conn, "correlation alert synchronization", |tx| {
            if evaluation_complete {
                withdraw_uncorrelated_alerts(tx, scope_portal_key.as_deref(), &identities)?;
            }
            for correlation in &correlations {
                tx.execute(
                    "INSERT INTO cross_correlation_alerts (
                    request_id, portal_subdomain, neighborhood_block, complaint_count,
                    correlation_window_hours, distinct_source_count, keyword_signal_count, correlation_density,
                    first_correlated_event_at, last_correlated_event_at, evaluated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(request_id, portal_subdomain) DO UPDATE SET
                    neighborhood_block = excluded.neighborhood_block,
                    complaint_count = excluded.complaint_count,
                    correlation_window_hours = excluded.correlation_window_hours,
                    distinct_source_count = excluded.distinct_source_count,
                    keyword_signal_count = excluded.keyword_signal_count,
                    correlation_density = excluded.correlation_density,
                    first_correlated_event_at = excluded.first_correlated_event_at,
                    last_correlated_event_at = excluded.last_correlated_event_at,
                    evaluated_at = excluded.evaluated_at",
                    params![
                        correlation.request_id,
                        correlation.portal_key,
                        correlation.neighborhood_block,
                        correlation.complaint_count,
                        correlation.correlation_window_hours,
                        correlation.distinct_source_count,
                        correlation.keyword_signal_count,
                        correlation.correlation_density,
                        correlation.first_correlated_event_at,
                        correlation.last_correlated_event_at,
                        evaluated_at,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to persist correlation alert: {error}"))
                })?;
                let audit_id =
                    format!("auto-{}-{}", correlation.portal_key, correlation.request_id);
                tx.execute(
                    "INSERT INTO case_audits (
                    audit_id, target_request_id, target_subdomain, auditor_notes,
                    verified_at, verification_status
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(audit_id) DO UPDATE SET verified_at = excluded.verified_at",
                    params![
                        audit_id,
                        correlation.request_id,
                        correlation.portal_key,
                        "Vectorized 311/enforcement correlation evidence",
                        evaluated_at,
                        VERIFICATION_STATUS_ANOMALY,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to synchronize correlation audit: {error}"))
                })?;
                tx.execute(
                    "INSERT INTO document_integrity_flags (audit_id, flag_type)
                 SELECT ?1, 'CROSS_AGENCY_311_CORRELATION'
                 WHERE NOT EXISTS (
                    SELECT 1 FROM document_integrity_flags
                    WHERE audit_id = ?1 AND flag_type = 'CROSS_AGENCY_311_CORRELATION'
                 )",
                    params![audit_id],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to synchronize correlation flag: {error}"))
                })?;
                // An existing audit keeps the status its earlier writer concluded, so a flag added
                // here has to re-derive it or the audit goes on reading as compliant.
                update_automatic_audit_status(tx, &audit_id)?;
            }
            Ok(())
        })
        })
        .await
    }
}

fn withdraw_uncorrelated_alerts(
    tx: &Transaction<'_>,
    scope_portal_key: Option<&str>,
    still_correlated: &std::collections::BTreeSet<(String, u64)>,
) -> Result<(), AppError> {
    let stored = {
        let mut statement = tx
            .prepare(
                "SELECT portal_subdomain, request_id FROM cross_correlation_alerts
                 WHERE ?1 IS NULL OR portal_subdomain = ?1",
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to read correlation alerts: {error}"))
            })?;
        statement
            .query_map(params![scope_portal_key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .and_then(|rows| rows.collect::<duckdb::Result<Vec<_>>>())
            .map_err(|error| {
                AppError::Storage(format!("Failed to read correlation alerts: {error}"))
            })?
    };
    for (portal_key, request_id) in stored {
        let correlated_id = u64::try_from(request_id).map_err(|error| {
            AppError::Storage(format!(
                "A stored correlation alert has an invalid request id: {error}"
            ))
        })?;
        if still_correlated.contains(&(portal_key.clone(), correlated_id)) {
            continue;
        }
        tx.execute(
            "DELETE FROM cross_correlation_alerts
             WHERE portal_subdomain = ?1 AND request_id = ?2",
            params![portal_key, request_id],
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to withdraw a correlation alert: {error}"))
        })?;
        let audit_id = format!("auto-{portal_key}-{request_id}");
        tx.execute(
            "DELETE FROM document_integrity_flags
             WHERE audit_id = ?1 AND flag_type = 'CROSS_AGENCY_311_CORRELATION'",
            params![audit_id],
        )
        .map_err(|error| {
            AppError::Storage(format!("Failed to withdraw a correlation flag: {error}"))
        })?;
        let audit_exists = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM case_audits WHERE audit_id = ?1)",
                params![audit_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to inspect a correlation audit: {error}"))
            })?;
        if audit_exists {
            update_automatic_audit_status(tx, &audit_id)?;
        }
    }
    Ok(())
}

pub(super) fn upsert_statutory_extension(
    connection: &Connection,
    request_id: u64,
    portal_key: &str,
    extension: &StatutoryExtensionRecord,
) -> Result<(), AppError> {
    connection
        .execute(
            "INSERT INTO statutory_extensions (
                request_id, portal_subdomain, notice_at, expected_determination_at,
                reason, notice_source
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(request_id, portal_subdomain) DO UPDATE SET
                notice_at = excluded.notice_at,
                expected_determination_at = excluded.expected_determination_at,
                reason = excluded.reason,
                notice_source = excluded.notice_source,
                recorded_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')",
            params![
                request_id,
                portal_key,
                extension.notice_at,
                extension.expected_determination_at,
                extension.reason.as_str(),
                extension.notice_source,
            ],
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to persist statutory extension evidence: {error}"
            ))
        })?;
    Ok(())
}

pub(super) fn map_request_analytical_state(
    row: &Row<'_>,
) -> duckdb::Result<RequestAnalyticalState> {
    Ok(RequestAnalyticalState {
        request_id: row.get(0)?,
        created_at: row.get(1)?,
        updated_at: row.get(2)?,
        closed_at: row.get(3)?,
        status: row.get(4)?,
        file_name: row.get(5)?,
    })
}

/// Grades a CPRA calendar finding by what the evaluation actually observed.
///
/// The finding rests on two dates. Receipt is always recorded. The determination is a sourced
/// determination event, a generic portal-activity proxy, or the observation instant. Where an
/// extension moved the deadline, its notice is either recorded or claimed without a source.
///
/// ```text
/// confidence = 10000 - uncertainty + margin_relief
/// uncertainty   = determination_penalty + extension_penalty
/// margin_relief = uncertainty / 2 * min(variance_days, 30) / 30
/// ```
///
/// `10000` is therefore reserved for the case where every date input was recorded: no penalty,
/// no relief to apply. `margin_relief` is capped at half the uncertainty so it can never reach
/// it, which keeps every other case strictly below 10000. Its shape reflects that a one-day
/// overrun turns on the exact instants while a month-long one survives any plausible correction
/// to them, so a long overrun recovers most of what the unobserved input cost.
pub(super) fn cpra_calendar_confidence_basis_points(
    analysis: &crate::analysis::StatutoryComplianceAnalysis,
    variance_days: i64,
) -> i64 {
    use crate::analysis::StatutoryExtensionStatus;

    let determination_penalty = match analysis.determination_basis {
        crate::analysis::DeterminationBasis::SourcedDeterminationEvent => 0,
        crate::analysis::DeterminationBasis::EarliestRecordedActivityProxy => 1_000,
        crate::analysis::DeterminationBasis::ObservationWithoutRecordedActivity => 2_500,
    };
    let extension_penalty = match analysis.extension_status {
        // No extension date entered the evaluation, or the notice behind it was recorded.
        StatutoryExtensionStatus::NotInvoked | StatutoryExtensionStatus::Valid => 0,
        // An extension was claimed and its notice was never recorded, so the deadline the
        // overrun is measured against rests on an input nothing observed.
        StatutoryExtensionStatus::MissingNoticeSource => 1_200,
        // The notice was recorded but contradicts the record around it. The dates exist; which
        // deadline they establish is contested.
        StatutoryExtensionStatus::NoticeBeforeReceipt
        | StatutoryExtensionStatus::NoticeAfterInitialDeadline
        | StatutoryExtensionStatus::ExpectedDateNotExtended
        | StatutoryExtensionStatus::ExtensionExceedsFourteenDays => 600,
    };
    let uncertainty = determination_penalty + extension_penalty;
    let margin_days = variance_days.clamp(0, 30);
    let margin_relief = uncertainty / 2 * margin_days / 30;
    10_000 - uncertainty + margin_relief
}

pub(super) fn synchronize_cpra_finding(
    tx: &Transaction<'_>,
    request_id: u64,
    portal_key: &str,
    analysis: &crate::analysis::StatutoryComplianceAnalysis,
) -> Result<(), AppError> {
    let version_id = tx
        .query_row(
            "SELECT current_version_id
             FROM legal_statutes
             WHERE statute_id = 'CPRA_7922.535'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to resolve the CPRA authority version: {error}"
            ))
        })?;
    let identity = format!(
        "{portal_key}\0{request_id}\0CPRA_7922.535\0{}",
        FLAG_STATUTORY_LATENCY_VIOLATION
    );
    let digest = sha256_hex(identity.as_bytes());
    let violation_id = format!("cpra-latency-{}", &digest[..32]);
    if !analysis.is_violation {
        tx.execute(
            "UPDATE statutory_violations
             SET review_status = 'RESOLVED_BY_NEW_EVIDENCE',
                 evaluated_at = strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
             WHERE violation_id = ?1 AND review_status = 'OPEN'",
            params![violation_id],
        )
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to resolve the CPRA timeline finding: {error}"
            ))
        })?;
        return Ok(());
    }
    let variance_days = analysis
        .calendar_days_elapsed
        .checked_sub(analysis.allowed_calendar_days)
        .ok_or_else(|| AppError::Compliance("CPRA deadline variance overflowed".to_string()))?;
    let variance_metric =
        format!("Exceeded the effective determination deadline by {variance_days} calendar days");
    tx.execute(
        "INSERT INTO statutory_violations (
            violation_id, portal_subdomain, request_id, matched_statute_id,
            matched_version_id, mandate_id, classification,
            calculated_variance_metric, variance_days, confidence_basis_points,
            evidence_summary, supporting_document_ids_json, missing_evidence_json,
            classifier_version, review_status, first_detected_at, evaluated_at
         ) VALUES (
            ?1, ?2, ?3, 'CPRA_7922.535', ?4, NULL, ?5, ?6, ?7, ?9,
            ?8, '[]', '[]', 'cpra-calendar-v1', 'OPEN',
            strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ'),
            strftime(now(), '%Y-%m-%dT%H:%M:%S.%fZ')
         )
         ON CONFLICT(violation_id) DO UPDATE SET
            matched_version_id = excluded.matched_version_id,
            calculated_variance_metric = excluded.calculated_variance_metric,
            variance_days = excluded.variance_days,
            confidence_basis_points = excluded.confidence_basis_points,
            evidence_summary = excluded.evidence_summary,
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
            version_id,
            FLAG_STATUTORY_LATENCY_VIOLATION,
            variance_metric,
            variance_days,
            analysis.evidence_summary,
            cpra_calendar_confidence_basis_points(analysis, variance_days),
        ],
    )
    .map_err(|error| {
        AppError::Storage(format!(
            "Failed to persist the CPRA timeline finding: {error}"
        ))
    })?;
    Ok(())
}

pub(super) fn synchronize_timeline_audit(
    tx: &Transaction<'_>,
    audit_id: &str,
    request_id: u64,
    subdomain: &str,
    timeline_flags: &[&str],
) -> Result<(), AppError> {
    let audit_exists = tx
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM case_audits WHERE audit_id = ?1
             )",
            params![audit_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| {
            AppError::Internal(format!(
                "Failed to inspect the automatic timeline audit state: {error}"
            ))
        })?;

    if timeline_flags.is_empty() && !audit_exists {
        return Ok(());
    }

    upsert_automatic_case_audit(
        tx,
        audit_id,
        request_id,
        subdomain,
        "Automatically derived during request timeline synchronization",
        if timeline_flags.contains(&FLAG_STATUTORY_LATENCY_VIOLATION) {
            VERIFICATION_STATUS_STATUTORY
        } else if timeline_flags.is_empty() {
            VERIFICATION_STATUS_COMPLIANT
        } else {
            VERIFICATION_STATUS_ANOMALY
        },
        false,
    )
    .map_err(|error| {
        AppError::Internal(format!(
            "Failed to synchronize the automatic timeline audit: {error}"
        ))
    })?;

    replace_managed_integrity_flags(tx, audit_id, &DERIVED_TIMELINE_FLAG_TYPES, timeline_flags)?;

    update_automatic_audit_status(tx, audit_id)
}

fn validate_auxiliary_event(event: &AuxiliaryEventInput) -> Result<(), AppError> {
    if event.external_id.trim().is_empty()
        || event.external_id.len() > 256
        || event.event_type.trim().is_empty()
        || event.neighborhood_block.trim().is_empty()
    {
        return Err(AppError::DataParsing(
            "Auxiliary event identifiers, type, and neighborhood are required".to_string(),
        ));
    }
    chrono::DateTime::parse_from_rfc3339(&event.occurred_at).map_err(|error| {
        AppError::DataParsing(format!("Auxiliary event timestamp is invalid: {error}"))
    })?;
    if event
        .latitude
        .is_some_and(|value| !value.is_finite() || !(-90.0..=90.0).contains(&value))
        || event
            .longitude
            .is_some_and(|value| !value.is_finite() || !(-180.0..=180.0).contains(&value))
    {
        return Err(AppError::DataParsing(
            "Auxiliary event coordinates are outside valid bounds".to_string(),
        ));
    }
    Ok(())
}

fn normalize_neighborhood(value: &str) -> String {
    collapse_whitespace(value)
}

pub(super) fn parse_extension_reason(value: &str) -> Result<StatutoryExtensionReason, AppError> {
    match value {
        "SEPARATE_FIELD_FACILITIES" => Ok(StatutoryExtensionReason::SeparateFieldFacilities),
        "VOLUMINOUS_RECORDS" => Ok(StatutoryExtensionReason::VoluminousRecords),
        "INTERAGENCY_CONSULTATION" => Ok(StatutoryExtensionReason::InteragencyConsultation),
        "DATA_COMPILATION" => Ok(StatutoryExtensionReason::DataCompilation),
        "CYBERATTACK_SYSTEMS_UNAVAILABLE" => {
            Ok(StatutoryExtensionReason::CyberattackSystemsUnavailable)
        }
        "DECLARED_EMERGENCY_OPERATIONAL_IMPACT" => {
            Ok(StatutoryExtensionReason::DeclaredEmergencyOperationalImpact)
        }
        other => Err(AppError::DataParsing(format!(
            "Unknown statutory extension reason '{other}'"
        ))),
    }
}
