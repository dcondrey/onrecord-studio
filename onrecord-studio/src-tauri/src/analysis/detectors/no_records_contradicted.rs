//! Denial-vs-corpus contradiction detector.
//!
//! For every response letter that plainly denies having responsive records ("no responsive
//! records", "no records exist", "unable to locate"), searches the rest of the corpus for public
//! documents from OTHER requests whose text mentions the same topic. When at least
//! [`MIN_CONTRADICTING_DOCS`] such documents exist, emits a finding so the reader can see the
//! agency told one requester nothing existed while releasing matching material to another.
//!
//! Guards:
//! - Scope-narrow denials ("for FY 2023, no records exist") are skipped — a contradicting match
//!   from outside that scope isn't a contradiction. See [`assertion_index::extract_denial`].
//! - Documents from the SAME request as the denial are excluded from the match set.
//! - Non-public documents (visibility != 'public') are excluded, so the finding reports only
//!   material the agency has already released.
//! - Empty or over-broad topic-token sets (fewer than [`MIN_TOPIC_TOKENS`]) are skipped, since a
//!   one-token FTS query would match half the corpus and produce false positives.

use crate::{
    analysis::{
        assertion_index::{extract_denial, extract_topic_tokens},
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;
use duckdb::params;
use std::collections::HashSet;

pub const SLUG: &str = "no_records_contradicted";

/// Max denial candidates inspected per invocation.
const MAX_CANDIDATES: i64 = 500;

/// Minimum contradicting-document count to fire.
const MIN_CONTRADICTING_DOCS: usize = 3;

/// A single-token FTS query would match too broadly. Require enough topic vocabulary to make the
/// claim of "corpus contradicts denial" credible.
const MIN_TOPIC_TOKENS: usize = 2;

/// Maximum contradicting-document rows to include in citations, to keep the inbox row compact.
const MAX_CITATION_ROWS: usize = 4;

struct Candidate {
    portal_key: String,
    request_id: i64,
    denial_document_id: i64,
    denial_file_name: Option<String>,
    request_title: String,
    request_description: String,
    denial_text: String,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let candidates = load_denial_candidates(repository).await?;
    let mut findings = Vec::new();
    for candidate in candidates {
        // Confirm the denial phrase actually lives in the text, and grab its position for the
        // citation. Guard against denial-shaped strings appearing incidentally with no scope
        // narrowing — the SQL LIKE gate above is intentionally coarse.
        let Some(fingerprint) = extract_denial(&candidate.denial_text) else {
            continue;
        };
        if fingerprint.scope_narrow {
            continue;
        }
        let tokens = extract_topic_tokens(&candidate.request_title, &candidate.request_description);
        if tokens.len() < MIN_TOPIC_TOKENS {
            continue;
        }
        let matches = search_contradicting_documents(
            repository,
            tokens.clone(),
            &candidate.portal_key,
            candidate.request_id,
        )
        .await?;
        if matches.len() < MIN_CONTRADICTING_DOCS {
            continue;
        }
        if let Some(finding) =
            build_finding(&candidate, &fingerprint.matched_phrase, &tokens, &matches)
        {
            findings.push(finding);
        }
    }
    Ok(findings)
}

async fn load_denial_candidates(repository: &DuckDbRepository) -> Result<Vec<Candidate>, AppError> {
    repository
        .query_connection("Denial-candidate query", move |conn| {
            // The LIKE gate is a coarse pre-filter; `extract_denial` re-verifies with the same
            // regex used by other callers so the true set is aligned. Ordered newest-first so a
            // hit cap surfaces the most recent evidence.
            let mut statement = conn
                .prepare(
                    "SELECT documents.portal_subdomain, documents.request_id,
                            documents.id, documents.file_name,
                            COALESCE(requests.title, ''), COALESCE(requests.description, ''),
                            documents_fts.extracted_text
                     FROM documents
                     INNER JOIN documents_fts
                        ON documents_fts.document_id = documents.id
                       AND documents_fts.portal_subdomain = documents.portal_subdomain
                     INNER JOIN requests
                        ON requests.id = documents.request_id
                       AND requests.portal_subdomain = documents.portal_subdomain
                     WHERE LOWER(TRIM(documents.visibility)) = 'public'
                       AND LENGTH(documents_fts.extracted_text) > 0
                       AND (
                           LOWER(documents_fts.extracted_text) LIKE '%no responsive records%' OR
                           LOWER(documents_fts.extracted_text) LIKE '%no records exist%' OR
                           LOWER(documents_fts.extracted_text) LIKE '%no records were found%' OR
                           LOWER(documents_fts.extracted_text) LIKE '%unable to locate%' OR
                           LOWER(documents_fts.extracted_text) LIKE '%cannot locate any records%'
                       )
                     ORDER BY documents.updated_at DESC
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare denial candidate query: {error}"))
                })?;
            let rows = statement
                .query_map(params![MAX_CANDIDATES], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run denial candidate query: {error}"))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (
                    portal_key,
                    request_id,
                    denial_document_id,
                    denial_file_name,
                    request_title,
                    request_description,
                    denial_text,
                ) = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode denial row: {error}"))
                })?;
                out.push(Candidate {
                    portal_key,
                    request_id,
                    denial_document_id,
                    denial_file_name,
                    request_title,
                    request_description,
                    denial_text,
                });
            }
            Ok(out)
        })
        .await
}

/// A contradicting-document row surfaced by the FTS search over topic tokens.
struct ContradictingMatch {
    portal_key: String,
    document_id: i64,
    request_id: i64,
    file_name: Option<String>,
}

/// Runs an all-terms conjunctive FTS query over the corpus's search-token index, then filters out
/// documents from the denying request. Reuses the same search-tokens table
/// (`document_search_tokens`) used elsewhere in the app so the semantics match search-driven
/// retrieval the user is already familiar with.
async fn search_contradicting_documents(
    repository: &DuckDbRepository,
    tokens: Vec<String>,
    denying_portal: &str,
    denying_request: i64,
) -> Result<Vec<ContradictingMatch>, AppError> {
    let denying_portal = denying_portal.to_string();
    repository
        .query_connection("Denial contradiction FTS", move |conn| {
            // Positional params: ?1 = denying_portal, ?2..?N+1 = tokens, ?N+2 = denying_request.
            let placeholders = (2..2 + tokens.len())
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let required = tokens.len();
            let denying_request_slot = 2 + tokens.len();
            let query = format!(
                "SELECT documents.portal_subdomain, documents.id, documents.request_id,
                        documents.file_name
                 FROM documents
                 INNER JOIN (
                     SELECT document_id, portal_subdomain
                     FROM document_search_tokens
                     WHERE token IN ({placeholders})
                     GROUP BY document_id, portal_subdomain
                     HAVING COUNT(*) = {required}
                 ) AS matched
                    ON matched.document_id = documents.id
                   AND matched.portal_subdomain = documents.portal_subdomain
                 WHERE LOWER(TRIM(documents.visibility)) = 'public'
                   AND NOT (documents.portal_subdomain = ?1
                            AND documents.request_id = ?{denying_request_slot})
                 ORDER BY documents.updated_at DESC
                 LIMIT 32"
            );
            let mut statement = conn.prepare(&query).map_err(|error| {
                AppError::Storage(format!("Failed to prepare contradiction FTS: {error}"))
            })?;
            let mut params_vec: Vec<Box<dyn duckdb::ToSql>> = Vec::with_capacity(tokens.len() + 2);
            params_vec.push(Box::new(denying_portal.clone()));
            for token in &tokens {
                params_vec.push(Box::new(token.clone()));
            }
            params_vec.push(Box::new(denying_request));
            let refs: Vec<&dyn duckdb::ToSql> =
                params_vec.iter().map(|boxed| boxed.as_ref()).collect();
            let rows = statement
                .query_map(refs.as_slice(), |row| {
                    Ok(ContradictingMatch {
                        portal_key: row.get(0)?,
                        document_id: row.get(1)?,
                        request_id: row.get(2)?,
                        file_name: row.get::<_, Option<String>>(3)?,
                    })
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run contradiction FTS: {error}"))
                })?;
            let mut out = Vec::new();
            let mut seen_requests: HashSet<(String, i64)> = HashSet::new();
            for row in rows {
                let matched = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode contradiction FTS row: {error}"))
                })?;
                // Deduplicate by (portal, request) so multiple documents in the same OTHER request
                // count once — the finding is "N other requests contain matching material", not
                // "N documents".
                let key = (matched.portal_key.clone(), matched.request_id);
                if seen_requests.insert(key) {
                    out.push(matched);
                }
            }
            Ok(out)
        })
        .await
}

fn build_finding(
    candidate: &Candidate,
    matched_phrase: &str,
    tokens: &[String],
    matches: &[ContradictingMatch],
) -> Option<Finding> {
    if matches.len() < MIN_CONTRADICTING_DOCS {
        return None;
    }
    // Base 6000bp at MIN_CONTRADICTING_DOCS, +500bp per additional distinct matching request,
    // capped at 9500bp.
    let extra = matches.len().saturating_sub(MIN_CONTRADICTING_DOCS);
    let confidence_bp = u16::min(6000 + (extra as u16).saturating_mul(500), 9500);
    let confidence = FindingConfidence::measured(confidence_bp);

    let subject = FindingSubject {
        portal_key: Some(candidate.portal_key.clone()),
        request_id: Some(candidate.request_id),
        document_id: Some(candidate.denial_document_id),
        ..Default::default()
    };

    let id = finding_id(
        FindingKind::NoRecordsContradictedByRelease,
        SLUG,
        &subject,
        &[&format!(
            "n_matches={};tokens={}",
            matches.len(),
            tokens.join("+")
        )],
    );

    let display_title = if candidate.request_title.is_empty() {
        format!("request {}", candidate.request_id)
    } else {
        candidate.request_title.clone()
    };
    let headline = format!(
        "Denial for '{display_title}' contradicted by {n} other public records",
        n = matches.len()
    );

    let token_list = tokens.join(", ");
    let explanation = format!(
        "The response to request {request_id} ('{display_title}') asserts \"{phrase}\". \
         A search of {n} other requests' public documents on the same topic ({token_list}) \
         returned {n_matches} contradicting matches. If those matches concern the same subject \
         the denial covers, the denial's factual basis needs review.",
        request_id = candidate.request_id,
        phrase = matched_phrase.trim(),
        n = matches.len(),
        n_matches = matches.len(),
    );

    let mut citations = Vec::with_capacity(1 + MAX_CITATION_ROWS);
    citations.push(FindingCitation {
        portal_key: candidate.portal_key.clone(),
        document_id: candidate.denial_document_id,
        character_start: None,
        character_end: None,
        quote: Some(matched_phrase.trim().to_string()),
        label: Some(format!(
            "Denial: {}",
            candidate
                .denial_file_name
                .clone()
                .unwrap_or_else(|| format!("document {}", candidate.denial_document_id))
        )),
    });
    for matched in matches.iter().take(MAX_CITATION_ROWS) {
        citations.push(FindingCitation {
            portal_key: matched.portal_key.clone(),
            document_id: matched.document_id,
            character_start: None,
            character_end: None,
            quote: None,
            label: Some(format!(
                "Contradicting release (request {}): {}",
                matched.request_id,
                matched
                    .file_name
                    .clone()
                    .unwrap_or_else(|| format!("document {}", matched.document_id))
            )),
        });
    }

    let now = Utc::now();
    Some(Finding {
        id,
        kind: FindingKind::NoRecordsContradictedByRelease,
        severity: FindingSeverity::Serious,
        confidence,
        subject,
        headline,
        explanation,
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(text: &str, title: &str, desc: &str) -> Candidate {
        Candidate {
            portal_key: "test".into(),
            request_id: 42,
            denial_document_id: 100,
            denial_file_name: Some("denial.pdf".into()),
            request_title: title.into(),
            request_description: desc.into(),
            denial_text: text.into(),
        }
    }

    fn one_match(id: i64, req: i64) -> ContradictingMatch {
        ContradictingMatch {
            portal_key: "test".into(),
            document_id: id,
            request_id: req,
            file_name: Some(format!("rel_{id}.pdf")),
        }
    }

    #[test]
    fn build_finding_returns_none_below_minimum() {
        let c = candidate(
            "After a diligent search, no responsive records were located.",
            "Contracts with Downtown Redevelopment",
            "Provide all contracts",
        );
        // Only 2 matches - below MIN_CONTRADICTING_DOCS.
        let matches = vec![one_match(1, 51), one_match(2, 52)];
        assert!(build_finding(
            &c,
            "no responsive records",
            &["contracts".into(), "downtown".into()],
            &matches
        )
        .is_none());
    }

    #[test]
    fn build_finding_serious_with_three_contradicting_matches() {
        let c = candidate(
            "After a diligent search, no responsive records were located.",
            "Contracts with Downtown Redevelopment",
            "Provide all contracts",
        );
        let matches = vec![one_match(1, 51), one_match(2, 52), one_match(3, 53)];
        let finding = build_finding(
            &c,
            "no responsive records",
            &[
                "contracts".into(),
                "downtown".into(),
                "redevelopment".into(),
            ],
            &matches,
        )
        .expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Serious);
        assert_eq!(finding.kind, FindingKind::NoRecordsContradictedByRelease);
        // 1 denial + 3 contradicting citations.
        assert_eq!(finding.citations.len(), 4);
        assert!(finding.citations[0]
            .label
            .as_ref()
            .unwrap()
            .contains("Denial"));
        assert!(finding.headline.contains("Downtown"));
    }

    #[test]
    fn confidence_rises_with_more_contradicting_requests() {
        let c = candidate("no records exist", "Housing contracts", "Provide records");
        let three = vec![one_match(1, 1), one_match(2, 2), one_match(3, 3)];
        let seven = vec![
            one_match(1, 1),
            one_match(2, 2),
            one_match(3, 3),
            one_match(4, 4),
            one_match(5, 5),
            one_match(6, 6),
            one_match(7, 7),
        ];
        let base = build_finding(
            &c,
            "no records exist",
            &["housing".into(), "contracts".into()],
            &three,
        )
        .unwrap();
        let strong = build_finding(
            &c,
            "no records exist",
            &["housing".into(), "contracts".into()],
            &seven,
        )
        .unwrap();
        assert!(strong.confidence.rank_weight() > base.confidence.rank_weight());
    }

    #[test]
    fn scope_narrow_denial_would_be_skipped_by_run() {
        // Direct test of extract_denial exercised in assertion_index tests; this covers the
        // integration decision: a narrow denial produces a fingerprint with scope_narrow=true,
        // which run() checks before calling build_finding. Verify the primitive here.
        let narrow = extract_denial("For FY 2023, no responsive records exist.").unwrap();
        assert!(narrow.scope_narrow);
    }
}
