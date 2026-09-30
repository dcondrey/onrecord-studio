//! Corpus-wide extraction of comparable factual assertions and denial fingerprints.
//!
//! A detector looking for contradictions between two documents needs a small, disciplined
//! representation of "what does this text claim." An [`Assertion`] is a
//! `(subject, predicate, value)` triple tied to a character range so any downstream finding can
//! cite the exact source. The extractors err on the side of NOT emitting: an assertion without a
//! clearly-resolved subject is silently dropped rather than published under a made-up referent.
//! Per this project's rule that a claim of unknown truth must not read as a claim, an under-
//! constrained match is treated as no match at all.
//!
//! [`extract_denial`] is a parallel primitive that picks up "no responsive records" / "no records
//! exist" / adjacent boilerplate from a response letter, plus a narrow-scope flag that tells the
//! `no_records_contradicted` detector to hold fire when the denial only asserts absence within a
//! specified year or period.

use std::sync::LazyLock;

use regex::Regex;

// ----- Assertions ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Assertion {
    pub subject: String,
    pub predicate: AssertionPredicate,
    pub value: AssertionValue,
    /// Byte offsets into the source text (regex match bounds); use for slicing the text.
    pub byte_start: usize,
    pub byte_end: usize,
    /// Character offsets, which is what a citation and the viewer's `characterStart` count in.
    /// Bytes and characters diverge on any non-ASCII text, so a byte offset cited as a character
    /// offset highlights the wrong span.
    pub character_start: usize,
    pub character_end: usize,
}

fn character_offset(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].chars().count()
}

/// The classes of assertion we extract. Named so a contradiction between two rows can be reported
/// in the user-facing language of the class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssertionPredicate {
    /// "Records retained for N days" — subject = record class, value = duration in days.
    RetentionPeriod,
    /// "The XYZ contract took effect on DATE" — subject = referent, value = date.
    EffectiveDate,
    /// "Contract #ABC for $X" — subject = "contract_ABC", value = cents.
    Amount,
    /// "$X per page" — subject = "fee_per_page", value = cents.
    Fee,
    /// "Response will be provided within N days" — subject = "response_time_commitment", value =
    /// duration in days. Cross-document contradictions here surface the same commitment being
    /// stated differently by the same organization.
    ResponseTimeCommitment,
}

impl AssertionPredicate {
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::RetentionPeriod => "retention period",
            Self::EffectiveDate => "effective date",
            Self::Amount => "stated amount",
            Self::Fee => "fee",
            Self::ResponseTimeCommitment => "response-time commitment",
        }
    }
}

/// Values are stored in canonical form so two documents with the same underlying fact produce
/// equal [`AssertionValue`]s — dates as ISO strings, amounts as integer cents, durations as
/// integer days. Comparison uses [`AssertionValue::materially_differs`] which absorbs small
/// bookkeeping noise (rounding pennies, off-by-one days).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AssertionValue {
    Date(String),
    Cents(i64),
    Days(u32),
}

impl AssertionValue {
    pub fn display(&self) -> String {
        match self {
            Self::Date(text) => text.clone(),
            Self::Cents(cents) => {
                let sign = if *cents < 0 { "-" } else { "" };
                let magnitude = cents.unsigned_abs();
                format!("{sign}${}.{:02}", magnitude / 100, magnitude % 100)
            }
            Self::Days(days) => format!("{days} day{}", if *days == 1 { "" } else { "s" }),
        }
    }

    /// Whether two values under the same predicate differ materially. Not raw `!=` because dates
    /// have already been canonicalized (a spurious inequality would be a bug in canonicalization,
    /// not a real contradiction), and amounts and durations tolerate one unit of accounting
    /// noise so the detector doesn't fire on "$1,000 vs $1,000.05".
    pub fn materially_differs(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Date(a), Self::Date(b)) => a != b,
            (Self::Cents(a), Self::Cents(b)) => (a - b).abs() > 100,
            (Self::Days(a), Self::Days(b)) => (*a as i64 - *b as i64).abs() > 1,
            _ => false,
        }
    }
}

static RETENTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    // "<SUBJECT> (are|will be)? (retained|kept|preserved|maintained) for N (days|weeks|months|years)"
    // Subject bound to a capitalised phrase of 3..40 chars so the match anchors on a nameable class.
    Regex::new(
        r"(?i)([A-Z][A-Za-z][A-Za-z0-9 /'&-]{2,40})\s+(?:are\s+|will\s+be\s+|shall\s+be\s+|is\s+|must\s+be\s+)?(?:retained|kept|preserved|maintained|held)\s+for\s+(?:a\s+(?:period\s+of\s+)?)?(\d{1,4})\s+(day|days|week|weeks|month|months|year|years)"
    ).expect("retention regex")
});

static RESPONSE_TIME_RE: LazyLock<Regex> = LazyLock::new(|| {
    // "response (will be) (provided|issued) within N business|calendar days|weeks"
    Regex::new(
        r"(?i)(?:respon(?:d|se)|reply|determination)\s+(?:will\s+be\s+(?:provided|made|issued|sent)\s+|is\s+(?:provided\s+|issued\s+|made\s+)?)?within\s+(\d{1,3})\s+(business\s+day|business\s+days|calendar\s+day|calendar\s+days|day|days|week|weeks)"
    ).expect("response time regex")
});

static FEE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // "(fee|charge|cost) (of)? $X per (page|copy|hour|record|document)"
    Regex::new(
        r"(?i)(?:fee|charge|cost)\s+(?:of\s+)?\$?(\d+(?:\.\d{2})?)\s+per\s+(page|copy|hour|record|document)"
    ).expect("fee regex")
});

static EFFECTIVE_DATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // "The <REFERENT> (contract|agreement|policy|MOU|resolution) (took|is|was) effective ... <DATE>"
    Regex::new(
        r"(?i)([A-Z][A-Za-z0-9 &/'-]{2,50}?)\s+(contract|agreement|policy|memorandum\s+of\s+understanding|mou|resolution)(?:\s+(?:took|takes|will\s+take)\s+effect|\s+(?:was|is|became)\s+effective|\s+effective(?:\s+date)?)\s+(?:on\s+)?(\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}/\d{4}|(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{1,2},?\s+\d{4})"
    ).expect("effective date regex")
});

static AMOUNT_RE: LazyLock<Regex> = LazyLock::new(|| {
    // "contract|PO|invoice|grant|award (#|number)? ID for|of|amount $N,NNN(.NN)?"
    Regex::new(
        r"(?i)(contract|purchase\s+order|po|invoice|grant|award)\s*(?:number\s+|no\.?\s+|#)?([A-Z0-9][A-Z0-9-]{1,25})\s+(?:for\s+|of\s+|amount\s+(?:of\s+)?|in\s+the\s+amount\s+of\s+)?\$([\d,]+(?:\.\d{2})?)"
    ).expect("amount regex")
});

/// Extract every clearly-anchored assertion from `text`. Order is stable given the same input so
/// downstream deduplication has a deterministic base. Unresolvable subjects and unparseable
/// values are silently dropped: a claim you can't cite is not a claim.
pub fn extract_assertions(text: &str) -> Vec<Assertion> {
    let mut out = Vec::new();

    for captured in RETENTION_RE.captures_iter(text) {
        let (Some(subject_match), Some(count_match), Some(unit_match), Some(full)) = (
            captured.get(1),
            captured.get(2),
            captured.get(3),
            captured.get(0),
        ) else {
            continue;
        };
        let subject = canonical_subject(subject_match.as_str());
        if subject.len() < 3 {
            continue;
        }
        let Ok(count) = count_match.as_str().parse::<u32>() else {
            continue;
        };
        if count == 0 {
            continue;
        }
        let unit = unit_match.as_str().to_lowercase();
        let days = match unit.as_str() {
            "day" | "days" => count,
            "week" | "weeks" => count.saturating_mul(7),
            "month" | "months" => count.saturating_mul(30),
            "year" | "years" => count.saturating_mul(365),
            _ => continue,
        };
        out.push(Assertion {
            subject,
            predicate: AssertionPredicate::RetentionPeriod,
            value: AssertionValue::Days(days),
            byte_start: full.start(),
            byte_end: full.end(),
            character_start: character_offset(text, full.start()),
            character_end: character_offset(text, full.end()),
        });
    }

    for captured in RESPONSE_TIME_RE.captures_iter(text) {
        let (Some(count_match), Some(unit_match), Some(full)) =
            (captured.get(1), captured.get(2), captured.get(0))
        else {
            continue;
        };
        let Ok(count) = count_match.as_str().parse::<u32>() else {
            continue;
        };
        if count == 0 {
            continue;
        }
        let unit = unit_match.as_str().to_lowercase();
        let days = if unit.contains("week") {
            count.saturating_mul(7)
        } else {
            count
        };
        out.push(Assertion {
            subject: "response_time_commitment".to_string(),
            predicate: AssertionPredicate::ResponseTimeCommitment,
            value: AssertionValue::Days(days),
            byte_start: full.start(),
            byte_end: full.end(),
            character_start: character_offset(text, full.start()),
            character_end: character_offset(text, full.end()),
        });
    }

    for captured in FEE_RE.captures_iter(text) {
        let (Some(dollars_match), Some(unit_match), Some(full)) =
            (captured.get(1), captured.get(2), captured.get(0))
        else {
            continue;
        };
        let Ok(dollars) = dollars_match.as_str().parse::<f64>() else {
            continue;
        };
        if !dollars.is_finite() || dollars < 0.0 {
            continue;
        }
        let cents = (dollars * 100.0).round() as i64;
        let category = unit_match.as_str().to_lowercase();
        out.push(Assertion {
            subject: format!("fee_per_{category}"),
            predicate: AssertionPredicate::Fee,
            value: AssertionValue::Cents(cents),
            byte_start: full.start(),
            byte_end: full.end(),
            character_start: character_offset(text, full.start()),
            character_end: character_offset(text, full.end()),
        });
    }

    for captured in EFFECTIVE_DATE_RE.captures_iter(text) {
        let (Some(referent_match), Some(kind_match), Some(date_match), Some(full)) = (
            captured.get(1),
            captured.get(2),
            captured.get(3),
            captured.get(0),
        ) else {
            continue;
        };
        let referent = canonical_subject(referent_match.as_str());
        if referent.len() < 3 {
            continue;
        }
        let kind = canonical_subject(kind_match.as_str());
        let Some(iso) = canonicalize_date(date_match.as_str()) else {
            continue;
        };
        out.push(Assertion {
            subject: format!("{kind}::{referent}"),
            predicate: AssertionPredicate::EffectiveDate,
            value: AssertionValue::Date(iso),
            byte_start: full.start(),
            byte_end: full.end(),
            character_start: character_offset(text, full.start()),
            character_end: character_offset(text, full.end()),
        });
    }

    for captured in AMOUNT_RE.captures_iter(text) {
        let (Some(kind_match), Some(id_match), Some(amount_match), Some(full)) = (
            captured.get(1),
            captured.get(2),
            captured.get(3),
            captured.get(0),
        ) else {
            continue;
        };
        let kind = canonical_subject(kind_match.as_str());
        let id = id_match.as_str().trim();
        if id.len() < 2 {
            continue;
        }
        let amount_text = amount_match.as_str().replace(',', "");
        let Ok(dollars) = amount_text.parse::<f64>() else {
            continue;
        };
        if !dollars.is_finite() || dollars <= 0.0 {
            continue;
        }
        let cents = (dollars * 100.0).round() as i64;
        out.push(Assertion {
            subject: format!("{kind}::{}", id.to_lowercase()),
            predicate: AssertionPredicate::Amount,
            value: AssertionValue::Cents(cents),
            byte_start: full.start(),
            byte_end: full.end(),
            character_start: character_offset(text, full.start()),
            character_end: character_offset(text, full.end()),
        });
    }

    out
}

fn canonical_subject(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn canonicalize_date(raw: &str) -> Option<String> {
    let trimmed = raw.trim_end_matches(',');
    if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Some(date.format("%Y-%m-%d").to_string());
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%m/%d/%Y") {
        return Some(date.format("%Y-%m-%d").to_string());
    }
    for fmt in ["%B %d, %Y", "%B %d %Y"] {
        if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, fmt) {
            return Some(date.format("%Y-%m-%d").to_string());
        }
    }
    None
}

// ----- Denial fingerprint --------------------------------------------------------------------

/// The signal `no_records_contradicted` looks for on a response letter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenialFingerprint {
    /// The matched denial phrase, verbatim.
    pub matched_phrase: String,
    /// Start and end of the matched phrase in the source text.
    pub position: (usize, usize),
    /// True when the denial's surrounding language limits it to a specific period or year (e.g.
    /// "for FY 2024", "prior to 2020"). Callers should hold fire on scope-narrow denials because
    /// cross-corpus matches from outside that scope aren't contradictions.
    pub scope_narrow: bool,
}

static DENIAL_PHRASE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\bno\s+(?:responsive\s+records|records\s+(?:exist|were\s+found|are\s+available|were\s+located|were\s+identified|responsive))\b|\bunable\s+to\s+locate\s+(?:any\s+)?records\b|\bcannot\s+locate\s+(?:any\s+)?records\b"
    ).expect("denial phrase regex")
});

static SCOPE_NARROW_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:for|in|during|between|prior\s+to|before|after|since|as\s+of)\s+(?:the\s+)?(?:FY\s*|fiscal\s+year\s*)?\d{4}\b"
    ).expect("scope narrow regex")
});

/// Look for a denial phrase in `text`, plus a narrow-scope flag. Only the first hit is returned;
/// a single response letter typically only makes one such claim. Scope-narrow is determined by
/// looking within ±120 chars of the denial for a year-bounded phrase — a conservative window that
/// catches "no records exist for FY 2024" without picking up unrelated dates elsewhere in the
/// letter.
pub fn extract_denial(text: &str) -> Option<DenialFingerprint> {
    let matched = DENIAL_PHRASE_RE.find(text)?;
    let start = matched.start();
    let end = matched.end();
    let window_start = start.saturating_sub(120);
    let window_end = usize::min(end + 120, text.len());
    // Guard against slicing inside a multi-byte codepoint: adjust ends to codepoint boundaries.
    let window = &text[byte_floor_char(text, window_start)..byte_ceil_char(text, window_end)];
    let scope_narrow = SCOPE_NARROW_RE.is_match(window);
    Some(DenialFingerprint {
        matched_phrase: matched.as_str().to_string(),
        position: (start, end),
        scope_narrow,
    })
}

fn byte_floor_char(text: &str, index: usize) -> usize {
    let mut cursor = usize::min(index, text.len());
    while cursor > 0 && !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

fn byte_ceil_char(text: &str, index: usize) -> usize {
    let mut cursor = usize::min(index, text.len());
    while cursor < text.len() && !text.is_char_boundary(cursor) {
        cursor += 1;
    }
    cursor
}

/// Coarse topic-token extraction from a request title/description. Used by
/// `no_records_contradicted` to build an FTS query for content that would contradict a denial.
/// Everything lowercase, stopwords stripped, tokens of length < 3 dropped, capped at 8 terms.
pub fn extract_topic_tokens(title: &str, description: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut ordered = Vec::new();
    for source in [title, description] {
        for raw in source.split(|c: char| !c.is_alphanumeric()) {
            let normalized = raw.trim().to_lowercase();
            if normalized.len() < 3 {
                continue;
            }
            if STOPWORDS.contains(&normalized.as_str()) {
                continue;
            }
            if seen.insert(normalized.clone()) {
                ordered.push(normalized);
                if ordered.len() >= 8 {
                    return ordered;
                }
            }
        }
    }
    ordered
}

const STOPWORDS: &[&str] = &[
    "the",
    "and",
    "for",
    "any",
    "all",
    "with",
    "from",
    "this",
    "that",
    "have",
    "has",
    "was",
    "were",
    "are",
    "not",
    "but",
    "into",
    "onto",
    "about",
    "please",
    "provide",
    "records",
    "record",
    "documents",
    "document",
    "related",
    "regarding",
    "concerning",
    "information",
    "requests",
    "request",
];

// ----- Tests ---------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_retention_period_in_days_years_and_months() {
        let text = "Personnel records will be retained for 3 years. Payroll records are retained for 7 years. Tax records are kept for a period of 84 months.";
        let assertions = extract_assertions(text);
        let retention: Vec<_> = assertions
            .iter()
            .filter(|a| a.predicate == AssertionPredicate::RetentionPeriod)
            .collect();
        assert!(
            retention
                .iter()
                .any(|a| a.subject.contains("personnel") && a.value == AssertionValue::Days(1095)),
            "expected personnel/1095 among {:?}",
            retention
        );
        assert!(retention
            .iter()
            .any(|a| a.subject.contains("payroll") && a.value == AssertionValue::Days(2555)));
        assert!(retention
            .iter()
            .any(|a| a.subject.contains("tax") && a.value == AssertionValue::Days(2520)));
    }

    #[test]
    fn extracts_response_time_commitment() {
        let text = "The City will respond within 10 business days of receipt.";
        let assertions = extract_assertions(text);
        assert_eq!(assertions.len(), 1);
        assert_eq!(
            assertions[0].predicate,
            AssertionPredicate::ResponseTimeCommitment
        );
        assert_eq!(assertions[0].value, AssertionValue::Days(10));
    }

    #[test]
    fn extracts_fee_amount_normalising_to_cents() {
        let text = "Copy fee of $0.10 per page and a cost of $25 per hour for research.";
        let assertions = extract_assertions(text);
        let fees: Vec<_> = assertions
            .iter()
            .filter(|a| a.predicate == AssertionPredicate::Fee)
            .collect();
        assert!(fees
            .iter()
            .any(|a| a.subject == "fee_per_page" && a.value == AssertionValue::Cents(10)));
        assert!(fees
            .iter()
            .any(|a| a.subject == "fee_per_hour" && a.value == AssertionValue::Cents(2500)));
    }

    #[test]
    fn extracts_effective_date_bound_to_a_named_referent() {
        let text = "The Downtown Housing Contract took effect on March 15, 2024.";
        let assertions = extract_assertions(text);
        let effective: Vec<_> = assertions
            .iter()
            .filter(|a| a.predicate == AssertionPredicate::EffectiveDate)
            .collect();
        assert_eq!(effective.len(), 1);
        assert!(effective[0].subject.contains("downtown housing"));
        assert_eq!(
            effective[0].value,
            AssertionValue::Date("2024-03-15".to_string())
        );
    }

    #[test]
    fn extracts_amount_bound_to_contract_or_po_identifier() {
        let text = "Contract #ABC-2024-19 for $1,500,000. Invoice number INV-7 in the amount of $47,382.19.";
        let assertions = extract_assertions(text);
        let amounts: Vec<_> = assertions
            .iter()
            .filter(|a| a.predicate == AssertionPredicate::Amount)
            .collect();
        assert!(
            amounts.iter().any(|a| a.subject == "contract::abc-2024-19"
                && a.value == AssertionValue::Cents(150_000_000)),
            "missing contract match: {:?}",
            amounts
        );
        assert!(amounts
            .iter()
            .any(|a| a.subject == "invoice::inv-7" && a.value == AssertionValue::Cents(4_738_219)));
    }

    #[test]
    fn does_not_extract_assertions_when_subject_is_unresolvable() {
        // No named contract/policy referent — bare "took effect on DATE" gets no subject anchor.
        let text = "It took effect on March 15, 2024.";
        let assertions = extract_assertions(text);
        assert!(
            !assertions
                .iter()
                .any(|a| a.predicate == AssertionPredicate::EffectiveDate),
            "must not emit an effective-date assertion without a resolved subject: {:?}",
            assertions
        );
    }

    #[test]
    fn materially_differs_absorbs_one_unit_of_bookkeeping_noise() {
        assert!(!AssertionValue::Days(30).materially_differs(&AssertionValue::Days(31)));
        assert!(AssertionValue::Days(30).materially_differs(&AssertionValue::Days(32)));
        assert!(!AssertionValue::Cents(10_000).materially_differs(&AssertionValue::Cents(10_050)));
        assert!(AssertionValue::Cents(10_000).materially_differs(&AssertionValue::Cents(10_101)));
        assert!(AssertionValue::Date("2024-03-15".into())
            .materially_differs(&AssertionValue::Date("2024-03-16".into())));
    }

    #[test]
    fn extract_denial_finds_common_phrasings() {
        assert!(extract_denial("After a thorough search, no responsive records exist.").is_some());
        assert!(extract_denial("The City was unable to locate any records.").is_some());
        assert!(extract_denial("We regret to inform you that no records were found.").is_some());
        assert!(extract_denial("Attached please find the responsive records.").is_none());
    }

    #[test]
    fn extract_denial_flags_scope_narrow_when_year_is_named_nearby() {
        let narrow = extract_denial("For FY 2023, no responsive records exist.").expect("match");
        assert!(narrow.scope_narrow);
        let broad = extract_denial("After a diligent search, no records exist.").expect("match");
        assert!(!broad.scope_narrow);
    }

    #[test]
    fn extract_topic_tokens_drops_stopwords_and_dedupes() {
        let tokens = extract_topic_tokens(
            "Records regarding contracts for the Downtown Redevelopment Project",
            "Please provide all documents related to Contract ABC-2024-19",
        );
        assert!(tokens.contains(&"contracts".to_string()));
        assert!(tokens.contains(&"downtown".to_string()));
        assert!(tokens.contains(&"redevelopment".to_string()));
        assert!(!tokens.iter().any(|t| t == "please" || t == "records"));
        assert!(tokens.len() <= 8);
    }
}

#[cfg(test)]
mod offset_tests {
    use super::extract_assertions;

    #[test]
    fn citations_count_characters_not_bytes() {
        // Two-byte 'é's and a three-byte dash sit before the match, so the byte and character
        // offsets of the same span differ; a citation must use the character ones.
        let text = "Résumé — Personnel records will be retained for 3 years.";
        let assertion = extract_assertions(text)
            .into_iter()
            .next()
            .expect("retention assertion");
        assert!(assertion.character_start < assertion.byte_start);
        assert_eq!(
            assertion.character_start,
            text[..assertion.byte_start].chars().count()
        );
        let by_characters: String = text
            .chars()
            .skip(assertion.character_start)
            .take(assertion.character_end - assertion.character_start)
            .collect();
        assert_eq!(
            by_characters,
            text[assertion.byte_start..assertion.byte_end]
        );
    }
}
