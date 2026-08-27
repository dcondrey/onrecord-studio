use std::char::REPLACEMENT_CHARACTER;
use std::path::Path;

use crate::error::AppError;
use crate::parsers::source::read_regular_file;

const MAX_TEXT_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TEXT_OUTPUT_BYTES: usize = 128 * 1024 * 1024;

pub fn extract_plain_text<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    let path = file_path.as_ref();
    let bytes = read_regular_file(path, "TEXT", MAX_TEXT_SOURCE_BYTES)?;

    let decoded = decode_text(&bytes, path)?;
    normalize_text(&decoded, path)
}

fn decode_text(bytes: &[u8], path: &Path) -> Result<String, AppError> {
    if let Some(bytes) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8(bytes.to_vec()).map_err(|error| {
            AppError::DataParsing(format!(
                "TEXT_UTF8_INVALID: '{}' contains invalid UTF-8 after its BOM: {error}",
                path.display()
            ))
        });
    }
    if let Some(bytes) = bytes.strip_prefix(&[0xff, 0xfe, 0x00, 0x00]) {
        return decode_utf32(bytes, true, path);
    }
    if let Some(bytes) = bytes.strip_prefix(&[0x00, 0x00, 0xfe, 0xff]) {
        return decode_utf32(bytes, false, path);
    }
    if let Some(bytes) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return decode_utf16(bytes, true, path);
    }
    if let Some(bytes) = bytes.strip_prefix(&[0xfe, 0xff]) {
        return decode_utf16(bytes, false, path);
    }
    if likely_utf16(bytes, true) {
        return decode_utf16(bytes, true, path);
    }
    if likely_utf16(bytes, false) {
        return decode_utf16(bytes, false, path);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(text.to_string());
    }
    reject_binary(bytes, path)?;
    Ok(decode_windows_1252(bytes))
}

fn likely_utf16(bytes: &[u8], little_endian: bool) -> bool {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return false;
    }
    let expected_zero_index = usize::from(little_endian);
    let pairs = bytes.len() / 2;
    let zero_count = bytes
        .chunks_exact(2)
        .filter(|pair| pair[expected_zero_index] == 0)
        .count();
    zero_count * 10 >= pairs * 6
}

fn decode_utf16(bytes: &[u8], little_endian: bool, path: &Path) -> Result<String, AppError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(AppError::DataParsing(format!(
            "TEXT_UTF16_INVALID: '{}' has an odd UTF-16 byte count",
            path.display()
        )));
    }
    let words = bytes.chunks_exact(2).map(|pair| {
        let pair = [pair[0], pair[1]];
        if little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    });
    Ok(char::decode_utf16(words)
        .map(|character| character.unwrap_or(REPLACEMENT_CHARACTER))
        .collect())
}

fn decode_utf32(bytes: &[u8], little_endian: bool, path: &Path) -> Result<String, AppError> {
    if !bytes.len().is_multiple_of(4) {
        return Err(AppError::DataParsing(format!(
            "TEXT_UTF32_INVALID: '{}' has an invalid UTF-32 byte count",
            path.display()
        )));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| {
            let chunk = [chunk[0], chunk[1], chunk[2], chunk[3]];
            let scalar = if little_endian {
                u32::from_le_bytes(chunk)
            } else {
                u32::from_be_bytes(chunk)
            };
            char::from_u32(scalar).unwrap_or(REPLACEMENT_CHARACTER)
        })
        .collect())
}

fn reject_binary(bytes: &[u8], path: &Path) -> Result<(), AppError> {
    let disallowed = bytes
        .iter()
        .filter(|byte| **byte == 0 || (**byte < 0x20 && !matches!(**byte, b'\t' | b'\n' | b'\r')))
        .count();
    let threshold = (bytes.len() / 100).max(1);
    if disallowed > threshold {
        return Err(AppError::DataParsing(format!(
            "TEXT_BINARY_CONTENT_REJECTED: '{}' does not contain a supported text encoding",
            path.display()
        )));
    }
    Ok(())
}

fn decode_windows_1252(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| windows_1252_char(*byte)).collect()
}

pub(crate) fn windows_1252_char(byte: u8) -> char {
    match byte {
        0x80 => '\u{20ac}',
        0x82 => '\u{201a}',
        0x83 => '\u{0192}',
        0x84 => '\u{201e}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02c6}',
        0x89 => '\u{2030}',
        0x8a => '\u{0160}',
        0x8b => '\u{2039}',
        0x8c => '\u{0152}',
        0x8e => '\u{017d}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201c}',
        0x94 => '\u{201d}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02dc}',
        0x99 => '\u{2122}',
        0x9a => '\u{0161}',
        0x9b => '\u{203a}',
        0x9c => '\u{0153}',
        0x9e => '\u{017e}',
        0x9f => '\u{0178}',
        0x81 | 0x8d | 0x8f | 0x90 | 0x9d => REPLACEMENT_CHARACTER,
        byte => char::from(byte),
    }
}

fn normalize_text(text: &str, path: &Path) -> Result<String, AppError> {
    let mut normalized = String::with_capacity(text.len().min(MAX_TEXT_OUTPUT_BYTES));
    let mut previous_was_carriage_return = false;
    for source_character in text.trim_start_matches('\u{feff}').chars() {
        if source_character == '\n' && previous_was_carriage_return {
            previous_was_carriage_return = false;
            continue;
        }
        let character = match source_character {
            '\r' => '\n',
            '\t' => ' ',
            character if character.is_control() => ' ',
            character => character,
        };
        previous_was_carriage_return = source_character == '\r';
        if normalized
            .len()
            .checked_add(character.len_utf8())
            .is_none_or(|size| size > MAX_TEXT_OUTPUT_BYTES)
        {
            return Err(AppError::DataParsing(format!(
                "TEXT_OUTPUT_SIZE_EXCEEDED: '{}' exceeds {MAX_TEXT_OUTPUT_BYTES} decoded bytes",
                path.display()
            )));
        }
        normalized.push(character);
    }
    let normalized = normalized.trim().to_string();
    if normalized.is_empty() {
        return Err(AppError::DataParsing(format!(
            "TEXT_OUTPUT_EMPTY: '{}' contains no searchable text",
            path.display()
        )));
    }
    Ok(normalized)
}
