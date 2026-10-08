//! PDF metadata signals in a structured form, for non-C2PA provenance and holdback detection.
//!
//! This module extracts the timestamp- and producer-chain fields a PDF reports about itself: the
//! Info dictionary (`Producer`, `Creator`, `CreationDate`, `ModDate`) and the XMP metadata stream
//! (`xmp:CreateDate`, `xmp:ModifyDate`, `xmp:MetadataDate`, `xmp:CreatorTool`, `pdf:Producer`).
//!
//! The `unredactor` module already reads these fields as *human-readable strings* for the residual
//! evidence audit; this module returns them as *typed values* usable by downstream detectors
//! (regenerated-response, held-back-response, and the non-C2PA provenance ladder). Both callers
//! parse the same underlying bytes; keeping the typed extractor here and the display extractor in
//! `unredactor` keeps their concerns separate without one having to depend on the other's return
//! shape.
//!
//! **Bounded**: rejects any file whose byte length exceeds `MAX_PDF_BYTES`. Returns a struct whose
//! every field is optional so a partially-parseable PDF still yields the pieces that were
//! recoverable. No fatal error on parse failure — an unreadable PDF simply carries no signals.

use chrono::{DateTime, FixedOffset, NaiveDate, TimeZone, Utc};
use lopdf::{Document, Object};
use std::path::Path;

use crate::error::AppError;
use crate::parsers::office::decode_xml_entities;
use crate::parsers::ooxml_docprops::extract_body;
use crate::parsers::pdf::{decode_text_string, resolve_dictionary, resolve_object};

/// Hard read cap. A metadata-only extractor never needs a large PDF loaded into memory; a file
/// larger than this is either not a records-response document (a scanned book) or is deliberately
/// oversized for a scan-me-please attack. Callers wanting the metadata for such a file can raise
/// this in their own wrapper.
pub const MAX_PDF_BYTES: u64 = 32 * 1024 * 1024;

/// All the metadata timestamps and producer-chain strings a PDF reports about itself.
///
/// Every field is optional. A field's absence means the PDF did not carry that value or the value
/// was unparseable — never zero, never epoch. Detectors that treat the absence as a signal must
/// name that intent explicitly (see the non-C2PA provenance ladder for how `NoSignals` is
/// distinguished from a self-consistent-but-thin document).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PdfMetadataSignals {
    /// PDF Info dictionary `CreationDate`, converted to UTC.
    pub info_creation_date: Option<DateTime<Utc>>,
    /// PDF Info dictionary `ModDate`, converted to UTC.
    pub info_mod_date: Option<DateTime<Utc>>,
    /// PDF Info dictionary `Producer` (typically the software that wrote the PDF bytes).
    pub info_producer: Option<String>,
    /// PDF Info dictionary `Creator` (typically the application that authored the source).
    pub info_creator: Option<String>,
    /// XMP `xmp:CreateDate`.
    pub xmp_create_date: Option<DateTime<Utc>>,
    /// XMP `xmp:ModifyDate`.
    pub xmp_modify_date: Option<DateTime<Utc>>,
    /// XMP `xmp:MetadataDate` (the last time the XMP block itself was rewritten).
    pub xmp_metadata_date: Option<DateTime<Utc>>,
    /// XMP `xmp:CreatorTool`.
    pub xmp_creator_tool: Option<String>,
    /// XMP `pdf:Producer` (mirrors Info Producer when both are present).
    pub xmp_pdf_producer: Option<String>,
}

impl PdfMetadataSignals {
    /// True when the extractor found nothing at all.
    pub fn is_empty(&self) -> bool {
        self.info_creation_date.is_none()
            && self.info_mod_date.is_none()
            && self.info_producer.is_none()
            && self.info_creator.is_none()
            && self.xmp_create_date.is_none()
            && self.xmp_modify_date.is_none()
            && self.xmp_metadata_date.is_none()
            && self.xmp_creator_tool.is_none()
            && self.xmp_pdf_producer.is_none()
    }

    /// The most recent modification instant carried by any timestamp field, if any exist. Prefer
    /// Info `ModDate` when present (that is the field applications write on save); fall back to
    /// XMP `xmp:ModifyDate`, then to `xmp:MetadataDate`.
    pub fn effective_modification(&self) -> Option<DateTime<Utc>> {
        self.info_mod_date
            .or(self.xmp_modify_date)
            .or(self.xmp_metadata_date)
    }

    /// True when Info and XMP producer strings both exist and match after trimming/lowercasing.
    pub fn producer_strings_match(&self) -> bool {
        match (
            self.info_producer.as_deref(),
            self.xmp_pdf_producer.as_deref(),
        ) {
            (Some(info), Some(xmp)) => info.trim().eq_ignore_ascii_case(xmp.trim()),
            _ => false,
        }
    }
}

/// Read a PDF's metadata signals from disk. Returns `Ok(default)` (all fields None) when the file
/// exists and is a PDF but carries no metadata; returns `Err` only on I/O failure or when the file
/// exceeds `MAX_PDF_BYTES`. A structurally-broken PDF is treated as empty metadata rather than an
/// error — the detector caller decides whether "no signals" is a finding.
pub fn read_pdf_metadata_signals(path: &Path) -> Result<PdfMetadataSignals, AppError> {
    // A file that lopdf refuses to open is not a signal by itself — an unrelated forensic
    // audit path already records that as an ingestion issue.
    Ok(read_parsed_pdf_metadata_signals(path)?.unwrap_or_default())
}

/// As [`read_pdf_metadata_signals`], but `None` when the file could not be parsed as a PDF, so a
/// caller showing the fields can tell "this PDF records no value" from "this file was not read".
pub fn read_parsed_pdf_metadata_signals(
    path: &Path,
) -> Result<Option<PdfMetadataSignals>, AppError> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| AppError::Storage(format!("PDF metadata stat failed: {error}")))?;
    if metadata.len() > MAX_PDF_BYTES {
        return Err(AppError::DataParsing(format!(
            "PDF exceeds {MAX_PDF_BYTES}-byte metadata read cap"
        )));
    }
    Ok(crate::parsers::pdf::load_bounded(path)
        .ok()
        .map(|document| extract_from_document(&document)))
}

/// Extract signals from an already-parsed lopdf Document. Split out from `read_pdf_metadata_signals`
/// so tests can supply a document without a filesystem path.
fn extract_from_document(document: &Document) -> PdfMetadataSignals {
    let mut signals = PdfMetadataSignals::default();
    let info_dictionary = document
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|value| resolve_dictionary(document, value));
    if let Some(dict) = info_dictionary {
        for (key, value) in dict.iter() {
            let Some(resolved) = resolve_object(document, value) else {
                continue;
            };
            match (key.as_slice(), resolved) {
                (b"CreationDate", Object::String(bytes, _)) => {
                    signals.info_creation_date = parse_pdf_date_literal(bytes);
                }
                (b"ModDate", Object::String(bytes, _)) => {
                    signals.info_mod_date = parse_pdf_date_literal(bytes);
                }
                (b"Producer", Object::String(bytes, _)) | (b"Producer", Object::Name(bytes)) => {
                    signals.info_producer = decode_pdf_string(bytes);
                }
                (b"Creator", Object::String(bytes, _)) | (b"Creator", Object::Name(bytes)) => {
                    signals.info_creator = decode_pdf_string(bytes);
                }
                _ => {}
            }
        }
    }

    if let Some(xmp_text) = read_xmp_stream(document) {
        signals.xmp_create_date = extract_xmp_tag(&xmp_text, "xmp:CreateDate")
            .and_then(|value| parse_iso_datetime(&value));
        signals.xmp_modify_date = extract_xmp_tag(&xmp_text, "xmp:ModifyDate")
            .and_then(|value| parse_iso_datetime(&value));
        signals.xmp_metadata_date = extract_xmp_tag(&xmp_text, "xmp:MetadataDate")
            .and_then(|value| parse_iso_datetime(&value));
        signals.xmp_creator_tool = extract_xmp_tag(&xmp_text, "xmp:CreatorTool");
        signals.xmp_pdf_producer = extract_xmp_tag(&xmp_text, "pdf:Producer");
    }

    signals
}

fn read_xmp_stream(document: &Document) -> Option<String> {
    let catalog = document.catalog().ok()?;
    let stream = resolve_object(document, catalog.get(b"Metadata").ok()?)?
        .as_stream()
        .ok()?
        .clone();
    let content = stream
        .decompressed_content_with_limit(crate::parsers::pdf::MAX_DECOMPRESSED_STREAM_BYTES)
        .ok()?;
    String::from_utf8(content).ok()
}

/// An XMP property in either RDF serialization: the element form `<tag>value</tag>` or the
/// abbreviated attribute form `tag="value"` on `rdf:Description`, which Acrobat writes for simple
/// properties. Not an XML parser; adequate for the well-formed XMP that producers emit.
fn extract_xmp_tag(xmp: &str, tag: &str) -> Option<String> {
    extract_body(xmp, tag).or_else(|| extract_xmp_attribute(xmp, tag))
}

fn extract_xmp_attribute(xmp: &str, name: &str) -> Option<String> {
    let mut cursor = 0;
    while let Some(relative) = xmp[cursor..].find(name) {
        let start = cursor + relative;
        cursor = start + name.len();
        if !xmp[..start].ends_with(|character: char| character.is_ascii_whitespace()) {
            continue;
        }
        let Some(after_equals) = xmp[cursor..].trim_start().strip_prefix('=') else {
            continue;
        };
        let quoted = after_equals.trim_start();
        let Some(quote) = quoted.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            continue;
        };
        let body = &quoted[1..];
        let value = decode_xml_entities(&body[..body.find(quote)?]).ok()?;
        let value = value.trim();
        return (!value.is_empty()).then(|| value.to_string());
    }
    None
}

fn parse_iso_datetime(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}

/// Parse a PDF Info-dictionary date literal (`D:YYYYMMDDHHmmSSOHH'mm'`). Shorter forms are legal;
/// missing fields default to unit values (month=1, day=1, HH=00, mm=00, SS=00, tz=Z). Returns
/// `None` when the year cannot be parsed — anything else defaults gracefully.
fn parse_pdf_date_literal(bytes: &[u8]) -> Option<DateTime<Utc>> {
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
    let offset = FixedOffset::east_opt(tz_hours * 3600 + tz_minutes * 60)?;
    let local = offset.from_local_datetime(&naive).single()?;
    Some(local.with_timezone(&Utc))
}

fn decode_pdf_string(bytes: &[u8]) -> Option<String> {
    let value = decode_text_string(bytes);
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_xmp_properties_in_element_and_attribute_form() {
        let xmp = r#"<x:xmpmeta xmlns:x='adobe:ns:meta/'>
            <rdf:RDF><rdf:Description
              xmlns:xmp='http://ns.adobe.com/xap/1.0/'
              xmp:CreateDate="2024-03-01T14:30:00Z"
              xmp:CreatorTool="Adobe Acrobat">
              <xmp:ModifyDate>2024-04-15T09:00:00Z</xmp:ModifyDate>
              <pdf:Producer>Test Producer 1.0</pdf:Producer>
            </rdf:Description></rdf:RDF>
        </x:xmpmeta>"#;
        assert_eq!(
            extract_xmp_tag(xmp, "xmp:ModifyDate"),
            Some("2024-04-15T09:00:00Z".to_string())
        );
        assert_eq!(
            extract_xmp_tag(xmp, "pdf:Producer"),
            Some("Test Producer 1.0".to_string())
        );
        assert_eq!(
            extract_xmp_tag(xmp, "xmp:CreateDate"),
            Some("2024-03-01T14:30:00Z".to_string())
        );
        assert_eq!(
            extract_xmp_tag(xmp, "xmp:CreatorTool"),
            Some("Adobe Acrobat".to_string())
        );
        assert_eq!(extract_xmp_tag(xmp, "xmp:Create"), None);
        assert_eq!(extract_xmp_tag(xmp, "xmp:AbsentTag"), None);
    }

    #[test]
    fn parses_pdf_date_literal_with_timezone_offset() {
        let parsed = parse_pdf_date_literal(b"D:20240301143000-05'00'").unwrap();
        // 14:30 in UTC-5 -> 19:30 UTC
        assert_eq!(parsed.timestamp(), 1_709_321_400);
    }

    #[test]
    fn signals_effective_modification_prefers_info_over_xmp() {
        let earlier = parse_iso_datetime("2024-03-01T10:00:00Z").unwrap();
        let later = parse_iso_datetime("2024-04-01T10:00:00Z").unwrap();
        let signals = PdfMetadataSignals {
            info_mod_date: Some(later),
            xmp_modify_date: Some(earlier),
            ..Default::default()
        };
        assert_eq!(signals.effective_modification(), Some(later));
        let no_info = PdfMetadataSignals {
            info_mod_date: None,
            xmp_modify_date: Some(earlier),
            ..Default::default()
        };
        assert_eq!(no_info.effective_modification(), Some(earlier));
    }

    #[test]
    fn producer_strings_match_is_case_and_whitespace_insensitive_but_requires_both() {
        let one_only = PdfMetadataSignals {
            info_producer: Some("Adobe PDF Library".into()),
            ..Default::default()
        };
        assert!(!one_only.producer_strings_match());
        let matching = PdfMetadataSignals {
            info_producer: Some("Adobe PDF Library".into()),
            xmp_pdf_producer: Some("  adobe pdf library  ".into()),
            ..Default::default()
        };
        assert!(matching.producer_strings_match());
        let mismatched = PdfMetadataSignals {
            info_producer: Some("Adobe PDF Library".into()),
            xmp_pdf_producer: Some("pdftk 3.1.0".into()),
            ..Default::default()
        };
        assert!(!mismatched.producer_strings_match());
    }

    #[test]
    fn decode_pdf_string_handles_utf16_be_bom_and_utf8() {
        // UTF-16 BE "PDF": 0x00 0x50, 0x00 0x44, 0x00 0x46
        let utf16 = [0xfe, 0xff, 0x00, 0x50, 0x00, 0x44, 0x00, 0x46];
        assert_eq!(decode_pdf_string(&utf16), Some("PDF".to_string()));
        assert_eq!(
            decode_pdf_string(b" plain UTF-8 "),
            Some("plain UTF-8".to_string())
        );
        assert_eq!(decode_pdf_string(b""), None);
    }

    #[test]
    fn read_pdf_metadata_signals_rejects_oversized_files() {
        use std::io::Write;
        let temp = tempfile::NamedTempFile::new().unwrap();
        let path = temp.path().to_owned();
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(MAX_PDF_BYTES + 1).unwrap();
        file.write_all(b"%PDF").unwrap();
        drop(file);
        let result = read_pdf_metadata_signals(&path);
        assert!(matches!(result, Err(AppError::DataParsing(_))));
    }

    #[test]
    fn read_pdf_metadata_signals_returns_default_for_unparseable_pdf() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        // Write junk that isn't a valid PDF; lopdf will refuse it.
        std::fs::write(temp.path(), b"not a real pdf at all").unwrap();
        let signals = read_pdf_metadata_signals(temp.path()).unwrap();
        assert!(signals.is_empty());
    }
}
