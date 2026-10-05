//! Meeting calendar and cited agenda suggestions, built only from ingested documents.
//!
//! A meeting's body and date come from its own documents' headings when present. A heading whose
//! dates conflict (more than one date, or a weekday that doesn't match its date) is listed as
//! "date not recorded"; a heading with no date at all falls back to the document's file name or
//! description, and absent any of those it too is "date not recorded" — never placed on a day
//! borrowed from its body text or a sibling document.
//! Agenda, minutes and video documents are linked only by a shared recorded identity: the
//! publisher's meeting identifier, or the same body and the same recorded date (heading or
//! fallback). Suggestions are deterministic phrase matches over complete agenda items; each
//! carries the agenda passage it rests on and a basis label.
use super::agenda::{heading_date, HeadingDate, ITEMS};
use crate::{
    analysis::{
        claim_validation::{
            claim_check_view, extract_claims, ClaimEvaluation, Evaluator, QuantitativeClaim,
            UnreconciledReason,
        },
        finding::FindingCitation,
        gaps::{self, EvidenceGap, GapRole},
        phrase_match::Phrases,
    },
    error::AppError,
    integrity::sha256_hex,
    storage::{findings::FindingsFilter, pii::mask_pii, sqlite::DuckDbRepository},
    types::{meetings::*, ClaimCheckOutcome, CoverageExpectation},
};
use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;

const HEADER_CHARACTERS: usize = 12_000;
const ROLE_CHARACTERS: usize = 1_500;
const ITEM_CHARACTERS: usize = 12_000;
const SOURCE_LIMIT: usize = 300;
const TEXT_CHARACTERS: usize = 300_000;
const SUGGESTIONS_PER_MEETING: usize = 12;
const PASSAGES_PER_KIND: usize = 3;
const QUOTE_CHARACTERS: usize = 400;

/// Publishing bodies, earliest heading match wins. Extend this table to recognize a new body.
const BODIES: &[(&str, Phrases)] = &[
    (
        "City Council",
        &[
            &["city", "council"],
            &["council", "agenda"],
            &["council", "minutes"],
            &["council", "meeting"],
        ],
    ),
    ("Board of Supervisors", &[&["board", "of", "supervisors"]]),
    ("Planning Commission", &[&["planning", "commission"]]),
    (
        "Board of Commissioners",
        &[&["board", "of", "commissioners"]],
    ),
    (
        "Commission on Police Practices",
        &[&["commission", "on", "police", "practices"]],
    ),
    ("Board of Education", &[&["board", "of", "education"]]),
    (
        "Parks and Recreation Board",
        &[
            &["park", "and", "recreation", "board"],
            &["parks", "and", "recreation", "board"],
        ],
    ),
];
const AGENDA: Phrases = &[&["agenda"]];
const MINUTES: Phrases = &[&["minutes"]];
const VIDEO: Phrases = &[&["video"], &["recording"], &["webcast"]];
const COMMENT: Phrases = &[
    &["comment"],
    &["comments"],
    &["testimony"],
    &["written", "materials"],
    &["speaker", "slip"],
    &["speaker", "slips"],
];
const DEADLINE: Phrases = &[
    &["deadline"],
    &["no", "later", "than"],
    &["received", "by"],
    &["hours", "before"],
    &["hours", "in", "advance"],
    &["prior", "to", "the", "meeting"],
];
const INSTRUCTION: Phrases = &[
    &["submit"],
    &["webform"],
    &["to", "speak"],
    &["sign", "up"],
    &["e", "mail"],
];
/// The research topics (`storage::research`), with inflections spelled out: a phrase matches whole
/// words only.
const TOPICS: &[(&str, Phrases)] = &[
    (
        "homelessness",
        &[
            &["homeless"],
            &["homelessness"],
            &["shelter"],
            &["shelters"],
            &["encampment"],
            &["encampments"],
            &["housing"],
            &["riverbed"],
        ],
    ),
    (
        "abatement",
        &[
            &["encampment"],
            &["encampments"],
            &["abatement"],
            &["abatements"],
            &["riverbed"],
            &["outreach"],
            &["impound"],
            &["impounded"],
        ],
    ),
    (
        "bathrooms",
        &[
            &["restroom"],
            &["restrooms"],
            &["bathroom"],
            &["bathrooms"],
            &["toilet"],
            &["toilets"],
            &["shower"],
            &["showers"],
            &["sanitation"],
            &["comfort", "station"],
            &["comfort", "stations"],
            &["handwash"],
            &["handwashing"],
        ],
    ),
    (
        "police",
        &[
            &["police"],
            &["overtime"],
            &["law", "enforcement"],
            &["surveillance"],
        ],
    ),
    (
        "waste",
        &[
            &["audit"],
            &["audits"],
            &["overpayment"],
            &["overpayments"],
            &["reimbursement"],
            &["reimbursements"],
            &["contract"],
            &["contracts"],
            &["amendment"],
            &["amendments"],
            &["expenditure"],
            &["expenditures"],
        ],
    ),
];

fn gap_terms(role: GapRole) -> (&'static str, Phrases) {
    match role {
        GapRole::Appropriations => (
            "appropriation",
            &[&["appropriation"], &["appropriations"], &["budget"]],
        ),
        GapRole::Encumbrances => (
            "encumbrance",
            &[&["encumbrance"], &["encumbrances"], &["encumber"]],
        ),
        GapRole::Contracts => (
            "contract",
            &[
                &["contract"],
                &["contracts"],
                &["agreement"],
                &["agreements"],
            ],
        ),
        GapRole::Amendments => ("amendment", &[&["amendment"], &["amendments"]]),
        GapRole::Payments => (
            "payment",
            &[
                &["payment"],
                &["payments"],
                &["expenditure"],
                &["expenditures"],
            ],
        ),
        GapRole::Outcomes => ("outcome", &[&["outcome"], &["outcomes"], &["deliverables"]]),
        GapRole::FundRestrictions => (
            "fund restriction",
            &[
                &["restricted", "fund"],
                &["restricted", "funds"],
                &["grant"],
            ],
        ),
        GapRole::DeterminationDates | GapRole::AgencyRecords => ("", &[]),
    }
}

/// One ingested document considered for the calendar. `text` is empty when no extraction is
/// retained (a video, or a document not yet searchable).
#[derive(Debug, Clone)]
pub(crate) struct MeetingSource {
    pub portal_key: String,
    pub jurisdiction: String,
    pub document_id: i64,
    pub file_name: String,
    pub description: String,
    pub content_type: String,
    pub source_url: Option<String>,
    pub text: String,
}

impl MeetingSource {
    fn label_text(&self) -> String {
        format!("{} {}", self.file_name, self.description)
    }
}

pub(crate) struct WatchedName {
    pub name: String,
    pub kind: SuggestionReasonKind,
    pub source: String,
}

pub(crate) struct FindingSubjectName {
    pub finding_id: String,
    pub name: String,
    pub kind_label: String,
    pub legal_claim: bool,
}

#[derive(Default)]
pub(crate) struct MatchContext {
    pub watched: Vec<WatchedName>,
    pub subjects: Vec<FindingSubjectName>,
    pub gaps: Vec<EvidenceGap>,
}

struct Token {
    text: String,
    start: usize,
    end: usize,
}

/// Lowercased word tokens with byte ranges, split as `phrase_match::words` splits.
fn tokens(text: &str) -> Vec<Token> {
    let mut output = Vec::new();
    let mut current = String::new();
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if character.is_alphanumeric() || (matches!(character, '\'' | '’') && !current.is_empty())
        {
            if current.is_empty() {
                start = index;
            }
            if character == '’' {
                current.push('\'');
            } else {
                current.extend(character.to_lowercase());
            }
        } else if !current.is_empty() {
            output.push(Token {
                text: std::mem::take(&mut current),
                start,
                end: index,
            });
        }
    }
    if !current.is_empty() {
        output.push(Token {
            text: current,
            start,
            end: text.len(),
        });
    }
    output
}

fn matches_at(tokens: &[Token], index: usize, phrase: &[&str]) -> bool {
    !phrase.is_empty()
        && tokens.len() >= index + phrase.len()
        && tokens[index..index + phrase.len()]
            .iter()
            .zip(phrase)
            .all(|(token, cue)| token.text == *cue)
}

/// The earliest match of any phrase, as a byte range.
fn first_phrase(tokens: &[Token], phrases: Phrases) -> Option<Range<usize>> {
    (0..tokens.len()).find_map(|index| {
        phrases
            .iter()
            .find(|phrase| matches_at(tokens, index, phrase))
            .map(|phrase| tokens[index].start..tokens[index + phrase.len() - 1].end)
    })
}

fn first_words(tokens: &[Token], words: &[String]) -> Option<Range<usize>> {
    let phrase = words.iter().map(String::as_str).collect::<Vec<_>>();
    (0..tokens.len())
        .find(|&index| matches_at(tokens, index, &phrase))
        .map(|index| tokens[index].start..tokens[index + phrase.len() - 1].end)
}

fn has_phrase(tokens: &[Token], phrases: Phrases) -> bool {
    first_phrase(tokens, phrases).is_some()
}

fn byte_at_char(text: &str, characters: usize) -> usize {
    text.char_indices()
        .nth(characters)
        .map_or(text.len(), |(index, _)| index)
}

fn trimmed(text: &str, range: Range<usize>) -> Range<usize> {
    let slice = &text[range.clone()];
    let start = range.start + (slice.len() - slice.trim_start().len());
    let end = range.end - (slice.len() - slice.trim_end().len());
    start..end.max(start)
}

/// A citation into the retained extraction: exact character offsets, PII masked in the quote.
fn cite_text(source: &MeetingSource, range: Range<usize>) -> Result<FindingCitation, AppError> {
    let range = trimmed(&source.text, range);
    let start = source.text[..range.start].chars().count();
    let quote = &source.text[range];
    Ok(FindingCitation {
        portal_key: source.portal_key.clone(),
        document_id: source.document_id,
        character_start: Some(start as i64),
        character_end: Some((start + quote.chars().count()) as i64),
        quote: Some(mask_pii(quote)?.masked_text),
        label: Some(source.file_name.clone()),
    })
}

/// A citation into the document's file name or description, which has no character range in the
/// extraction and says so.
fn cite_label(source: &MeetingSource, range: Range<usize>) -> Result<FindingCitation, AppError> {
    let label = source.label_text();
    let range = trimmed(&label, range);
    Ok(FindingCitation {
        portal_key: source.portal_key.clone(),
        document_id: source.document_id,
        character_start: None,
        character_end: None,
        quote: Some(mask_pii(&label[range])?.masked_text),
        label: Some(format!("{} (file name or description)", source.file_name)),
    })
}

/// Sentence ranges: split at bullets, line breaks, and a period followed by space and a capital.
fn sentences(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let slice = &text[range.clone()];
    let mut output = Vec::new();
    let mut start = 0;
    let characters = slice.char_indices().collect::<Vec<_>>();
    for (position, &(index, character)) in characters.iter().enumerate() {
        let boundary = match character {
            '•' | '\n' => Some((index, index + character.len_utf8())),
            '.' => {
                let space = characters
                    .get(position + 1)
                    .is_some_and(|(_, c)| c.is_whitespace());
                let capital = characters
                    .get(position + 2)
                    .is_some_and(|(_, c)| c.is_uppercase() || c.is_ascii_digit());
                (space && capital).then_some((index + 1, index + 1))
            }
            _ => None,
        };
        if let Some((end, next)) = boundary {
            if end > start {
                output.push(range.start + start..range.start + end);
            }
            start = next;
        }
    }
    if start < slice.len() {
        output.push(range.start + start..range.end);
    }
    output
        .into_iter()
        .map(|r| trimmed(text, r))
        .filter(|r| !r.is_empty())
        .collect()
}

/// The sentence of `within` that contains `hit`, bounded to a quotable length around the hit.
fn sentence_around(text: &str, within: Range<usize>, hit: Range<usize>) -> Range<usize> {
    let sentence = sentences(text, within.clone())
        .into_iter()
        .find(|s| s.start <= hit.start && hit.end <= s.end)
        .unwrap_or(hit.clone());
    if text[sentence.clone()].chars().count() <= QUOTE_CHARACTERS {
        return sentence;
    }
    let before = text[sentence.start..hit.start]
        .char_indices()
        .rev()
        .nth(QUOTE_CHARACTERS / 2)
        .map_or(sentence.start, |(i, _)| sentence.start + i);
    let after = text[hit.end..sentence.end]
        .char_indices()
        .nth(QUOTE_CHARACTERS / 2)
        .map_or(sentence.end, |(i, _)| hit.end + i);
    before..after
}

enum DateReading {
    Recorded {
        date: NaiveDate,
        range: Range<usize>,
        in_text: bool,
    },
    Conflicting,
    Absent,
}

struct Recognized {
    role: MeetingDocumentRole,
    body: &'static str,
    body_range: Range<usize>,
    body_in_text: bool,
    date: DateReading,
    publisher_meeting: Option<String>,
}

fn prefix(text: &str, characters: usize) -> &str {
    &text[..byte_at_char(text, characters)]
}

fn first_body(tokens: &[Token]) -> Option<(&'static str, Range<usize>)> {
    BODIES
        .iter()
        .filter_map(|(label, phrases)| first_phrase(tokens, phrases).map(|r| (*label, r)))
        .min_by_key(|(_, range)| range.start)
}

/// Agenda or minutes by whichever names itself first; "non-agenda" is not an agenda.
fn agenda_or_minutes(tokens: &[Token]) -> Option<MeetingDocumentRole> {
    let agenda = (0..tokens.len())
        .find(|&i| matches_at(tokens, i, AGENDA[0]) && (i == 0 || tokens[i - 1].text != "non"));
    let minutes = (0..tokens.len()).find(|&i| matches_at(tokens, i, MINUTES[0]));
    match (agenda, minutes) {
        (Some(a), Some(m)) if m < a => Some(MeetingDocumentRole::Minutes),
        (Some(_), _) => Some(MeetingDocumentRole::Agenda),
        (None, Some(_)) => Some(MeetingDocumentRole::Minutes),
        (None, None) => None,
    }
}

fn publisher_meeting(url: Option<&str>) -> Option<String> {
    let url = reqwest::Url::parse(url?).ok()?;
    if url.host_str()? != "sandiego.hylandcloud.com" {
        return None;
    }
    url.query_pairs()
        .find(|(key, _)| key.eq_ignore_ascii_case("meetingId"))
        .map(|(_, value)| value.into_owned())
        .filter(|id| !id.is_empty() && id.len() <= 18 && id.bytes().all(|b| b.is_ascii_digit()))
        .map(|id| format!("sandiego.hylandcloud.com:{id}"))
}

fn header_end(text: &str) -> usize {
    let bound = byte_at_char(text, HEADER_CHARACTERS);
    ITEMS
        .find(text)
        .map_or(bound, |item| item.start().min(bound))
}

fn recognize(source: &MeetingSource) -> Option<Recognized> {
    let label = source.label_text();
    let label_tokens = tokens(&label);
    let header = &source.text[..header_end(&source.text)];
    let header_tokens = tokens(header);
    let role = if source.content_type.to_lowercase().starts_with("video/")
        || has_phrase(&label_tokens, VIDEO)
    {
        MeetingDocumentRole::Video
    } else {
        agenda_or_minutes(&tokens(prefix(&source.text, ROLE_CHARACTERS)))
            .or_else(|| agenda_or_minutes(&label_tokens))?
    };
    let (body, body_range, body_in_text) = match first_body(&header_tokens) {
        Some((body, range)) => (body, range, true),
        None => {
            let (body, range) = first_body(&label_tokens)?;
            (body, range, false)
        }
    };
    let date = match heading_date(header) {
        HeadingDate::Recorded(date, range) => DateReading::Recorded {
            date,
            range,
            in_text: true,
        },
        HeadingDate::Conflicting => DateReading::Conflicting,
        HeadingDate::Absent => match heading_date(&label) {
            HeadingDate::Recorded(date, range) => DateReading::Recorded {
                date,
                range,
                in_text: false,
            },
            HeadingDate::Conflicting => DateReading::Conflicting,
            HeadingDate::Absent => DateReading::Absent,
        },
    };
    Some(Recognized {
        role,
        body,
        body_range,
        body_in_text,
        date,
        publisher_meeting: publisher_meeting(source.source_url.as_deref()),
    })
}

fn comment_passages(
    source: &MeetingSource,
) -> Result<(Vec<FindingCitation>, Vec<FindingCitation>), AppError> {
    let mut deadlines = Vec::new();
    let mut instructions = Vec::new();
    let mut seen = BTreeSet::new();
    for sentence in sentences(&source.text, 0..source.text.len()) {
        let quote = &source.text[sentence.clone()];
        let length = quote.chars().count();
        if !(12..=QUOTE_CHARACTERS).contains(&length) || !seen.insert(quote.to_lowercase()) {
            continue;
        }
        let words = tokens(quote);
        if !has_phrase(&words, COMMENT) {
            continue;
        }
        if has_phrase(&words, DEADLINE) {
            if deadlines.len() < PASSAGES_PER_KIND {
                deadlines.push(cite_text(source, sentence)?);
            }
        } else if has_phrase(&words, INSTRUCTION) && instructions.len() < PASSAGES_PER_KIND {
            instructions.push(cite_text(source, sentence)?);
        }
    }
    Ok((deadlines, instructions))
}

pub(crate) struct AgendaItem {
    pub meeting: usize,
    pub source: usize,
    pub label: String,
    pub title: String,
    pub range: Range<usize>,
    pub citation: FindingCitation,
}

fn agenda_items(
    source: &MeetingSource,
    meeting: usize,
    source_index: usize,
    limitations: &mut Vec<String>,
) -> Result<Vec<AgendaItem>, AppError> {
    let text = &source.text;
    let headings = ITEMS.captures_iter(text).collect::<Vec<_>>();
    let mut items = Vec::new();
    // Only a following item heading proves where an item ends.
    for pair in headings.windows(2) {
        let (Some(first), Some(next)) = (pair[0].get(0), pair[1].get(0)) else {
            continue;
        };
        let range = first.start()..next.start();
        if text[range.clone()].chars().count() > ITEM_CHARACTERS {
            limitations.push(format!(
                "{} {} exceeds the complete-item bound and was not evaluated.",
                source.file_name,
                pair[0]["label"].trim()
            ));
            continue;
        }
        let rest = &text[first.end()..next.start()];
        let title_end = ["Council District", "Proposed Actions", "Recommendation"]
            .iter()
            .filter_map(|marker| rest.find(marker))
            .min()
            .unwrap_or(rest.len());
        let title = rest[..title_end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(300)
            .collect();
        items.push(AgendaItem {
            meeting,
            source: source_index,
            label: pair[0]["label"]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            title,
            citation: cite_text(source, range.clone())?,
            range,
        });
    }
    if let Some(last) = headings.last() {
        limitations.push(format!(
            "{}: the last listed item ({}) has no following item heading, so its end is not proven and it was not evaluated.",
            source.file_name,
            last["label"].trim()
        ));
    }
    Ok(items)
}

pub(crate) struct ParsedCalendar {
    pub meetings: Vec<CalendarMeeting>,
    pub items: Vec<AgendaItem>,
    pub limitations: Vec<String>,
}

/// Groups meeting documents into meetings and extracts complete items from upcoming agendas.
/// `today` counts as upcoming.
pub(crate) fn parse_calendar(
    sources: &[MeetingSource],
    today: NaiveDate,
) -> Result<ParsedCalendar, AppError> {
    let mut groups: BTreeMap<String, Vec<(usize, Recognized)>> = BTreeMap::new();
    for (index, source) in sources.iter().enumerate() {
        let Some(found) = recognize(source) else {
            continue;
        };
        let key = match (&found.publisher_meeting, &found.date) {
            (Some(id), _) => format!("publisher:{id}"),
            (None, DateReading::Recorded { date, .. }) => {
                format!("body:{}:{}:{date}", source.portal_key, found.body)
            }
            (None, _) => format!("document:{}:{}", source.portal_key, source.document_id),
        };
        groups.entry(key).or_default().push((index, found));
    }
    let mut meetings = Vec::new();
    let mut items = Vec::new();
    let mut limitations = Vec::new();
    for (key, mut members) in groups {
        members.sort_by_key(|(index, found)| (found.role, sources[*index].document_id));
        let (lead_index, lead) = &members[0];
        let lead_source = &sources[*lead_index];
        let body_citation = if lead.body_in_text {
            cite_text(lead_source, lead.body_range.clone())?
        } else {
            cite_label(lead_source, lead.body_range.clone())?
        };
        let recorded = members
            .iter()
            .filter_map(|(index, found)| match &found.date {
                DateReading::Recorded {
                    date,
                    range,
                    in_text,
                } => Some((*index, *date, range.clone(), *in_text)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let distinct = recorded.iter().map(|r| r.1).collect::<BTreeSet<_>>();
        let (date, date_citation, date_note) = match (distinct.len(), recorded.first()) {
            (1, Some((index, date, range, in_text))) => {
                let citation = if *in_text {
                    cite_text(&sources[*index], range.clone())?
                } else {
                    cite_label(&sources[*index], range.clone())?
                };
                (Some(*date), Some(citation), None)
            }
            (0, _) => {
                let conflicting = members
                    .iter()
                    .any(|(_, found)| matches!(found.date, DateReading::Conflicting));
                let note = if conflicting {
                    "The heading names more than one date, or a weekday that does not match its date."
                } else {
                    "No weekday and full date appears in the heading or file name."
                };
                (None, None, Some(note.to_string()))
            }
            _ => (
                None,
                None,
                Some(
                    "Documents sharing this meeting's identifier record different dates."
                        .to_string(),
                ),
            ),
        };
        let timing = match date {
            Some(date) if date >= today => MeetingTiming::Upcoming,
            Some(_) => MeetingTiming::Past,
            None => MeetingTiming::DateNotRecorded,
        };
        let mut documents = Vec::new();
        let mut comment_deadlines = Vec::new();
        let mut comment_instructions = Vec::new();
        let meeting_index = meetings.len();
        // Sources arrive newest first; an amended or revised agenda supersedes earlier copies,
        // so items are read from the newest agenda only.
        let newest_agenda = members
            .iter()
            .filter(|(index, found)| {
                found.role == MeetingDocumentRole::Agenda && !sources[*index].text.is_empty()
            })
            .map(|(index, _)| *index)
            .min();
        let mut quoted_passages = BTreeSet::new();
        for (index, found) in &members {
            let source = &sources[*index];
            documents.push(MeetingDocument {
                role: found.role,
                portal_key: source.portal_key.clone(),
                document_id: source.document_id,
                label: source.file_name.clone(),
                source_url: source.source_url.clone(),
            });
            if found.role != MeetingDocumentRole::Agenda || source.text.is_empty() {
                continue;
            }
            let (deadlines, instructions) = comment_passages(source)?;
            let mut fresh =
                |citation: &FindingCitation| quoted_passages.insert(citation.quote.clone());
            comment_deadlines.extend(deadlines.into_iter().filter(|c| fresh(c)));
            comment_instructions.extend(instructions.into_iter().filter(|c| fresh(c)));
            if Some(*index) != newest_agenda {
                if timing == MeetingTiming::Upcoming {
                    limitations.push(format!(
                        "{}: a newer agenda for the same meeting was collected, so items were read from that one only.",
                        source.file_name
                    ));
                }
            } else if timing == MeetingTiming::Upcoming {
                items.extend(agenda_items(
                    source,
                    meeting_index,
                    *index,
                    &mut limitations,
                )?);
            }
        }
        comment_deadlines.truncate(PASSAGES_PER_KIND);
        comment_instructions.truncate(PASSAGES_PER_KIND);
        meetings.push(CalendarMeeting {
            id: sha256_hex(key.as_bytes())[..16].to_string(),
            jurisdiction: lead_source.jurisdiction.clone(),
            body: lead.body.to_string(),
            body_citation,
            date: date.map(|d| d.format("%Y-%m-%d").to_string()),
            date_citation,
            date_note,
            timing,
            documents,
            comment_deadlines,
            comment_instructions,
            suggestions: Vec::new(),
        });
    }
    Ok(ParsedCalendar {
        meetings,
        items,
        limitations,
    })
}

fn name_words(name: &str) -> Vec<String> {
    tokens(name).into_iter().map(|t| t.text).collect()
}

fn stated_reason(
    source: &MeetingSource,
    item: &AgendaItem,
    hit: Range<usize>,
    kind: SuggestionReasonKind,
    reason: String,
) -> Result<SuggestionReason, AppError> {
    let matched = source.text[hit.clone()].to_string();
    Ok(SuggestionReason {
        kind,
        basis: SuggestionBasis::Stated,
        reason,
        matched,
        citation: cite_text(
            source,
            sentence_around(&source.text, item.range.clone(), hit),
        )?,
        finding_id: None,
        gap_key: None,
        claim_check: None,
    })
}

/// Every phrase-matched reason for one complete agenda item. Figure checks are added separately
/// because they query the imported tables.
pub(crate) fn item_reasons(
    source: &MeetingSource,
    item: &AgendaItem,
    context: &MatchContext,
) -> Result<Vec<SuggestionReason>, AppError> {
    let words = tokens(&source.text[item.range.clone()])
        .into_iter()
        .map(|t| Token {
            start: t.start + item.range.start,
            end: t.end + item.range.start,
            text: t.text,
        })
        .collect::<Vec<_>>();
    let mut reasons = Vec::new();
    for (topic, phrases) in TOPICS {
        if let Some(hit) = first_phrase(&words, phrases) {
            let term = source.text[hit.clone()].to_string();
            reasons.push(stated_reason(
                source,
                item,
                hit,
                SuggestionReasonKind::InvestigationTopic,
                format!("The item names \"{term}\", a term of the '{topic}' investigation topic."),
            )?);
        }
    }
    let mut named = BTreeSet::new();
    for watched in &context.watched {
        let phrase = name_words(&watched.name);
        if phrase.len() < 2 || !named.insert((watched.kind, phrase.clone())) {
            continue;
        }
        if let Some(hit) = first_words(&words, &phrase) {
            reasons.push(stated_reason(
                source,
                item,
                hit,
                watched.kind,
                format!(
                    "The item names \"{}\", listed in {}.",
                    watched.name, watched.source
                ),
            )?);
        }
    }
    for subject in &context.subjects {
        let phrase = name_words(&subject.name);
        if phrase.len() < 2 {
            continue;
        }
        let Some(hit) = first_words(&words, &phrase) else {
            continue;
        };
        let basis = if subject.legal_claim {
            SuggestionBasis::Counted
        } else {
            SuggestionBasis::Pattern
        };
        let described = if subject.legal_claim {
            "a counted comparison"
        } else {
            "a pattern observation over released documents"
        };
        reasons.push(SuggestionReason {
            kind: SuggestionReasonKind::FindingSubject,
            basis,
            reason: format!(
                "The item names \"{}\", the agency a stored finding ({}) is about. That finding is {described}; this item's mention does not extend it to the item.",
                subject.name, subject.kind_label
            ),
            matched: source.text[hit.clone()].to_string(),
            citation: cite_text(source, sentence_around(&source.text, item.range.clone(), hit))?,
            finding_id: Some(subject.finding_id.clone()),
            gap_key: None,
            claim_check: None,
        });
    }
    for gap in &context.gaps {
        let (record, phrases) = gap_terms(gap.role);
        let Some(hit) = first_phrase(&words, phrases) else {
            continue;
        };
        let mut reason = stated_reason(
            source,
            item,
            hit,
            SuggestionReasonKind::EvidenceGap,
            format!(
                "An evidence gap is open: {}. It cannot be tested without {record} records ({}). This item names {record} terms, so its attachments may hold that record. A gap is an untested route, not a finding.",
                gap.route, gap.blocked
            ),
        )?;
        reason.gap_key = Some(gap.key.clone());
        reasons.push(reason);
    }
    Ok(reasons)
}

fn figure_reason(
    source: &MeetingSource,
    item: &AgendaItem,
    claim: &QuantitativeClaim,
    evaluation: &ClaimEvaluation,
) -> Result<SuggestionReason, AppError> {
    let item_text = &source.text[item.range.clone()];
    let start = item.range.start + byte_at_char(item_text, claim.char_start);
    let end = item.range.start + byte_at_char(item_text, claim.char_end);
    let view = claim_check_view(claim, evaluation);
    let (basis, reason) = match &view.result {
        ClaimCheckOutcome::Consistent { observed, band, .. } => (
            SuggestionBasis::Counted,
            format!(
                "The item states a figure ({}); the imported tables show {observed}, within {band}.",
                view.kind_label
            ),
        ),
        ClaimCheckOutcome::Mismatch {
            observed,
            difference,
            band,
            ..
        } => (
            SuggestionBasis::Counted,
            format!(
                "The item states a figure ({}); the imported tables show {observed}, a difference of {difference} beyond {band}. This is a difference between two records, not a determination about either.",
                view.kind_label
            ),
        ),
        ClaimCheckOutcome::Unreconciled { reason, detail } => (
            SuggestionBasis::Stated,
            format!(
                "The item states a figure ({}) that could not be checked against the imported tables: {reason}. {detail}",
                view.kind_label
            ),
        ),
    };
    Ok(SuggestionReason {
        kind: SuggestionReasonKind::CheckableFigure,
        basis,
        reason,
        matched: view.claim_text.clone(),
        citation: cite_text(source, start..end)?,
        finding_id: None,
        gap_key: None,
        claim_check: Some(view),
    })
}

fn basis_sentence(basis: SuggestionBasis) -> &'static str {
    match basis {
        SuggestionBasis::Counted => {
            "Counted: a comparison computed from recorded figures in this corpus."
        }
        SuggestionBasis::Stated => "Stated: text a published document states, quoted as written.",
        SuggestionBasis::Pattern => {
            "Pattern: a statistical observation over released documents, not a determination about this item."
        }
    }
}

fn quoted(citation: &FindingCitation) -> String {
    let quote = citation.quote.as_deref().unwrap_or_default();
    let capped = quote.chars().take(QUOTE_CHARACTERS).collect::<String>();
    if capped.chars().count() < quote.chars().count() {
        format!("\"{capped}…\"")
    } else {
        format!("\"{capped}\"")
    }
}

/// A comment scaffold that quotes only cited passages and names each basis. Stored findings are
/// referenced by id, never quoted, so a finding's headline cannot become the commenter's assertion.
pub(crate) fn draft_comment(
    meeting: &CalendarMeeting,
    agenda_item: &str,
    title: &str,
    item_citation: &FindingCitation,
    reasons: &[SuggestionReason],
) -> PublicCommentDraft {
    let mut lines = vec![
        format!("Public comment: {}, {}", meeting.body, meeting.jurisdiction),
        format!(
            "Meeting date: {}",
            meeting.date.as_deref().unwrap_or("date not recorded")
        ),
        format!("Item: {agenda_item} ({title})"),
        String::new(),
        "[YOUR NAME], [OPTIONAL AFFILIATION]".to_string(),
        String::new(),
        format!(
            "I am commenting on {agenda_item}. The published agenda lists it as: {}",
            quoted(item_citation)
        ),
        String::new(),
        "What the cited record shows:".to_string(),
    ];
    let mut questions = BTreeSet::new();
    for reason in reasons {
        let line = match reason.kind {
            SuggestionReasonKind::InvestigationTopic
            | SuggestionReasonKind::WatchedAgency
            | SuggestionReasonKind::WatchedEntity => {
                questions.insert("What records support the proposed action, and are the staff report and every attachment for this item published?".to_string());
                format!("- (Stated) The agenda reads: {}", quoted(&reason.citation))
            }
            SuggestionReasonKind::FindingSubject => {
                questions.insert(format!(
                    "How does this item relate to the records about {} already released?",
                    reason.matched
                ));
                format!(
                    "- ({}) This corpus holds a stored {} concerning {} (finding {}). The agenda reads: {}",
                    if reason.basis == SuggestionBasis::Counted { "Counted" } else { "Pattern" },
                    if reason.basis == SuggestionBasis::Counted { "counted comparison" } else { "pattern observation" },
                    reason.matched,
                    reason.finding_id.as_deref().unwrap_or("id not recorded"),
                    quoted(&reason.citation)
                )
            }
            SuggestionReasonKind::EvidenceGap => {
                questions.insert("Will the records attached to this item be published so the open question can be tested?".to_string());
                format!(
                    "- (Stated) The agenda reads: {}. Records that would test a related route are not in the collected sources.",
                    quoted(&reason.citation)
                )
            }
            SuggestionReasonKind::CheckableFigure => {
                let check = reason.claim_check.as_ref();
                match check.map(|c| &c.result) {
                    Some(ClaimCheckOutcome::Consistent { observed, .. }) => format!(
                        "- (Counted) The item states {}. The imported tables show {observed}.",
                        quoted(&reason.citation)
                    ),
                    Some(ClaimCheckOutcome::Mismatch {
                        observed,
                        difference,
                        ..
                    }) => {
                        questions.insert("Which figure is current, and what accounts for the difference between the stated amount and the published table?".to_string());
                        format!(
                            "- (Counted) The item states {}. The imported tables show {observed}, a difference of {difference}.",
                            quoted(&reason.citation)
                        )
                    }
                    _ => {
                        questions.insert(
                            "Which published records support the stated amount?".to_string(),
                        );
                        format!(
                            "- (Stated) The item states {}. It could not be checked against the imported tables.",
                            quoted(&reason.citation)
                        )
                    }
                }
            }
        };
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    lines.push(String::new());
    lines.push("Questions:".to_string());
    lines.extend(questions.into_iter().map(|q| format!("- {q}")));
    lines.push(String::new());
    let deadline = meeting.comment_deadlines.first().cloned();
    match &deadline {
        Some(citation) => lines.push(format!(
            "Comment deadline, as the agenda records it: {}",
            quoted(citation)
        )),
        None => lines.push("Comment deadline: not recorded in the collected documents. Confirm it with the body before the meeting.".to_string()),
    }
    if meeting.comment_instructions.is_empty() {
        lines.push("How to comment: not recorded in the collected documents.".to_string());
    } else {
        for citation in &meeting.comment_instructions {
            lines.push(format!(
                "How to comment, as the agenda records it: {}",
                quoted(citation)
            ));
        }
    }
    let bases = reasons
        .iter()
        .map(|r| r.basis)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    lines.push(String::new());
    lines.push("Basis:".to_string());
    lines.extend(bases.iter().map(|b| format!("- {}", basis_sentence(*b))));
    lines.push("This draft quotes only the cited agenda text and records in this corpus. It does not characterize anyone's motives or the lawfulness of any action.".to_string());
    PublicCommentDraft {
        text: lines.join("\n"),
        bases,
        deadline,
        instructions: meeting.comment_instructions.clone(),
    }
}

/// Attaches suggestions to their meetings, strongest first (most distinct reason kinds).
pub(crate) fn assemble(
    parsed: &mut ParsedCalendar,
    sources: &[MeetingSource],
    mut reasons: Vec<Vec<SuggestionReason>>,
) {
    let mut by_meeting: BTreeMap<usize, Vec<(usize, usize, Vec<SuggestionReason>)>> =
        BTreeMap::new();
    for (position, item) in parsed.items.iter().enumerate() {
        let found = std::mem::take(&mut reasons[position]);
        if found.is_empty() {
            continue;
        }
        let kinds = found.iter().map(|r| r.kind).collect::<BTreeSet<_>>().len();
        by_meeting
            .entry(item.meeting)
            .or_default()
            .push((kinds, position, found));
    }
    for (meeting_index, mut candidates) in by_meeting {
        candidates.sort_by_key(|(kinds, position, _)| (std::cmp::Reverse(*kinds), *position));
        if candidates.len() > SUGGESTIONS_PER_MEETING {
            parsed.limitations.push(format!(
                "{} {}: {} agenda items matched; the {SUGGESTIONS_PER_MEETING} with the most distinct reasons are shown.",
                parsed.meetings[meeting_index].body,
                parsed.meetings[meeting_index].date.as_deref().unwrap_or("date not recorded"),
                candidates.len()
            ));
            candidates.truncate(SUGGESTIONS_PER_MEETING);
        }
        for (_, position, found) in candidates {
            let item = &parsed.items[position];
            let source = &sources[item.source];
            let meeting = &parsed.meetings[meeting_index];
            let comment = draft_comment(meeting, &item.label, &item.title, &item.citation, &found);
            let id = sha256_hex(
                format!(
                    "{}\u{1f}{}\u{1f}{}\u{1f}{}",
                    meeting.id, source.portal_key, source.document_id, item.label
                )
                .as_bytes(),
            )[..16]
                .to_string();
            let suggestion = MeetingSuggestion {
                id,
                agenda_item: item.label.clone(),
                title: item.title.clone(),
                item_citation: item.citation.clone(),
                reasons: found,
                comment,
            };
            parsed.meetings[meeting_index].suggestions.push(suggestion);
        }
    }
}

fn order(meetings: &mut [CalendarMeeting]) {
    meetings.sort_by(|a, b| {
        let rank = |m: &CalendarMeeting| match m.timing {
            MeetingTiming::Upcoming => 0,
            MeetingTiming::Past => 1,
            MeetingTiming::DateNotRecorded => 2,
        };
        rank(a).cmp(&rank(b)).then_with(|| match a.timing {
            MeetingTiming::Upcoming => a.date.cmp(&b.date),
            MeetingTiming::Past => b.date.cmp(&a.date),
            MeetingTiming::DateNotRecorded => a.id.cmp(&b.id),
        })
    });
}

struct LoadedSources {
    sources: Vec<MeetingSource>,
    agencies: HashMap<(String, i64), String>,
    coverage: Vec<String>,
    unreadable_coverage: usize,
}

async fn load_sources(repository: &DuckDbRepository) -> Result<LoadedSources, AppError> {
    repository
        .query_connection("Read meeting documents", |conn| {
            let storage = |e: duckdb::Error| AppError::Storage(e.to_string());
            let _deadline = crate::analysis::sql::QueryDeadline::start(
                conn,
                std::time::Duration::from_secs(15),
            )?;
            let mut statement = conn
                .prepare(
                    "SELECT d.portal_subdomain, p.display_name, d.id, COALESCE(d.file_name, ''),
                        COALESCE(d.description, ''), COALESCE(d.content_type, ''), d.download_url,
                        CASE WHEN d.extraction_status IN ('SEARCHABLE', 'SALVAGED')
                             THEN substr(COALESCE(f.extracted_text, ''), 1, ?1) ELSE '' END
                     FROM documents d
                     JOIN portals p ON p.subdomain = d.portal_subdomain
                     LEFT JOIN documents_fts f
                       ON f.document_id = d.id AND f.portal_subdomain = d.portal_subdomain
                     WHERE lower(trim(d.visibility)) = 'public' AND p.lifecycle_state <> 'REMOVED'
                       AND (lower(COALESCE(d.file_name, '') || ' ' || COALESCE(d.description, ''))
                              SIMILAR TO '.*(agenda|minutes|video|recording|webcast).*'
                            OR lower(COALESCE(d.content_type, '')) LIKE 'video/%'
                            OR lower(substr(COALESCE(f.extracted_text, ''), 1, 1500))
                              SIMILAR TO '.*(agenda|minutes).*')
                     ORDER BY d.updated_at DESC, d.id
                     LIMIT ?2",
                )
                .map_err(storage)?;
            let sources = statement
                .query_map(
                    duckdb::params![TEXT_CHARACTERS as i64, SOURCE_LIMIT as i64],
                    |row| {
                        Ok(MeetingSource {
                            portal_key: row.get(0)?,
                            jurisdiction: row.get(1)?,
                            document_id: row.get(2)?,
                            file_name: row.get(3)?,
                            description: row.get(4)?,
                            content_type: row.get(5)?,
                            source_url: row.get(6)?,
                            text: row.get(7)?,
                        })
                    },
                )
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            let mut agencies = HashMap::new();
            let mut statement = conn
                .prepare("SELECT portal_subdomain, id, name FROM agencies")
                .map_err(storage)?;
            for row in statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get(2)?))
                })
                .map_err(storage)?
            {
                let (portal, id, name) = row.map_err(storage)?;
                agencies.insert((portal, id), name);
            }
            let has_coverage: bool = conn
                .query_row(
                    "SELECT count(*) > 0 FROM duckdb_tables() WHERE table_name = 'monitor_coverage'",
                    [],
                    |row| row.get(0),
                )
                .map_err(storage)?;
            let mut coverage = Vec::new();
            let mut unreadable_coverage = 0usize;
            if has_coverage {
                let mut statement = conn
                    .prepare("SELECT payload FROM monitor_coverage ORDER BY id")
                    .map_err(storage)?;
                for payload in statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(storage)?
                {
                    let payload = payload.map_err(storage)?;
                    match serde_json::from_str::<CoverageExpectation>(&payload) {
                        Ok(expectation) => coverage.push(expectation.agency),
                        Err(_) => unreadable_coverage += 1,
                    }
                }
            }
            Ok(LoadedSources {
                sources,
                agencies,
                coverage,
                unreadable_coverage,
            })
        })
        .await
}

async fn match_context(
    repository: &DuckDbRepository,
    loaded: &LoadedSources,
    limitations: &mut Vec<String>,
) -> MatchContext {
    let mut context = MatchContext::default();
    if loaded.unreadable_coverage > 0 {
        limitations.push(format!(
            "{} monitoring coverage expectations could not be read, so their agencies were not matched.",
            loaded.unreadable_coverage
        ));
    }
    for agency in &loaded.coverage {
        context.watched.push(WatchedName {
            name: agency.clone(),
            kind: SuggestionReasonKind::WatchedAgency,
            source: "your monitoring coverage expectations".into(),
        });
    }
    match repository.casework_state().await {
        Ok(state) => {
            for case in state.cases {
                for commitment in case.commitments {
                    context.watched.push(WatchedName {
                        name: commitment.responsible_party,
                        kind: SuggestionReasonKind::WatchedEntity,
                        source: format!("a commitment in the casework case \"{}\"", case.title),
                    });
                }
            }
        }
        Err(error) => limitations.push(format!(
            "Casework commitments were not read, so responsible parties were not matched: {error}"
        )),
    }
    match repository
        .list_findings(FindingsFilter::default(), 500)
        .await
    {
        Ok(findings) => {
            for finding in findings {
                let (Some(portal), Some(agency)) = (
                    finding.subject.portal_key.as_ref(),
                    finding.subject.agency_id,
                ) else {
                    continue;
                };
                if let Some(name) = loaded.agencies.get(&(portal.clone(), agency)) {
                    context.subjects.push(FindingSubjectName {
                        finding_id: finding.id.clone(),
                        name: name.clone(),
                        kind_label: finding.kind.as_str().to_string(),
                        legal_claim: finding.kind.is_legal_claim(),
                    });
                }
            }
        }
        Err(error) => limitations.push(format!(
            "Stored findings were not read, so finding subjects were not matched: {error}"
        )),
    }
    let (found, errors) = gaps::collect(repository).await;
    context.gaps = found;
    limitations.extend(
        errors
            .into_iter()
            .map(|e| format!("Evidence gaps partly unavailable: {e}")),
    );
    context
}

/// The calendar for every meeting document in the corpus, with suggestions for upcoming agendas.
pub async fn calendar(
    repository: &DuckDbRepository,
    today: NaiveDate,
) -> Result<MeetingCalendar, AppError> {
    let loaded = load_sources(repository).await?;
    let mut limitations = Vec::new();
    let context = match_context(repository, &loaded, &mut limitations).await;
    let sources = &loaded.sources;
    let mut parsed = parse_calendar(sources, today)?;
    let mut reasons = Vec::with_capacity(parsed.items.len());
    let mut evaluator: Option<Evaluator> = None;
    let mut figures_unavailable = false;
    for item in &parsed.items {
        let source = &sources[item.source];
        let mut found = item_reasons(source, item, &context)?;
        let claims = extract_claims(&source.text[item.range.clone()]);
        if !claims.is_empty() && evaluator.is_none() && !figures_unavailable {
            match Evaluator::load(repository).await {
                Ok(loaded) => evaluator = Some(loaded),
                Err(error) => {
                    figures_unavailable = true;
                    limitations.push(format!(
                        "Stated figures were not checked; the imported tables could not be read: {error}"
                    ));
                }
            }
        }
        if let Some(evaluator) = evaluator.as_mut() {
            for claim in &claims {
                let evaluation = match evaluator
                    .evaluate(repository, &source.portal_key, claim)
                    .await
                {
                    Ok(evaluation) => evaluation,
                    Err(error) => {
                        limitations.push(format!(
                            "{} {}: a stated figure was not checked: {error}",
                            source.file_name, item.label
                        ));
                        continue;
                    }
                };
                // A figure with no mapped table to compare against is not checkable.
                if matches!(
                    evaluation,
                    ClaimEvaluation::Unreconciled {
                        reason: UnreconciledReason::NoMappedDataset,
                        ..
                    }
                ) {
                    continue;
                }
                found.push(figure_reason(source, item, claim, &evaluation)?);
            }
        }
        reasons.push(found);
    }
    assemble(&mut parsed, sources, reasons);
    order(&mut parsed.meetings);
    limitations.extend(parsed.limitations);
    limitations.push(format!(
        "Covers at most {SOURCE_LIMIT} public agenda, minutes and video documents, {TEXT_CHARACTERS} characters each. A body is recognized only from its heading or file name ({}); a date comes from a weekday-and-full-date heading, or, when the heading carries no date, is inferred from the file name or description. Items are recognized only by explicit \"Item N:\" headings. Suggestions are phrase matches, not assessments of the item.",
        BODIES.iter().map(|(label, _)| *label).collect::<Vec<_>>().join(", ")
    ));
    Ok(MeetingCalendar {
        today: today.format("%Y-%m-%d").to_string(),
        meetings: parsed.meetings,
        limitations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curly_and_straight_apostrophes_make_the_same_token_at_the_same_offsets() {
        let text = "The MAYOR’S office, the Mayor's ’note";
        let words = tokens(text)
            .into_iter()
            .map(|t| (t.text, t.start, t.end))
            .collect::<Vec<_>>();
        assert_eq!(
            words,
            vec![
                ("the".into(), 0, 3),
                ("mayor's".into(), 4, 13),
                ("office".into(), 14, 20),
                ("the".into(), 22, 25),
                ("mayor's".into(), 26, 33),
                ("note".into(), 37, 41),
            ]
        );
    }

    const AGENDA: &str = "CITY OF SAN DIEGO COUNCIL AGENDA {WEEKDAY}, {DATE}, AT 2:00 PM \
        Deadline for Distribution: Comments received by 8:00 AM on the day of the meeting will be distributed to the City Council. \
        • You can submit comments for agenda items via the webform at www.sandiego.gov/comment. \
        Item 601: Approval of Council Minutes. Proposed Actions: Approval of Council Minutes for May 7, 2026. \
        Item 605: First Amendment to the Riverbed Encampment Resolution Funding Agreement with the Housing Authority of the County. Proposed Actions: Resolution authorizing expenditure of $3,666,572. \
        Item 606: Honorary Street Name. Proposed Actions: Resolution designating a street. \
        NON-AGENDA PUBLIC COMMENT";

    fn source(id: i64, file_name: &str, text: String) -> MeetingSource {
        MeetingSource {
            portal_key: "city".into(),
            jurisdiction: "City of San Diego".into(),
            document_id: id,
            file_name: file_name.into(),
            description: String::new(),
            content_type: "application/pdf".into(),
            source_url: Some(format!("https://www.sandiego.gov/{file_name}")),
            text,
        }
    }

    fn agenda(id: i64, date: NaiveDate) -> MeetingSource {
        let text = AGENDA
            .replace("{WEEKDAY}", &date.format("%A").to_string().to_uppercase())
            .replace(
                "{DATE}",
                &date.format("%B %-d, %Y").to_string().to_uppercase(),
            );
        source(id, &format!("agenda-{id}.pdf"), text)
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 30).unwrap()
    }

    #[test]
    fn today_and_tomorrow_are_upcoming_and_yesterday_is_past() {
        let day = chrono::Duration::days(1);
        let sources = [
            agenda(1, today() - day),
            agenda(2, today()),
            agenda(3, today() + day),
        ];
        let parsed = parse_calendar(&sources, today()).unwrap();
        let timing = |date: NaiveDate| {
            parsed
                .meetings
                .iter()
                .find(|m| m.date.as_deref() == Some(&date.format("%Y-%m-%d").to_string()))
                .unwrap()
                .timing
        };
        assert_eq!(timing(today() - day), MeetingTiming::Past);
        assert_eq!(timing(today()), MeetingTiming::Upcoming);
        assert_eq!(timing(today() + day), MeetingTiming::Upcoming);
        // Items come only from upcoming agendas, and never the unbounded last item.
        let item_sources = parsed
            .items
            .iter()
            .map(|i| i.source)
            .collect::<BTreeSet<_>>();
        assert_eq!(item_sources, BTreeSet::from([1, 2]));
        assert!(parsed.items.iter().all(|i| i.label != "Item 606"));
    }

    #[test]
    fn undated_and_conflicting_headings_are_not_placed_on_a_day() {
        let undated = source(
            4,
            "council-agenda.pdf",
            "CITY OF SAN DIEGO COUNCIL AGENDA Item 1: Budget. Item 2: Parks.".into(),
        );
        let conflicting = source(
            5,
            "council-agenda-2.pdf",
            "CITY COUNCIL AGENDA Monday, September 28, 2026 and Tuesday, September 29, 2026".into(),
        );
        let parsed = parse_calendar(&[undated, conflicting], today()).unwrap();
        assert_eq!(parsed.meetings.len(), 2);
        for meeting in &parsed.meetings {
            assert_eq!(meeting.timing, MeetingTiming::DateNotRecorded);
            assert!(meeting.date.is_none() && meeting.date_citation.is_none());
            assert!(meeting.date_note.is_some());
        }
        assert!(parsed.items.is_empty());
    }

    #[test]
    fn minutes_link_by_body_and_heading_date_and_a_second_date_unlinks_them() {
        let meeting = today() + chrono::Duration::days(3);
        let heading = meeting.format("%A, %B %-d, %Y");
        let minutes = source(
            7,
            "minutes.pdf",
            format!("MINUTES OF THE CITY COUNCIL {heading}"),
        );
        let approving = source(
            8,
            "minutes-2.pdf",
            format!("MINUTES OF THE CITY COUNCIL {heading} Approval of minutes for Monday, May 11, 2026."),
        );
        let agenda_source = agenda(6, meeting);
        let parsed = parse_calendar(&[agenda_source.clone(), minutes, approving], today()).unwrap();
        assert_eq!(parsed.meetings.len(), 2);
        let linked = parsed
            .meetings
            .iter()
            .find(|m| m.timing == MeetingTiming::Upcoming)
            .unwrap();
        let roles = linked.documents.iter().map(|d| d.role).collect::<Vec<_>>();
        assert_eq!(
            roles,
            vec![MeetingDocumentRole::Agenda, MeetingDocumentRole::Minutes]
        );
        assert_eq!(linked.body, "City Council");
        let unlinked = parsed
            .meetings
            .iter()
            .find(|m| m.timing == MeetingTiming::DateNotRecorded)
            .unwrap();
        assert_eq!(unlinked.documents[0].document_id, 8);
        let citation = &linked.comment_deadlines[0];
        assert!(citation
            .quote
            .as_deref()
            .unwrap()
            .contains("received by 8:00 AM"));
        let start = citation.character_start.unwrap() as usize;
        let end = citation.character_end.unwrap() as usize;
        assert_eq!(
            agenda_source
                .text
                .chars()
                .skip(start)
                .take(end - start)
                .collect::<String>(),
            *citation.quote.as_ref().unwrap()
        );
    }

    fn context() -> MatchContext {
        MatchContext {
            watched: vec![WatchedName {
                name: "Housing Authority".into(),
                kind: SuggestionReasonKind::WatchedAgency,
                source: "your monitoring coverage expectations".into(),
            }],
            subjects: vec![FindingSubjectName {
                finding_id: "f1".into(),
                name: "Housing Authority".into(),
                kind_label: "POLICY_VIOLATION_DETECTED".into(),
                legal_claim: true,
            }],
            gaps: vec![EvidenceGap {
                detector: "financial",
                key: "financial:amendments".into(),
                route: "Spending was reconciled".into(),
                blocked: "No amendment register was retrieved".into(),
                role: GapRole::Amendments,
                request: "Provide amendments".into(),
            }],
        }
    }

    #[test]
    fn reasons_cite_the_agenda_and_the_draft_asserts_neither_intent_nor_illegality() {
        let sources = [agenda(8, today())];
        let mut parsed = parse_calendar(&sources, today()).unwrap();
        let reasons = parsed
            .items
            .iter()
            .map(|item| item_reasons(&sources[item.source], item, &context()).unwrap())
            .collect::<Vec<_>>();
        assert!(reasons[0].is_empty(), "Item 601 matches nothing");
        let kinds = reasons[1].iter().map(|r| r.kind).collect::<BTreeSet<_>>();
        assert_eq!(
            kinds,
            BTreeSet::from([
                SuggestionReasonKind::InvestigationTopic,
                SuggestionReasonKind::WatchedAgency,
                SuggestionReasonKind::FindingSubject,
                SuggestionReasonKind::EvidenceGap,
            ])
        );
        let finding = reasons[1]
            .iter()
            .find(|r| r.kind == SuggestionReasonKind::FindingSubject)
            .unwrap();
        assert_eq!(finding.basis, SuggestionBasis::Counted);
        for reason in &reasons[1] {
            assert!(sources[0]
                .text
                .contains(reason.citation.quote.as_deref().unwrap()));
            assert!(reason
                .citation
                .quote
                .as_deref()
                .unwrap()
                .contains(&reason.matched));
        }
        assemble(&mut parsed, &sources, reasons);
        let suggestion = &parsed.meetings[0].suggestions[0];
        assert_eq!(suggestion.agenda_item, "Item 605");
        let draft = &suggestion.comment.text;
        let lower = draft.to_lowercase();
        for forbidden in [
            "illegal", "unlawful", "violat", "intent", "fraud", "corrupt",
        ] {
            assert!(
                !lower.contains(forbidden),
                "draft says {forbidden}: {draft}"
            );
        }
        assert!(draft.contains("received by 8:00 AM"));
        assert!(suggestion.comment.deadline.is_some());
        assert!(draft.contains("Counted:") && draft.contains("Stated:"));
    }

    #[test]
    fn an_unrecorded_deadline_says_so() {
        let date = today();
        let mut undocumented = agenda(9, date);
        undocumented.text = undocumented
            .text
            .replace("Deadline for Distribution: Comments received by 8:00 AM on the day of the meeting will be distributed to the City Council.", "")
            .replace("• You can submit comments for agenda items via the webform at www.sandiego.gov/comment.", "");
        let sources = [undocumented];
        let mut parsed = parse_calendar(&sources, today()).unwrap();
        assert!(parsed.meetings[0].comment_deadlines.is_empty());
        let reasons = parsed
            .items
            .iter()
            .map(|item| item_reasons(&sources[item.source], item, &context()).unwrap())
            .collect();
        assemble(&mut parsed, &sources, reasons);
        let comment = &parsed.meetings[0].suggestions[0].comment;
        assert!(comment.deadline.is_none());
        assert!(comment.text.contains("Comment deadline: not recorded"));
        assert!(comment.text.contains("How to comment: not recorded"));
    }

    #[test]
    fn a_compared_figure_is_counted_and_cited_at_its_exact_offsets() {
        let mut dated = agenda(10, today());
        dated.text = dated.text.replace(
            "Item 606:",
            "The adopted budget of $1.2 billion for FY 2023 was approved for the Général fund. Item 606:",
        );
        let sources = [dated];
        let parsed = parse_calendar(&sources, today()).unwrap();
        let item = parsed.items.iter().find(|i| i.label == "Item 605").unwrap();
        let claim = extract_claims(&sources[0].text[item.range.clone()])
            .into_iter()
            .find(|c| c.source_text.contains("$1.2 billion"))
            .expect("budget claim");
        let evaluation = ClaimEvaluation::Mismatch {
            query_ids: vec!["q1".into()],
            claimed: "$1.2B".into(),
            observed: "$1.32B".into(),
            difference: "$120M".into(),
            band: "±$50M".into(),
            bound_scope: "FY2023".into(),
            fund_explicit: false,
            relative_basis_points: 900,
        };
        let reason = figure_reason(&sources[0], item, &claim, &evaluation).unwrap();
        assert_eq!(reason.basis, SuggestionBasis::Counted);
        let start = reason.citation.character_start.unwrap() as usize;
        let end = reason.citation.character_end.unwrap() as usize;
        assert_eq!(
            sources[0]
                .text
                .chars()
                .skip(start)
                .take(end - start)
                .collect::<String>(),
            *reason.citation.quote.as_ref().unwrap()
        );
        assert!(reason
            .citation
            .quote
            .as_deref()
            .unwrap()
            .contains("$1.2 billion"));
        let draft = draft_comment(
            &parsed.meetings[0],
            &item.label,
            &item.title,
            &item.citation,
            &[reason],
        );
        assert!(draft.text.contains("$1.32B") && draft.text.contains("Which figure is current"));
    }

    #[test]
    fn a_newer_agenda_supersedes_an_older_copy_without_duplicating_passages() {
        let date = today() + chrono::Duration::days(2);
        let sources = [agenda(12, date), agenda(11, date)];
        let parsed = parse_calendar(&sources, today()).unwrap();
        assert_eq!(parsed.meetings.len(), 1);
        assert_eq!(parsed.meetings[0].documents.len(), 2);
        assert!(parsed.items.iter().all(|item| item.source == 0));
        assert_eq!(parsed.meetings[0].comment_deadlines.len(), 1);
    }

    #[test]
    fn the_retained_council_agenda_yields_a_cited_upcoming_meeting() {
        let fixture = source(
            2,
            "9-28-26-special.pdf",
            include_str!("../../tests/fixtures/council-agenda-2026-09-28.txt").into(),
        );
        let day_before = NaiveDate::from_ymd_opt(2026, 9, 27).unwrap();
        let sources = [fixture];
        let mut parsed = parse_calendar(&sources, day_before).unwrap();
        let meeting = &parsed.meetings[0];
        assert_eq!(meeting.timing, MeetingTiming::Upcoming);
        assert_eq!(meeting.body, "City Council");
        assert_eq!(meeting.date.as_deref(), Some("2026-09-28"));
        assert!(meeting.comment_deadlines.iter().any(|c| c
            .quote
            .as_deref()
            .unwrap()
            .contains("received by 8:00 AM")));
        let reasons = parsed
            .items
            .iter()
            .map(|item| item_reasons(&sources[0], item, &MatchContext::default()).unwrap())
            .collect();
        assemble(&mut parsed, &sources, reasons);
        let item = parsed.meetings[0]
            .suggestions
            .iter()
            .find(|s| s.agenda_item == "Item 605")
            .unwrap();
        assert!(item
            .reasons
            .iter()
            .any(|r| r.kind == SuggestionReasonKind::InvestigationTopic));
        let start = item.item_citation.character_start.unwrap() as usize;
        let end = item.item_citation.character_end.unwrap() as usize;
        assert_eq!(
            sources[0]
                .text
                .chars()
                .skip(start)
                .take(end - start)
                .collect::<String>(),
            *item.item_citation.quote.as_ref().unwrap()
        );
        for label in ["Item 602", "Item 605"] {
            let item = parsed.items.iter().find(|i| i.label == label).unwrap();
            assert!(
                !extract_claims(&sources[0].text[item.range.clone()]).is_empty(),
                "{label} should yield at least one extracted dollar-figure claim"
            );
        }
    }

    #[tokio::test]
    async fn calendar_reads_ingested_documents_and_skips_non_meeting_records() {
        let temp = tempfile::tempdir().unwrap();
        let repository = DuckDbRepository::new(temp.path().join("core.db")).unwrap();
        repository
            .run_migrations(include_str!("../../migrations/0001_initial_schema.sql"))
            .await
            .unwrap();
        let conn = repository.analytics_connection().await.unwrap();
        conn.execute("INSERT INTO portals(subdomain,display_name,base_url) VALUES ('city','City of San Diego','https://www.sandiego.gov')",[]).unwrap();
        conn.execute("INSERT INTO requests(id,portal_subdomain,title,status,created_at,updated_at) VALUES (1,'city','Meetings','closed','2026-09-27','2026-09-27')",[]).unwrap();
        let upcoming = agenda(2, today() + chrono::Duration::days(1));
        for (id, name, text, visibility) in [
            (2, "agenda.pdf", upcoming.text.as_str(), "public"),
            (
                3,
                "memo.pdf",
                "A memo about the city council budget.",
                "public",
            ),
            (
                4,
                "private-agenda.pdf",
                upcoming.text.as_str(),
                "staff_only",
            ),
        ] {
            conn.execute("INSERT INTO documents(id,request_id,portal_subdomain,file_name,visibility,created_at,updated_at,extraction_status) VALUES (?1,1,'city',?2,?3,'2026-09-27','2026-09-27','SEARCHABLE')", duckdb::params![id, name, visibility]).unwrap();
            conn.execute("INSERT INTO documents_fts(document_id,portal_subdomain,file_name,extracted_text) VALUES (?1,'city',?2,?3)", duckdb::params![id, name, text]).unwrap();
        }
        conn.execute("INSERT INTO documents(id,request_id,portal_subdomain,file_name,description,content_type,visibility,created_at,updated_at) VALUES (5,1,'city','council-video.mp4','City Council meeting video','video/mp4','public','2026-09-27','2026-09-27')",[]).unwrap();
        drop(conn);
        let calendar = calendar(&repository, today()).await.unwrap();
        assert_eq!(calendar.meetings.len(), 2);
        let first = &calendar.meetings[0];
        assert_eq!(first.timing, MeetingTiming::Upcoming);
        assert_eq!(first.documents.len(), 1);
        assert!(first
            .suggestions
            .iter()
            .any(|s| s.agenda_item == "Item 605"));
        let video = &calendar.meetings[1];
        assert_eq!(video.timing, MeetingTiming::DateNotRecorded);
        assert_eq!(video.documents[0].role, MeetingDocumentRole::Video);
        assert!(video.body_citation.character_start.is_none());
    }
}
