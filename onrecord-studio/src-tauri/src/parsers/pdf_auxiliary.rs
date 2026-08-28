//! Deterministic, bounded PDF auxiliary-object collection.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use lopdf::{Document, Object};

const MAXIMUM_OBJECTS: usize = 250_000;
const MAXIMUM_FINDINGS: usize = 10_000;
const MAXIMUM_VALUE_BYTES: usize = 1024 * 1024;
const MAXIMUM_TOTAL_VALUE_BYTES: usize = 16 * 1024 * 1024;
const AUXILIARY_STRING_KEYS: &[&[u8]] = &[
    b"Contents",
    b"ActualText",
    b"Alt",
    b"V",
    b"DV",
    b"TU",
    b"T",
    b"Subj",
    b"Title",
    b"Author",
    b"Subject",
    b"Keywords",
    b"Reason",
    b"Location",
    b"ContactInfo",
];
const STRUCTURAL_KEYS: &[&[u8]] = &[
    b"EmbeddedFiles",
    b"EF",
    b"XFA",
    b"StructTreeRoot",
    b"OCProperties",
];

/// Collects non-page-canvas PDF evidence that may survive an attempted redaction.
pub fn collect_pdf_auxiliary_evidence(
    document: &Document,
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // FEATURE(pdf-audit): Walk outlines, annotation contents/reply chains, AcroForm values,
    // StructTreeRoot ActualText/Alt attributes, page thumbnails, embedded files, XFA, and
    // optional-content relationships. Emit object IDs and page coordinates for every finding.
    // TODO(pdf-audit): Compare each incremental xref/object generation and report orphan objects.
    // NOTE(pdf-audit): A tagged structure tree usually references page content; it is not an
    // independent full text layer. Only present strings should be classified as Preserved.
    if document.objects.len() > MAXIMUM_OBJECTS {
        return Err(AppError::DataParsing(format!(
            "PDF_AUXILIARY_OBJECT_LIMIT_EXCEEDED: document contains more than {MAXIMUM_OBJECTS} objects"
        )));
    }
    let page_numbers = document
        .get_pages()
        .into_iter()
        .map(|(page, object_id)| (object_id, page))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut bundle = EvidenceBundle {
        coverage_complete: true,
        ..EvidenceBundle::default()
    };
    let mut total_value_bytes = 0_usize;
    for (object_id, object) in &document.objects {
        let Some(dictionary) = object_dictionary(object) else {
            continue;
        };
        for key in AUXILIARY_STRING_KEYS {
            let Ok(value) = dictionary.get(key) else {
                continue;
            };
            let mut values = Vec::new();
            collect_strings(document, value, 0, &mut values);
            for text in values {
                if text.is_empty() {
                    continue;
                }
                if text.len() > MAXIMUM_VALUE_BYTES {
                    bundle.coverage_complete = false;
                    bundle.warnings.push(format!(
                        "PDF_AUXILIARY_VALUE_SKIPPED_SIZE:{}:{}",
                        object_id.0,
                        String::from_utf8_lossy(key)
                    ));
                    continue;
                }
                total_value_bytes = total_value_bytes.checked_add(text.len()).ok_or_else(|| {
                    AppError::DataParsing("PDF_AUXILIARY_VALUE_SIZE_OVERFLOW".to_string())
                })?;
                if total_value_bytes > MAXIMUM_TOTAL_VALUE_BYTES
                    || bundle.findings.len() >= MAXIMUM_FINDINGS
                {
                    bundle.coverage_complete = false;
                    bundle
                        .warnings
                        .push("PDF_AUXILIARY_COLLECTION_LIMIT_REACHED".to_string());
                    return Ok(bundle);
                }
                let key_name = String::from_utf8_lossy(key);
                bundle.findings.push(finding_for_text(
                    EvidenceClass::Preserved,
                    EvidenceConfidence::Exact,
                    format!("PDF_AUXILIARY_{}", key_name.to_ascii_uppercase()),
                    EvidenceLocator {
                        object_reference: Some(format!("{} {} R", object_id.0, object_id.1)),
                        page_number: page_numbers.get(object_id).copied(),
                        ..EvidenceLocator::default()
                    },
                    &text,
                    "A string is preserved in a non-canvas PDF dictionary field. Its presence is exact; its meaning depends on the owning PDF object.",
                    options,
                )?);
            }
        }
        for key in STRUCTURAL_KEYS {
            if dictionary.get(key).is_ok() {
                let key_name = String::from_utf8_lossy(key);
                bundle.findings.push(finding_for_text(
                    EvidenceClass::Preserved,
                    EvidenceConfidence::Exact,
                    format!("PDF_STRUCTURE_{}", key_name.to_ascii_uppercase()),
                    EvidenceLocator { object_reference: Some(format!("{} {} R", object_id.0, object_id.1)), ..EvidenceLocator::default() },
                    &key_name,
                    "The PDF object graph preserves this auxiliary structure. The finding does not claim that it contains recoverable hidden text.",
                    options,
                )?);
            }
        }
    }
    Ok(bundle)
}

fn object_dictionary(object: &Object) -> Option<&lopdf::Dictionary> {
    match object {
        Object::Dictionary(dictionary) => Some(dictionary),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

fn resolve_object<'a>(document: &'a Document, mut object: &'a Object) -> &'a Object {
    for _ in 0..16 {
        let Object::Reference(reference) = object else {
            break;
        };
        let Ok(resolved) = document.get_object(*reference) else {
            break;
        };
        object = resolved;
    }
    object
}

fn collect_strings(document: &Document, object: &Object, depth: usize, output: &mut Vec<String>) {
    if depth > 4 || output.len() >= 64 {
        return;
    }
    match resolve_object(document, object) {
        Object::String(bytes, _) => output.push(String::from_utf8_lossy(bytes).into_owned()),
        Object::Array(values) => {
            for value in values {
                collect_strings(document, value, depth + 1, output);
                if output.len() >= 64 {
                    break;
                }
            }
        }
        _ => {}
    }
}

/// Produces a non-destructive preview with selected overlay operators suppressed.
pub fn render_overlay_suppressed_preview(
    _document: &Document,
    _options: ForensicAuditOptions,
) -> Result<Vec<u8>, AppError> {
    // TODO(pdf-audit): Preserve the original object graph, operator indexes, CTMs, clipping,
    // transparency groups, and Form XObject boundaries in the preview manifest.
    // NOTE(pdf-audit): This must return a derived artifact, never overwrite or rewrite evidence.
    Err(AppError::DataParsing(
        "PDF_OVERLAY_PREVIEW_NOT_IMPLEMENTED".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Object, ObjectId};

    #[test]
    fn collects_auxiliary_strings_with_object_attribution() {
        let mut document = Document::with_version("1.7");
        let object_id: ObjectId = document.add_object(dictionary! {
            "Type" => "Annot",
            "Contents" => Object::string_literal("review note"),
        });
        let result = collect_pdf_auxiliary_evidence(&document, ForensicAuditOptions::default())
            .expect("audit");
        assert!(result.coverage_complete);
        let expected_reference = format!("{} {} R", object_id.0, object_id.1);
        assert!(result.findings.iter().any(|finding| {
            finding.kind == "PDF_AUXILIARY_CONTENTS"
                && finding.locator.object_reference.as_deref() == Some(expected_reference.as_str())
        }));
    }
}
