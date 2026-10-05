//! Metadata-vs-content-contradiction detector.
//!
//! Compares a PDF's own declared creation timestamp (Info dictionary `CreationDate` or XMP
//! `xmp:CreateDate`) against dates mentioned inside the document's extracted text. Emits a
//! finding when the text references an event dated AFTER the document's declared creation, which
//! is either backdating or silent modification.
//!
//! Three sub-signals distinguished in the finding's explanation prose (not as separate
//! `FindingKind` variants — one kind, three flavors):
//!
//! 1. Any text-cited date more than 24 hours after `CreationDate` (baseline signal).
//! 2. Document titled/labelled "Final" but text mentions a "Draft" dated after itself.
//! 3. A signed contract that cites an amendment dated after its own signature.
//!
//! **What this is not**: a legal-claim finding. A contradiction is a pattern signal about
//! backdating or silent modification; the individual document's authenticity claim (if any) is
//! evaluated separately by the provenance ladder.
//!
//! **Bounds**: at most `MAX_CANDIDATES` documents inspected per invocation; PDF reads capped at
//! [`MAX_PDF_BYTES_FOR_METADATA`] to keep the sweep responsive.

use crate::{
    analysis::finding::{
        finding_id, Finding, FindingCitation, FindingConfidence, FindingKind, FindingSeverity,
        FindingSubject,
    },
    error::AppError,
    storage::DuckDbRepository,
};
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use duckdb::params;
use lopdf::{Document, Object};
use regex::Regex;
use std::path::PathBuf;
use std::sync::LazyLock;

pub const SLUG: &str = "metadata_vs_content";

const MAX_CANDIDATES: i64 = 500;
const MAX_PDF_BYTES_FOR_METADATA: u64 = 16 * 1024 * 1024;

/// Text-cited dates must be at least this far after the metadata creation date before the
/// contradiction fires. Sub-day differences are timezone/rounding artifacts, not backdating.
const MIN_CONTRADICTION_DAYS: i64 = 1;

/// Severity thresholds (calendar days between metadata CreationDate and the future-cited event).
const SERIOUS_DAYS: i64 = 30;
const CRITICAL_DAYS: i64 = 365;

/// One `YYYY-MM-DD`, `M/D/YYYY`, or `Month DD, YYYY` date in the extracted text.
static DATE_IN_TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:(?P<iso>\d{4}-\d{2}-\d{2})|(?P<slash>\d{1,2}/\d{1,2}/\d{4})|(?P<verbose>(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{1,2},?\s+\d{4}))\b"
    )
    .unwrap()
});

/// Sub-signal: a "Draft" mention preceding a date. Used to distinguish "text refers to a later
/// draft" from just "text names a later date."
static DRAFT_NEAR_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\bdraft\b[^.\n\r]{0,40}(?:\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}/\d{4}|(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{1,2},?\s+\d{4})"
    )
    .unwrap()
});

/// Sub-signal: an "amendment" cite preceding a date. Used in the contract flavor.
static AMENDMENT_NEAR_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\bamendment\b[^.\n\r]{0,40}(?:\d{4}-\d{2}-\d{2}|\d{1,2}/\d{1,2}/\d{4}|(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{1,2},?\s+\d{4})"
    )
    .unwrap()
});

struct Candidate {
    document_id: i64,
    portal_key: String,
    request_id: Option<i64>,
    file_name: Option<String>,
    local_path: String,
    extracted_text: String,
}

pub async fn run(repository: &DuckDbRepository) -> Result<Vec<Finding>, AppError> {
    let candidates = load_candidates(repository).await?;
    let mut findings = Vec::new();
    for candidate in candidates {
        match extract_creation_date(PathBuf::from(&candidate.local_path)) {
            Ok(Some(creation_date)) => {
                if let Some(finding) = evaluate(&candidate, creation_date) {
                    findings.push(finding);
                }
            }
            Ok(None) | Err(_) => {
                // No creation date reachable, or PDF unreadable. Skip silently — the detector is
                // opportunistic on a signal that many PDFs simply don't carry.
                continue;
            }
        }
    }
    findings.sort_by_key(|f| std::cmp::Reverse(f.rank_key()));
    Ok(findings)
}

async fn load_candidates(repository: &DuckDbRepository) -> Result<Vec<Candidate>, AppError> {
    repository
        .query_connection("Metadata-vs-content candidate query", move |conn| {
            let mut statement = conn
                .prepare(
                    "SELECT d.id, d.portal_subdomain, d.request_id, d.file_name, d.local_path,
                        fts.extracted_text
                     FROM documents d
                     LEFT JOIN documents_fts fts
                        ON fts.document_id = d.id
                       AND fts.portal_subdomain = d.portal_subdomain
                     WHERE d.local_path IS NOT NULL
                       AND LOWER(TRIM(d.visibility)) = 'public'
                       AND d.portal_subdomain NOT IN (SELECT subdomain FROM user_supplied_sources)
                       AND LOWER(COALESCE(d.content_type, '')) LIKE 'application/pdf%'
                     ORDER BY d.updated_at DESC
                     LIMIT ?1",
                )
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to prepare metadata-vs-content query: {error}"
                    ))
                })?;
            let rows = statement
                .query_map(params![MAX_CANDIDATES], |row| {
                    Ok(Candidate {
                        document_id: row.get(0)?,
                        portal_key: row.get(1)?,
                        request_id: row.get::<_, Option<i64>>(2)?,
                        file_name: row.get::<_, Option<String>>(3)?,
                        local_path: row.get(4)?,
                        extracted_text: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    })
                })
                .map_err(|error| {
                    AppError::Storage(format!(
                        "Failed to execute metadata-vs-content query: {error}"
                    ))
                })?;
            let mut collected = Vec::new();
            for row in rows {
                collected.push(row.map_err(|error| {
                    AppError::Storage(format!("Failed to decode metadata-vs-content row: {error}"))
                })?);
            }
            Ok(collected)
        })
        .await
}

fn extract_creation_date(path: PathBuf) -> Result<Option<DateTime<Utc>>, AppError> {
    let metadata = std::fs::metadata(&path)
        .map_err(|error| AppError::Storage(format!("Failed to stat PDF: {error}")))?;
    if metadata.len() > MAX_PDF_BYTES_FOR_METADATA {
        return Ok(None);
    }
    let document = crate::parsers::pdf::load_bounded(&path).map_err(|error| {
        AppError::DataParsing(format!("Failed to open PDF for metadata: {error}"))
    })?;
    Ok(read_info_date(&document, b"CreationDate").or_else(|| read_xmp_create_date(&document)))
}

fn read_info_date(document: &Document, key: &[u8]) -> Option<DateTime<Utc>> {
    let info_object = document.trailer.get(b"Info").ok()?;
    let dictionary = match info_object {
        Object::Dictionary(dict) => dict,
        Object::Reference(id) => match document.get_object(*id).ok()? {
            Object::Dictionary(dict) => dict,
            _ => return None,
        },
        _ => return None,
    };
    let value = dictionary.get(key).ok()?;
    let bytes = match value {
        Object::String(bytes, _) => bytes,
        _ => return None,
    };
    parse_pdf_date(bytes)
}

fn read_xmp_create_date(document: &Document) -> Option<DateTime<Utc>> {
    let catalog = document.catalog().ok()?;
    let metadata_ref = catalog.get(b"Metadata").ok()?;
    let stream_object = match metadata_ref {
        Object::Stream(stream) => stream.clone(),
        Object::Reference(id) => match document.get_object(*id).ok()? {
            Object::Stream(stream) => stream.clone(),
            _ => return None,
        },
        _ => return None,
    };
    let content = stream_object
        .decompressed_content_with_limit(crate::parsers::pdf::MAX_DECOMPRESSED_STREAM_BYTES)
        .ok()?;
    let text = std::str::from_utf8(&content).ok()?;
    let start = text.find("<xmp:CreateDate>")?;
    let after_open = start + "<xmp:CreateDate>".len();
    let end_relative = text[after_open..].find("</xmp:CreateDate>")?;
    let iso_slice = text[after_open..after_open + end_relative].trim();
    DateTime::parse_from_rfc3339(iso_slice)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}

fn parse_pdf_date(bytes: &[u8]) -> Option<DateTime<Utc>> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    let stripped = text.strip_prefix("D:").unwrap_or(text);
    if stripped.len() < 4 {
        return None;
    }
    let year: i32 = stripped.get(0..4)?.parse().ok()?;
    let month: u32 = stripped.get(4..6).map_or(1, |v| v.parse().unwrap_or(1));
    let day: u32 = stripped.get(6..8).map_or(1, |v| v.parse().unwrap_or(1));
    let hour: u32 = stripped.get(8..10).map_or(0, |v| v.parse().unwrap_or(0));
    let minute: u32 = stripped.get(10..12).map_or(0, |v| v.parse().unwrap_or(0));
    let second: u32 = stripped.get(12..14).map_or(0, |v| v.parse().unwrap_or(0));
    let (tz_sign_char, tz_body) = if let Some(rest) = stripped.get(14..) {
        if rest.is_empty() || rest.starts_with('Z') {
            ('Z', "")
        } else {
            (rest.chars().next().unwrap_or('Z'), &rest[1..])
        }
    } else {
        ('Z', "")
    };
    let (tz_hours, tz_minutes): (i32, i32) = if tz_sign_char == 'Z' {
        (0, 0)
    } else {
        let hours: i32 = tz_body.get(0..2).map_or(0, |v| v.parse().unwrap_or(0));
        let after_hours = tz_body.get(2..).unwrap_or("").trim_start_matches('\'');
        let minutes: i32 = after_hours
            .split('\'')
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let sign = if tz_sign_char == '-' { -1 } else { 1 };
        (sign * hours, sign * minutes)
    };
    let naive = NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, second)?;
    let offset = chrono::FixedOffset::east_opt(tz_hours * 3600 + tz_minutes * 60)?;
    let local = offset.from_local_datetime(&naive).single()?;
    Some(local.with_timezone(&Utc))
}

/// Parse one text-cited date. Returns UTC noon on the calendar day (dates in text are day-scoped
/// so we avoid pretending we know the hour).
fn parse_text_date(text: &str) -> Option<DateTime<Utc>> {
    let trimmed = text.trim();
    // ISO: YYYY-MM-DD
    if let Ok(iso) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Some(Utc.from_utc_datetime(&iso.and_hms_opt(12, 0, 0)?));
    }
    // Slash: M/D/YYYY or MM/DD/YYYY
    for pattern in ["%m/%d/%Y", "%-m/%-d/%Y"] {
        if let Ok(date) = NaiveDate::parse_from_str(trimmed, pattern) {
            return Some(Utc.from_utc_datetime(&date.and_hms_opt(12, 0, 0)?));
        }
    }
    // Verbose: Month DD, YYYY  or  Month DD YYYY
    for pattern in ["%B %d, %Y", "%B %d %Y"] {
        if let Ok(date) = NaiveDate::parse_from_str(trimmed, pattern) {
            return Some(Utc.from_utc_datetime(&date.and_hms_opt(12, 0, 0)?));
        }
    }
    None
}

/// Locates the latest text-cited date in the passage, along with whether it appears near "Draft"
/// or "amendment" language.
fn scan_text_dates(text: &str) -> Option<TextDateEvidence> {
    let mut latest: Option<DateTime<Utc>> = None;
    let mut latest_slice: Option<String> = None;
    for capture in DATE_IN_TEXT.captures_iter(text) {
        let matched = capture.get(0)?.as_str();
        let Some(parsed) = parse_text_date(matched) else {
            continue;
        };
        if latest.is_none_or(|current| parsed > current) {
            latest = Some(parsed);
            latest_slice = Some(matched.to_string());
        }
    }
    let latest = latest?;
    let latest_slice = latest_slice?;
    Some(TextDateEvidence {
        latest,
        latest_slice,
        near_draft: DRAFT_NEAR_DATE.is_match(text),
        near_amendment: AMENDMENT_NEAR_DATE.is_match(text),
    })
}

struct TextDateEvidence {
    latest: DateTime<Utc>,
    latest_slice: String,
    near_draft: bool,
    near_amendment: bool,
}

fn evaluate(candidate: &Candidate, creation_date: DateTime<Utc>) -> Option<Finding> {
    let evidence = scan_text_dates(&candidate.extracted_text)?;
    let gap_days = (evidence.latest - creation_date).num_days();
    if gap_days < MIN_CONTRADICTION_DAYS {
        return None;
    }
    // Compute the flavor based on textual cues and filename.
    let title_lower = candidate
        .file_name
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let flavor = if title_lower.contains("final") && evidence.near_draft {
        "final_references_later_draft"
    } else if title_lower.contains("contract") && evidence.near_amendment {
        "contract_cites_later_amendment"
    } else {
        "content_dates_after_creation"
    };
    let severity = if gap_days >= CRITICAL_DAYS {
        FindingSeverity::Critical
    } else if gap_days >= SERIOUS_DAYS {
        FindingSeverity::Serious
    } else {
        FindingSeverity::Notable
    };
    // Confidence: both anchors are unambiguous when they're both parseable to a specific day; we
    // require that for the detector to fire at all, so full-weight one-signal confidence applies.
    // The flavored variants (Draft, amendment cited) earn a slight bump for the semantic anchor.
    let base_bp = 6_500_u16;
    let flavor_bp = if flavor == "content_dates_after_creation" {
        0
    } else {
        1_500
    };
    let confidence = FindingConfidence::measured(base_bp.saturating_add(flavor_bp).min(9_500));
    let subject = FindingSubject {
        portal_key: Some(candidate.portal_key.clone()),
        request_id: candidate.request_id,
        document_id: Some(candidate.document_id),
        ..Default::default()
    };
    let salient = format!(
        "flavor={flavor};gap_days={gap_days};latest={}",
        evidence.latest_slice
    );
    let id = finding_id(
        FindingKind::MetadataContradictsContent,
        SLUG,
        &subject,
        &[&salient],
    );
    let file_display = candidate
        .file_name
        .clone()
        .unwrap_or_else(|| format!("document {}", candidate.document_id));
    let headline = match flavor {
        "final_references_later_draft" => format!(
            "\"Final\" document references a draft dated {gap_days} days after its own creation",
        ),
        "contract_cites_later_amendment" => format!(
            "Signed contract cites an amendment dated {gap_days} days after its own signing",
        ),
        _ => {
            format!("Document text references a date {gap_days} days after its declared creation",)
        }
    };
    let explanation = format!(
        "\"{file_display}\" declares its creation on {creation}. Its extracted text references \
         the date {latest} — {gap_days} days later. {flavor_note} A metadata/content \
         contradiction is a pattern signal about backdating or silent modification; it does not \
         by itself establish tampering.",
        creation = format_short(creation_date),
        latest = format_short(evidence.latest),
        flavor_note = match flavor {
            "final_references_later_draft" =>
                "The filename is labelled \"final\" and the text mentions \"draft\" near the later date, so the reference is likely to a subsequent revision.",
            "contract_cites_later_amendment" =>
                "The filename identifies a contract and the text cites an \"amendment\" near the later date.",
            _ => "",
        },
    );
    let citations = vec![FindingCitation {
        portal_key: candidate.portal_key.clone(),
        document_id: candidate.document_id,
        character_start: None,
        character_end: None,
        quote: Some(format!("Text-cited date: {}", evidence.latest_slice)),
        label: Some(format!(
            "CreationDate {} vs. content date {}",
            format_short(creation_date),
            format_short(evidence.latest)
        )),
    }];
    let now = Utc::now();
    Some(Finding {
        id,
        kind: FindingKind::MetadataContradictsContent,
        severity,
        confidence,
        subject,
        headline,
        explanation,
        citations,
        discovered_at: now,
        last_seen_at: now,
        dismissed_at: None,
        source_detector: SLUG.into(),
    })
}

fn format_short(instant: DateTime<Utc>) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        instant.year(),
        instant.month(),
        instant.day()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(text: &str, file_name: Option<&str>) -> Candidate {
        Candidate {
            document_id: 42,
            portal_key: "p".into(),
            request_id: Some(7),
            file_name: file_name.map(String::from),
            local_path: "/dev/null".into(),
            extracted_text: text.into(),
        }
    }

    fn ts(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    #[test]
    fn parses_all_three_text_date_forms() {
        assert!(parse_text_date("2026-05-14").is_some());
        assert!(parse_text_date("5/14/2026").is_some());
        assert!(parse_text_date("May 14, 2026").is_some());
        assert!(parse_text_date("May 14 2026").is_some());
    }

    #[test]
    fn no_finding_when_all_text_dates_predate_creation() {
        let text = "Reviewed on 2020-01-05 and finalized on 2020-02-01.";
        let candidate = candidate(text, Some("memo.pdf"));
        assert!(
            evaluate(&candidate, ts(2024, 1, 1)).is_none(),
            "text dates before creation is normal, must not fire"
        );
    }

    #[test]
    fn no_finding_when_gap_is_under_the_minimum() {
        // Text date same day as creation is timezone noise, not backdating.
        let text = "Effective 2024-05-14 per section 3.";
        let candidate = candidate(text, Some("memo.pdf"));
        assert!(evaluate(&candidate, ts(2024, 5, 14)).is_none());
    }

    #[test]
    fn notable_finding_for_a_short_forward_reference() {
        let text = "Follow-up meeting scheduled for June 5, 2024.";
        let candidate = candidate(text, Some("memo.pdf"));
        let finding = evaluate(&candidate, ts(2024, 5, 20)).expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Notable);
        assert_eq!(finding.kind, FindingKind::MetadataContradictsContent);
        assert!(finding.headline.contains("date"));
    }

    #[test]
    fn serious_finding_when_gap_exceeds_thirty_days() {
        let text = "Referenced report published 2024-08-15.";
        let candidate = candidate(text, Some("memo.pdf"));
        let finding = evaluate(&candidate, ts(2024, 5, 1)).expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Serious);
    }

    #[test]
    fn critical_finding_when_gap_exceeds_a_year() {
        let text = "Later analysis on 2026-01-15.";
        let candidate = candidate(text, Some("memo.pdf"));
        let finding = evaluate(&candidate, ts(2024, 1, 1)).expect("must fire");
        assert_eq!(finding.severity, FindingSeverity::Critical);
    }

    #[test]
    fn final_labelled_document_carries_the_draft_flavor() {
        let text = "Superseded by draft dated 2024-07-01.";
        let candidate = candidate(text, Some("policy_final.pdf"));
        let finding = evaluate(&candidate, ts(2024, 5, 1)).expect("must fire");
        assert!(finding.headline.contains("Final"));
        // Flavor confidence bump.
        match finding.confidence {
            FindingConfidence::Measured { basis_points } => {
                assert!(basis_points >= 7_500);
            }
            other => panic!("expected measured confidence, got {other:?}"),
        }
    }

    #[test]
    fn contract_labelled_document_carries_the_amendment_flavor() {
        let text = "See amendment dated 2024-09-30.";
        let candidate = candidate(text, Some("services_contract.pdf"));
        let finding = evaluate(&candidate, ts(2024, 5, 1)).expect("must fire");
        assert!(finding.headline.contains("contract"));
        assert!(finding.headline.contains("amendment"));
    }

    #[test]
    fn finding_id_is_stable_for_same_evidence() {
        let text = "Effective July 1, 2024.";
        let candidate = candidate(text, Some("memo.pdf"));
        let a = evaluate(&candidate, ts(2024, 5, 1)).unwrap();
        let b = evaluate(&candidate, ts(2024, 5, 1)).unwrap();
        assert_eq!(a.id, b.id);
    }
}
