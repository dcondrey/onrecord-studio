//! Cross-record reconstruction from retained text. No model-generated identities or digits.
//! A matching passage is evidence for a hypothesis, never recovered bytes from a redaction.
use crate::{error::AppError, integrity::sha256_hex, storage::pii::mask_pii, types::*};
use chrono::NaiveDate;
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

pub const VERSION: &str = "cross-record-constraints-v1";
pub const MAX_FINDINGS: usize = 48;
static MASK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\[(?:redacted|withheld|removed|name redacted|redaction)[^\]\r\n]{0,30}\]|[█■▇]{2,}|\*{2,}|[xX]{3,}|\?{2,}").unwrap()
});
static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)\b(?P<label>contract|agreement|invoice|purchase order|po|grant|project|solicitation|bid|payment|appropriation|budget|amount|total)(?:\s+(?:number|no\.?|id|amount))?\s*[:#=]\s*(?P<value>\$?[0-9Xx*?█■▇., -]*(?:\[(?:REDACTED|WITHHELD)\][0-9Xx*?█■▇., -]*)?)").unwrap()
});
static ROLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:(?:assistant|deputy|acting|interim|senior)\s+)*(?:chief of police|police chief|chief operating officer|chief financial officer|independent budget analyst|city auditor|city attorney|city manager|mayor|(?:homelessness strategies and solutions|public works|environmental services|parks and recreation|housing and community development|finance|purchasing and contracting) director|director of [a-z][a-z &/\-]{3,75}|[a-z][a-z &/\-]{2,55} department director)\b").unwrap()
});
static NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\p{Lu}][\p{L}'’.-]+(?:\s+(?:[\p{Lu}][\p{L}'’.-]+|[\p{Lu}]\.)){1,4}$").unwrap()
});
static KEYS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)(?:^|[;|])\s*(project|vendor|program|subject|organization|agency|department|reference|date|fiscal year|fiscal_year)\s*:\s*([^\n\r;|]{3,120})").unwrap()
});
static DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}/\d{4}|(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{1,2},?\s+\d{4})\b").unwrap()
});

#[derive(Debug, Clone)]
pub struct ReconstructionDocument {
    pub document_id: u64,
    pub portal_key: String,
    pub title: String,
    pub source_url: Option<String>,
    pub source_sha256: String,
    pub extraction_sha256: String,
    pub text: String,
    pub character_offset: usize,
    pub total_characters: usize,
}

impl ReconstructionDocument {
    fn cite(&self, start: usize, end: usize) -> ReconstructionCitation {
        let quote = &self.text[start..end];
        let offset = self.character_offset + self.text[..start].chars().count();
        ReconstructionCitation {
            id: format!(
                "{}:{}:{offset}:{}",
                self.portal_key,
                self.document_id,
                &sha256_hex(quote.as_bytes())[..12]
            ),
            document_id: self.document_id,
            portal_key: self.portal_key.clone(),
            title: self.title.clone(),
            source_url: self.source_url.clone(),
            source_sha256: self.source_sha256.clone(),
            extraction_sha256: self.extraction_sha256.clone(),
            character_offset: offset,
            quote: mask_pii(quote)
                .map(|v| v.masked_text)
                .unwrap_or_else(|_| "[Preview unavailable]".into()),
        }
    }
}

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
fn window(text: &str, start: usize, end: usize, radius: usize) -> (usize, usize) {
    let mut left = start.saturating_sub(radius);
    let mut right = (end + radius).min(text.len());
    while !text.is_char_boundary(left) {
        left += 1;
    }
    while !text.is_char_boundary(right) {
        right -= 1;
    }
    (left, right)
}
fn line_bounds(text: &str, start: usize, end: usize) -> (usize, usize) {
    (
        text[..start].rfind(['\n', '\r']).map_or(0, |p| p + 1),
        text[end..]
            .find(['\n', '\r'])
            .map_or(text.len(), |p| end + p),
    )
}
fn keys(text: &str) -> BTreeMap<String, String> {
    KEYS.captures_iter(text)
        .map(|c| (normalized(&c[1]), normalized(&c[2])))
        .collect()
}
fn record_link(a: &str, b: &str) -> Option<String> {
    let left = keys(a);
    let right = keys(b);
    if left
        .iter()
        .any(|(key, value)| right.get(key).is_some_and(|other| other != value))
    {
        return None;
    }
    left.iter()
        .find(|(key, value)| {
            ![
                "organization",
                "agency",
                "department",
                "date",
                "fiscal year",
                "fiscal_year",
            ]
            .contains(&key.as_str())
                && value.len() >= 8
                && right.get(*key) == Some(*value)
        })
        .map(|(key, value)| format!("Same {key}: {value}; no conflicting shared record keys."))
}

pub fn search_terms(
    target: &ReconstructionDocument,
    options: &ReconstructionOptions,
) -> Result<Vec<String>, AppError> {
    if options
        .context_quote
        .as_ref()
        .is_some_and(|q| q.trim().len() < 8 || q.len() > 1000 || !target.text.contains(q))
    {
        return Err(AppError::DataParsing("Additional context must be an exact 8–1000 character quotation from this document's retained text.".into()));
    }
    let mut terms = BTreeSet::new();
    for role in ROLE.find_iter(&target.text).take(16) {
        terms.insert(role.as_str().trim().to_string());
    }
    for mask in MASK.find_iter(&target.text).take(MAX_FINDINGS) {
        let (left, right) = window(&target.text, mask.start(), mask.end(), 180);
        for (key, value) in keys(&target.text[left..right]) {
            if key != "organization" {
                terms.insert(value);
            }
        }
        for anchor in anchors(&target.text, mask.start(), mask.end()) {
            if anchor.len() >= 8 {
                terms.insert(anchor);
            }
        }
    }
    if let Some(quote) = &options.context_quote {
        for role in ROLE.find_iter(quote) {
            terms.insert(role.as_str().trim().into());
        }
        for (_, value) in keys(quote) {
            terms.insert(value);
        }
    }
    Ok(terms
        .into_iter()
        .filter(|s| (3..=160).contains(&s.len()))
        .take(24)
        .collect())
}

pub fn reconstruct(
    target: &ReconstructionDocument,
    related: &[ReconstructionDocument],
    options: &ReconstructionOptions,
) -> Result<ReconstructionReport, AppError> {
    let terms = search_terms(target, options)?;
    let documents: Vec<_> = std::iter::once(target).chain(related).collect();
    let mut findings = numeric_findings(target, &documents, options);
    findings.extend(passage_findings(target, &documents));
    findings.extend(role_findings(target, &documents, options));
    let truncated = findings.len() > MAX_FINDINGS;
    findings.truncate(MAX_FINDINGS);
    let identities: BTreeSet<_> = documents
        .iter()
        .map(|d| {
            (
                &d.portal_key,
                d.document_id,
                &d.source_sha256,
                &d.extraction_sha256,
            )
        })
        .collect();
    let fingerprint = sha256_hex(format!("{VERSION}:{identities:?}").as_bytes());
    let mut limitations = vec![
        "These are cross-record inferences, not bytes recovered from a redaction. Agreement does not prove that two records describe the same event or person.".into(),
        "Analysis uses retained extracted text. Image-only masks, missing OCR, undocumented title aliases and unretrieved records can leave gaps. Character locators refer to the extraction; page numbers are not inferred.".into(),
        "Search is bounded to 24 clues, 16 matching records per clue, and six passages within 120,000 characters after the first hit in each record. Later or additional matches may be absent; no match means unresolved. Duplicate releases are not independent corroboration.".into(),
    ];
    if target.text.chars().count() < target.total_characters {
        limitations.push("The target exceeded the one-million-character analysis window; later text was not inspected.".into());
    }
    if options.fixed_width_masks {
        limitations.push("Analyst supplied the assumption that each X, *, ?, or block in a numeric mask represents one character. Bracketed redaction labels always have unknown width.".into());
    }
    if truncated {
        limitations.push(format!("Results were limited to {MAX_FINDINGS} findings."));
    }
    Ok(ReconstructionReport {
        analysis_version: VERSION.into(),
        document_id: target.document_id,
        portal_key: target.portal_key.clone(),
        source_sha256: target.source_sha256.clone(),
        corpus_fingerprint: fingerprint,
        documents_compared: identities.len(),
        passages_compared: documents.len(),
        findings,
        limitations,
        search_terms: terms,
    })
}

fn finding(
    method: &str,
    subject: String,
    observation: ReconstructionCitation,
    mut candidates: Vec<ReconstructionCandidate>,
    explanation: &str,
    next: &str,
) -> ReconstructionFinding {
    // Merge repeated copies without treating their count as independent confirmation.
    let mut merged: Vec<ReconstructionCandidate> = vec![];
    for candidate in candidates.drain(..) {
        if let Some(prior) = merged.iter_mut().find(|p| {
            normalized(&p.value) == normalized(&candidate.value)
                && p.contradicted == candidate.contradicted
        }) {
            for cite in candidate.evidence {
                if !prior.evidence.iter().any(|c| c.id == cite.id) && prior.evidence.len() < 8 {
                    prior.evidence.push(cite);
                }
            }
            for limit in candidate.limitations {
                if !prior.limitations.contains(&limit) {
                    prior.limitations.push(limit);
                }
            }
        } else if merged.len() < 12 {
            merged.push(candidate);
        }
    }
    let accepted: Vec<_> = merged.iter().filter(|c| !c.contradicted).collect();
    let status = match accepted.as_slice() {
        [] => "unresolved",
        [candidate] if candidate.limitations.is_empty() => "corroborated",
        [_] => "candidate",
        _ => "ambiguous",
    };
    let id = sha256_hex(format!("{VERSION}:{method}:{}", observation.id).as_bytes());
    let request_draft = format!("Please provide existing records sufficient to establish {subject}, including the unredacted public-business fields and any applicable version or role-assignment history for the date of the referenced record. Reference: {} ({}), extracted character {}. Please include available native files, headers and attachments that link the versions or identify the responsible public office. If portions remain withheld, please provide the segregable remainder and the stated basis for each withholding. This request does not seek private personal identifiers.", observation.title, observation.source_url.as_deref().unwrap_or("retained source reference"), observation.character_offset);
    ReconstructionFinding {
        id,
        method: method.into(),
        status: status.into(),
        subject,
        observation,
        candidates: merged,
        explanation: explanation.into(),
        next_step: next.into(),
        request_draft,
    }
}

#[derive(Clone)]
struct NumberObservation<'a> {
    document: &'a ReconstructionDocument,
    field: String,
    pattern: String,
    start: usize,
    end: usize,
    context: String,
}
fn numbers(document: &ReconstructionDocument) -> Vec<NumberObservation<'_>> {
    NUMBER
        .captures_iter(&document.text)
        .take(500)
        .filter_map(|capture| {
            let value = capture.name("value")?;
            let pattern = value
                .as_str()
                .trim()
                .trim_end_matches([',', '.'])
                .to_string();
            if pattern.len() < 3
                || pattern.len() > 96
                || !pattern.chars().any(|c| c.is_ascii_digit())
            {
                return None;
            }
            let (left, right) = window(&document.text, value.start(), value.end(), 400);
            // Never reconstruct private account/contact/beneficiary identifiers through this route.
            let context = document.text[left..right].to_string();
            if [
                "social security",
                "bank account",
                "routing number",
                "patient",
                "phone number",
                "credit card",
            ]
            .iter()
            .any(|s| normalized(&context).contains(s))
            {
                return None;
            }
            Some(NumberObservation {
                document,
                field: match normalized(&capture["label"]).as_str() {
                    "po" => "purchase order".into(),
                    other => other.into(),
                },
                pattern,
                start: value.start(),
                end: value.start() + value.as_str().trim_end().len(),
                context,
            })
        })
        .collect()
}
fn numeric_pattern(value: &str, fixed: bool) -> String {
    let mut output = String::new();
    let mut last = 0;
    for hit in MASK.find_iter(value) {
        output.push_str(&regex::escape(
            &value[last..hit.start()].replace([' ', ',', '$'], ""),
        ));
        if fixed && !hit.as_str().starts_with('[') {
            output.push_str(&format!("[0-9]{{{}}}", hit.as_str().chars().count()));
        } else {
            output.push_str("[0-9]{1,32}");
        }
        last = hit.end();
    }
    output.push_str(&regex::escape(&value[last..].replace([' ', ',', '$'], "")));
    format!("^{output}$")
}
fn numeric_findings(
    target: &ReconstructionDocument,
    documents: &[&ReconstructionDocument],
    options: &ReconstructionOptions,
) -> Vec<ReconstructionFinding> {
    let observations: Vec<_> = documents.iter().flat_map(|d| numbers(d)).collect();
    numbers(target).into_iter().filter(|o| MASK.is_match(&o.pattern)).take(24).map(|clue| {
        let constraint = Regex::new(&numeric_pattern(&clue.pattern, options.fixed_width_masks)).unwrap();
        let (left,right) = window(&target.text, clue.start, clue.end, 200);
        let cite = target.cite(left,right);
        let mut candidates = vec![];
        for other in &observations {
            if other.field != clue.field || (other.document.document_id == target.document_id && other.document.portal_key == target.portal_key && other.document.character_offset + other.start == clue.start) { continue; }
            let Some(link) = record_link(&clue.context, &other.context) else { continue; };
            let (a,b) = window(&other.document.text, other.start, other.end, 200);
            let evidence = vec![other.document.cite(a,b)];
            let other_clean = other.pattern.replace([' ', ',', '$'], "");
            if !MASK.is_match(&other.pattern) {
                candidates.push(ReconstructionCandidate { value: other.pattern.clone(), evidence, supports: vec![link, "A related record publishes the complete value.".into()], limitations: vec!["Record linkage uses matching nearby keys; confirm the original records refer to the same transaction/version.".into()], contradicted: !constraint.is_match(&other_clean) });
                continue;
            }
            let other_constraint = Regex::new(&numeric_pattern(&other.pattern, options.fixed_width_masks)).unwrap();
            let mut proposals = BTreeSet::new();
            if options.fixed_width_masks {
                if let Some(merged) = merge_fixed_masks(&clue.pattern, &other.pattern) { proposals.insert(merged); }
            } else {
                let ca = visible_chunks(&clue.pattern); let cb = visible_chunks(&other.pattern);
                for a in &ca { for b in &cb { for (prefix,suffix) in [(a,b),(b,a)] {
                    for overlap in 3..=prefix.len().min(suffix.len()) {
                        if prefix.ends_with(&suffix[..overlap]) { proposals.insert(format!("{prefix}{}", &suffix[overlap..])); }
                    }
                } } }
            }
            for proposal in proposals.into_iter().filter(|v| constraint.is_match(v) && other_constraint.is_match(v)).take(8) {
                candidates.push(ReconstructionCandidate { value: proposal, evidence: evidence.clone(), supports: vec![link.clone(), format!("Every digit is visible in at least one source; both patterns ({}, {}) accept this alignment.", clue.pattern, other.pattern)], limitations: vec![if options.fixed_width_masks { "Alignment depends on the analyst's fixed-width assumption; verify the masking convention and record identity.".into() } else { "Mask widths are unknown. Visible overlap supports this alignment, but longer alignments cannot be excluded.".into() }], contradicted: false });
            }
        }
        finding("complementary_fragments", format!("{} value behind {}", clue.field, clue.pattern), cite, candidates,
            "Compares the same public-business field only when nearby record keys match and do not conflict. Never invents a missing digit. Mask width is unknown unless explicitly supplied by the analyst.",
            "Obtain the native transaction/contract register or another release with the same record keys; verify the number and masking convention before relying on the reconstruction.")
    }).collect()
}
fn visible_chunks(value: &str) -> Vec<String> {
    MASK.split(value)
        .map(|s| s.chars().filter(char::is_ascii_digit).collect::<String>())
        .filter(|s| s.len() >= 3)
        .collect()
}
fn merge_fixed_masks(left: &str, right: &str) -> Option<String> {
    if left.contains('[') || right.contains('[') {
        return None;
    }
    let a: Vec<_> = left
        .chars()
        .filter(|c| ![' ', ',', '$'].contains(c))
        .collect();
    let b: Vec<_> = right
        .chars()
        .filter(|c| ![' ', ',', '$'].contains(c))
        .collect();
    if a.len() != b.len() {
        return None;
    }
    let mut result = String::new();
    for (a, b) in a.into_iter().zip(b) {
        let known_a = a.is_ascii_digit() || a == '.' || a == '-';
        let known_b = b.is_ascii_digit() || b == '.' || b == '-';
        match (known_a, known_b) {
            (true, true) if a != b => return None,
            (true, _) => result.push(a),
            (_, true) => result.push(b),
            _ => return None,
        }
    }
    Some(result)
}

fn anchors(text: &str, start: usize, end: usize) -> Vec<String> {
    let (left, right) = window(text, start, end, 160);
    let before = text[left..start]
        .split_whitespace()
        .rev()
        .take(5)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" ");
    let after = text[end..right]
        .split_whitespace()
        .take(5)
        .collect::<Vec<_>>()
        .join(" ");
    vec![before, after]
}
fn whitespace_pattern(text: &str) -> String {
    text.split_whitespace()
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join(r"\s+")
}
fn passage_findings(
    target: &ReconstructionDocument,
    documents: &[&ReconstructionDocument],
) -> Vec<ReconstructionFinding> {
    let mut findings = vec![];
    for mask in MASK.find_iter(&target.text).take(MAX_FINDINGS) {
        let anchor = anchors(&target.text, mask.start(), mask.end());
        if anchor
            .iter()
            .any(|a| a.split_whitespace().count() < 3 || MASK.is_match(a))
        {
            continue;
        }
        let Ok(pattern) = Regex::new(&format!(
            r"(?i){}\s*(?P<value>[^\r\n]{{1,160}}?)\s*{}",
            whitespace_pattern(&anchor[0]),
            whitespace_pattern(&anchor[1])
        )) else {
            continue;
        };
        let mut candidates = vec![];
        for document in documents {
            for capture in pattern.captures_iter(&document.text).take(12) {
                let value = capture.name("value").unwrap().as_str().trim();
                if MASK.is_match(value) || value.is_empty() {
                    continue;
                }
                let all = capture.get(0).unwrap();
                let (left, right) = window(&document.text, all.start(), all.end(), 0);
                // Personal contact/identity masks are never a target for this public-record tool.
                if mask_pii(value).is_ok_and(|p| !p.findings.is_empty()) {
                    continue;
                }
                candidates.push(ReconstructionCandidate { value: value.into(), evidence: vec![document.cite(left,right)], supports: vec!["Both surrounding passages match across whitespace and case differences.".into()], limitations: vec!["Matching prose can be reused in templates or amended versions; compare dates and record context.".into()], contradicted: false });
            }
        }
        // Keep unresolved spans visible only when they have substantial useful context.
        let (left, right) = window(&target.text, mask.start(), mask.end(), 160);
        findings.push(finding("parallel_passage", format!("missing passage: {}",mask.as_str()),target.cite(left,right),candidates,
            "Locates an unmasked occurrence between the same left and right text anchors, including elsewhere in this document. Conflicting completions remain separate candidates.",
            "Compare the cited versions, dates and surrounding paragraphs; request the corresponding native record or attachment when the match remains ambiguous."));
    }
    findings
}

fn parsed_date(text: &str) -> Option<NaiveDate> {
    DATE.find_iter(text).find_map(|hit| {
        ["%Y-%m-%d", "%m/%d/%Y", "%B %d, %Y", "%B %d %Y"]
            .iter()
            .find_map(|format| NaiveDate::parse_from_str(hit.as_str(), format).ok())
    })
}
fn record_date(text: &str) -> Option<NaiveDate> {
    text.lines()
        .take(100)
        .find(|line| {
            let lower = normalized(line);
            lower.starts_with("date:") || lower.starts_with("sent:") || lower.starts_with("as of:")
        })
        .and_then(parsed_date)
}
fn organization(document: &ReconstructionDocument, text: &str) -> Option<String> {
    let explicit = keys(text);
    explicit
        .get("organization")
        .or_else(|| explicit.get("agency"))
        .cloned()
        .or_else(|| {
            document
                .source_url
                .as_deref()
                .and_then(|s| reqwest::Url::parse(s).ok())
                .and_then(|url| match url.host_str()? {
                    "sandiego.gov" | "www.sandiego.gov" => Some("city of san diego".into()),
                    "sandiegocounty.gov" | "www.sandiegocounty.gov" => {
                        Some("county of san diego".into())
                    }
                    "sdhc.org" | "www.sdhc.org" => Some("san diego housing commission".into()),
                    _ => None,
                })
        })
}
fn candidate_name(
    document: &ReconstructionDocument,
    start: usize,
    end: usize,
) -> Option<(String, usize, usize)> {
    let (left, right) = line_bounds(&document.text, start, end);
    let clean = |value: &str| {
        value
            .trim()
            .trim_matches([',', ':', '-', '–', '—', '|'])
            .trim()
            .to_string()
    };
    let before = clean(&document.text[left..start]);
    let after = clean(&document.text[end..right]);
    for name in [before, after] {
        if NAME.is_match(&name) && !ROLE.is_match(&name) {
            return Some((name, left, right));
        }
    }
    if left > 0 {
        let previous_end = left.saturating_sub(1);
        let previous_start = document.text[..previous_end]
            .rfind('\n')
            .map_or(0, |v| v + 1);
        let name = clean(&document.text[previous_start..previous_end]);
        if NAME.is_match(&name) && !ROLE.is_match(&name) {
            return Some((name, previous_start, right));
        }
    }
    None
}
fn role_findings(
    target: &ReconstructionDocument,
    documents: &[&ReconstructionDocument],
    options: &ReconstructionOptions,
) -> Vec<ReconstructionFinding> {
    let headers = Regex::new(r"(?im)^(?:from|to|sender|recipient)\s*:\s*[^\n]{0,100}").unwrap();
    let masked_headers: Vec<_> = headers
        .find_iter(&target.text)
        .filter(|h| MASK.is_match(h.as_str()))
        .take(8)
        .collect();
    if masked_headers.is_empty() {
        return vec![];
    }
    let mut roles = BTreeMap::new();
    for role in ROLE.find_iter(&target.text).take(32) {
        roles.entry(normalized(role.as_str())).or_insert(role);
    }
    if let Some(quote) = &options.context_quote {
        roles.retain(|_, role| quote.contains(role.as_str()));
    }
    let target_org = organization(target, &target.text);
    let target_date = record_date(&target.text);
    let mut results = vec![];
    for header in &masked_headers {
        for (role_name, role) in &roles {
            let (a, b) = window(&target.text, role.start(), role.end(), 120);
            let role_cite = target.cite(a, b);
            let mut candidates = vec![];
            for document in documents {
                for occurrence in ROLE.find_iter(&document.text) {
                    if normalized(occurrence.as_str()) != *role_name {
                        continue;
                    }
                    let Some((name, start, end)) =
                        candidate_name(document, occurrence.start(), occurrence.end())
                    else {
                        continue;
                    };
                    let (left, right) = window(&document.text, start, end, 240);
                    let context = &document.text[left..right];
                    let org = organization(document, &document.text);
                    if target_org.is_some() && org.is_some() && target_org != org {
                        continue;
                    }
                    let tenure = context
                        .lines()
                        .find(|l| normalized(l).starts_with("tenure:"))
                        .map(|l| {
                            DATE.find_iter(l)
                                .filter_map(|d| parsed_date(d.as_str()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let as_of = record_date(&document.text);
                    let time_supported = target_date.is_some_and(|date| {
                        (tenure.len() == 2 && tenure[0] <= date && date <= tenure[1])
                            || as_of == Some(date)
                    });
                    let contradicted = target_date.is_some_and(|date| {
                        tenure.len() == 2 && (date < tenure[0] || date > tenure[1])
                    });
                    let mut limits = vec!["Holding a title does not prove this person sent or received this particular message; delegated or shared mailboxes remain possible.".into()];
                    if !time_supported {
                        limits.push("A matching role holder at the message date has not been established by a dated roster or explicit tenure interval.".into());
                    }
                    if target_org.is_none() || org.is_none() {
                        limits.push("Organization is missing from at least one source.".into());
                    }
                    if masked_headers.len() > 1 || roles.len() > 1 {
                        limits.push("Multiple masked participants or roles appear; their association is not established.".into());
                    }
                    let mut supports = vec![format!("Exact normalized public role: {role_name}.")];
                    if target_org.is_some() && target_org == org {
                        supports.push(format!(
                            "Same organization: {}.",
                            target_org.as_deref().unwrap()
                        ));
                    }
                    if time_supported {
                        supports.push(format!(
                            "Dated evidence covers message date {}.",
                            target_date.unwrap()
                        ));
                    }
                    if contradicted {
                        limits.push("The explicitly stated tenure excludes the message date; this candidate is contradicted.".into());
                    }
                    candidates.push(ReconstructionCandidate {
                        value: name,
                        evidence: vec![role_cite.clone(), document.cite(left, right)],
                        supports,
                        limitations: limits,
                        contradicted,
                    });
                }
            }
            results.push(finding("public_role_link",format!("{} linked to {role_name}",header.as_str().trim()),target.cite(header.start(),header.end()),candidates,
                "Connects a masked correspondence participant to a public role mentioned in the document, then looks for an explicitly named holder of that exact role. Organization and historical tenure are evaluated separately; a current office holder is not assumed to have held the role earlier.",
                "Obtain the dated public staff directory, appointment/delegation record and native message headers for this message date. These records can distinguish the office holder from an assistant, acting appointee or shared mailbox."));
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc(id: u64, text: &str) -> ReconstructionDocument {
        ReconstructionDocument {
            document_id: id,
            portal_key: "city".into(),
            title: format!("record-{id}"),
            source_url: Some(format!("https://www.sandiego.gov/{id}.pdf")),
            source_sha256: format!("source-{id}"),
            extraction_sha256: sha256_hex(text.as_bytes()),
            text: text.into(),
            character_offset: 0,
            total_characters: text.chars().count(),
        }
    }
    fn number_doc(id: u64, number: &str, project: &str) -> ReconstructionDocument {
        doc(
            id,
            &format!(
                "Project: {project}\nVendor: Harbor Services\nContract: {number}\nDate: 2024-06-01"
            ),
        )
    }
    #[test]
    fn complementary_unknown_width_masks_offer_only_supported_alignments() {
        let target = number_doc(1, "***456789", "Riverbed restoration");
        let other = number_doc(2, "123456***", "Riverbed restoration");
        let report = reconstruct(&target, &[other], &Default::default()).unwrap();
        let finding = report
            .findings
            .iter()
            .find(|f| f.method == "complementary_fragments")
            .unwrap();
        assert_eq!(finding.status, "candidate");
        assert!(finding.candidates.iter().any(|c| c.value == "123456789"
            && !c.contradicted
            && c.limitations
                .iter()
                .any(|l| l.contains("widths are unknown"))));
        assert_eq!(finding.candidates[0].evidence[0].document_id, 2);
    }
    #[test]
    fn disjoint_digits_need_verified_width_and_never_fill_an_unseen_position() {
        let target = number_doc(1, "****6789", "Riverbed restoration");
        let other = number_doc(2, "1234****", "Riverbed restoration");
        let report =
            reconstruct(&target, std::slice::from_ref(&other), &Default::default()).unwrap();
        assert!(report.findings[0].candidates.is_empty());
        let report = reconstruct(
            &target,
            &[other],
            &ReconstructionOptions {
                fixed_width_masks: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.findings[0].candidates[0].value, "12346789");
        assert_eq!(report.findings[0].status, "candidate");
        assert!(merge_fixed_masks("*****789", "123*****").is_none());
        assert!(merge_fixed_masks("1234****", "***56789").is_none());
    }
    #[test]
    fn conflicting_keys_and_visible_digits_do_not_become_confirmations() {
        let target = number_doc(1, "***6789", "Riverbed restoration");
        let unrelated = number_doc(2, "1236789", "Library construction");
        let conflict = number_doc(3, "1239876", "Riverbed restoration");
        let report = reconstruct(&target, &[unrelated, conflict], &Default::default()).unwrap();
        let finding = &report.findings[0];
        assert_eq!(finding.status, "unresolved");
        assert_eq!(finding.candidates.len(), 1);
        assert!(finding.candidates[0].contradicted);
    }
    #[test]
    fn multiple_complete_numbers_remain_ambiguous_and_duplicate_copies_do_not_vote() {
        let target = number_doc(1, "***6789", "Riverbed restoration");
        let report = reconstruct(
            &target,
            &[
                number_doc(2, "1236789", "Riverbed restoration"),
                number_doc(3, "2346789", "Riverbed restoration"),
                number_doc(4, "1236789", "Riverbed restoration"),
            ],
            &Default::default(),
        )
        .unwrap();
        assert_eq!(report.findings[0].status, "ambiguous");
        assert_eq!(report.findings[0].candidates.len(), 2);
    }
    #[test]
    fn role_lookup_checks_historical_tenure_and_keeps_identity_as_inference() {
        let target=doc(1,"From: [REDACTED]\nDate: June 1, 2024\nOrganization: City of San Diego\nThe sender holds the following public role:\nChief of Police\n");
        let historic=doc(2,"Organization: City of San Diego\nTenure: 2022-01-01 to 2024-12-31\nAlex Rivera\nChief of Police\n");
        let current=doc(3,"Organization: City of San Diego\nTenure: 2025-01-01 to 2026-12-31\nCasey Morgan\nChief of Police\n");
        let different = doc(
            4,
            "Organization: City of Example\nAlex Jones\nChief of Police\n",
        );
        let report = reconstruct(
            &target,
            &[historic, current, different],
            &Default::default(),
        )
        .unwrap();
        let finding = report
            .findings
            .iter()
            .find(|f| f.method == "public_role_link")
            .unwrap();
        assert_eq!(finding.status, "candidate");
        assert_eq!(finding.candidates.len(), 2);
        let valid = finding
            .candidates
            .iter()
            .find(|c| c.value == "Alex Rivera")
            .unwrap();
        assert!(!valid.contradicted);
        assert!(valid.supports.iter().any(|s| s.contains("2024-06-01")));
        assert!(
            finding
                .candidates
                .iter()
                .find(|c| c.value == "Casey Morgan")
                .unwrap()
                .contradicted
        );
    }
    #[test]
    fn same_document_and_other_release_passages_are_located_with_unicode_offsets() {
        let prefix = "Résumé: The city approved funding for ";
        let tail = " to operate the public bathrooms during winter.";
        let text = format!("{prefix}[REDACTED]{tail}\n\n{prefix}Harbor Services{tail}");
        let target = doc(1, &text);
        let other = doc(2, &format!("{prefix}Seaside Services{tail}"));
        let report = reconstruct(&target, &[other], &Default::default()).unwrap();
        let finding = report
            .findings
            .iter()
            .find(|f| f.method == "parallel_passage")
            .unwrap();
        assert_eq!(finding.status, "ambiguous");
        assert!(finding
            .candidates
            .iter()
            .any(|c| c.value == "Harbor Services"));
        for candidate in &finding.candidates {
            for cite in &candidate.evidence {
                if cite.document_id == 1 {
                    let quote: String = target
                        .text
                        .chars()
                        .skip(cite.character_offset)
                        .take(cite.quote.chars().count())
                        .collect();
                    assert_eq!(quote, cite.quote);
                }
            }
        }
    }
    #[test]
    fn invented_context_and_private_identifiers_are_not_reconstructed() {
        let target = doc(
            1,
            "Project: Riverbed restoration\nBank account contract: ****6789\n",
        );
        assert!(reconstruct(&target, &[], &Default::default())
            .unwrap()
            .findings
            .is_empty());
        assert!(reconstruct(
            &target,
            &[],
            &ReconstructionOptions {
                context_quote: Some("This quotation does not exist".into()),
                ..Default::default()
            }
        )
        .is_err());
    }
}
