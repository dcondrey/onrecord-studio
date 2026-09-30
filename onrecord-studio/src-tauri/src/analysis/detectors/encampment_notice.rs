//! Encampment abatement notice timing.
//!
//! A released document about an encampment that states when a notice was posted (or issued) and
//! when the cleanup was set, or states the notice period it gave, is compared with a minimum
//! notice period from one of two sources:
//!
//! - a policy document of the same agency, in force on the posting date, that states a
//!   calendar-day notice duty before removal (paired as `policy_violation` pairs a policy with
//!   its agency's requests);
//! - a row already stored in `legal_statutes` whose own title or text is about encampment notice,
//!   whose `mandated_deadline_days` is recorded, whose jurisdiction equals the portal's, and whose
//!   effective window covers the posting date. No minimum is ever supplied by this detector.
//!
//! Evidence rules:
//!
//! - Only calendar-day periods are read, on both sides. A business- or working-day period is
//!   dropped because counting it needs a holiday calendar, and a guess would turn a compliant
//!   notice into a shortfall. A period in hours is out of scope and not read.
//! - A date with no role cue before it is not a posting, notice or cleanup date. A role no date is
//!   given for is not recorded, never zero.
//! - The start is the earliest posting date, or the earliest notice date when no posting date is
//!   recorded; the end is the latest cleanup date. Both choices can only lengthen the interval, so
//!   conflicting dates yield fewer findings, not more. An end before the start is skipped.
//! - `EncampmentNoticeShortfall` needs both a start and an end date recorded: a counted comparison
//!   of two stated dates against a stated minimum. `EncampmentNoticeStatedShortfall` is a notice
//!   whose stated period is below the minimum when the dates are not both recorded: one statement
//!   against another, a pattern signal.

use super::public_documents::{load_public_documents, PublicDocument};
use crate::{
    analysis::{
        assertion_index::{extract_assertions, AssertionPredicate, AssertionValue},
        document_class::{classify, DocumentClass},
        finding::{
            finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
            FindingSubject,
        },
        phrase_match::{has_phrase, words, Phrases, Word},
    },
    error::AppError,
    ingestion::regional::source_jurisdiction,
    storage::{corpus::match_date, DuckDbRepository},
};
use chrono::{DateTime, NaiveDate, Utc};
use duckdb::params;
use std::collections::HashMap;

pub const SLUG: &str = "encampment_notice";

const MAX_DOCUMENTS: i64 = 5_000;
const MAX_REQUESTS: i64 = 50_000;
const MAX_STATUTES: i64 = 1_000;
const MAX_FINDINGS: usize = 100;
const QUOTE_CHARACTERS: usize = 240;
/// Words before a date searched for the cue that says what the date is.
const ROLE_CUE_WINDOW: usize = 6;
const MAX_PERIOD_DAYS: u32 = 365;
/// Reserved for a counted finding whose posting date, cleanup date and duty effective date were
/// all stated.
const FULL_CONFIDENCE: u16 = 10_000;
const STATED_PERIOD_CONFIDENCE: u16 = 5_000;
const NOTICE_DATE_START_PENALTY: u16 = 1_000;
const UNSTATED_EFFECTIVE_DATE_PENALTY: u16 = 1_500;
const DOCUMENT_POSTED_TIMING_PENALTY: u16 = 1_000;

const ENCAMPMENT: Phrases = &[
    &["encampment"],
    &["encampments"],
    &["camp"],
    &["camps"],
    &["camping"],
    &["campsite"],
    &["homeless"],
    &["homelessness"],
    &["unhoused"],
    &["unsheltered"],
    &["tent"],
    &["tents"],
];
const POSTING: Phrases = &[&["posted"], &["posting"]];
const NOTICE: Phrases = &[
    &["notice"],
    &["noticed"],
    &["notified"],
    &["notification"],
    &["notify"],
    &["served"],
];
const ABATEMENT: Phrases = &[
    &["cleanup"],
    &["cleanups"],
    &["clean", "up"],
    &["cleaned"],
    &["cleaning"],
    &["abatement"],
    &["abate"],
    &["abated"],
    &["removal"],
    &["remove"],
    &["removed"],
    &["vacate"],
    &["vacated"],
    &["sweep"],
    &["cleared"],
    &["clearing"],
    &["clearance"],
];
const OBLIGATION: Phrases = &[
    &["shall"],
    &["must"],
    &["will"],
    &["required"],
    &["requires"],
];
const LEAD: Phrases = &[&["before"], &["prior", "to"], &["advance"]];
/// Property storage and retrieval periods ("stored for 90 days") are not notice periods.
const STORAGE: Phrases = &[
    &["store"],
    &["stored"],
    &["storage"],
    &["retain"],
    &["retained"],
    &["held"],
    &["kept"],
    &["claim"],
    &["claimed"],
    &["retrieve"],
    &["retrieved"],
    &["discarded"],
    &["disposed"],
];
const NO_DUTY: Phrases = &[
    &["no", "notice"],
    &["without", "notice"],
    &["not", "required"],
    &["need", "not"],
];
const NUMBER_WORDS: &[(&str, u32)] = &[
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
];
const TENS_WORDS: &[&str] = &[
    "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

type Span = (usize, usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Posting,
    Notice,
    Abatement,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Dated {
    date: NaiveDate,
    span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Start {
    dated: Dated,
    /// False when no posting date is recorded and the notice date stands in for it.
    posted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Period {
    days: u32,
    span: Span,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct NoticeRecord {
    start: Option<Start>,
    end: Option<Dated>,
    stated_period: Option<Period>,
}

/// Byte spans of lines and sentences. A period after a run of three letters or fewer ("Mar.",
/// "St.", "No.") does not end a sentence, so an abbreviated month stays with its date.
fn units(text: &str) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        let ends = match character {
            '\n' | ';' | '!' | '?' => true,
            '.' => {
                let next_is_break = bytes
                    .get(index + 1)
                    .is_none_or(|next| next.is_ascii_whitespace());
                let letters = bytes[..index]
                    .iter()
                    .rev()
                    .take_while(|byte| byte.is_ascii_alphabetic())
                    .count();
                next_is_break && !(1..=3).contains(&letters)
            }
            _ => false,
        };
        if ends {
            let end = index + character.len_utf8();
            if !text[start..end].trim().is_empty() {
                spans.push((start, end));
            }
            start = end;
        }
    }
    if !text[start..].trim().is_empty() {
        spans.push((start, text.len()));
    }
    spans
}

fn dates_in(text: &str, (start, end): Span) -> Vec<(Span, NaiveDate)> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut index = start;
    while index < end {
        let at_boundary = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        if at_boundary {
            if let Some(matched) = match_date(&text[index..end]) {
                let after = index + matched.length;
                let ends_cleanly = after >= end
                    || (!bytes[after].is_ascii_alphanumeric()
                        && !matches!(bytes[after], b'/' | b'-'));
                if ends_cleanly {
                    found.push(((index, after), matched.date));
                    index = after;
                    continue;
                }
            }
        }
        index += text[index..].chars().next().map_or(1, char::len_utf8);
    }
    found
}

fn ends_at(tokens: &[Word], index: usize, phrases: Phrases) -> bool {
    phrases.iter().any(|phrase| {
        index + 1 >= phrase.len()
            && tokens[index + 1 - phrase.len()..=index]
                .iter()
                .zip(phrase.iter())
                .all(|(word, cue)| word.text == *cue)
    })
}

/// The role named by the cue closest before a date, within [`ROLE_CUE_WINDOW`] words.
fn role_before(tokens: &[Word]) -> Option<Role> {
    let window_start = tokens.len().saturating_sub(ROLE_CUE_WINDOW);
    (window_start..tokens.len()).rev().find_map(|index| {
        if ends_at(tokens, index, POSTING) {
            Some(Role::Posting)
        } else if ends_at(tokens, index, ABATEMENT) {
            Some(Role::Abatement)
        } else if ends_at(tokens, index, NOTICE) {
            Some(Role::Notice)
        } else {
            None
        }
    })
}

fn number(tokens: &[Word], index: usize) -> Option<u32> {
    let word = tokens[index].text.as_str();
    if word.bytes().all(|byte| byte.is_ascii_digit()) {
        return word.parse().ok();
    }
    let after_tens = index > 0 && TENS_WORDS.contains(&tokens[index - 1].text.as_str());
    NUMBER_WORDS
        .iter()
        .find(|(name, _)| *name == word && !after_tens)
        .map(|(_, value)| *value)
}

/// Every `N days` or `N calendar days` in the tokens. A business-day, working-day or hour period
/// does not have `day`/`days` directly after the number (or after `calendar`), so it is not read.
fn calendar_periods(tokens: &[Word]) -> Vec<u32> {
    (0..tokens.len())
        .filter_map(|index| {
            let days = number(tokens, index)?;
            let mut unit = index + 1;
            if tokens.get(unit).is_some_and(|word| word.text == "calendar") {
                unit += 1;
            }
            let is_day = tokens
                .get(unit)
                .is_some_and(|word| word.text == "day" || word.text == "days");
            (is_day && (1..=MAX_PERIOD_DAYS).contains(&days)).then_some(days)
        })
        .collect()
}

fn notice_record(text: &str) -> Option<NoticeRecord> {
    let mut postings = Vec::new();
    let mut notices = Vec::new();
    let mut abatements = Vec::new();
    let mut stated = Vec::new();
    for unit in units(text) {
        let mut segment_start = unit.0;
        for (span, date) in dates_in(text, unit) {
            let dated = Dated { date, span: unit };
            match role_before(&words(&text[segment_start..span.0])) {
                Some(Role::Posting) => postings.push(dated),
                Some(Role::Notice) => notices.push(dated),
                Some(Role::Abatement) => abatements.push(dated),
                None => {}
            }
            segment_start = span.1;
        }
        let tokens = words(&text[unit.0..unit.1]);
        if (has_phrase(&tokens, NOTICE) || has_phrase(&tokens, ABATEMENT))
            && !has_phrase(&tokens, STORAGE)
        {
            stated.extend(
                calendar_periods(&tokens)
                    .into_iter()
                    .map(|days| Period { days, span: unit }),
            );
        }
    }
    let earliest = |dates: &[Dated]| dates.iter().min_by_key(|dated| dated.date).copied();
    let start = earliest(&postings)
        .map(|dated| Start {
            dated,
            posted: true,
        })
        .or_else(|| {
            earliest(&notices).map(|dated| Start {
                dated,
                posted: false,
            })
        });
    let end = abatements.iter().max_by_key(|dated| dated.date).copied();
    if let (Some(start), Some(end)) = (start, end) {
        if end.date < start.dated.date {
            return None;
        }
    }
    let record = NoticeRecord {
        start,
        end,
        stated_period: stated.into_iter().max_by_key(|period| period.days),
    };
    (record != NoticeRecord::default()).then_some(record)
}

/// The smallest calendar-day notice duty the policy states: a sentence with an obligation, a
/// notice or posting, a lead cue (`before`, `prior to`, `advance`) and a removal, and exactly one
/// calendar-day period.
fn policy_duty(text: &str) -> Option<Period> {
    units(text)
        .into_iter()
        .filter_map(|unit| {
            let tokens = words(&text[unit.0..unit.1]);
            let is_duty = has_phrase(&tokens, OBLIGATION)
                && (has_phrase(&tokens, NOTICE) || has_phrase(&tokens, POSTING))
                && has_phrase(&tokens, LEAD)
                && has_phrase(&tokens, ABATEMENT)
                && !has_phrase(&tokens, STORAGE)
                && !has_phrase(&tokens, NO_DUTY);
            if !is_duty {
                return None;
            }
            let mut periods = calendar_periods(&tokens);
            periods.dedup();
            match periods.as_slice() {
                [days] => Some(Period {
                    days: *days,
                    span: unit,
                }),
                _ => None,
            }
        })
        .min_by_key(|period| period.days)
}

fn stated_effective_date(text: &str) -> Option<NaiveDate> {
    extract_assertions(text)
        .into_iter()
        .filter(|assertion| {
            assertion.predicate == AssertionPredicate::EffectiveDate
                && assertion.subject.starts_with("policy::")
        })
        .filter_map(|assertion| match assertion.value {
            AssertionValue::Date(iso) => NaiveDate::parse_from_str(&iso, "%Y-%m-%d").ok(),
            _ => None,
        })
        .max()
}

fn posted_date(value: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|time| time.with_timezone(&Utc).date_naive())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Agency {
    id: i64,
    name: String,
}

/// `(portal, request id)` to the agency the request was made to.
type AgencyIndex = HashMap<(String, i64), Agency>;

struct StatuteRow {
    statute_id: String,
    jurisdiction: String,
    citation: String,
    title: String,
    full_text: String,
    mandated_days: Option<i64>,
    effective_from: Option<String>,
    effective_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NoticeStatute {
    statute_id: String,
    jurisdiction: String,
    citation: String,
    title: String,
    days: u32,
    effective_from: Option<NaiveDate>,
    effective_to: Option<NaiveDate>,
}

impl NoticeStatute {
    fn in_force_on(&self, date: NaiveDate) -> bool {
        self.effective_from.is_none_or(|from| from <= date)
            && self.effective_to.is_none_or(|to| date <= to)
    }
}

/// Stored authorities whose own title or text is about encampment notice. `mandated_deadline_days`
/// is read as a minimum notice period only for these; on any other row (the CPRA determination
/// deadline among them) it means something else.
fn notice_statutes(rows: Vec<StatuteRow>) -> Vec<NoticeStatute> {
    rows.into_iter()
        .filter_map(|row| {
            let days = u32::try_from(row.mandated_days?).ok()?;
            if !(1..=MAX_PERIOD_DAYS).contains(&days) {
                return None;
            }
            let tokens = words(&format!("{}\n{}", row.title, row.full_text));
            if !(has_phrase(&tokens, NOTICE) || has_phrase(&tokens, POSTING))
                || !has_phrase(&tokens, ENCAMPMENT)
            {
                return None;
            }
            let bound = |value: Option<String>| -> Option<Option<NaiveDate>> {
                match value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                    None => Some(None),
                    Some(text) => NaiveDate::parse_from_str(text.get(..10)?, "%Y-%m-%d")
                        .ok()
                        .map(Some),
                }
            };
            Some(NoticeStatute {
                statute_id: row.statute_id,
                jurisdiction: row.jurisdiction.trim().to_string(),
                citation: row.citation,
                title: row.title,
                days,
                effective_from: bound(row.effective_from)?,
                effective_to: bound(row.effective_to)?,
            })
        })
        .collect()
}

struct PolicyDuty<'a> {
    document: &'a PublicDocument,
    agency_id: i64,
    effective: NaiveDate,
    effective_stated: bool,
    duty: Period,
}

struct Notice<'a> {
    document: &'a PublicDocument,
    agency: Option<&'a Agency>,
    posted: NaiveDate,
    record: NoticeRecord,
}

struct Sources<'a> {
    policies: Vec<PolicyDuty<'a>>,
    notices: Vec<Notice<'a>>,
    statutes: Vec<NoticeStatute>,
    /// Portal key to the names its jurisdiction is known by.
    jurisdictions: HashMap<String, Vec<String>>,
}

fn sources<'a>(
    documents: &'a [PublicDocument],
    agencies: &'a AgencyIndex,
    statutes: Vec<NoticeStatute>,
    jurisdictions: HashMap<String, Vec<String>>,
) -> Sources<'a> {
    let mut found = Sources {
        policies: Vec::new(),
        notices: Vec::new(),
        statutes,
        jurisdictions,
    };
    for document in documents {
        if !has_phrase(&words(&document.text), ENCAMPMENT) {
            continue;
        }
        let Some(posted) = posted_date(&document.created_at) else {
            continue;
        };
        let agency = agencies.get(&(document.portal_key.clone(), document.request_id));
        if classify(document.file_name.as_deref(), &document.text) == Some(DocumentClass::Policy) {
            let (Some(agency), Some(duty)) = (agency, policy_duty(&document.text)) else {
                continue;
            };
            let stated = stated_effective_date(&document.text);
            found.policies.push(PolicyDuty {
                document,
                agency_id: agency.id,
                effective: stated.unwrap_or(posted),
                effective_stated: stated.is_some(),
                duty,
            });
        } else if let Some(record) = notice_record(&document.text) {
            found.notices.push(Notice {
                document,
                agency,
                posted,
                record,
            });
        }
    }
    found
}

enum Duty<'a> {
    Policy(&'a PolicyDuty<'a>),
    Statute(&'a NoticeStatute),
}

impl Duty<'_> {
    fn days(&self) -> u32 {
        match self {
            Self::Policy(policy) => policy.duty.days,
            Self::Statute(statute) => statute.days,
        }
    }

    fn effective_stated(&self) -> bool {
        match self {
            Self::Policy(policy) => policy.effective_stated,
            Self::Statute(statute) => statute.effective_from.is_some(),
        }
    }

    fn key(&self) -> String {
        match self {
            Self::Policy(policy) => format!(
                "policy:{}:{}",
                policy.document.document_id, policy.duty.span.0
            ),
            Self::Statute(statute) => format!("statute:{}", statute.statute_id),
        }
    }

    fn named(&self) -> String {
        match self {
            Self::Policy(_) => "the agency's own policy".to_string(),
            Self::Statute(statute) => statute.citation.clone(),
        }
    }

    fn description(&self) -> String {
        match self {
            Self::Policy(policy) => format!(
                "'{}' (in force from {}) states: \"{}\".",
                document_name(policy.document),
                policy.effective,
                excerpt(&policy.document.text[policy.duty.span.0..policy.duty.span.1])
            ),
            Self::Statute(statute) => format!(
                "The stored authority {} ('{}', {}) records a minimum notice period of {} calendar days.",
                statute.citation, statute.title, statute.jurisdiction, statute.days
            ),
        }
    }

    fn effective_caveat(&self) -> Option<String> {
        if self.effective_stated() {
            return None;
        }
        Some(match self {
            Self::Policy(policy) => format!(
                "'{}' states no effective date, so the date it was posted ({}) is used.",
                document_name(policy.document),
                policy.effective
            ),
            Self::Statute(statute) => format!(
                "{} has no recorded effective date, so it is assumed to have been in force.",
                statute.citation
            ),
        })
    }
}

/// The duties a notice is measured against: the agency's policy most recently in force on `on`,
/// and every stored notice authority of the portal's jurisdiction in force on `on`.
fn duties_for<'a>(sources: &'a Sources, notice: &Notice, on: NaiveDate) -> Vec<Duty<'a>> {
    let portal = &notice.document.portal_key;
    let mut duties: Vec<Duty> = notice
        .agency
        .and_then(|agency| {
            sources
                .policies
                .iter()
                .filter(|policy| {
                    &policy.document.portal_key == portal
                        && policy.agency_id == agency.id
                        && policy.effective <= on
                })
                .max_by_key(|policy| {
                    (
                        policy.effective,
                        std::cmp::Reverse(policy.document.document_id),
                    )
                })
        })
        .map(Duty::Policy)
        .into_iter()
        .collect();
    let names = sources.jurisdictions.get(portal);
    duties.extend(
        sources
            .statutes
            .iter()
            .filter(|statute| {
                names.is_some_and(|names| {
                    names
                        .iter()
                        .any(|name| name.eq_ignore_ascii_case(&statute.jurisdiction))
                }) && statute.in_force_on(on)
            })
            .map(Duty::Statute),
    );
    duties
}

fn findings(sources: &Sources) -> Vec<Finding> {
    let mut found: Vec<Finding> = sources
        .notices
        .iter()
        .flat_map(|notice| {
            let record = &notice.record;
            let on = record
                .start
                .map(|start| start.dated.date)
                .or(record.end.map(|end| end.date));
            duties_for(sources, notice, on.unwrap_or(notice.posted))
                .into_iter()
                .filter_map(|duty| finding(notice, &duty, on.is_none()))
                .collect::<Vec<_>>()
        })
        .collect();
    found.sort_by(|a, b| {
        b.severity
            .rank_weight()
            .cmp(&a.severity.rank_weight())
            .then_with(|| b.confidence.rank_weight().cmp(&a.confidence.rank_weight()))
            .then_with(|| a.id.cmp(&b.id))
    });
    found.truncate(MAX_FINDINGS);
    found
}

fn finding(notice: &Notice, duty: &Duty, timed_by_upload: bool) -> Option<Finding> {
    let record = &notice.record;
    let required = duty.days();
    let document = notice.document;
    let agency = notice
        .agency
        .map_or("The agency".to_string(), |agency| agency.name.clone());
    let name = document_name(document);
    let mut caveats: Vec<String> = duty.effective_caveat().into_iter().collect();
    let effective_penalty = if duty.effective_stated() {
        0
    } else {
        UNSTATED_EFFECTIVE_DATE_PENALTY
    };
    let mut citations = Vec::new();

    let (kind, severity, basis_points, headline, statement) = match (record.start, record.end) {
        (Some(start), Some(end)) => {
            let elapsed = (end.date - start.dated.date).num_days();
            if elapsed >= i64::from(required) {
                return None;
            }
            let start_label = if start.posted { "posting" } else { "notice" };
            let mut basis_points = FULL_CONFIDENCE - effective_penalty;
            if !start.posted {
                basis_points -= NOTICE_DATE_START_PENALTY;
                caveats.push(
                    "No posting date is recorded, so the notice date is used; posting at the site may have been later."
                        .to_string(),
                );
            }
            if start.dated.span == end.span {
                citations.push(citation(
                    document,
                    end.span,
                    &format!("Recorded {start_label} and cleanup dates"),
                ));
            } else {
                citations.push(citation(
                    document,
                    start.dated.span,
                    &format!("Recorded {start_label} date"),
                ));
                citations.push(citation(document, end.span, "Recorded cleanup date"));
            }
            (
                FindingKind::EncampmentNoticeShortfall,
                FindingSeverity::Serious,
                basis_points,
                format!(
                    "{agency}: an encampment notice records {elapsed} calendar days from {start_label} to cleanup, below the {required}-day minimum {} states",
                    duty.named()
                ),
                format!(
                    "'{name}' records the {start_label} date as {} and the cleanup date as {}, {elapsed} calendar days apart.",
                    start.dated.date, end.date
                ),
            )
        }
        _ => {
            let period = record.stated_period?;
            if period.days >= required {
                return None;
            }
            let mut basis_points = STATED_PERIOD_CONFIDENCE - effective_penalty;
            if timed_by_upload {
                basis_points -= DOCUMENT_POSTED_TIMING_PENALTY;
                caveats.push(format!(
                    "The notice records no dated posting, notice or cleanup, so which rule was in force rests on the date '{name}' was posted ({}).",
                    notice.posted
                ));
            }
            let missing = match (record.start, record.end) {
                (None, None) => "no posting, notice or cleanup date",
                (Some(_), None) => "no cleanup date",
                _ => "no posting or notice date",
            };
            citations.push(citation(document, period.span, "Stated notice period"));
            (
                FindingKind::EncampmentNoticeStatedShortfall,
                FindingSeverity::Notable,
                basis_points,
                format!(
                    "{agency}: an encampment notice states a {}-day period, below the {required}-day minimum {} states",
                    period.days,
                    duty.named()
                ),
                format!(
                    "'{name}' states a notice period of {} calendar days. It records {missing}, so the period actually given is not recorded and is not counted.",
                    period.days
                ),
            )
        }
    };
    if let Duty::Policy(policy) = duty {
        citations.push(citation(
            policy.document,
            policy.duty.span,
            "Agency notice duty",
        ));
    }
    let explanation = format!(
        "{} {statement} This compares dates and periods the documents state, in calendar days only; \
         periods stated in hours or business days are not compared, and it does not establish when \
         the notice was physically posted or the cleanup took place beyond what the documents say. {}",
        duty.description(),
        caveats.join(" ")
    );
    let subject = FindingSubject {
        portal_key: Some(document.portal_key.clone()),
        agency_id: notice.agency.map(|agency| agency.id),
        request_id: Some(document.request_id),
        document_id: Some(document.document_id),
        ..Default::default()
    };
    let salient = format!(
        "duty={};required={required};start={:?};end={:?};period={:?}",
        duty.key(),
        record.start.map(|start| start.dated.date),
        record.end.map(|end| end.date),
        record.stated_period.map(|period| period.days),
    );
    let now = Utc::now();
    Some(Finding {
        id: finding_id(kind, SLUG, &subject, &[&salient]),
        kind,
        severity,
        confidence: FindingConfidence::measured(basis_points),
        subject,
        headline,
        explanation: explanation.trim_end().to_string(),
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.to_string(),
    })
}

fn excerpt(text: &str) -> String {
    text.trim().chars().take(QUOTE_CHARACTERS).collect()
}

fn citation(document: &PublicDocument, (start, end): Span, label: &str) -> FindingCitation {
    let text = &document.text;
    FindingCitation {
        portal_key: document.portal_key.clone(),
        document_id: document.document_id,
        character_start: i64::try_from(text[..start].chars().count()).ok(),
        character_end: i64::try_from(text[..end].chars().count()).ok(),
        quote: Some(excerpt(&text[start..end])),
        label: Some(label.to_string()),
    }
}

fn document_name(document: &PublicDocument) -> String {
    document
        .file_name
        .clone()
        .unwrap_or_else(|| format!("document {}", document.document_id))
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let documents = load_public_documents(repository, MAX_DOCUMENTS).await?;
    if documents.is_empty() {
        return Ok(Vec::new());
    }
    let agencies = load_agencies(repository).await?;
    let statutes = notice_statutes(load_statutes(repository).await?);
    let jurisdictions = load_jurisdictions(repository).await?;
    Ok(findings(&sources(
        &documents,
        &agencies,
        statutes,
        jurisdictions,
    )))
}

async fn load_agencies(repository: &DuckDbRepository) -> Result<AgencyIndex, AppError> {
    repository
        .query_connection("Encampment notice agency sweep", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT r.portal_subdomain, r.id, a.id, a.name
                     FROM requests r
                     INNER JOIN agencies a
                        ON a.id = r.agency_id AND a.portal_subdomain = r.portal_subdomain
                     ORDER BY r.created_at DESC
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare agency sweep: {error}"))
                })?;
            let rows = statement
                .query_map(params![MAX_REQUESTS], |row| {
                    Ok((
                        (row.get::<_, String>(0)?, row.get::<_, i64>(1)?),
                        Agency {
                            id: row.get(2)?,
                            name: row.get(3)?,
                        },
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run agency sweep: {error}"))
                })?;
            rows.collect::<Result<AgencyIndex, _>>()
                .map_err(|error| AppError::Storage(format!("Failed to decode agency row: {error}")))
        })
        .await
}

async fn load_statutes(repository: &DuckDbRepository) -> Result<Vec<StatuteRow>, AppError> {
    repository
        .query_connection("Encampment notice statute sweep", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT statute_id, jurisdiction, statute_citation, title, full_text,
                            mandated_deadline_days, effective_from, effective_to
                     FROM legal_statutes
                     WHERE mandated_deadline_days IS NOT NULL
                     ORDER BY statute_id
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare statute sweep: {error}"))
                })?;
            let rows = statement
                .query_map(params![MAX_STATUTES], |row| {
                    Ok(StatuteRow {
                        statute_id: row.get(0)?,
                        jurisdiction: row.get(1)?,
                        citation: row.get(2)?,
                        title: row.get(3)?,
                        full_text: row.get(4)?,
                        mandated_days: row.get(5)?,
                        effective_from: row.get(6)?,
                        effective_to: row.get(7)?,
                    })
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run statute sweep: {error}"))
                })?;
            rows.collect::<Result<Vec<_>, _>>().map_err(|error| {
                AppError::Storage(format!("Failed to decode statute row: {error}"))
            })
        })
        .await
}

async fn load_jurisdictions(
    repository: &DuckDbRepository,
) -> Result<HashMap<String, Vec<String>>, AppError> {
    repository
        .query_connection("Encampment notice portal sweep", move |conn| {
            let mut statement = conn
                .prepare("SELECT subdomain, display_name, base_url FROM portals")
                .map_err(|error| {
                    AppError::Storage(format!("Failed to prepare portal sweep: {error}"))
                })?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|error| {
                    AppError::Storage(format!("Failed to run portal sweep: {error}"))
                })?;
            let mut names = HashMap::new();
            for row in rows {
                let (portal, display_name, base_url) = row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode portal row: {error}"))
                })?;
                let mut known = vec![display_name.trim().to_string()];
                let recognised = source_jurisdiction(&base_url);
                if recognised != "Unclassified source" {
                    known.push(recognised.to_string());
                }
                names.insert(portal, known);
            }
            Ok(names)
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_loaders_run_against_the_schema_and_the_seeded_cpra_row_is_not_a_notice_minimum() {
        let repo = DuckDbRepository::new(":memory:").expect("open in-memory core");
        repo.run_migrations(include_str!("../../../migrations/0001_initial_schema.sql"))
            .await
            .expect("run migrations");
        let rows = load_statutes(&repo).await.expect("statutes");
        assert!(rows.iter().any(|row| row.statute_id == "CPRA_7922.535"));
        assert!(notice_statutes(rows).is_empty());
        load_agencies(&repo).await.expect("agencies");
        load_jurisdictions(&repo).await.expect("portals");
    }

    const POLICY: &str = "Encampment Resolution Policy\n\
        The Encampment Resolution Policy is effective January 1, 2026.\n\
        Notice shall be posted at least 3 calendar days before any encampment cleanup.\n\
        Personal property removed will be stored for 90 days.";

    fn document(id: i64, file_name: &str, text: &str) -> PublicDocument {
        PublicDocument {
            portal_key: "sd".into(),
            document_id: id,
            request_id: id,
            file_name: Some(file_name.into()),
            created_at: "2026-02-01T00:00:00Z".into(),
            text: text.into(),
        }
    }

    fn notice_text(elapsed: u32) -> String {
        format!(
            "Notice to vacate encampment.\nDate posted: March 1, 2026.\nCleanup will occur on March {}, 2026.",
            1 + elapsed
        )
    }

    fn agencies(ids: &[i64]) -> AgencyIndex {
        ids.iter()
            .map(|id| {
                (
                    ("sd".to_string(), *id),
                    Agency {
                        id: 9,
                        name: "Environmental Services".into(),
                    },
                )
            })
            .collect()
    }

    fn statute(jurisdiction: &str, days: i64, title: &str, text: &str) -> StatuteRow {
        StatuteRow {
            statute_id: format!("S-{title}"),
            jurisdiction: jurisdiction.into(),
            citation: "Mun. Code § 1".into(),
            title: title.into(),
            full_text: text.into(),
            mandated_days: Some(days),
            effective_from: Some("2025-01-01".into()),
            effective_to: None,
        }
    }

    #[test]
    fn dates_take_their_role_from_the_nearest_cue_and_conflicts_lengthen_the_interval() {
        let record = notice_record(
            "Encampment notice. Notice issued Feb. 27, 2026; posted 3/1/2026 and posted again 3/2/2026.\n\
             The area will be cleaned up on March 4, 2026. Cleanup moved to March 5, 2026. Date: March 9, 2026.",
        )
        .expect("record");
        let start = record.start.expect("start");
        assert!(start.posted);
        assert_eq!(
            start.dated.date,
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()
        );
        assert_eq!(
            record.end.unwrap().date,
            NaiveDate::from_ymd_opt(2026, 3, 5).unwrap()
        );
        assert_eq!(
            notice_record("Encampment cleanup March 1, 2026. Posted March 4, 2026."),
            None,
            "a cleanup before the posting is skipped, not counted as zero"
        );
    }

    #[test]
    fn business_day_hour_and_storage_periods_are_not_read() {
        let stated = |text: &str| notice_record(text).and_then(|r| r.stated_period.map(|p| p.days));
        assert_eq!(stated("You must vacate within 3 business days."), None);
        assert_eq!(stated("You must vacate within 72 hours."), None);
        assert_eq!(stated("Removed property will be stored for 90 days."), None);
        assert_eq!(stated("You must vacate within three (3) days."), Some(3));
        assert_eq!(stated("You must vacate within twenty-one days."), None);
        assert_eq!(policy_duty(POLICY).map(|d| d.days), Some(3));
        assert_eq!(
            policy_duty("Notice shall be posted 3 business days before cleanup. Notice shall be posted 72 hours before removal."),
            None
        );
        assert_eq!(
            policy_duty("Removed property shall be stored 90 days before it is discarded."),
            None
        );
    }

    #[test]
    fn a_counted_shortfall_is_below_the_policy_minimum_and_nothing_at_or_above_it() {
        let policy = document(1, "encampment policy.pdf", POLICY);
        for (elapsed, expected) in [(2, 1), (3, 0), (4, 0)] {
            let documents = [
                policy.clone_for_test(),
                document(2, "notice.pdf", &notice_text(elapsed)),
            ];
            let index = agencies(&[1, 2]);
            let found = findings(&sources(&documents, &index, Vec::new(), HashMap::new()));
            assert_eq!(found.len(), expected, "elapsed {elapsed}");
            if let Some(finding) = found.first() {
                assert_eq!(finding.kind, FindingKind::EncampmentNoticeShortfall);
                assert_eq!(
                    finding.confidence,
                    FindingConfidence::measured(FULL_CONFIDENCE)
                );
                assert_eq!(finding.citations.len(), 3);
                assert!(finding.check_recordable().is_ok());
            }
        }
    }

    #[test]
    fn a_stated_period_is_compared_only_when_the_dates_are_not_both_recorded() {
        let policy = document(1, "encampment policy.pdf", POLICY);
        for (days, expected) in [(2, 1), (3, 0), (4, 0)] {
            let text = format!(
                "Encampment notice. Date posted: March 1, 2026. You must vacate within {days} days."
            );
            let documents = [policy.clone_for_test(), document(2, "notice.pdf", &text)];
            let index = agencies(&[1, 2]);
            let found = findings(&sources(&documents, &index, Vec::new(), HashMap::new()));
            assert_eq!(found.len(), expected, "stated {days}");
            if let Some(finding) = found.first() {
                assert_eq!(finding.kind, FindingKind::EncampmentNoticeStatedShortfall);
                assert!(!finding.kind.is_legal_claim());
                assert!(finding.explanation.contains("no cleanup date"));
            }
        }
    }

    #[test]
    fn only_a_stored_encampment_notice_authority_of_the_portals_jurisdiction_applies() {
        let cpra = statute(
            "City of San Diego",
            10,
            "California Public Records Act determination timing",
            "Determination timing authority for California public records requests.",
        );
        let camping = statute(
            "City of San Diego",
            3,
            "Encampment notice",
            "Notice shall be posted before removal of an encampment.",
        );
        let statutes = notice_statutes(vec![cpra, camping]);
        assert_eq!(statutes.len(), 1);
        assert_eq!(statutes[0].days, 3);

        let documents = [document(2, "notice.pdf", &notice_text(2))];
        let index = AgencyIndex::new();
        let run = |jurisdiction: &str| {
            let names = HashMap::from([("sd".to_string(), vec![jurisdiction.to_string()])]);
            findings(&sources(&documents, &index, statutes.clone(), names))
        };
        let found = run("city of san diego");
        assert_eq!(found.len(), 1);
        assert!(found[0].headline.contains("Mun. Code § 1"));
        assert!(found[0].check_recordable().is_ok());
        assert!(run("City of Escondido").is_empty());
    }

    #[test]
    fn citations_count_characters_not_bytes() {
        let text = "Campamento — aviso. Date posted: March 1, 2026. Cleanup on March 2, 2026.";
        let doc = document(2, "aviso.pdf", text);
        let record = notice_record(text).unwrap();
        let span = record.end.unwrap().span;
        let cite = citation(&doc, span, "x");
        let start = usize::try_from(cite.character_start.unwrap()).unwrap();
        let quoted: String = text.chars().skip(start).take(7).collect();
        assert_eq!(quoted.trim_start(), "Cleanu");
    }

    impl PublicDocument {
        fn clone_for_test(&self) -> Self {
            Self {
                portal_key: self.portal_key.clone(),
                document_id: self.document_id,
                request_id: self.request_id,
                file_name: self.file_name.clone(),
                created_at: self.created_at.clone(),
                text: self.text.clone(),
            }
        }
    }
}
