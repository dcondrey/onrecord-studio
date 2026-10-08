//! Deterministic, bounded PDF auxiliary-object collection.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use crate::parsers::pdf::{decode_text_string, resolve_object};
use lopdf::{Document, Object, ObjectId};
use std::collections::{BTreeMap, HashSet};

const MAXIMUM_OBJECTS: usize = 250_000;
const MAXIMUM_FINDINGS: usize = 10_000;
const MAXIMUM_VALUE_BYTES: usize = 1024 * 1024;
const MAXIMUM_TOTAL_VALUE_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_GRAPH_DEPTH: usize = 64;
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
    b"Outlines",
    b"Annots",
    b"AcroForm",
    b"EmbeddedFiles",
    b"EF",
    b"Thumb",
    b"XFA",
    b"StructTreeRoot",
    b"OCProperties",
];

/// Collects non-page-canvas PDF evidence that may survive an attempted redaction.
pub fn collect_pdf_auxiliary_evidence(
    document: &Document,
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // Outlines, annotation strings/replies, AcroForm values, structure-tree alternatives, page
    // thumbnails, embedded-file relationships, XFA, and optional-content structures are walked
    // through the shared string/structure keys. Object IDs, generations, page ownership, and
    // graph reachability remain attributed rather than being flattened into a second text layer.
    // NOTE(pdf-audit): A tagged structure tree usually references page content; it is not an
    // independent full text layer. Only present strings should be classified as Preserved.
    if document.objects.len() > MAXIMUM_OBJECTS {
        return Err(AppError::DataParsing(format!(
            "PDF_AUXILIARY_OBJECT_LIMIT_EXCEEDED: document contains more than {MAXIMUM_OBJECTS} objects"
        )));
    }
    let page_numbers = collect_page_ownership(document);
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
    let graph = inspect_object_graph(document);
    if !graph.traversal_complete {
        bundle.coverage_complete = false;
        bundle
            .warnings
            .push("PDF_OBJECT_GRAPH_TRAVERSAL_INCOMPLETE".to_string());
    }
    for (object_id, object) in &document.objects {
        if object_id.1 > 0 {
            append_structural_object_finding(
                &mut bundle,
                *object_id,
                page_numbers.get(object_id).copied(),
                "PDF_NONZERO_OBJECT_GENERATION",
                format!("generation={}", object_id.1),
                "The loaded cross-reference chain preserves a nonzero object generation. This is revision evidence, not proof that the object contains superseded text.",
                options,
            )?;
        }
        if !graph.reachable.contains(object_id) {
            append_structural_object_finding(
                &mut bundle,
                *object_id,
                page_numbers.get(object_id).copied(),
                if matches!(object, Object::Stream(_)) {
                    "PDF_UNREFERENCED_STREAM"
                } else {
                    "PDF_UNREFERENCED_OBJECT"
                },
                object.type_name().map(|name| String::from_utf8_lossy(name).into_owned()).unwrap_or_else(|_| "untyped".to_string()),
                "The object is loaded from the PDF cross-reference data but is unreachable from the active trailer graph. It may be residual revision data; no hidden-content claim is made.",
                options,
            )?;
        }
    }
    Ok(bundle)
}

fn append_structural_object_finding(
    bundle: &mut EvidenceBundle,
    object_id: ObjectId,
    page_number: Option<u32>,
    kind: &str,
    value: String,
    explanation: &str,
    options: ForensicAuditOptions,
) -> Result<(), AppError> {
    if bundle.findings.len() >= MAXIMUM_FINDINGS {
        bundle.coverage_complete = false;
        if !bundle
            .warnings
            .iter()
            .any(|warning| warning == "PDF_AUXILIARY_COLLECTION_LIMIT_REACHED")
        {
            bundle
                .warnings
                .push("PDF_AUXILIARY_COLLECTION_LIMIT_REACHED".to_string());
        }
        return Ok(());
    }
    bundle.findings.push(finding_for_text(
        EvidenceClass::Preserved,
        EvidenceConfidence::Exact,
        kind,
        EvidenceLocator {
            object_reference: Some(format!("{} {} R", object_id.0, object_id.1)),
            page_number,
            ..EvidenceLocator::default()
        },
        &value,
        explanation,
        options,
    )?);
    Ok(())
}

fn collect_page_ownership(document: &Document) -> BTreeMap<ObjectId, u32> {
    let mut owners = BTreeMap::new();
    for (page, page_id) in document.get_pages() {
        owners.insert(page_id, page);
        let Ok(dictionary) = document.get_dictionary(page_id) else {
            continue;
        };
        for key in [b"Annots".as_slice(), b"Thumb", b"Contents"] {
            let Ok(value) = dictionary.get(key) else {
                continue;
            };
            collect_owned_references(value, page, 0, &mut owners);
        }
    }
    owners
}

fn collect_owned_references(
    object: &Object,
    page: u32,
    depth: usize,
    owners: &mut BTreeMap<ObjectId, u32>,
) {
    if depth > 8 {
        return;
    }
    match object {
        Object::Reference(object_id) => {
            owners.insert(*object_id, page);
        }
        Object::Array(values) => {
            for value in values {
                collect_owned_references(value, page, depth + 1, owners);
            }
        }
        _ => {}
    }
}

struct ObjectGraphInspection {
    reachable: HashSet<ObjectId>,
    traversal_complete: bool,
}

fn inspect_object_graph(document: &Document) -> ObjectGraphInspection {
    let mut reachable = HashSet::new();
    let mut pending = Vec::new();
    let mut traversal_complete = collect_graph_references(
        &Object::Dictionary(document.trailer.clone()),
        0,
        &mut pending,
    );
    while let Some(object_id) = pending.pop() {
        if !reachable.insert(object_id) {
            continue;
        }
        let Some(object) = document.objects.get(&object_id) else {
            traversal_complete = false;
            continue;
        };
        traversal_complete &= collect_graph_references(object, 0, &mut pending);
    }
    ObjectGraphInspection {
        reachable,
        traversal_complete,
    }
}

fn collect_graph_references(object: &Object, depth: usize, pending: &mut Vec<ObjectId>) -> bool {
    if depth > MAXIMUM_GRAPH_DEPTH {
        return false;
    }
    match object {
        Object::Reference(object_id) => pending.push(*object_id),
        Object::Array(values) => {
            let mut complete = true;
            for value in values {
                complete = collect_graph_references(value, depth + 1, pending) && complete;
            }
            return complete;
        }
        Object::Dictionary(dictionary) => {
            let mut complete = true;
            for (_, value) in dictionary {
                complete = collect_graph_references(value, depth + 1, pending) && complete;
            }
            return complete;
        }
        Object::Stream(stream) => {
            let mut complete = true;
            for (_, value) in &stream.dict {
                complete = collect_graph_references(value, depth + 1, pending) && complete;
            }
            return complete;
        }
        _ => {}
    }
    true
}

fn object_dictionary(object: &Object) -> Option<&lopdf::Dictionary> {
    match object {
        Object::Dictionary(dictionary) => Some(dictionary),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

fn collect_strings(document: &Document, object: &Object, depth: usize, output: &mut Vec<String>) {
    if depth > 4 || output.len() >= 64 {
        return;
    }
    match resolve_object(document, object) {
        Some(Object::String(bytes, _)) => output.push(decode_text_string(bytes)),
        Some(Object::Array(values)) => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Object, ObjectId};

    #[test]
    fn collects_auxiliary_strings_with_object_attribution() {
        let mut document = Document::with_version("1.7");
        let object_id: ObjectId = document.add_object(dictionary! {
            "Type" => "Annot",
            // Acrobat writes annotation text as UTF-16BE behind a byte-order mark.
            "Contents" => Object::String(
                [0xfe, 0xff]
                    .into_iter()
                    .chain("review note".encode_utf16().flat_map(u16::to_be_bytes))
                    .collect(),
                lopdf::StringFormat::Literal,
            ),
        });
        let result = collect_pdf_auxiliary_evidence(&document, ForensicAuditOptions::default())
            .expect("audit");
        assert!(result.coverage_complete);
        let expected_reference = format!("{} {} R", object_id.0, object_id.1);
        assert!(result.findings.iter().any(|finding| {
            finding.kind == "PDF_AUXILIARY_CONTENTS"
                && finding.locator.object_reference.as_deref() == Some(expected_reference.as_str())
                && finding.masked_preview.as_deref() == Some("review note")
        }));
        assert!(result.findings.iter().any(|finding| {
            finding.kind == "PDF_UNREFERENCED_OBJECT"
                && finding.locator.object_reference.as_deref() == Some(expected_reference.as_str())
        }));
    }
}
