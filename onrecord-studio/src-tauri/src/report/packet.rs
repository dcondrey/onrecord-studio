//! Citable evidence packets: a plain-text (Markdown) and JSON rendering of findings a journalist
//! can cite and a second person can regenerate and compare.
//!
//! The packet is built from the same `VerifiableReportManifest` the signed report uses, so basis
//! labels, confidence states, quote checks, PII masking and withholding come from one place and
//! cannot drift. What it adds is the source documents' download URL, an explicit "what this does
//! not show" list per claim derived only from what was observed, the evidence gaps of the
//! detectors involved (kept in their own section and type; a gap is never a claim), and a
//! content hash.
//!
//! **Content hash.** `content_sha256` is the SHA-256 of the compact JSON of [`PacketContent`]. It
//! excludes the generation time, finding timestamps (`last_seen_at` moves on every refresh), the
//! author and the build version, so regenerating from unchanged findings gives the same hash.
//! Claims and gaps are sorted and no map appears in the hashed structure, so the byte stream is
//! deterministic.

use super::{builder::SIGNATURE_LIMITATIONS, SourceDocument, SourceDocuments};
use crate::analysis::gaps::EvidenceGap;
use crate::integrity::sha256_hex;
use crate::storage::pii::mask_pii;
use crate::types::report::{
    QuoteStatus, ReportCitation, ReportClaim, ReportConfidence, ReportEvidenceBasis,
    VerifiableReportManifest, WithheldItem,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const PACKET_SCHEMA_VERSION: &str = "onrecord-evidence-packet-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PacketSource {
    pub portal_key: String,
    pub document_id: i64,
    pub label: Option<String>,
    pub file_name: Option<String>,
    /// Public documents only, without query string or fragment. `None` is "not recorded".
    pub download_url: Option<String>,
    /// `None` is "not recorded", never an empty digest.
    pub raw_file_sha256: Option<String>,
    pub extracted_text_projection_sha256: Option<String>,
    pub character_start: Option<i64>,
    pub character_end: Option<i64>,
    pub quote: Option<String>,
    pub quote_status: QuoteStatus,
    pub quote_truncated: bool,
    pub pii_masked_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PacketClaim {
    pub finding_id: String,
    pub kind: String,
    pub severity: String,
    pub basis: ReportEvidenceBasis,
    pub basis_label: String,
    /// The slug of the detector that produced the finding. No per-detector version is recorded.
    pub detector_slug: String,
    pub headline: String,
    /// The detector's own account of what it compared or counted.
    pub detector_explanation: String,
    pub confidence: ReportConfidence,
    pub confidence_observed: String,
    pub portal_key: Option<String>,
    pub request_id: Option<i64>,
    pub sources: Vec<PacketSource>,
    /// Keys of gaps listed in the packet's gap section that belong to this claim's detector.
    pub gap_keys: Vec<String>,
    pub does_not_show: Vec<String>,
}

/// A route the corpus identified but could not test. Deliberately shares no field with a claim:
/// no basis, no severity, no confidence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PacketGap {
    pub key: String,
    pub detector_slug: String,
    pub route: String,
    pub blocked: String,
    pub records_request: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PacketContent {
    pub schema_version: String,
    pub scope_portal_key: Option<String>,
    pub claims: Vec<PacketClaim>,
    pub gaps: Vec<PacketGap>,
    pub withheld: Vec<WithheldItem>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePacket {
    pub content_sha256: String,
    pub generated_at: String,
    /// The program build that wrote this packet. Not covered by `content_sha256`.
    pub build_version: String,
    pub content: PacketContent,
}

pub fn content_sha256(content: &PacketContent) -> Result<String, String> {
    serde_json::to_vec(content)
        .map(|bytes| sha256_hex(&bytes))
        .map_err(|error| format!("Failed to encode packet content: {error}"))
}

/// True when the stored hash matches the content, as a recipient re-checking the JSON would find.
pub fn hash_matches(packet: &EvidencePacket) -> bool {
    content_sha256(&packet.content).is_ok_and(|hash| hash == packet.content_sha256)
}

/// Keeps scheme, host and path only. A URL that carries credentials or is not http(s) is not
/// reproduced: a signed query string can be a bearer token.
fn public_url(value: &str) -> Option<String> {
    let value = value.trim();
    let rest = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))?;
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if rest[..authority_end].contains('@') || authority_end == 0 {
        return None;
    }
    let cut = value.find(['?', '#']).unwrap_or(value.len());
    let url = &value[..cut];
    (url.chars().all(|c| c.is_ascii_graphic()) && url.len() <= 2_048).then(|| url.to_string())
}

fn percent(basis_points: u16) -> String {
    format!("{}.{:02}%", basis_points / 100, basis_points % 100)
}

fn confidence_observed(confidence: ReportConfidence) -> String {
    match confidence {
        ReportConfidence::Unassessed => {
            "Not assessed: the detector had no measurable basis to score this finding. This is \
             not a score of zero."
                .to_string()
        }
        ReportConfidence::Measured { basis_points } => format!(
            "Measured {} ({basis_points} basis points), computed from what the detector \
             observed. It is not the probability that the conclusion is correct.",
            percent(basis_points)
        ),
    }
}

fn does_not_show(claim: &ReportClaim, sources: &[PacketSource], gap_count: usize) -> Vec<String> {
    let mut lines = vec![match claim.evidence_basis {
        ReportEvidenceBasis::CountedViolation => {
            "It counts a comparison against a rule or commitment the record states. It does not \
             establish that the conduct was unlawful, intentional, or unexplained."
                .to_string()
        }
        ReportEvidenceBasis::PatternSignal => {
            "It is a pattern signal. It does not establish a violation of any rule, and it does \
             not say why the pattern occurs."
                .to_string()
        }
    }];
    if matches!(claim.confidence, ReportConfidence::Unassessed) {
        lines.push("No confidence was assessed; a missing score is not a low score.".to_string());
    }
    if sources.is_empty() {
        lines.push(
            "The finding cites no source document, so nothing here can be matched to a record."
                .to_string(),
        );
    }
    for source in sources {
        let who = format!("document {} ({})", source.document_id, source.portal_key);
        match source.quote_status {
            QuoteStatus::VerifiedAgainstStoredText | QuoteStatus::FromStoredTextRange => {}
            QuoteStatus::UncheckedDetectorQuote => lines.push(format!(
                "The quoted excerpt from {who} is the detector's and was not checked against \
                 stored text."
            )),
            QuoteStatus::WithheldMismatch => lines.push(format!(
                "An excerpt from {who} was withheld because it did not appear at the cited range."
            )),
            QuoteStatus::WithheldNonPublicDocument => lines.push(format!(
                "{who} is not public; no name, digest, URL or excerpt is published for it."
            )),
            QuoteStatus::NoQuote => {
                lines.push(format!("The citation of {who} carries no excerpt."));
            }
        }
        if source.quote_status != QuoteStatus::WithheldNonPublicDocument {
            if source.raw_file_sha256.is_none() {
                lines.push(format!(
                    "The file digest of {who} is not recorded, so the file cannot be matched to \
                     this packet by hash."
                ));
            }
            if source.extracted_text_projection_sha256.is_none() {
                lines.push(format!(
                    "The extracted-text digest of {who} is not recorded, so its quoted text \
                     cannot be checked against a digest."
                ));
            }
            if source.download_url.is_none() {
                lines.push(format!("No download URL is recorded for {who}."));
            }
        }
    }
    if gap_count > 0 {
        lines.push(format!(
            "{gap_count} evidence gap(s) for this detector are listed separately; the routes \
             they name were not tested."
        ));
    }
    lines
}

fn build_source(
    citation: ReportCitation,
    documents: &SourceDocuments,
) -> Result<PacketSource, String> {
    // The file name is masked by the builder; a URL cannot be masked and stay valid, so one that
    // carries a personal value is not reproduced.
    let download_url = documents
        .get(&(citation.portal_key.clone(), citation.document_id))
        .filter(|document: &&SourceDocument| document.is_public())
        .and_then(|document| document.download_url.as_deref())
        .and_then(public_url);
    let download_url = match download_url {
        Some(url) => mask_pii(&url)
            .map_err(|error| format!("Failed to screen a download URL: {error}"))?
            .findings
            .is_empty()
            .then_some(url),
        None => None,
    };
    Ok(PacketSource {
        portal_key: citation.portal_key,
        document_id: citation.document_id,
        label: citation.label,
        file_name: citation.file_name,
        download_url,
        raw_file_sha256: citation.raw_file_sha256,
        extracted_text_projection_sha256: citation.extracted_text_projection_sha256,
        character_start: citation.character_start,
        character_end: citation.character_end,
        quote: citation.quote,
        quote_status: citation.quote_status,
        quote_truncated: citation.quote_truncated,
        pii_masked_count: citation.pii_masked_count,
    })
}

fn gap_sweep_name(failure: &str) -> &str {
    failure
        .split_once(" gap sweep failed")
        .map_or("an unnamed detector", |(name, _)| name)
}

/// `gaps` are the corpus-wide gaps; only those of a detector that produced a claim are kept.
/// `gap_sweep_failures` are the collector's error strings; their detail is not reproduced.
pub fn build_packet(
    manifest: VerifiableReportManifest,
    documents: &SourceDocuments,
    gaps: Vec<EvidenceGap>,
    gap_sweep_failures: &[String],
    generated_at: String,
    build_version: &str,
) -> Result<EvidencePacket, String> {
    let slugs: BTreeSet<&str> = manifest
        .claims
        .iter()
        .map(|claim| claim.source_detector.as_str())
        .collect();
    let mut packet_gaps: Vec<PacketGap> = gaps
        .into_iter()
        .filter(|gap| slugs.contains(gap.detector))
        .map(|gap| PacketGap {
            key: gap.key,
            detector_slug: gap.detector.to_string(),
            route: gap.route,
            blocked: gap.blocked,
            records_request: gap.request,
        })
        .collect();
    packet_gaps.sort_by(|a, b| a.key.cmp(&b.key));

    let mut claims: Vec<PacketClaim> = manifest
        .claims
        .into_iter()
        .map(|claim| {
            let gap_keys: Vec<String> = packet_gaps
                .iter()
                .filter(|gap| gap.detector_slug == claim.source_detector)
                .map(|gap| gap.key.clone())
                .collect();
            let sources = claim
                .citations
                .iter()
                .cloned()
                .map(|citation| build_source(citation, documents))
                .collect::<Result<Vec<PacketSource>, String>>()?;
            let does_not_show = does_not_show(&claim, &sources, gap_keys.len());
            Ok(PacketClaim {
                finding_id: claim.finding_id,
                kind: claim.kind,
                severity: claim.severity,
                basis: claim.evidence_basis,
                basis_label: claim.evidence_basis.label().to_string(),
                detector_slug: claim.source_detector,
                headline: claim.headline,
                detector_explanation: claim.explanation,
                confidence: claim.confidence,
                confidence_observed: confidence_observed(claim.confidence),
                portal_key: claim.portal_key,
                request_id: claim.request_id,
                sources,
                gap_keys,
                does_not_show,
            })
        })
        .collect::<Result<_, String>>()?;
    claims.sort_by(|a, b| a.finding_id.cmp(&b.finding_id));

    let mut limitations: Vec<String> = manifest
        .limitations
        .into_iter()
        .filter(|line| !SIGNATURE_LIMITATIONS.contains(&line.as_str()))
        .collect();
    limitations.extend([
        "This packet is unsigned. The content hash detects a change to the content; it does not \
         say who produced it. Keep the hash where the recipient can compare it."
            .to_string(),
        "Detectors are identified by slug only; no per-detector version is recorded. A different \
         program build can produce different content from the same findings."
            .to_string(),
        "No retrieval date is recorded per document. Download URLs are shown only for public \
         documents and without query strings or fragments."
            .to_string(),
    ]);
    if !gap_sweep_failures.is_empty() {
        let names: BTreeSet<&str> = gap_sweep_failures
            .iter()
            .map(|failure| gap_sweep_name(failure))
            .collect();
        limitations.push(format!(
            "The evidence-gap sweep failed for: {}. The gaps listed may be incomplete.",
            names.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }

    let content = PacketContent {
        schema_version: PACKET_SCHEMA_VERSION.to_string(),
        scope_portal_key: manifest.scope_portal_key,
        claims,
        gaps: packet_gaps,
        withheld: manifest.withheld,
        limitations,
    };
    Ok(EvidencePacket {
        content_sha256: content_sha256(&content)?,
        generated_at,
        build_version: build_version.to_string(),
        content,
    })
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One line of prose with `<` escaped, so text from a finding or a document cannot open raw HTML
/// in a Markdown viewer that passes it through.
fn prose(value: &str) -> String {
    single_line(value).replace('<', "&lt;")
}

fn code(value: &str) -> String {
    format!("`{}`", single_line(value).replace('`', "'"))
}

fn block_quote(value: &str) -> String {
    // A lone carriage return ends a line in Markdown but not in `str::lines`, so a line that
    // starts after one would escape the quote and could read as a heading.
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('<', "&lt;")
        .lines()
        .map(|line| {
            let line = line.trim_end();
            if line.is_empty() {
                ">".to_string()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn digest_line(label: &str, value: Option<&str>) -> String {
    match value {
        Some(value) => format!("- {label}: {}\n", code(value)),
        None => format!("- {label}: not recorded\n"),
    }
}

/// Heading prefix of every claim block; a gap or limitation line is never rendered under it.
pub const CLAIM_HEADING_PREFIX: &str = "### Claim ";

pub fn render_markdown(packet: &EvidencePacket) -> String {
    let content = &packet.content;
    let mut out = String::new();
    out.push_str("# Evidence packet\n\n");
    out.push_str(&format!(
        "- Content SHA-256: {}\n",
        code(&packet.content_sha256)
    ));
    out.push_str(&format!("- Schema: {}\n", code(&content.schema_version)));
    out.push_str(&format!("- Generated: {}\n", code(&packet.generated_at)));
    out.push_str(&format!("- Build: {}\n", code(&packet.build_version)));
    if let Some(portal) = &content.scope_portal_key {
        out.push_str(&format!("- Scope: portal {}\n", code(portal)));
    }
    out.push_str(
        "\n## Reproducing this packet\n\nRegenerate from the same findings and compare the \
         content hash. The hash is the SHA-256 of the compact JSON of the `content` object in the \
         accompanying `.json` file. It excludes the generation time, finding timestamps, the \
         author and the build version.\n",
    );
    out.push_str(&format!("\n## Claims ({})\n", content.claims.len()));
    for (index, claim) in content.claims.iter().enumerate() {
        out.push_str(&format!(
            "\n{CLAIM_HEADING_PREFIX}{}: {}\n\n",
            index + 1,
            prose(&claim.headline)
        ));
        out.push_str(&format!("- Basis: {}\n", claim.basis_label));
        out.push_str(&format!("- Finding: {}\n", code(&claim.finding_id)));
        out.push_str(&format!(
            "- Kind and severity: {} / {}\n",
            code(&claim.kind),
            code(&claim.severity)
        ));
        out.push_str(&format!("- Detector: {}\n", code(&claim.detector_slug)));
        out.push_str(&format!("- Confidence: {}\n", claim.confidence_observed));
        out.push_str("\nHow the detector reached this, in its own words:\n\n");
        out.push_str(&block_quote(&claim.detector_explanation));
        out.push_str("\n\nSources:\n");
        if claim.sources.is_empty() {
            out.push_str("\n- none cited\n");
        }
        for (position, source) in claim.sources.iter().enumerate() {
            out.push_str(&format!(
                "\n{}. Document {} in portal {}",
                position + 1,
                source.document_id,
                code(&source.portal_key)
            ));
            if let Some(name) = &source.file_name {
                out.push_str(&format!(", file {}", code(name)));
            }
            out.push('\n');
            out.push_str(&format!(
                "   - URL: {}\n",
                source
                    .download_url
                    .as_deref()
                    .map_or("not recorded".to_string(), code)
            ));
            out.push_str(&format!(
                "   - Location: {}\n",
                match (source.character_start, source.character_end) {
                    (Some(start), Some(end)) =>
                        format!("characters {start} to {end} of the stored text"),
                    _ => "not recorded".to_string(),
                }
            ));
            for line in [
                digest_line("File SHA-256", source.raw_file_sha256.as_deref()),
                digest_line(
                    "Extracted-text SHA-256",
                    source.extracted_text_projection_sha256.as_deref(),
                ),
            ] {
                out.push_str(&format!("   {line}"));
            }
            out.push_str(&format!("   - Excerpt check: {:?}\n", source.quote_status));
            if let Some(quote) = &source.quote {
                let note = if source.quote_truncated {
                    " (truncated)"
                } else {
                    ""
                };
                out.push_str(&format!("   - Excerpt{note}:\n\n"));
                for line in block_quote(quote).lines() {
                    out.push_str(&format!("     {line}\n"));
                }
            }
        }
        out.push_str("\nWhat this does not show:\n\n");
        for line in &claim.does_not_show {
            out.push_str(&format!("- {}\n", prose(line)));
        }
        if !claim.gap_keys.is_empty() {
            let keys: Vec<String> = claim.gap_keys.iter().map(|key| code(key)).collect();
            out.push_str(&format!("- Related gap keys: {}\n", keys.join(", ")));
        }
    }
    out.push_str("\n## Evidence gaps (not findings)\n\n");
    out.push_str(
        "A gap is a route the corpus identified but could not test. It is not a finding, carries \
         no basis or confidence, and is not evidence of compliance.\n\n",
    );
    if content.gaps.is_empty() {
        out.push_str(
            "No gaps were recorded for the detectors behind these claims. That is not a \
             statement that none exist.\n",
        );
    }
    for gap in &content.gaps {
        out.push_str(&format!(
            "- {} ({}): {} Cannot be tested: {} Records request: {}\n",
            code(&gap.key),
            code(&gap.detector_slug),
            prose(&gap.route),
            prose(&gap.blocked),
            prose(&gap.records_request)
        ));
    }
    out.push_str("\n## Withheld\n\n");
    if content.withheld.is_empty() {
        out.push_str("Nothing was withheld.\n");
    }
    for item in &content.withheld {
        out.push_str(&format!(
            "- {} x {}: {}\n",
            code(&format!("{:?}", item.reason)),
            item.count,
            prose(&item.explanation)
        ));
    }
    out.push_str("\n## Limitations\n\n");
    for line in &content.limitations {
        out.push_str(&format!("- {}\n", prose(line)));
    }
    out
}

#[cfg(test)]
mod tests;
