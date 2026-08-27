use std::path::Path;

use scraper::{ElementRef, Html, Selector};

use crate::error::AppError;
use crate::parsers::source::read_regular_file;

const MAX_HTML_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_HTML_TEXT_BYTES: usize = 32 * 1024 * 1024;
const IGNORED_ELEMENTS: &[&str] = &["script", "style", "noscript", "template", "svg"];

/// Extracts human-visible text from an HTML document without indexing markup,
/// scripts, styles, or hidden template content.
pub fn extract_html_text<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    // FEATURE(html-audit): Add a source-preserving mode that records comments, hidden attributes,
    // CSS visibility/opacity, overlay geometry, forms, metadata, and accessibility text.
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
}
