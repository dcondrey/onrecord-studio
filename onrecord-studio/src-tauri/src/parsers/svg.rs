//! Bounded SVG text extraction.

use crate::error::AppError;
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
/// This extractor deliberately makes no visibility or redaction-integrity claim.
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
}
