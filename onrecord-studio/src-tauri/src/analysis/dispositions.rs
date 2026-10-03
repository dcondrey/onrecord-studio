//! How each department's requests were closed, counted from the agency's own closure notice.
//!
//! A descriptive count, not a finding: a share of requests closed "no responsive documents" or
//! withheld says what the agency answered, never that the answer was wrong. Two kinds of absence
//! are counted on their own and never folded into a disposition: a request with no stored closure
//! notice (still open, or closed without one) and a notice whose wording no rule recognises.

use std::collections::BTreeMap;

use duckdb::params;
use serde::{Deserialize, Serialize};

use crate::analysis::phrase_match::{has_phrase, words, Phrases};
use crate::error::AppError;
use crate::storage::DuckDbRepository;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Disposition {
    Released,
    ReleasedInPart,
    Withheld,
    NoRecords,
    Withdrawn,
    /// A closure notice is stored but its wording matches no rule, or its text is not extracted.
    NotClassified,
    /// No closure notice is stored for the request.
    NotRecorded,
}

const ALL: [Disposition; 7] = [
    Disposition::Released,
    Disposition::ReleasedInPart,
    Disposition::Withheld,
    Disposition::NoRecords,
    Disposition::Withdrawn,
    Disposition::NotClassified,
    Disposition::NotRecorded,
];

const WITHDRAWN: Phrases = &[&["request", "withdrawn"], &["withdrawn", "this", "request"]];
const NO_RECORDS: Phrases = &[
    &["no", "responsive", "documents"],
    &["no", "responsive", "records"],
];
const WITHHELD: Phrases = &[
    &["not", "released"],
    &[
        "any",
        "documents",
        "responsive",
        "to",
        "the",
        "request",
        "are",
        "being",
        "withheld",
    ],
];
const RELEASED_IN_PART: Phrases = &[
    &["released", "except"],
    &["released", "redacted"],
    &["released", "withheld"],
];
const RELEASED: Phrases = &[&["released"]];

/// First matching rule wins, in this order. The order is the rule: every partial-release notice
/// also says documents "have been released", a full withholding is labelled "Not Released -
/// Withheld" (which contains "released withheld"), and partial-release notices quote exemption
/// text saying documents "are being withheld from disclosure".
const RULES: [(Disposition, Phrases); 5] = [
    (Disposition::Withdrawn, WITHDRAWN),
    (Disposition::NoRecords, NO_RECORDS),
    (Disposition::Withheld, WITHHELD),
    (Disposition::ReleasedInPart, RELEASED_IN_PART),
    (Disposition::Released, RELEASED),
];

/// The disposition a closure notice states, or `None` when no rule recognises its wording.
pub fn classify_closure_notice(notice: &str) -> Option<Disposition> {
    let tokens = words(notice);
    RULES
        .iter()
        .find(|(_, phrases)| has_phrase(&tokens, phrases))
        .map(|(disposition, _)| *disposition)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DispositionRequest {
    pub request_id: i64,
    pub rpid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DispositionBucket {
    pub disposition: Disposition,
    pub requests: Vec<DispositionRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DepartmentDispositions {
    pub portal_subdomain: String,
    /// `None` when the request names no department; such requests form their own row.
    pub agency_id: Option<i64>,
    pub agency_name: Option<String>,
    /// Every request of the department in scope: the denominator of every share.
    pub total_requests: u64,
    /// One bucket per [`Disposition`], in declaration order, including empty ones.
    pub buckets: Vec<DispositionBucket>,
}

struct RequestRow {
    portal: String,
    agency_id: Option<i64>,
    agency_name: Option<String>,
    request: DispositionRequest,
    status: String,
    has_notice: bool,
    notice_text: Option<String>,
}

fn disposition_of(row: &RequestRow) -> Disposition {
    if row.status.eq_ignore_ascii_case("withdrawn") {
        return Disposition::Withdrawn;
    }
    if !row.has_notice {
        return Disposition::NotRecorded;
    }
    row.notice_text
        .as_deref()
        .and_then(classify_closure_notice)
        .unwrap_or(Disposition::NotClassified)
}

fn group(rows: Vec<RequestRow>) -> Vec<DepartmentDispositions> {
    let mut departments: BTreeMap<(String, Option<i64>), DepartmentDispositions> = BTreeMap::new();
    for row in rows {
        let disposition = disposition_of(&row);
        let department = departments
            .entry((row.portal.clone(), row.agency_id))
            .or_insert_with(|| DepartmentDispositions {
                portal_subdomain: row.portal.clone(),
                agency_id: row.agency_id,
                agency_name: row.agency_name.clone(),
                total_requests: 0,
                buckets: ALL
                    .iter()
                    .map(|disposition| DispositionBucket {
                        disposition: *disposition,
                        requests: Vec::new(),
                    })
                    .collect(),
            });
        department.total_requests += 1;
        department
            .buckets
            .iter_mut()
            .find(|bucket| bucket.disposition == disposition)
            .expect("every disposition has a bucket")
            .requests
            .push(row.request);
    }
    let mut departments: Vec<_> = departments.into_values().collect();
    departments.sort_by(|left, right| {
        left.portal_subdomain
            .cmp(&right.portal_subdomain)
            .then(right.total_requests.cmp(&left.total_requests))
            .then(left.agency_name.cmp(&right.agency_name))
    });
    departments
}

impl DuckDbRepository {
    /// Disposition counts per department, for one portal or every portal.
    pub async fn department_dispositions(
        &self,
        portal_subdomain: Option<String>,
    ) -> Result<Vec<DepartmentDispositions>, AppError> {
        let rows = self
            .query_connection("Count department dispositions", move |conn| {
                let storage = |error: duckdb::Error| {
                    AppError::Storage(format!("Failed to count dispositions: {error}"))
                };
                // The closure notice is the document `ingestion::queue::crawl::closure_notice_task`
                // stores; both its description and its file name are matched.
                let mut statement = conn
                    .prepare(
                        "WITH notices AS (
                             SELECT d.request_id, d.portal_subdomain, f.extracted_text,
                                    ROW_NUMBER() OVER (
                                        PARTITION BY d.request_id, d.portal_subdomain
                                        ORDER BY d.created_at DESC, d.id
                                    ) AS position
                             FROM documents d
                             LEFT JOIN documents_fts f
                               ON f.document_id = d.id AND f.portal_subdomain = d.portal_subdomain
                             WHERE d.description LIKE 'NextRequest closure notice for request %'
                               AND d.file_name LIKE 'closure-notice-%'
                         )
                         SELECT r.portal_subdomain, r.agency_id, a.name, r.id, r.rpid, r.status,
                                n.request_id IS NOT NULL, n.extracted_text
                         FROM requests r
                         LEFT JOIN agencies a
                           ON a.id = r.agency_id AND a.portal_subdomain = r.portal_subdomain
                         LEFT JOIN notices n
                           ON n.request_id = r.id AND n.portal_subdomain = r.portal_subdomain
                          AND n.position = 1
                         WHERE CAST(?1 AS VARCHAR) IS NULL OR r.portal_subdomain = ?1
                         ORDER BY r.portal_subdomain, r.id",
                    )
                    .map_err(storage)?;
                let rows = statement
                    .query_map(params![portal_subdomain], |row| {
                        Ok(RequestRow {
                            portal: row.get(0)?,
                            agency_id: row.get(1)?,
                            agency_name: row.get(2)?,
                            request: DispositionRequest {
                                request_id: row.get(3)?,
                                rpid: row.get(4)?,
                            },
                            status: row.get(5)?,
                            has_notice: row.get(6)?,
                            notice_text: row.get(7)?,
                        })
                    })
                    .map_err(storage)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(storage)?;
                Ok(rows)
            })
            .await?;
        Ok(group(rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Notice wording as the San Diego NextRequest portal published it, read from the encampment
    /// collection of 2026-10-03 (one example per wording; exemption citations abridged only where
    /// marked "…").
    #[test]
    fn san_diego_closure_notices_classify_by_their_stated_disposition() {
        let cases = [
            ("02. Released All responsive documents have been released pursuant to the California Public Records Act.", Some(Disposition::Released)),
            ("All responsive documents have been released pursuant to the California Public Records Act.", Some(Disposition::Released)),
            ("02. Released", Some(Disposition::Released)),
            ("02a. Released - Redacted All responsive documents have been released except for portions redacted pursuant to: Responsive document(s) are being withheld from disclosure pursuant to Government Code section 7923.600", Some(Disposition::ReleasedInPart)),
            ("02b. Released - Withheld All responsive documents have been released except those that have been withheld pursuant to: [Law enforcement investigation] Government Code section 7923.600 …", Some(Disposition::ReleasedInPart)),
            ("02c. Released – Redacted and/or Withheld All responsive documents have been released except those that have been redacted and/or withheld pursuant to: Government Code section 7923.600 [investigatory files] … Responsive document(s) are being withheld from disclosure pursuant to Government Code section 7922.000", Some(Disposition::ReleasedInPart)),
            ("01a. Not Released - Withheld Any documents responsive to the request are being withheld pursuant to: [Law enforcement investigation] Government Code section 7923.600 …", Some(Disposition::Withheld)),
            ("No responsive documents The City of San Diego has no responsive documents.", Some(Disposition::NoRecords)),
            ("Request withdrawn The requestor has withdrawn this request for public records.", Some(Disposition::Withdrawn)),
            ("Your request has been closed.", None),
        ];
        for (notice, expected) in cases {
            assert_eq!(classify_closure_notice(notice), expected, "{notice}");
        }
    }

    async fn seeded() -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let conn = repository.analytics_connection().await.unwrap();
        conn.execute_batch(
            "INSERT INTO portals (subdomain, display_name, base_url) VALUES ('sd', 'SD', 'https://sd.example'), ('la', 'LA', 'https://la.example');
             INSERT INTO agencies (id, portal_subdomain, name) VALUES (3, 'sd', 'Police');
             INSERT INTO requests (id, portal_subdomain, rpid, status, created_at, updated_at, agency_id) VALUES
               (1, 'sd', '24-1', 'closed', '2024-03-01', '2024-03-02', 3),
               (2, 'sd', '24-2', 'closed', '2024-03-01', '2024-03-02', 3),
               (3, 'sd', '24-3', 'open',   '2024-03-01', '2024-03-02', 3),
               (4, 'sd', '24-4', 'closed', '2024-03-01', '2024-03-02', 3),
               (5, 'sd', '24-5', 'closed', '2024-03-01', '2024-03-02', NULL),
               (6, 'la', '24-6', 'closed', '2024-03-01', '2024-03-02', NULL);
             INSERT INTO documents (id, request_id, portal_subdomain, file_name, description, visibility, created_at, updated_at) VALUES
               (11, 1, 'sd', 'closure-notice-24-1.txt', 'NextRequest closure notice for request 24-1', 'public', '2024-03-02', '2024-03-02'),
               (12, 2, 'sd', 'closure-notice-24-2.txt', 'NextRequest closure notice for request 24-2', 'public', '2024-03-02', '2024-03-02'),
               (14, 4, 'sd', 'closure-notice-24-4.txt', 'NextRequest closure notice for request 24-4', 'public', '2024-03-02', '2024-03-02'),
               (15, 5, 'sd', 'closure-notice-24-5.txt', 'NextRequest closure notice for request 24-5', 'public', '2024-03-02', '2024-03-02'),
               (21, 1, 'sd', 'released.pdf', 'No responsive documents', 'public', '2024-03-02', '2024-03-02');
             INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text) VALUES
               (11, 'sd', 'closure-notice-24-1.txt', 'No responsive documents The City of San Diego has no responsive documents.'),
               (12, 'sd', 'closure-notice-24-2.txt', 'Your request has been closed.'),
               (15, 'sd', 'closure-notice-24-5.txt', '02. Released All responsive documents have been released pursuant to the California Public Records Act.'),
               (21, 'sd', 'released.pdf', 'Request withdrawn');",
        )
        .unwrap();
        repository
    }

    fn requests(department: &DepartmentDispositions, disposition: Disposition) -> Vec<i64> {
        department
            .buckets
            .iter()
            .find(|bucket| bucket.disposition == disposition)
            .unwrap()
            .requests
            .iter()
            .map(|request| request.request_id)
            .collect()
    }

    /// A request with no notice, a notice no rule recognises, and a notice not yet extracted are
    /// each counted as absence, never as a disposition; a request naming no department keeps its
    /// own row; and every request is in exactly one bucket of its row.
    #[tokio::test]
    async fn absent_and_unrecognised_dispositions_are_counted_apart_from_any_disposition() {
        let repository = seeded().await;
        let rows = repository
            .department_dispositions(Some("sd".into()))
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        let police = rows
            .iter()
            .find(|row| row.agency_id == Some(3))
            .expect("Police");
        assert_eq!(police.total_requests, 4);
        assert_eq!(requests(police, Disposition::NoRecords), vec![1]);
        assert_eq!(requests(police, Disposition::NotClassified), vec![2, 4]);
        assert_eq!(requests(police, Disposition::NotRecorded), vec![3]);
        let unnamed = rows.iter().find(|row| row.agency_id.is_none()).unwrap();
        assert_eq!(unnamed.agency_name, None);
        assert_eq!(requests(unnamed, Disposition::Released), vec![5]);
        for row in &rows {
            let bucketed: usize = row.buckets.iter().map(|bucket| bucket.requests.len()).sum();
            assert_eq!(bucketed as u64, row.total_requests);
        }
        assert_eq!(
            repository
                .department_dispositions(None)
                .await
                .unwrap()
                .len(),
            3
        );
    }
}
