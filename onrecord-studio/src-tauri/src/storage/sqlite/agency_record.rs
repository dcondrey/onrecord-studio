//! Counts one agency's closed requests for [`AgencyRecord`]. Every request is evaluated by the
//! shared statute evaluator, so a count here and a deadline in a letter follow one rule.

use super::timeline::parse_extension_reason;
use super::DuckDbRepository;
use crate::analysis::artifacts::agency_record::{
    AgencyIdentity, AgencyRecord, AgencyRecordOption, MINIMUM_CLOSED_REQUESTS,
};
use crate::analysis::dispositions::Disposition;
use crate::analysis::finding::FindingKind;
use crate::analysis::statute::DeterminationBasis;
use crate::analysis::timeline::{
    evaluate_statutory_compliance_with_determination_at, StatutoryExtensionStatus,
};
use crate::error::AppError;
use crate::types::StatutoryExtensionRecord;
use duckdb::params;
use std::collections::BTreeSet;

const CLOSED: &str = "r.closed_at IS NOT NULL AND TRIM(r.closed_at) <> ''";

struct ClosedRequest {
    id: i64,
    created_at: String,
    updated_at: String,
    closed_at: String,
    determination_at: Option<String>,
    determination_source: Option<String>,
    extension: Option<(String, String, String, String)>,
}

/// `Some(late)` when the request carries a sourced determination the evaluator can count against
/// an absent or valid extension; `None` when it cannot be counted either way.
fn sourced_and_late(request: &ClosedRequest, observed_at: &str) -> Option<bool> {
    let extension = match &request.extension {
        Some((notice_at, expected, reason, source)) => Some(StatutoryExtensionRecord {
            notice_at: notice_at.clone(),
            expected_determination_at: expected.clone(),
            reason: parse_extension_reason(reason).ok()?,
            notice_source: source.clone(),
        }),
        None => None,
    };
    let analysis = evaluate_statutory_compliance_with_determination_at(
        u64::try_from(request.id).ok()?,
        &request.created_at,
        Some(&request.updated_at),
        Some(&request.closed_at),
        request.determination_at.as_deref(),
        request.determination_source.as_deref(),
        extension.as_ref(),
        observed_at,
    )
    .ok()?;
    (analysis.determination_basis == DeterminationBasis::SourcedDeterminationEvent
        && matches!(
            analysis.extension_status,
            StatutoryExtensionStatus::NotInvoked | StatutoryExtensionStatus::Valid
        ))
    .then_some(analysis.is_violation)
}

impl DuckDbRepository {
    /// Agencies with enough closed requests for a paragraph, each keyed by portal and id.
    pub async fn agency_record_options(&self) -> Result<Vec<AgencyRecordOption>, AppError> {
        self.query_connection("List agencies with a counted record", |conn| {
            let storage = |error: duckdb::Error| {
                AppError::Storage(format!("Failed to list agencies: {error}"))
            };
            let mut statement = conn
                .prepare(&format!(
                    "SELECT r.portal_subdomain, r.agency_id, a.name, COUNT(*)
                     FROM requests r
                     LEFT JOIN agencies a
                       ON a.id = r.agency_id AND a.portal_subdomain = r.portal_subdomain
                     WHERE r.agency_id IS NOT NULL AND {CLOSED}
                     GROUP BY r.portal_subdomain, r.agency_id, a.name
                     HAVING COUNT(*) >= ?1
                     ORDER BY a.name, r.portal_subdomain, r.agency_id"
                ))
                .map_err(storage)?;
            let options = statement
                .query_map(params![MINIMUM_CLOSED_REQUESTS], |row| {
                    Ok(AgencyRecordOption {
                        portal_key: row.get(0)?,
                        agency_id: row.get(1)?,
                        agency_name: row.get(2)?,
                        closed_requests: row.get(3)?,
                    })
                })
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            Ok(options)
        })
        .await
    }

    /// The counted record of one agency on one portal, evaluated as of `observed_at`.
    pub async fn agency_record(
        &self,
        identity: AgencyIdentity,
        observed_at: String,
    ) -> Result<AgencyRecord, AppError> {
        let AgencyIdentity {
            portal_key,
            agency_id,
        } = identity;
        let portal = portal_key.clone();
        let (agency_name, closed, contradicted) = self
            .query_connection("Count an agency's closed requests", move |conn| {
                let storage = |error: duckdb::Error| {
                    AppError::Storage(format!("Failed to count agency requests: {error}"))
                };
                let agency_name = conn
                    .prepare("SELECT name FROM agencies WHERE id = ?1 AND portal_subdomain = ?2")
                    .map_err(storage)?
                    .query_map(params![agency_id, portal], |row| row.get::<_, String>(0))
                    .map_err(storage)?
                    .next()
                    .transpose()
                    .map_err(storage)?;
                let closed = conn
                    .prepare(&format!(
                        "SELECT r.id, r.created_at, r.updated_at, r.closed_at,
                                r.determination_at, r.determination_source,
                                e.notice_at, e.expected_determination_at, e.reason, e.notice_source
                         FROM requests r
                         LEFT JOIN statutory_extensions e
                           ON e.request_id = r.id AND e.portal_subdomain = r.portal_subdomain
                         WHERE r.portal_subdomain = ?1 AND r.agency_id = ?2 AND {CLOSED}
                         ORDER BY r.id"
                    ))
                    .map_err(storage)?
                    .query_map(params![portal, agency_id], |row| {
                        let notice_at: Option<String> = row.get(6)?;
                        Ok(ClosedRequest {
                            id: row.get(0)?,
                            created_at: row.get(1)?,
                            updated_at: row.get(2)?,
                            closed_at: row.get(3)?,
                            determination_at: row.get(4)?,
                            determination_source: row.get(5)?,
                            extension: match notice_at {
                                Some(notice_at) => {
                                    Some((notice_at, row.get(7)?, row.get(8)?, row.get(9)?))
                                }
                                None => None,
                            },
                        })
                    })
                    .map_err(storage)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(storage)?;
                let contradicted = conn
                    .prepare(
                        "SELECT DISTINCT subject_request_id FROM findings
                         WHERE kind = ?1 AND subject_portal_key = ?2 AND subject_agency_id = ?3
                           AND dismissed_at IS NULL AND subject_request_id IS NOT NULL",
                    )
                    .map_err(storage)?
                    .query_map(
                        params![
                            FindingKind::NoRecordsContradictedByRelease.as_str(),
                            portal,
                            agency_id
                        ],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(storage)?
                    .collect::<Result<BTreeSet<_>, _>>()
                    .map_err(storage)?;
                Ok((agency_name, closed, contradicted))
            })
            .await?;

        let closed_ids: BTreeSet<i64> = closed.iter().map(|request| request.id).collect();
        let no_records: BTreeSet<i64> = self
            .department_dispositions(Some(portal_key.clone()))
            .await?
            .into_iter()
            .filter(|department| department.agency_id == Some(agency_id))
            .flat_map(|department| department.buckets)
            .filter(|bucket| bucket.disposition == Disposition::NoRecords)
            .flat_map(|bucket| bucket.requests)
            .map(|request| request.request_id)
            .filter(|id| closed_ids.contains(id))
            .collect();
        let counted: Vec<bool> = closed
            .iter()
            .filter_map(|request| sourced_and_late(request, &observed_at))
            .collect();
        let received = |pick: fn(&String, &String) -> bool| {
            closed
                .iter()
                .map(|request| &request.created_at)
                .reduce(|kept, next| if pick(next, kept) { next } else { kept })
                .map(|instant| instant.chars().take(10).collect::<String>())
        };
        Ok(AgencyRecord {
            portal_key,
            agency_id,
            agency_name,
            closed_requests: closed.len() as u64,
            sourced_determinations: counted.len() as u64,
            late_sourced_determinations: counted.iter().filter(|late| **late).count() as u64,
            no_records_closures: no_records.len() as u64,
            no_records_with_recorded_earlier_release: no_records.intersection(&contradicted).count()
                as u64,
            period: received(|next, kept| next < kept).zip(received(|next, kept| next > kept)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twelve closed requests for agency 7 on `sd` and two for agency 7 on `oak`. On `sd`:
    /// requests 1-3 carry a sourced determination (1 and 2 after the deadline, 3 within it),
    /// request 4 a late determination with no source, request 5 a stored no-records closure notice
    /// with a detector finding, request 6 the same notice without one. `oak` request 1 is sourced
    /// and late.
    async fn seeded() -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").expect("open");
        repository
            .run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .expect("migrate");
        repository
            .query_connection("agency record fixture", |conn| {
                let mut sql = String::from(
                    "INSERT INTO portals (subdomain, display_name, base_url) VALUES
                       ('sd', 'SD', 'https://sd.example'), ('oak', 'Oak', 'https://oak.example');
                     INSERT INTO agencies (id, portal_subdomain, name) VALUES
                       (7, 'sd', 'Purchasing'), (7, 'oak', 'Clerk');",
                );
                for id in 1..=12 {
                    let (determination, source) = match id {
                        1 | 2 => ("'2026-02-20T00:00:00Z'", "'portal event log'"),
                        3 => ("'2026-01-08T00:00:00Z'", "'portal event log'"),
                        4 => ("'2026-02-20T00:00:00Z'", "NULL"),
                        _ => ("NULL", "NULL"),
                    };
                    sql.push_str(&format!(
                        "INSERT INTO requests (id, portal_subdomain, status, created_at, updated_at,
                                               closed_at, agency_id, determination_at, determination_source)
                         VALUES ({id}, 'sd', 'closed', '2026-01-{day:02}T00:00:00Z',
                                 '2026-03-01T00:00:00Z', '2026-03-01T00:00:00Z', 7,
                                 {determination}, {source});",
                        day = id + 1,
                    ));
                }
                sql.push_str(
                    "INSERT INTO requests (id, portal_subdomain, status, created_at, updated_at,
                                           closed_at, agency_id, determination_at, determination_source)
                     VALUES (1, 'oak', 'closed', '2025-06-02T00:00:00Z', '2025-08-01T00:00:00Z',
                             '2025-08-01T00:00:00Z', 7, '2025-08-01T00:00:00Z', 'portal event log'),
                            (2, 'oak', 'closed', '2025-06-03T00:00:00Z', '2025-08-01T00:00:00Z',
                             '2025-08-01T00:00:00Z', 7, NULL, NULL),
                            (3, 'oak', 'open', '2025-06-04T00:00:00Z', '2025-06-04T00:00:00Z',
                             NULL, 7, NULL, NULL);
                     INSERT INTO documents (id, request_id, portal_subdomain, file_name, description,
                                            visibility, created_at, updated_at) VALUES
                       (105, 5, 'sd', 'closure-notice-5.txt', 'NextRequest closure notice for request 5',
                        'public', '2026-03-01', '2026-03-01'),
                       (106, 6, 'sd', 'closure-notice-6.txt', 'NextRequest closure notice for request 6',
                        'public', '2026-03-01', '2026-03-01');
                     INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text) VALUES
                       (105, 'sd', 'closure-notice-5.txt', 'No responsive documents The City of San Diego has no responsive documents.'),
                       (106, 'sd', 'closure-notice-6.txt', 'No responsive documents The City of San Diego has no responsive documents.');
                     INSERT INTO findings (id, kind, severity, subject_portal_key, subject_agency_id,
                                           subject_request_id, headline, explanation, discovered_at,
                                           last_seen_at, source_detector) VALUES
                       ('f5', 'NO_RECORDS_CONTRADICTED_BY_RELEASE', 'NOTABLE', 'sd', 7, 5, 'h', 'e',
                        '2026-03-02', '2026-03-02', 'no_records_contradicted'),
                       ('f9', 'NO_RECORDS_CONTRADICTED_BY_RELEASE', 'NOTABLE', 'sd', 7, 9, 'h', 'e',
                        '2026-03-02', '2026-03-02', 'no_records_contradicted'),
                       ('foak', 'NO_RECORDS_CONTRADICTED_BY_RELEASE', 'NOTABLE', 'oak', 7, 5, 'h', 'e',
                        '2026-03-02', '2026-03-02', 'no_records_contradicted');",
                );
                conn.execute_batch(&sql)
                    .map_err(|error| AppError::Storage(format!("fixture: {error}")))
            })
            .await
            .expect("seed");
        repository
    }

    fn identity(portal: &str) -> AgencyIdentity {
        AgencyIdentity {
            portal_key: portal.into(),
            agency_id: 7,
        }
    }

    #[tokio::test]
    async fn counts_are_per_portal_and_agency_and_only_sourced_determinations_are_counted() {
        let repository = seeded().await;
        let observed = "2026-10-05T00:00:00Z".to_string();
        let sd = repository
            .agency_record(identity("sd"), observed.clone())
            .await
            .unwrap();
        assert_eq!(sd.agency_name.as_deref(), Some("Purchasing"));
        assert_eq!(sd.closed_requests, 12);
        assert_eq!(sd.sourced_determinations, 3);
        assert_eq!(sd.late_sourced_determinations, 2);
        assert_eq!(sd.no_records_closures, 2);
        // The finding on request 9 has no stored no-records notice and the one on `oak` belongs
        // to another agency, so neither enters this numerator.
        assert_eq!(sd.no_records_with_recorded_earlier_release, 1);
        assert_eq!(
            sd.period,
            Some(("2026-01-02".to_string(), "2026-01-13".to_string()))
        );
        assert!(sd.paragraph().is_some());

        // Same agency id on another portal: its own two closed requests, never pooled with `sd`.
        let oak = repository
            .agency_record(identity("oak"), observed.clone())
            .await
            .unwrap();
        assert_eq!(oak.closed_requests, 2);
        assert_eq!(oak.sourced_determinations, 1);
        assert_eq!(oak.late_sourced_determinations, 1);
        assert_eq!(oak.no_records_closures, 0);
        assert!(oak.paragraph().is_none());

        let options = repository.agency_record_options().await.unwrap();
        assert_eq!(options.len(), 1);
        assert_eq!(
            (options[0].portal_key.as_str(), options[0].closed_requests),
            ("sd", 12)
        );

        let absent = repository
            .agency_record(
                AgencyIdentity {
                    portal_key: "sd".into(),
                    agency_id: 99,
                },
                observed,
            )
            .await
            .unwrap();
        assert_eq!((absent.closed_requests, absent.period.clone()), (0, None));
        assert!(absent.paragraph().is_none());
    }
}
