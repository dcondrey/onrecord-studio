//! Bounded OOXML/ODF package archaeology with attributed part-level findings.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use crate::parsers::format::DocumentFormat;
use crate::parsers::source::inspect_regular_file;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

const MAXIMUM_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;
const MAXIMUM_ENTRIES: usize = 10_000;
const MAXIMUM_AUDIT_PART_BYTES: u64 = 8 * 1024 * 1024;
const MAXIMUM_TOTAL_AUDIT_BYTES: u64 = 64 * 1024 * 1024;

const REVISION_MARKERS: &[(&str, &str)] = &[
    ("<w:del", "OFFICE_TRACKED_DELETION"),
    ("<w:ins", "OFFICE_TRACKED_INSERTION"),
    ("<w:moveFrom", "OFFICE_TRACKED_MOVE_FROM"),
    ("<w:moveTo", "OFFICE_TRACKED_MOVE_TO"),
    ("<w:rPrChange", "OFFICE_RUN_PROPERTY_CHANGE"),
    ("<w:pPrChange", "OFFICE_PARAGRAPH_PROPERTY_CHANGE"),
    ("<text:tracked-changes", "ODF_TRACKED_CHANGES"),
];

pub fn audit_office_package<P: AsRef<Path>>(
    path: P,
    format: DocumentFormat,
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // NOTE(office-audit): Stripping rPr is a derived forensic view; do not modify document.xml.
    if !matches!(
        format,
        DocumentFormat::WordPackage
            | DocumentFormat::PresentationPackage
            | DocumentFormat::OpenDocumentText
            | DocumentFormat::OpenDocumentPresentation
    ) {
        return Err(AppError::DataParsing(
            "OFFICE_AUDIT_FORMAT_INVALID: expected an OOXML or ODF package".to_string(),
        ));
    }
    let path = path.as_ref();
    inspect_regular_file(path, "OFFICE_AUDIT", MAXIMUM_PACKAGE_BYTES)?;
    let file = File::open(path)
        .map_err(|error| AppError::Storage(format!("OFFICE_AUDIT_OPEN_FAILED: {error}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| AppError::DataParsing(format!("OFFICE_AUDIT_PACKAGE_INVALID: {error}")))?;
    if archive.len() > MAXIMUM_ENTRIES {
        return Err(AppError::DataParsing(format!(
            "OFFICE_AUDIT_ENTRY_LIMIT_EXCEEDED: package contains more than {MAXIMUM_ENTRIES} parts"
        )));
    }

    let mut bundle = EvidenceBundle {
        coverage_complete: true,
        ..EvidenceBundle::default()
    };
    let mut total_read = 0_u64;
    let mut names = BTreeMap::<String, usize>::new();
    let relationship_pattern =
        regex::Regex::new(r#"<Relationship\b[^>]*\bId="([^"]+)"[^>]*\bTarget="([^"]+)"[^>]*/?>"#)
            .map_err(|error| {
            AppError::Internal(format!("OFFICE_AUDIT_RELATIONSHIP_PATTERN:{error}"))
        })?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            AppError::DataParsing(format!("OFFICE_AUDIT_PART_INVALID at {index}: {error}"))
        })?;
        let name = entry.name().replace('\\', "/");
        *names.entry(name.clone()).or_default() += 1;
        if is_attributed_part(&name) {
            bundle.findings.push(finding_for_text(
                EvidenceClass::Preserved,
                EvidenceConfidence::Exact,
                attributed_part_kind(&name),
                EvidenceLocator { container_part: Some(name.clone()), ..EvidenceLocator::default() },
                &name,
                "The named package part is preserved in the source container and may carry revisions, comments, metadata, or embedded content.",
                options,
            )?);
        }
        if !is_xml_part(&name) {
            continue;
        }
        if entry.size() > MAXIMUM_AUDIT_PART_BYTES {
            bundle.coverage_complete = false;
            bundle
                .warnings
                .push(format!("OFFICE_AUDIT_PART_SKIPPED_SIZE:{name}"));
            continue;
        }
        total_read = total_read
            .checked_add(entry.size())
            .ok_or_else(|| AppError::DataParsing("OFFICE_AUDIT_SIZE_OVERFLOW".to_string()))?;
        if total_read > MAXIMUM_TOTAL_AUDIT_BYTES {
            bundle.coverage_complete = false;
            bundle
                .warnings
                .push("OFFICE_AUDIT_TOTAL_XML_LIMIT_REACHED".to_string());
            break;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|error| {
            AppError::DataParsing(format!("OFFICE_AUDIT_PART_READ_FAILED:{name}:{error}"))
        })?;
        let Ok(xml) = std::str::from_utf8(&bytes) else {
            bundle.coverage_complete = false;
            bundle
                .warnings
                .push(format!("OFFICE_AUDIT_XML_NOT_UTF8:{name}"));
            continue;
        };
        for (marker, kind) in REVISION_MARKERS {
            if xml.contains(marker) {
                bundle.findings.push(finding_for_text(
                    EvidenceClass::Preserved,
                    EvidenceConfidence::Exact,
                    *kind,
                    EvidenceLocator { container_part: Some(name.clone()), xml_path: Some((*marker).to_string()), ..EvidenceLocator::default() },
                    &format!("{name}:{marker}"),
                    "A tracked-change element is present in this package part. This records structure, not an interpretation of the edited text.",
                    options,
                )?);
            }
        }
        if name.to_ascii_lowercase().ends_with(".rels") {
            for capture in relationship_pattern.captures_iter(xml) {
                let relationship_id = capture
                    .get(1)
                    .map(|value| value.as_str())
                    .unwrap_or_default();
                let target = capture
                    .get(2)
                    .map(|value| value.as_str())
                    .unwrap_or_default();
                bundle.findings.push(finding_for_text(
                    EvidenceClass::Preserved,
                    EvidenceConfidence::Exact,
                    "OFFICE_PACKAGE_RELATIONSHIP",
                    EvidenceLocator { container_part: Some(name.clone()), xml_path: Some(format!("Relationship[@Id='{relationship_id}']")), ..EvidenceLocator::default() },
                    &format!("{relationship_id} -> {target}"),
                    "The relationship identifier and target are preserved in the package relationship part. External targets are reported but never fetched.",
                    options,
                )?);
            }
        }
    }
    for (name, count) in names.into_iter().filter(|(_, count)| *count > 1) {
        bundle.findings.push(finding_for_text(
            EvidenceClass::Preserved,
            EvidenceConfidence::Exact,
            "OFFICE_DUPLICATE_PART_NAME",
            EvidenceLocator { container_part: Some(name.clone()), ..EvidenceLocator::default() },
            &format!("{name} occurs {count} times"),
            "Duplicate ZIP part names can cause Office consumers to select different preserved objects.",
            options,
        )?);
    }
    Ok(bundle)
}

fn is_xml_part(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".xml") || lower.ends_with(".rels")
}

fn is_attributed_part(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("customxml/")
        || lower.contains("/comments")
        || lower.ends_with(".rels")
        || lower.contains("/embeddings/")
        || lower.contains("/glossary/")
        || lower.ends_with("people.xml")
        || lower.starts_with("docprops/")
}

fn attributed_part_kind(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("customxml/") {
        "OFFICE_CUSTOM_XML_PART"
    } else if lower.contains("/comments") {
        "OFFICE_COMMENT_PART"
    } else if lower.contains("/embeddings/") {
        "OFFICE_EMBEDDED_OBJECT"
    } else if lower.contains("/glossary/") {
        "OFFICE_GLOSSARY_PART"
    } else if lower.ends_with("people.xml") {
        "OFFICE_PEOPLE_METADATA"
    } else if lower.ends_with(".rels") {
        "OFFICE_RELATIONSHIP_PART"
    } else {
        "OFFICE_DOCUMENT_PROPERTIES"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn attributes_tracked_deletion_to_its_package_part() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("tracked.docx");
        let file = File::create(&path).expect("create");
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("part");
        archive
            .write_all(b"<w:document><w:del><w:delText>removed</w:delText></w:del></w:document>")
            .expect("xml");
        archive
            .start_file(
                "word/_rels/document.xml.rels",
                zip::write::SimpleFileOptions::default(),
            )
            .expect("rels part");
        archive
            .write_all(br#"<Relationships><Relationship Id="rId7" Target="comments.xml"/></Relationships>"#)
            .expect("rels xml");
        archive.finish().expect("finish");

        let result = audit_office_package(
            &path,
            DocumentFormat::WordPackage,
            ForensicAuditOptions::default(),
        )
        .expect("audit");
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "OFFICE_TRACKED_DELETION"
                && finding.locator.container_part.as_deref() == Some("word/document.xml")));
        assert!(result.findings.iter().any(|finding| {
            finding.kind == "OFFICE_PACKAGE_RELATIONSHIP"
                && finding.locator.xml_path.as_deref() == Some("Relationship[@Id='rId7']")
        }));
    }
}
