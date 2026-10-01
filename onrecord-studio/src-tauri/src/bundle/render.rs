//! Renders a bundle as one static HTML page. There is no script to run and nothing to fetch: the
//! page is plain markup and inline style under a policy that forbids every network load, and its
//! data document sits in an inert `application/json` element for any tool to read.

use super::{BundleDocument, BundleFinding, BUNDLE_FORMAT, BUNDLE_SCHEMA_VERSION};
use crate::error::AppError;
use crate::report::packet::annotations::json_carries_section;
use crate::types::report::{QuoteStatus, ReportConfidence};
use std::fmt::Write;

const DATA_OPEN: &str = "<script type=\"application/json\" id=\"onrecord-bundle-data\">";
const DATA_CLOSE: &str = "</script>";

const STYLE: &str =
    "body{font:16px/1.5 system-ui,sans-serif;max-width:52rem;margin:0 auto;padding:1rem;\
color:#1a1a1a;background:#fff}h1{font-size:1.5rem}h2{font-size:1.15rem;margin:0}\
article{border:1px solid #bbb;border-radius:6px;padding:1rem;margin:1rem 0}\
.meta,.hash{font-size:.85rem;color:#444;overflow-wrap:anywhere}.badge{display:inline-block;\
border:1px solid #777;border-radius:4px;padding:0 .4rem;margin-right:.4rem;font-size:.8rem}\
blockquote{margin:.5rem 0;padding-left:.75rem;border-left:3px solid #999;white-space:pre-wrap}\
ul{padding-left:1.2rem}@media (prefers-color-scheme:dark){body{background:#111;color:#eee}\
.meta,.hash{color:#bbb}article{border-color:#555}.badge{border-color:#999}}";

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

fn quote_status_label(status: QuoteStatus) -> &'static str {
    match status {
        QuoteStatus::VerifiedAgainstStoredText => "checked against stored text",
        QuoteStatus::FromStoredTextRange => "taken from stored text",
        QuoteStatus::UncheckedDetectorQuote => "detector wording, not checked",
        QuoteStatus::WithheldMismatch => "withheld: did not match stored text",
        QuoteStatus::WithheldNonPublicDocument => "withheld: not a public record",
        QuoteStatus::NoQuote => "no excerpt",
    }
}

fn confidence_label(confidence: ReportConfidence) -> String {
    match confidence {
        ReportConfidence::Unassessed => "Confidence: not assessed".to_string(),
        ReportConfidence::Measured { basis_points } => format!(
            "Confidence: {}.{:02}%",
            basis_points / 100,
            basis_points % 100
        ),
    }
}

fn write_finding(out: &mut String, finding: &BundleFinding) {
    let claim = &finding.claim;
    let _ = write!(
        out,
        "<article id=\"finding-{id}\"><h2>{headline}</h2><p><span class=\"badge\">{severity}</span>\
         <span class=\"badge\">{basis}</span><span class=\"badge\">{confidence}</span></p>\
         <p>{explanation}</p>",
        id = escape(&claim.finding_id),
        headline = escape(&claim.headline),
        severity = escape(&claim.severity),
        basis = escape(&finding.basis_label),
        confidence = escape(&confidence_label(claim.confidence)),
        explanation = escape(&claim.explanation),
    );
    out.push_str("<h3>Limits</h3><ul>");
    for limit in &finding.limits {
        let _ = write!(out, "<li>{}</li>", escape(limit));
    }
    out.push_str("</ul><h3>Sources</h3><ul>");
    for citation in &claim.citations {
        let name = citation
            .file_name
            .as_deref()
            .or(citation.label.as_deref())
            .map_or_else(
                || format!("Document {}", citation.document_id),
                str::to_string,
            );
        let _ = write!(
            out,
            "<li><strong>{}</strong> <span class=\"meta\">(portal {}, document {}",
            escape(&name),
            escape(&citation.portal_key),
            citation.document_id
        );
        if let (Some(start), Some(end)) = (citation.character_start, citation.character_end) {
            let _ = write!(out, ", characters {start} to {end}");
        }
        out.push_str(")</span>");
        match &citation.raw_file_sha256 {
            Some(digest) => {
                let _ = write!(
                    out,
                    "<div class=\"hash\">File SHA-256 {}</div>",
                    escape(digest)
                );
            }
            None => out.push_str("<div class=\"hash\">File SHA-256 not recorded</div>"),
        }
        if let Some(quote) = &citation.quote {
            let _ = write!(
                out,
                "<blockquote>{}</blockquote><div class=\"meta\">Excerpt: {}{}</div>",
                escape(quote),
                quote_status_label(citation.quote_status),
                if citation.quote_truncated {
                    "; shortened"
                } else {
                    ""
                }
            );
        } else {
            let _ = write!(
                out,
                "<div class=\"meta\">Excerpt: {}</div>",
                quote_status_label(citation.quote_status)
            );
        }
        out.push_str("</li>");
    }
    out.push_str("</ul></article>");
}

/// Escapes `<` so no sequence in the data can close the element or open a comment, and so a
/// reader's HTML parser never sees markup inside it. JSON parsers read `<` as `<`.
fn embeddable_json(document: &BundleDocument) -> Result<String, AppError> {
    let json = serde_json::to_string_pretty(document)
        .map_err(|error| AppError::Export(format!("Failed to encode bundle data: {error}")))?;
    Ok(json.replace('<', "\\u003c"))
}

pub fn render_html(document: &BundleDocument) -> Result<String, AppError> {
    let content = &document.content;
    if content
        .packets
        .iter()
        .any(|packet| json_carries_section(&packet.payload))
    {
        return Err(AppError::Export(
            "A shareable bundle never carries reviewer annotations; the embedded packet has them."
                .to_string(),
        ));
    }
    let mut out = String::new();
    let _ = write!(
        out,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">\
         <meta name=\"referrer\" content=\"no-referrer\"><title>{title}</title><style>{STYLE}</style></head><body>\
         <h1>{title}</h1><p class=\"meta\">Read-only. Generated {generated}. {count} finding(s). \
         Bundle format {BUNDLE_FORMAT}, schema version {BUNDLE_SCHEMA_VERSION}.</p>\
         <p class=\"hash\">Content SHA-256 {hash}</p>",
        title = escape(&content.title),
        generated = escape(&document.generated_at),
        count = content.findings.len(),
        hash = escape(&document.content_sha256),
    );
    for finding in &content.findings {
        write_finding(&mut out, finding);
    }
    if !content.packets.is_empty() {
        let _ = write!(
            out,
            "<h2>Embedded evidence packets</h2><p class=\"meta\">{} packet(s) are carried in the \
             data block of this file.</p>",
            content.packets.len()
        );
    }
    out.push_str("<h2>Limitations of this bundle</h2><ul>");
    for limitation in &content.limitations {
        let _ = write!(out, "<li>{}</li>", escape(limitation));
    }
    out.push_str("</ul>");
    let _ = write!(
        out,
        "{DATA_OPEN}{}{DATA_CLOSE}</body></html>",
        embeddable_json(document)?
    );
    Ok(out)
}

/// Reads the data document back out of a bundle page, as any other tool would. Refuses a format
/// or schema version it does not know.
pub fn parse_embedded(html: &str) -> Result<BundleDocument, AppError> {
    let start = html
        .find(DATA_OPEN)
        .ok_or_else(|| AppError::DataParsing("The file has no bundle data block".to_string()))?
        + DATA_OPEN.len();
    let length = html[start..]
        .find(DATA_CLOSE)
        .ok_or_else(|| AppError::DataParsing("The bundle data block is not closed".to_string()))?;
    let document: BundleDocument = serde_json::from_str(&html[start..start + length])
        .map_err(|error| AppError::DataParsing(format!("Bundle data is malformed: {error}")))?;
    if document.format != BUNDLE_FORMAT || document.schema_version != BUNDLE_SCHEMA_VERSION {
        return Err(AppError::DataParsing(format!(
            "Unsupported bundle {} version {}",
            document.format, document.schema_version
        )));
    }
    Ok(document)
}
