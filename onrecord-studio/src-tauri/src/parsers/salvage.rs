use crate::error::AppError;
use crate::integrity::{digest_hex, sha256_hex};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

const MAXIMUM_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
const MAXIMUM_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const MAXIMUM_RECOVERED_RUNS: usize = 100_000;
const MAXIMUM_SINGLE_RUN_BYTES: usize = 16 * 1024;
const MINIMUM_RUN_CHARACTERS: usize = 4;
const MINIMUM_TOTAL_CHARACTERS: usize = 48;
const MINIMUM_TOTAL_ALPHABETIC_CHARACTERS: usize = 16;
const MINIMUM_LEXICAL_TOKENS: usize = 6;
const SALVAGE_PROVENANCE_MARKER: &str = "FORENSIC_RECOVERY_FALLBACK_V1";
const MAXIMUM_PRIMARY_ERROR_CHARACTERS: usize = 2_048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SalvageDecoder {
    Ascii,
    Utf16LeEven,
    Utf16LeOdd,
    Utf16BeEven,
    Utf16BeOdd,
}

impl SalvageDecoder {
    fn label(self) -> &'static str {
        match self {
            Self::Ascii => "ASCII",
            Self::Utf16LeEven => "UTF16_LE_EVEN",
            Self::Utf16LeOdd => "UTF16_LE_ODD",
            Self::Utf16BeEven => "UTF16_BE_EVEN",
            Self::Utf16BeOdd => "UTF16_BE_ODD",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SalvagedTextRun {
    pub decoder: SalvageDecoder,
    pub recovered_character_count: usize,
    pub text_sha256: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SalvageReport {
    pub provenance_marker: &'static str,
    pub primary_error: String,
    pub source_sha256: String,
    pub source_size_bytes: u64,
    pub recovered_character_count: usize,
    pub alphabetic_character_count: usize,
    pub lexical_token_count: usize,
    pub runs: Vec<SalvagedTextRun>,
    pub text: String,
}

impl SalvageReport {
    pub fn decoder_summary(&self) -> String {
        let mut counts = BTreeMap::<SalvageDecoder, usize>::new();
        for run in &self.runs {
            *counts.entry(run.decoder).or_default() += 1;
        }
        counts
            .into_iter()
            .map(|(decoder, count)| format!("{}={count}", decoder.label()))
            .collect::<Vec<_>>()
            .join(";")
    }
}

#[derive(Default)]
struct RecoveryAccumulator {
    runs: Vec<SalvagedTextRun>,
    output_bytes: usize,
    seen: HashSet<[u8; 32]>,
    recovered_characters: usize,
    alphabetic_characters: usize,
    lexical_tokens: usize,
}

#[derive(Default)]
struct TextRun {
    value: String,
}

#[derive(Default)]
struct Utf16Run {
    text: TextRun,
    pending_high_surrogate: Option<u16>,
}

pub fn extract_salvage_report<P: AsRef<Path>>(
    file_path: P,
    primary_error: impl Into<String>,
) -> Result<SalvageReport, AppError> {
    // This is heuristic string recovery, not a format-aware parse. Every admitted run retains its
    // decoder and digest in a structured report. The threshold is a binary gate, not a confidence
    // score, and output order is decoder flush order rather than asserted document order.
    let path = file_path.as_ref();
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        AppError::Storage(format!(
            "Failed to inspect salvage source '{}': {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAXIMUM_SOURCE_BYTES
    {
        return Err(AppError::DataParsing(format!(
            "Salvage source '{}' must be a regular, non-symlink file between 1 and {MAXIMUM_SOURCE_BYTES} bytes",
            path.display()
        )));
    }

    let file = File::open(path).map_err(|error| {
        AppError::Storage(format!(
            "Failed to open salvage source '{}': {error}",
            path.display()
        ))
    })?;
    let mut reader = BufReader::new(file.take(MAXIMUM_SOURCE_BYTES + 1));
    let mut buffer = [0_u8; 64 * 1024];
    let mut accumulator = RecoveryAccumulator::default();
    let mut ascii = TextRun::default();
    let mut utf16_little = [Utf16Run::default(), Utf16Run::default()];
    let mut utf16_big = [Utf16Run::default(), Utf16Run::default()];
    let mut pair_first = [None, None];
    let mut consumed = 0_u64;
    let mut source_hasher = Sha256::new();

    loop {
        let read = reader.read(&mut buffer).map_err(|error| {
            AppError::Storage(format!(
                "Failed to read salvage source '{}': {error}",
                path.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        source_hasher.update(&buffer[..read]);
        consumed = consumed.checked_add(read as u64).ok_or_else(|| {
            AppError::DataParsing("Salvage source byte count overflowed".to_string())
        })?;
        if consumed > MAXIMUM_SOURCE_BYTES {
            return Err(AppError::DataParsing(format!(
                "Salvage source '{}' changed while being read",
                path.display()
            )));
        }
        let chunk_start = consumed - read as u64;
        for (chunk_offset, byte) in buffer[..read].iter().copied().enumerate() {
            process_ascii_byte(
                byte,
                &mut ascii,
                &mut accumulator,
                SalvageDecoder::Ascii,
                path,
            )?;
            let absolute_offset = chunk_start + chunk_offset as u64;
            for alignment in 0..=1_usize {
                if absolute_offset < alignment as u64 {
                    continue;
                }
                if (absolute_offset - alignment as u64).is_multiple_of(2) {
                    pair_first[alignment] = Some(byte);
                } else if let Some(first) = pair_first[alignment].take() {
                    process_utf16_unit(
                        u16::from_le_bytes([first, byte]),
                        &mut utf16_little[alignment],
                        &mut accumulator,
                        if alignment == 0 {
                            SalvageDecoder::Utf16LeEven
                        } else {
                            SalvageDecoder::Utf16LeOdd
                        },
                        path,
                    )?;
                    process_utf16_unit(
                        u16::from_be_bytes([first, byte]),
                        &mut utf16_big[alignment],
                        &mut accumulator,
                        if alignment == 0 {
                            SalvageDecoder::Utf16BeEven
                        } else {
                            SalvageDecoder::Utf16BeOdd
                        },
                        path,
                    )?;
                }
            }
        }
    }

    flush_run(&mut ascii, &mut accumulator, SalvageDecoder::Ascii, path)?;
    for (alignment, run) in utf16_little.iter_mut().enumerate() {
        run.pending_high_surrogate = None;
        flush_run(
            &mut run.text,
            &mut accumulator,
            if alignment == 0 {
                SalvageDecoder::Utf16LeEven
            } else {
                SalvageDecoder::Utf16LeOdd
            },
            path,
        )?;
    }
    for (alignment, run) in utf16_big.iter_mut().enumerate() {
        run.pending_high_surrogate = None;
        flush_run(
            &mut run.text,
            &mut accumulator,
            if alignment == 0 {
                SalvageDecoder::Utf16BeEven
            } else {
                SalvageDecoder::Utf16BeOdd
            },
            path,
        )?;
    }
    if consumed != metadata.len() {
        return Err(AppError::DataParsing(
            "Salvage source changed while the recovery snapshot was being acquired".to_string(),
        ));
    }
    if accumulator.recovered_characters < MINIMUM_TOTAL_CHARACTERS
        || accumulator.alphabetic_characters < MINIMUM_TOTAL_ALPHABETIC_CHARACTERS
        || accumulator.lexical_tokens < MINIMUM_LEXICAL_TOKENS
        || accumulator.alphabetic_characters.saturating_mul(5) < accumulator.recovered_characters
    {
        return Err(AppError::DataParsing(format!(
            "Salvage source '{}' did not contain enough coherent text to satisfy the recovery confidence threshold",
            path.display()
        )));
    }

    let text = accumulator
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let primary_error = primary_error
        .into()
        .chars()
        .take(MAXIMUM_PRIMARY_ERROR_CHARACTERS)
        .collect();
    let source_digest = source_hasher.finalize();
    Ok(SalvageReport {
        provenance_marker: SALVAGE_PROVENANCE_MARKER,
        primary_error,
        source_sha256: digest_hex(source_digest),
        source_size_bytes: consumed,
        recovered_character_count: accumulator.recovered_characters,
        alphabetic_character_count: accumulator.alphabetic_characters,
        lexical_token_count: accumulator.lexical_tokens,
        runs: accumulator.runs,
        text,
    })
}

fn process_ascii_byte(
    byte: u8,
    run: &mut TextRun,
    accumulator: &mut RecoveryAccumulator,
    decoder: SalvageDecoder,
    path: &Path,
) -> Result<(), AppError> {
    if matches!(byte, b'\t' | b'\n' | b'\r' | 0x20..=0x7e) {
        run.value.push(if matches!(byte, b'\t' | b'\n' | b'\r') {
            ' '
        } else {
            char::from(byte)
        });
        flush_oversized_run(run, accumulator, decoder, path)
    } else {
        flush_run(run, accumulator, decoder, path)
    }
}

fn process_utf16_unit(
    unit: u16,
    run: &mut Utf16Run,
    accumulator: &mut RecoveryAccumulator,
    decoder: SalvageDecoder,
    path: &Path,
) -> Result<(), AppError> {
    if (0xd800..=0xdbff).contains(&unit) {
        if run.pending_high_surrogate.replace(unit).is_some() {
            flush_run(&mut run.text, accumulator, decoder, path)?;
        }
        return Ok(());
    }
    let character = if (0xdc00..=0xdfff).contains(&unit) {
        let Some(high) = run.pending_high_surrogate.take() else {
            flush_run(&mut run.text, accumulator, decoder, path)?;
            return Ok(());
        };
        let scalar = 0x1_0000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(unit) - 0xdc00);
        char::from_u32(scalar)
    } else {
        if run.pending_high_surrogate.take().is_some() {
            flush_run(&mut run.text, accumulator, decoder, path)?;
        }
        char::from_u32(u32::from(unit))
    };
    if let Some(character) = character.filter(|value| is_salvage_character(*value)) {
        run.text.value.push(character);
        flush_oversized_run(&mut run.text, accumulator, decoder, path)
    } else {
        flush_run(&mut run.text, accumulator, decoder, path)
    }
}

fn is_salvage_character(character: char) -> bool {
    !character.is_control()
        && (character.is_alphanumeric()
            || character.is_whitespace()
            || character.is_ascii_punctuation()
            || ('\u{00a0}'..='\u{024f}').contains(&character)
            || ('\u{2000}'..='\u{206f}').contains(&character)
            || ('\u{20a0}'..='\u{20cf}').contains(&character))
}

fn flush_oversized_run(
    run: &mut TextRun,
    accumulator: &mut RecoveryAccumulator,
    decoder: SalvageDecoder,
    path: &Path,
) -> Result<(), AppError> {
    if run.value.len() >= MAXIMUM_SINGLE_RUN_BYTES {
        flush_run(run, accumulator, decoder, path)?;
    }
    Ok(())
}

fn flush_run(
    run: &mut TextRun,
    accumulator: &mut RecoveryAccumulator,
    decoder: SalvageDecoder,
    path: &Path,
) -> Result<(), AppError> {
    let normalized = run.value.split_whitespace().collect::<Vec<_>>().join(" ");
    run.value.clear();
    let character_count = normalized.chars().count();
    if character_count < MINIMUM_RUN_CHARACTERS {
        return Ok(());
    }
    let alphabetic_count = normalized
        .chars()
        .filter(|character| character.is_alphabetic())
        .count();
    let lexical_tokens = normalized
        .split_whitespace()
        .filter(|token| {
            token
                .chars()
                .filter(|character| character.is_alphabetic())
                .count()
                >= 2
        })
        .count();
    if alphabetic_count < 2
        || lexical_tokens == 0
        || alphabetic_count.saturating_mul(5) < character_count
    {
        return Ok(());
    }
    let digest: [u8; 32] = Sha256::digest(normalized.as_bytes()).into();
    if !accumulator.seen.insert(digest) {
        return Ok(());
    }
    if accumulator.seen.len() > MAXIMUM_RECOVERED_RUNS {
        return Err(AppError::DataParsing(format!(
            "Salvage output for '{}' exceeds {MAXIMUM_RECOVERED_RUNS} distinct text runs",
            path.display()
        )));
    }
    if accumulator
        .output_bytes
        .checked_add(normalized.len())
        .and_then(|size| size.checked_add(1))
        .is_none_or(|size| size > MAXIMUM_OUTPUT_BYTES)
    {
        return Err(AppError::DataParsing(format!(
            "Salvage output for '{}' exceeds {MAXIMUM_OUTPUT_BYTES} bytes",
            path.display()
        )));
    }
    accumulator.recovered_characters = accumulator
        .recovered_characters
        .checked_add(character_count)
        .ok_or_else(|| AppError::DataParsing("Recovered character count overflowed".to_string()))?;
    accumulator.alphabetic_characters = accumulator
        .alphabetic_characters
        .checked_add(alphabetic_count)
        .ok_or_else(|| {
            AppError::DataParsing("Recovered alphabetic count overflowed".to_string())
        })?;
    accumulator.lexical_tokens = accumulator
        .lexical_tokens
        .checked_add(lexical_tokens)
        .ok_or_else(|| AppError::DataParsing("Recovered token count overflowed".to_string()))?;
    accumulator.output_bytes += normalized.len() + 1;
    accumulator.runs.push(SalvagedTextRun {
        decoder,
        recovered_character_count: character_count,
        text_sha256: sha256_hex(normalized.as_bytes()),
        text: normalized,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn salvage(bytes: &[u8]) -> Result<SalvageReport, AppError> {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("fixture.bin");
        let mut file = std::fs::File::create(&path).expect("create fixture");
        file.write_all(bytes).expect("write fixture");
        file.sync_all().expect("flush fixture");
        extract_salvage_report(&path, "primary parser fixture failure")
    }

    #[test]
    fn every_successful_salvage_has_structured_provenance_and_decoder_tags() {
        let text = "The requested records concern the municipal budget hearing minutes. \
                    Please review the attached schedule and confirm receipt promptly.";
        let report = salvage(text.as_bytes()).expect("coherent prose should salvage");

        assert_eq!(report.provenance_marker, SALVAGE_PROVENANCE_MARKER);
        assert_eq!(report.primary_error, "primary parser fixture failure");
        assert!(report.text.contains("municipal budget hearing"));
        assert!(report.runs.iter().any(|run| {
            run.decoder == SalvageDecoder::Ascii
                && run.text.contains("municipal budget hearing")
                && run.text_sha256 == sha256_hex(run.text.as_bytes())
        }));
        assert_eq!(report.source_sha256, sha256_hex(text.as_bytes()));
    }

    // LIMIT(forensics): the coherence gate is a binary admission threshold, not a score. Input
    // below it is rejected outright rather than returned with low confidence.
    #[test]
    fn input_below_the_coherence_gate_is_rejected_rather_than_returned_weakly() {
        let error = salvage(b"a b c").expect_err("too little text to clear the gate");
        assert!(matches!(error, AppError::DataParsing(_)), "got {error:?}");

        let error = salvage(&[0xFF_u8; 64]).expect_err("no recoverable characters");
        assert!(matches!(error, AppError::DataParsing(_)), "got {error:?}");
    }

    // SECURITY(forensics): salvage refuses sources it cannot treat as a consistent snapshot.
    // An empty file has nothing to recover and must not produce a marked, empty "recovery".
    #[test]
    fn empty_and_non_regular_sources_are_refused() {
        let error = salvage(b"").expect_err("an empty file has nothing to salvage");
        assert!(matches!(error, AppError::DataParsing(_)), "got {error:?}");
    }

    #[test]
    #[cfg(unix)]
    fn symlinked_sources_are_refused() {
        let directory = tempfile::tempdir().expect("temp dir");
        let target = directory.path().join("real.bin");
        std::fs::write(
            &target,
            "The municipal budget hearing minutes are attached herewith.",
        )
        .expect("write target");
        let link = directory.path().join("link.bin");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        assert!(
            extract_salvage_report(&link, "fixture").is_err(),
            "a symlinked salvage source must be refused",
        );
    }

    // LIMIT(forensics): five decoders share one accumulator, so decoding ASCII as UTF-16 yields
    // alphanumeric CJK runs that the gate counts as recovered text. This pins that the real text
    // is still present and still marked; it deliberately does NOT assert the output is clean.
    #[test]
    fn utf16_decoders_add_mojibake_alongside_the_real_text() {
        let text = "The requested records concern the municipal budget hearing minutes. \
                    Please review the attached schedule and confirm receipt promptly.";
        let recovered = salvage(text.as_bytes()).expect("salvage");

        assert!(recovered.text.contains("municipal budget hearing"));
        assert!(
            recovered
                .runs
                .iter()
                .any(|run| run.decoder != SalvageDecoder::Ascii),
            "expected decoder twins alongside the real text: {recovered:?}",
        );
    }
}
