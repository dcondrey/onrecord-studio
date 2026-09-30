//! Persistence for the autonomous findings inbox.
//!
//! Every detector emits a `Finding` and calls [`DuckDbRepository::record_findings`], which upserts
//! by content-hashed id. A re-run over unchanged evidence updates `last_seen_at` without adding a
//! row; a detector that stopped firing has its stale rows dropped via
//! [`DuckDbRepository::clear_stale_findings`]. Dismissals set `dismissed_at` and are preserved
//! through re-runs (the upsert leaves `dismissed_at` untouched when a row already exists).

use super::sqlite::DuckDbRepository;
use crate::{
    analysis::finding::{
        Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
    },
    error::AppError,
};
use chrono::{DateTime, Utc};
use duckdb::{params, params_from_iter};
use serde::{Deserialize, Serialize};

fn storage(e: impl std::fmt::Display) -> AppError {
    AppError::Storage(e.to_string())
}

/// Query filter for [`DuckDbRepository::list_findings`]. All fields optional; omitted = "no
/// filter." Dismissed findings are excluded by default; pass `include_dismissed = true` to include
/// them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindingsFilter {
    pub portal_key: Option<String>,
    pub agency_id: Option<i64>,
    pub request_id: Option<i64>,
    pub document_id: Option<i64>,
    pub source_detector: Option<String>,
    pub include_dismissed: Option<bool>,
}

impl DuckDbRepository {
    /// Upserts a batch of findings. Existing rows keyed by `id` have their `last_seen_at` (and
    /// any updated headline/explanation/citations) refreshed; `discovered_at` and `dismissed_at`
    /// are preserved. New rows are inserted.
    pub async fn record_findings(&self, findings: Vec<Finding>) -> Result<usize, AppError> {
        if findings.is_empty() {
            return Ok(0);
        }
        if let Some(reason) = findings
            .iter()
            .find_map(|finding| finding.check_recordable().err())
        {
            return Err(AppError::Analysis(reason));
        }
        self.query_connection("Record findings", move |conn| {
            let tx = conn.transaction().map_err(storage)?;
            let mut written = 0_usize;
            for finding in findings {
                let citations_json = serde_json::to_string(&finding.citations).map_err(storage)?;
                let affected = tx
                    .execute(
                        "INSERT INTO findings (id, kind, severity, confidence_basis_points,
                            subject_portal_key, subject_agency_id, subject_request_id,
                            subject_document_id, subject_entity_id, headline, explanation,
                            citations_json, discovered_at, last_seen_at, dismissed_at,
                            source_detector)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                         ON CONFLICT (id) DO UPDATE SET
                            severity = EXCLUDED.severity,
                            confidence_basis_points = EXCLUDED.confidence_basis_points,
                            headline = EXCLUDED.headline,
                            explanation = EXCLUDED.explanation,
                            citations_json = EXCLUDED.citations_json,
                            last_seen_at = EXCLUDED.last_seen_at",
                        params![
                            finding.id,
                            finding.kind.as_str(),
                            finding.severity.as_str(),
                            finding.confidence.stored_basis_points(),
                            finding.subject.portal_key,
                            finding.subject.agency_id,
                            finding.subject.request_id,
                            finding.subject.document_id,
                            finding.subject.entity_id,
                            finding.headline,
                            finding.explanation,
                            citations_json,
                            finding.discovered_at.to_rfc3339(),
                            finding.last_seen_at.to_rfc3339(),
                            finding.dismissed_at.map(|t| t.to_rfc3339()),
                            finding.source_detector,
                        ],
                    )
                    .map_err(storage)?;
                written += affected;
            }
            tx.commit().map_err(storage)?;
            Ok(written)
        })
        .await
    }

    /// Returns findings ranked by (severity desc, confidence desc, freshness desc). Dismissed
    /// findings excluded unless `filter.include_dismissed = Some(true)`.
    pub async fn list_findings(
        &self,
        filter: FindingsFilter,
        limit: usize,
    ) -> Result<Vec<Finding>, AppError> {
        let capped_limit = limit.clamp(1, 500) as i64;
        self.query_connection("List findings", move |conn| {
            let mut sql = String::from(
                "SELECT id, kind, severity, confidence_basis_points,
                    subject_portal_key, subject_agency_id, subject_request_id,
                    subject_document_id, subject_entity_id, headline, explanation,
                    citations_json, discovered_at, last_seen_at, dismissed_at, source_detector
                 FROM findings
                 WHERE 1 = 1",
            );
            let mut binds: Vec<Box<dyn duckdb::types::ToSql>> = Vec::new();
            if !filter.include_dismissed.unwrap_or(false) {
                sql.push_str(" AND dismissed_at IS NULL");
            }
            if let Some(portal) = filter.portal_key {
                sql.push_str(&format!(" AND subject_portal_key = ?{}", binds.len() + 1));
                binds.push(Box::new(portal));
            }
            if let Some(agency) = filter.agency_id {
                sql.push_str(&format!(" AND subject_agency_id = ?{}", binds.len() + 1));
                binds.push(Box::new(agency));
            }
            if let Some(request) = filter.request_id {
                sql.push_str(&format!(" AND subject_request_id = ?{}", binds.len() + 1));
                binds.push(Box::new(request));
            }
            if let Some(document) = filter.document_id {
                sql.push_str(&format!(" AND subject_document_id = ?{}", binds.len() + 1));
                binds.push(Box::new(document));
            }
            if let Some(detector) = filter.source_detector {
                sql.push_str(&format!(" AND source_detector = ?{}", binds.len() + 1));
                binds.push(Box::new(detector));
            }
            sql.push_str(
                " ORDER BY CASE severity
                    WHEN 'CRITICAL' THEN 4
                    WHEN 'SERIOUS' THEN 3
                    WHEN 'NOTABLE' THEN 2
                    WHEN 'INFO' THEN 1
                    ELSE 0 END DESC,
                  COALESCE(confidence_basis_points, -1) DESC,
                  last_seen_at DESC",
            );
            sql.push_str(&format!(" LIMIT ?{}", binds.len() + 1));
            binds.push(Box::new(capped_limit));
            let mut statement = conn.prepare(&sql).map_err(storage)?;
            let rows = statement
                .query_map(params_from_iter(binds.iter().map(|b| b.as_ref())), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<i32>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, String>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, String>(15)?,
                    ))
                })
                .map_err(storage)?;
            let mut findings = Vec::new();
            for row in rows {
                let (
                    id,
                    kind,
                    severity,
                    confidence_bp,
                    portal_key,
                    agency_id,
                    request_id,
                    document_id,
                    entity_id,
                    headline,
                    explanation,
                    citations_json,
                    discovered_at,
                    last_seen_at,
                    dismissed_at,
                    source_detector,
                ) = row.map_err(storage)?;
                let kind = FindingKind::from_stored(&kind).ok_or_else(|| {
                    AppError::Storage(format!("Unknown finding kind in storage: {kind}"))
                })?;
                let severity = FindingSeverity::from_stored(&severity).ok_or_else(|| {
                    AppError::Storage(format!("Unknown finding severity in storage: {severity}"))
                })?;
                let confidence =
                    FindingConfidence::from_stored(confidence_bp.map(|v| v.max(0) as u16));
                let citations: Vec<FindingCitation> =
                    serde_json::from_str(&citations_json).map_err(storage)?;
                findings.push(Finding {
                    id,
                    kind,
                    severity,
                    confidence,
                    subject: FindingSubject {
                        portal_key,
                        agency_id,
                        request_id,
                        document_id,
                        entity_id,
                    },
                    headline,
                    explanation,
                    citations,
                    discovered_at: parse_ts(&discovered_at)?,
                    last_seen_at: parse_ts(&last_seen_at)?,
                    dismissed_at: dismissed_at.map(|value| parse_ts(&value)).transpose()?,
                    source_detector,
                });
            }
            Ok(findings)
        })
        .await
    }

    /// Reads one finding by id. Returns `Ok(None)` when no row matches. Used by artifact
    /// generators to draft a follow-up or appeal directly from the stored finding without asking
    /// the caller to marshal one.
    pub async fn get_finding_by_id(&self, id: String) -> Result<Option<Finding>, AppError> {
        self.query_connection("Get finding by id", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT id, kind, severity, confidence_basis_points,
                        subject_portal_key, subject_agency_id, subject_request_id,
                        subject_document_id, subject_entity_id, headline, explanation,
                        citations_json, discovered_at, last_seen_at, dismissed_at, source_detector
                     FROM findings
                     WHERE id = ?1",
                )
                .map_err(storage)?;
            let mut rows = statement
                .query_map(params![id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<i32>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, String>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, String>(15)?,
                    ))
                })
                .map_err(storage)?;
            let Some(row) = rows.next() else {
                return Ok(None);
            };
            let (
                row_id,
                kind,
                severity,
                confidence_bp,
                portal_key,
                agency_id,
                request_id,
                document_id,
                entity_id,
                headline,
                explanation,
                citations_json,
                discovered_at,
                last_seen_at,
                dismissed_at,
                source_detector,
            ) = row.map_err(storage)?;
            let kind = FindingKind::from_stored(&kind).ok_or_else(|| {
                AppError::Storage(format!("Unknown finding kind in storage: {kind}"))
            })?;
            let severity = FindingSeverity::from_stored(&severity).ok_or_else(|| {
                AppError::Storage(format!("Unknown finding severity in storage: {severity}"))
            })?;
            let confidence = FindingConfidence::from_stored(confidence_bp.map(|v| v.max(0) as u16));
            let citations: Vec<FindingCitation> =
                serde_json::from_str(&citations_json).map_err(storage)?;
            Ok(Some(Finding {
                id: row_id,
                kind,
                severity,
                confidence,
                subject: FindingSubject {
                    portal_key,
                    agency_id,
                    request_id,
                    document_id,
                    entity_id,
                },
                headline,
                explanation,
                citations,
                discovered_at: parse_ts(&discovered_at)?,
                last_seen_at: parse_ts(&last_seen_at)?,
                dismissed_at: dismissed_at.map(|value| parse_ts(&value)).transpose()?,
                source_detector,
            }))
        })
        .await
    }

    /// Marks one finding dismissed. Idempotent.
    pub async fn dismiss_finding(&self, id: String) -> Result<(), AppError> {
        self.query_connection("Dismiss finding", move |conn| {
            conn.execute(
                "UPDATE findings SET dismissed_at = ?1 WHERE id = ?2 AND dismissed_at IS NULL",
                params![Utc::now().to_rfc3339(), id],
            )
            .map_err(storage)?;
            Ok(())
        })
        .await
    }

    /// Removes non-dismissed findings a given detector emitted that haven't been re-emitted since
    /// `cutoff`. Dismissed findings are preserved as a user-facing audit trail.
    pub async fn clear_stale_findings(
        &self,
        source_detector: String,
        cutoff: DateTime<Utc>,
    ) -> Result<usize, AppError> {
        self.query_connection("Clear stale findings", move |conn| {
            let affected = conn
                .execute(
                    "DELETE FROM findings
                     WHERE source_detector = ?1
                       AND dismissed_at IS NULL
                       AND last_seen_at < ?2",
                    params![source_detector, cutoff.to_rfc3339()],
                )
                .map_err(storage)?;
            Ok(affected)
        })
        .await
    }
}

fn parse_ts(value: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|error| AppError::Storage(format!("Invalid finding timestamp {value}: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::finding::finding_id;
    use crate::storage::sqlite::DuckDbRepository;

    async fn fresh_repo() -> DuckDbRepository {
        let repo = DuckDbRepository::new(":memory:").expect("open in-memory core");
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .expect("run migrations");
        repo
    }

    fn sample_finding(document_id: i64, headline: &str, severity: FindingSeverity) -> Finding {
        let subject = FindingSubject {
            portal_key: Some("test-portal".into()),
            document_id: Some(document_id),
            ..Default::default()
        };
        let id = finding_id(
            FindingKind::HeldBackResponse,
            "held_back_response",
            &subject,
            &[headline],
        );
        let now = Utc::now();
        Finding {
            id,
            kind: FindingKind::HeldBackResponse,
            severity,
            confidence: FindingConfidence::measured(7500),
            subject,
            headline: headline.into(),
            explanation: "test explanation".into(),
            citations: vec![],
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: "held_back_response".into(),
        }
    }

    #[tokio::test]
    async fn record_findings_upserts_by_id_and_preserves_dismissal() {
        let repo = fresh_repo().await;
        let one = sample_finding(1, "held 3 days", FindingSeverity::Serious);
        let id = one.id.clone();
        repo.record_findings(vec![one.clone()]).await.unwrap();
        repo.dismiss_finding(id.clone()).await.unwrap();
        // Re-run: same id, would-be replacement, but dismissed_at must be preserved.
        repo.record_findings(vec![one]).await.unwrap();
        let listed = repo
            .list_findings(
                FindingsFilter {
                    include_dismissed: Some(true),
                    ..Default::default()
                },
                50,
            )
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert!(
            listed[0].dismissed_at.is_some(),
            "dismissal must survive re-record"
        );
    }

    #[tokio::test]
    async fn list_findings_ranks_critical_above_serious_regardless_of_confidence() {
        let repo = fresh_repo().await;
        let critical_low_conf = Finding {
            confidence: FindingConfidence::measured(1000),
            ..sample_finding(1, "critical low", FindingSeverity::Critical)
        };
        let serious_high_conf = Finding {
            confidence: FindingConfidence::measured(9500),
            ..sample_finding(2, "serious high", FindingSeverity::Serious)
        };
        repo.record_findings(vec![serious_high_conf, critical_low_conf])
            .await
            .unwrap();
        let listed = repo
            .list_findings(FindingsFilter::default(), 50)
            .await
            .unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].severity, FindingSeverity::Critical);
        assert_eq!(listed[1].severity, FindingSeverity::Serious);
    }

    #[tokio::test]
    async fn dismissed_findings_hidden_by_default() {
        let repo = fresh_repo().await;
        let one = sample_finding(1, "one", FindingSeverity::Notable);
        let id = one.id.clone();
        repo.record_findings(vec![one]).await.unwrap();
        repo.dismiss_finding(id).await.unwrap();
        let default = repo
            .list_findings(FindingsFilter::default(), 50)
            .await
            .unwrap();
        assert!(default.is_empty(), "dismissed hidden by default");
        let included = repo
            .list_findings(
                FindingsFilter {
                    include_dismissed: Some(true),
                    ..Default::default()
                },
                50,
            )
            .await
            .unwrap();
        assert_eq!(included.len(), 1);
    }

    #[tokio::test]
    async fn clear_stale_findings_drops_only_older_non_dismissed_rows_for_that_detector() {
        let repo = fresh_repo().await;
        let mut old = sample_finding(1, "old", FindingSeverity::Info);
        old.last_seen_at = Utc::now() - chrono::Duration::hours(48);
        let fresh = sample_finding(2, "fresh", FindingSeverity::Info);
        let mut dismissed_old = sample_finding(3, "dismissed old", FindingSeverity::Info);
        dismissed_old.last_seen_at = Utc::now() - chrono::Duration::hours(48);
        let dismissed_id = dismissed_old.id.clone();
        repo.record_findings(vec![old, fresh, dismissed_old])
            .await
            .unwrap();
        repo.dismiss_finding(dismissed_id.clone()).await.unwrap();
        let cutoff = Utc::now() - chrono::Duration::hours(24);
        let removed = repo
            .clear_stale_findings("held_back_response".into(), cutoff)
            .await
            .unwrap();
        assert_eq!(removed, 1, "only the un-dismissed old row is cleared");
        let all = repo
            .list_findings(
                FindingsFilter {
                    include_dismissed: Some(true),
                    ..Default::default()
                },
                50,
            )
            .await
            .unwrap();
        assert_eq!(all.len(), 2, "fresh + dismissed-old survive");
    }

    #[tokio::test]
    async fn filter_by_subject_returns_only_matching_findings() {
        let repo = fresh_repo().await;
        let mut for_doc_one = sample_finding(1, "one", FindingSeverity::Notable);
        for_doc_one.subject.document_id = Some(1);
        let mut for_doc_two = sample_finding(2, "two", FindingSeverity::Notable);
        for_doc_two.subject.document_id = Some(2);
        repo.record_findings(vec![for_doc_one, for_doc_two])
            .await
            .unwrap();
        let filtered = repo
            .list_findings(
                FindingsFilter {
                    document_id: Some(1),
                    ..Default::default()
                },
                50,
            )
            .await
            .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].subject.document_id, Some(1));
    }
}
