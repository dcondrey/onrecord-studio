//! Reading a notification email the user chose, and suggesting timeline events from its text.
//!
//! A suggestion is a cue phrase found in one sentence. It is not an event: it carries the email's
//! Date header as a candidate date and waits for the user. Cues are word-token phrase lists
//! (`analysis::phrase_match`), not patterns over raw text. Quoted reply lines (starting with `>`)
//! are skipped because they repeat earlier messages, not this one.

use crate::analysis::phrase_match::{has_phrase, words, Phrases};
use crate::error::AppError;
use crate::integrity::sha256_hex;
use crate::parsers::eml::{normalize_message_id, parse_eml, MAXIMUM_EML_BYTES};
use crate::parsers::source::{inspect_regular_file, read_regular_file};
use crate::parsers::{extract_msg_layers, MsgExtractionContext};
use crate::types::tracker::{
    EmailEvidence, TimelineEventKind, MAXIMUM_MATCHED_SENTENCE_BYTES,
    MAXIMUM_SUBJECT_EXCERPT_CHARACTERS,
};
use crate::types::DocumentMetadata;
use chrono::DateTime;
use std::path::{Path, PathBuf};

/// Emails are notifications, not archives. `.msg` files carry attachments the tracker never opens,
/// but the whole file is still read into memory to parse it.
pub const MAXIMUM_EMAIL_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAXIMUM_SUGGESTIONS_PER_EMAIL: usize = 12;
const CONTEXT_BYTES: usize = 160;

const ACKNOWLEDGED: Phrases = &[
    &["received"],
    &["acknowledge"],
    &["acknowledged"],
    &["acknowledges"],
    &["acknowledging"],
];
const EXTENSION: Phrases = &[
    &["additional", "14", "days"],
    &["additional", "fourteen", "days"],
    &["extension"],
    &["extend"],
    &["extended"],
];
const DETERMINATION: Phrases = &[
    &["determination"],
    &["denied"],
    &["records", "are", "available"],
    &["responsive", "records"],
];
const CLOSED: Phrases = &[&["closed"]];

const CUES: [(TimelineEventKind, Phrases); 4] = [
    (TimelineEventKind::Acknowledged, ACKNOWLEDGED),
    (TimelineEventKind::ExtensionNotice, EXTENSION),
    (TimelineEventKind::Determination, DETERMINATION),
    (TimelineEventKind::Closed, CLOSED),
];

/// What the tracker keeps of one read email while suggestions are made. The body is used here and
/// dropped; only bounded excerpts leave this module.
#[derive(Debug, Clone)]
pub struct ReadEmail {
    pub file_name: String,
    /// Normalized with `parsers::eml::normalize_message_id`, the same for `.eml` and `.msg`. The
    /// hash normalizes again, so a caller cannot get a different identity by bracket or case.
    pub message_id: Option<String>,
    pub subject: Option<String>,
    /// RFC 3339. From a `.msg` this is UTC (the format keeps no offset); from an `.eml` it keeps
    /// the sender's offset. Never a guess: absent or unparseable is `None`.
    pub email_date: Option<String>,
    pub body_text: String,
    pub notes: Vec<String>,
}

impl ReadEmail {
    /// SHA-256 of the normalized Message-ID. `None` when the email has none: an empty string is
    /// never hashed into an identity.
    pub fn message_id_hash(&self) -> Option<String> {
        self.message_id
            .as_deref()
            .and_then(normalize_message_id)
            .map(|id| sha256_hex(id.as_bytes()))
    }

    pub fn subject_excerpt(&self) -> Option<String> {
        self.subject
            .as_deref()
            .map(collapse)
            .map(|subject| {
                subject
                    .chars()
                    .take(MAXIMUM_SUBJECT_EXCERPT_CHARACTERS)
                    .collect::<String>()
            })
            .map(|subject| subject.trim().to_string())
            .filter(|subject| !subject.is_empty())
    }
}

pub struct SuggestionSet {
    pub suggestions: Vec<(TimelineEventKind, EmailEvidence)>,
    pub notes: Vec<String>,
}

fn collapse(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Byte ranges `[start, end)` of sentences in one whitespace-collapsed line. A sentence ends at
/// `.`, `!` or `?` followed by a space or the end of the line.
fn sentence_ranges(line: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut characters = line.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        let ends = matches!(character, '.' | '!' | '?')
            && characters
                .peek()
                .is_none_or(|(_, next)| next.is_whitespace());
        if ends {
            let end = index + character.len_utf8();
            if end > start {
                ranges.push((start, end));
            }
            start = end;
            while let Some((next_index, next)) = characters.peek().copied() {
                if next.is_whitespace() {
                    start = next_index + next.len_utf8();
                    characters.next();
                } else {
                    break;
                }
            }
        }
    }
    if start < line.len() {
        ranges.push((start, line.len()));
    }
    ranges
}

pub fn suggest_events(email: &ReadEmail, message_id_hash: &str) -> SuggestionSet {
    let mut suggestions: Vec<(TimelineEventKind, EmailEvidence)> = Vec::new();
    let mut notes = Vec::new();
    let mut skipped_long = 0usize;
    let mut capped = false;
    let subject_excerpt = email.subject_excerpt();
    'lines: for raw in email.body_text.lines() {
        if raw.trim_start().starts_with('>') {
            continue;
        }
        let line = collapse(raw);
        for (start, end) in sentence_ranges(&line) {
            let sentence = &line[start..end];
            let tokens = words(sentence);
            let kinds: Vec<TimelineEventKind> = CUES
                .iter()
                .filter(|(_, phrases)| has_phrase(&tokens, phrases))
                .map(|(kind, _)| *kind)
                .collect();
            if kinds.is_empty() {
                continue;
            }
            if sentence.len() > MAXIMUM_MATCHED_SENTENCE_BYTES {
                skipped_long += 1;
                continue;
            }
            // 2 * CONTEXT_BYTES + the sentence cap + boundary rounding stays under the excerpt cap.
            let excerpt_start = line.floor_char_boundary(start.saturating_sub(CONTEXT_BYTES));
            let excerpt_end = line.ceil_char_boundary(end + CONTEXT_BYTES);
            for kind in kinds {
                if suggestions.iter().any(|(known, evidence)| {
                    *known == kind && evidence.matched_sentence == sentence
                }) {
                    continue;
                }
                if suggestions.len() >= MAXIMUM_SUGGESTIONS_PER_EMAIL {
                    capped = true;
                    break 'lines;
                }
                suggestions.push((
                    kind,
                    EmailEvidence {
                        message_id_hash: message_id_hash.to_string(),
                        subject_excerpt: subject_excerpt.clone(),
                        email_date: email.email_date.clone(),
                        excerpt: line[excerpt_start..excerpt_end].to_string(),
                        sentence_start: start - excerpt_start,
                        sentence_end: end - excerpt_start,
                        matched_sentence: sentence.to_string(),
                    },
                ));
            }
        }
    }
    if skipped_long > 0 {
        notes.push(format!(
            "{skipped_long} matching sentence(s) were longer than {MAXIMUM_MATCHED_SENTENCE_BYTES} bytes and were not suggested."
        ));
    }
    if capped {
        notes.push(format!(
            "Only the first {MAXIMUM_SUGGESTIONS_PER_EMAIL} suggestions from this email were kept."
        ));
    }
    SuggestionSet { suggestions, notes }
}

/// A path the user chose, checked before it is read: a regular, non-symlink `.eml` or `.msg`
/// file. The symlink test runs on the path as given, because canonicalizing first would resolve
/// the link and hide it.
pub fn validate_email_path(path: &str) -> Result<(PathBuf, EmailFormat), AppError> {
    let invalid =
        |reason: &str| AppError::DataParsing(format!("TRACKER_EMAIL_PATH_INVALID: {reason}"));
    let path = path.trim();
    if path.is_empty() || path.chars().any(char::is_control) {
        return Err(invalid("the path is empty or contains control characters"));
    }
    let given = Path::new(path);
    let format = match given
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("eml") => EmailFormat::Eml,
        Some("msg") => EmailFormat::Msg,
        _ => return Err(invalid("only .eml and .msg files can be imported")),
    };
    inspect_regular_file(given, "EMAIL", MAXIMUM_EMAIL_FILE_BYTES)?;
    let canonical = given
        .canonicalize()
        .map_err(|error| invalid(&format!("the path could not be resolved: {error}")))?;
    Ok((canonical, format))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmailFormat {
    Eml,
    Msg,
}

fn rfc3339_or_none(value: Option<&str>) -> Option<String> {
    value
        .and_then(|value| DateTime::parse_from_rfc3339(value.trim()).ok())
        .map(|date| date.to_rfc3339())
}

pub async fn read_email_file(path: &str) -> Result<ReadEmail, AppError> {
    let (canonical, format) = validate_email_path(path)?;
    let file_name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    match format {
        EmailFormat::Eml => {
            let source = canonical.clone();
            let parsed = crate::blocking::spawn("Email reader", move || {
                let bytes = read_regular_file(&source, "EML", MAXIMUM_EML_BYTES)?;
                parse_eml(&bytes)
            })
            .await??;
            Ok(ReadEmail {
                file_name,
                message_id: parsed.message_id,
                subject: parsed.subject,
                email_date: rfc3339_or_none(parsed.date.as_deref()),
                body_text: parsed.body_text,
                notes: parsed.notes,
            })
        }
        EmailFormat::Msg => {
            let placeholder = DocumentMetadata {
                id: 0,
                file_name: None,
                file_size: None,
                description: None,
                content_type: None,
                download_url: None,
                created_at: String::new(),
                updated_at: String::new(),
                visibility: "private".to_string(),
                local_path: None,
                extracted_text_sha256: None,
                was_ocr_processed: false,
            };
            // Attachments are not collected, so nothing is written under the cache directory.
            let cache = std::env::temp_dir();
            let payload = extract_msg_layers(
                &canonical,
                MsgExtractionContext {
                    source_document: &placeholder,
                    cache_directory: &cache,
                    collect_attachments: false,
                },
            )
            .await?;
            Ok(ReadEmail {
                file_name,
                message_id: payload
                    .metadata
                    .message_id
                    .as_deref()
                    .and_then(normalize_message_id),
                subject: payload.metadata.subject,
                email_date: rfc3339_or_none(payload.metadata.sent_at.as_deref()),
                body_text: payload.body_text,
                notes: vec![
                    "The date of a .msg file is read in UTC; the sender's offset is not kept."
                        .to_string(),
                ],
            })
        }
    }
}
