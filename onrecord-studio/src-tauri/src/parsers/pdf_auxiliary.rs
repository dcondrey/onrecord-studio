//! Deterministic, bounded PDF auxiliary-object collection.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use lopdf::content::Content;
use lopdf::{Document, Object, ObjectId};
use std::collections::{BTreeMap, HashSet};

const MAXIMUM_OBJECTS: usize = 250_000;
const MAXIMUM_FINDINGS: usize = 10_000;
const MAXIMUM_VALUE_BYTES: usize = 1024 * 1024;
const MAXIMUM_TOTAL_VALUE_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_GRAPH_DEPTH: usize = 64;
const MAXIMUM_PREVIEW_OPERATIONS: usize = 1_000_000;
const MAXIMUM_PREVIEW_ENTRIES: usize = 10_000;
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
            return values.iter().fold(true, |complete, value| {
                collect_graph_references(value, depth + 1, pending) && complete
            });
        }
        Object::Dictionary(dictionary) => {
            return dictionary.iter().fold(true, |complete, (_, value)| {
                collect_graph_references(value, depth + 1, pending) && complete
            });
        }
        Object::Stream(stream) => {
            return stream.dict.iter().fold(true, |complete, (_, value)| {
                collect_graph_references(value, depth + 1, pending) && complete
            });
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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySuppressionManifest {
    pub document_version: String,
    pub entries: Vec<OverlayOperationEvidence>,
    pub coverage_complete: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayOperationEvidence {
    pub page_number: u32,
    pub page_object_reference: String,
    pub operation_index: usize,
    pub operator: String,
    pub operands: Vec<String>,
    pub current_transform: [f64; 6],
    pub clipping_active: bool,
    pub active_graphics_state: Option<String>,
    pub rectangle: Option<[f64; 4]>,
    pub form_xobject_name: Option<String>,
}

/// Builds a non-destructive manifest for a future derived preview. It identifies the exact paint
/// operations a preview renderer may suppress, but never rewrites the evidence PDF or claims that
/// suppressing an overlay restores content that is not present in another source object.
pub fn build_overlay_suppression_manifest(
    document: &Document,
) -> Result<OverlaySuppressionManifest, AppError> {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    let mut coverage_complete = true;
    let mut inspected_operations = 0_usize;
    for (page_number, page_id) in document.get_pages() {
        let bytes = match crate::parsers::unredactor::bounded_page_content(document, page_id) {
            Ok(bytes) => bytes,
            Err(error) => {
                coverage_complete = false;
                warnings.push(format!("PDF_OVERLAY_PAGE_SKIPPED:{page_number}:{error}"));
                continue;
            }
        };
        let content = match Content::decode(&bytes) {
            Ok(content) => content,
            Err(error) => {
                coverage_complete = false;
                warnings.push(format!("PDF_OVERLAY_CONTENT_INVALID:{page_number}:{error}"));
                continue;
            }
        };
        let mut transform = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let mut stack = Vec::new();
        let mut clipping_active = false;
        let mut active_graphics_state = None;
        let mut rectangle = None;
        for (operation_index, operation) in content.operations.iter().enumerate() {
            inspected_operations += 1;
            if inspected_operations > MAXIMUM_PREVIEW_OPERATIONS {
                coverage_complete = false;
                warnings.push("PDF_OVERLAY_OPERATION_LIMIT_REACHED".to_string());
                break;
            }
            match operation.operator.as_str() {
                "q" => stack.push((transform, clipping_active, active_graphics_state.clone())),
                "Q" => {
                    if let Some(state) = stack.pop() {
                        (transform, clipping_active, active_graphics_state) = state;
                    } else {
                        coverage_complete = false;
                        warnings.push(format!(
                            "PDF_OVERLAY_GRAPHICS_STACK_UNDERFLOW:{page_number}:{operation_index}"
                        ));
                    }
                }
                "cm" => {
                    if let Some(matrix) = numeric_operands(&operation.operands, 6) {
                        transform = multiply_transform(transform, matrix);
                    }
                }
                "W" | "W*" => clipping_active = true,
                "gs" => {
                    active_graphics_state = operation
                        .operands
                        .first()
                        .and_then(|value| value.as_name().ok())
                        .map(|value| String::from_utf8_lossy(value).into_owned());
                }
                "re" => rectangle = numeric_operands(&operation.operands, 4),
                "n" => rectangle = None,
                _ => {}
            }
            let painted_rectangle = matches!(
                operation.operator.as_str(),
                "f" | "F" | "f*" | "B" | "B*" | "b" | "b*"
            ) && rectangle.is_some();
            let form_xobject_name = (operation.operator == "Do")
                .then(|| {
                    operation
                        .operands
                        .first()
                        .and_then(|value| value.as_name().ok())
                        .map(|value| String::from_utf8_lossy(value).into_owned())
                })
                .flatten();
            if painted_rectangle || form_xobject_name.is_some() {
                if entries.len() >= MAXIMUM_PREVIEW_ENTRIES {
                    coverage_complete = false;
                    warnings.push("PDF_OVERLAY_ENTRY_LIMIT_REACHED".to_string());
                    break;
                }
                entries.push(OverlayOperationEvidence {
                    page_number,
                    page_object_reference: format!("{} {} R", page_id.0, page_id.1),
                    operation_index,
                    operator: operation.operator.clone(),
                    operands: operation.operands.iter().map(format_object).collect(),
                    current_transform: transform,
                    clipping_active,
                    active_graphics_state: active_graphics_state.clone(),
                    rectangle,
                    form_xobject_name,
                });
            }
            if matches!(
                operation.operator.as_str(),
                "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "S" | "s"
            ) {
                rectangle = None;
            }
        }
    }
    Ok(OverlaySuppressionManifest {
        document_version: document.version.clone(),
        entries,
        coverage_complete,
        warnings,
    })
}

fn numeric_operands<const N: usize>(operands: &[Object], count: usize) -> Option<[f64; N]> {
    if count != N || operands.len() < N {
        return None;
    }
    let mut values = [0.0; N];
    for (index, object) in operands.iter().take(N).enumerate() {
        values[index] = match object {
            Object::Integer(value) => *value as f64,
            Object::Real(value) => (*value).into(),
            _ => return None,
        };
    }
    Some(values)
}

fn multiply_transform(left: [f64; 6], right: [f64; 6]) -> [f64; 6] {
    [
        left[0] * right[0] + left[2] * right[1],
        left[1] * right[0] + left[3] * right[1],
        left[0] * right[2] + left[2] * right[3],
        left[1] * right[2] + left[3] * right[3],
        left[0] * right[4] + left[2] * right[5] + left[4],
        left[1] * right[4] + left[3] * right[5] + left[5],
    ]
}

fn format_object(object: &Object) -> String {
    match object {
        Object::Integer(value) => value.to_string(),
        Object::Real(value) => format!("{value:.4}"),
        Object::Name(value) => format!("/{}", String::from_utf8_lossy(value)),
        Object::Reference((number, generation)) => format!("{number} {generation} R"),
        Object::String(value, _) => format!("stringBytes={}", value.len()),
        Object::Array(values) => format!("arrayItems={}", values.len()),
        Object::Dictionary(values) => format!("dictionaryEntries={}", values.len()),
        Object::Stream(stream) => format!("streamBytes={}", stream.content.len()),
        Object::Boolean(value) => value.to_string(),
        Object::Null => "null".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Object, ObjectId, Stream};

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
        assert!(result.findings.iter().any(|finding| {
            finding.kind == "PDF_UNREFERENCED_OBJECT"
                && finding.locator.object_reference.as_deref() == Some(expected_reference.as_str())
        }));
    }

    #[test]
    fn builds_attributed_overlay_suppression_manifest() {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![1.into(), 0.into(), 0.into(), 1.into(), 12.into(), 24.into()],
                ),
                Operation::new("re", vec![0.into(), 0.into(), 120.into(), 18.into()]),
                Operation::new("W", vec![]),
                Operation::new("f", vec![]),
                Operation::new("Do", vec![Object::Name(b"MaskForm".to_vec())]),
                Operation::new("Q", vec![]),
            ],
        };
        let content_id = document.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("encode"),
        ));
        let page_id = document.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content_id, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()] });
        document.objects.insert(
            pages_id,
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 }.into(),
        );
        let catalog_id =
            document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        document.trailer.set("Root", catalog_id);

        let manifest = build_overlay_suppression_manifest(&document).expect("manifest");
        assert!(manifest.coverage_complete);
        assert!(manifest.entries.iter().any(|entry| {
            entry.operator == "f"
                && entry.rectangle == Some([0.0, 0.0, 120.0, 18.0])
                && entry.current_transform == [1.0, 0.0, 0.0, 1.0, 12.0, 24.0]
                && entry.clipping_active
        }));
        assert!(manifest.entries.iter().any(|entry| {
            entry.form_xobject_name.as_deref() == Some("MaskForm")
                && entry.page_object_reference == format!("{} {} R", page_id.0, page_id.1)
        }));
    }
}
