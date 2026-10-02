//! Re-fetch observations of collected documents. The collected row is never repointed: citations
//! and extracted text belong to the bytes that were collected, so a later fetch is recorded beside
//! them, as a check and (when bytes arrived) an acquisition.

use super::connection::execute_transaction;
use super::documents::insert_document_acquisition;
use super::integrity::normalize_sha256;
use super::*;
use crate::ingestion::client::AcquisitionRecord;
use crate::types::{SourceCheck, SourceCheckOutcome};

const MAXIMUM_DETAIL_CHARACTERS: usize = 2_048;
const MAXIMUM_LISTED_CHECKS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRecheckTarget {
    pub document_id: u64,
    pub download_url: String,
    pub collected_sha256: String,
}

/// What one re-fetch produced, before it is compared with what was collected.
#[derive(Debug, Clone)]
pub enum SourceCheckObservation {
    Received {
        sha256: String,
        byte_count: u64,
        acquisition: AcquisitionRecord,
    },
    /// The source answered 404 or 410.
    Gone { acquisition: AcquisitionRecord },
    /// No answer about the record: network failure, server error, refusal, or a local limit.
    Unreachable { detail: String },
}

pub(crate) fn classify(
    collected_sha256: &str,
    observation: &SourceCheckObservation,
) -> SourceCheckOutcome {
    match observation {
        SourceCheckObservation::Received { sha256, .. } if sha256 == collected_sha256 => {
            SourceCheckOutcome::SameBytes
        }
        SourceCheckObservation::Received { .. } => SourceCheckOutcome::DifferentBytes,
        SourceCheckObservation::Gone { .. } => SourceCheckOutcome::Gone,
        SourceCheckObservation::Unreachable { .. } => SourceCheckOutcome::Unreachable,
    }
}

impl DuckDbRepository {
    /// Collected documents of one source that have a download URL, least recently checked first,
    /// so repeated capped runs rotate through the whole source.
    pub async fn source_recheck_targets(
        &self,
        portal_key: &str,
        limit: usize,
    ) -> Result<(Vec<SourceRecheckTarget>, u64), AppError> {
        let portal_key = portal_key.to_string();
        let limit = i64::try_from(limit)
            .map_err(|_| AppError::DataParsing("Recheck limit exceeds capacity".to_string()))?;
        self.query_connection("Source recheck targets", move |conn| {
            let eligible = "FROM documents
                 WHERE documents.portal_subdomain = ?1
                   AND documents.download_status = ?2
                   AND NULLIF(TRIM(documents.download_url), '') IS NOT NULL
                   AND documents.raw_file_sha256 IS NOT NULL";
            let total: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) {eligible}"),
                    params![portal_key, DOWNLOAD_STATUS_COMPLETED],
                    |row| row.get(0),
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to count recheck targets: {error}"))
                })?;
            let mut statement = conn
                .prepare(&format!(
                    "SELECT documents.id, documents.download_url, documents.raw_file_sha256
                     {eligible}
                     ORDER BY (SELECT MAX(checks.checked_at) FROM document_source_checks checks
                               WHERE checks.portal_subdomain = documents.portal_subdomain
                                 AND checks.document_id = documents.id) NULLS FIRST,
                              documents.id
                     LIMIT ?3"
                ))
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare recheck targets: {error}"))
                })?;
            let targets = statement
                .query_map(
                    params![portal_key, DOWNLOAD_STATUS_COMPLETED, limit],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read recheck targets: {error}"))
                })?
                .map(|row| {
                    let (id, download_url, collected_sha256) = row.map_err(|error| {
                        AppError::Storage(format!("Failed to decode a recheck target: {error}"))
                    })?;
                    Ok(SourceRecheckTarget {
                        document_id: u64::try_from(id).map_err(|_| {
                            AppError::Storage("Document identifier was negative".to_string())
                        })?,
                        download_url,
                        collected_sha256,
                    })
                })
                .collect::<Result<Vec<_>, AppError>>()?;
            let total = u64::try_from(total)
                .map_err(|_| AppError::Storage("Recheck target count was negative".to_string()))?;
            Ok((targets, total))
        })
        .await
    }

    /// Records one re-fetch. When bytes arrived, the fetch is also appended to the acquisition
    /// history in the same transaction, so a check never points at a fetch that was not recorded.
    pub async fn record_source_check(
        &self,
        run_id: &str,
        portal_key: &str,
        target: &SourceRecheckTarget,
        observation: SourceCheckObservation,
    ) -> Result<SourceCheckOutcome, AppError> {
        let collected_sha256 = normalize_sha256(&target.collected_sha256)?;
        let outcome = classify(&collected_sha256, &observation);
        let document_id = i64::try_from(target.document_id).map_err(|_| {
            AppError::DataParsing("Document identifier exceeds capacity".to_string())
        })?;
        let run_id = run_id.to_string();
        let portal_key = portal_key.to_string();
        let download_url = target.download_url.clone();
        self.query_connection("Record source check", move |conn| {
            execute_transaction(conn, "source check", |tx| {
                let (requested_url, http_status, observed_sha256, detail) = match &observation {
                    SourceCheckObservation::Received {
                        sha256,
                        byte_count,
                        acquisition,
                    } => {
                        let sha256 = normalize_sha256(sha256)?;
                        let byte_count = i64::try_from(*byte_count).map_err(|_| {
                            AppError::DataParsing("Document size cannot be stored".to_string())
                        })?;
                        insert_document_acquisition(
                            tx,
                            &portal_key,
                            document_id,
                            None,
                            &sha256,
                            byte_count,
                            acquisition,
                        )?;
                        (
                            acquisition.requested_url.clone(),
                            Some(i64::from(acquisition.http_status)),
                            Some(sha256),
                            None,
                        )
                    }
                    SourceCheckObservation::Gone { acquisition } => (
                        acquisition.requested_url.clone(),
                        Some(i64::from(acquisition.http_status)),
                        None,
                        None,
                    ),
                    SourceCheckObservation::Unreachable { detail } => (
                        download_url.clone(),
                        None,
                        None,
                        Some(
                            detail
                                .chars()
                                .take(MAXIMUM_DETAIL_CHARACTERS)
                                .collect::<String>(),
                        ),
                    ),
                };
                tx.execute(
                    "INSERT INTO document_source_checks (
                        run_id, portal_subdomain, document_id, requested_url, checked_at, outcome,
                        http_status, collected_sha256, observed_sha256, detail
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        run_id,
                        portal_key,
                        document_id,
                        requested_url,
                        chrono::Utc::now().to_rfc3339(),
                        outcome.as_str(),
                        http_status,
                        collected_sha256,
                        observed_sha256,
                        detail,
                    ],
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to record source check: {error}"))
                })?;
                Ok(())
            })
        })
        .await?;
        Ok(outcome)
    }

    /// Checks newest first, for one source or one of its documents.
    pub async fn list_source_checks(
        &self,
        portal_key: &str,
        document_id: Option<u64>,
        limit: usize,
    ) -> Result<Vec<SourceCheck>, AppError> {
        if !(1..=MAXIMUM_LISTED_CHECKS).contains(&limit) {
            return Err(AppError::DataParsing(format!(
                "Source check limit must be between 1 and {MAXIMUM_LISTED_CHECKS}"
            )));
        }
        let portal_key = portal_key.to_string();
        let document_id = document_id.map(i64::try_from).transpose().map_err(|_| {
            AppError::DataParsing("Document identifier exceeds capacity".to_string())
        })?;
        let limit = limit as i64;
        self.query_connection("List source checks", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT check_id, run_id, portal_subdomain, document_id, requested_url,
                        checked_at, outcome, http_status, collected_sha256, observed_sha256, detail
                     FROM document_source_checks
                     WHERE portal_subdomain = ?1 AND (?2 IS NULL OR document_id = ?2)
                     ORDER BY checked_at DESC, check_id DESC
                     LIMIT ?3",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare source checks: {error}"))
                })?;
            let rows = statement
                .query_map(params![portal_key, document_id, limit], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read source checks: {error}"))
                })?;
            rows.map(|row| {
                let (
                    check_id,
                    run_id,
                    portal_key,
                    document_id,
                    requested_url,
                    checked_at,
                    outcome,
                    http_status,
                    collected_sha256,
                    observed_sha256,
                    detail,
                ) = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode a source check: {error}"))
                })?;
                let corrupt =
                    || AppError::Storage("A source check row is out of range".to_string());
                Ok(SourceCheck {
                    check_id: u64::try_from(check_id).map_err(|_| corrupt())?,
                    run_id,
                    portal_key,
                    document_id: u64::try_from(document_id).map_err(|_| corrupt())?,
                    requested_url,
                    checked_at,
                    outcome: SourceCheckOutcome::parse(&outcome).ok_or_else(corrupt)?,
                    http_status: http_status
                        .map(u16::try_from)
                        .transpose()
                        .map_err(|_| corrupt())?,
                    collected_sha256,
                    observed_sha256,
                    detail,
                })
            })
            .collect()
        })
        .await
    }
}
