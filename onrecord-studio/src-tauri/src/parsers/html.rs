use std::path::Path;

use scraper::{ElementRef, Html, Selector};

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use crate::parsers::source::read_regular_file;

const MAX_HTML_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_HTML_TEXT_BYTES: usize = 32 * 1024 * 1024;
const IGNORED_ELEMENTS: &[&str] = &["script", "style", "noscript", "template", "svg"];

/// Extracts human-visible text from an HTML document without indexing markup,
/// scripts, styles, or hidden template content.
pub fn extract_html_text<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    // NOTE(html-audit): Never execute scripts or fetch remote resources while inspecting HTML.
    let path = file_path.as_ref();
    let bytes = read_regular_file(path, "HTML", MAX_HTML_SOURCE_BYTES)?;
    let source = String::from_utf8_lossy(&bytes);
    let document = Html::parse_document(&source);
    let body_selector = Selector::parse("body").expect("the static body selector is valid");
    let root = document
        .select(&body_selector)
        .next()
        .unwrap_or_else(|| document.root_element());
    let mut output = String::with_capacity(source.len().min(MAX_HTML_TEXT_BYTES));

    for node in root.descendants() {
        let Some(text) = node.value().as_text() else {
            continue;
        };
        if node.ancestors().any(|ancestor| {
            ElementRef::wrap(ancestor)
                .is_some_and(|element| IGNORED_ELEMENTS.contains(&element.value().name()))
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
        if required > MAX_HTML_TEXT_BYTES {
            return Err(AppError::DataParsing(format!(
                "HTML_OUTPUT_SIZE_EXCEEDED: '{}' exceeds {MAX_HTML_TEXT_BYTES} extracted bytes",
                path.display()
            )));
        }
        if separator == 1 {
            output.push('\n');
        }
        output.push_str(&normalized);
    }

    if output.is_empty() {
        return Err(AppError::DataParsing(format!(
            "HTML_VISIBLE_TEXT_EMPTY: '{}' contains no visible text",
            path.display()
        )));
    }
    Ok(output)
}

/// Source-preserving HTML audit. It records preserved comments, metadata, accessibility labels,
/// form values, and statically hidden element text without executing CSS or JavaScript.
pub fn audit_html_file<P: AsRef<Path>>(
    file_path: P,
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    const MAX_FINDINGS: usize = 5_000;
    let path = file_path.as_ref();
    let bytes = read_regular_file(path, "HTML_AUDIT", MAX_HTML_SOURCE_BYTES)?;
    let source = String::from_utf8_lossy(&bytes);
    let document = Html::parse_document(&source);
    let mut bundle = EvidenceBundle {
        coverage_complete: true,
        ..EvidenceBundle::default()
    };

    let comment_pattern = regex::Regex::new(r"(?s)<!--(.*?)-->")
        .map_err(|error| AppError::Internal(format!("HTML comment pattern failed: {error}")))?;
    for capture in comment_pattern.captures_iter(&source) {
        let Some(value) = capture
            .get(1)
            .map(|value| value.as_str().trim())
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        push_html_finding(
            &mut bundle,
            "HTML_COMMENT",
            value,
            None,
            options,
            MAX_FINDINGS,
        )?;
    }

    let selector = Selector::parse("[hidden], [aria-hidden='true'], input[type='hidden'], [style], meta[content], [aria-label], [title], input[value], textarea")
        .map_err(|error| AppError::Internal(format!("HTML audit selector failed: {error}")))?;
    for (index, element) in document.select(&selector).enumerate() {
        let attributes = element.value();
        let style = attributes
            .attr("style")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .replace(' ', "");
        let statically_hidden = attributes.attr("hidden").is_some()
            || attributes
                .attr("aria-hidden")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
            || attributes
                .attr("type")
                .is_some_and(|value| value.eq_ignore_ascii_case("hidden"))
            || style.split(';').any(|rule| {
                matches!(
                    rule,
                    "display:none"
                        | "visibility:hidden"
                        | "visibility:collapse"
                        | "opacity:0"
                        | "opacity:0.0"
                )
            });
        let locator = Some(format!("html/{}[{index}]", attributes.name()));
        let text = element
            .text()
            .flat_map(str::split_whitespace)
            .collect::<Vec<_>>()
            .join(" ");
        if statically_hidden && !text.is_empty() {
            push_html_finding(
                &mut bundle,
                "HTML_STATICALLY_HIDDEN_TEXT",
                &text,
                locator.clone(),
                options,
                MAX_FINDINGS,
            )?;
        }
        for (attribute, kind) in [
            ("content", "HTML_METADATA_CONTENT"),
            ("aria-label", "HTML_ACCESSIBILITY_LABEL"),
            ("title", "HTML_TITLE_ATTRIBUTE"),
            ("value", "HTML_FORM_VALUE"),
        ] {
            if let Some(value) = attributes
                .attr(attribute)
                .filter(|value| !value.trim().is_empty())
            {
                push_html_finding(
                    &mut bundle,
                    kind,
                    value,
                    locator.clone(),
                    options,
                    MAX_FINDINGS,
                )?;
            }
        }
        if bundle.findings.len() >= MAX_FINDINGS {
            bundle.coverage_complete = false;
            bundle
                .warnings
                .push("HTML_AUDIT_FINDING_LIMIT_REACHED".to_string());
            break;
        }
    }
    Ok(bundle)
}

fn push_html_finding(
    bundle: &mut EvidenceBundle,
    kind: &str,
    value: &str,
    xml_path: Option<String>,
    options: ForensicAuditOptions,
    limit: usize,
) -> Result<(), AppError> {
    if bundle.findings.len() >= limit {
        return Ok(());
    }
    bundle.findings.push(finding_for_text(
        EvidenceClass::Preserved,
        EvidenceConfidence::Exact,
        kind,
        EvidenceLocator { xml_path, ..EvidenceLocator::default() },
        value,
        "The value is preserved in the original HTML source. Scripts and remote resources were not executed or fetched.",
        options,
    )?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn extracts_visible_text_and_ignores_executable_content() {
        let path = std::env::temp_dir().join(format!(
            "onrecord-html-parser-{}.html",
            uuid::Uuid::new_v4()
        ));
        fs::write(
            &path,
            r#"<!doctype html><html><head><style>.secret{}</style></head>
               <body><main><h1>Annual Report</h1><p>Total: <strong>10,234</strong></p>
               <script>doNotIndex("hidden")</script><template>also hidden</template></main></body></html>"#,
        )
        .expect("fixture should be written");

        let extracted = extract_html_text(&path).expect("HTML should parse");
        let _ = fs::remove_file(path);
        assert!(extracted.contains("Annual Report"));
        assert!(extracted.contains("Total:"));
        assert!(extracted.contains("10,234"));
        assert!(!extracted.contains("doNotIndex"));
        assert!(!extracted.contains("also hidden"));
        assert!(!extracted.contains("secret"));
    }

    #[test]
    fn forensic_audit_retains_hidden_and_accessibility_source_values() {
        let path =
            std::env::temp_dir().join(format!("onrecord-html-audit-{}.html", uuid::Uuid::new_v4()));
        fs::write(&path, r#"<html><body><!-- reviewer note --><div hidden>preserved value</div><button aria-label="Download release">icon</button><input type="hidden" value="case-42"></body></html>"#).unwrap();
        let result = audit_html_file(&path, ForensicAuditOptions::default()).unwrap();
        let _ = fs::remove_file(path);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "HTML_COMMENT"));
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "HTML_STATICALLY_HIDDEN_TEXT"));
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "HTML_ACCESSIBILITY_LABEL"));
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "HTML_FORM_VALUE"));
    }
}
