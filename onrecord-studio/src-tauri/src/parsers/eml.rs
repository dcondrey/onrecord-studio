//! `.eml` (RFC 5322 / MIME) reading for the request tracker.
//!
//! Reads headers and the readable text of a message and nothing else. Attachments are recognized
//! and skipped without being decoded, nested `message/rfc822` parts are not opened, and HTML is
//! reduced to its text nodes without loading any remote content or running any script.
//!
//! Charsets: UTF-8, US-ASCII, ISO-8859-1 and windows-1252 are decoded exactly. Any other charset
//! is read as UTF-8 with replacement characters and says so in `notes`; guessing a mapping would
//! put invented words into evidence. `encoding_rs` would cover the rest but is not a direct
//! dependency of this crate.

use crate::error::AppError;
use base64::engine::general_purpose::GeneralPurposeConfig;
use base64::engine::{DecodePaddingMode, GeneralPurpose};
use base64::{alphabet, Engine};
use chrono::DateTime;
use scraper::{node::Node, Html};

pub const MAXIMUM_EML_BYTES: u64 = 16 * 1024 * 1024;
const MAXIMUM_HEADER_BYTES: usize = 256 * 1024;
const MAXIMUM_MIME_DEPTH: usize = 8;
const MAXIMUM_MIME_PARTS: usize = 200;
/// Decoded message text kept for scanning. Longer text is cut at a character boundary and noted.
pub const MAXIMUM_TEXT_BYTES: usize = 1024 * 1024;

const LENIENT_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEml {
    /// Normalized: angle brackets removed, trimmed, ASCII-lowercased.
    pub message_id: Option<String>,
    pub subject: Option<String>,
    /// The Date header as RFC 3339 in the offset the sender wrote. `None` when absent or invalid.
    pub date: Option<String>,
    pub body_text: String,
    /// Things a reader should know about how the text was obtained.
    pub notes: Vec<String>,
}

/// The identity form used for both `.eml` and `.msg`: what sits inside the angle brackets, or the
/// bare trimmed value, lowercased. `None` for an empty id.
pub fn normalize_message_id(value: &str) -> Option<String> {
    let inner = match (value.find('<'), value.find('>')) {
        (Some(start), Some(end)) if end > start => &value[start + 1..end],
        _ => value,
    };
    let inner = inner.trim();
    (!inner.is_empty()).then(|| inner.to_ascii_lowercase())
}

pub fn parse_eml(bytes: &[u8]) -> Result<ParsedEml, AppError> {
    if bytes.len() as u64 > MAXIMUM_EML_BYTES {
        return Err(malformed(&format!(
            "the message is larger than {MAXIMUM_EML_BYTES} bytes"
        )));
    }
    if bytes.contains(&0) {
        return Err(malformed("the message contains a NUL byte"));
    }
    let (headers, body) = split_entity(bytes)?;
    let mut state = Walk::default();
    collect_text(&headers, body, 0, &mut state)?;

    let mut notes = state.notes;
    let body_text = if !state.plain.trim().is_empty() {
        state.plain
    } else if !state.html.trim().is_empty() {
        notes.push("The message has no plain-text part; its HTML text was used.".to_string());
        html_text(&state.html)
    } else {
        return Err(malformed("the message has no readable text part"));
    };
    let body_text = truncate_on_boundary(body_text, MAXIMUM_TEXT_BYTES, &mut notes);

    let subject = header(&headers, "subject")
        .map(|value| collapse_whitespace(&decode_encoded_words(value)))
        .filter(|value| !value.is_empty());
    let date = header(&headers, "date")
        .and_then(|value| DateTime::parse_from_rfc2822(value.trim()).ok())
        .map(|date| date.to_rfc3339());
    Ok(ParsedEml {
        message_id: header(&headers, "message-id").and_then(normalize_message_id),
        subject,
        date,
        body_text,
        notes,
    })
}

fn malformed(reason: &str) -> AppError {
    AppError::DataParsing(format!("EML_MALFORMED: {reason}"))
}

type Headers = Vec<(String, String)>;

fn header<'a>(headers: &'a Headers, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, value)| value.as_str())
}

/// Splits one entity into unfolded headers (names lowercased) and the body after the first blank
/// line. A body-less entity is valid; a header section without any header is not.
fn split_entity(bytes: &[u8]) -> Result<(Headers, &[u8]), AppError> {
    let mut headers: Headers = Vec::new();
    let mut position = 0;
    let mut first = true;
    loop {
        if position >= bytes.len() {
            return finish_headers(headers, &[]);
        }
        let end = bytes[position..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |offset| position + offset);
        let next = (end + 1).min(bytes.len());
        let line = bytes[position..end]
            .strip_suffix(b"\r")
            .unwrap_or(&bytes[position..end]);
        if position > MAXIMUM_HEADER_BYTES {
            return Err(malformed("the header section is too long or never ends"));
        }
        if line.is_empty() {
            return finish_headers(headers, &bytes[next..]);
        }
        let text = String::from_utf8_lossy(line);
        if line[0] == b' ' || line[0] == b'\t' {
            let Some((_, value)) = headers.last_mut() else {
                return Err(malformed("a header continuation has nothing to continue"));
            };
            value.push(' ');
            value.push_str(text.trim());
        } else if first && text.starts_with("From ") {
            // An mbox envelope line, not a header.
        } else {
            let Some((name, value)) = text.split_once(':') else {
                return Err(malformed("a header line has no colon"));
            };
            if name.is_empty() || name.contains(|c: char| c.is_whitespace() || c.is_control()) {
                return Err(malformed("a header name is not valid"));
            }
            headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
        }
        first = false;
        position = next;
    }
}

fn finish_headers(headers: Headers, body: &[u8]) -> Result<(Headers, &[u8]), AppError> {
    if headers.is_empty() {
        return Err(malformed("the message has no headers"));
    }
    Ok((headers, body))
}

#[derive(Default)]
struct Walk {
    plain: String,
    html: String,
    parts: usize,
    notes: Vec<String>,
}

struct ContentType {
    kind: String,
    parameters: Vec<(String, String)>,
}

impl ContentType {
    fn parse(value: &str) -> Self {
        let mut pieces = split_parameters(value).into_iter();
        let kind = pieces
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let parameters = pieces
            .filter_map(|piece| {
                let (name, value) = piece.split_once('=')?;
                let value = value.trim();
                let value = value
                    .strip_prefix('"')
                    .and_then(|rest| rest.strip_suffix('"'))
                    .unwrap_or(value);
                Some((name.trim().to_ascii_lowercase(), value.to_string()))
            })
            .collect();
        Self { kind, parameters }
    }

    fn parameter(&self, name: &str) -> Option<&str> {
        self.parameters
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Splits on `;` outside double quotes.
fn split_parameters(value: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in value.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                current.push(character);
            }
            ';' if !quoted => pieces.push(std::mem::take(&mut current)),
            _ => current.push(character),
        }
    }
    pieces.push(current);
    pieces
}

fn collect_text(
    headers: &Headers,
    body: &[u8],
    depth: usize,
    state: &mut Walk,
) -> Result<(), AppError> {
    if depth > MAXIMUM_MIME_DEPTH {
        return Err(malformed("MIME parts are nested too deeply"));
    }
    state.parts += 1;
    if state.parts > MAXIMUM_MIME_PARTS {
        return Err(malformed("the message has too many MIME parts"));
    }
    let content_type = ContentType::parse(header(headers, "content-type").unwrap_or("text/plain"));
    let disposition = header(headers, "content-disposition")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if disposition.starts_with("attachment") {
        return Ok(());
    }
    if content_type.kind.starts_with("multipart/") {
        let boundary = content_type
            .parameter("boundary")
            .filter(|boundary| !boundary.is_empty())
            .ok_or_else(|| malformed("a multipart entity has no boundary"))?;
        let parts = split_multipart(body, boundary);
        if parts.is_empty() {
            return Err(malformed("a multipart entity has no delimiter line"));
        }
        for part in parts {
            let (part_headers, part_body) = split_entity(part)?;
            collect_text(&part_headers, part_body, depth + 1, state)?;
        }
        return Ok(());
    }
    let is_plain = content_type.kind == "text/plain";
    let is_html = content_type.kind == "text/html";
    if !is_plain && !is_html {
        return Ok(());
    }
    let encoding = header(headers, "content-transfer-encoding")
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let decoded = match encoding.as_str() {
        "" | "7bit" | "8bit" | "binary" => body.to_vec(),
        "quoted-printable" => decode_quoted_printable(body),
        "base64" => match decode_base64(body) {
            Some(bytes) => bytes,
            None => {
                state
                    .notes
                    .push("A base64 text part was not valid base64 and was skipped.".to_string());
                return Ok(());
            }
        },
        other => {
            state.notes.push(format!(
                "A text part used the unsupported transfer encoding '{other}' and was skipped."
            ));
            return Ok(());
        }
    };
    let charset = content_type.parameter("charset").unwrap_or("us-ascii");
    let (text, note) = decode_charset(charset, &decoded);
    if let Some(note) = note {
        if !state.notes.contains(&note) {
            state.notes.push(note);
        }
    }
    let target = if is_html {
        &mut state.html
    } else {
        &mut state.plain
    };
    if !target.is_empty() {
        target.push_str("\n\n");
    }
    target.push_str(&text);
    // A hostile message must not grow the working text past a bounded multiple of what is kept.
    if target.len() > MAXIMUM_TEXT_BYTES * 2 {
        let mut notes = Vec::new();
        *target = truncate_on_boundary(std::mem::take(target), MAXIMUM_TEXT_BYTES * 2, &mut notes);
        state.notes.extend(notes);
    }
    Ok(())
}

/// The parts between delimiter lines, without the line break that precedes each delimiter.
fn split_multipart<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
    let delimiter = format!("--{boundary}");
    let delimiter = delimiter.as_bytes();
    let mut parts = Vec::new();
    let mut part_start: Option<usize> = None;
    let mut position = 0;
    while position <= body.len() {
        let end = body[position..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(body.len(), |offset| position + offset);
        let line = body[position..end]
            .strip_suffix(b"\r")
            .unwrap_or(&body[position..end]);
        if line.starts_with(delimiter) {
            let rest = &line[delimiter.len()..];
            let closing = rest.starts_with(b"--");
            let tail = if closing { &rest[2..] } else { rest };
            if tail.iter().all(|byte| *byte == b' ' || *byte == b'\t') {
                if let Some(start) = part_start.take() {
                    // The line break before a delimiter belongs to the delimiter.
                    let mut stop = position.max(start);
                    if stop > start && body[stop - 1] == b'\n' {
                        stop -= 1;
                        if stop > start && body[stop - 1] == b'\r' {
                            stop -= 1;
                        }
                    }
                    parts.push(&body[start..stop]);
                }
                if closing {
                    return parts;
                }
                part_start = Some((end + 1).min(body.len()));
            }
        }
        if end >= body.len() {
            break;
        }
        position = end + 1;
    }
    if let Some(start) = part_start {
        parts.push(&body[start..]);
    }
    parts
}

fn decode_quoted_printable(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let ends_with_newline = input.ends_with(b"\n");
    let mut lines: Vec<&[u8]> = input.split(|byte| *byte == b'\n').collect();
    if ends_with_newline {
        lines.pop();
    }
    let last = lines.len().saturating_sub(1);
    for (index, raw) in lines.into_iter().enumerate() {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        let (content, soft_break) = match line.strip_suffix(b"=") {
            Some(content) => (content, true),
            None => (trim_trailing_blanks(line), false),
        };
        let mut position = 0;
        while position < content.len() {
            let escape = (content[position] == b'=' && position + 2 < content.len())
                .then(|| {
                    Some(hex_value(content[position + 1])? * 16 + hex_value(content[position + 2])?)
                })
                .flatten();
            match escape {
                Some(byte) => {
                    output.push(byte);
                    position += 3;
                }
                None => {
                    output.push(content[position]);
                    position += 1;
                }
            }
        }
        if !soft_break && (index != last || ends_with_newline) {
            output.push(b'\n');
        }
    }
    output
}

fn trim_trailing_blanks(line: &[u8]) -> &[u8] {
    let end = line
        .iter()
        .rposition(|byte| *byte != b' ' && *byte != b'\t')
        .map_or(0, |index| index + 1);
    &line[..end]
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_base64(input: &[u8]) -> Option<Vec<u8>> {
    let compact: Vec<u8> = input
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    LENIENT_BASE64.decode(compact).ok()
}

/// The decoded text and, when the charset was not decoded exactly, a note saying so.
fn decode_charset(charset: &str, bytes: &[u8]) -> (String, Option<String>) {
    let name = charset.trim().to_ascii_lowercase();
    match name.as_str() {
        "utf-8" | "utf8" => match std::str::from_utf8(bytes) {
            Ok(text) => (text.to_string(), None),
            Err(_) => (
                String::from_utf8_lossy(bytes).into_owned(),
                Some("A part declared UTF-8 but held invalid bytes; they were replaced.".to_string()),
            ),
        },
        "us-ascii" | "ascii" | "ansi_x3.4-1968" => {
            if bytes.is_ascii() {
                (String::from_utf8_lossy(bytes).into_owned(), None)
            } else {
                (
                    String::from_utf8_lossy(bytes).into_owned(),
                    Some("A part declared US-ASCII but held other bytes; they were read as UTF-8 with replacement.".to_string()),
                )
            }
        }
        "iso-8859-1" | "latin1" | "iso_8859-1" | "l1" => {
            (bytes.iter().map(|byte| char::from(*byte)).collect(), None)
        }
        "windows-1252" | "cp1252" => (bytes.iter().map(|byte| windows_1252(*byte)).collect(), None),
        _ => (
            String::from_utf8_lossy(bytes).into_owned(),
            Some(format!(
                "A part used the charset '{}', which this reader does not decode; it was read as UTF-8 with replacement.",
                charset.trim()
            )),
        ),
    }
}

fn windows_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}',
        '\u{017D}', '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}',
        '\u{2013}', '\u{2014}', '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}',
        '\u{9D}', '\u{017E}', '\u{0178}',
    ];
    match byte {
        0x80..=0x9F => HIGH[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

/// RFC 2047 encoded-words. Whitespace between two adjacent encoded-words is dropped; an encoded
/// word that does not parse stays as written.
fn decode_encoded_words(value: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    let mut previous_was_encoded = false;
    while !rest.is_empty() {
        if let Some((decoded, consumed)) = parse_encoded_word(rest) {
            output.push_str(&decoded);
            rest = &rest[consumed..];
            previous_was_encoded = true;
            continue;
        }
        let whitespace_end = rest
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(rest.len());
        if whitespace_end > 0 {
            let after = &rest[whitespace_end..];
            if previous_was_encoded && parse_encoded_word(after).is_some() {
                rest = after;
            } else {
                output.push_str(&rest[..whitespace_end]);
                rest = after;
                previous_was_encoded = false;
            }
            continue;
        }
        let next = rest
            .char_indices()
            .find(|(index, c)| *index > 0 && (c.is_whitespace() || *c == '='))
            .map_or(rest.len(), |(index, _)| index);
        output.push_str(&rest[..next]);
        rest = &rest[next..];
        previous_was_encoded = false;
    }
    output
}

fn parse_encoded_word(text: &str) -> Option<(String, usize)> {
    let body = text.strip_prefix("=?")?;
    let charset_end = body.find('?')?;
    let charset = body[..charset_end].split('*').next()?;
    let after_charset = &body[charset_end + 1..];
    let encoding = after_charset.get(..1)?;
    let payload_start = after_charset.get(1..)?.strip_prefix('?')?;
    // The payload may itself begin with '=', so the closing "?=" is searched for only after the
    // third '?'.
    let payload_end = payload_start.find("?=")?;
    let payload = &payload_start[..payload_end];
    if charset.is_empty() || payload.contains(char::is_whitespace) {
        return None;
    }
    let bytes = match encoding {
        "B" | "b" => decode_base64(payload.as_bytes())?,
        "Q" | "q" => decode_q(payload),
        _ => return None,
    };
    let consumed = text.len() - payload_start[payload_end + 2..].len();
    Some((decode_charset(charset, &bytes).0, consumed))
}

fn decode_q(payload: &str) -> Vec<u8> {
    let bytes = payload.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'_' => {
                output.push(b' ');
                index += 1;
            }
            b'=' if index + 2 < bytes.len()
                && hex_value(bytes[index + 1]).is_some()
                && hex_value(bytes[index + 2]).is_some() =>
            {
                output.push(
                    hex_value(bytes[index + 1]).unwrap_or(0) * 16
                        + hex_value(bytes[index + 2]).unwrap_or(0),
                );
                index += 3;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    output
}

fn html_text(html: &str) -> String {
    let document = Html::parse_fragment(html);
    let mut lines: Vec<String> = Vec::new();
    for node in document.tree.root().descendants() {
        let Node::Text(text) = node.value() else {
            continue;
        };
        let inactive = node.ancestors().any(|ancestor| {
            matches!(
                ancestor.value(),
                Node::Element(element) if matches!(element.name(), "script" | "style" | "head")
            )
        });
        if inactive {
            continue;
        }
        let text = collapse_whitespace(text);
        if !text.is_empty() {
            lines.push(text);
        }
    }
    lines.join("\n")
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_on_boundary(mut text: String, limit: usize, notes: &mut Vec<String>) -> String {
    if text.len() <= limit {
        return text;
    }
    let mut cut = limit;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    notes.push(format!(
        "The message text was longer than {limit} bytes; only the first part was read."
    ));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eml(headers: &str, body: &str) -> Vec<u8> {
        format!("{headers}\r\n\r\n{body}").into_bytes()
    }

    const BASE_HEADERS: &str = "From: Records <records@example.gov>\r\n\
        Message-ID: <Ab.12@Example.GOV>\r\n\
        Date: Mon, 05 Jan 2026 09:30:00 -0800\r\n\
        Subject: =?UTF-8?B?UmU6IFJlcXVlc3Qg?= =?UTF-8?Q?=E2=80=9C26-114=E2=80=9D?=";

    #[test]
    fn multipart_quoted_printable_prefers_the_plain_part_and_skips_attachments() {
        let body = "preamble\r\n--b1\r\nContent-Type: multipart/alternative; boundary=\"b2\"\r\n\r\n\
            --b2\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n\
            We have received your request. Caf=C3=A9 is open =\r\nall day.\r\n\
            --b2\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>html only</p>\r\n--b2--\r\n\
            --b1\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=a.txt\r\n\r\nSECRET ATTACHMENT\r\n--b1--\r\n";
        let headers = format!(
            "{BASE_HEADERS}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b1"
        );
        let parsed = parse_eml(&eml(&headers, body)).unwrap();
        assert_eq!(
            parsed.body_text,
            "We have received your request. Café is open all day."
        );
        assert!(!parsed.body_text.contains("SECRET"));
        assert!(!parsed.body_text.contains("html only"));
        assert_eq!(parsed.message_id.as_deref(), Some("ab.12@example.gov"));
        assert_eq!(parsed.subject.as_deref(), Some("Re: Request “26-114”"));
        assert_eq!(parsed.date.as_deref(), Some("2026-01-05T09:30:00-08:00"));
    }

    #[test]
    fn base64_latin1_and_windows_1252_text_decode_exactly() {
        let encoded = LENIENT_BASE64.encode(b"Determination made \xe9\x92.");
        let headers = format!(
            "{BASE_HEADERS}\r\nContent-Type: text/plain; charset=windows-1252\r\nContent-Transfer-Encoding: base64"
        );
        let parsed = parse_eml(&eml(
            &headers,
            &format!("{}\r\n{}", &encoded[..8], &encoded[8..]),
        ))
        .unwrap();
        assert_eq!(parsed.body_text, "Determination made é\u{2019}.");
        let latin = format!("{BASE_HEADERS}\r\nContent-Type: text/plain; charset=iso-8859-1");
        let mut raw = format!("{latin}\r\n\r\ncaf").into_bytes();
        raw.push(0xE9);
        assert_eq!(parse_eml(&raw).unwrap().body_text, "café");
    }

    #[test]
    fn an_undecodable_charset_is_reported_not_guessed() {
        let headers = format!("{BASE_HEADERS}\r\nContent-Type: text/plain; charset=koi8-r");
        let parsed = parse_eml(&eml(&headers, "plain ascii")).unwrap();
        assert!(parsed.notes.iter().any(|note| note.contains("koi8-r")));
    }

    #[test]
    fn html_only_messages_yield_text_without_scripts() {
        let headers = format!("{BASE_HEADERS}\r\nContent-Type: text/html; charset=utf-8");
        let parsed = parse_eml(&eml(
            &headers,
            "<html><head><style>p{}</style></head><body><script>alert(1)</script><p>Closed today.</p><img src=\"http://tracker.example/x.gif\"></body></html>",
        ))
        .unwrap();
        assert_eq!(parsed.body_text, "Closed today.");
    }

    #[test]
    fn malformed_messages_are_errors() {
        assert!(parse_eml(b"").is_err());
        assert!(parse_eml(b"no colon here\r\n\r\nbody").is_err());
        assert!(parse_eml(b" continuation first\r\n\r\nbody").is_err());
        assert!(parse_eml(b"Subject: x\r\n\r\n\0nul").is_err());
        // Declared multipart without a boundary, and with no delimiter line in the body.
        let no_boundary = eml("Subject: x\r\nContent-Type: multipart/mixed", "body");
        assert!(parse_eml(&no_boundary).is_err());
        let no_delimiter = eml(
            "Subject: x\r\nContent-Type: multipart/mixed; boundary=zz",
            "body",
        );
        assert!(parse_eml(&no_delimiter).is_err());
        // Invalid base64 leaves nothing readable.
        let bad = eml(
            "Subject: x\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64",
            "***not base64***",
        );
        assert!(parse_eml(&bad).is_err());
        // A header block that never ends is refused, not scanned forever.
        let endless = "X-Filler: y\r\n".repeat(MAXIMUM_HEADER_BYTES / 8);
        assert!(parse_eml(endless.as_bytes()).is_err());
    }

    #[test]
    fn size_and_nesting_limits_hold_at_limit_and_one_past() {
        assert!(parse_eml(&vec![b'a'; MAXIMUM_EML_BYTES as usize + 1]).is_err());

        let nested = |levels: usize| {
            let mut body = "Content-Type: text/plain\r\n\r\ndeep text".to_string();
            for level in (0..levels).rev() {
                body = format!(
                    "Content-Type: multipart/mixed; boundary=b{level}\r\n\r\n--b{level}\r\n{body}\r\n--b{level}--\r\n"
                );
            }
            format!("Subject: x\r\n{body}").into_bytes()
        };
        // Depth counts entities below the root: the root multipart is depth 0.
        assert!(parse_eml(&nested(MAXIMUM_MIME_DEPTH)).is_ok());
        assert!(parse_eml(&nested(MAXIMUM_MIME_DEPTH + 1)).is_err());

        let long = "word ".repeat(MAXIMUM_TEXT_BYTES / 5 + 10);
        let parsed = parse_eml(&eml("Subject: x", &long)).unwrap();
        assert!(parsed.body_text.len() <= MAXIMUM_TEXT_BYTES);
        assert!(!parsed.notes.is_empty());
    }

    #[test]
    fn message_ids_normalize_the_same_way_for_brackets_and_bare_values() {
        assert_eq!(normalize_message_id(" <A@B.C> ").as_deref(), Some("a@b.c"));
        assert_eq!(normalize_message_id("A@B.C").as_deref(), Some("a@b.c"));
        assert_eq!(normalize_message_id("<>"), None);
        assert_eq!(normalize_message_id("  "), None);
    }
}
