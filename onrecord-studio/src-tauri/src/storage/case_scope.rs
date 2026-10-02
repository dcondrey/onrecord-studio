//! What one case covers, for scoping views: the documents its sources name, the requests that
//! hold those documents, and the undismissed findings that cite one of those documents or are
//! about one of them or their requests. Derived from `casework_sources`, which a case save keeps
//! in step with the case's cited evidence.

use super::sqlite::DuckDbRepository;
use crate::{analysis::finding::FindingCitation, error::AppError};
use duckdb::params;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseDocumentKey {
    pub portal_key: String,
    pub document_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseRequestKey {
    pub portal_key: String,
    pub request_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseScope {
    pub case_id: String,
    pub documents: Vec<CaseDocumentKey>,
    /// Requests holding a case document. A source whose document is no longer stored contributes
    /// its document key but no request.
    pub requests: Vec<CaseRequestKey>,
    pub finding_ids: Vec<String>,
}

fn storage(error: impl std::fmt::Display) -> AppError {
    AppError::Storage(error.to_string())
}

impl DuckDbRepository {
    pub async fn case_scope(&self, case_id: String) -> Result<CaseScope, AppError> {
        self.query_connection("Read case scope", move |conn| {
            let documents = conn
                .prepare("SELECT portal_key, document_id FROM casework_sources WHERE case_id = ?1")
                .map_err(storage)?
                .query_map(params![case_id], |row| {
                    Ok(CaseDocumentKey { portal_key: row.get(0)?, document_id: row.get(1)? })
                })
                .map_err(storage)?
                .collect::<Result<BTreeSet<_>, _>>()
                .map_err(storage)?;
            let requests = conn
                .prepare(
                    "SELECT DISTINCT d.portal_subdomain, d.request_id
                     FROM casework_sources s
                     JOIN documents d ON d.id = s.document_id AND d.portal_subdomain = s.portal_key
                     WHERE s.case_id = ?1",
                )
                .map_err(storage)?
                .query_map(params![case_id], |row| {
                    Ok(CaseRequestKey { portal_key: row.get(0)?, request_id: row.get(1)? })
                })
                .map_err(storage)?
                .collect::<Result<BTreeSet<_>, _>>()
                .map_err(storage)?;

            let mut finding_ids = Vec::new();
            if !documents.is_empty() {
                let mut statement = conn
                    .prepare(
                        "SELECT id, subject_portal_key, subject_request_id, subject_document_id, citations_json
                         FROM findings WHERE dismissed_at IS NULL ORDER BY id",
                    )
                    .map_err(storage)?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<i64>>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })
                    .map_err(storage)?;
                for row in rows {
                    let (id, portal, request, document, citations_json) = row.map_err(storage)?;
                    let about_document = matches!((&portal, document), (Some(portal), Some(document))
                        if documents.contains(&CaseDocumentKey { portal_key: portal.clone(), document_id: document }));
                    let about_request = matches!((&portal, request), (Some(portal), Some(request))
                        if requests.contains(&CaseRequestKey { portal_key: portal.clone(), request_id: request }));
                    // A row whose citations cannot be parsed is judged on its subject alone.
                    let cites_document = serde_json::from_str::<Vec<FindingCitation>>(&citations_json)
                        .map(|citations| {
                            citations.into_iter().any(|citation| {
                                documents.contains(&CaseDocumentKey {
                                    portal_key: citation.portal_key,
                                    document_id: citation.document_id,
                                })
                            })
                        })
                        .unwrap_or(false);
                    if about_document || about_request || cites_document {
                        finding_ids.push(id);
                    }
                }
            }

            Ok(CaseScope {
                case_id,
                documents: documents.into_iter().collect(),
                requests: requests.into_iter().collect(),
                finding_ids,
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::finding::{
        finding_id, Finding, FindingConfidence, FindingKind, FindingSeverity, FindingSubject,
    };
    use chrono::Utc;

    fn finding(headline: &str, subject: FindingSubject, citations: Vec<FindingCitation>) -> Finding {
        let id = finding_id(FindingKind::HeldBackResponse, "held_back_response", &subject, &[headline]);
        let now = Utc::now();
        Finding {
            id,
            kind: FindingKind::HeldBackResponse,
            severity: FindingSeverity::Notable,
            confidence: FindingConfidence::Unassessed,
            subject,
            headline: headline.into(),
            explanation: String::new(),
            citations,
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: "held_back_response".into(),
        }
    }

    fn citation(portal: &str, document: i64) -> FindingCitation {
        FindingCitation {
            portal_key: portal.into(),
            document_id: document,
            character_start: None,
            character_end: None,
            quote: None,
            label: None,
        }
    }

    #[tokio::test]
    async fn scope_covers_findings_that_cite_or_concern_case_documents_and_their_requests_only() {
        let repo = DuckDbRepository::new(":memory:").expect("open in-memory core");
        repo.run_migrations(include_str!("../../migrations/0001_initial_schema.sql")).await.unwrap();
        repo.initialize_casework().await.unwrap();
        repo.query_connection("Seed case scope fixture", |conn| {
            conn.execute_batch(
                "INSERT INTO portals(subdomain,display_name,base_url) VALUES ('city','City','https://example.gov');
                 INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at) VALUES (42,'city','R','closed','2026-09-26','2026-09-26');
                 INSERT INTO documents(id,request_id,portal_subdomain,file_name,visibility,created_at,updated_at) VALUES
                   (1,42,'city','in-case.pdf','public','2026-09-26','2026-09-26'),
                   (2,42,'city','same-request.pdf','public','2026-09-26','2026-09-26'),
                   (3,77,'city','elsewhere.pdf','public','2026-09-26','2026-09-26');
                 INSERT INTO casework_sources VALUES ('case-a','city',1,''), ('case-a','city',99,'');",
            )
            .map_err(storage)
        })
        .await
        .unwrap();

        let cites = finding("cites", FindingSubject::default(), vec![citation("city", 1)]);
        let request = finding("request", FindingSubject { portal_key: Some("city".into()), request_id: Some(42), ..Default::default() }, vec![]);
        let other_portal = finding("other portal", FindingSubject::default(), vec![citation("county", 1)]);
        let elsewhere = finding("elsewhere", FindingSubject { portal_key: Some("city".into()), document_id: Some(3), ..Default::default() }, vec![citation("city", 3)]);
        let dismissed = finding("dismissed", FindingSubject::default(), vec![citation("city", 1)]);
        let dismissed_id = dismissed.id.clone();
        let mut expected = vec![cites.id.clone(), request.id.clone()];
        expected.sort();
        repo.record_findings(vec![cites, request, other_portal, elsewhere, dismissed]).await.unwrap();
        repo.dismiss_finding(dismissed_id).await.unwrap();

        let scope = repo.case_scope("case-a".into()).await.unwrap();
        assert_eq!(scope.documents, vec![
            CaseDocumentKey { portal_key: "city".into(), document_id: 1 },
            CaseDocumentKey { portal_key: "city".into(), document_id: 99 },
        ]);
        assert_eq!(scope.requests, vec![CaseRequestKey { portal_key: "city".into(), request_id: 42 }], "a missing document adds no request");
        assert_eq!(scope.finding_ids, expected);

        let empty = repo.case_scope("no-such-case".into()).await.unwrap();
        assert!(empty.documents.is_empty() && empty.requests.is_empty() && empty.finding_ids.is_empty());
    }
}
