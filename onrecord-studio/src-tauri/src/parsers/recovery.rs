use crate::error::AppError;
use crate::integrity::sha256_hex;
use crate::normalization::collapse_whitespace;
use crate::parsers::source::read_regular_file;
use crate::storage::pii::mask_pii;
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
pub struct RecoveredVersionProfile {
    pub source_sha256: String,
    pub source_size_bytes: u64,
    pub total_eof_markers: usize,
    pub historical_versions_found: usize,
    pub inspected_historical_versions: usize,
    pub recovered_text_segments: Vec<RecoveredTextSegment>,
    pub post_eof_metadata_artifacts: Vec<RecoveredMetadataArtifact>,
    pub analysis_truncated: bool,
    pub warnings: Vec<String>,
}

pub fn carve_historical_pdf_versions<P: AsRef<Path>>(
    file_path: P,
) -> Result<RecoveredVersionProfile, AppError> {
    // FEATURE(recovery): This is a bounded PDF revision reader, not a general deleted-data
    // carver. Future object-generation archaeology should report exact offsets and confidence
    // separately from recovered page text.
    let path = file_path.as_ref();
    let source = read_regular_file(path, "PDF_RECOVERY", MAXIMUM_SOURCE_BYTES)?;
    if !has_pdf_signature(&source) {
        return Err(AppError::DataParsing(
            "Historical version recovery is restricted to structurally identified PDF evidence"
                .to_string(),
        ));
    }

    let source_sha256 = sha256_hex(&source);
    // TODO(recovery): Follow /Prev and xref streams, compare object generations, and detect
    // orphaned streams instead of relying only on %%EOF snapshot boundaries.
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
        let historical_document = match lopdf::Document::load_mem(&source[..snapshot_end]) {
            Ok(document) => document,
            Err(error) => {
                warnings.push(format!(
                    "Historical revision {revision_number} could not be structurally parsed: {error}"
                ));
                continue;
            }
        };
        let pages = historical_document.get_pages();
        if pages.len() > MAXIMUM_PAGES_PER_REVISION {
            analysis_truncated = true;
            warnings.push(format!(
                "Historical revision {revision_number} exceeded the {MAXIMUM_PAGES_PER_REVISION}-page inspection limit"
            ));
        }
        let mut recovered_text = String::new();
        let mut recovered_text_characters = 0_usize;
        for page_number in pages.keys().take(MAXIMUM_PAGES_PER_REVISION) {
            // TODO(recovery): Retain bounded page-extraction failure counts and representative
            // diagnostics so an empty recovery result is distinguishable from failed parsing.
            let page_text = match historical_document.extract_text(&[*page_number]) {
                Ok(text) => text,
                Err(_) => continue,
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
        recovered_text_segments,
        post_eof_metadata_artifacts,
        analysis_truncated,
        warnings,
    })
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

    use super::find_eof_markers;

    #[test]
    fn eof_scanner_ignores_marker_text_inside_a_stream() -> Result<(), AppError> {
        let source = b"%PDF-1.7\n(stream %%EOF bytes)\n%%EOF\nrevision\n%%EOF\n";
        assert_eq!(find_eof_markers(source)?.len(), 2);
        Ok(())
    }
}
