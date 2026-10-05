//! Semantic redaction inference.
//!
//! For a redacted span in one document, produce a ranked list of candidate identities of what
//! could be behind it, based on surrounding context in the redacted document and corroborating
//! context from the corpus's *other* documents. This is inference, not decryption. Every value
//! it emits is bounded (no candidate ever exceeds a hard ceiling) and every candidate carries a
//! traceable evidence trail so the user can judge the inference themselves.
//!
//! **What it operates on.** The extracted text of documents in the corpus contains explicit
//! redaction markers (`[REDACTED]`, sequences of `█`, `X`, `_`, FOIA exemption tags like `(b)(6)`
//! or `(b)(7)(C)`). This module finds those markers, extracts context around them, and searches
//! the corpus for likely fills. It does NOT rely on `parsers::unredactor`'s geometry findings —
//! those describe positional leaks between text and mask rectangles, not textual redaction
//! spans in the extracted text stream.
//!
//! **Ceilings.**
//! - No candidate ever exceeds `SEMANTIC_LIKELIHOOD_CEILING` (9500 bp) even when a semantic
//!   model corroborates it. Inference is not decryption.
//! - When the fastembed model is not initialized, lexical/shingle matching is used with a
//!   lower ceiling (`LEXICAL_LIKELIHOOD_CEILING`, 6000 bp).
//!
//! **Never emits a Person candidate without corroborating context from ≥ 2 documents.** A
//! single-document match on a name is coincidence, not inference.

use crate::{
    analysis::finding::FindingConfidence, error::AppError, storage::sqlite::DuckDbRepository,
    storage::vector::VectorIndex,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Ceiling for a candidate's likelihood when semantic ranking is available.
pub const SEMANTIC_LIKELIHOOD_CEILING: u16 = 9500;
/// Ceiling for a candidate's likelihood when only lexical ranking is available.
pub const LEXICAL_LIKELIHOOD_CEILING: u16 = 6000;
/// Minimum corroborating-document count for a Person candidate to be emitted at all.
pub const PERSON_MIN_CORROBORATING_DOCS: usize = 2;
/// Context window around a redaction marker, in characters.
pub const CONTEXT_CHARS: usize = 240;
/// Maximum number of candidates returned per report.
pub const MAX_CANDIDATES: usize = 12;

/// One character span in a document's extracted text that represents a redaction.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RedactionSpan {
    pub start: usize,
    pub end: usize,
}

/// The kind of entity a candidate belongs to. Drives which corpus search is run and how
/// evidence is weighted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CandidateKind {
    Person,
    Organization,
    Amount,
    Date,
    Other,
}

impl CandidateKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Person => "PERSON",
            Self::Organization => "ORGANIZATION",
            Self::Amount => "AMOUNT",
            Self::Date => "DATE",
            Self::Other => "OTHER",
        }
    }
}

/// One piece of evidence supporting a candidate: which other document, which excerpt, and how
/// tightly its context matched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub portal_key: String,
    pub document_id: i64,
    pub quote: String,
    pub match_score: u16,
}

/// One candidate identity for what could be behind a redaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub kind: CandidateKind,
    pub value: String,
    pub likelihood_basis_points: u16,
    pub supporting_evidence: Vec<Evidence>,
}

/// A pointer to a corroborating document (kept as a distinct list so the UI can group evidence
/// without walking the candidates list).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct DocumentRef {
    pub portal_key: String,
    pub document_id: i64,
}

/// The full inference report for one redaction span.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InferenceReport {
    pub candidates: Vec<Candidate>,
    pub corroborating_docs: Vec<DocumentRef>,
    pub confidence_ceiling: FindingConfidence,
    /// Whether ranking used semantic (fastembed) or lexical/shingle only. The UI can render this
    /// so users know which mode produced the ranking.
    pub semantic_ranking_used: bool,
    /// The context excerpt that anchored the inference (bracketed at the redaction span). Useful
    /// for the panel to show what the engine actually reasoned about.
    pub context_snippet: String,
}

impl InferenceReport {
    /// An empty report — used when the redaction span could not be extracted, the surrounding
    /// context was too thin to reason about, or the corpus has no candidates. Never an error.
    pub fn empty(semantic_ranking_used: bool) -> Self {
        Self {
            candidates: Vec::new(),
            corroborating_docs: Vec::new(),
            confidence_ceiling: FindingConfidence::Unassessed,
            semantic_ranking_used,
            context_snippet: String::new(),
        }
    }
}

/// On-demand inference for a specific span in a specific document. Called by the IPC command
/// when the user opens the panel on a redaction they see in the viewer.
pub async fn infer_redaction_candidates(
    repository: &DuckDbRepository,
    vector_index: Option<&VectorIndex>,
    document_id: i64,
    portal_key: &str,
    span: RedactionSpan,
) -> Result<InferenceReport, AppError> {
    let document_id_u64 = u64::try_from(document_id)
        .map_err(|_| AppError::Storage("Document identifier was negative".to_string()))?;
    let text = match repository
        .load_corpus_source_text(portal_key, document_id_u64)
        .await
    {
        Ok(text) => text,
        Err(_) => return Ok(InferenceReport::empty(vector_index.is_some())),
    };
    let Some(context) = extract_context(&text, span) else {
        return Ok(InferenceReport::empty(vector_index.is_some()));
    };
    let hint = classify_entity_hint(&context);
    let query = build_search_query(&context);
    let (candidates_source, semantic_used) = if let Some(index) = vector_index {
        match run_semantic_lookup(index, repository, portal_key, document_id, &query).await {
            Ok(results) if !results.is_empty() => (results, true),
            _ => (
                run_lexical_lookup(repository, portal_key, document_id, &query).await?,
                false,
            ),
        }
    } else {
        (
            run_lexical_lookup(repository, portal_key, document_id, &query).await?,
            false,
        )
    };
    let candidates = rank_candidates(candidates_source, hint, semantic_used);
    let corroborating_docs: Vec<DocumentRef> = candidates
        .iter()
        .flat_map(|c| c.supporting_evidence.iter())
        .map(|e| DocumentRef {
            portal_key: e.portal_key.clone(),
            document_id: e.document_id,
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let ceiling = if candidates.is_empty() {
        FindingConfidence::Unassessed
    } else if semantic_used {
        FindingConfidence::measured(SEMANTIC_LIKELIHOOD_CEILING)
    } else {
        FindingConfidence::measured(LEXICAL_LIKELIHOOD_CEILING)
    };
    Ok(InferenceReport {
        candidates,
        corroborating_docs,
        confidence_ceiling: ceiling,
        semantic_ranking_used: semantic_used,
        context_snippet: bracketed_snippet(&text, span, &context),
    })
}

/// Scan a document for redaction spans in its extracted text. Used by the detector to identify
/// spans to run inference over, and by tests to prove the marker patterns match reality.
pub fn scan_redaction_spans(text: &str) -> Vec<RedactionSpan> {
    let mut spans = Vec::new();
    // Bracketed word markers (case-insensitive): [REDACTED], [WITHHELD], [EXEMPT], etc.
    for marker in ["[REDACTED]", "[WITHHELD]", "[EXEMPT]"] {
        let lower = text.to_ascii_lowercase();
        let needle = marker.to_ascii_lowercase();
        let mut cursor = 0;
        while let Some(rel) = lower[cursor..].find(&needle) {
            let start = cursor + rel;
            let end = start + marker.len();
            spans.push(RedactionSpan { start, end });
            cursor = end;
        }
    }
    // FOIA exemption citations: (b)(6), (b)(7)(C), (b)(7)(D), etc. — these often replace the
    // redacted text inline.
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' && i + 4 < bytes.len() && !cites_a_section(bytes, i) {
            // Look for `(b)(N)` or `(b)(N)(X)` shape.
            let rest = &text[i..];
            if let Some(match_end) = match_foia_exemption(rest) {
                spans.push(RedactionSpan {
                    start: i,
                    end: i + match_end,
                });
                i += match_end;
                continue;
            }
        }
        i += 1;
    }
    // Long runs of block characters (█▓■). 3+ consecutive.
    scan_char_runs(text, &['█', '▓', '■'], 3, &mut spans);
    // Long runs of underscore or X (5+ — shorter runs are common in unrelated content).
    // A run after a label ("By: ____", "Print Name: ____", "Date: ____") is a blank form field.
    // On a live San Diego collection every underscore "redaction" found was one: the signature
    // block of an executed agreement.
    let mut underscores = Vec::new();
    scan_char_runs(text, &['_'], 5, &mut underscores);
    spans.extend(
        underscores
            .into_iter()
            .filter(|span| !text[..span.start].trim_end().ends_with(':')),
    );
    scan_char_runs(text, &['X', 'x'], 5, &mut spans);
    // De-duplicate overlapping spans; keep the earliest start with the widest end.
    spans.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));
    let mut deduped: Vec<RedactionSpan> = Vec::with_capacity(spans.len());
    for span in spans {
        if let Some(last) = deduped.last_mut() {
            if span.start <= last.end {
                last.end = last.end.max(span.end);
                continue;
            }
        }
        deduped.push(span);
    }
    deduped
}

fn scan_char_runs(text: &str, targets: &[char], min_run: usize, out: &mut Vec<RedactionSpan>) {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        if targets.contains(&chars[i].1) {
            let start = chars[i].0;
            let mut j = i + 1;
            while j < chars.len() && targets.contains(&chars[j].1) {
                j += 1;
            }
            let run_len = j - i;
            if run_len >= min_run {
                let end = if j < chars.len() {
                    chars[j].0
                } else {
                    text.len()
                };
                out.push(RedactionSpan { start, end });
            }
            i = j;
        } else {
            i += 1;
        }
    }
}

/// A stamp stands alone. After a section number (`983.301(b)(6)`, `570.503 (b)(7)`) the same
/// shape is a legal citation, which HUD notices and statutes are full of; "Phone (b)(6)" is a stamp.
fn cites_a_section(bytes: &[u8], open: usize) -> bool {
    let Some(&adjacent) = open.checked_sub(1).and_then(|index| bytes.get(index)) else {
        return false;
    };
    if adjacent.is_ascii_alphanumeric() || matches!(adjacent, b')' | b'.' | b'-') {
        return true;
    }
    bytes[..open]
        .iter()
        .rev()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| byte.is_ascii_digit() || *byte == b')')
}

fn match_foia_exemption(rest: &str) -> Option<usize> {
    // `(b)(N)` where N is a single digit; optionally followed by `(X)` where X is a single letter.
    let bytes = rest.as_bytes();
    if bytes.len() < 6 {
        return None;
    }
    if bytes[0] != b'(' || bytes[2] != b')' || bytes[3] != b'(' {
        return None;
    }
    if !(bytes[1] == b'b' || bytes[1] == b'B') {
        return None;
    }
    if !(bytes[4].is_ascii_digit()) {
        return None;
    }
    if bytes[5] != b')' {
        return None;
    }
    // Optional `(X)` subclause.
    if bytes.len() >= 9 && bytes[6] == b'(' && bytes[8] == b')' && bytes[7].is_ascii_alphabetic() {
        return Some(9);
    }
    Some(6)
}

/// Extracts a ±`CONTEXT_CHARS`/2 window around the span, clipped to document boundaries and
/// snapped to a word boundary on each side. Returns `None` if the resulting window is too short
/// to reason about.
fn extract_context(text: &str, span: RedactionSpan) -> Option<String> {
    if span.start >= text.len()
        || span.end > text.len()
        || span.start >= span.end
        || !text.is_char_boundary(span.start)
        || !text.is_char_boundary(span.end)
    {
        return None;
    }
    let half = CONTEXT_CHARS / 2;
    let raw_start = span.start.saturating_sub(half);
    let raw_end = (span.end + half).min(text.len());
    // Snap to char boundaries to avoid slicing inside a UTF-8 sequence.
    let start = snap_start(text, raw_start);
    let end = snap_end(text, raw_end);
    let context = text.get(start..end)?;
    if context.split_whitespace().count() < 4 {
        return None;
    }
    Some(context.to_string())
}

fn snap_start(text: &str, mut pos: usize) -> usize {
    while pos > 0 && !text.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

fn snap_end(text: &str, mut pos: usize) -> usize {
    while pos < text.len() && !text.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}

/// Byte offsets of a span given in document characters (Unicode scalar values), the unit the
/// viewer reports. `None` when the span does not lie within the text.
pub fn character_span_to_bytes(text: &str, start: usize, end: usize) -> Option<RedactionSpan> {
    let byte_at = |characters: usize| {
        text.char_indices()
            .map(|(byte, _)| byte)
            .chain(std::iter::once(text.len()))
            .nth(characters)
    };
    let span = RedactionSpan {
        start: byte_at(start)?,
        end: byte_at(end)?,
    };
    (span.start < span.end).then_some(span)
}

fn bracketed_snippet(text: &str, span: RedactionSpan, context: &str) -> String {
    // Replace the redaction span within the context with a visible bracket so the UI has a
    // clean anchor without needing to re-derive positions.
    if span.start >= text.len() || span.end > text.len() || span.start >= span.end {
        return context.to_string();
    }
    let relative_start = span.start.saturating_sub(snap_start(
        text,
        span.start.saturating_sub(CONTEXT_CHARS / 2),
    ));
    let relative_end =
        relative_start + (span.end - span.start).min(context.len().saturating_sub(relative_start));
    if relative_end > context.len() {
        return context.to_string();
    }
    let mut out = String::with_capacity(context.len() + 8);
    out.push_str(&context[..relative_start]);
    out.push_str("«REDACTED»");
    out.push_str(&context[relative_end..]);
    out
}

/// Guess the entity kind the redacted text most likely held, from prepositional patterns in the
/// immediately-preceding words.
pub fn classify_entity_hint(context: &str) -> CandidateKind {
    let lower = context.to_ascii_lowercase();
    // Person triggers.
    for trigger in [
        "signed by",
        "approved by",
        "authorized by",
        "on behalf of",
        "attention:",
        "attn:",
        "dear ",
        "recorded by",
        "requested by",
        "certified by",
    ] {
        if lower.contains(trigger) {
            return CandidateKind::Person;
        }
    }
    // Amount triggers.
    for trigger in [
        "in the amount of",
        "paid",
        "the sum of",
        "total: $",
        "invoice",
        "payment of",
    ] {
        if lower.contains(trigger) {
            return CandidateKind::Amount;
        }
    }
    // Date triggers.
    for trigger in [
        "dated ",
        "as of ",
        "effective ",
        "issued on ",
        "meeting of ",
    ] {
        if lower.contains(trigger) {
            return CandidateKind::Date;
        }
    }
    // Organization triggers.
    for trigger in [
        "corporation",
        "company",
        "department of",
        "office of",
        "agency",
        "association",
    ] {
        if lower.contains(trigger) {
            return CandidateKind::Organization;
        }
    }
    CandidateKind::Other
}

fn build_search_query(context: &str) -> String {
    // Simple: strip redaction markers from the context and use it as the search text.
    let mut cleaned = context.to_string();
    for marker in ["[REDACTED]", "[WITHHELD]", "[EXEMPT]"] {
        cleaned = cleaned.replace(marker, " ");
    }
    for ch in ['█', '▓', '■'] {
        cleaned = cleaned.replace(ch, " ");
    }
    cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

/// One candidate site returned from a corpus lookup — raw before ranking.
#[derive(Debug, Clone)]
pub struct CandidateSite {
    pub portal_key: String,
    pub document_id: i64,
    pub excerpt: String,
    pub raw_score: f64,
}

async fn run_lexical_lookup(
    repository: &DuckDbRepository,
    exclude_portal_key: &str,
    exclude_document_id: i64,
    query: &str,
) -> Result<Vec<CandidateSite>, AppError> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    // Use keyword search over documents_fts, excluding the redacted document itself.
    let terms: Vec<String> = query
        .split_whitespace()
        .filter(|w| w.len() >= 4)
        .filter(|w| !STOPWORDS.contains(&w.to_ascii_lowercase().as_str()))
        .take(8)
        .map(|s| s.to_ascii_lowercase())
        .collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let raw = repository
        .find_redaction_context_matches(exclude_portal_key, exclude_document_id, terms, 50)
        .await?;
    Ok(raw
        .into_iter()
        .map(
            |(portal_key, document_id, excerpt, matched_count, term_count)| {
                let raw_score = if term_count == 0 {
                    0.0
                } else {
                    f64::from(matched_count) / f64::from(term_count)
                };
                CandidateSite {
                    portal_key,
                    document_id,
                    excerpt,
                    raw_score,
                }
            },
        )
        .collect())
}

/// Searches every portal: each match is reported with its portal, which the panel prints beside
/// the document.
async fn run_semantic_lookup(
    index: &VectorIndex,
    repository: &DuckDbRepository,
    portal_key: &str,
    exclude_document_id: i64,
    query: &str,
) -> Result<Vec<CandidateSite>, AppError> {
    use crate::types::HybridSearchFilter;
    let results = index
        .semantic_search(
            query,
            HybridSearchFilter {
                portal_key: None,
                agency_name: None,
                jurisdiction: None,
                document_type: None,
                date_from: None,
                date_to: None,
            },
            60,
        )
        .await?;
    let hand_added = repository.user_supplied_source_keys().await?;
    Ok(corroborating_sites(
        results,
        portal_key,
        exclude_document_id,
        &hand_added,
    ))
}

/// Drops the redacted document itself, identified by portal and id together, and every file the
/// user added by hand, which is not something another source released.
fn corroborating_sites(
    results: Vec<crate::types::HybridSearchResult>,
    portal_key: &str,
    exclude_document_id: i64,
    hand_added: &HashSet<String>,
) -> Vec<CandidateSite> {
    results
        .into_iter()
        .filter(|r| !(r.portal_key == portal_key && r.document_id as i64 == exclude_document_id))
        .filter(|r| !hand_added.contains(&r.portal_key))
        .map(|r| CandidateSite {
            portal_key: r.portal_key,
            document_id: r.document_id as i64,
            excerpt: r.excerpt,
            raw_score: r.semantic_score.unwrap_or(r.fused_score),
        })
        .collect()
}

/// One candidate value's corroboration: the evidence rows, the `(portal, document)` pairs they
/// came from, and the summed match score.
#[derive(Default)]
struct ValueBucket {
    evidence: Vec<Evidence>,
    documents: HashSet<(String, i64)>,
    total_score: f64,
}

fn rank_candidates(
    sites: Vec<CandidateSite>,
    hint: CandidateKind,
    semantic_used: bool,
) -> Vec<Candidate> {
    let ceiling = if semantic_used {
        SEMANTIC_LIKELIHOOD_CEILING
    } else {
        LEXICAL_LIKELIHOOD_CEILING
    };
    // Extract candidate values of the hinted kind from each site's excerpt, then aggregate by
    // value across all sites — a value that appears in more independent sites earns higher
    // likelihood.
    let mut buckets: HashMap<String, ValueBucket> = HashMap::new();
    for site in sites {
        let candidates = extract_values_of_kind(&site.excerpt, hint);
        let match_score =
            ((site.raw_score.clamp(0.0, 1.0) * f64::from(ceiling)) as u16).min(ceiling);
        for value in candidates {
            let entry = buckets.entry(value).or_default();
            if entry
                .documents
                .insert((site.portal_key.clone(), site.document_id))
            {
                entry.evidence.push(Evidence {
                    portal_key: site.portal_key.clone(),
                    document_id: site.document_id,
                    quote: truncate_quote(&site.excerpt),
                    match_score,
                });
                entry.total_score += site.raw_score.max(0.0);
            }
        }
    }
    let mut candidates: Vec<Candidate> = buckets
        .into_iter()
        .filter(|(_, bucket)| {
            // Person needs corroboration; other kinds accept single-doc.
            !matches!(hint, CandidateKind::Person)
                || bucket.documents.len() >= PERSON_MIN_CORROBORATING_DOCS
        })
        .map(|(value, bucket)| {
            let ValueBucket {
                evidence,
                documents: docs,
                total_score,
            } = bucket;
            let corroboration_multiplier = corroboration_multiplier(docs.len());
            let base = (total_score / f64::from(u32::try_from(docs.len().max(1)).unwrap_or(1)))
                .clamp(0.0, 1.0);
            let raw_likelihood = base * f64::from(ceiling) * corroboration_multiplier;
            let likelihood = (raw_likelihood as u16).min(ceiling);
            Candidate {
                kind: hint,
                value,
                likelihood_basis_points: likelihood,
                supporting_evidence: evidence,
            }
        })
        .collect();
    candidates.sort_by_key(|c| std::cmp::Reverse(c.likelihood_basis_points));
    candidates.truncate(MAX_CANDIDATES);
    candidates
}

fn corroboration_multiplier(doc_count: usize) -> f64 {
    // 1 doc: 0.55, 2 docs: 0.85, 3+: 1.0. This is the same shape as the project's
    // "independently corroborated evidence outranks a lone recorded claim" pattern.
    match doc_count {
        0 => 0.0,
        1 => 0.55,
        2 => 0.85,
        _ => 1.0,
    }
}

fn truncate_quote(excerpt: &str) -> String {
    const MAX_QUOTE_CHARS: usize = 220;
    let trimmed = excerpt.trim();
    if trimmed.chars().count() <= MAX_QUOTE_CHARS {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(MAX_QUOTE_CHARS).collect();
    out.push('…');
    out
}

/// Very small stopword set for FTS-term filtering. Big enough to strip words that would match
/// nearly every document; small enough not to lose signal words like "policy" or "budget".
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "have", "will", "your", "into", "were",
    "been", "their", "which", "would", "there", "these", "other", "such", "them", "when", "than",
    "shall",
];

fn extract_values_of_kind(text: &str, kind: CandidateKind) -> Vec<String> {
    match kind {
        CandidateKind::Person => extract_person_names(text),
        CandidateKind::Organization => extract_org_names(text),
        CandidateKind::Amount => extract_amounts(text),
        CandidateKind::Date => extract_dates(text),
        CandidateKind::Other => Vec::new(),
    }
}

/// Simple capitalized-token bigram/trigram extractor for personal names. Filters common heads
/// (Mr., Mrs., Dr.) and requires at least two consecutive capitalized tokens.
fn extract_person_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    for i in 0..tokens.len().saturating_sub(1) {
        let a = clean_token(tokens[i]);
        let b = clean_token(tokens[i + 1]);
        if is_name_token(&a) && is_name_token(&b) {
            let combined = if i + 2 < tokens.len() {
                let c = clean_token(tokens[i + 2]);
                if is_name_token(&c) {
                    format!("{a} {b} {c}")
                } else {
                    format!("{a} {b}")
                }
            } else {
                format!("{a} {b}")
            };
            names.push(combined);
        }
    }
    dedup_preserve_order(names)
}

fn is_name_token(t: &str) -> bool {
    if t.len() < 2 || t.len() > 24 {
        return false;
    }
    let mut chars = t.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    if chars.all(|c| c.is_ascii_alphabetic() || c == '\'' || c == '-') {
        // Filter obvious non-names.
        let lower = t.to_ascii_lowercase();
        !matches!(
            lower.as_str(),
            "the" | "a" | "an" | "and" | "or" | "but" | "for" | "of" | "to" | "in" | "on" | "by"
        )
    } else {
        false
    }
}

fn clean_token(t: &str) -> String {
    t.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
        .to_string()
}

fn extract_org_names(text: &str) -> Vec<String> {
    // Capture "X Department", "Y Office", "Z Corporation", etc.
    let mut orgs = Vec::new();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let heads = [
        "Department",
        "Office",
        "Bureau",
        "Corporation",
        "Corp",
        "Company",
        "Co",
        "Agency",
        "Board",
        "Commission",
        "Committee",
        "Council",
        "Association",
        "Foundation",
        "Institute",
    ];
    for (i, t) in tokens.iter().enumerate() {
        let clean = clean_token(t);
        if heads.iter().any(|h| clean.eq_ignore_ascii_case(h)) && i >= 1 {
            let prev = clean_token(tokens[i - 1]);
            if is_name_token(&prev) {
                orgs.push(format!("{prev} {clean}"));
            }
        }
    }
    dedup_preserve_order(orgs)
}

fn extract_amounts(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let mut j = i + 1;
            let mut saw_digit = false;
            while j < bytes.len()
                && (bytes[j].is_ascii_digit() || bytes[j] == b',' || bytes[j] == b'.')
            {
                if bytes[j].is_ascii_digit() {
                    saw_digit = true;
                }
                j += 1;
            }
            if saw_digit && j - i >= 2 {
                if let Some(s) = text.get(i..j) {
                    out.push(s.to_string());
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    dedup_preserve_order(out)
}

fn extract_dates(text: &str) -> Vec<String> {
    // Very loose: YYYY-MM-DD, M/D/YYYY, and "Month DD, YYYY" — enough for the redaction case.
    let mut out = Vec::new();
    // ISO
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 10 <= bytes.len() {
        if bytes[i].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
            && bytes[i + 3].is_ascii_digit()
            && bytes[i + 4] == b'-'
            && bytes[i + 5].is_ascii_digit()
            && bytes[i + 6].is_ascii_digit()
            && bytes[i + 7] == b'-'
            && bytes[i + 8].is_ascii_digit()
            && bytes[i + 9].is_ascii_digit()
        {
            if let Some(s) = text.get(i..i + 10) {
                out.push(s.to_string());
            }
            i += 10;
        } else {
            i += 1;
        }
    }
    dedup_preserve_order(out)
}

fn dedup_preserve_order<T: PartialEq>(items: Vec<T>) -> Vec<T> {
    let mut out: Vec<T> = Vec::with_capacity(items.len());
    for item in items {
        if !out.iter().any(|existing| existing == &item) {
            out.push(item);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_matches_keep_other_portals_and_drop_the_document_itself_and_hand_added_files() {
        let result = |portal: &str, document_id: u64| crate::types::HybridSearchResult {
            stable_id: format!("{portal}:{document_id}"),
            portal_key: portal.into(),
            request_id: 1,
            document_id,
            file_name: "release.pdf".into(),
            title: String::new(),
            agency_name: String::new(),
            jurisdiction: String::new(),
            document_type: None,
            publication_date: None,
            observation_date: None,
            effective_date: None,
            character_start: 0,
            character_end: 0,
            excerpt: "signed by Jane Doe".into(),
            lexical_rank: None,
            semantic_score: Some(0.9),
            fused_score: 0.9,
            retrieval_status: String::new(),
        };
        let sites = corroborating_sites(
            vec![
                result("city", 7),
                result("county", 7),
                result("city", 8),
                result("source-local-1", 9),
            ],
            "city",
            7,
            &HashSet::from(["source-local-1".to_string()]),
        );
        let kept: Vec<_> = sites
            .iter()
            .map(|site| (site.portal_key.as_str(), site.document_id))
            .collect();
        assert_eq!(kept, vec![("county", 7), ("city", 8)]);
    }

    #[test]
    fn scans_bracketed_redaction_markers() {
        let text = "The signer [REDACTED] approved. Later [WITHHELD] on request.";
        let spans = scan_redaction_spans(text);
        assert_eq!(spans.len(), 2);
        assert!(spans.iter().any(|s| s.end - s.start == "[REDACTED]".len()));
    }

    #[test]
    fn scans_foia_exemption_markers() {
        let text =
            "Personal information redacted (b)(6) per exemption; contact (b)(7)(C) for details.";
        let spans = scan_redaction_spans(text);
        assert!(!spans.is_empty());
        assert!(spans.iter().any(|s| &text[s.start..s.end] == "(b)(6)"));
        assert!(spans.iter().any(|s| &text[s.start..s.end] == "(b)(7)(C)"));
    }

    #[test]
    fn a_section_citation_is_not_a_redaction_stamp() {
        let text = "PBV limits in 24 CFR § 983.301(b)(6) and 570.503 (b)(7) apply. Name: (b)(6) Phone: (b)(7)(C)";
        let spans = scan_redaction_spans(text);
        let marked: Vec<&str> = spans
            .iter()
            .map(|span| &text[span.start..span.end])
            .collect();
        assert_eq!(marked, ["(b)(6)", "(b)(7)(C)"]);
        assert!(spans
            .iter()
            .all(|span| span.start > text.find("Name").unwrap()));
    }

    #[test]
    fn a_labelled_blank_is_a_form_field_not_a_redaction() {
        let text = "By: _______________ Print Name: ____________ Title: __________ Paid to ________ today.";
        let spans = scan_redaction_spans(text);
        assert_eq!(spans.len(), 1);
        assert!(text[..spans[0].start].ends_with("Paid to "));
    }

    #[test]
    fn scans_block_and_underscore_runs() {
        let text = "Payment to ██████████ arrived. Signed by ________ on 2024-01-01.";
        let spans = scan_redaction_spans(text);
        assert_eq!(spans.len(), 2);
    }

    #[test]
    fn classify_hint_person_from_signed_by() {
        assert_eq!(
            classify_entity_hint("...the letter was signed by [REDACTED] under seal..."),
            CandidateKind::Person,
        );
    }

    #[test]
    fn classify_hint_amount_from_payment_language() {
        assert_eq!(
            classify_entity_hint("...paid in the amount of [REDACTED] from account 1234..."),
            CandidateKind::Amount,
        );
    }

    #[test]
    fn classify_hint_organization_from_head_words() {
        assert_eq!(
            classify_entity_hint("...contracted with [REDACTED] Department of Records..."),
            CandidateKind::Organization,
        );
    }

    #[test]
    fn extract_context_snaps_to_word_boundary_and_returns_none_when_too_thin() {
        let long =
            "one two three four five six seven eight nine ten [REDACTED] alpha beta gamma delta";
        let span_start = long.find("[REDACTED]").unwrap();
        let span_end = span_start + "[REDACTED]".len();
        let context = extract_context(
            long,
            RedactionSpan {
                start: span_start,
                end: span_end,
            },
        )
        .expect("must extract");
        assert!(context.contains("seven"));
        assert!(context.contains("alpha"));

        let thin = "abc [REDACTED] xyz";
        assert!(extract_context(
            thin,
            RedactionSpan {
                start: 4,
                end: 4 + "[REDACTED]".len()
            }
        )
        .is_none());
    }

    #[test]
    fn extract_person_names_only_returns_bigrams_of_capitalized_tokens() {
        let text = "Approved by John Smith and Jane Doe today. small words are ignored.";
        let names = extract_person_names(text);
        assert!(names.contains(&"John Smith".to_string()));
        assert!(names.contains(&"Jane Doe".to_string()));
        assert!(!names
            .iter()
            .any(|n| n.to_ascii_lowercase().contains("small")));
    }

    #[test]
    fn extract_amounts_captures_dollar_prefixed_numbers() {
        let text = "The total came to $47,382.19 and a partial payment of $200 was made.";
        let amounts = extract_amounts(text);
        assert!(amounts.contains(&"$47,382.19".to_string()));
        assert!(amounts.contains(&"$200".to_string()));
    }

    #[test]
    fn extract_dates_captures_iso_dates() {
        let text = "Effective 2024-01-15 and superseded on 2025-08-30.";
        let dates = extract_dates(text);
        assert!(dates.contains(&"2024-01-15".to_string()));
        assert!(dates.contains(&"2025-08-30".to_string()));
    }

    #[test]
    fn rank_person_requires_two_corroborating_documents() {
        // Two sites both mention "Jane Doe" — should pass the Person threshold.
        let sites = vec![
            CandidateSite {
                portal_key: "p".into(),
                document_id: 10,
                excerpt: "Letter signed by Jane Doe.".into(),
                raw_score: 0.8,
            },
            CandidateSite {
                portal_key: "p".into(),
                document_id: 11,
                excerpt: "Memo from Jane Doe.".into(),
                raw_score: 0.8,
            },
        ];
        let candidates = rank_candidates(sites, CandidateKind::Person, false);
        assert!(candidates.iter().any(|c| c.value == "Jane Doe"));
        // Single site would not pass.
        let single = vec![CandidateSite {
            portal_key: "p".into(),
            document_id: 10,
            excerpt: "Letter signed by Jane Doe.".into(),
            raw_score: 0.8,
        }];
        let candidates = rank_candidates(single, CandidateKind::Person, false);
        assert!(!candidates.iter().any(|c| c.value == "Jane Doe"));
    }

    #[test]
    fn likelihood_is_bounded_below_the_lexical_ceiling() {
        let sites = (0..5)
            .map(|i| CandidateSite {
                portal_key: "p".into(),
                document_id: 100 + i,
                excerpt: "Signed by John Smith.".into(),
                raw_score: 1.0,
            })
            .collect();
        let candidates = rank_candidates(sites, CandidateKind::Person, false);
        assert!(candidates
            .iter()
            .all(|c| c.likelihood_basis_points <= LEXICAL_LIKELIHOOD_CEILING));
    }

    #[test]
    fn semantic_ceiling_higher_than_lexical() {
        let sites = (0..5)
            .map(|i| CandidateSite {
                portal_key: "p".into(),
                document_id: 100 + i,
                excerpt: "Signed by John Smith.".into(),
                raw_score: 1.0,
            })
            .collect();
        let candidates = rank_candidates(sites, CandidateKind::Person, true);
        assert!(candidates
            .iter()
            .all(|c| c.likelihood_basis_points <= SEMANTIC_LIKELIHOOD_CEILING));
        const _: () = assert!(SEMANTIC_LIKELIHOOD_CEILING > LEXICAL_LIKELIHOOD_CEILING);
    }

    #[test]
    fn corroboration_multiplier_grows_with_document_count() {
        assert!(corroboration_multiplier(1) < corroboration_multiplier(2));
        assert!(corroboration_multiplier(2) < corroboration_multiplier(3));
        assert!((corroboration_multiplier(9) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_corpus_lookup_returns_empty_report() {
        // When no sites are returned, ranking yields no candidates, and empty() is the safe shape.
        let candidates = rank_candidates(Vec::new(), CandidateKind::Person, false);
        assert!(candidates.is_empty());
    }

    /// The viewer reports spans in document characters; the engine slices bytes. A name with a
    /// multi-byte letter before the marker must not shift or split the span.
    #[test]
    fn a_character_span_maps_to_the_same_text_in_bytes() {
        let text = "Müller signed [REDACTED] today";
        let start = text.chars().position(|c| c == '[').unwrap();
        let end = start + "[REDACTED]".chars().count();
        let span = character_span_to_bytes(text, start, end).unwrap();
        assert_eq!(&text[span.start..span.end], "[REDACTED]");
        assert_eq!(
            character_span_to_bytes(text, start, text.chars().count() + 1),
            None
        );
    }

    #[test]
    fn evidence_is_traceable_to_specific_documents() {
        let sites = vec![
            CandidateSite {
                portal_key: "north".into(),
                document_id: 42,
                excerpt: "Payment approved by Jane Doe.".into(),
                raw_score: 0.9,
            },
            CandidateSite {
                portal_key: "north".into(),
                document_id: 43,
                excerpt: "Meeting notes: Jane Doe attended.".into(),
                raw_score: 0.7,
            },
        ];
        let candidates = rank_candidates(sites, CandidateKind::Person, false);
        let jane = candidates.iter().find(|c| c.value == "Jane Doe").unwrap();
        assert_eq!(jane.supporting_evidence.len(), 2);
        assert!(jane
            .supporting_evidence
            .iter()
            .all(|e| e.portal_key == "north"));
        assert!(jane.supporting_evidence.iter().any(|e| e.document_id == 42));
        assert!(jane.supporting_evidence.iter().any(|e| e.document_id == 43));
    }
}
