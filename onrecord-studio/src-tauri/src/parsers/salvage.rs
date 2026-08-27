use crate::error::AppError;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
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

#[derive(Default)]
struct RecoveryAccumulator {
    output: String,
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

pub fn extract_salvaged_text<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    // NOTE(forensics): This is heuristic string recovery, not a format-aware parse. Preserve the
    // fallback marker and primary extraction error anywhere this output is presented as evidence.
    //
    // LIMIT(forensics): There is no confidence score. The thresholds below are a binary admission
    // gate, so a successful return means only "enough characters passed the gate", not "this text
    // is trustworthy". The single machine-readable provenance signal is the
    // `[FORENSIC_RECOVERY_FALLBACK]` prefix baked into the returned string.
    //
    // LIMIT(forensics): Five decoders (ASCII plus UTF-16 LE/BE at two alignments) share one
    // accumulator, so decoding ASCII as UTF-16 yields alphanumeric CJK runs that the gate counts
    // as recovered text. Output order follows flush order across those decoders, not byte order in
    // the source. Do not present these lines as document order.
    // TODO(forensics): Return a structured result carrying the marker, the primary error, and a
    // per-run decoder tag instead of an in-band string prefix, so a caller cannot lose the
    // provenance by trimming the first line.
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
            process_ascii_byte(byte, &mut ascii, &mut accumulator, path)?;
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
                        path,
                    )?;
                    process_utf16_unit(
                        u16::from_be_bytes([first, byte]),
                        &mut utf16_big[alignment],
                        &mut accumulator,
                        path,
                    )?;
                }
            }
        }
    }

    flush_run(&mut ascii, &mut accumulator, path)?;
    for run in utf16_little.iter_mut().chain(utf16_big.iter_mut()) {
        run.pending_high_surrogate = None;
        flush_run(&mut run.text, &mut accumulator, path)?;
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

    Ok(format!(
        "[FORENSIC_RECOVERY_FALLBACK]\n{}",
        accumulator.output.trim_end()
    ))
}

fn process_ascii_byte(
    byte: u8,
    run: &mut TextRun,
    accumulator: &mut RecoveryAccumulator,
    path: &Path,
) -> Result<(), AppError> {
    if matches!(byte, b'\t' | b'\n' | b'\r' | 0x20..=0x7e) {
        run.value.push(if matches!(byte, b'\t' | b'\n' | b'\r') {
            ' '
        } else {
            char::from(byte)
        });
        flush_oversized_run(run, accumulator, path)
    } else {
        flush_run(run, accumulator, path)
    }
}

fn process_utf16_unit(
    unit: u16,
    run: &mut Utf16Run,
    accumulator: &mut RecoveryAccumulator,
    path: &Path,
) -> Result<(), AppError> {
    if (0xd800..=0xdbff).contains(&unit) {
        if run.pending_high_surrogate.replace(unit).is_some() {
            flush_run(&mut run.text, accumulator, path)?;
        }
        return Ok(());
    }
    let character = if (0xdc00..=0xdfff).contains(&unit) {
        let Some(high) = run.pending_high_surrogate.take() else {
            flush_run(&mut run.text, accumulator, path)?;
            return Ok(());
        };
        let scalar = 0x1_0000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(unit) - 0xdc00);
        char::from_u32(scalar)
    } else {
        if run.pending_high_surrogate.take().is_some() {
            flush_run(&mut run.text, accumulator, path)?;
        }
        char::from_u32(u32::from(unit))
    };
    if let Some(character) = character.filter(|value| is_salvage_character(*value)) {
        run.text.value.push(character);
        flush_oversized_run(&mut run.text, accumulator, path)
    } else {
        flush_run(&mut run.text, accumulator, path)
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
    path: &Path,
) -> Result<(), AppError> {
    if run.value.len() >= MAXIMUM_SINGLE_RUN_BYTES {
        flush_run(run, accumulator, path)?;
    }
    Ok(())
}

fn flush_run(
    run: &mut TextRun,
    accumulator: &mut RecoveryAccumulator,
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
        .output
        .len()
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
    accumulator.output.push_str(&normalized);
    accumulator.output.push('\n');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn salvage(bytes: &[u8]) -> Result<String, AppError> {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("fixture.bin");
        let mut file = std::fs::File::create(&path).expect("create fixture");
        file.write_all(bytes).expect("write fixture");
        file.sync_all().expect("flush fixture");
        extract_salvaged_text(&path)
    }

    // INVARIANT(forensics): every successful salvage carries the fallback marker as its first
    // line. That prefix is the ONLY machine-readable signal separating recovered text from a
    // verified parse once the text is stored, so losing it silently promotes a guess to evidence.
    #[test]
    fn every_successful_salvage_is_marked_as_a_fallback() {
        let text = "The requested records concern the municipal budget hearing minutes. \
                    Please review the attached schedule and confirm receipt promptly.";
        let recovered = salvage(text.as_bytes()).expect("coherent prose should salvage");

        assert!(
            recovered.starts_with("[FORENSIC_RECOVERY_FALLBACK]\n"),
            "missing provenance marker: {recovered:?}",
        );
        assert!(recovered.contains("municipal budget hearing"));
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
            extract_salvaged_text(&link).is_err(),
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

        assert!(recovered.contains("municipal budget hearing"));
        // If this ever becomes a single line, the decoders were changed and the LIMIT comment on
        // `extract_salvaged_text` needs revisiting.
        assert!(
            recovered.lines().count() >= 2,
            "expected decoder twins alongside the real text: {recovered:?}",
        );
    }
}
