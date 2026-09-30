//! Corpus-wide "same claim, different value" contradiction detector.
//!
//! Runs [`assertion_index::extract_assertions`] over every public document with usable extracted
//! text, groups the resulting `(subject, predicate)` pairs across documents, and emits one
//! finding per group in which two documents supply materially different values for the same
//! claim. This is not a legal-claim finding: it says "these two pieces of paper disagree with
//! each other," not "the organization violated a stated rule."
//!
//! Boundaries:
//! - Iterates at most [`MAX_DOCUMENTS`] documents per invocation, newest first, so a huge corpus
//!   doesn't stall the refresh.
//! - Emits at most [`MAX_FINDINGS`] findings per invocation. Contradictions can be surprisingly
//!   common when the corpus is large; surfacing every one would drown the inbox. Trimmed by
//!   confidence descending so the strongest evidence surfaces first.
//! - A `(subject, predicate)` group produces AT MOST one finding, listing every document
//!   involved in citations, rather than N-choose-2 pairwise findings for the same disagreement.

use crate::{
    analysis::{
        assertion_index::{extract_assertions, Assertion, AssertionPredicate},
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
use std::collections::HashMap;

pub const SLUG: &str = "contradictory_statement";

/// Bound on the corpus sweep. 5000 public documents cover any San Diego portal snapshot the
/// current adapters produce; larger corpora will need incremental sweeps.
const MAX_DOCUMENTS: i64 = 5000;

/// Bound on findings emitted per run. See module docs.
const MAX_FINDINGS: usize = 200;

/// A single assertion tied to its source document.
struct SourcedAssertion {
    portal_key: String,
    document_id: i64,
    request_id: i64,
    file_name: Option<String>,
    likely_authoritative: bool,
    assertion: Assertion,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let documents = load_documents(repository).await?;
    let mut by_group: HashMap<(String, AssertionPredicate), Vec<SourcedAssertion>> = HashMap::new();
    for document in documents {
        for assertion in extract_assertions(&document.extracted_text) {
            let key = (assertion.subject.clone(), assertion.predicate);
            let sourced = SourcedAssertion {
                portal_key: document.portal_key.clone(),
                document_id: document.document_id,
                request_id: document.request_id,
                file_name: document.file_name.clone(),
                likely_authoritative: document.likely_authoritative,
                assertion,
            };
            by_group.entry(key).or_default().push(sourced);
        }
    }

    let mut findings = Vec::new();
    for ((subject, predicate), rows) in by_group {
        if rows.len() < 2 {
            continue;
        }
        // A contradiction exists if any two rows in the group differ materially.
        let mut materially_conflict = false;
        for i in 0..rows.len() {
            for j in (i + 1)..rows.len() {
                if rows[i].document_id == rows[j].document_id
                    && rows[i].portal_key == rows[j].portal_key
                {
                    continue; // same document restating the same claim isn't a contradiction
                }
                if rows[i]
                    .assertion
                    .value
                    .materially_differs(&rows[j].assertion.value)
                {
                    materially_conflict = true;
                    break;
                }
            }
            if materially_conflict {
                break;
            }
        }
        if !materially_conflict {
            continue;
        }
        if let Some(finding) = build_finding(&subject, predicate, rows) {
            findings.push(finding);
        }
    }

    // Rank by (severity desc, confidence desc) then keep the top MAX_FINDINGS. Emit oldest-first
    // within the surviving set so successive runs' upserts are order-stable.
    findings.sort_by(|a, b| {
        b.severity
            .rank_weight()
            .cmp(&a.severity.rank_weight())
            .then_with(|| b.confidence.rank_weight().cmp(&a.confidence.rank_weight()))
    });
    findings.truncate(MAX_FINDINGS);
    Ok(findings)
}

struct DocumentRow {
    portal_key: String,
    document_id: i64,
    request_id: i64,
    file_name: Option<String>,
    likely_authoritative: bool,
    extracted_text: String,
}

async fn load_documents(repository: &DuckDbRepository) -> Result<Vec<DocumentRow>, AppError> {
    repository
        .query_connection("Contradictory-statement corpus sweep", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT documents.portal_subdomain, documents.id, documents.request_id,
                            documents.file_name, documents.content_type, documents.file_size,
                            documents_fts.extracted_text
                     FROM documents
                     INNER JOIN documents_fts
                        ON documents_fts.document_id = documents.id
                       AND documents_fts.portal_subdomain = documents.portal_subdomain
                     WHERE LOWER(TRIM(documents.visibility)) = 'public'
                       AND COALESCE(documents.extracted_text_sha256, '') <> ''
                       AND LENGTH(documents_fts.extracted_text) > 0
                     ORDER BY documents.updated_at DESC
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare contradiction sweep query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![MAX_DOCUMENTS], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run contradiction sweep query: {error}"))
                })?;
            let mut out = Vec::new();
            for row in rows {
                let (
                    portal_key,
                    document_id,
                    request_id,
                    file_name,
                    content_type,
                    file_size,
                    extracted_text,
                ) = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode contradiction row: {error}"))
                })?;
                let likely_authoritative =
                    infer_authoritative(&file_name, &content_type, file_size);
                out.push(DocumentRow {
                    portal_key,
                    document_id,
                    request_id,
                    file_name,
                    likely_authoritative,
                    extracted_text,
                });
            }
            Ok(out)
        })
        .await
}

/// A rough "is this document authoritative" heuristic. A contradiction between two authoritative
/// documents (policies, handbooks, MOUs, resolutions) is more serious than one between two ad-hoc
/// letters; conversely, correspondence often has minor drift that isn't worth surfacing at
/// Serious rank. The file-name signal is deliberately narrow: matching "policy"/"handbook"/
/// "manual"/"MOU"/"resolution"/"contract" produces a true positive; anything else stays false.
fn infer_authoritative(
    file_name: &Option<String>,
    _content_type: &Option<String>,
    _file_size: Option<i64>,
) -> bool {
    let Some(name) = file_name else {
        return false;
    };
    let lower = name.to_lowercase();
    [
        "policy",
        "handbook",
        "manual",
        "mou",
        "resolution",
        "contract",
        "agreement",
        "ordinance",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn build_finding(
    subject: &str,
    predicate: AssertionPredicate,
    rows: Vec<SourcedAssertion>,
) -> Option<Finding> {
    // Pick two distinct-value exemplars to lead the explanation; keep every row as a citation so
    // the user can trace the disagreement across all involved documents.
    let (a_index, b_index) = find_conflict_pair(&rows)?;
    let a = &rows[a_index];
    let b = &rows[b_index];

    // Severity: both exemplars from authoritative documents (policies, MOUs, resolutions) is
    // Serious. Otherwise Notable. Reserved Critical for the flagship policy-vs-practice detector.
    let severity = if a.likely_authoritative && b.likely_authoritative {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };

    // Confidence: distinct-document count strengthens the signal, capped at 5 distinct sources.
    // Two documents => 5000 bp; 5+ documents => 9000 bp.
    let distinct = distinct_document_count(&rows);
    let confidence = FindingConfidence::measured(4000 + (u16::min(distinct, 5) - 2) * 1250);

    // The subject anchors on the two exemplars' request context; the primary subject on the
    // Finding is portal-scoped so the inbox can filter, but the row-level detail stays in
    // citations.
    let subject_ref = FindingSubject {
        portal_key: Some(a.portal_key.clone()),
        request_id: Some(a.request_id),
        document_id: Some(a.document_id),
        ..Default::default()
    };

    let salient_summary = format!(
        "subject={subject};predicate={predicate:?};a={a_value};b={b_value};n={distinct}",
        a_value = a.assertion.value.display(),
        b_value = b.assertion.value.display(),
    );
    let id = finding_id(
        FindingKind::ContradictoryStatement,
        SLUG,
        &subject_ref,
        &[&salient_summary],
    );

    let display_subject = subject.replace("::", " ");
    let headline = format!(
        "'{display_subject}' {label}: {a} in one document, {b} in another",
        label = predicate.as_label(),
        a = a.assertion.value.display(),
        b = b.assertion.value.display(),
    );
    let explanation = format!(
        "The corpus contains {distinct} documents that state a {label} for '{display_subject}', \
         and they disagree. '{a_file}' (request {a_req}) reports {a_val}; '{b_file}' \
         (request {b_req}) reports {b_val}. Contradictions between two authoritative-looking \
         documents are surfaced at higher severity; drift between routine correspondence is \
         surfaced at lower severity.",
        distinct = distinct,
        label = predicate.as_label(),
        a_file = a
            .file_name
            .clone()
            .unwrap_or_else(|| format!("document {}", a.document_id)),
        a_req = a.request_id,
        a_val = a.assertion.value.display(),
        b_file = b
            .file_name
            .clone()
            .unwrap_or_else(|| format!("document {}", b.document_id)),
        b_req = b.request_id,
        b_val = b.assertion.value.display(),
    );

    let citations = rows
        .iter()
        .take(6) // cap on inbox footprint per finding
        .map(|row| FindingCitation {
            portal_key: row.portal_key.clone(),
            document_id: row.document_id,
            character_start: Some(row.assertion.char_start as i64),
            character_end: Some(row.assertion.char_end as i64),
            quote: None,
            label: Some(format!(
                "{} reports {}",
                row.file_name
                    .clone()
                    .unwrap_or_else(|| format!("document {}", row.document_id)),
                row.assertion.value.display()
            )),
        })
        .collect();

    let now = Utc::now();
    Some(Finding {
        id,
        kind: FindingKind::ContradictoryStatement,
        severity,
        confidence,
        subject: subject_ref,
        headline,
        explanation,
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.into(),
    })
}

fn find_conflict_pair(rows: &[SourcedAssertion]) -> Option<(usize, usize)> {
    for i in 0..rows.len() {
        for j in (i + 1)..rows.len() {
            if rows[i].document_id == rows[j].document_id
                && rows[i].portal_key == rows[j].portal_key
            {
                continue;
            }
            if rows[i]
                .assertion
                .value
                .materially_differs(&rows[j].assertion.value)
            {
                return Some((i, j));
            }
        }
    }
    None
}

fn distinct_document_count(rows: &[SourcedAssertion]) -> u16 {
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        seen.insert((row.portal_key.clone(), row.document_id));
    }
    u16::try_from(seen.len()).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::assertion_index::AssertionValue;

    fn sourced(
        portal: &str,
        doc: i64,
        req: i64,
        name: &str,
        auth: bool,
        value: AssertionValue,
    ) -> SourcedAssertion {
        SourcedAssertion {
            portal_key: portal.to_string(),
            document_id: doc,
            request_id: req,
            file_name: Some(name.to_string()),
            likely_authoritative: auth,
            assertion: Assertion {
                subject: "records retention policy".to_string(),
                predicate: AssertionPredicate::RetentionPeriod,
                value,
                char_start: 0,
                char_end: 10,
            },
        }
    }

    #[test]
    fn build_finding_serious_when_both_exemplars_are_authoritative() {
        let rows = vec![
            sourced(
                "p",
                1,
                101,
                "personnel_policy.pdf",
                true,
                AssertionValue::Days(1095),
            ),
            sourced(
                "p",
                2,
                102,
                "records_retention_manual.pdf",
                true,
                AssertionValue::Days(2555),
            ),
        ];
        let finding = build_finding(
            "records retention policy",
            AssertionPredicate::RetentionPeriod,
            rows,
        )
        .expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Serious);
        assert_eq!(finding.kind, FindingKind::ContradictoryStatement);
        assert_eq!(finding.citations.len(), 2);
        assert!(finding.headline.contains("retention period"));
        assert!(finding.explanation.contains("personnel_policy.pdf"));
    }

    #[test]
    fn build_finding_notable_when_documents_are_correspondence() {
        let rows = vec![
            sourced(
                "p",
                3,
                201,
                "letter_march.pdf",
                false,
                AssertionValue::Days(30),
            ),
            sourced(
                "p",
                4,
                202,
                "letter_april.pdf",
                false,
                AssertionValue::Days(60),
            ),
        ];
        let finding = build_finding(
            "response_time_commitment",
            AssertionPredicate::ResponseTimeCommitment,
            rows,
        )
        .expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Notable);
    }

    #[test]
    fn build_finding_returns_none_when_no_pair_materially_differs() {
        // Two rows with values within noise: no conflict pair, no finding.
        let rows = vec![
            sourced("p", 5, 301, "a.pdf", true, AssertionValue::Days(30)),
            sourced("p", 6, 302, "b.pdf", true, AssertionValue::Days(31)),
        ];
        assert!(build_finding(
            "response_time_commitment",
            AssertionPredicate::ResponseTimeCommitment,
            rows
        )
        .is_none());
    }

    #[test]
    fn build_finding_reflects_distinct_document_count_in_confidence() {
        let rows = vec![
            sourced("p", 10, 401, "a.pdf", true, AssertionValue::Days(30)),
            sourced("p", 11, 402, "b.pdf", true, AssertionValue::Days(90)),
        ];
        let two = build_finding(
            "records retention policy",
            AssertionPredicate::RetentionPeriod,
            rows,
        )
        .unwrap();
        let rows_five = vec![
            sourced("p", 20, 501, "a.pdf", true, AssertionValue::Days(30)),
            sourced("p", 21, 502, "b.pdf", true, AssertionValue::Days(90)),
            sourced("p", 22, 503, "c.pdf", true, AssertionValue::Days(30)),
            sourced("p", 23, 504, "d.pdf", true, AssertionValue::Days(90)),
            sourced("p", 24, 505, "e.pdf", true, AssertionValue::Days(30)),
        ];
        let five = build_finding(
            "records retention policy",
            AssertionPredicate::RetentionPeriod,
            rows_five,
        )
        .unwrap();
        assert!(
            five.confidence.rank_weight() > two.confidence.rank_weight(),
            "more sources should raise confidence"
        );
    }
}
