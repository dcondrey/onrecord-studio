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
    /// Only findings first discovered strictly after this instant. Applied before the severity
    /// ranking and limit, so a new low-severity finding is not hidden behind an older backlog.
    pub discovered_after: Option<DateTime<Utc>>,
    /// Exactly one finding by id; a lookup, not a search.
    pub id: Option<String>,
}

impl FindingsFilter {
    /// Appends this filter's predicates to a query ending in `WHERE 1 = 1` and returns the
    /// binds in the order the placeholders were numbered. Shared by list and count so the two
    /// can never disagree about which rows a filter names.
    fn append_where(self, sql: &mut String) -> Vec<Box<dyn duckdb::types::ToSql>> {
        let mut binds: Vec<Box<dyn duckdb::types::ToSql>> = Vec::new();
        if !self.include_dismissed.unwrap_or(false) {
            sql.push_str(" AND dismissed_at IS NULL");
        }
        if let Some(portal) = self.portal_key {
            sql.push_str(&format!(" AND subject_portal_key = ?{}", binds.len() + 1));
            binds.push(Box::new(portal));
        }
        if let Some(agency) = self.agency_id {
            sql.push_str(&format!(" AND subject_agency_id = ?{}", binds.len() + 1));
            binds.push(Box::new(agency));
        }
        if let Some(request) = self.request_id {
            sql.push_str(&format!(" AND subject_request_id = ?{}", binds.len() + 1));
            binds.push(Box::new(request));
        }
        if let Some(document) = self.document_id {
            sql.push_str(&format!(" AND subject_document_id = ?{}", binds.len() + 1));
            binds.push(Box::new(document));
        }
        if let Some(detector) = self.source_detector {
            sql.push_str(&format!(" AND source_detector = ?{}", binds.len() + 1));
            binds.push(Box::new(detector));
        }
        if let Some(after) = self.discovered_after {
            // The column holds RFC 3339 text; comparing as timestamps keeps a row written
            // with a different offset or precision comparable, and drops an unparsable one.
            sql.push_str(&format!(
                " AND TRY_CAST(discovered_at AS TIMESTAMPTZ) > TRY_CAST(?{} AS TIMESTAMPTZ)",
                binds.len() + 1
            ));
            binds.push(Box::new(after.to_rfc3339()));
        }
        if let Some(id) = self.id {
            sql.push_str(&format!(" AND id = ?{}", binds.len() + 1));
            binds.push(Box::new(id));
        }
        binds
    }
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

    /// Number of findings the filter names, with no ranking or limit applied.
    pub async fn count_findings(&self, filter: FindingsFilter) -> Result<u64, AppError> {
        self.query_connection("Count findings", move |conn| {
            let mut sql = String::from("SELECT COUNT(*) FROM findings WHERE 1 = 1");
            let binds = filter.append_where(&mut sql);
            let count: i64 = conn
                .query_row(
                    &sql,
                    params_from_iter(binds.iter().map(|b| b.as_ref())),
                    |row| row.get(0),
                )
                .map_err(storage)?;
            Ok(u64::try_from(count).unwrap_or(0))
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
        self.list_findings_for_surface(filter, limit.clamp(1, 500), false)
            .await
    }

    pub async fn list_story_candidates(&self, limit: usize) -> Result<Vec<Finding>, AppError> {
        self.list_findings_for_surface(FindingsFilter::default(), limit.clamp(1, 2_000), true)
            .await
    }

    async fn list_findings_for_surface(
        &self,
        filter: FindingsFilter,
        limit: usize,
        story: bool,
    ) -> Result<Vec<Finding>, AppError> {
        let capped_limit = limit as i64;
        self.query_connection("List findings", move |conn| {
            let mut sql = String::from(
                "SELECT id, kind, severity, confidence_basis_points,
                    subject_portal_key, subject_agency_id, subject_request_id,
                    subject_document_id, subject_entity_id, headline, explanation,
                    citations_json, discovered_at, last_seen_at, dismissed_at, source_detector
                 FROM findings
                 WHERE 1 = 1",
            );
            let mut binds = filter.append_where(&mut sql);
            if story {
                for kind in crate::analysis::story_desk::CONTEXT_ONLY_KINDS {
                    sql.push_str(&format!(" AND kind != ?{}", binds.len() + 1));
                    binds.push(Box::new(kind.as_str().to_string()));
                }
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

    /// Clears a dismissal, for undo. Idempotent. A restored finding is stale-cleared like any
    /// other if its detector no longer emits it.
    pub async fn restore_finding(&self, id: String) -> Result<(), AppError> {
        self.query_connection("Restore finding", move |conn| {
            conn.execute(
                "UPDATE findings SET dismissed_at = NULL WHERE id = ?1 AND dismissed_at IS NOT NULL",
                params![id],
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
    async fn story_candidates_exclude_context_before_limiting_without_deleting_it() {
        let repo = fresh_repo().await;
        let mut context = sample_finding(1, "Redaction candidates", FindingSeverity::Critical);
        context.kind = FindingKind::RedactionCandidateInferred;
        let lead = sample_finding(2, "Accountability lead", FindingSeverity::Notable);
        repo.record_findings(vec![context.clone(), lead.clone()])
            .await
            .unwrap();
        let ranked = repo.list_story_candidates(1).await.unwrap();
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, lead.id);
        let records = repo
            .list_findings(FindingsFilter::default(), 2)
            .await
            .unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().any(|row| row.id == context.id));
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
    async fn a_dismissal_hides_the_finding_until_restored_and_keeps_its_discovery_time() {
        let repo = fresh_repo().await;
        let one = sample_finding(1, "one", FindingSeverity::Notable);
        let id = one.id.clone();
        repo.record_findings(vec![one]).await.unwrap();
        let before = repo
            .list_findings(FindingsFilter::default(), 50)
            .await
            .unwrap();
        repo.dismiss_finding(id.clone()).await.unwrap();
        assert!(repo
            .list_findings(FindingsFilter::default(), 50)
            .await
            .unwrap()
            .is_empty());
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
        repo.restore_finding(id.clone()).await.unwrap();
        repo.restore_finding(id).await.unwrap();
        let after = repo
            .list_findings(FindingsFilter::default(), 50)
            .await
            .unwrap();
        assert_eq!(after.len(), 1);
        assert!(after[0].dismissed_at.is_none());
        assert_eq!(after[0].discovered_at, before[0].discovered_at);
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
    async fn discovered_after_admits_a_new_low_severity_finding_past_an_older_critical_backlog() {
        let repo = fresh_repo().await;
        let cutoff = Utc::now() - chrono::Duration::minutes(5);
        let old = Finding {
            discovered_at: cutoff - chrono::Duration::hours(1),
            ..sample_finding(1, "old critical", FindingSeverity::Critical)
        };
        let new = Finding {
            discovered_at: cutoff + chrono::Duration::minutes(1),
            ..sample_finding(2, "new info", FindingSeverity::Info)
        };
        repo.record_findings(vec![old, new]).await.unwrap();
        let listed = repo
            .list_findings(
                FindingsFilter {
                    discovered_after: Some(cutoff),
                    ..Default::default()
                },
                1,
            )
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].headline, "new info");
        let counted = repo
            .count_findings(FindingsFilter {
                discovered_after: Some(cutoff),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(counted, 1);
        assert_eq!(
            repo.count_findings(FindingsFilter::default())
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn document_and_id_filters_each_select_exactly_their_finding() {
        let repo = fresh_repo().await;
        let one = sample_finding(1, "one", FindingSeverity::Notable);
        let two = sample_finding(2, "two", FindingSeverity::Notable);
        let two_id = two.id.clone();
        repo.record_findings(vec![one, two]).await.unwrap();
        for filter in [
            FindingsFilter {
                document_id: Some(2),
                ..Default::default()
            },
            FindingsFilter {
                id: Some(two_id.clone()),
                ..Default::default()
            },
        ] {
            let listed = repo.list_findings(filter, 50).await.unwrap();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].id, two_id);
        }
    }
}
