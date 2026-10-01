use crate::error::AppError;
use crate::integrity::sha256_hex;
use crate::normalization::collapse_whitespace;
use crate::parsers::source::read_regular_file;
use crate::storage::pii::mask_pii;
use lopdf::{Document, Object, ObjectId};
use std::collections::HashSet;
use std::path::Path;

const MAXIMUM_SOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAXIMUM_HISTORICAL_REVISIONS: usize = 16;
const MAXIMUM_EOF_MARKERS: usize = 4_096;
const MAXIMUM_PAGES_PER_REVISION: usize = 2_500;
const MAXIMUM_TEXT_CHARACTERS_PER_REVISION: usize = 200_000;
const MAXIMUM_TOTAL_RECOVERED_CHARACTERS: usize = 500_000;
const MAXIMUM_CUMULATIVE_SNAPSHOT_BYTES: usize = 512 * 1024 * 1024;
const MAXIMUM_METADATA_FINDINGS: usize = 64;
const MAXIMUM_METADATA_RUN_BYTES: usize = 4_096;
const MAXIMUM_METADATA_DISPLAY_CHARACTERS: usize = 512;
const MAXIMUM_REPRESENTATIVE_PAGE_ERRORS: usize = 8;
const MAXIMUM_OBJECT_GRAPH_DEPTH: usize = 64;

#[derive(Debug, serde::Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredTextSegment {
    pub revision_number: usize,
    pub snapshot_end_offset: u64,
    pub recovered_character_count: usize,
    pub recovered_text_sha256: String,
    pub masked_finding_count: usize,
    pub masked_text: String,
}

#[derive(Debug, serde::Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredMetadataArtifact {
    pub byte_offset: u64,
    pub category: String,
    pub value_sha256: String,
    pub masked_value: String,
    pub masked_finding_count: usize,
}

#[derive(Debug, serde::Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredRevisionDiagnostic {
    pub revision_number: usize,
    pub snapshot_end_offset: u64,
    pub xref_start_offset: Option<u64>,
    pub cross_reference_type: Option<String>,
    pub declared_previous_xref_offset: Option<u64>,
    pub object_count: usize,
    pub nonzero_generation_object_count: usize,
    pub unreferenced_object_count: usize,
    pub unreferenced_stream_count: usize,
    pub object_graph_traversal_complete: bool,
    pub page_count: usize,
    pub page_extraction_failure_count: usize,
    pub representative_page_errors: Vec<String>,
    pub structural_parse_error: Option<String>,
}

#[derive(Debug, serde::Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredVersionProfile {
    pub source_sha256: String,
    pub source_size_bytes: u64,
    pub total_eof_markers: usize,
    pub historical_versions_found: usize,
    pub inspected_historical_versions: usize,
    pub revision_diagnostics: Vec<RecoveredRevisionDiagnostic>,
    pub recovered_text_segments: Vec<RecoveredTextSegment>,
    pub post_eof_metadata_artifacts: Vec<RecoveredMetadataArtifact>,
    pub analysis_truncated: bool,
    pub warnings: Vec<String>,
}

pub fn carve_historical_pdf_versions<P: AsRef<Path>>(
    file_path: P,
) -> Result<RecoveredVersionProfile, AppError> {
    // This is a bounded incremental-revision reader, not a deleted-sector or unallocated-space
    // carver. lopdf follows classic /Prev chains, hybrid xrefs, and xref streams while loading
    // each snapshot; the diagnostics below preserve the resulting structural facts separately
    // from any recovered page text.
    let path = file_path.as_ref();
    let source = read_regular_file(path, "PDF_RECOVERY", MAXIMUM_SOURCE_BYTES)?;
    if !has_pdf_signature(&source) {
        return Err(AppError::DataParsing(
            "Historical version recovery is restricted to structurally identified PDF evidence"
                .to_string(),
        ));
    }

    let source_sha256 = sha256_hex(&source);
    let eof_offsets = find_eof_markers(&source)?;
    if eof_offsets.is_empty() {
        return Err(AppError::DataParsing(
            "The PDF has no terminal %%EOF marker and cannot be revision-carved reliably"
                .to_string(),
        ));
    }

    let historical_versions_found = eof_offsets.len().saturating_sub(1);
    let first_revision = historical_versions_found.saturating_sub(MAXIMUM_HISTORICAL_REVISIONS);
    let selected_offsets = &eof_offsets[first_revision..historical_versions_found];
    let mut recovered_text_segments = Vec::new();
    let mut revision_diagnostics = Vec::new();
    let mut seen_text_hashes = HashSet::new();
    let mut inspected_historical_versions = 0_usize;
    let mut cumulative_snapshot_bytes = 0_usize;
    let mut total_recovered_characters = 0_usize;
    let mut analysis_truncated = historical_versions_found > MAXIMUM_HISTORICAL_REVISIONS;
    let mut warnings = Vec::new();

    if analysis_truncated {
        warnings.push(format!(
            "Only the {MAXIMUM_HISTORICAL_REVISIONS} most recent historical revisions were inspected"
        ));
    }

    for (selected_index, eof_offset) in selected_offsets.iter().copied().enumerate() {
        let snapshot_end = eof_offset
            .checked_add(b"%%EOF".len())
            .ok_or_else(|| AppError::DataParsing("PDF revision offset overflowed".to_string()))?;
        cumulative_snapshot_bytes = cumulative_snapshot_bytes
            .checked_add(snapshot_end)
            .ok_or_else(|| {
                AppError::DataParsing("Cumulative PDF recovery budget overflowed".to_string())
            })?;
        if cumulative_snapshot_bytes > MAXIMUM_CUMULATIVE_SNAPSHOT_BYTES {
            analysis_truncated = true;
            warnings.push(
                "Historical inspection stopped at the cumulative snapshot memory budget"
                    .to_string(),
            );
            break;
        }
        inspected_historical_versions += 1;
        let revision_number = first_revision + selected_index + 1;
        let xref_start_offset = find_startxref_offset(&source[..snapshot_end]);
        let historical_document = match crate::parsers::pdf::load_bounded_mem(
            &source[..snapshot_end],
        ) {
            Ok(document) => document,
            Err(error) => {
                revision_diagnostics.push(RecoveredRevisionDiagnostic {
                    revision_number,
                    snapshot_end_offset: snapshot_end as u64,
                    xref_start_offset,
                    cross_reference_type: None,
                    declared_previous_xref_offset: None,
                    object_count: 0,
                    nonzero_generation_object_count: 0,
                    unreferenced_object_count: 0,
                    unreferenced_stream_count: 0,
                    object_graph_traversal_complete: false,
                    page_count: 0,
                    page_extraction_failure_count: 0,
                    representative_page_errors: Vec::new(),
                    structural_parse_error: Some(error.to_string()),
                });
                warnings.push(format!(
                    "Historical revision {revision_number} could not be structurally parsed: {error}"
                ));
                continue;
            }
        };
        let pages = historical_document.get_pages();
        let (unreferenced_object_count, unreferenced_stream_count, graph_complete) =
            object_graph_diagnostics(&historical_document);
        let declared_previous_xref_offset = xref_start_offset.and_then(|start| {
            find_declared_previous_xref_offset(&source[start as usize..snapshot_end])
        });
        let diagnostic_index = revision_diagnostics.len();
        revision_diagnostics.push(RecoveredRevisionDiagnostic {
            revision_number,
            snapshot_end_offset: snapshot_end as u64,
            xref_start_offset: Some(historical_document.xref_start as u64),
            cross_reference_type: Some(format!(
                "{:?}",
                historical_document.reference_table.cross_reference_type
            )),
            declared_previous_xref_offset,
            object_count: historical_document.objects.len(),
            nonzero_generation_object_count: historical_document
                .objects
                .keys()
                .filter(|object_id| object_id.1 > 0)
                .count(),
            unreferenced_object_count,
            unreferenced_stream_count,
            object_graph_traversal_complete: graph_complete,
            page_count: pages.len(),
            page_extraction_failure_count: 0,
            representative_page_errors: Vec::new(),
            structural_parse_error: None,
        });
        if pages.len() > MAXIMUM_PAGES_PER_REVISION {
            analysis_truncated = true;
            warnings.push(format!(
                "Historical revision {revision_number} exceeded the {MAXIMUM_PAGES_PER_REVISION}-page inspection limit"
            ));
        }
        let mut recovered_text = String::new();
        let mut recovered_text_characters = 0_usize;
        for page_number in pages.keys().take(MAXIMUM_PAGES_PER_REVISION) {
            let page_text = match historical_document.extract_text_with_limit(
                &[*page_number],
                crate::parsers::pdf::MAX_DECOMPRESSED_STREAM_BYTES,
            ) {
                Ok(text) => text,
                Err(error) => {
                    let diagnostic = &mut revision_diagnostics[diagnostic_index];
                    diagnostic.page_extraction_failure_count += 1;
                    if diagnostic.representative_page_errors.len()
                        < MAXIMUM_REPRESENTATIVE_PAGE_ERRORS
                    {
                        diagnostic
                            .representative_page_errors
                            .push(format!("page {page_number}: {error}"));
                    }
                    continue;
                }
            };
            append_bounded_text(
                &mut recovered_text,
                &page_text,
                &mut recovered_text_characters,
                MAXIMUM_TEXT_CHARACTERS_PER_REVISION,
            );
            if recovered_text_characters >= MAXIMUM_TEXT_CHARACTERS_PER_REVISION {
                analysis_truncated = true;
                warnings.push(format!(
                    "Recovered text for historical revision {revision_number} reached the per-revision output limit"
                ));
                break;
            }
        }
        let recovered_text = normalize_recovered_text(&recovered_text);
        if !is_meaningful_recovered_text(&recovered_text) {
            continue;
        }
        let recovered_character_count = recovered_text.chars().count();
        if total_recovered_characters
            .checked_add(recovered_character_count)
            .is_none_or(|count| count > MAXIMUM_TOTAL_RECOVERED_CHARACTERS)
        {
            analysis_truncated = true;
            warnings.push("Recovery stopped at the aggregate text output limit".to_string());
            break;
        }
        let recovered_text_sha256 = sha256_hex(recovered_text.as_bytes());
        if !seen_text_hashes.insert(recovered_text_sha256.clone()) {
            continue;
        }
        total_recovered_characters += recovered_character_count;
        let masked = mask_pii(&recovered_text)?;
        recovered_text_segments.push(RecoveredTextSegment {
            revision_number,
            snapshot_end_offset: snapshot_end as u64,
            recovered_character_count,
            recovered_text_sha256,
            masked_finding_count: masked.findings.len(),
            masked_text: masked.masked_text,
        });
    }

    let terminal_eof_end = eof_offsets[eof_offsets.len() - 1] + b"%%EOF".len();
    let (post_eof_metadata_artifacts, metadata_truncated) =
        recover_post_eof_metadata(&source, terminal_eof_end)?;
    analysis_truncated |= metadata_truncated;
    if metadata_truncated {
        warnings.push(format!(
            "Post-EOF metadata artifacts were limited to {MAXIMUM_METADATA_FINDINGS} findings"
        ));
    }

    Ok(RecoveredVersionProfile {
        source_sha256,
        source_size_bytes: source.len() as u64,
        total_eof_markers: eof_offsets.len(),
        historical_versions_found,
        inspected_historical_versions,
        revision_diagnostics,
        recovered_text_segments,
        post_eof_metadata_artifacts,
        analysis_truncated,
        warnings,
    })
}

fn find_startxref_offset(snapshot: &[u8]) -> Option<u64> {
    let marker = b"startxref";
    let offset = snapshot
        .windows(marker.len())
        .rposition(|window| window == marker)?;
    let value = snapshot.get(offset + marker.len()..)?;
    parse_leading_pdf_integer(value)
}

fn find_declared_previous_xref_offset(xref_tail: &[u8]) -> Option<u64> {
    let marker = b"/Prev";
    let offset = xref_tail
        .windows(marker.len())
        .rposition(|window| window == marker)?;
    parse_leading_pdf_integer(xref_tail.get(offset + marker.len()..)?)
}

fn parse_leading_pdf_integer(value: &[u8]) -> Option<u64> {
    let value = value
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take_while(u8::is_ascii_digit)
        .collect::<Vec<_>>();
    (!value.is_empty())
        .then(|| std::str::from_utf8(&value).ok()?.parse().ok())
        .flatten()
}

fn object_graph_diagnostics(document: &Document) -> (usize, usize, bool) {
    let mut reachable = HashSet::<ObjectId>::new();
    let mut pending = Vec::<ObjectId>::new();
    let mut complete = collect_references(
        &Object::Dictionary(document.trailer.clone()),
        0,
        &mut pending,
    );
    while let Some(object_id) = pending.pop() {
        if !reachable.insert(object_id) {
            continue;
        }
        let Some(object) = document.objects.get(&object_id) else {
            complete = false;
            continue;
        };
        complete &= collect_references(object, 0, &mut pending);
    }
    let unreferenced = document
        .objects
        .iter()
        .filter(|(object_id, _)| !reachable.contains(object_id))
        .collect::<Vec<_>>();
    let streams = unreferenced
        .iter()
        .filter(|(_, object)| matches!(object, Object::Stream(_)))
        .count();
    (unreferenced.len(), streams, complete)
}

fn collect_references(object: &Object, depth: usize, pending: &mut Vec<ObjectId>) -> bool {
    if depth > MAXIMUM_OBJECT_GRAPH_DEPTH {
        return false;
    }
    match object {
        Object::Reference(object_id) => pending.push(*object_id),
        Object::Array(values) => {
            let mut complete = true;
            for value in values {
                complete = collect_references(value, depth + 1, pending) && complete;
            }
            return complete;
        }
        Object::Dictionary(dictionary) => {
            let mut complete = true;
            for (_, value) in dictionary {
                complete = collect_references(value, depth + 1, pending) && complete;
            }
            return complete;
        }
        Object::Stream(stream) => {
            let mut complete = true;
            for (_, value) in &stream.dict {
                complete = collect_references(value, depth + 1, pending) && complete;
            }
            return complete;
        }
        _ => {}
    }
    true
}

fn has_pdf_signature(source: &[u8]) -> bool {
    source
        .windows(b"%PDF-".len())
        .take(1_024)
        .any(|window| window == b"%PDF-")
}

fn find_eof_markers(source: &[u8]) -> Result<Vec<usize>, AppError> {
    const MARKER: &[u8] = b"%%EOF";
    let mut offsets = Vec::new();
    for (offset, window) in source.windows(MARKER.len()).enumerate() {
        if window != MARKER {
            continue;
        }
        let begins_on_line = offset == 0
            || source
                .get(offset.wrapping_sub(1))
                .is_some_and(|byte| matches!(byte, b'\r' | b'\n'));
        let end = offset + MARKER.len();
        let has_valid_terminator = end == source.len()
            || source
                .get(end)
                .is_some_and(|byte| byte.is_ascii_whitespace());
        if begins_on_line && has_valid_terminator {
            if offsets.len() >= MAXIMUM_EOF_MARKERS {
                return Err(AppError::DataParsing(format!(
                    "PDF revision marker count exceeds the {MAXIMUM_EOF_MARKERS}-marker structural limit"
                )));
            }
            offsets.push(offset);
        }
    }
    Ok(offsets)
}

fn append_bounded_text(
    target: &mut String,
    source: &str,
    current_characters: &mut usize,
    character_limit: usize,
) {
    if source.is_empty() || *current_characters >= character_limit {
        return;
    }
    let separator_characters = usize::from(!target.is_empty());
    if current_characters.saturating_add(separator_characters) >= character_limit {
        return;
    }
    if separator_characters == 1 {
        target.push('\n');
    }
    let remaining = character_limit - *current_characters - separator_characters;
    let mut appended = 0_usize;
    target.extend(source.chars().take(remaining).inspect(|_| appended += 1));
    *current_characters += separator_characters + appended;
}

fn normalize_recovered_text(text: &str) -> String {
    text.lines()
        .map(collapse_whitespace)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_meaningful_recovered_text(text: &str) -> bool {
    let character_count = text.chars().count();
    if character_count < 32 {
        return false;
    }
    let alphabetic_count = text
        .chars()
        .filter(|character| character.is_alphabetic())
        .count();
    let lexical_tokens = text
        .split_whitespace()
        .filter(|token| {
            token
                .chars()
                .filter(|character| character.is_alphabetic())
                .count()
                >= 2
        })
        .take(6)
        .count();
    alphabetic_count >= 12
        && lexical_tokens >= 4
        && alphabetic_count.saturating_mul(10) >= character_count
}

fn recover_post_eof_metadata(
    source: &[u8],
    terminal_eof_end: usize,
) -> Result<(Vec<RecoveredMetadataArtifact>, bool), AppError> {
    let trailing = source.get(terminal_eof_end..).ok_or_else(|| {
        AppError::DataParsing("Terminal PDF marker offset was invalid".to_string())
    })?;
    let mut artifacts = Vec::new();
    let mut run_start = None;
    let mut truncated = false;
    for (relative_offset, byte) in trailing
        .iter()
        .copied()
        .chain(std::iter::once(0))
        .enumerate()
    {
        if matches!(byte, b'\t' | b'\n' | b'\r' | 0x20..=0x7e) {
            run_start.get_or_insert(relative_offset);
            continue;
        }
        let Some(start) = run_start.take() else {
            continue;
        };
        let end = relative_offset.min(trailing.len());
        if end <= start || end - start < 4 {
            continue;
        }
        let bounded_end = end.min(start + MAXIMUM_METADATA_RUN_BYTES);
        let candidate = String::from_utf8_lossy(&trailing[start..bounded_end]);
        let Some(category) = metadata_category(&candidate) else {
            continue;
        };
        if artifacts.len() >= MAXIMUM_METADATA_FINDINGS {
            truncated = true;
            break;
        }
        let normalized = collapse_whitespace(&candidate);
        let display = normalized
            .chars()
            .take(MAXIMUM_METADATA_DISPLAY_CHARACTERS)
            .collect::<String>();
        let masked = mask_pii(&display)?;
        artifacts.push(RecoveredMetadataArtifact {
            byte_offset: (terminal_eof_end + start) as u64,
            category: category.to_string(),
            value_sha256: sha256_hex(normalized.as_bytes()),
            masked_value: masked.masked_text,
            masked_finding_count: masked.findings.len(),
        });
    }
    Ok((artifacts, truncated))
}

fn metadata_category(value: &str) -> Option<&'static str> {
    let folded = value.to_ascii_lowercase();
    [
        ("author", "AUTHOR_METADATA"),
        ("creator", "CREATOR_METADATA"),
        ("producer", "PRODUCER_METADATA"),
        ("processor", "PROCESSOR_METADATA"),
        ("creationdate", "CREATION_DATE_METADATA"),
        ("moddate", "MODIFICATION_DATE_METADATA"),
        ("uuid:", "UUID_METADATA"),
        ("xmpmeta", "XMP_METADATA"),
    ]
    .into_iter()
    .find_map(|(needle, category)| folded.contains(needle).then_some(category))
}

#[cfg(test)]
mod tests {
    use crate::error::AppError;
    use lopdf::{dictionary, Document, Stream};

    use super::{
        find_declared_previous_xref_offset, find_eof_markers, find_startxref_offset,
        object_graph_diagnostics,
    };

    #[test]
    fn eof_scanner_ignores_marker_text_inside_a_stream() -> Result<(), AppError> {
        let source = b"%PDF-1.7\n(stream %%EOF bytes)\n%%EOF\nrevision\n%%EOF\n";
        assert_eq!(find_eof_markers(source)?.len(), 2);
        Ok(())
    }

    #[test]
    fn reports_xref_offsets_and_unreferenced_streams() {
        let source = b"xref\ntrailer << /Prev 41 >>\nstartxref\n92\n%%EOF";
        assert_eq!(find_startxref_offset(source), Some(92));
        assert_eq!(find_declared_previous_xref_offset(source), Some(41));

        let mut document = Document::with_version("1.7");
        let reachable_stream = document.add_object(Stream::new(dictionary! {}, b"shown".to_vec()));
        let root = document.add_object(dictionary! { "Evidence" => reachable_stream });
        document.add_object(Stream::new(dictionary! {}, b"unreferenced".to_vec()));
        document.trailer.set("Root", root);
        let (unreferenced, streams, complete) = object_graph_diagnostics(&document);
        assert!(complete);
        assert_eq!(unreferenced, 1);
        assert_eq!(streams, 1);
    }
}
