use crate::error::AppError;
use crate::integrity::sha256_hex;
use crate::types::LegalAuthorityImport;
use chrono::NaiveDate;
use reqwest::Url;
use std::collections::HashSet;

const MAXIMUM_LEGAL_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAXIMUM_LEGAL_SOURCE_CHARACTERS: usize = 4_000_000;
const MAXIMUM_PROVISIONS: usize = 10_000;
const MAXIMUM_MANDATES: usize = 256;
const MAXIMUM_PHRASES_PER_MANDATE: usize = 128;
const MAXIMUM_PHRASE_CHARACTERS: usize = 256;

#[derive(Debug, Clone)]
pub struct ParsedLegalProvision {
    pub provision_id: String,
    pub ordinal: usize,
    pub heading_level: usize,
    pub heading_path: String,
    pub title: String,
    pub provision_text: String,
    pub token_count: usize,
    pub content_sha256: String,
}

#[derive(Debug, Clone)]
pub struct ParsedLegalAuthority {
    pub version_id: String,
    pub normalized_text: String,
    pub content_sha256: String,
    pub provisions: Vec<ParsedLegalProvision>,
}

struct SectionBuilder {
    heading_level: usize,
    heading_path: String,
    title: String,
    body: Vec<String>,
}

pub fn parse_legal_authority(
    input: &LegalAuthorityImport,
) -> Result<ParsedLegalAuthority, AppError> {
    validate_import(input)?;
    let normalized_text = normalize_source_text(&input.raw_text)?;
    let content_sha256 = sha256_hex(normalized_text.as_bytes());
    let version_digest =
        sha256_hex(format!("{}\0{}", input.statute_id.trim(), content_sha256).as_bytes());
    let version_id = format!("legal-version-{}", &version_digest[..32]);
    let sections = split_sections(&normalized_text, input.title.trim())?;
    let mut provisions = Vec::with_capacity(sections.len());
    for section in sections {
        let provision_text = section
            .body
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        if provision_text.is_empty() {
            continue;
        }
        let ordinal = provisions.len();
        let provision_digest = sha256_hex(
            format!(
                "{}\0{}\0{}\0{}",
                version_id, ordinal, section.heading_path, provision_text
            )
            .as_bytes(),
        );
        provisions.push(ParsedLegalProvision {
            provision_id: format!("legal-provision-{}", &provision_digest[..32]),
            ordinal,
            heading_level: section.heading_level,
            heading_path: section.heading_path,
            title: section.title,
            token_count: policy_tokens(&provision_text).len(),
            content_sha256: sha256_hex(provision_text.as_bytes()),
            provision_text,
        });
    }
    if provisions.is_empty() {
        return Err(AppError::DataParsing(
            "Legal source contains no substantive provisions".to_string(),
        ));
    }
    Ok(ParsedLegalAuthority {
        version_id,
        normalized_text,
        content_sha256,
        provisions,
    })
}

pub fn policy_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    for character in text.chars().chain(std::iter::once('\0')) {
        if character.is_alphanumeric() || matches!(character, '\'' | '’') {
            token.extend(character.to_lowercase());
        } else if !token.is_empty() {
            let trimmed = token.trim_matches(['\'', '’']);
            if !trimmed.is_empty() {
                tokens.push(stem_policy_token(trimmed));
            }
            token.clear();
        }
    }
    tokens
}

pub fn token_sequence_match_count(tokens: &[String], phrase_tokens: &[String]) -> usize {
    if phrase_tokens.is_empty() || phrase_tokens.len() > tokens.len() {
        return 0;
    }
    tokens
        .windows(phrase_tokens.len())
        .filter(|window| *window == phrase_tokens)
        .count()
}

fn validate_import(input: &LegalAuthorityImport) -> Result<(), AppError> {
    validate_identifier(&input.statute_id, "Statute identifier", 128)?;
    validate_required_text(&input.jurisdiction, "Jurisdiction", 256)?;
    validate_required_text(&input.statute_citation, "Statute citation", 512)?;
    validate_required_text(&input.title, "Legal authority title", 512)?;
    if input.raw_text.len() > MAXIMUM_LEGAL_SOURCE_BYTES
        || input.raw_text.chars().count() > MAXIMUM_LEGAL_SOURCE_CHARACTERS
    {
        return Err(AppError::DataParsing(format!(
            "Legal source exceeds the {MAXIMUM_LEGAL_SOURCE_BYTES}-byte or {MAXIMUM_LEGAL_SOURCE_CHARACTERS}-character limit"
        )));
    }
    if input.raw_text.trim().is_empty() {
        return Err(AppError::DataParsing(
            "Legal source text must not be empty".to_string(),
        ));
    }
    if input
        .mandated_deadline_days
        .is_some_and(|days| days > 36_500)
    {
        return Err(AppError::DataParsing(
            "Mandated deadline cannot exceed 36500 days".to_string(),
        ));
    }
    validate_optional_date(input.effective_from.as_deref(), "Effective-from date")?;
    validate_optional_date(input.effective_to.as_deref(), "Effective-to date")?;
    if let (Some(start), Some(end)) = (&input.effective_from, &input.effective_to) {
        let start = NaiveDate::parse_from_str(start.trim(), "%Y-%m-%d").map_err(|error| {
            AppError::DataParsing(format!("Invalid effective-from date: {error}"))
        })?;
        let end = NaiveDate::parse_from_str(end.trim(), "%Y-%m-%d").map_err(|error| {
            AppError::DataParsing(format!("Invalid effective-to date: {error}"))
        })?;
        if end < start {
            return Err(AppError::DataParsing(
                "Effective-to date cannot precede effective-from date".to_string(),
            ));
        }
    }
    if let Some(source_uri) = input
        .canonical_source_uri
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let url = Url::parse(source_uri.trim()).map_err(|error| {
            AppError::DataParsing(format!("Invalid canonical legal source URL: {error}"))
        })?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(AppError::DataParsing(
                "Canonical legal source URL must be an authenticated-host-free HTTPS URL"
                    .to_string(),
            ));
        }
    }
    if input.mandates.len() > MAXIMUM_MANDATES {
        return Err(AppError::DataParsing(format!(
            "A legal authority may define at most {MAXIMUM_MANDATES} mandates"
        )));
    }
    let mut mandate_ids = HashSet::new();
    for mandate in &input.mandates {
        validate_identifier(&mandate.mandate_id, "Mandate identifier", 128)?;
        if !mandate_ids.insert(mandate.mandate_id.trim().to_ascii_lowercase()) {
            return Err(AppError::DataParsing(format!(
                "Duplicate mandate identifier '{}'",
                mandate.mandate_id
            )));
        }
        validate_required_text(&mandate.title, "Mandate title", 512)?;
        validate_required_text(&mandate.description, "Mandate description", 8_192)?;
        validate_phrases(
            &mandate.applicability_phrases,
            "Mandate applicability phrases",
        )?;
        validate_phrases(
            &mandate.required_evidence_phrases,
            "Mandate evidence phrases",
        )?;
    }
    Ok(())
}

fn validate_phrases(phrases: &[String], label: &str) -> Result<(), AppError> {
    if phrases.is_empty() || phrases.len() > MAXIMUM_PHRASES_PER_MANDATE {
        return Err(AppError::DataParsing(format!(
            "{label} must contain between 1 and {MAXIMUM_PHRASES_PER_MANDATE} entries"
        )));
    }
    let mut normalized = HashSet::new();
    for phrase in phrases {
        validate_required_text(phrase, label, MAXIMUM_PHRASE_CHARACTERS)?;
        let tokens = policy_tokens(phrase);
        if tokens.is_empty() || tokens.len() > 24 {
            return Err(AppError::DataParsing(format!(
                "{label} entries must contain between 1 and 24 searchable words"
            )));
        }
        if !normalized.insert(tokens.join(" ")) {
            return Err(AppError::DataParsing(format!(
                "{label} contains a duplicate phrase"
            )));
        }
    }
    Ok(())
}

fn validate_identifier(value: &str, label: &str, maximum_bytes: usize) -> Result<(), AppError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum_bytes {
        return Err(AppError::DataParsing(format!(
            "{label} must contain between 1 and {maximum_bytes} bytes"
        )));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return Err(AppError::DataParsing(format!(
            "{label} contains unsupported characters"
        )));
    }
    Ok(())
}

fn validate_required_text(value: &str, label: &str, maximum_bytes: usize) -> Result<(), AppError> {
    let value = value.trim();
    if value.is_empty() || value.len() > maximum_bytes {
        return Err(AppError::DataParsing(format!(
            "{label} must contain between 1 and {maximum_bytes} bytes"
        )));
    }
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
    {
        return Err(AppError::DataParsing(format!(
            "{label} contains unsupported control characters"
        )));
    }
    Ok(())
}

fn validate_optional_date(value: Option<&str>, label: &str) -> Result<(), AppError> {
    let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
        .map(|_| ())
        .map_err(|error| AppError::DataParsing(format!("Invalid {label}: {error}")))
}

fn normalize_source_text(source: &str) -> Result<String, AppError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut normalized = String::with_capacity(source.len());
    let mut characters = source.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\r' {
            if characters.peek() == Some(&'\n') {
                characters.next();
            }
            normalized.push('\n');
        } else if character.is_control() && !matches!(character, '\n' | '\t') {
            return Err(AppError::DataParsing(
                "Legal source contains unsupported control characters".to_string(),
            ));
        } else {
            normalized.push(character);
        }
    }
    let mut output = Vec::new();
    let mut preceding_blank = true;
    for line in normalized.lines() {
        let line = collapse_horizontal_whitespace(line).trim().to_string();
        if line.is_empty() {
            if !preceding_blank && !output.is_empty() {
                output.push(String::new());
            }
            preceding_blank = true;
        } else {
            output.push(line);
            preceding_blank = false;
        }
    }
    while output.last().is_some_and(String::is_empty) {
        output.pop();
    }
    let output = output.join("\n");
    if output.chars().count() > MAXIMUM_LEGAL_SOURCE_CHARACTERS {
        return Err(AppError::DataParsing(
            "Normalized legal source exceeds the character limit".to_string(),
        ));
    }
    Ok(output)
}

fn split_sections(source: &str, default_title: &str) -> Result<Vec<SectionBuilder>, AppError> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut sections = Vec::new();
    let mut heading_stack = vec![String::new(); 6];
    let mut current = SectionBuilder {
        heading_level: 0,
        heading_path: default_title.to_string(),
        title: default_title.to_string(),
        body: Vec::new(),
    };
    let mut index = 0_usize;
    while index < lines.len() {
        let line = lines[index].trim();
        let setext_level = lines
            .get(index + 1)
            .and_then(|underline| setext_heading_level(underline.trim()));
        let heading = if !line.is_empty() && setext_level.is_some() {
            let level = setext_level.unwrap_or(1);
            index += 1;
            Some((level, line.to_string()))
        } else {
            atx_heading(line).or_else(|| structural_legal_heading(line))
        };
        if let Some((level, title)) = heading {
            if !current.body.iter().all(|line| line.is_empty()) {
                sections.push(current);
                if sections.len() >= MAXIMUM_PROVISIONS {
                    return Err(AppError::DataParsing(format!(
                        "Legal source exceeds {MAXIMUM_PROVISIONS} provisions"
                    )));
                }
            }
            heading_stack[level - 1] = title.clone();
            for entry in heading_stack.iter_mut().skip(level) {
                entry.clear();
            }
            let heading_path = heading_stack
                .iter()
                .filter(|entry| !entry.is_empty())
                .cloned()
                .collect::<Vec<_>>()
                .join(" › ");
            current = SectionBuilder {
                heading_level: level,
                heading_path,
                title,
                body: Vec::new(),
            };
        } else {
            current.body.push(line.to_string());
        }
        index += 1;
    }
    if !current.body.iter().all(|line| line.is_empty()) {
        sections.push(current);
    }
    Ok(sections)
}

fn atx_heading(line: &str) -> Option<(usize, String)> {
    let marker_count = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&marker_count)
        || !line
            .chars()
            .nth(marker_count)
            .is_some_and(char::is_whitespace)
    {
        return None;
    }
    let title = line[marker_count..]
        .trim()
        .trim_end_matches('#')
        .trim()
        .to_string();
    (!title.is_empty()).then_some((marker_count, title))
}

fn setext_heading_level(line: &str) -> Option<usize> {
    if line.len() < 3 {
        return None;
    }
    if line.chars().all(|character| character == '=') {
        Some(1)
    } else if line.chars().all(|character| character == '-') {
        Some(2)
    } else {
        None
    }
}

fn structural_legal_heading(line: &str) -> Option<(usize, String)> {
    if line.is_empty() || line.chars().count() > 200 {
        return None;
    }
    let folded = line.to_lowercase();
    let starts_as_legal_heading = line.starts_with('§')
        || [
            "section ", "sec. ", "article ", "chapter ", "part ", "title ",
        ]
        .iter()
        .any(|prefix| folded.starts_with(prefix));
    let alphabetic = line
        .chars()
        .filter(|character| character.is_alphabetic())
        .count();
    let uppercase = line
        .chars()
        .filter(|character| character.is_alphabetic())
        .all(|character| character.is_uppercase());
    (starts_as_legal_heading || (alphabetic >= 4 && uppercase)).then(|| (2, line.to_string()))
}

fn collapse_horizontal_whitespace(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut pending_space = false;
    for character in value.chars() {
        if matches!(character, ' ' | '\t' | '\u{00a0}') {
            pending_space = !output.is_empty();
        } else {
            if pending_space {
                output.push(' ');
                pending_space = false;
            }
            output.push(character);
        }
    }
    output
}

fn stem_policy_token(token: &str) -> String {
    let mut stem = token.to_string();
    for suffix in [
        "ments", "ment", "ations", "ation", "ingly", "edly", "ing", "ied", "ies", "ed", "es", "s",
    ] {
        if stem.len() > suffix.len() + 3 && stem.ends_with(suffix) {
            stem.truncate(stem.len() - suffix.len());
            if matches!(suffix, "ied" | "ies") {
                stem.push('y');
            }
            break;
        }
    }
    stem
}
