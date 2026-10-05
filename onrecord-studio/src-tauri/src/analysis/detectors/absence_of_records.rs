//! Absence-of-records detector.
//!
//! Scans each request's released document text for tokens that follow an agency-authored
//! sequential numbering scheme (invoices, memos, resolutions, agenda items, contract amendments,
//! personnel actions). Within a request, when the released set includes members `1..N` of a
//! sequence but is missing one or more numbers inside that tight span, the gap is flagged.
//!
//! **What this is not**: a legal-claim finding. A gap in released numbering is evidence records
//! were withheld or omitted from the release; it does not itself say the omission was unlawful,
//! and reasonable exclusions exist (superseded drafts, executive session materials, records
//! already released under another request). The detector deliberately requires a dense sequence
//! before flagging (≥ 3 present + gap fraction < span for `Notable`, ≥ 5 present + gap < 20% of
//! span for `Serious`) to keep the signal-to-noise ratio high.
//!
//! **Bounds**: at most `MAX_DOCUMENTS_SCANNED` document texts per invocation, at most
//! `MAX_FINDINGS_PER_RUN` findings surfaced overall.

use crate::{
    analysis::finding::{
        finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
        FindingSubject,
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::Utc;
use duckdb::params;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

pub const SLUG: &str = "absence_of_records";

/// Bounded scan. Sequences are almost always found inside a single request's release set, so
/// scanning per-request keeps the cost predictable.
const MAX_DOCUMENTS_SCANNED: i64 = 4_000;

/// Cap the surfaced findings per invocation. A single very productive request could otherwise
/// generate hundreds of one-integer gaps, drowning the inbox.
const MAX_FINDINGS_PER_RUN: usize = 100;

/// Minimum members of a sequence present before we consider it a sequence at all.
const MIN_SEQUENCE_MEMBERS: usize = 3;

/// Threshold above which the finding is `Serious`.
const SERIOUS_MEMBER_COUNT: usize = 5;

/// Gap fraction ceiling for `Serious`: for a tight sequence, missing pieces should not be more
/// than 20% of the total span. A very sparse sequence (say 3 numbers in a span of 1000) is not
/// meaningful evidence of omission.
const SERIOUS_MAX_GAP_FRACTION: f64 = 0.20;

/// Missing fraction above which a span is not a "tight" sequence and nothing is emitted. Agendas
/// number items in category blocks (30s proclamations, 50s ordinances, 100s resolutions), so a
/// span of 10..331 with 25 members is a numbering convention, not a release with holes.
const MAX_GAP_FRACTION: f64 = 0.5;

/// Structured-data exports. Their text is row content (free-text complaints, addresses, IDs), so
/// "item 2" or "invoice 7" in a cell says nothing about an agency's numbering scheme.
const TABULAR_EXTENSIONS: [&str; 6] = [".csv", ".tsv", ".json", ".ndjson", ".xls", ".xlsx"];

/// Labels of numbering schemes agencies routinely use. The regex only fires when the token starts
/// with one of these words so ordinary numbers, dates, and phone numbers don't produce noise.
static SEQUENCE_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?P<label>invoice|purchase\s+order|po|memo|memorandum|resolution|ordinance|amendment|agenda\s+item|item|personnel\s+action)\s*[#\-:.]?\s*(?P<num>\d{1,6})\b"
    )
    .unwrap()
});

/// Row fetched per document to be scanned. `extracted_text` is the FTS-normalized projection —
/// the same text the app searches against and the same text quoted in legal briefs.
struct ScannedDocument {
    portal_key: String,
    request_id: i64,
    document_id: i64,
    file_name: Option<String>,
    extracted_text: String,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let scanned = load_scanned_documents(repository).await?;
    let mut findings = collect_findings(&scanned);
    findings.truncate(MAX_FINDINGS_PER_RUN);
    Ok(findings)
}

async fn load_scanned_documents(
    repository: &DuckDbRepository,
) -> Result<Vec<ScannedDocument>, AppError> {
    repository
        .query_connection("Absence-of-records scan", move |conn| {
            let mut statement = conn
                .prepare(
                    // Only a records release is a set whose gaps mean anything. A crawled site's
                    // files are fetched by link and, under a collection subject, deliberately in
                    // part, so a missing number there is a file not collected, not one withheld.
                    // Files the user added by hand are whatever the user chose to add.
                    "SELECT d.portal_subdomain, d.request_id, d.id, d.file_name, fts.extracted_text
                     FROM documents_fts fts
                     JOIN documents d
                        ON d.id = fts.document_id
                       AND d.portal_subdomain = fts.portal_subdomain
                     JOIN portals p ON p.subdomain = d.portal_subdomain
                     WHERE LOWER(TRIM(d.visibility)) = 'public'
                       AND COALESCE(p.source_kind, 'NEXT_REQUEST') = 'NEXT_REQUEST'
                     ORDER BY d.request_id ASC, d.portal_subdomain ASC, d.id ASC
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare absence-of-records scan: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![MAX_DOCUMENTS_SCANNED], |row| {
                    Ok(ScannedDocument {
                        portal_key: row.get(0)?,
                        request_id: row.get(1)?,
                        document_id: row.get(2)?,
                        file_name: row.get(3)?,
                        extracted_text: row.get(4)?,
                    })
                })
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to execute absence-of-records scan: {error}"
                    ))
                })?;
            let mut collected = Vec::new();
            for row in rows {
                collected.push(row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode absence-of-records row: {error}"))
                })?);
            }
            Ok(collected)
        })
        .await
}

/// Group observed numbers by `(portal_key, request_id, normalized label)` and detect gaps.
///
/// The label normalization folds "PO" and "purchase order" together so both spellings feed the
/// same sequence; "agenda item" and "item" stay separate because "item" alone is much noisier and
/// carries lower confidence.
fn collect_findings(scanned: &[ScannedDocument]) -> Vec<Finding> {
    #[derive(Default)]
    struct SequenceEvidence {
        // Number -> earliest (portal_key, document_id, file_name) mentioning it.
        observations: BTreeMap<u64, (String, i64, Option<String>)>,
        // All documents that touched this sequence, in insertion order (used for citations).
        documents: BTreeSet<i64>,
    }

    let mut buckets: BTreeMap<(String, i64, String), SequenceEvidence> = BTreeMap::new();

    for document in scanned {
        if is_tabular_export(document.file_name.as_deref()) {
            continue;
        }
        for capture in SEQUENCE_TOKEN.captures_iter(&document.extracted_text) {
            let label_raw = capture.name("label").map(|m| m.as_str()).unwrap_or("");
            let Some(normalized_label) = normalize_label(label_raw) else {
                continue;
            };
            let Some(number_str) = capture.name("num").map(|m| m.as_str()) else {
                continue;
            };
            let Ok(number) = number_str.parse::<u64>() else {
                continue;
            };
            let entry = buckets
                .entry((
                    document.portal_key.clone(),
                    document.request_id,
                    normalized_label,
                ))
                .or_default();
            entry.observations.entry(number).or_insert_with(|| {
                (
                    document.portal_key.clone(),
                    document.document_id,
                    document.file_name.clone(),
                )
            });
            entry.documents.insert(document.document_id);
        }
    }

    let mut findings = Vec::new();
    let now = Utc::now();
    for ((portal_key, request_id, label), evidence) in buckets {
        let numbers: Vec<u64> = evidence.observations.keys().copied().collect();
        let present_count = numbers.len();
        if present_count < MIN_SEQUENCE_MEMBERS {
            continue;
        }
        let min = *numbers.first().unwrap();
        let max = *numbers.last().unwrap();
        let span = max.saturating_sub(min).saturating_add(1) as usize;
        if span <= present_count {
            // No gaps: every integer in `min..=max` is present.
            continue;
        }
        let missing_count = span - present_count;
        let missing: Vec<u64> = (min..=max)
            .filter(|value| !evidence.observations.contains_key(value))
            .collect();
        if missing.is_empty() {
            continue;
        }
        let gap_fraction = missing_count as f64 / span as f64;
        if gap_fraction > MAX_GAP_FRACTION {
            continue;
        }
        let severity =
            if present_count >= SERIOUS_MEMBER_COUNT && gap_fraction <= SERIOUS_MAX_GAP_FRACTION {
                FindingSeverity::Serious
            } else {
                FindingSeverity::Notable
            };
        // Confidence: derived from label reliability and evidence density. A generic "item"
        // label carries less confidence than a specific "invoice" or "resolution" — the label
        // regex allows both, but the finding hedges accordingly.
        let label_bp: u16 = if label == "item" { 3_000 } else { 7_000 };
        // Density factor: more members present raises confidence, capped.
        let density_bp: u16 = ((present_count as u32).saturating_mul(400).min(3_000)) as u16;
        let confidence =
            FindingConfidence::measured(label_bp.saturating_add(density_bp).min(9_500));
        let subject = FindingSubject {
            portal_key: Some(portal_key.clone()),
            request_id: Some(request_id),
            ..Default::default()
        };
        // Citations: two documents flanking a real gap for readability. Pick the neighbors of the
        // smallest missing value where possible.
        let citations = build_citations(&portal_key, &missing, &evidence.observations);
        let missing_preview = summarize_missing(&missing);
        let salient = format!(
            "label={label};present={present_count};missing={missing_count};range={min}-{max}"
        );
        let id = finding_id(FindingKind::AbsenceOfRecords, SLUG, &subject, &[&salient]);
        let headline = format!(
            "{missing_count} {label}{plural} missing between #{min} and #{max}",
            plural = if missing_count == 1 { "" } else { "s" }
        );
        let explanation = format!(
            "Released documents for this request reference {label} #{min} through #{max}, but \
             the following numbers in that range are not among the released documents: \
             {missing_preview}. A gap inside an otherwise-dense sequence is evidence records \
             were withheld or omitted; the pattern does not by itself say the omission was \
             unlawful."
        );
        findings.push(Finding {
            id,
            kind: FindingKind::AbsenceOfRecords,
            severity,
            confidence,
            subject,
            headline,
            explanation,
            citations,
            discovered_at: now,
            last_seen_at: now,
            dismissed_at: None,
            source_detector: SLUG.into(),
        });
    }

    // Rank the highest-value gaps first so the truncation at MAX_FINDINGS_PER_RUN keeps them.
    findings.sort_by_key(|f| std::cmp::Reverse(f.rank_key()));
    findings
}

fn is_tabular_export(file_name: Option<&str>) -> bool {
    let Some(name) = file_name else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    TABULAR_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

fn normalize_label(raw: &str) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    // Collapse internal whitespace.
    let collapsed: String = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    let folded = match collapsed.as_str() {
        "po" | "purchase order" => "purchase order".to_string(),
        "memorandum" => "memo".to_string(),
        // Everything else keeps its lowercased form.
        other => other.to_string(),
    };
    if folded.is_empty() {
        None
    } else {
        Some(folded)
    }
}

fn summarize_missing(missing: &[u64]) -> String {
    // Render up to the first 12 missing numbers; abbreviate the tail if more.
    const MAX_LISTED: usize = 12;
    if missing.len() <= MAX_LISTED {
        return missing
            .iter()
            .map(|n| format!("#{n}"))
            .collect::<Vec<_>>()
            .join(", ");
    }
    let head = missing
        .iter()
        .take(MAX_LISTED)
        .map(|n| format!("#{n}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{head}, and {} more", missing.len() - MAX_LISTED)
}

fn build_citations(
    portal_key: &str,
    missing: &[u64],
    observations: &BTreeMap<u64, (String, i64, Option<String>)>,
) -> Vec<FindingCitation> {
    let mut citations = Vec::new();
    let Some(&first_missing) = missing.first() else {
        return citations;
    };
    // Nearest present neighbor below and above the smallest missing number.
    let below = observations
        .range(..first_missing)
        .next_back()
        .map(|(number, entry)| (*number, entry));
    let above = observations
        .range(first_missing..)
        .next()
        .map(|(number, entry)| (*number, entry));
    if let Some((number, entry)) = below {
        citations.push(FindingCitation {
            portal_key: entry.0.clone(),
            document_id: entry.1,
            character_start: None,
            character_end: None,
            quote: None,
            label: Some(format!(
                "Nearest released neighbor below the gap: #{number} in {}",
                entry
                    .2
                    .clone()
                    .unwrap_or_else(|| format!("document {}", entry.1))
            )),
        });
    }
    if let Some((number, entry)) = above {
        citations.push(FindingCitation {
            portal_key: entry.0.clone(),
            document_id: entry.1,
            character_start: None,
            character_end: None,
            quote: None,
            label: Some(format!(
                "Nearest released neighbor above the gap: #{number} in {}",
                entry
                    .2
                    .clone()
                    .unwrap_or_else(|| format!("document {}", entry.1))
            )),
        });
    }
    // Safety net: if only one neighbor found, cite the anchor document at least.
    if citations.is_empty() {
        citations.push(FindingCitation {
            portal_key: portal_key.to_string(),
            document_id: observations.values().next().map(|e| e.1).unwrap_or(0),
            character_start: None,
            character_end: None,
            quote: None,
            label: Some("Sequence anchor".into()),
        });
    }
    citations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(portal: &str, request_id: i64, id: i64, text: &str) -> ScannedDocument {
        ScannedDocument {
            portal_key: portal.into(),
            request_id,
            document_id: id,
            file_name: Some(format!("doc-{id}.pdf")),
            extracted_text: text.into(),
        }
    }

    #[test]
    fn no_finding_when_sequence_is_dense_and_complete() {
        // Three consecutive invoices, no gap.
        let scanned = vec![
            doc("p", 1, 10, "Invoice #101 paid on time."),
            doc("p", 1, 11, "Invoice #102 recorded."),
            doc("p", 1, 12, "Invoice #103 approved."),
        ];
        let findings = collect_findings(&scanned);
        assert!(findings.is_empty(), "dense complete sequence must not fire");
    }

    #[test]
    fn no_finding_when_sequence_too_short_to_be_meaningful() {
        // Two members isn't enough to call it a sequence.
        let scanned = vec![
            doc("p", 1, 10, "Invoice #101"),
            doc("p", 1, 12, "Invoice #103"),
        ];
        let findings = collect_findings(&scanned);
        assert!(
            findings.is_empty(),
            "two members below MIN_SEQUENCE_MEMBERS"
        );
    }

    #[test]
    fn notable_finding_for_single_gap_in_short_dense_sequence() {
        let scanned = vec![
            doc("p", 1, 10, "Invoice #101 approved."),
            doc("p", 1, 11, "Invoice #102 approved."),
            doc("p", 1, 12, "Invoice #104 approved."),
        ];
        let findings = collect_findings(&scanned);
        assert_eq!(findings.len(), 1, "single-gap sequence must fire once");
        let finding = &findings[0];
        assert_eq!(finding.severity, FindingSeverity::Notable);
        assert_eq!(finding.kind, FindingKind::AbsenceOfRecords);
        assert!(finding.headline.contains("invoice"));
        assert!(finding.explanation.contains("#103"));
        assert!(!finding.citations.is_empty());
    }

    #[test]
    fn serious_finding_when_dense_sequence_has_a_tight_gap() {
        // 10 present of 12 span => gap fraction 2/12 = 16.7%, below the 20% ceiling.
        let mut scanned = Vec::new();
        for (i, n) in [1_u64, 2, 3, 4, 5, 6, 7, 8, 11, 12].iter().enumerate() {
            scanned.push(doc("p", 1, 10 + i as i64, &format!("Resolution #{n}")));
        }
        let findings = collect_findings(&scanned);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, FindingSeverity::Serious);
        assert!(findings[0].explanation.contains("#9"));
        assert!(findings[0].explanation.contains("#10"));
    }

    #[test]
    fn confidence_is_lower_for_generic_item_label() {
        let scanned = vec![
            doc("p", 1, 10, "Item 1 discussed."),
            doc("p", 1, 11, "Item 2 discussed."),
            doc("p", 1, 12, "Item 4 discussed."),
        ];
        let findings = collect_findings(&scanned);
        assert_eq!(findings.len(), 1);
        match findings[0].confidence {
            FindingConfidence::Measured { basis_points } => {
                assert!(
                    basis_points < 6_000,
                    "generic 'item' label must derate confidence, got {basis_points}"
                );
            }
            other => panic!("expected measured confidence, got {other:?}"),
        }
    }

    #[test]
    fn purchase_order_and_po_are_folded_into_one_sequence() {
        let scanned = vec![
            doc("p", 1, 10, "PO-1001 authorized."),
            doc("p", 1, 11, "Purchase order 1002 authorized."),
            doc("p", 1, 12, "PO 1004 authorized."),
        ];
        let findings = collect_findings(&scanned);
        assert_eq!(
            findings.len(),
            1,
            "PO and 'purchase order' must fold to one sequence"
        );
        assert!(findings[0].explanation.contains("#1003"));
    }

    #[test]
    fn open_data_rows_and_block_numbered_agendas_are_not_sequences() {
        // Measured on the San Diego corpus: 4 of 5 findings came from free-text cells of Get It
        // Done CSV exports ("Invoice# 403238", "no resolution.1."), the fifth from a Council
        // agenda whose items are numbered in category blocks (10, 30-31, 50-56, 100-106, 330-331).
        let mut csv_rows = doc(
            "p",
            1,
            10,
            "\"a\",\"towed, Invoice# 1\" \"b\",\"invoice 2 unpaid\" \"c\",\"invoice 4 and item 2\"",
        );
        csv_rows.file_name = Some("get_it_done_requests.CSV".into());
        let agenda = doc(
            "p",
            1,
            11,
            "Item 10: Minutes. Item 30: Proclamation. Item 31: Day. Item 50: Ordinance. Item 51: Repeal. \
             Item 100: Grant. Item 101: Sole source. Item 330: Definition. Item 331: Lease.",
        );
        let findings = collect_findings(&[csv_rows, agenda]);
        assert!(
            findings.is_empty(),
            "got {:?}",
            findings.iter().map(|f| &f.headline).collect::<Vec<_>>()
        );
    }

    #[test]
    fn missing_summary_abbreviates_after_twelve_entries() {
        let missing: Vec<u64> = (1..=20).collect();
        let summary = summarize_missing(&missing);
        assert!(summary.contains("and 8 more"));
    }

    #[test]
    fn finding_id_is_stable_across_re_runs_for_same_evidence() {
        let scanned = vec![
            doc("p", 1, 10, "Invoice #101"),
            doc("p", 1, 11, "Invoice #102"),
            doc("p", 1, 12, "Invoice #104"),
        ];
        let a = collect_findings(&scanned);
        let b = collect_findings(&scanned);
        assert_eq!(a[0].id, b[0].id);
    }

    #[tokio::test]
    async fn a_crawled_sites_numbering_gap_is_not_read_as_withheld_records() {
        let temp = tempfile::tempdir().unwrap();
        let repository = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repository
            .run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        {
            let conn = repository.analytics_connection().await.unwrap();
            for (portal, kind) in [("release", "NEXT_REQUEST"), ("site", "WEB_CRAWL")] {
                conn.execute(
                    "INSERT INTO portals (subdomain, display_name, base_url, source_kind)
                     VALUES (?1, ?1, 'https://example.test', ?2)",
                    params![portal, kind],
                )
                .unwrap();
                for (id, number) in [(1_i64, 101), (2, 102), (3, 104), (4, 105), (5, 106)] {
                    conn.execute(
                        "INSERT INTO documents (id, request_id, portal_subdomain, visibility, created_at, updated_at)
                         VALUES (?1, 7, ?2, 'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                        params![id, portal],
                    )
                    .unwrap();
                    conn.execute(
                        "INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
                         VALUES (?1, ?2, ?3, ?4)",
                        params![id, portal, format!("doc-{id}.pdf"), format!("Invoice #{number} approved.")],
                    )
                    .unwrap();
                }
            }
        }
        let findings = run(&repository).await.unwrap();
        assert!(
            !findings.is_empty(),
            "the release's gap must still be found"
        );
        assert!(
            findings
                .iter()
                .all(|finding| finding.subject.portal_key.as_deref() == Some("release")),
            "a crawled site's gap must not be reported: {findings:#?}"
        );
    }
}
