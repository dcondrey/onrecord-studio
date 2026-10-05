//! Redaction-context detector.
//!
//! Finds an explicit redaction mark (`[REDACTED]`, an exemption stamp, a run of black boxes) whose
//! surrounding words appear in another collected document, or in a row of an imported table, with
//! text standing where the mark is. The comparison is the anchor-bounded one the on-demand
//! reconstruction uses (`reconstruction::AnchoredSpan`): up to five words each side, at least
//! three, compared across case and whitespace, and the other text must lie between the same two
//! anchors. Words that merely occur nearby are not a match.
//!
//! **What a finding states.** That the same surrounding words appear elsewhere with text between
//! them, and where: both passages are cited by character range, a table cell by snapshot, row and
//! column. It never prints that text and never says what the redaction covers.
//!
//! **Rules that withhold a finding.**
//! - A value `mask_pii` recognises (a phone number, an address, an identifier) is never matched.
//! - Two different values between the same anchors mean the wording is reused prose; the span is
//!   marked and not reported.
//! - A span whose context reads as naming a person (a "signed by" style lead-in, a personal
//!   privacy stamp such as `(b)(6)`), or whose matching text is shaped like a name, needs the same
//!   text in two independent sources. Two copies of one file are one source.
//! - A file the user added by hand is never read, on either side.
//!
//! **Coverage.** Every eligible document is taken in once (`storage::redaction_sweep`), so an old
//! redacted document is compared with a source that arrives later and a new redacted document with
//! every source already held. Work per run is bounded by [`SweepLimits`]; what a bound leaves is
//! done by later runs, and stored matches are reported on every run, so a finding stands until
//! its documents change.
//!
//! **Not a legal claim.** `FindingKind::RedactionCandidateInferred::is_legal_claim() → false`,
//! and confidence is `Unassessed`: matching words are not a measured probability.

use std::collections::{BTreeSet, HashMap};

use crate::{
    analysis::{
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
        reconstruction::{is_name_shaped, is_personal_identifier, normalized, AnchoredSpan},
        redaction_inference::{classify_entity_hint, scan_redaction_spans, CandidateKind},
    },
    error::AppError,
    integrity::sha256_hex,
    storage::{
        mask_pii,
        redaction_sweep::{
            DocumentKey, MatchSource, SweepAnchor, SweepCandidate, SweepDocument, SweepMatch,
            SweepSpan,
        },
        DuckDbRepository,
    },
};
use aho_corasick::AhoCorasick;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::public_documents::MAX_DOCUMENT_TEXT_BYTES;

pub const SLUG: &str = "inferred_redaction_identity";

/// Work one run may do. Whatever a limit leaves is picked up by the next run.
#[derive(Debug, Clone)]
pub(crate) struct SweepLimits {
    /// Documents taken in for the first time, and the stored text read for them.
    pub new_documents: usize,
    pub new_text_bytes: u64,
    /// Documents read again because a span was added after they were last read.
    pub rescan_documents: usize,
    pub rescan_text_bytes: u64,
    /// Imported tables read, and rows read across them.
    pub tables: usize,
    pub table_rows: i64,
}

impl Default for SweepLimits {
    fn default() -> Self {
        Self {
            new_documents: 500,
            new_text_bytes: 64 * 1024 * 1024,
            rescan_documents: 500,
            rescan_text_bytes: 64 * 1024 * 1024,
            tables: 8,
            table_rows: 400_000,
        }
    }
}

/// Marks compared per document: the first this many with usable anchors.
const MAX_SPANS_PER_DOCUMENT: usize = 24;
/// Findings per document, so one heavily redacted release does not fill the inbox.
const MAX_FINDINGS_PER_DOCUMENT: usize = 3;
/// Sources listed in one finding; the rest are counted.
const MAX_CITED_SOURCES: usize = 5;
/// Different values kept per span and source. Two is enough to mark the wording as reused.
const MAX_VALUES_PER_SOURCE: usize = 2;
const TABLE_ROW_CHUNK: i64 = 50_000;

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    run_bounded(repository, &SweepLimits::default()).await
}

pub(crate) async fn run_bounded(
    repository: &DuckDbRepository,
    limits: &SweepLimits,
) -> Result<Vec<Finding>, AppError> {
    let run = repository
        .begin_redaction_sweep(MAX_DOCUMENT_TEXT_BYTES)
        .await?;
    let new_documents = repository
        .load_unswept_documents(
            MAX_DOCUMENT_TEXT_BYTES,
            limits.new_documents,
            limits.new_text_bytes,
        )
        .await?;
    let (mut anchors, stored_newest) = repository.load_sweep_anchors().await?;
    let (new_documents, new_spans) = crate::blocking::spawn("Redaction span scan", move || {
        let spans: Vec<SweepSpan> = new_documents.iter().flat_map(spans_of).collect();
        Ok((new_documents, spans))
    })
    .await??;
    let newest_span_run = if new_spans.is_empty() {
        stored_newest
    } else {
        run
    };
    anchors.extend(new_spans.iter().map(|span| SweepAnchor {
        document: span.document.clone(),
        passage_start: span.passage_start,
        left_anchor: span.left_anchor.clone(),
        right_anchor: span.right_anchor.clone(),
        row_anchors: span.row_anchors.clone(),
    }));
    let rescans = repository
        .load_documents_to_rescan(
            newest_span_run,
            limits.rescan_documents,
            limits.rescan_text_bytes,
        )
        .await?;
    let admitted = new_documents
        .iter()
        .map(|document| (document.key.clone(), document.text_version.clone()))
        .collect();
    let rescanned = rescans
        .iter()
        .map(|document| document.key.clone())
        .collect();
    let (anchors, matches) = crate::blocking::spawn("Redaction context scan", move || {
        let mut matches = Vec::new();
        if let Some(mut matcher) = Matcher::for_documents(&anchors)? {
            for document in new_documents.iter().chain(&rescans) {
                scan_document(&mut matcher, document, &mut matches);
            }
        }
        Ok((anchors, matches))
    })
    .await??;
    repository
        .record_document_sweep(run, admitted, new_spans, rescanned, matches)
        .await?;

    let tables = if anchors.is_empty() {
        Vec::new()
    } else {
        repository
            .load_tables_to_sweep(newest_span_run, limits.tables)
            .await?
    };
    if !tables.is_empty() {
        let mut matcher = crate::blocking::spawn("Redaction table index", move || {
            Matcher::for_tables(&anchors)
        })
        .await??;
        let mut rows_left = limits.table_rows;
        'tables: for table in tables {
            let mut after = table.resume_after_row;
            loop {
                if rows_left <= 0 {
                    break 'tables;
                }
                let through = (after + TABLE_ROW_CHUNK.min(rows_left)).min(table.last_row);
                let rows = if through > after {
                    repository
                        .load_table_rows(table.snapshot_id.clone(), after, through)
                        .await?
                } else {
                    Vec::new()
                };
                let snapshot_id = table.snapshot_id.clone();
                let (returned, matches) =
                    crate::blocking::spawn("Redaction table scan", move || {
                        let mut matches = Vec::new();
                        if let Some(matcher) = matcher.as_mut() {
                            for (row_number, payload) in &rows {
                                scan_row(matcher, &snapshot_id, *row_number, payload, &mut matches);
                            }
                        }
                        Ok((matcher, matches))
                    })
                    .await??;
                matcher = returned;
                rows_left -= through - after;
                repository
                    .record_table_sweep(run, table.clone(), through, matches)
                    .await?;
                after = through;
                if through >= table.last_row {
                    break;
                }
            }
        }
    }

    let candidates = repository.load_sweep_candidates().await?;
    Ok(build_findings(candidates, Utc::now()))
}

/// Only an explicit mark (a [REDACTED] label, an exemption stamp, black boxes) is compared. In
/// released contracts and forms, underscore and X runs are blank fields: on a live San Diego
/// collection every such "redaction" was a signature or address line.
fn is_explicit_mark(marked: &str) -> bool {
    !marked.chars().all(|c| matches!(c, '_' | 'X' | 'x'))
}

/// `(b)(6)` and `(b)(7)(C)` are the personal-privacy exemptions.
fn is_personal_privacy_stamp(marked: &str) -> bool {
    let marked = marked.to_ascii_lowercase();
    marked.starts_with("(b)(6)") || marked.starts_with("(b)(7)(c)")
}

fn character_offset(text: &str, byte: usize) -> i64 {
    text[..byte].chars().count() as i64
}

fn spans_of(document: &SweepDocument) -> Vec<SweepSpan> {
    let text = &document.text;
    let mut spans: Vec<SweepSpan> = Vec::new();
    for mark in scan_redaction_spans(text) {
        if spans.len() == MAX_SPANS_PER_DOCUMENT {
            break;
        }
        let marked = &text[mark.start..mark.end];
        if !is_explicit_mark(marked) {
            continue;
        }
        let Some((anchored, passage)) = AnchoredSpan::around(text, mark.start, mark.end) else {
            continue;
        };
        if [anchored.left(), anchored.right()]
            .iter()
            .any(|anchor| !scan_redaction_spans(anchor).is_empty())
        {
            continue;
        }
        let passage_start = character_offset(text, passage.start);
        if spans
            .last()
            .is_some_and(|previous| previous.passage_start == passage_start)
        {
            continue;
        }
        let context = &text[passage.clone()];
        let line_start = text[..mark.start]
            .rfind(['\n', '\r'])
            .map_or(0, |at| at + 1);
        let line_end = text[mark.end..]
            .find(['\n', '\r'])
            .map_or(text.len(), |at| mark.end + at);
        let row_anchors = AnchoredSpan::around(
            &text[line_start..line_end],
            mark.start - line_start,
            mark.end - line_start,
        )
        .map(|(line, _)| (line.left().to_string(), line.right().to_string()))
        .filter(|(left, right)| {
            scan_redaction_spans(left).is_empty() && scan_redaction_spans(right).is_empty()
        });
        spans.push(SweepSpan {
            document: document.key.clone(),
            passage_start,
            passage_end: character_offset(text, passage.end),
            left_anchor: anchored.left().to_string(),
            right_anchor: anchored.right().to_string(),
            row_anchors,
            quote: mask_pii(context)
                .map(|masked| masked.masked_text)
                .unwrap_or_default(),
            person_like: is_personal_privacy_stamp(marked)
                || classify_entity_hint(context) == CandidateKind::Person,
        });
    }
    spans
}

/// One span as a matcher compares it: by its own anchors against a document, by its line-bounded
/// anchors against a table row.
struct MatcherAnchor {
    document: DocumentKey,
    passage_start: i64,
    left: String,
    right: String,
}

/// Finds which spans' anchors occur in a text before any pattern is run against it: one pass of
/// the text for every left anchor at once.
struct Matcher {
    automaton: AhoCorasick,
    /// Anchors sharing each distinct left anchor, in the automaton's pattern order.
    by_pattern: Vec<Vec<usize>>,
    anchors: Vec<MatcherAnchor>,
    right_anchors: Vec<String>,
    compiled: HashMap<usize, Option<AnchoredSpan>>,
}

impl Matcher {
    fn for_documents(anchors: &[SweepAnchor]) -> Result<Option<Self>, AppError> {
        Self::new(
            anchors
                .iter()
                .map(|anchor| MatcherAnchor {
                    document: anchor.document.clone(),
                    passage_start: anchor.passage_start,
                    left: anchor.left_anchor.clone(),
                    right: anchor.right_anchor.clone(),
                })
                .collect(),
        )
    }

    /// A table row holds one line, so a span is compared by the anchors on its mark's own line.
    /// A span whose line is too short keeps its full anchors, which only a cell holding the whole
    /// passage can match.
    fn for_tables(anchors: &[SweepAnchor]) -> Result<Option<Self>, AppError> {
        Self::new(
            anchors
                .iter()
                .map(|anchor| {
                    let (left, right) = anchor.row_anchors.clone().unwrap_or_else(|| {
                        (anchor.left_anchor.clone(), anchor.right_anchor.clone())
                    });
                    MatcherAnchor {
                        document: anchor.document.clone(),
                        passage_start: anchor.passage_start,
                        left,
                        right,
                    }
                })
                .collect(),
        )
    }

    fn new(anchors: Vec<MatcherAnchor>) -> Result<Option<Self>, AppError> {
        if anchors.is_empty() {
            return Ok(None);
        }
        let mut patterns: Vec<String> = Vec::new();
        let mut by_pattern: Vec<Vec<usize>> = Vec::new();
        let mut pattern_of: HashMap<String, usize> = HashMap::new();
        for (index, anchor) in anchors.iter().enumerate() {
            let left = normalized(&anchor.left);
            let pattern = *pattern_of.entry(left.clone()).or_insert_with(|| {
                patterns.push(left);
                by_pattern.push(Vec::new());
                patterns.len() - 1
            });
            by_pattern[pattern].push(index);
        }
        let automaton = AhoCorasick::new(&patterns).map_err(|error| {
            AppError::Analysis(format!("Failed to index redaction anchors: {error}"))
        })?;
        Ok(Some(Self {
            automaton,
            by_pattern,
            right_anchors: anchors
                .iter()
                .map(|anchor| normalized(&anchor.right))
                .collect(),
            anchors,
            compiled: HashMap::new(),
        }))
    }

    /// Spans whose two anchors both occur in `haystack`, which must be [`normalized`].
    fn spans_in(&self, haystack: &str) -> Vec<usize> {
        let patterns: BTreeSet<usize> = self
            .automaton
            .find_overlapping_iter(haystack)
            .map(|found| found.pattern().as_usize())
            .collect();
        patterns
            .into_iter()
            .flat_map(|pattern| self.by_pattern[pattern].iter().copied())
            .filter(|&index| haystack.contains(&self.right_anchors[index]))
            .collect()
    }

    fn anchored(&mut self, index: usize) -> Option<&AnchoredSpan> {
        let anchor = &self.anchors[index];
        self.compiled
            .entry(index)
            .or_insert_with(|| {
                AnchoredSpan::from_anchors(anchor.left.clone(), anchor.right.clone())
            })
            .as_ref()
    }
}

/// The digest a value is compared by, or `None` for a value that is never a candidate: a personal
/// identifier, or something that is itself a mark or a blank field.
fn candidate_value(value: &str) -> Option<String> {
    if is_personal_identifier(value) || !scan_redaction_spans(value).is_empty() {
        return None;
    }
    Some(sha256_hex(normalized(value).as_bytes()))
}

fn scan_document(matcher: &mut Matcher, document: &SweepDocument, out: &mut Vec<SweepMatch>) {
    let haystack = normalized(&document.text);
    for index in matcher.spans_in(&haystack) {
        let redacted = matcher.anchors[index].document.clone();
        if redacted == document.key {
            continue;
        }
        let passage_start = matcher.anchors[index].passage_start;
        let Some(anchored) = matcher.anchored(index) else {
            continue;
        };
        let mut values: Vec<String> = Vec::new();
        for found in anchored.matches(&document.text) {
            let value = &document.text[found.value.clone()];
            let Some(digest) = candidate_value(value) else {
                continue;
            };
            if values.contains(&digest) {
                continue;
            }
            out.push(SweepMatch {
                document: redacted.clone(),
                passage_start,
                source: MatchSource::Document {
                    key: document.key.clone(),
                    start: character_offset(&document.text, found.passage.start),
                    end: character_offset(&document.text, found.passage.end),
                },
                value_sha256: digest.clone(),
                name_shaped: is_name_shaped(value),
            });
            values.push(digest);
            if values.len() == MAX_VALUES_PER_SOURCE {
                break;
            }
        }
    }
}

#[derive(Deserialize)]
struct RowPayload {
    #[serde(default)]
    headers: Vec<String>,
    #[serde(default)]
    values: Vec<Option<String>>,
}

/// A table row is read as one line: its cells in column order, empty cells left out, joined by
/// single spaces. The row matches a span when that line holds the left anchor, some text and the
/// right anchor in that order **and the text lies inside one cell**. The anchors may be in the
/// neighbouring cells of the same row or in the same cell. Text running across a cell boundary is
/// not a match, because no single recorded value stands between the anchors.
fn scan_row(
    matcher: &mut Matcher,
    snapshot_id: &str,
    row_number: i64,
    payload: &str,
    out: &mut Vec<SweepMatch>,
) {
    let Ok(Some(row)) = serde_json::from_str::<Option<RowPayload>>(payload) else {
        return;
    };
    let mut line = String::new();
    let mut cells: Vec<(std::ops::Range<usize>, usize)> = Vec::new();
    for (column, value) in row.values.iter().enumerate() {
        let Some(value) = value.as_deref().filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        if !line.is_empty() {
            line.push(' ');
        }
        let start = line.len();
        line.push_str(value);
        cells.push((start..line.len(), column));
    }
    let haystack = normalized(&line);
    for index in matcher.spans_in(&haystack) {
        let redacted = matcher.anchors[index].document.clone();
        let passage_start = matcher.anchors[index].passage_start;
        let Some(anchored) = matcher.anchored(index) else {
            continue;
        };
        let mut values: Vec<String> = Vec::new();
        for found in anchored.matches(&line) {
            let Some((_, column)) = cells
                .iter()
                .find(|(cell, _)| cell.start <= found.value.start && found.value.end <= cell.end)
            else {
                continue;
            };
            let value = &line[found.value.clone()];
            let Some(digest) = candidate_value(value) else {
                continue;
            };
            if values.contains(&digest) {
                continue;
            }
            out.push(SweepMatch {
                document: redacted.clone(),
                passage_start,
                source: MatchSource::TableCell {
                    snapshot_id: snapshot_id.to_string(),
                    row_number,
                    column_name: row
                        .headers
                        .get(*column)
                        .cloned()
                        .unwrap_or_else(|| format!("column {}", column + 1)),
                },
                value_sha256: digest.clone(),
                name_shaped: is_name_shaped(value),
            });
            values.push(digest);
            if values.len() == MAX_VALUES_PER_SOURCE {
                break;
            }
        }
    }
}

fn build_findings(candidates: Vec<SweepCandidate>, now: DateTime<Utc>) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut per_document: HashMap<(String, i64), usize> = HashMap::new();
    let mut candidates = candidates.into_iter().peekable();
    while let Some(first) = candidates.next() {
        let mut sources = vec![first];
        while let Some(next) = candidates.next_if(|next| {
            next.document == sources[0].document && next.passage_start == sources[0].passage_start
        }) {
            sources.push(next);
        }
        let document = &sources[0].document;
        let reported = per_document
            .entry((document.portal_key.clone(), document.document_id))
            .or_default();
        if *reported == MAX_FINDINGS_PER_DOCUMENT {
            continue;
        }
        if let Some(finding) = build_finding(&sources, now) {
            *reported += 1;
            findings.push(finding);
        }
    }
    findings
}

fn build_finding(sources: &[SweepCandidate], now: DateTime<Utc>) -> Option<Finding> {
    let first = sources.first()?;
    let span = first;
    if sources
        .iter()
        .any(|source| source.value_sha256 != first.value_sha256)
    {
        return None;
    }
    let independent: BTreeSet<&str> = sources
        .iter()
        .map(|source| source.source_digest.as_str())
        .collect();
    let names_a_person = span.person_like || sources.iter().any(|source| source.name_shaped);
    if names_a_person && independent.len() < 2 {
        return None;
    }
    let subject = FindingSubject {
        portal_key: Some(span.document.portal_key.clone()),
        request_id: first.request_id,
        document_id: Some(span.document.document_id),
        ..Default::default()
    };
    let salient = format!("passage={}-{}", span.passage_start, span.passage_end);
    let id = finding_id(
        FindingKind::RedactionCandidateInferred,
        SLUG,
        &subject,
        &[&salient],
    );
    let file_name = first
        .file_name
        .clone()
        .unwrap_or_else(|| format!("document {}", span.document.document_id));
    let count = sources.len();
    let plural = if count == 1 { "" } else { "s" };

    let mut citations = vec![FindingCitation {
        portal_key: span.document.portal_key.clone(),
        document_id: span.document.document_id,
        character_start: Some(span.passage_start),
        character_end: Some(span.passage_end),
        quote: Some(span.quote.clone()).filter(|quote| !quote.is_empty()),
        label: Some(format!(
            "redaction mark and the words around it in {file_name}"
        )),
    }];
    let mut listed = Vec::new();
    let mut tables = Vec::new();
    for source in sources.iter().take(MAX_CITED_SOURCES) {
        match &source.source {
            MatchSource::Document { key, start, end } => {
                let source_file = source
                    .source_file_name
                    .clone()
                    .unwrap_or_else(|| format!("document {}", key.document_id));
                listed.push(format!(
                    "{source_file} (document {} from source '{}'), characters {start}–{end}",
                    key.document_id, key.portal_key
                ));
                citations.push(FindingCitation {
                    portal_key: key.portal_key.clone(),
                    document_id: key.document_id,
                    character_start: Some(*start),
                    character_end: Some(*end),
                    quote: None,
                    label: Some(format!(
                        "the same surrounding words with text between them in {source_file}"
                    )),
                });
            }
            MatchSource::TableCell {
                row_number,
                column_name,
                ..
            } => {
                let table = source.table.as_ref()?;
                listed.push(format!(
                    "row {row_number}, column '{column_name}' of the imported table '{}'",
                    table.display_name
                ));
                tables.push(format!(
                    "{}. '{}' ({}), retrieved {}, sha256 {}: row {row_number}, column '{column_name}'",
                    tables.len() + 1,
                    table.display_name,
                    table
                        .source_uri
                        .as_deref()
                        .unwrap_or("no source address recorded"),
                    table.retrieved_at.get(..10).unwrap_or(&table.retrieved_at),
                    table.source_sha256,
                ));
            }
        }
    }
    let more = count.saturating_sub(MAX_CITED_SOURCES);
    let more = if more == 0 {
        String::new()
    } else {
        format!("; and {more} more")
    };
    let tables = if tables.is_empty() {
        String::new()
    } else {
        format!("\n\nTables read:\n{}", tables.join("\n"))
    };
    let explanation = format!(
        "A redaction mark in {file_name} (characters {start}–{end}) stands between words that \
         also appear, in the same order, in {count} other source{plural}, with text where this \
         document has the mark: {listed}{more}. This reports matching words around the mark. It \
         does not show what the redaction covers: agencies reuse wording in templates and in \
         amended versions of a record, so the other passage may concern a different record, date \
         or person. Compare the cited passages before relying on either.{tables}",
        start = span.passage_start,
        end = span.passage_end,
        listed = listed.join("; "),
    );
    Some(Finding {
        id,
        kind: FindingKind::RedactionCandidateInferred,
        severity: if independent.len() >= 2 {
            FindingSeverity::Serious
        } else {
            FindingSeverity::Notable
        },
        confidence: FindingConfidence::Unassessed,
        subject,
        headline: format!(
            "Words around a redaction in {file_name} appear in {count} other source{plural} \
             with text in its place"
        ),
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
    use crate::types::{AnalyticalDatasetKind, AnalyticalImportRequest};
    use duckdb::params;

    const SCHEMA: &str = include_str!("../../../migrations/0001_initial_schema.sql");
    const LEAD_IN: &str = "Minutes of the finance committee. The city approved funding of";
    const TAIL: &str = "to operate the public bathrooms during the winter season.";

    async fn fresh_repo() -> DuckDbRepository {
        let repository = DuckDbRepository::new(":memory:").expect("open in-memory core");
        repository.run_migrations(SCHEMA).await.expect("migrate");
        repository
    }

    fn insert_source(conn: &duckdb::Connection, portal: &str, kind: &str) {
        conn.execute(
            "INSERT INTO portals (subdomain, display_name, base_url, source_kind)
             VALUES (?1, ?1, 'https://example.test', ?2)",
            params![portal, kind],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO requests (id, portal_subdomain, title, status, created_at, updated_at)
             VALUES (1, ?1, 'Fixture', 'closed', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            params![portal],
        )
        .unwrap();
    }

    fn insert_document(conn: &duckdb::Connection, portal: &str, id: i64, text: &str) {
        conn.execute(
            "INSERT INTO documents (id, request_id, portal_subdomain, file_name, visibility,
                created_at, updated_at, extracted_text_sha256)
             VALUES (?1, 1, ?2, ?3, 'public', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', ?4)",
            params![
                id,
                portal,
                format!("{portal}-{id}.pdf"),
                sha256_hex(text.as_bytes())
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO documents_fts (document_id, portal_subdomain, file_name, extracted_text)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, portal, format!("{portal}-{id}.pdf"), text],
        )
        .unwrap();
    }

    fn redacted() -> String {
        format!("{LEAD_IN} [REDACTED] {TAIL}")
    }

    fn released(value: &str) -> String {
        format!("Résumé of the vote. {LEAD_IN} {value} {TAIL} Carried.")
    }

    fn slice(text: &str, citation: &FindingCitation) -> String {
        let start = citation.character_start.unwrap() as usize;
        let end = citation.character_end.unwrap() as usize;
        text.chars().skip(start).take(end - start).collect()
    }

    #[tokio::test]
    async fn an_empty_corpus_yields_nothing() {
        let repository = fresh_repo().await;
        assert!(run(&repository).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn words_that_only_occur_nearby_are_not_a_match() {
        let repository = fresh_repo().await;
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            insert_document(&conn, "city", 1, &redacted());
            insert_document(
                &conn,
                "city",
                2,
                &format!("$48,200 was requested. {LEAD_IN} [REDACTED] {TAIL} Vendor paid $48,200."),
            );
            insert_document(
                &conn,
                "city",
                3,
                "The city approved funding of the public bathrooms: $48,200 to operate them \
                 during the winter season.",
            );
        }
        assert!(run(&repository).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn text_between_the_same_anchors_in_another_portal_is_cited_on_both_sides() {
        let repository = fresh_repo().await;
        let source_text = released("$48,200");
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            insert_source(&conn, "county", "NEXT_REQUEST");
            insert_document(&conn, "city", 7, &redacted());
            // The same id under another portal is a different document, not the document itself.
            insert_document(&conn, "county", 7, &source_text);
        }
        let findings = run(&repository).await.unwrap();
        let [finding] = &findings[..] else {
            panic!("one redaction, one finding: {findings:#?}");
        };
        assert_eq!(finding.subject.portal_key.as_deref(), Some("city"));
        assert_eq!(finding.subject.document_id, Some(7));
        assert_eq!(finding.confidence, FindingConfidence::Unassessed);
        let [mark, other] = &finding.citations[..] else {
            panic!("both passages are cited: {:#?}", finding.citations);
        };
        assert_eq!((mark.portal_key.as_str(), mark.document_id), ("city", 7));
        assert_eq!(
            slice(&redacted(), mark),
            "The city approved funding of [REDACTED] to operate the public bathrooms"
        );
        assert_eq!(
            (other.portal_key.as_str(), other.document_id),
            ("county", 7)
        );
        assert_eq!(
            slice(&source_text, other),
            "The city approved funding of $48,200 to operate the public bathrooms"
        );
        let stored = serde_json::to_string(finding).unwrap();
        assert!(!stored.contains("48,200"), "the text is cited, not printed");
        assert!(!stored.contains("likely"));
        assert!(finding.explanation.contains("source 'county'"));
    }

    #[tokio::test]
    async fn two_portals_with_the_same_document_ids_keep_their_own_findings() {
        let repository = fresh_repo().await;
        let other_lead = "Report of the parks board. The county renewed its lease of";
        let other_tail = "for the storage of maintenance equipment through next year.";
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            insert_source(&conn, "county", "NEXT_REQUEST");
            insert_document(&conn, "city", 7, &redacted());
            insert_document(
                &conn,
                "county",
                7,
                &format!("{other_lead} [REDACTED] {other_tail}"),
            );
            insert_document(
                &conn,
                "city",
                8,
                &format!("{other_lead} lot 14 {other_tail}"),
            );
            insert_document(&conn, "county", 8, &released("$48,200"));
        }
        let mut cited: Vec<_> = run(&repository)
            .await
            .unwrap()
            .into_iter()
            .map(|finding| {
                let other = &finding.citations[1];
                (
                    finding.subject.portal_key.unwrap(),
                    finding.subject.document_id.unwrap(),
                    other.portal_key.clone(),
                    other.document_id,
                )
            })
            .collect();
        cited.sort();
        assert_eq!(
            cited,
            vec![
                ("city".to_string(), 7, "county".to_string(), 8),
                ("county".to_string(), 7, "city".to_string(), 8),
            ]
        );
    }

    #[tokio::test]
    async fn a_redacted_document_outside_the_newest_two_hundred_meets_a_later_source() {
        let repository = fresh_repo().await;
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            insert_document(&conn, "city", 1, &redacted());
            for id in 2..=260 {
                insert_document(&conn, "city", id, &format!("Routine notice number {id}."));
            }
        }
        assert!(run(&repository).await.unwrap().is_empty());
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_document(&conn, "city", 900, &released("$48,200"));
        }
        let findings = run(&repository).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].subject.document_id, Some(1));
        assert_eq!(findings[0].citations[1].document_id, 900);
        // Nothing is new on the next run; the stored match is still reported, so the registry's
        // stale sweep does not drop it.
        let again = run(&repository).await.unwrap();
        assert_eq!(again[0].id, findings[0].id);
    }

    #[tokio::test]
    async fn a_hand_added_file_is_neither_a_source_nor_a_subject() {
        let repository = fresh_repo().await;
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            insert_source(&conn, "source-local-1", "LOCAL_FILE");
            insert_source(&conn, "source-paste-1", "PASTED_CONTENT");
            insert_document(&conn, "city", 1, &redacted());
            insert_document(&conn, "source-local-1", 2, &released("$48,200"));
            insert_document(&conn, "source-paste-1", 3, &redacted());
            insert_document(
                &conn,
                "city",
                4,
                "An unrelated notice about street sweeping.",
            );
        }
        assert!(run(&repository).await.unwrap().is_empty());
        // The same text under a collected source is reported, and only for the collected subject.
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_document(&conn, "city", 5, &released("$48,200"));
        }
        let findings = run(&repository).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert!(!serde_json::to_string(&findings)
            .unwrap()
            .contains("source-"));
    }

    #[tokio::test]
    async fn work_past_a_bound_is_done_by_the_next_run() {
        let repository = fresh_repo().await;
        let limits = SweepLimits {
            new_documents: 2,
            rescan_documents: 1,
            ..SweepLimits::default()
        };
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            insert_document(
                &conn,
                "city",
                1,
                "An unrelated notice about street sweeping.",
            );
            insert_document(&conn, "city", 2, &released("$48,200"));
            insert_document(
                &conn,
                "city",
                3,
                "A second unrelated notice about tree trimming.",
            );
            insert_document(&conn, "city", 4, &redacted());
        }
        // Run 1 takes in documents 1 and 2; nothing is redacted yet.
        assert!(run_bounded(&repository, &limits).await.unwrap().is_empty());
        // Run 2 takes in 3 and 4, and may read again only one of the two older documents: 1.
        assert!(run_bounded(&repository, &limits).await.unwrap().is_empty());
        // Run 3 reads the other one.
        let findings = run_bounded(&repository, &limits).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].citations[1].document_id, 2);
    }

    #[tokio::test]
    async fn a_name_shaped_value_needs_two_independent_sources_and_is_never_printed() {
        let repository = fresh_repo().await;
        let one = released("Jordan Example");
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            insert_source(&conn, "county", "NEXT_REQUEST");
            insert_document(&conn, "city", 1, &redacted());
            insert_document(&conn, "city", 2, &one);
            // A second copy of the same file is not a second observation.
            insert_document(&conn, "county", 2, &one);
        }
        assert!(run(&repository).await.unwrap().is_empty());
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_document(
                &conn,
                "county",
                3,
                &format!("Agenda item 4. {LEAD_IN} Jordan Example {TAIL}"),
            );
        }
        let findings = run(&repository).await.unwrap();
        assert_eq!(findings.len(), 1, "{findings:#?}");
        assert_eq!(findings[0].severity, FindingSeverity::Serious);
        assert!(!serde_json::to_string(&findings).unwrap().contains("Jordan"));
    }

    #[tokio::test]
    async fn two_different_values_between_the_anchors_are_reused_wording_not_a_finding() {
        let repository = fresh_repo().await;
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            insert_document(&conn, "city", 1, &redacted());
            insert_document(&conn, "city", 2, &released("$48,200"));
            insert_document(&conn, "city", 3, &released("$51,900"));
        }
        assert!(run(&repository).await.unwrap().is_empty());
        // A personal identifier between the anchors is never matched at all.
        let repository = fresh_repo().await;
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            insert_document(&conn, "city", 1, &redacted());
            insert_document(&conn, "city", 2, &released("(619) 555-0142"));
        }
        assert!(run(&repository).await.unwrap().is_empty());
    }

    /// A released ledger page: each line is one row, so the words a table row can share with it
    /// are the ones on the mark's own line.
    const TABLE_LINE: &str = "Ledger extract, page 3\n\
        2024-02-01 Street sweeping HSS-2024-0098 9100.00 Paid in full quarterly\n\
        2024-03-01 Shelter operations [REDACTED] 48200.00 Paid in full quarterly\n";

    async fn import_table(repository: &DuckDbRepository, directory: &std::path::Path) -> String {
        let path = directory.join("contracts.csv");
        std::fs::write(
            &path,
            "posted,programme,contract,amount,note\n\
             2024-02-01,Street sweeping,HSS-2024-0098,9100.00,Paid in full quarterly\n\
             2024-03-01,Shelter operations,HSS-2024-0117,48200.00,Paid in full quarterly\n",
        )
        .unwrap();
        repository
            .import_analytical_csv(AnalyticalImportRequest {
                dataset_kind: AnalyticalDatasetKind::RegionalData,
                display_name: "Contract ledger 2024".into(),
                source_uri: Some("https://example.test/contracts.csv".into()),
                local_path: path.to_string_lossy().into_owned(),
                observation_period_start: None,
                observation_period_end: None,
                partial_dataset: false,
                worksheet: None,
                source_file_name: None,
                expected_source_sha256: None,
                derived_subset: None,
            })
            .await
            .unwrap()
            .snapshot_id
    }

    #[tokio::test]
    async fn a_table_cell_between_the_anchors_is_cited_by_snapshot_row_and_column() {
        let directory = tempfile::tempdir().unwrap();
        let repository = fresh_repo().await;
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            insert_document(&conn, "city", 1, TABLE_LINE);
        }
        let snapshot_id = import_table(&repository, directory.path()).await;
        let sha256: String = repository
            .analytics_connection()
            .await
            .unwrap()
            .query_row(
                "SELECT source_sha256 FROM analytical_snapshots WHERE snapshot_id = ?1",
                params![snapshot_id],
                |row| row.get(0),
            )
            .unwrap();
        // One row a run: the matching row is the second, so the first run cannot reach it.
        let limits = SweepLimits {
            table_rows: 1,
            ..SweepLimits::default()
        };
        assert!(run_bounded(&repository, &limits).await.unwrap().is_empty());
        let findings = run_bounded(&repository, &limits).await.unwrap();
        let [finding] = &findings[..] else {
            panic!("the second row matches: {findings:#?}");
        };
        assert_eq!(finding.citations.len(), 1, "a table is cited in the text");
        assert!(
            finding.explanation.contains(&format!(
                "Tables read:\n1. 'Contract ledger 2024' (https://example.test/contracts.csv), \
                 retrieved {}, sha256 {sha256}: row 2, column 'contract'",
                Utc::now().format("%Y-%m-%d")
            )),
            "{}",
            finding.explanation
        );
        assert!(!finding.explanation.contains("HSS-2024-0117"));
    }

    #[test]
    fn a_table_value_must_lie_inside_one_cell() {
        let anchor = |passage_start: i64, left: &str| SweepAnchor {
            document: DocumentKey {
                portal_key: "city".into(),
                document_id: 1,
            },
            passage_start,
            left_anchor: "words from the line above".into(),
            right_anchor: "words from the line below".into(),
            row_anchors: Some((left.into(), "48200.00 Paid in full".into())),
        };
        let mut matcher = Matcher::for_tables(&[
            anchor(0, "A 2024-03-01 Shelter operations"),
            anchor(1, "Extract A 2024-03-01"),
        ])
        .unwrap()
        .unwrap();
        let payload = serde_json::json!({
            "headers": ["page", "posted", "programme", "contract", "amount", "note"],
            "values": ["Extract A", "2024-03-01", "Shelter operations", "HSS-2024-0117", "48200.00", "Paid in full quarterly"],
        })
        .to_string();
        let mut matches = Vec::new();
        scan_row(&mut matcher, "snapshot", 2, &payload, &mut matches);
        let found: Vec<_> = matches
            .iter()
            .map(|found| (found.passage_start, &found.source))
            .collect();
        assert_eq!(
            found,
            vec![(
                0,
                &MatchSource::TableCell {
                    snapshot_id: "snapshot".into(),
                    row_number: 2,
                    column_name: "contract".into(),
                }
            )],
            "'Shelter operations HSS-2024-0117' runs across two cells and is not one value"
        );
    }

    /// A core written before the sweep tables existed, in a file at the storage version older
    /// cores carry, is swept after the upgrade.
    #[tokio::test]
    async fn a_core_that_predates_the_sweep_tables_is_swept_after_the_upgrade() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("core.db");
        {
            let legacy = SCHEMA
                .split("-- REDACTION SWEEP")
                .next()
                .expect("the schema names the section");
            assert!(!legacy.contains("redaction_sweep_documents"));
            let conn = duckdb::Connection::open(&path).unwrap();
            conn.execute_batch(legacy).unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            insert_document(&conn, "city", 1, &redacted());
            insert_document(&conn, "city", 2, &released("$48,200"));
        }
        let repository = DuckDbRepository::new(&path).unwrap();
        repository.run_migrations(SCHEMA).await.unwrap();
        assert_eq!(run(&repository).await.unwrap().len(), 1);
        drop(repository);
        let repository = DuckDbRepository::new(&path).unwrap();
        repository.run_migrations(SCHEMA).await.unwrap();
        assert_eq!(run(&repository).await.unwrap().len(), 1);
    }

    /// Not a correctness test: prints what one run costs against a 300,000-row imported table.
    /// Of the 2,000 one-mark documents a run takes in `new_documents`, so that many marks are
    /// compared. Run with `--ignored --nocapture`.
    #[tokio::test]
    #[ignore = "measurement"]
    async fn table_scan_cost() {
        use std::fmt::Write;
        let directory = tempfile::tempdir().unwrap();
        let repository = fresh_repo().await;
        let mut csv = String::from("posted,programme,contract,amount,note\n");
        for row in 0..300_000 {
            writeln!(
                csv,
                "2024-03-{:02},Programme number {row},HSS-2024-{row:06},{}.00,Paid in full quarterly",
                row % 28 + 1,
                row * 7
            )
            .unwrap();
        }
        let path = directory.path().join("ledger.csv");
        std::fs::write(&path, csv).unwrap();
        {
            let conn = repository.analytics_connection().await.unwrap();
            insert_source(&conn, "city", "NEXT_REQUEST");
            for id in 0..2_000_i64 {
                let row = id * 150;
                insert_document(
                    &conn,
                    "city",
                    id,
                    &format!(
                        "Ledger page\n2024-03-{:02} Programme number {row} [REDACTED] {}.00 Paid in full quarterly\n",
                        row % 28 + 1,
                        row * 7
                    ),
                );
            }
        }
        repository
            .import_analytical_csv(AnalyticalImportRequest {
                dataset_kind: AnalyticalDatasetKind::RegionalData,
                display_name: "Synthetic ledger".into(),
                source_uri: None,
                local_path: path.to_string_lossy().into_owned(),
                observation_period_start: None,
                observation_period_end: None,
                partial_dataset: false,
                worksheet: None,
                source_file_name: None,
                expected_source_sha256: None,
                derived_subset: None,
            })
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let findings = run(&repository).await.unwrap();
        println!(
            "300,000 rows, {} marks: {:?}",
            findings.len(),
            started.elapsed()
        );
        assert_eq!(findings.len(), SweepLimits::default().new_documents);
    }

    #[test]
    fn blank_fields_are_not_explicit_redaction_marks() {
        assert!(!is_explicit_mark("_______________"));
        assert!(!is_explicit_mark("XXXXXXX"));
        assert!(is_explicit_mark("[REDACTED]"));
        assert!(is_explicit_mark("(b)(6)"));
        assert!(is_explicit_mark("\u{2588}\u{2588}\u{2588}"));
    }
}
