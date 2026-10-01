use crate::error::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PiiKind {
    SocialSecurityNumber,
    TelephoneNumber,
    PrivateStreetAddress,
    PaymentCardNumber,
    DateOfBirth,
    GovernmentIdentifier,
    MedicalIdentifier,
    BankAccountNumber,
}

impl PiiKind {
    const fn replacement(self) -> &'static str {
        match self {
            Self::SocialSecurityNumber => "[MASKED_SSN]",
            Self::TelephoneNumber => "[MASKED_PHONE]",
            Self::PrivateStreetAddress => "[MASKED_PRIVATE_ADDRESS]",
            Self::PaymentCardNumber => "[MASKED_PAYMENT_CARD]",
            Self::DateOfBirth => "[MASKED_DATE_OF_BIRTH]",
            Self::GovernmentIdentifier => "[MASKED_GOVERNMENT_ID]",
            Self::MedicalIdentifier => "[MASKED_MEDICAL_ID]",
            Self::BankAccountNumber => "[MASKED_BANK_ACCOUNT]",
        }
    }

    const fn priority(self) -> u8 {
        match self {
            Self::SocialSecurityNumber => 3,
            Self::TelephoneNumber => 2,
            Self::PrivateStreetAddress => 1,
            // A labelled value or a checksum-valid card is more specific than any shape match.
            Self::PaymentCardNumber
            | Self::DateOfBirth
            | Self::GovernmentIdentifier
            | Self::MedicalIdentifier
            | Self::BankAccountNumber => 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PiiFinding {
    pub kind: PiiKind,
    pub byte_start: usize,
    pub byte_end: usize,
    pub character_start: usize,
    pub character_end: usize,
    pub matched_character_count: usize,
    pub confidence_basis_points: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PiiMaskResult {
    pub masked_text: String,
    pub findings: Vec<PiiFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Candidate {
    kind: PiiKind,
    start: usize,
    end: usize,
    confidence_basis_points: u16,
}

#[derive(Debug, Clone)]
struct Token<'a> {
    text: &'a str,
    normalized: String,
    start: usize,
    end: usize,
}

pub fn mask_pii(text: &str) -> Result<PiiMaskResult, AppError> {
    let mut candidates = Vec::new();
    candidates.extend(detect_ssns(text));
    candidates.extend(detect_phone_numbers(text));
    candidates.extend(detect_street_addresses(text));
    candidates.extend(detect_payment_cards(text));
    candidates.extend(detect_labelled_identifiers(text));
    let selected = select_non_overlapping_candidates(candidates);
    let mut findings = Vec::with_capacity(selected.len());
    let mut copied_until = 0;
    let mut character_cursor = 0;

    for candidate in selected {
        if candidate.start < copied_until
            || candidate.end > text.len()
            || !text.is_char_boundary(candidate.start)
            || !text.is_char_boundary(candidate.end)
        {
            return Err(AppError::Internal(
                "PII detector produced an invalid text boundary".to_string(),
            ));
        }
        let evidence = &text[candidate.start..candidate.end];
        // Candidates are ordered and non-overlapping, so advancing from the preceding byte avoids
        // rescanning the prefix for every finding. Across the loop each source character is
        // visited at most once.
        character_cursor += text[copied_until..candidate.start].chars().count();
        let character_start = character_cursor;
        let matched_character_count = evidence.chars().count();
        findings.push(PiiFinding {
            kind: candidate.kind,
            byte_start: candidate.start,
            byte_end: candidate.end,
            character_start,
            character_end: character_start + matched_character_count,
            matched_character_count,
            confidence_basis_points: candidate.confidence_basis_points,
        });
        character_cursor += matched_character_count;
        copied_until = candidate.end;
    }
    Ok(mask_byte_range(text, findings, 0, text.len()))
}

/// Masks a character range after detecting PII against the complete supplied context.
///
/// INVARIANT(pii): Detection runs over the whole `text`, never over the visible slice alone. A
/// finding that straddles either boundary is emitted as a full mask inside the slice, so paging
/// through a document cannot reveal a value that would have been masked when read whole. Passing
/// a pre-sliced `text` defeats this and is the mistake this function exists to prevent.
///
/// LIMIT(pii): "Masked" is not "clean". The detectors are structural (separator layout, NANP
/// digit rules, street-suffix tokens), so a value written in an unusual format is not found.
/// Treat the output as defense in depth behind human review, never as a release gate.
///
/// INVARIANT(pii): The returned findings keep offsets into the ORIGINAL `text`, while
/// `masked_text` is a rebased slice of it. Indexing `masked_text` with `character_start` is
/// therefore wrong; the offsets exist to correlate a finding back to the stored document.
pub fn mask_pii_character_range(
    text: &str,
    character_start: usize,
    character_limit: usize,
) -> Result<PiiMaskResult, AppError> {
    let total_character_count = text.chars().count();
    if character_start > total_character_count {
        return Err(AppError::DataParsing(format!(
            "PII mask range starts at {character_start}, beyond {total_character_count} characters"
        )));
    }
    let character_end = character_start
        .checked_add(character_limit)
        .ok_or_else(|| AppError::DataParsing("PII mask range overflowed".to_string()))?
        .min(total_character_count);
    let byte_start = byte_offset_for_character(text, character_start);
    let byte_end = byte_offset_for_character(text, character_end);
    let detected = mask_pii(text)?;
    Ok(mask_byte_range(
        text,
        detected.findings,
        byte_start,
        byte_end,
    ))
}

fn mask_byte_range(
    text: &str,
    mut findings: Vec<PiiFinding>,
    byte_start: usize,
    byte_end: usize,
) -> PiiMaskResult {
    findings.retain(|finding| finding.byte_start < byte_end && finding.byte_end > byte_start);
    let mut masked_text = String::with_capacity(byte_end.saturating_sub(byte_start));
    let mut copied_until = byte_start;
    for finding in &findings {
        let visible_finding_start = finding.byte_start.max(byte_start);
        if visible_finding_start > copied_until {
            masked_text.push_str(&text[copied_until..visible_finding_start]);
        }
        masked_text.push_str(finding.kind.replacement());
        copied_until = finding.byte_end.min(byte_end);
    }
    if copied_until < byte_end {
        masked_text.push_str(&text[copied_until..byte_end]);
    }
    PiiMaskResult {
        masked_text,
        findings,
    }
}

fn byte_offset_for_character(text: &str, character_offset: usize) -> usize {
    text.char_indices()
        .nth(character_offset)
        .map_or(text.len(), |(byte_offset, _)| byte_offset)
}

pub fn mask_json_pii(value: &serde_json::Value) -> Result<serde_json::Value, AppError> {
    // Structural keys are deliberately preserved for schema compatibility. Callers handling
    // user-defined keys must normalize or mask those keys before treating the object as shareable.
    match value {
        serde_json::Value::String(text) => {
            Ok(serde_json::Value::String(mask_pii(text)?.masked_text))
        }
        serde_json::Value::Array(values) => values
            .iter()
            .map(mask_json_pii)
            .collect::<Result<Vec<_>, _>>()
            .map(serde_json::Value::Array),
        serde_json::Value::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), mask_json_pii(value)?)))
            .collect::<Result<serde_json::Map<_, _>, AppError>>()
            .map(serde_json::Value::Object),
        other => Ok(other.clone()),
    }
}

fn detect_ssns(text: &str) -> Vec<Candidate> {
    lexical_chunks(text)
        .into_iter()
        .filter_map(|(chunk_start, chunk_end)| {
            // A hyphen glued to the value ("SSN-123-45-6789") is a separator, not part of it.
            let chunk = &text[chunk_start..chunk_end];
            let candidate = chunk.trim_matches('-');
            let start = chunk_start + (chunk.len() - chunk.trim_start_matches('-').len());
            let end = start + candidate.len();
            let digits = candidate
                .bytes()
                .filter(u8::is_ascii_digit)
                .collect::<Vec<_>>();
            let separator_layout = candidate.len() == 11
                && candidate
                    .as_bytes()
                    .get(3)
                    .is_some_and(|value| *value == b'-')
                && candidate
                    .as_bytes()
                    .get(6)
                    .is_some_and(|value| *value == b'-');
            if digits.len() != 9 || !separator_layout || !valid_ssn_digits(&digits) {
                return None;
            }
            Some(Candidate {
                kind: PiiKind::SocialSecurityNumber,
                start,
                end,
                confidence_basis_points: 9_900,
            })
        })
        .collect()
}

fn valid_ssn_digits(digits: &[u8]) -> bool {
    let area = decimal_value(&digits[0..3]);
    let group = decimal_value(&digits[3..5]);
    let serial = decimal_value(&digits[5..9]);
    area != 0 && area != 666 && area < 900 && group != 0 && serial != 0
}

fn detect_phone_numbers(text: &str) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    let mut segment_start = None;
    for (index, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        // Letters must never extend a phone segment: a run of digits/punctuation bounded only by
        // non-digit, non-phone-punctuation characters (crucially including ordinary letters) is
        // what keeps two unrelated numbers in the same sentence ("Item 605 ... budget of 4500")
        // from being merged into one digit run and read as a single 10-digit phone-shaped window.
        // Previously allowing letters here let the segment span whole sentences of prose.
        let allowed =
            character.is_ascii_digit() || matches!(character, '+' | '(' | ')' | '-' | '.' | ' ');
        if allowed {
            segment_start.get_or_insert(index);
            continue;
        }
        if let Some(start) = segment_start.take() {
            candidates.extend(phone_candidates_in_segment(text, start, index));
        }
    }
    candidates
}

fn phone_candidates_in_segment(
    text: &str,
    segment_start: usize,
    segment_end: usize,
) -> Vec<Candidate> {
    let segment = &text[segment_start..segment_end];
    let digit_offsets = segment
        .char_indices()
        .filter(|(_, character)| character.is_ascii_digit())
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    for digit_count in [11_usize, 10_usize] {
        if digit_offsets.len() < digit_count {
            continue;
        }
        for window in digit_offsets.windows(digit_count) {
            let first = window[0];
            let last = window[digit_count - 1] + 1;
            let mut start = first;
            while start > 0 {
                let previous = segment[..start].chars().next_back();
                if previous.is_some_and(|value| matches!(value, '+' | '(')) {
                    start -= previous.map(char::len_utf8).unwrap_or(0);
                } else {
                    break;
                }
            }
            let mut end = last;
            if segment[end..].starts_with(')') {
                end += 1;
            }
            let raw = &segment[start..end];
            let digits = raw
                .bytes()
                .filter(u8::is_ascii_digit)
                .map(|digit| digit - b'0')
                .collect::<Vec<_>>();
            if digits.len() != digit_count || !valid_nanp_digits(&digits) {
                continue;
            }
            let has_format_signal = raw
                .bytes()
                .any(|byte| matches!(byte, b'+' | b'(' | b')' | b'-' | b'.' | b' '));
            if !has_format_signal {
                continue;
            }
            candidates.push(Candidate {
                kind: PiiKind::TelephoneNumber,
                start: segment_start + start,
                end: segment_start + end,
                confidence_basis_points: if raw.contains('(') || raw.contains("tel") {
                    9_700
                } else {
                    9_300
                },
            });
        }
    }
    candidates
}

fn valid_nanp_digits(digits: &[u8]) -> bool {
    let national = match digits {
        [1, remainder @ ..] if remainder.len() == 10 => remainder,
        values if values.len() == 10 => values,
        _ => return false,
    };
    national[0] >= 2 && national[3] >= 2
}

fn detect_street_addresses(text: &str) -> Vec<Candidate> {
    let tokens = tokens(text);
    let directions = ["n", "s", "e", "w", "ne", "nw", "se", "sw"];
    let units = ["apt", "apartment", "unit", "suite"];
    let mut candidates = Vec::new();

    for start_index in 0..tokens.len() {
        let number = tokens[start_index].normalized.trim_end_matches('-');
        if number.is_empty()
            || number.len() > 7
            || !number.chars().all(|character| character.is_ascii_digit())
        {
            continue;
        }
        let mut name_start = start_index + 1;
        if tokens
            .get(name_start)
            .is_some_and(|token| directions.contains(&token.normalized.as_str()))
        {
            name_start += 1;
        }
        for suffix_index in name_start + 1..=(name_start + 6).min(tokens.len().saturating_sub(1)) {
            if !is_street_suffix(tokens[suffix_index].normalized.as_str()) {
                continue;
            }
            if tokens[name_start..suffix_index].iter().any(|token| {
                token.normalized.is_empty()
                    || !token
                        .normalized
                        .chars()
                        .any(|character| character.is_alphanumeric())
            }) {
                continue;
            }
            let mut end_index = suffix_index;
            if let Some(next) = tokens.get(end_index + 1) {
                if next.text.starts_with('#') && next.normalized.is_empty() {
                    if tokens.get(end_index + 2).is_some() {
                        end_index += 2;
                    }
                } else if next.text.starts_with('#') {
                    end_index += 1;
                } else if units.contains(&next.normalized.as_str())
                    && tokens.get(end_index + 2).is_some()
                {
                    end_index += 2;
                }
            }
            candidates.push(Candidate {
                kind: PiiKind::PrivateStreetAddress,
                start: tokens[start_index].start,
                end: tokens[end_index].end,
                confidence_basis_points: 9_100,
            });
            break;
        }
    }
    candidates
}

/// 13 to 19 digits in one run, separated at most by single spaces or hyphens, passing the Luhn
/// check and starting with a major-network prefix. The checksum is what separates a card from the
/// long numeric identifiers public records are full of.
fn detect_payment_cards(text: &str) -> Vec<Candidate> {
    let bytes = text.as_bytes();
    let mut candidates = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() || (index > 0 && bytes[index - 1].is_ascii_alphanumeric())
        {
            index += 1;
            continue;
        }
        let start = index;
        let mut digits = Vec::with_capacity(19);
        let mut end = index;
        let mut cursor = index;
        while cursor < bytes.len() && digits.len() <= 19 {
            match bytes[cursor] {
                byte if byte.is_ascii_digit() => {
                    digits.push(byte - b'0');
                    cursor += 1;
                    end = cursor;
                }
                b' ' | b'-' if bytes.get(cursor + 1).is_some_and(u8::is_ascii_digit) => {
                    cursor += 1;
                }
                _ => break,
            }
        }
        let bounded = bytes
            .get(end)
            .is_none_or(|byte| !byte.is_ascii_alphanumeric());
        if bounded
            && (13..=19).contains(&digits.len())
            && matches!(digits[0], 3..=6)
            && luhn_valid(&digits)
        {
            candidates.push(Candidate {
                kind: PiiKind::PaymentCardNumber,
                start,
                end,
                confidence_basis_points: 9_800,
            });
        }
        index = end.max(index + 1);
    }
    candidates
}

fn luhn_valid(digits: &[u8]) -> bool {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(position, &digit)| {
            let digit = u32::from(digit);
            if position % 2 == 1 {
                let doubled = digit * 2;
                if doubled > 9 {
                    doubled - 9
                } else {
                    doubled
                }
            } else {
                digit
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// A value is masked only when a label naming it sits directly before it. Generic labels such as
/// "account number" are deliberately absent: budget and procurement records publish ledger
/// account numbers, and masking those would hide the public finance the product exists to read.
const IDENTIFIER_LABELS: &[(&[&str], PiiKind)] = &[
    (&["dob"], PiiKind::DateOfBirth),
    (&["d", "o", "b"], PiiKind::DateOfBirth),
    (&["date", "of", "birth"], PiiKind::DateOfBirth),
    (&["birth", "date"], PiiKind::DateOfBirth),
    (&["birthdate"], PiiKind::DateOfBirth),
    (&["born"], PiiKind::DateOfBirth),
    (&["driver's", "license"], PiiKind::GovernmentIdentifier),
    (&["drivers", "license"], PiiKind::GovernmentIdentifier),
    (&["driver", "license"], PiiKind::GovernmentIdentifier),
    (&["driver's", "licence"], PiiKind::GovernmentIdentifier),
    (&["dl"], PiiKind::GovernmentIdentifier),
    (&["cdl"], PiiKind::GovernmentIdentifier),
    (&["passport"], PiiKind::GovernmentIdentifier),
    (&["mrn"], PiiKind::MedicalIdentifier),
    (&["medical", "record"], PiiKind::MedicalIdentifier),
    (&["patient", "id"], PiiKind::MedicalIdentifier),
    (&["medi-cal"], PiiKind::MedicalIdentifier),
    (&["medicare"], PiiKind::MedicalIdentifier),
    (&["bank", "account"], PiiKind::BankAccountNumber),
    (&["checking", "account"], PiiKind::BankAccountNumber),
    (&["savings", "account"], PiiKind::BankAccountNumber),
    (&["routing", "number"], PiiKind::BankAccountNumber),
    (&["iban"], PiiKind::BankAccountNumber),
];

const LABEL_FILLERS: &[&str] = &["no", "number", "num", "id", "is", "on", "of", "#"];

fn detect_labelled_identifiers(text: &str) -> Vec<Candidate> {
    let tokens = tokens(text);
    let mut candidates = Vec::new();
    for start_index in 0..tokens.len() {
        for (label, kind) in IDENTIFIER_LABELS {
            let Some(label_tokens) = tokens.get(start_index..start_index + label.len()) else {
                continue;
            };
            if !label_tokens
                .iter()
                .zip(label.iter())
                .all(|(token, word)| token.normalized == *word)
            {
                continue;
            }
            let mut value_index = start_index + label.len();
            let mut fillers = 0;
            while let Some(token) = tokens.get(value_index) {
                if fillers < 3 && LABEL_FILLERS.contains(&token.normalized.as_str()) {
                    value_index += 1;
                    fillers += 1;
                } else {
                    break;
                }
            }
            let Some(value) = tokens.get(value_index) else {
                continue;
            };
            let previous_end = tokens[value_index - 1].end;
            if !text[previous_end..value.start].chars().all(|character| {
                character.is_whitespace() || matches!(character, ':' | '.' | '#' | '-')
            }) {
                continue;
            }
            let span = if *kind == PiiKind::DateOfBirth {
                date_end(text, value.start).map(|end| (value.start, end))
            } else {
                identifier_value_span(value)
            };
            if let Some((start, end)) = span {
                candidates.push(Candidate {
                    kind: *kind,
                    start,
                    end,
                    confidence_basis_points: 9_500,
                });
            }
        }
    }
    candidates
}

/// An identifier is one token of 4 to 34 letters, digits and hyphens holding at least 4 digits,
/// less a glued `#` ("#X1234567").
fn identifier_value_span(token: &Token<'_>) -> Option<(usize, usize)> {
    let start = token.start + (token.text.len() - token.text.trim_start_matches('#').len());
    let value = token
        .text
        .trim_start_matches('#')
        .trim_end_matches(['-', '\'']);
    let digit_count = value.bytes().filter(u8::is_ascii_digit).count();
    ((4..=34).contains(&value.len())
        && digit_count >= 4
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'))
    .then_some((start, start + value.len()))
}

/// The end of a calendar date starting at `start`: `3/14/1990`, `1990-03-14`, `03.14.90`, or
/// `March 14, 1990`.
fn date_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let digits_at = |position: usize, minimum: usize, maximum: usize| -> Option<usize> {
        let count = bytes[position..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        (minimum..=maximum)
            .contains(&count)
            .then_some(position + count)
    };
    if let Some(first_end) = digits_at(start, 1, 4) {
        let separator = *bytes.get(first_end)?;
        if !matches!(separator, b'/' | b'-' | b'.') {
            return None;
        }
        let second_end = digits_at(first_end + 1, 1, 2)?;
        if bytes.get(second_end) != Some(&separator) {
            return None;
        }
        return digits_at(second_end + 1, 2, 4);
    }
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let word_end = start
        + bytes[start..]
            .iter()
            .take_while(|byte| byte.is_ascii_alphabetic())
            .count();
    let word = text[start..word_end].to_ascii_lowercase();
    let word = word.trim_end_matches('.');
    if word.len() < 3 || !MONTHS.iter().any(|month| month.starts_with(word)) {
        return None;
    }
    let mut cursor = word_end;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
    }
    cursor += bytes[cursor..]
        .iter()
        .take_while(|byte| **byte == b' ')
        .count();
    let day_end = digits_at(cursor, 1, 2)?;
    cursor = day_end;
    if bytes.get(cursor) == Some(&b',') {
        cursor += 1;
    }
    cursor += bytes[cursor..]
        .iter()
        .take_while(|byte| **byte == b' ')
        .count();
    digits_at(cursor, 4, 4)
}

fn select_non_overlapping_candidates(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| right.kind.priority().cmp(&left.kind.priority()))
            .then_with(|| (right.end - right.start).cmp(&(left.end - left.start)))
    });
    candidates.dedup_by_key(|candidate| (candidate.start, candidate.end, candidate.kind));
    let mut selected: Vec<Candidate> = Vec::new();
    for candidate in candidates {
        if let Some(previous) = selected.last_mut() {
            if candidate.start < previous.end {
                if candidate.kind == previous.kind {
                    // Sliding phone windows over one digit run overlap without either being the
                    // real number ("2023 619-555-0198"). Keeping only the earliest window
                    // leaves the tail of the real number in cleartext, so the span is unioned.
                    previous.end = previous.end.max(candidate.end);
                } else if candidate.kind.priority() > previous.kind.priority() {
                    // The higher-priority kind wins the label either way. Whether it also keeps
                    // the wider span depends on where it sits: a phone-shaped unit number ends
                    // its address ("125 Ocean View Drive Unit 619-555-0198"), so replacing the
                    // address span with the phone span left the house number and street in
                    // cleartext; the whole span is kept. But an address candidate that continues
                    // past the phone ("(619) 555-0198, address 440 Cedar Avenue") only exists
                    // because the address detector accepts arbitrary interior tokens, so keeping
                    // its span would swallow the genuine address that follows and merge two
                    // findings into one. `previous.start <= candidate.start` by sort order.
                    let terminates_previous = candidate.end >= previous.end;
                    if terminates_previous {
                        previous.kind = candidate.kind;
                        previous.confidence_basis_points = candidate.confidence_basis_points;
                        previous.end = candidate.end;
                    } else {
                        *previous = candidate;
                    }
                }
                // A later-starting lower-priority candidate is dropped, not unioned: the address
                // detector accepts arbitrary tokens between a house number and a street suffix,
                // so "(619) 555-0198, address 440 Cedar Avenue" yields a spurious address
                // candidate starting inside the phone number. Extending the phone span by it
                // would swallow the genuine "440 Cedar Avenue" finding that follows.
                continue;
            }
        }
        selected.push(candidate);
    }
    selected.sort_by_key(|candidate| candidate.start);
    selected
}

fn lexical_chunks(text: &str) -> Vec<(usize, usize)> {
    let mut chunks = Vec::new();
    let mut start = None;
    for (index, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if character.is_ascii_digit() || character == '-' {
            start.get_or_insert(index);
        } else if let Some(chunk_start) = start.take() {
            chunks.push((chunk_start, index));
        }
    }
    chunks
}

fn tokens(text: &str) -> Vec<Token<'_>> {
    let mut result = Vec::new();
    let mut start = None;
    for (index, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if character.is_alphanumeric() || matches!(character, '#' | '\'' | '-') {
            start.get_or_insert(index);
        } else if let Some(token_start) = start.take() {
            let value = &text[token_start..index];
            result.push(Token {
                text: value,
                normalized: value
                    .trim_matches(|character: char| matches!(character, '#' | '\'' | '-'))
                    .to_lowercase(),
                start: token_start,
                end: index,
            });
        }
    }
    result
}

fn is_street_suffix(value: &str) -> bool {
    matches!(
        value,
        "street"
            | "st"
            | "avenue"
            | "ave"
            | "road"
            | "rd"
            | "boulevard"
            | "blvd"
            | "lane"
            | "ln"
            | "drive"
            | "dr"
            | "court"
            | "ct"
            | "way"
            | "place"
            | "pl"
            | "terrace"
            | "trail"
            | "highway"
            | "hwy"
            | "circle"
            | "cir"
            | "parkway"
            | "pkwy"
    )
}

fn decimal_value(digits: &[u8]) -> u32 {
    digits.iter().fold(0_u32, |value, digit| {
        value
            .saturating_mul(10)
            .saturating_add((digit - b'0') as u32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_labelled_identifiers_and_checksum_valid_cards_only() {
        let sensitive = [
            ("Card 4111 1111 1111 1111 on file.", "4111 1111 1111 1111"),
            ("DOB: 03/14/1990.", "03/14/1990"),
            (
                "Date of birth March 14, 1990, per intake.",
                "March 14, 1990",
            ),
            ("Driver's License No. B1234567 was shown.", "B1234567"),
            ("Passport #X12345678 expired.", "X12345678"),
            ("MRN 00451234 admitted.", "00451234"),
            ("Bank account number 000123456789 closed.", "000123456789"),
        ];
        for (input, value) in sensitive {
            let masked = mask_pii(input).unwrap().masked_text;
            assert!(!masked.contains(value), "{input} -> {masked}");
        }
        for public in [
            // Ledger account numbers are public finance, and a Luhn failure is not a card.
            "Account number 500128 funds overtime.",
            "Invoice 4111 1111 1111 1112 was paid.",
            "The program was born out of a 2019 audit.",
            "Passport services moved downtown.",
        ] {
            assert_eq!(mask_pii(public).unwrap().masked_text, public);
        }
    }

    #[test]
    fn masks_high_confidence_pii_with_structural_detectors() {
        let input = "SSN 123-45-6789, phone (619) 555-0198, address 440 Cedar Avenue Apt 2.";
        let result = mask_pii(input).unwrap();
        assert_eq!(result.findings.len(), 3);
        assert!(!result.masked_text.contains("123-45-6789"));
        assert!(!result.masked_text.contains("555-0198"));
        assert!(!result.masked_text.contains("440 Cedar"));
    }

    /// The phone detector's segment used to include ordinary letters, so an entire sentence of
    /// prose became one digit run: the tail of "605", all of "2023", and the head of "4500"
    /// formed a spurious 10-digit NANP-shaped window spanning three unrelated numbers, and
    /// ordinary case-number/year/budget prose was masked as a phone number that was never there.
    #[test]
    fn does_not_fabricate_a_phone_number_from_unrelated_digits_in_the_same_sentence() {
        let input = "Item 605 approved 2023 budget of 4500";
        let result = mask_pii(input).unwrap();
        assert!(
            result.findings.is_empty(),
            "no real phone number exists in this text; got {:?}",
            result.findings
        );
        assert_eq!(result.masked_text, input);
    }

    /// The same over-permissive segmentation let a spurious cross-word digit run ("4412" +
    /// "619") start earlier than the real phone number and win the overlap tie-break by position,
    /// masking a bogus span while leaking the real number's last digit in cleartext.
    #[test]
    fn detects_the_real_phone_number_and_not_a_cross_word_digit_run_that_starts_earlier() {
        let input = "Room 4412 call 619-555-0198";
        let result = mask_pii(input).unwrap();
        assert_eq!(result.findings.len(), 1, "findings: {:?}", result.findings);
        assert_eq!(
            &input[result.findings[0].byte_start..result.findings[0].byte_end],
            "619-555-0198"
        );
        assert!(
            !result.masked_text.contains("0198"),
            "the real phone number must be masked"
        );
        assert!(
            result.masked_text.contains("Room 4412"),
            "the unrelated room number must survive untouched"
        );
    }

    #[test]
    fn masks_findings_that_cross_visible_chunk_boundaries() {
        let input = "prefix 123-45-6789 suffix";

        let left = mask_pii_character_range(input, 0, 12).unwrap();
        assert_eq!(left.findings.len(), 1);
        assert_eq!(left.masked_text, "prefix [MASKED_SSN]");
        assert!(!left.masked_text.contains("123-45"));

        let right = mask_pii_character_range(input, 12, 14).unwrap();
        assert_eq!(right.findings.len(), 1);
        assert_eq!(right.masked_text, "[MASKED_SSN] suffix");
        assert!(!right.masked_text.contains("6789"));
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;

    // INVARIANT(pii): findings keep offsets into the ORIGINAL text while `masked_text` is a
    // rebased slice. A consumer indexing the masked string with these offsets gets garbage, so
    // this pins which coordinate system the numbers belong to.
    #[test]
    fn finding_offsets_address_the_original_text_not_the_masked_output() {
        let input = "call 619-555-0198 today";
        let result = mask_pii(input).expect("mask");
        let finding = result.findings.first().expect("one phone finding");

        assert_eq!(&input[finding.byte_start..finding.byte_end], "619-555-0198");
        let characters: String = input
            .chars()
            .skip(finding.character_start)
            .take(finding.matched_character_count)
            .collect();
        assert_eq!(characters, "619-555-0198");
        assert_ne!(
            result.masked_text.len(),
            input.len(),
            "the masked text is rebased, so original offsets cannot index it",
        );
    }

    // INVARIANT(pii): byte and character offsets must stay distinct and correct once the text
    // contains multi-byte characters. An ASCII-only fixture cannot catch a swap of the two.
    #[test]
    fn byte_and_character_offsets_diverge_correctly_on_multibyte_text() {
        let input = "Café note — SSN 123-45-6789 follows";
        let result = mask_pii(input).expect("mask");
        let finding = result.findings.first().expect("one SSN finding");

        assert_eq!(&input[finding.byte_start..finding.byte_end], "123-45-6789");
        assert!(
            finding.byte_start > finding.character_start,
            "multi-byte prefix must push the byte offset ahead of the character offset: \
             byte {} vs character {}",
            finding.byte_start,
            finding.character_start,
        );
        let characters: String = input
            .chars()
            .skip(finding.character_start)
            .take(finding.matched_character_count)
            .collect();
        assert_eq!(characters, "123-45-6789");
        assert!(!result.masked_text.contains("123-45-6789"));
        assert!(result.masked_text.contains("Café"));
    }

    // SECURITY(pii): paging must not become a disclosure oracle. A value straddling a chunk edge
    // is masked whole in every window that shows any part of it.
    #[test]
    fn a_value_straddling_a_chunk_edge_is_masked_in_both_windows() {
        let input = "prefix 123-45-6789 suffix";
        let split = 12;

        let left = mask_pii_character_range(input, 0, split).expect("left window");
        let right = mask_pii_character_range(input, split, 64).expect("right window");

        for fragment in ["123", "45-67", "6789", "123-45"] {
            assert!(
                !left.masked_text.contains(fragment),
                "left leaked {fragment}"
            );
            assert!(
                !right.masked_text.contains(fragment),
                "right leaked {fragment}"
            );
        }
        assert!(left.masked_text.contains("[MASKED_SSN]"));
        assert!(right.masked_text.contains("[MASKED_SSN]"));
    }

    // Detection runs over the whole document, so a window that contains none of a value still
    // reports nothing, while every window overlapping it reports the finding.
    #[test]
    fn windows_report_only_the_findings_they_overlap() {
        let input = "aaaa 123-45-6789 bbbb";
        let head = mask_pii_character_range(input, 0, 4).expect("head window");
        assert!(head.findings.is_empty(), "{:?}", head.findings);
        assert_eq!(head.masked_text, "aaaa");

        let covering = mask_pii_character_range(input, 0, 21).expect("full window");
        assert_eq!(covering.findings.len(), 1);
    }

    #[test]
    fn a_range_beyond_the_document_is_an_error_not_a_silent_empty_result() {
        let error = mask_pii_character_range("short", 99, 10)
            .expect_err("a start past the end must be rejected");
        assert!(matches!(error, AppError::DataParsing(_)), "got {error:?}");
    }

    // SECURITY(pii): JSON masking must reach values at every depth, including inside arrays and
    // nested objects, or a structured export leaks what the flat masker would have caught.
    #[test]
    fn json_masking_reaches_values_at_every_depth() {
        let value = serde_json::json!({
            "note": "SSN 123-45-6789",
            "nested": { "phone": "call (619) 555-0198" },
            "list": ["ssn 456-78-9012", { "deep": "SSN 123-45-6789" }],
            "count": 42,
            "flag": true,
            "absent": null,
        });

        let masked = mask_json_pii(&value).expect("mask json");
        let rendered = masked.to_string();

        assert!(!rendered.contains("123-45-6789"), "{rendered}");
        assert!(!rendered.contains("456-78-9012"), "{rendered}");
        assert!(!rendered.contains("555-0198"), "{rendered}");
        assert!(rendered.contains("[MASKED_SSN]"));
        // The array element uses a valid SSA area number; 900-999 is not a valid SSN area, so a
        // fixture in that range would be skipped by design rather than by a masking failure.
        // Non-string scalars pass through unchanged.
        assert_eq!(masked["count"], serde_json::json!(42));
        assert_eq!(masked["flag"], serde_json::json!(true));
        assert_eq!(masked["absent"], serde_json::Value::Null);
    }

    // LIMIT(pii): object KEYS are deliberately preserved for schema compatibility, so a caller
    // with user-defined keys must mask those itself. Pinned so the gap stays deliberate.
    #[test]
    fn json_masking_deliberately_preserves_object_keys() {
        let value = serde_json::json!({ "123-45-6789": "harmless" });
        let masked = mask_json_pii(&value).expect("mask json");
        assert!(
            masked.get("123-45-6789").is_some(),
            "keys are not masked by design"
        );
    }

    // SECURITY(pii): a spurious earlier phone window must not truncate the real number's mask.
    #[test]
    fn overlapping_phone_windows_mask_the_whole_number() {
        for input in ["Dated 2023 619-555-0198 end", "Room 12 (619) 555-0198."] {
            let result = mask_pii(input).expect("mask");
            assert!(
                !result.masked_text.contains("0198"),
                "{}",
                result.masked_text
            );
            assert!(
                !result.masked_text.contains("98."),
                "{}",
                result.masked_text
            );
            assert!(!result.masked_text.contains("8."), "{}", result.masked_text);
            assert_eq!(result.findings.len(), 1, "{:?}", result.findings);
        }
    }

    #[test]
    fn a_hyphen_glued_to_an_ssn_does_not_hide_it() {
        let input = "SSN-123-45-6789-";
        let result = mask_pii(input).expect("mask");
        assert_eq!(result.masked_text, "SSN-[MASKED_SSN]-");
        assert_eq!(
            &input[result.findings[0].byte_start..result.findings[0].byte_end],
            "123-45-6789"
        );
    }

    #[test]
    fn a_compact_unit_marker_is_part_of_the_masked_address() {
        let result = mask_pii("x 440 Cedar Avenue #4 y").expect("mask");
        assert_eq!(result.masked_text, "x [MASKED_PRIVATE_ADDRESS] y");
    }

    // SECURITY(pii): a higher-priority overlap used to replace a lower-priority finding's span
    // outright instead of unioning it, so whatever part of the wider finding the narrower one
    // didn't itself cover survived in cleartext. A street address whose unit number is
    // phone-shaped is exactly that: the address candidate spans the whole "125 ... 619-555-0198"
    // range, the phone candidate only spans "619-555-0198", and the phone wins on priority.
    // Before the fix this masked only the phone digits and left the house number and street name
    // ("125 Ocean View Drive Unit") in the clear.
    #[test]
    fn a_higher_priority_overlap_does_not_leave_the_rest_of_the_lower_priority_finding_unmasked() {
        let input = "125 Ocean View Drive Unit 619-555-0198 filed today";
        let result = mask_pii(input).expect("mask");
        assert!(
            !result.masked_text.contains("125"),
            "{}",
            result.masked_text
        );
        assert!(
            !result.masked_text.contains("Ocean View"),
            "{}",
            result.masked_text
        );
        assert!(
            !result.masked_text.contains("619-555-0198"),
            "{}",
            result.masked_text
        );
        assert_eq!(result.masked_text, "[MASKED_PHONE] filed today");
    }
}
