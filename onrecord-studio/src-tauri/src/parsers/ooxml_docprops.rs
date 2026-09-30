//! OOXML `docProps/app.xml` and `docProps/core.xml` metadata as typed fields.
//!
//! Where `office_audit` catalogs the *presence* of the `docProps/` directory as attributed
//! forensic evidence, this module reads the *values* it holds: revision counts, edit-time-in-
//! minutes, last-printed timestamp, last-modified-by, creation and modification timestamps. Those
//! are the highest-signal metadata a regenerated-response detector or a non-C2PA provenance
//! ladder can consume.
//!
//! Two files matter:
//! - `docProps/app.xml`: extended properties (`RevNum`, `TotalTime`, `Application`, `Company`,
//!   `Pages`, `Words`).
//! - `docProps/core.xml`: Dublin Core / OpenPackaging (`cp:lastModifiedBy`, `cp:revision`,
//!   `cp:lastPrinted`, `dcterms:created`, `dcterms:modified`).
//!
//! We read them with the same bounded zip-walker and string-based tag extraction that
//! `office_audit` uses — a full XML parser is unnecessary for docProps, which are small,
//! well-formed, and written by Office applications. Malformed inputs return the fields that were
//! parseable and leave the rest `None`; nothing panics or errors on a broken package.
//!
//! Bounds: reject packages larger than `MAX_PACKAGE_BYTES`; reject a single `docProps/*.xml`
//! part larger than `MAX_PART_BYTES`. Neither bound has ever been legitimately exceeded by an
//! Office-authored document.

use chrono::{DateTime, Utc};
use std::fs::File;
use std::path::Path;

use crate::error::AppError;

pub const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_PART_BYTES: u64 = 2 * 1024 * 1024;

/// Everything the two docProps files reported. Every field optional so a partially-parseable
/// package still yields the recoverable pieces.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OoxmlDocProps {
    /// From `docProps/core.xml` `cp:lastModifiedBy` — the account that last saved the file.
    pub last_modified_by: Option<String>,
    /// From `docProps/core.xml` `cp:revision` — Office's local revision counter for save cycles.
    pub revision: Option<u32>,
    /// From `docProps/core.xml` `cp:lastPrinted` — a print action is a separate signal from an
    /// edit; a document last-printed weeks before its declared response date is evidence the
    /// physical page was prepared before the digital release was issued.
    pub last_printed: Option<DateTime<Utc>>,
    /// From `docProps/core.xml` `dcterms:created`.
    pub created: Option<DateTime<Utc>>,
    /// From `docProps/core.xml` `dcterms:modified`.
    pub modified: Option<DateTime<Utc>>,
    /// From `docProps/app.xml` `Application` (e.g. "Microsoft Office Word").
    pub application: Option<String>,
    /// From `docProps/app.xml` `Company`.
    pub company: Option<String>,
    /// From `docProps/app.xml` `RevNum` (author revision count, separate from `cp:revision`).
    pub rev_num: Option<u32>,
    /// From `docProps/app.xml` `TotalTime` — cumulative edit time in minutes across all sessions.
    /// Compare against document length: hundreds of minutes on a three-paragraph letter is
    /// evidence of iterative rewriting, not one-pass drafting.
    pub total_time_minutes: Option<u32>,
    /// From `docProps/app.xml` `Pages`.
    pub pages: Option<u32>,
    /// From `docProps/app.xml` `Words`.
    pub words: Option<u32>,
}

impl OoxmlDocProps {
    /// True when nothing at all was extracted.
    pub fn is_empty(&self) -> bool {
        self.last_modified_by.is_none()
            && self.revision.is_none()
            && self.last_printed.is_none()
            && self.created.is_none()
            && self.modified.is_none()
            && self.application.is_none()
            && self.company.is_none()
            && self.rev_num.is_none()
            && self.total_time_minutes.is_none()
            && self.pages.is_none()
            && self.words.is_none()
    }

    /// The most recent timestamp any of the three date fields reports. Lets a caller compare
    /// against a stated response date without deciding which field is authoritative for this
    /// producer application.
    pub fn latest_touch(&self) -> Option<DateTime<Utc>> {
        [self.modified, self.last_printed, self.created]
            .into_iter()
            .flatten()
            .max()
    }
}

/// Read docProps from an OOXML/ODF package on disk. Returns `Ok(default)` if the file exists but
/// is not a zip-backed package or carries no `docProps/`; returns `Err` only for I/O failures or
/// bound violations.
pub fn read_ooxml_docprops(path: &Path) -> Result<OoxmlDocProps, AppError> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| AppError::Storage(format!("OOXML stat failed: {error}")))?;
    if metadata.len() > MAX_PACKAGE_BYTES {
        return Err(AppError::DataParsing(format!(
            "OOXML package exceeds {MAX_PACKAGE_BYTES}-byte cap"
        )));
    }
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Ok(OoxmlDocProps::default()),
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => return Ok(OoxmlDocProps::default()),
    };
    let mut props = OoxmlDocProps::default();
    // Extract core.xml and app.xml. Names may vary in case (some producers write `docprops/`).
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_ascii_lowercase();
        if name.contains("../") || entry.size() > MAX_PART_BYTES {
            continue;
        }
        if name.ends_with("docprops/core.xml") {
            let mut buffer = Vec::with_capacity(entry.size() as usize);
            if std::io::Read::read_to_end(&mut entry, &mut buffer).is_ok() {
                if let Ok(text) = std::str::from_utf8(&buffer) {
                    apply_core_xml(&mut props, text);
                }
            }
        } else if name.ends_with("docprops/app.xml") {
            let mut buffer = Vec::with_capacity(entry.size() as usize);
            if std::io::Read::read_to_end(&mut entry, &mut buffer).is_ok() {
                if let Ok(text) = std::str::from_utf8(&buffer) {
                    apply_app_xml(&mut props, text);
                }
            }
        }
    }
    Ok(props)
}

fn apply_core_xml(props: &mut OoxmlDocProps, xml: &str) {
    props.last_modified_by = extract_body(xml, "cp:lastModifiedBy");
    props.revision = extract_body(xml, "cp:revision").and_then(|v| v.parse().ok());
    props.last_printed = extract_body(xml, "cp:lastPrinted").and_then(|v| parse_iso(&v));
    props.created = extract_body(xml, "dcterms:created").and_then(|v| parse_iso(&v));
    props.modified = extract_body(xml, "dcterms:modified").and_then(|v| parse_iso(&v));
}

fn apply_app_xml(props: &mut OoxmlDocProps, xml: &str) {
    // The app.xml root uses the extended-properties namespace; tags are unprefixed inside.
    props.application = extract_body(xml, "Application");
    props.company = extract_body(xml, "Company");
    props.rev_num = extract_body(xml, "RevNum").and_then(|v| v.parse().ok());
    props.total_time_minutes = extract_body(xml, "TotalTime").and_then(|v| v.parse().ok());
    props.pages = extract_body(xml, "Pages").and_then(|v| v.parse().ok());
    props.words = extract_body(xml, "Words").and_then(|v| v.parse().ok());
}

/// Extract the text between `<tag>` and `</tag>`. Ignores attributes on the open tag. Returns
/// `None` for self-closing tags, absent tags, or an empty body after trim.
fn extract_body(xml: &str, tag: &str) -> Option<String> {
    let open_prefix = format!("<{tag}");
    let mut cursor = 0;
    while let Some(rel_start) = xml[cursor..].find(&open_prefix) {
        let absolute_start = cursor + rel_start;
        let after_prefix = absolute_start + open_prefix.len();
        // Peek the character after the tag name to ensure it's a boundary (space, /, or >).
        let boundary_byte = xml.as_bytes().get(after_prefix).copied();
        if !matches!(boundary_byte, Some(b'>') | Some(b' ') | Some(b'/')) {
            cursor = after_prefix;
            continue;
        }
        let after_prefix_slice = &xml[after_prefix..];
        let gt = after_prefix_slice.find('>')?;
        // Self-closing tag: nothing to extract.
        if after_prefix_slice.as_bytes().get(gt.saturating_sub(1)) == Some(&b'/') {
            return None;
        }
        let body_start = after_prefix + gt + 1;
        let close_tag = format!("</{tag}>");
        let close_rel = xml[body_start..].find(&close_tag)?;
        let body = xml[body_start..body_start + close_rel].trim();
        return if body.is_empty() {
            None
        } else {
            Some(decode_xml_entities(body))
        };
    }
    None
}

/// Decode the five predefined XML entities. docProps content rarely uses more.
fn decode_xml_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn parse_iso(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn build_ooxml_package(core_xml: &str, app_xml: &str) -> tempfile::NamedTempFile {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(temp.path())
            .unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        zip.start_file("docProps/core.xml", options).unwrap();
        zip.write_all(core_xml.as_bytes()).unwrap();
        zip.start_file("docProps/app.xml", options).unwrap();
        zip.write_all(app_xml.as_bytes()).unwrap();
        zip.finish().unwrap();
        temp
    }

    #[test]
    fn extracts_core_and_app_fields_from_a_normal_ooxml_package() {
        let core = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
  xmlns:dc="http://purl.org/dc/elements/1.1/"
  xmlns:dcterms="http://purl.org/dc/terms/">
  <cp:lastModifiedBy>Jane Doe</cp:lastModifiedBy>
  <cp:revision>47</cp:revision>
  <cp:lastPrinted>2024-01-15T09:00:00Z</cp:lastPrinted>
  <dcterms:created xsi:type="dcterms:W3CDTF">2024-03-01T14:30:00Z</dcterms:created>
  <dcterms:modified xsi:type="dcterms:W3CDTF">2024-04-15T10:00:00Z</dcterms:modified>
</cp:coreProperties>"#;
        let app = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">
  <Application>Microsoft Office Word</Application>
  <Company>Test Agency</Company>
  <RevNum>47</RevNum>
  <TotalTime>320</TotalTime>
  <Pages>3</Pages>
  <Words>412</Words>
</Properties>"#;
        let file = build_ooxml_package(core, app);
        let props = read_ooxml_docprops(file.path()).unwrap();
        assert_eq!(props.last_modified_by.as_deref(), Some("Jane Doe"));
        assert_eq!(props.revision, Some(47));
        assert_eq!(
            props.last_printed,
            Some("2024-01-15T09:00:00Z".parse::<DateTime<Utc>>().unwrap())
        );
        assert_eq!(props.application.as_deref(), Some("Microsoft Office Word"));
        assert_eq!(props.company.as_deref(), Some("Test Agency"));
        assert_eq!(props.rev_num, Some(47));
        assert_eq!(props.total_time_minutes, Some(320));
        assert_eq!(props.pages, Some(3));
        assert_eq!(props.words, Some(412));
    }

    #[test]
    fn partial_extraction_returns_only_the_fields_that_parsed() {
        let core =
            "<coreProperties><cp:lastModifiedBy>Someone</cp:lastModifiedBy></coreProperties>";
        let app = "";
        let file = build_ooxml_package(core, app);
        let props = read_ooxml_docprops(file.path()).unwrap();
        assert_eq!(props.last_modified_by.as_deref(), Some("Someone"));
        assert!(props.revision.is_none());
        assert!(props.application.is_none());
    }

    #[test]
    fn latest_touch_prefers_modified_over_last_printed_and_created() {
        let props = OoxmlDocProps {
            modified: Some("2024-04-15T10:00:00Z".parse().unwrap()),
            last_printed: Some("2024-06-01T10:00:00Z".parse().unwrap()),
            created: Some("2024-01-01T10:00:00Z".parse().unwrap()),
            ..Default::default()
        };
        assert_eq!(
            props.latest_touch(),
            Some("2024-06-01T10:00:00Z".parse().unwrap()),
            "last_printed is later than modified, so it wins the max"
        );
    }

    #[test]
    fn xml_entities_in_last_modified_by_are_decoded() {
        let core = "<cp:lastModifiedBy>Jones &amp; Co</cp:lastModifiedBy>";
        let app = "";
        let file = build_ooxml_package(core, app);
        let props = read_ooxml_docprops(file.path()).unwrap();
        assert_eq!(props.last_modified_by.as_deref(), Some("Jones & Co"));
    }

    #[test]
    fn unparseable_zip_returns_empty_props_not_error() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), b"not a zip").unwrap();
        let props = read_ooxml_docprops(temp.path()).unwrap();
        assert!(props.is_empty());
    }

    #[test]
    fn oversized_package_returns_data_parsing_error() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(temp.path())
            .unwrap();
        file.set_len(MAX_PACKAGE_BYTES + 1).unwrap();
        drop(file);
        assert!(matches!(
            read_ooxml_docprops(temp.path()),
            Err(AppError::DataParsing(_))
        ));
    }

    #[test]
    fn extract_body_ignores_tags_where_the_name_is_only_a_prefix_of_another() {
        // A `<RevNum>` search must not match `<RevNumSuffix>`.
        let xml = "<RevNumSuffix>bogus</RevNumSuffix><RevNum>7</RevNum>";
        assert_eq!(extract_body(xml, "RevNum"), Some("7".to_string()));
    }
}
