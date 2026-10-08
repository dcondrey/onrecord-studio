use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::error::AppError;
use crate::parsers::format::DocumentFormat;
use crate::parsers::source::inspect_regular_file;

const MAX_PACKAGE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_PACKAGE_ENTRIES: usize = 10_000;
const MAX_XML_PART_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TOTAL_XML_BYTES: u64 = 512 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: usize = 128 * 1024 * 1024;

// Searchable-text projection only. `office_audit.rs` is the separate attributed route for tracked
// changes, comments, embedded parts, relationship IDs, duplicate parts, and revision structure.
pub fn extract_office_package<P: AsRef<Path>>(
    file_path: P,
    format: DocumentFormat,
) -> Result<String, AppError> {
    let path = file_path.as_ref();
    if !matches!(
        format,
        DocumentFormat::WordPackage
            | DocumentFormat::PresentationPackage
            | DocumentFormat::OpenDocumentText
            | DocumentFormat::OpenDocumentPresentation
            | DocumentFormat::Ebook
    ) {
        return Err(AppError::DataParsing(
            "OFFICE_FORMAT_INVALID: format is not an extractable office package".to_string(),
        ));
    }
    inspect_regular_file(path, "OFFICE", MAX_PACKAGE_BYTES)?;

    let file = File::open(path).map_err(|error| {
        AppError::Internal(format!(
            "OFFICE_OPEN_FAILED: failed to open '{}': {error}",
            path.display()
        ))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        AppError::DataParsing(format!(
            "OFFICE_PACKAGE_INVALID: failed to parse '{}': {error}",
            path.display()
        ))
    })?;
    if archive.len() > MAX_PACKAGE_ENTRIES {
        return Err(AppError::DataParsing(format!(
            "OFFICE_PART_COUNT_EXCEEDED: '{}' contains more than {MAX_PACKAGE_ENTRIES} parts",
            path.display()
        )));
    }
    let mut parts = Vec::new();
    let mut total_xml_bytes = 0_u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| {
            AppError::DataParsing(format!(
                "OFFICE_PART_READ_FAILED: failed to inspect part {index}: {error}"
            ))
        })?;
        let name = entry.name().replace('\\', "/");
        if !is_content_part(&name, format) {
            continue;
        }
        if entry.size() > MAX_XML_PART_BYTES {
            return Err(AppError::DataParsing(format!(
                "OFFICE_PART_SIZE_EXCEEDED: '{name}' exceeds {MAX_XML_PART_BYTES} bytes"
            )));
        }
        total_xml_bytes = total_xml_bytes.checked_add(entry.size()).ok_or_else(|| {
            AppError::DataParsing("OFFICE_PART_SIZE_OVERFLOW: XML size overflow".to_string())
        })?;
        if total_xml_bytes > MAX_TOTAL_XML_BYTES {
            return Err(AppError::DataParsing(format!(
                "OFFICE_TOTAL_SIZE_EXCEEDED: content XML exceeds {MAX_TOTAL_XML_BYTES} bytes"
            )));
        }
        parts.push((part_sort_key(&name, format), index, name));
    }
    parts.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.2.cmp(&right.2)));

    let mut output = String::new();
    for (_, index, name) in parts {
        let mut entry = archive.by_index(index).map_err(|error| {
            AppError::DataParsing(format!(
                "OFFICE_PART_READ_FAILED: failed to open '{name}': {error}"
            ))
        })?;
        let mut bytes = Vec::new();
        let expected_size = entry.size();
        entry
            .by_ref()
            .take(MAX_XML_PART_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                AppError::DataParsing(format!(
                    "OFFICE_PART_READ_FAILED: failed to read '{name}': {error}"
                ))
            })?;
        if bytes.len() as u64 != expected_size {
            return Err(AppError::DataParsing(format!(
                "OFFICE_PART_SIZE_MISMATCH: '{name}' declared {expected_size} bytes but produced {}",
                bytes.len()
            )));
        }
        let xml = std::str::from_utf8(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
            .map_err(|error| {
                AppError::DataParsing(format!(
                    "OFFICE_PART_ENCODING_INVALID: '{name}' is not UTF-8 XML: {error}"
                ))
            })?;
        // NOTE(office): This deliberately ignores presentation styling, including rPr. That is
        // useful for search, but insufficient to prove a style-based concealment or revision.
        // Styling is deliberately absent from FTS; the attributed audit retains part and XML
        // provenance separately so a search projection is never presented as reconstruction.
        append_xml_text(xml, &mut output, path)?;
        push_boundary(&mut output, path)?;
    }

    let output = output.trim().to_string();
    if output.is_empty() {
        return Err(AppError::DataParsing(format!(
            "OFFICE_OUTPUT_EMPTY: '{}' contains no searchable text",
            path.display()
        )));
    }
    Ok(output)
}

fn is_content_part(name: &str, format: DocumentFormat) -> bool {
    match format {
        DocumentFormat::WordPackage => {
            name == "word/document.xml"
                || name == "word/footnotes.xml"
                || name == "word/endnotes.xml"
                || name == "word/comments.xml"
                || name == "docProps/core.xml"
                || name == "docProps/app.xml"
                || (name.starts_with("word/header") && name.ends_with(".xml"))
                || (name.starts_with("word/footer") && name.ends_with(".xml"))
        }
        DocumentFormat::PresentationPackage => {
            (name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
                || (name.starts_with("ppt/notesSlides/notesSlide") && name.ends_with(".xml"))
                || name == "docProps/core.xml"
                || name == "docProps/app.xml"
        }
        DocumentFormat::OpenDocumentText | DocumentFormat::OpenDocumentPresentation => {
            name == "content.xml" || name == "meta.xml"
        }
        DocumentFormat::Ebook => {
            name.ends_with(".xhtml")
                || name.ends_with(".html")
                || name.ends_with(".htm")
                || name.ends_with(".opf")
        }
        _ => false,
    }
}

fn part_sort_key(name: &str, format: DocumentFormat) -> (u8, u64) {
    let priority = match format {
        DocumentFormat::WordPackage if name == "word/document.xml" => 0,
        DocumentFormat::WordPackage if name.starts_with("word/header") => 1,
        DocumentFormat::WordPackage if name.starts_with("word/footer") => 2,
        DocumentFormat::WordPackage => 3,
        DocumentFormat::PresentationPackage if name.starts_with("ppt/slides/") => 0,
        DocumentFormat::PresentationPackage => 1,
        _ => 0,
    };
    let number = name
        .bytes()
        .filter(u8::is_ascii_digit)
        .fold(0_u64, |value, digit| {
            value
                .saturating_mul(10)
                .saturating_add(u64::from(digit - b'0'))
        });
    (priority, number)
}

fn append_xml_text(xml: &str, output: &mut String, path: &Path) -> Result<(), AppError> {
    let bytes = xml.as_bytes();
    let mut index = 0_usize;
    let mut ignored_depth = 0_usize;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            let start = index;
            while index < bytes.len() && bytes[index] != b'<' {
                index += 1;
            }
            if ignored_depth == 0 {
                append_text_segment(&xml[start..index], output, path)?;
            }
            continue;
        }
        if xml[index..].starts_with("<!--") {
            let end = xml[index + 4..]
                .find("-->")
                .map(|offset| index + 4 + offset + 3)
                .ok_or_else(|| {
                    AppError::DataParsing(format!(
                        "OFFICE_XML_INVALID: '{}' contains an unterminated XML comment",
                        path.display()
                    ))
                })?;
            index = end;
            continue;
        }
        if xml[index..].starts_with("<![CDATA[") {
            let content_start = index + 9;
            let content_end = xml[content_start..]
                .find("]]>")
                .map(|offset| content_start + offset)
                .ok_or_else(|| {
                    AppError::DataParsing(format!(
                        "OFFICE_XML_INVALID: '{}' contains an unterminated CDATA section",
                        path.display()
                    ))
                })?;
            if ignored_depth == 0 {
                append_text_segment(&xml[content_start..content_end], output, path)?;
            }
            index = content_end + 3;
            continue;
        }
        let end = find_tag_end(xml, index).ok_or_else(|| {
            AppError::DataParsing(format!(
                "OFFICE_XML_INVALID: '{}' contains an unterminated XML tag",
                path.display()
            ))
        })?;
        let tag = xml[index + 1..end].trim();
        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        let local_name = tag
            .trim_start_matches('/')
            .trim_start_matches('?')
            .split_ascii_whitespace()
            .next()
            .unwrap_or_default()
            .trim_end_matches('/')
            .rsplit(':')
            .next()
            .unwrap_or_default();
        let ignored_element =
            local_name.eq_ignore_ascii_case("script") || local_name.eq_ignore_ascii_case("style");
        let boundary_element = ["br", "cr", "p", "row", "tr"]
            .iter()
            .any(|name| local_name.eq_ignore_ascii_case(name));
        let spacing_element = ["tab", "s"]
            .iter()
            .any(|name| local_name.eq_ignore_ascii_case(name));
        if ignored_element {
            if closing {
                ignored_depth = ignored_depth.saturating_sub(1);
            } else if !self_closing {
                ignored_depth = ignored_depth.checked_add(1).ok_or_else(|| {
                    AppError::DataParsing(
                        "OFFICE_XML_IGNORED_DEPTH_OVERFLOW: ignored element depth overflow"
                            .to_string(),
                    )
                })?;
            }
        } else if ignored_depth == 0 && boundary_element {
            push_boundary(output, path)?;
        } else if ignored_depth == 0 && spacing_element {
            append_character(output, ' ', path)?;
        }
        index = end + 1;
    }
    Ok(())
}

fn find_tag_end(xml: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, character) in xml[start + 1..].char_indices() {
        match character {
            '\'' | '"' if quote == Some(character) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(character),
            '>' if quote.is_none() => return Some(start + 1 + offset),
            _ => {}
        }
    }
    None
}

fn append_text_segment(segment: &str, output: &mut String, path: &Path) -> Result<(), AppError> {
    let decoded = decode_xml_entities(segment).map_err(|entity| invalid_entity(path, &entity))?;
    for word in decoded.split_whitespace() {
        if !output.is_empty() && !output.ends_with(' ') && !output.ends_with('\n') {
            append_character(output, ' ', path)?;
        }
        append_string(output, word, path)?;
    }
    Ok(())
}

/// Decodes XML character and named entity references. An unknown name is kept literally; a
/// numeric reference naming no Unicode scalar is an error carrying the entity text.
pub(crate) fn decode_xml_entities(segment: &str) -> Result<String, String> {
    let mut decoded = String::with_capacity(segment.len());
    let mut remainder = segment;
    while let Some(start) = remainder.find('&') {
        decoded.push_str(&remainder[..start]);
        remainder = &remainder[start + 1..];
        let Some(end) = remainder.find(';') else {
            decoded.push('&');
            decoded.push_str(remainder);
            return Ok(decoded);
        };
        let entity = &remainder[..end];
        let character = match entity {
            "amp" => Some('&'),
            "apos" => Some('\''),
            "gt" => Some('>'),
            "lt" => Some('<'),
            "quot" => Some('"'),
            "nbsp" => Some(' '),
            "copy" => Some('\u{00a9}'),
            "reg" => Some('\u{00ae}'),
            "bull" => Some('\u{2022}'),
            "ndash" => Some('\u{2013}'),
            "mdash" => Some('\u{2014}'),
            "lsquo" => Some('\u{2018}'),
            "rsquo" => Some('\u{2019}'),
            "ldquo" => Some('\u{201c}'),
            "rdquo" => Some('\u{201d}'),
            "hellip" => Some('\u{2026}'),
            entity if entity.starts_with("#x") => Some(
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| entity.to_string())?,
            ),
            entity if entity.starts_with('#') => Some(
                entity[1..]
                    .parse::<u32>()
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| entity.to_string())?,
            ),
            _ => None,
        };
        if let Some(character) = character {
            decoded.push(character);
        } else {
            decoded.push('&');
            decoded.push_str(entity);
            decoded.push(';');
        }
        remainder = &remainder[end + 1..];
    }
    decoded.push_str(remainder);
    Ok(decoded)
}

fn invalid_entity(path: &Path, entity: &str) -> AppError {
    AppError::DataParsing(format!(
        "OFFICE_XML_ENTITY_INVALID: '{}' contains unsupported entity '&{entity};'",
        path.display()
    ))
}

fn push_boundary(output: &mut String, path: &Path) -> Result<(), AppError> {
    if !output.is_empty() && !output.ends_with('\n') {
        append_character(output, '\n', path)?;
    }
    Ok(())
}

fn append_string(output: &mut String, value: &str, path: &Path) -> Result<(), AppError> {
    if output
        .len()
        .checked_add(value.len())
        .is_none_or(|size| size > MAX_EXTRACTED_BYTES)
    {
        return Err(size_error(path));
    }
    output.push_str(value);
    Ok(())
}

fn append_character(output: &mut String, character: char, path: &Path) -> Result<(), AppError> {
    if output
        .len()
        .checked_add(character.len_utf8())
        .is_none_or(|size| size > MAX_EXTRACTED_BYTES)
    {
        return Err(size_error(path));
    }
    output.push(character);
    Ok(())
}

fn size_error(path: &Path) -> AppError {
    AppError::DataParsing(format!(
        "OFFICE_OUTPUT_SIZE_EXCEEDED: '{}' exceeds {MAX_EXTRACTED_BYTES} extracted bytes",
        path.display()
    ))
}
