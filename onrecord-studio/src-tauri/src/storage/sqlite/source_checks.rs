//! Re-fetch observations of collected documents. The collected row is never repointed: citations
//! and extracted text belong to the bytes that were collected, so a later fetch is recorded beside
//! them, as a check and (when bytes arrived) an acquisition. Different bytes are kept as a
//! content-addressed object the check references, which purge releases by reference count.

use super::connection::execute_transaction;
use super::documents::insert_document_acquisition;
use super::integrity::normalize_sha256;
use super::*;
use crate::ingestion::client::AcquisitionRecord;
use crate::types::{DocumentAcquisition, DocumentSourceHistory, SourceCheck, SourceCheckOutcome};

const MAXIMUM_DETAIL_CHARACTERS: usize = 2_048;
const MAXIMUM_LISTED_CHECKS: usize = 500;
const MAXIMUM_LISTED_ACQUISITIONS: i64 = 500;

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
        /// The object store path the received bytes were committed under. Only a
        /// DIFFERENT_BYTES fetch may carry one.
        object_path: Option<String>,
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
    ///
    /// `None`: the document was purged while it was being fetched, and nothing was recorded. A
    /// check row for a missing document is an orphan the startup relation audit refuses to open.
    pub async fn record_source_check(
        &self,
        run_id: &str,
        portal_key: &str,
        target: &SourceRecheckTarget,
        observation: SourceCheckObservation,
    ) -> Result<Option<SourceCheckOutcome>, AppError> {
        let collected_sha256 = normalize_sha256(&target.collected_sha256)?;
        let outcome = classify(&collected_sha256, &observation);
        if matches!(
            &observation,
            SourceCheckObservation::Received {
                object_path: Some(_),
                ..
            }
        ) && outcome != SourceCheckOutcome::DifferentBytes
        {
            return Err(AppError::Internal(
                "Only different bytes are kept from a source check".to_string(),
            ));
        }
        let document_id = i64::try_from(target.document_id).map_err(|_| {
            AppError::DataParsing("Document identifier exceeds capacity".to_string())
        })?;
        let run_id = run_id.to_string();
        let portal_key = portal_key.to_string();
        let download_url = target.download_url.clone();
        let recorded = self
            .query_connection("Record source check", move |conn| {
                execute_transaction(conn, "source check", |tx| {
                    let present: bool = tx
                        .query_row(
                            "SELECT EXISTS (SELECT 1 FROM documents
                                        WHERE id = ?1 AND portal_subdomain = ?2)",
                            params![document_id, portal_key],
                            |row| row.get(0),
                        )
                        .map_err(|error| {
                            AppError::Storage(format!(
                                "Failed to find the checked document: {error}"
                            ))
                        })?;
                    if !present {
                        return Ok(false);
                    }
                    let (
                        requested_url,
                        http_status,
                        observed_sha256,
                        detail,
                        acquisition_id,
                        object_path,
                    ) = match &observation {
                        SourceCheckObservation::Received {
                            sha256,
                            byte_count,
                            acquisition,
                            object_path,
                        } => {
                            let sha256 = normalize_sha256(sha256)?;
                            let byte_count = i64::try_from(*byte_count).map_err(|_| {
                                AppError::DataParsing("Document size cannot be stored".to_string())
                            })?;
                            let acquisition_id = insert_document_acquisition(
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
                                Some(acquisition_id),
                                object_path.clone(),
                            )
                        }
                        SourceCheckObservation::Gone { acquisition } => (
                            acquisition.requested_url.clone(),
                            Some(i64::from(acquisition.http_status)),
                            None,
                            None,
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
                            None,
                            None,
                        ),
                    };
                    tx.execute(
                        "INSERT INTO document_source_checks (
                        run_id, portal_subdomain, document_id, requested_url, checked_at, outcome,
                        http_status, collected_sha256, observed_sha256, detail, acquisition_id,
                        observed_object_path
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                            acquisition_id,
                            object_path,
                        ],
                    )
                    .map_err(|error| {
                        AppError::Storage(format!("Failed to record source check: {error}"))
                    })?;
                    Ok(true)
                })
            })
            .await?;
        Ok(recorded.then_some(outcome))
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
                        checked_at, outcome, http_status, collected_sha256, observed_sha256, detail,
                        acquisition_id, observed_object_path IS NOT NULL
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
                        row.get::<_, Option<i64>>(11)?,
                        row.get::<_, bool>(12)?,
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
                    acquisition_id,
                    observed_copy_retained,
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
                    acquisition_id: acquisition_id
                        .map(u64::try_from)
                        .transpose()
                        .map_err(|_| corrupt())?,
                    observed_copy_retained,
                })
            })
            .collect()
        })
        .await
    }

    /// One document's checks and acquisitions, each newest first.
    pub async fn document_source_history(
        &self,
        portal_key: &str,
        document_id: u64,
    ) -> Result<DocumentSourceHistory, AppError> {
        let checks = self
            .list_source_checks(portal_key, Some(document_id), MAXIMUM_LISTED_CHECKS)
            .await?;
        let acquisitions = self
            .list_document_acquisitions(portal_key, document_id)
            .await?;
        Ok(DocumentSourceHistory {
            checks,
            acquisitions,
        })
    }

    async fn list_document_acquisitions(
        &self,
        portal_key: &str,
        document_id: u64,
    ) -> Result<Vec<DocumentAcquisition>, AppError> {
        let portal_key = portal_key.to_string();
        let document_id = i64::try_from(document_id).map_err(|_| {
            AppError::DataParsing("Document identifier exceeds capacity".to_string())
        })?;
        self.query_connection("List document acquisitions", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT a.acquisition_id, a.sync_run_id, a.requested_url, a.resolved_url,
                        a.http_status, a.content_type, a.raw_file_sha256, a.file_size,
                        a.previous_raw_file_sha256, a.retrieved_at,
                        (SELECT MIN(c.check_id) FROM document_source_checks c
                         WHERE c.acquisition_id = a.acquisition_id)
                     FROM document_acquisitions a
                     WHERE a.portal_subdomain = ?1 AND a.document_id = ?2
                     ORDER BY a.retrieved_at DESC, a.acquisition_id DESC
                     LIMIT ?3",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare acquisitions: {error}"))
                })?;
            let rows = statement
                .query_map(
                    params![portal_key, document_id, MAXIMUM_LISTED_ACQUISITIONS],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, Option<String>>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, Option<String>>(8)?,
                            row.get::<_, String>(9)?,
                            row.get::<_, Option<i64>>(10)?,
                        ))
                    },
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to read acquisitions: {error}"))
                })?;
            rows.map(|row| {
                let (
                    acquisition_id,
                    sync_run_id,
                    requested_url,
                    resolved_url,
                    http_status,
                    content_type,
                    raw_file_sha256,
                    file_size,
                    previous_raw_file_sha256,
                    retrieved_at,
                    source_check_id,
                ) = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode an acquisition: {error}"))
                })?;
                let corrupt =
                    || AppError::Storage("An acquisition row is out of range".to_string());
                Ok(DocumentAcquisition {
                    acquisition_id: u64::try_from(acquisition_id).map_err(|_| corrupt())?,
                    sync_run_id,
                    requested_url,
                    resolved_url,
                    http_status: u16::try_from(http_status).map_err(|_| corrupt())?,
                    content_type,
                    raw_file_sha256,
                    file_size: u64::try_from(file_size).map_err(|_| corrupt())?,
                    previous_raw_file_sha256,
                    retrieved_at,
                    source_check_id: source_check_id
                        .map(u64::try_from)
                        .transpose()
                        .map_err(|_| corrupt())?,
                })
            })
            .collect()
        })
        .await
    }

    /// The kept object of one DIFFERENT_BYTES check of a public document, with the digest the
    /// check observed, so a reader verifies the file against what was received.
    pub async fn source_check_copy(
        &self,
        portal_key: &str,
        document_id: u64,
        check_id: u64,
    ) -> Result<(PathBuf, String), AppError> {
        let portal_key = portal_key.to_string();
        let document_id = i64::try_from(document_id).map_err(|_| {
            AppError::DataParsing("Document identifier exceeds capacity".to_string())
        })?;
        let check_id = i64::try_from(check_id)
            .map_err(|_| AppError::DataParsing("Check identifier exceeds capacity".to_string()))?;
        let found = self
            .query_connection("Source check copy lookup", move |conn| {
                conn.query_row(
                    "SELECT c.observed_object_path, c.observed_sha256
                     FROM document_source_checks c
                     JOIN documents d
                       ON d.id = c.document_id AND d.portal_subdomain = c.portal_subdomain
                     WHERE c.check_id = ?1 AND c.document_id = ?2 AND c.portal_subdomain = ?3
                       AND LOWER(TRIM(d.visibility)) = 'public'",
                    params![check_id, document_id, portal_key],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                        ))
                    },
                )
                .optional()
                .map_err(|error| {
                    AppError::Storage(format!("Failed to find the source check copy: {error}"))
                })
            })
            .await?
            .ok_or_else(|| AppError::Storage("That source check was not found".to_string()))?;
        match found {
            (Some(path), Some(sha256)) if !path.trim().is_empty() => {
                Ok((PathBuf::from(path), sha256))
            }
            _ => Err(AppError::Storage(
                "That source check's bytes were not kept".to_string(),
            )),
        }
    }

    /// When this source was last checked, by any run.
    pub async fn latest_source_check_at(
        &self,
        portal_key: &str,
    ) -> Result<Option<String>, AppError> {
        let portal_key = portal_key.to_string();
        self.query_connection("Latest source check", move |conn| {
            conn.query_row(
                "SELECT MAX(checked_at) FROM document_source_checks WHERE portal_subdomain = ?1",
                params![portal_key],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|error| AppError::Storage(format!("Failed to read the last check: {error}")))
        })
        .await
    }

    /// Whether any row still references `path` as stored evidence.
    pub async fn evidence_object_referenced(&self, path: &str) -> Result<bool, AppError> {
        let path = path.to_string();
        self.query_connection("Evidence object references", move |conn| {
            conn.query_row(
                &format!("SELECT {}", object_referenced_sql("?1")),
                params![path],
                |row| row.get(0),
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to count evidence references: {error}"))
            })
        })
        .await
    }
}

/// SQL that is true when `path` (an SQL expression) is referenced by any document, pending
/// retention receipt, or source check.
fn object_referenced_sql(path: &str) -> String {
    format!(
        "(EXISTS (SELECT 1 FROM documents WHERE local_path = {path})
          OR EXISTS (SELECT 1 FROM data_retention_receipts
                     WHERE pending_path = {path} AND state IN ('PREPARED', 'REFERENCED'))
          OR EXISTS (SELECT 1 FROM document_source_checks WHERE observed_object_path = {path}))"
    )
}
