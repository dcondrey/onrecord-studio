//! SVG text extraction and conservative static visibility inspection.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use crate::parsers::source::read_regular_file;
use scraper::{ElementRef, Html};
use std::path::Path;

const MAX_SVG_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SVG_TEXT_BYTES: usize = 4 * 1024 * 1024;

// Code, not evidence. Unlike the HTML extractor's list this one does NOT contain `svg`: there the
// SVG subtree is an illustration inside a page, here it is the whole document.
const IGNORED_SVG_ELEMENTS: &[&str] = &["script", "style"];

/// Extracts readable text from an SVG document: `<text>`/`<tspan>` labels together with the
/// `<title>`, `<desc>`, and `<metadata>` strings.
///
/// NOTE(security): the html5ever tree builder resolves no DTDs and expands no entities, so an SVG
/// declaring `<!ENTITY x SYSTEM "file:///etc/passwd">` cannot read local files or reach the
/// network, and no billion-laughs expansion is reachable. Scripts are never executed.
///
/// LIMIT(svg-audit): this establishes text *presence*, not text *visibility*. Characters hidden
/// under an opaque rectangle, pushed off-canvas by a transform, clipped away, or painted at zero
/// opacity are extracted exactly like visible ones. Deciding what a reader actually saw is the
/// separate audit tracked in `audit_svg_file` below.
pub fn extract_svg_text<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    let path = file_path.as_ref();
    let bytes = read_regular_file(path, "SVG", MAX_SVG_SOURCE_BYTES)?;
    let source = String::from_utf8_lossy(&bytes);
    let document = Html::parse_document(&source);
    let mut output = String::with_capacity(source.len().min(MAX_SVG_TEXT_BYTES));

    for node in document.root_element().descendants() {
        let Some(text) = node.value().as_text() else {
            continue;
        };
        if node.ancestors().any(|ancestor| {
            ElementRef::wrap(ancestor)
                .is_some_and(|element| IGNORED_SVG_ELEMENTS.contains(&element.value().name()))
        }) {
            continue;
        }
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.is_empty() {
            continue;
        }
        let separator = usize::from(!output.is_empty());
        let required = output
            .len()
            .saturating_add(separator)
            .saturating_add(normalized.len());
        if required > MAX_SVG_TEXT_BYTES {
            return Err(AppError::DataParsing(format!(
                "SVG_OUTPUT_SIZE_EXCEEDED: '{}' exceeds {MAX_SVG_TEXT_BYTES} extracted bytes",
                path.display()
            )));
        }
        if separator == 1 {
            output.push('\n');
        }
        output.push_str(&normalized);
    }

    // A graphic-only SVG legitimately carries no text. Reporting that as a specific parse failure
    // -- rather than silently indexing an empty document -- keeps it in the salvage path, which is
    // where the previous `Unknown` classification never let it go.
    if output.is_empty() {
        return Err(AppError::DataParsing(format!(
            "SVG_TEXT_EMPTY: '{}' contains no extractable text",
            path.display()
        )));
    }
    Ok(output)
}

pub fn audit_svg_file<P: AsRef<Path>>(
    path: P,
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    let bytes = read_regular_file(path.as_ref(), "SVG_AUDIT", MAX_SVG_SOURCE_BYTES)?;
    let source = String::from_utf8_lossy(&bytes);
    let document = Html::parse_document(&source);
    let mut bundle = EvidenceBundle {
        coverage_complete: false,
        warnings: vec!["SVG_GEOMETRIC_OCCLUSION_NOT_EVALUATED".to_string()],
        ..EvidenceBundle::default()
    };
    let mut text_index = 0_usize;
    for node in document.root_element().descendants() {
        let Some(element) = ElementRef::wrap(node) else {
            continue;
        };
        if !matches!(
            element.value().name(),
            "text" | "tspan" | "title" | "desc" | "metadata"
        ) {
            continue;
        }
        text_index += 1;
        let value = element
            .text()
            .flat_map(str::split_whitespace)
            .collect::<Vec<_>>()
            .join(" ");
        if value.is_empty() {
            continue;
        }
        let hidden_reason = element
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find_map(hidden_reason);
        if let Some(reason) = hidden_reason {
            bundle.findings.push(finding_for_text(
                EvidenceClass::Preserved,
                EvidenceConfidence::Exact,
                "SVG_TEXT_EXPLICITLY_HIDDEN",
                EvidenceLocator { xml_path: Some(format!("svg/{}[{text_index}]", element.value().name())), ..EvidenceLocator::default() },
                &value,
                format!("Text is preserved under a static SVG visibility rule: {reason}. Scripted styles, transforms, clipping, masks, and geometric overlays are not interpreted."),
                options,
            )?);
        }
        for attribute in [
            "x",
            "y",
            "dx",
            "dy",
            "textLength",
            "lengthAdjust",
            "transform",
            "mask",
            "clip-path",
            "filter",
            "opacity",
            "fill-opacity",
            "fill",
            "stroke",
        ] {
            if let Some(raw) = element.value().attr(attribute) {
                bundle.findings.push(finding_for_text(
                    EvidenceClass::Preserved,
                    EvidenceConfidence::Exact,
                    format!("SVG_TEXT_{}_REFERENCE", attribute.replace('-', "_").to_ascii_uppercase()),
                    EvidenceLocator { xml_path: Some(format!("svg/{}[{text_index}]", element.value().name())), ..EvidenceLocator::default() },
                    raw,
                    "The text element preserves this geometry, paint, transform, mask, clip, or opacity attribute. Its source order is retained by the locator index; effects requiring full CSS or path rendering are not inferred.",
                    options,
                )?);
            }
        }
    }
    let cover_selector = scraper::Selector::parse("rect, path, polygon, image, use")
        .map_err(|error| AppError::Internal(format!("SVG cover selector failed: {error}")))?;
    for (index, element) in document.select(&cover_selector).enumerate() {
        let attributes = element.value();
        let summary = [
            "x",
            "y",
            "width",
            "height",
            "d",
            "points",
            "transform",
            "mask",
            "clip-path",
            "opacity",
            "fill",
        ]
        .into_iter()
        .filter_map(|attribute| {
            attributes
                .attr(attribute)
                .map(|value| format!("{attribute}={value}"))
        })
        .collect::<Vec<_>>()
        .join(";");
        if summary.is_empty() {
            continue;
        }
        bundle.findings.push(finding_for_text(
            EvidenceClass::Preserved,
            EvidenceConfidence::Exact,
            "SVG_POTENTIAL_COVERING_ELEMENT",
            EvidenceLocator { xml_path: Some(format!("svg/{}[{index}]", attributes.name())), ..EvidenceLocator::default() },
            &summary,
            "A source-ordered shape or image preserves geometry that may cover text. The audit records it without claiming occlusion unless a full renderer confirms the visual result.",
            options,
        )?);
    }
    Ok(bundle)
}

fn hidden_reason(element: ElementRef<'_>) -> Option<&'static str> {
    let attributes = element.value();
    if attributes
        .attr("display")
        .is_some_and(|value| value.eq_ignore_ascii_case("none"))
    {
        return Some("display=none");
    }
    if attributes
        .attr("visibility")
        .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "hidden" | "collapse"))
    {
        return Some("visibility is hidden or collapsed");
    }
    if attributes.attr("opacity").is_some_and(is_zero_css_number)
        || attributes
            .attr("fill-opacity")
            .is_some_and(is_zero_css_number)
    {
        return Some("opacity is zero");
    }
    let style = attributes
        .attr("style")?
        .to_ascii_lowercase()
        .replace(' ', "");
    if style.split(';').any(|rule| {
        matches!(
            rule,
            "display:none"
                | "visibility:hidden"
                | "visibility:collapse"
                | "opacity:0"
                | "opacity:0.0"
                | "fill-opacity:0"
                | "fill-opacity:0.0"
        )
    }) {
        return Some("inline style suppresses visibility");
    }
    None
}

fn is_zero_css_number(value: &str) -> bool {
    value.trim().parse::<f64>().is_ok_and(|value| value == 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture(contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("onrecord-svg-{}.svg", uuid::Uuid::new_v4()));
        fs::write(&path, contents).expect("fixture should be written");
        path
    }

    #[test]
    fn extracts_labels_titles_and_descriptions_but_never_script_bodies() {
        let path = fixture(
            r#"<?xml version="1.0" encoding="UTF-8"?>
               <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 200">
                 <title>Redaction Log</title>
                 <desc>Exhibit 14, page 3</desc>
                 <style>.label { fill: black; }</style>
                 <script>doNotIndex("hidden")</script>
                 <rect x="0" y="0" width="400" height="200" fill="white"/>
                 <text x="10" y="40" class="label">Invoice total 10,234</text>
                 <text x="10" y="80"><tspan>Approved by</tspan><tspan> J. Alvarez</tspan></text>
                 <path d="M0 0 L400 200"/>
               </svg>"#,
        );

        let extracted = extract_svg_text(&path).expect("SVG should parse");
        let _ = fs::remove_file(&path);

        assert!(extracted.contains("Redaction Log"), "{extracted}");
        assert!(extracted.contains("Exhibit 14, page 3"), "{extracted}");
        assert!(extracted.contains("Invoice total 10,234"), "{extracted}");
        assert!(extracted.contains("Approved by"), "{extracted}");
        assert!(extracted.contains("J. Alvarez"), "{extracted}");
        assert!(!extracted.contains("doNotIndex"), "{extracted}");
        assert!(!extracted.contains("fill: black"), "{extracted}");
        assert!(!extracted.contains("M0 0 L400 200"), "{extracted}");
    }

    #[test]
    fn declared_external_entities_are_never_resolved() {
        // The classic SVG XXE shape. html5ever ignores the DTD outright, so the reference is
        // either dropped or kept as literal text; either way the file is never opened.
        let path = fixture(
            r#"<?xml version="1.0"?>
               <!DOCTYPE svg [<!ENTITY leak SYSTEM "file:///etc/passwd">]>
               <svg xmlns="http://www.w3.org/2000/svg"><text>start &leak; end</text></svg>"#,
        );

        let extracted = extract_svg_text(&path).expect("SVG should parse");
        let _ = fs::remove_file(&path);

        assert!(extracted.contains("start"), "{extracted}");
        assert!(!extracted.contains("root:"), "{extracted}");
        assert!(!extracted.contains("/bin/"), "{extracted}");
    }

    #[test]
    fn a_graphic_only_svg_reports_an_empty_extraction_rather_than_succeeding_silently() {
        let path = fixture(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/></svg>"#,
        );
        let outcome = extract_svg_text(&path);
        let _ = fs::remove_file(&path);
        assert!(
            matches!(outcome, Err(AppError::DataParsing(message)) if message.contains("SVG_TEXT_EMPTY"))
        );
    }

    #[test]
    fn audit_attributes_statically_hidden_text_without_claiming_full_visibility_coverage() {
        let path = fixture(
            r#"<svg><g style="display:none"><text x="4" transform="translate(2 3)">preserved note</text></g><rect x="0" y="0" width="50" height="12" opacity="0.9"/></svg>"#,
        );
        let result = audit_svg_file(&path, ForensicAuditOptions::default()).expect("audit");
        let _ = fs::remove_file(&path);
        assert!(!result.coverage_complete);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "SVG_TEXT_EXPLICITLY_HIDDEN"
                && finding.masked_preview.as_deref() == Some("preserved note")));
        assert!(result
            .warnings
            .contains(&"SVG_GEOMETRIC_OCCLUSION_NOT_EVALUATED".to_string()));
        assert!(result.findings.iter().any(|finding| {
            finding.kind == "SVG_TEXT_TRANSFORM_REFERENCE"
                && finding.masked_preview.as_deref() == Some("translate(2 3)")
        }));
        assert!(result.findings.iter().any(|finding| {
            finding.kind == "SVG_POTENTIAL_COVERING_ELEMENT"
                && finding
                    .masked_preview
                    .as_deref()
                    .is_some_and(|preview| preview.contains("width=50"))
        }));
    }
}
