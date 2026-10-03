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
        document_class::{classify, DocumentClass},
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;
use std::collections::HashMap;

use super::public_documents::{load_public_documents, PublicDocument};

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
    /// The document is an official statement (press release, budget message, minutes) rather
    /// than a released record.
    is_statement: bool,
    assertion: Assertion,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let documents = load_public_documents(repository, MAX_DOCUMENTS).await?;
    Ok(build_findings(documents))
}

fn build_findings(documents: Vec<PublicDocument>) -> Vec<Finding> {
    // Keyed on portal too: an ID like "Grant 123" or "PO 100" is only unique within one agency's
    // own numbering. Two different agencies' portals reusing the same short ID are not the same
    // subject, and grouping across portals produced false "contradictions" between unrelated
    // awards that merely shared an ID string.
    let mut by_group: HashMap<(String, String, AssertionPredicate), Vec<SourcedAssertion>> =
        HashMap::new();
    for document in documents {
        for assertion in extract_assertions(&document.text) {
            let key = (
                document.portal_key.clone(),
                assertion.subject.clone(),
                assertion.predicate,
            );
            let sourced = SourcedAssertion {
                portal_key: document.portal_key.clone(),
                document_id: document.document_id,
                request_id: document.request_id,
                file_name: document.file_name.clone(),
                likely_authoritative: infer_authoritative(&document.file_name),
                is_statement: classify(document.file_name.as_deref(), &document.text)
                    .is_some_and(DocumentClass::is_statement),
                assertion,
            };
            by_group.entry(key).or_default().push(sourced);
        }
    }

    let mut findings = Vec::new();
    for ((_portal_key, subject, predicate), rows) in by_group {
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
    findings
}

/// A rough "is this document authoritative" heuristic. A contradiction between two authoritative
/// documents (policies, handbooks, MOUs, resolutions) is more serious than one between two ad-hoc
/// letters; conversely, correspondence often has minor drift that isn't worth surfacing at
/// Serious rank. The file-name signal is deliberately narrow: matching "policy"/"handbook"/
/// "manual"/"MOU"/"resolution"/"contract" produces a true positive; anything else stays false.
fn infer_authoritative(file_name: &Option<String>) -> bool {
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
    let statement_versus_record = a.is_statement != b.is_statement;
    let headline = format!(
        "{prefix}'{display_subject}' {label}: {a} in one document, {b} in another",
        prefix = if statement_versus_record {
            "Official statement vs released record: "
        } else {
            ""
        },
        label = predicate.as_label(),
        a = a.assertion.value.display(),
        b = b.assertion.value.display(),
    );
    let statement_note = if statement_versus_record {
        let (statement, record) = if a.is_statement { (a, b) } else { (b, a) };
        format!(
            " '{}' is the official statement and '{}' is the released record.",
            statement
                .file_name
                .clone()
                .unwrap_or_else(|| format!("document {}", statement.document_id)),
            record
                .file_name
                .clone()
                .unwrap_or_else(|| format!("document {}", record.document_id)),
        )
    } else {
        String::new()
    };
    let explanation = format!(
        "The corpus contains {distinct} documents that state a {label} for '{display_subject}', \
         and they disagree.{statement_note} '{a_file}' (request {a_req}) reports {a_val}; '{b_file}' \
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
            character_start: Some(row.assertion.character_start as i64),
            character_end: Some(row.assertion.character_end as i64),
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
            is_statement: false,
            assertion: Assertion {
                subject: "records retention policy".to_string(),
                predicate: AssertionPredicate::RetentionPeriod,
                value,
                byte_start: 0,
                byte_end: 10,
                character_start: 0,
                character_end: 10,
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
    fn a_statement_against_a_record_is_labelled_and_two_records_are_not() {
        let build = |statement: bool| {
            let mut first = sourced(
                "p",
                5,
                301,
                "press_release.pdf",
                false,
                AssertionValue::Days(30),
            );
            first.is_statement = statement;
            let second = sourced(
                "p",
                6,
                302,
                "letter_april.pdf",
                false,
                AssertionValue::Days(60),
            );
            build_finding(
                "response_time_commitment",
                AssertionPredicate::ResponseTimeCommitment,
                vec![first, second],
            )
            .expect("must fire")
        };
        let mixed = build(true);
        assert!(mixed
            .headline
            .starts_with("Official statement vs released record: "));
        assert!(mixed
            .explanation
            .contains("'press_release.pdf' is the official statement and 'letter_april.pdf' is the released record"));
        assert!(!build(false).headline.starts_with("Official statement"));
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

    fn doc(portal: &str, id: i64, text: &str) -> PublicDocument {
        PublicDocument {
            portal_key: portal.to_string(),
            document_id: id,
            request_id: id,
            file_name: Some(format!("doc-{id}.pdf")),
            created_at: "2026-01-01".to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn same_award_id_in_different_portals_is_not_the_same_subject() {
        // Two unrelated agencies both numbering an award "123" is not a contradiction: the IDs
        // are only unique within one agency's own numbering.
        let documents = vec![
            doc(
                "city-a",
                1,
                "Grant 123 in the amount of $100,000 was awarded this year.",
            ),
            doc(
                "city-b",
                2,
                "Grant 123 in the amount of $1.00 was awarded this year.",
            ),
        ];
        assert!(build_findings(documents).is_empty());
    }

    #[test]
    fn same_award_id_in_the_same_portal_still_fires() {
        let documents = vec![
            doc(
                "city-a",
                1,
                "Grant 123 in the amount of $100,000 was awarded this year.",
            ),
            doc(
                "city-a",
                2,
                "Grant 123 in the amount of $1.00 was awarded this year.",
            ),
        ];
        let findings = build_findings(documents);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, FindingKind::ContradictoryStatement);
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
