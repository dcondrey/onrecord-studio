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
    // FEATURE(pdf-audit): Compare adjacent glyph/text-run positions, text-state matrices,
    // TJ adjustments, character spacing, and embedded font metrics around redaction bounds.
    // TODO(pdf-audit): Model editor-specific padding, line justification, CTM rounding, and
    // workflow rewrites before reporting a usable interval for the missing span.
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
            if operation.operator != "TJ" {
                continue;
            }
            let Some(Object::Array(values)) = operation.operands.first() else {
                continue;
            };
            let adjustments = values
                .iter()
                .filter_map(object_number)
                .filter(|value| value.abs() >= NOTABLE_TJ_ADJUSTMENT)
                .collect::<Vec<_>>();
            if adjustments.is_empty() {
                continue;
            }
            if bundle.findings.len() >= MAXIMUM_FINDINGS {
                bundle.coverage_complete = false;
                bundle
                    .warnings
                    .push("PDF_LAYOUT_FINDING_LIMIT_REACHED".to_string());
                return Ok(bundle);
            }
            let measurement = adjustments
                .iter()
                .map(|value| format!("{value:.3}"))
                .collect::<Vec<_>>()
                .join(",");
            bundle.findings.push(finding_for_text(
                EvidenceClass::Preserved,
                EvidenceConfidence::Exact,
                "PDF_TJ_POSITION_ADJUSTMENT",
                EvidenceLocator {
                    object_reference: Some(format!("{} {} R operation {operation_index}", page_id.0, page_id.1)),
                    page_number: Some(page_number),
                    ..EvidenceLocator::default()
                },
                &measurement,
                "The page content stream preserves notable TJ positioning adjustments. They measure layout spacing and do not identify missing characters.",
                options,
            )?);
        }
    }
    Ok(bundle)
}

fn object_number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some((*value).into()),
        _ => None,
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
            operations: vec![Operation::new(
                "TJ",
                vec![Object::Array(vec![
                    Object::string_literal("a"),
                    Object::Integer(250),
                    Object::string_literal("b"),
                ])],
            )],
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
            .all(|finding| !finding.explanation.contains("recovered text")));
    }
}
