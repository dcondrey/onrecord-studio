//! Deterministic PDF layout side-channel measurement.
//!
//! The goal is to report leaked geometry, not to identify a person's name. Any candidate
//! matching must remain in the separate, opt-in hypothesis layer.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use lopdf::content::Content;
use lopdf::{Document, Object};

const MAXIMUM_PAGES: usize = 25_000;
const MAXIMUM_OPERATIONS: usize = 1_000_000;
const MAXIMUM_FINDINGS: usize = 5_000;
const NOTABLE_TJ_ADJUSTMENT: f64 = 100.0;

pub fn assess_glyph_position_leak(
    document: &Document,
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // Exact text-state matrices, text movement, spacing, scaling, font size, and TJ adjustments
    // are preserved as measurements. Editor padding, justification, rounding, and rewrite
    // behavior are intentionally not converted into a missing-character interval here.
    // NOTE(pdf-audit): A leaked width is preserved side-channel evidence, not recovered text.
    let pages = document.get_pages();
    if pages.len() > MAXIMUM_PAGES {
        return Err(AppError::DataParsing(format!(
            "PDF_LAYOUT_PAGE_LIMIT_EXCEEDED: document contains more than {MAXIMUM_PAGES} pages"
        )));
    }
    let mut bundle = EvidenceBundle {
        coverage_complete: true,
        ..EvidenceBundle::default()
    };
    let mut operation_count = 0_usize;
    for (page_number, page_id) in pages {
        let bytes = match crate::parsers::unredactor::bounded_page_content(document, page_id) {
            Ok(bytes) => bytes,
            Err(error) => {
                bundle.coverage_complete = false;
                bundle
                    .warnings
                    .push(format!("PDF_LAYOUT_PAGE_SKIPPED:{page_number}:{error}"));
                continue;
            }
        };
        let content = match Content::decode(&bytes) {
            Ok(content) => content,
            Err(error) => {
                bundle.coverage_complete = false;
                bundle
                    .warnings
                    .push(format!("PDF_LAYOUT_CONTENT_INVALID:{page_number}:{error}"));
                continue;
            }
        };
        operation_count = operation_count
            .checked_add(content.operations.len())
            .ok_or_else(|| {
                AppError::DataParsing("PDF_LAYOUT_OPERATION_COUNT_OVERFLOW".to_string())
            })?;
        if operation_count > MAXIMUM_OPERATIONS {
            bundle.coverage_complete = false;
            bundle
                .warnings
                .push("PDF_LAYOUT_OPERATION_LIMIT_REACHED".to_string());
            break;
        }
        for (operation_index, operation) in content.operations.iter().enumerate() {
            if operation.operator == "TJ" {
                let Some(Object::Array(values)) = operation.operands.first() else {
                    continue;
                };
                let adjustments = values
                    .iter()
                    .filter_map(object_number)
                    .filter(|value| value.abs() >= NOTABLE_TJ_ADJUSTMENT)
                    .collect::<Vec<_>>();
                if !adjustments.is_empty() {
                    append_layout_finding(
                        &mut bundle,
                        page_number,
                        page_id,
                        operation_index,
                        "PDF_TJ_POSITION_ADJUSTMENT",
                        adjustments
                            .iter()
                            .map(|value| format!("{value:.3}"))
                            .collect::<Vec<_>>()
                            .join(","),
                        "The page content stream preserves notable TJ positioning adjustments. They measure layout spacing and do not identify missing characters.",
                        options,
                    )?;
                }
                continue;
            }
            let (kind, explanation) = match operation.operator.as_str() {
                "Tm" => (
                    "PDF_TEXT_MATRIX_MEASUREMENT",
                    "The content stream preserves an exact six-value text matrix. It locates a text run but does not identify omitted characters.",
                ),
                "Td" | "TD" => (
                    "PDF_TEXT_POSITION_DELTA",
                    "The content stream preserves an exact text-position delta. It is layout evidence, not a reconstruction of missing text.",
                ),
                "Tc" => (
                    "PDF_CHARACTER_SPACING_MEASUREMENT",
                    "The content stream preserves the character-spacing state used by later text operations.",
                ),
                "Tw" => (
                    "PDF_WORD_SPACING_MEASUREMENT",
                    "The content stream preserves the word-spacing state used by later text operations.",
                ),
                "Tz" => (
                    "PDF_HORIZONTAL_TEXT_SCALE_MEASUREMENT",
                    "The content stream preserves the horizontal text scale used by later text operations.",
                ),
                "TL" | "Ts" => (
                    "PDF_TEXT_VERTICAL_STATE_MEASUREMENT",
                    "The content stream preserves a text leading or rise measurement.",
                ),
                "Tf" => (
                    "PDF_FONT_STATE_MEASUREMENT",
                    "The content stream preserves the selected font resource name and size; this does not establish the rendered glyph identity by itself.",
                ),
                _ => continue,
            };
            let measurement = operation
                .operands
                .iter()
                .map(format_operand)
                .collect::<Vec<_>>()
                .join(",");
            append_layout_finding(
                &mut bundle,
                page_number,
                page_id,
                operation_index,
                kind,
                measurement,
                explanation,
                options,
            )?;
        }
    }
    Ok(bundle)
}

#[allow(clippy::too_many_arguments)]
fn append_layout_finding(
    bundle: &mut EvidenceBundle,
    page_number: u32,
    page_id: lopdf::ObjectId,
    operation_index: usize,
    kind: &str,
    measurement: String,
    explanation: &str,
    options: ForensicAuditOptions,
) -> Result<(), AppError> {
    if bundle.findings.len() >= MAXIMUM_FINDINGS {
        bundle.coverage_complete = false;
        if !bundle
            .warnings
            .iter()
            .any(|warning| warning == "PDF_LAYOUT_FINDING_LIMIT_REACHED")
        {
            bundle
                .warnings
                .push("PDF_LAYOUT_FINDING_LIMIT_REACHED".to_string());
        }
        return Ok(());
    }
    bundle.findings.push(finding_for_text(
        EvidenceClass::Preserved,
        EvidenceConfidence::Exact,
        kind,
        EvidenceLocator {
            object_reference: Some(format!(
                "{} {} R operation {operation_index}",
                page_id.0, page_id.1
            )),
            page_number: Some(page_number),
            ..EvidenceLocator::default()
        },
        &measurement,
        explanation,
        options,
    )?);
    Ok(())
}

fn object_number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some((*value).into()),
        _ => None,
    }
}

fn format_operand(object: &Object) -> String {
    match object {
        Object::Integer(value) => value.to_string(),
        Object::Real(value) => format!("{value:.4}"),
        Object::Name(value) => format!("/{}", String::from_utf8_lossy(value)),
        Object::Reference((number, generation)) => format!("{number} {generation} R"),
        _ => "non-scalar".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Stream};

    #[test]
    fn reports_large_tj_adjustment_without_claiming_text_recovery() {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let content = Content {
            operations: vec![
                Operation::new(
                    "Tm",
                    vec![1.into(), 0.into(), 0.into(), 1.into(), 12.into(), 24.into()],
                ),
                Operation::new(
                    "TJ",
                    vec![Object::Array(vec![
                        Object::string_literal("a"),
                        Object::Integer(250),
                        Object::string_literal("b"),
                    ])],
                ),
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
        let result =
            assess_glyph_position_leak(&document, ForensicAuditOptions::default()).expect("audit");
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "PDF_TJ_POSITION_ADJUSTMENT"));
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "PDF_TEXT_MATRIX_MEASUREMENT"));
        assert!(result
            .findings
            .iter()
            .all(|finding| !finding.explanation.contains("recovered text")));
    }
}
