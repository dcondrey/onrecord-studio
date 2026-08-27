use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::error::AppError;
use crate::parsers::text::windows_1252_char;

const MAX_RTF_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RTF_OUTPUT_BYTES: usize = 128 * 1024 * 1024;
const MAX_RTF_GROUP_DEPTH: usize = 512;

#[derive(Clone, Copy)]
struct RtfState {
    ignorable: bool,
    unicode_fallback_count: usize,
}

pub fn extract_rtf_text<P: AsRef<Path>>(file_path: P) -> Result<String, AppError> {
    // FEATURE(rtf-audit): Add a parallel attributed parser for hidden/deleted text, comments,
    // revision marks, fields, and HTML encapsulation. Keep this flattening path stable for FTS.
    let path = file_path.as_ref();
    let metadata = std::fs::metadata(path).map_err(|error| {
        AppError::Internal(format!(
            "RTF_INSPECTION_FAILED: failed to inspect '{}': {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() || metadata.len() > MAX_RTF_BYTES {
        return Err(AppError::DataParsing(format!(
            "RTF_SOURCE_INVALID: '{}' must be a regular file no larger than {MAX_RTF_BYTES} bytes",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_RTF_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| {
            AppError::Internal(format!(
                "RTF_READ_FAILED: failed to read '{}': {error}",
                path.display()
            ))
        })?;
    if bytes.len() as u64 > MAX_RTF_BYTES || !bytes.starts_with(b"{\\rtf") {
        return Err(AppError::DataParsing(format!(
            "RTF_FORMAT_INVALID: '{}' is not a supported RTF document",
            path.display()
        )));
    }
    parse_rtf(&bytes, path)
}

fn parse_rtf(bytes: &[u8], path: &Path) -> Result<String, AppError> {
    let mut output = String::new();
    let mut stack = vec![RtfState {
        ignorable: false,
        unicode_fallback_count: 1,
    }];
    let mut index = 0_usize;
    let mut fallback_characters = 0_usize;
    let mut high_surrogate = None;

    while index < bytes.len() {
        match bytes[index] {
            b'{' => {
                if stack.len() >= MAX_RTF_GROUP_DEPTH {
                    return Err(AppError::DataParsing(format!(
                        "RTF_GROUP_DEPTH_EXCEEDED: '{}' exceeds {MAX_RTF_GROUP_DEPTH} nested groups",
                        path.display()
                    )));
                }
                stack.push(*stack.last().unwrap_or(&RtfState {
                    ignorable: false,
                    unicode_fallback_count: 1,
                }));
                index += 1;
            }
            b'}' => {
                if stack.len() == 1 {
                    return Err(AppError::DataParsing(format!(
                        "RTF_GROUP_UNBALANCED: '{}' contains an unmatched closing group",
                        path.display()
                    )));
                }
                stack.pop();
                index += 1;
            }
            b'\\' => {
                index += 1;
                if index >= bytes.len() {
                    break;
                }
                if matches!(bytes[index], b'\\' | b'{' | b'}') {
                    append_fallback_or_text(
                        &mut output,
                        windows_1252_char(bytes[index]),
                        &mut fallback_characters,
                        stack.last().is_some_and(|state| state.ignorable),
                        path,
                    )?;
                    index += 1;
                    continue;
                }
                if bytes[index] == b'\'' {
                    if index + 2 >= bytes.len() {
                        return Err(AppError::DataParsing(format!(
                            "RTF_HEX_ESCAPE_INVALID: '{}' ends inside a hexadecimal escape",
                            path.display()
                        )));
                    }
                    let value =
                        parse_hex_byte(bytes[index + 1], bytes[index + 2]).ok_or_else(|| {
                            AppError::DataParsing(format!(
                            "RTF_HEX_ESCAPE_INVALID: '{}' contains an invalid hexadecimal escape",
                            path.display()
                        ))
                        })?;
                    append_fallback_or_text(
                        &mut output,
                        windows_1252_char(value),
                        &mut fallback_characters,
                        stack.last().is_some_and(|state| state.ignorable),
                        path,
                    )?;
                    index += 3;
                    continue;
                }
                if bytes[index] == b'*' {
                    if let Some(state) = stack.last_mut() {
                        state.ignorable = true;
                    }
                    index += 1;
                    continue;
                }
                if matches!(bytes[index], b'\r' | b'\n') {
                    if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                        index += 1;
                    }
                    index += 1;
                    continue;
                }
                if !bytes[index].is_ascii_alphabetic() {
                    let symbol = bytes[index];
                    index += 1;
                    if let Some(character) = match symbol {
                        b'~' => Some(' '),
                        b'_' => Some('-'),
                        _ => None,
                    } {
                        append_fallback_or_text(
                            &mut output,
                            character,
                            &mut fallback_characters,
                            stack.last().is_some_and(|state| state.ignorable),
                            path,
                        )?;
                    }
                    continue;
                }

                let word_start = index;
                while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
                    index += 1;
                }
                let word = std::str::from_utf8(&bytes[word_start..index]).map_err(|error| {
                    AppError::DataParsing(format!(
                        "RTF_CONTROL_WORD_INVALID: '{}' contains an invalid control word: {error}",
                        path.display()
                    ))
                })?;
                let negative = bytes.get(index) == Some(&b'-');
                if negative {
                    index += 1;
                }
                let parameter_start = index;
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
                let parameter = if parameter_start == index {
                    None
                } else {
                    std::str::from_utf8(&bytes[parameter_start..index])
                        .ok()
                        .and_then(|value| value.parse::<i32>().ok())
                        .map(|value| if negative { -value } else { value })
                };
                if bytes.get(index) == Some(&b' ') {
                    index += 1;
                }

                if is_ignored_destination(word) {
                    if let Some(state) = stack.last_mut() {
                        state.ignorable = true;
                    }
                    continue;
                }
                let state = *stack.last().unwrap_or(&RtfState {
                    ignorable: false,
                    unicode_fallback_count: 1,
                });
                if state.ignorable {
                    if word == "bin" {
                        index = skip_binary(index, parameter, bytes.len(), path)?;
                    }
                    continue;
                }
                match word {
                    "par" | "line" => append_character(&mut output, '\n', path)?,
                    "tab" => append_character(&mut output, ' ', path)?,
                    "emdash" => append_character(&mut output, '\u{2014}', path)?,
                    "endash" => append_character(&mut output, '\u{2013}', path)?,
                    "bullet" => append_character(&mut output, '\u{2022}', path)?,
                    "lquote" => append_character(&mut output, '\u{2018}', path)?,
                    "rquote" => append_character(&mut output, '\u{2019}', path)?,
                    "ldblquote" => append_character(&mut output, '\u{201c}', path)?,
                    "rdblquote" => append_character(&mut output, '\u{201d}', path)?,
                    "uc" => {
                        if let Some(value) = parameter.and_then(|value| usize::try_from(value).ok())
                        {
                            if let Some(state) = stack.last_mut() {
                                state.unicode_fallback_count = value.min(32);
                            }
                        }
                    }
                    "u" => {
                        if let Some(value) = parameter {
                            append_utf16_unit(
                                &mut output,
                                value as i16 as u16,
                                &mut high_surrogate,
                                path,
                            )?;
                            fallback_characters = state.unicode_fallback_count;
                        }
                    }
                    "bin" => index = skip_binary(index, parameter, bytes.len(), path)?,
                    _ => {}
                }
            }
            b'\r' | b'\n' => index += 1,
            byte => {
                append_fallback_or_text(
                    &mut output,
                    windows_1252_char(byte),
                    &mut fallback_characters,
                    stack.last().is_some_and(|state| state.ignorable),
                    path,
                )?;
                index += 1;
            }
        }
    }

    if stack.len() != 1 {
        return Err(AppError::DataParsing(format!(
            "RTF_GROUP_UNBALANCED: '{}' contains an unclosed group",
            path.display()
        )));
    }
    if high_surrogate.is_some() {
        append_character(&mut output, char::REPLACEMENT_CHARACTER, path)?;
    }
    let output = output.trim().to_string();
    if output.is_empty() {
        return Err(AppError::DataParsing(format!(
            "RTF_OUTPUT_EMPTY: '{}' contains no searchable text",
            path.display()
        )));
    }
    Ok(output)
}

fn append_fallback_or_text(
    output: &mut String,
    character: char,
    fallback_characters: &mut usize,
    ignorable: bool,
    path: &Path,
) -> Result<(), AppError> {
    if *fallback_characters > 0 {
        *fallback_characters -= 1;
    } else if !ignorable {
        append_character(output, character, path)?;
    }
    Ok(())
}

fn append_utf16_unit(
    output: &mut String,
    unit: u16,
    high_surrogate: &mut Option<u16>,
    path: &Path,
) -> Result<(), AppError> {
    if (0xd800..=0xdbff).contains(&unit) {
        if high_surrogate.replace(unit).is_some() {
            append_character(output, char::REPLACEMENT_CHARACTER, path)?;
        }
        return Ok(());
    }
    if (0xdc00..=0xdfff).contains(&unit) {
        if let Some(high) = high_surrogate.take() {
            let scalar = 0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(unit) - 0xdc00);
            append_character(
                output,
                char::from_u32(scalar).unwrap_or(char::REPLACEMENT_CHARACTER),
                path,
            )?;
        } else {
            append_character(output, char::REPLACEMENT_CHARACTER, path)?;
        }
        return Ok(());
    }
    if high_surrogate.take().is_some() {
        append_character(output, char::REPLACEMENT_CHARACTER, path)?;
    }
    append_character(
        output,
        char::from_u32(u32::from(unit)).unwrap_or(char::REPLACEMENT_CHARACTER),
        path,
    )
}

fn append_character(output: &mut String, character: char, path: &Path) -> Result<(), AppError> {
    if output
        .len()
        .checked_add(character.len_utf8())
        .is_none_or(|size| size > MAX_RTF_OUTPUT_BYTES)
    {
        return Err(AppError::DataParsing(format!(
            "RTF_OUTPUT_SIZE_EXCEEDED: '{}' exceeds {MAX_RTF_OUTPUT_BYTES} extracted bytes",
            path.display()
        )));
    }
    output.push(character);
    Ok(())
}

fn skip_binary(
    index: usize,
    parameter: Option<i32>,
    input_length: usize,
    path: &Path,
) -> Result<usize, AppError> {
    let byte_count = parameter
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "RTF_BINARY_LENGTH_INVALID: '{}' contains an invalid binary length",
                path.display()
            ))
        })?;
    let end = index.checked_add(byte_count).ok_or_else(|| {
        AppError::DataParsing("RTF_BINARY_LENGTH_OVERFLOW: binary range overflow".to_string())
    })?;
    if end > input_length {
        return Err(AppError::DataParsing(format!(
            "RTF_BINARY_TRUNCATED: '{}' ends inside binary data",
            path.display()
        )));
    }
    Ok(end)
}

fn parse_hex_byte(high: u8, low: u8) -> Option<u8> {
    Some(hex_nibble(high)? * 16 + hex_nibble(low)?)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn is_ignored_destination(word: &str) -> bool {
    // TODO(rtf-audit): Record skipped destinations as evidence instead of discarding them in the
    // forensic path. The ordinary extractor may continue treating style tables as non-text.
    matches!(
        word,
        "colortbl"
            | "datastore"
            | "datafield"
            | "filetbl"
            | "fonttbl"
            | "generator"
            | "info"
            | "listoverridetable"
            | "listtable"
            | "object"
            | "pict"
            | "stylesheet"
            | "xmlnstbl"
    )
}
