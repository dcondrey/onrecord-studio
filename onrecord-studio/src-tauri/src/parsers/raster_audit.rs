//! Conservative, bounded raster metadata and embedded-preview inventory.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_bytes, finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence,
    EvidenceLocator, ForensicAuditOptions,
};
use crate::parsers::source::read_regular_file;
use std::path::Path;

const MAXIMUM_RASTER_BYTES: u64 = 256 * 1024 * 1024;
const MAXIMUM_CHUNKS_OR_SEGMENTS: usize = 100_000;
const MAXIMUM_DECODED_RASTER_BYTES: usize = 512 * 1024 * 1024;

pub fn audit_raster_file<P: AsRef<Path>>(
    path: P,
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // NOTE(raster-audit): JPEG DCT artifacts can indicate editing or uncovered edge remnants;
    // they cannot restore pixels that were completely overwritten.
    let bytes = read_regular_file(path.as_ref(), "RASTER_AUDIT", MAXIMUM_RASTER_BYTES)?;
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        audit_png(&bytes, options)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        audit_jpeg(&bytes, options)
    } else {
        Ok(EvidenceBundle {
            coverage_complete: false,
            warnings: vec!["RASTER_METADATA_FORMAT_UNSUPPORTED".to_string()],
            ..EvidenceBundle::default()
        })
    }
}

fn audit_png(bytes: &[u8], options: ForensicAuditOptions) -> Result<EvidenceBundle, AppError> {
    let mut bundle = EvidenceBundle {
        coverage_complete: true,
        ..EvidenceBundle::default()
    };
    let mut offset = 8_usize;
    let mut chunks = 0_usize;
    let mut saw_end = false;
    while offset.checked_add(12).is_some_and(|end| end <= bytes.len()) {
        chunks += 1;
        if chunks > MAXIMUM_CHUNKS_OR_SEGMENTS {
            return Err(AppError::DataParsing(
                "PNG_CHUNK_LIMIT_EXCEEDED".to_string(),
            ));
        }
        let length =
            u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("four bytes")) as usize;
        let data_start = offset + 8;
        let Some(data_end) = data_start.checked_add(length) else {
            return Err(AppError::DataParsing("PNG_CHUNK_SIZE_OVERFLOW".to_string()));
        };
        let Some(chunk_end) = data_end.checked_add(4) else {
            return Err(AppError::DataParsing("PNG_CHUNK_SIZE_OVERFLOW".to_string()));
        };
        if chunk_end > bytes.len() {
            bundle.coverage_complete = false;
            bundle.warnings.push("PNG_TRUNCATED_CHUNK".to_string());
            break;
        }
        let kind = &bytes[offset + 4..offset + 8];
        let data = &bytes[data_start..data_end];
        if kind == b"IHDR" && data.len() >= 10 && matches!(data[9], 4 | 6) {
            bundle.findings.push(finding_for_bytes(
                EvidenceClass::Preserved,
                EvidenceConfidence::Exact,
                "PNG_ALPHA_CHANNEL_PRESENT",
                EvidenceLocator { byte_offset: Some(offset as u64), ..EvidenceLocator::default() },
                data,
                Some("PNG color type includes an alpha channel"),
                "The PNG header declares an alpha channel. This identifies preserved transparency data, not hidden content.",
                options,
            )?);
        }
        if matches!(
            kind,
            b"tEXt" | b"iTXt" | b"zTXt" | b"eXIf" | b"iCCP" | b"gAMA" | b"cHRM" | b"sRGB"
        ) {
            let kind_name = String::from_utf8_lossy(kind);
            let preview = matches!(kind, b"tEXt" | b"iTXt")
                .then(|| String::from_utf8_lossy(data).into_owned());
            bundle.findings.push(finding_for_bytes(
                EvidenceClass::Preserved,
                EvidenceConfidence::Exact,
                format!("PNG_METADATA_{kind_name}"),
                EvidenceLocator {
                    byte_offset: Some(offset as u64),
                    ..EvidenceLocator::default()
                },
                data,
                preview.as_deref(),
                "The named ancillary PNG chunk is preserved in the source bytes.",
                options,
            )?);
        }
        if kind == b"IEND" {
            saw_end = true;
            if chunk_end < bytes.len() {
                bundle.findings.push(finding_for_bytes(
                    EvidenceClass::Preserved,
                    EvidenceConfidence::Exact,
                    "PNG_TRAILING_BYTES",
                    EvidenceLocator { byte_offset: Some(chunk_end as u64), ..EvidenceLocator::default() },
                    &bytes[chunk_end..],
                    None,
                    "Bytes remain after the PNG IEND chunk. Their presence is exact; their origin is not inferred.",
                    options,
                )?);
            }
            break;
        }
        offset = chunk_end;
    }
    if !saw_end {
        bundle.coverage_complete = false;
        bundle.warnings.push("PNG_IEND_NOT_FOUND".to_string());
    }
    append_png_pixel_measurements(bytes, options, &mut bundle)?;
    Ok(bundle)
}

fn append_png_pixel_measurements(
    bytes: &[u8],
    options: ForensicAuditOptions,
    bundle: &mut EvidenceBundle,
) -> Result<(), AppError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|error| {
        AppError::DataParsing(format!("PNG_PIXEL_AUDIT_HEADER_INVALID:{error}"))
    })?;
    let buffer_size = reader
        .output_buffer_size()
        .ok_or_else(|| AppError::DataParsing("PNG_PIXEL_AUDIT_SIZE_UNKNOWN".to_string()))?;
    if buffer_size > MAXIMUM_DECODED_RASTER_BYTES {
        return Err(AppError::DataParsing(
            "PNG_PIXEL_AUDIT_SIZE_EXCEEDED".to_string(),
        ));
    }
    let mut decoded = vec![0; buffer_size];
    let output = reader
        .next_frame(&mut decoded)
        .map_err(|error| AppError::DataParsing(format!("PNG_PIXEL_AUDIT_DECODE_FAILED:{error}")))?;
    decoded.truncate(output.buffer_size());
    let channels = match output.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Ok(()),
    };
    let alpha_index = matches!(channels, 2 | 4).then_some(channels - 1);
    let mut transparent = 0usize;
    let mut translucent = 0usize;
    let mut opaque = 0usize;
    if let Some(alpha_index) = alpha_index {
        for pixel in decoded.chunks_exact(channels) {
            match pixel[alpha_index] {
                0 => transparent += 1,
                255 => opaque += 1,
                _ => translucent += 1,
            }
        }
        bundle.findings.push(finding_for_text(
            EvidenceClass::Derived,
            EvidenceConfidence::Exact,
            "PNG_ALPHA_CHANNEL_STATISTICS",
            EvidenceLocator::default(),
            &format!("transparent={transparent};translucent={translucent};opaque={opaque}"),
            "Counts are derived exactly from decoded alpha samples. Transparency can preserve edge remnants but does not by itself imply hidden content.",
            options,
        )?);
    }
    let row_bytes = output.line_size;
    let width = output.width as usize;
    let height = output.height as usize;
    let mut tested_boundaries = 0usize;
    let mut high_contrast_boundaries = 0usize;
    for y in 0..height {
        for x in 1..width {
            let left = y * row_bytes + (x - 1) * channels;
            let right = y * row_bytes + x * channels;
            let delta = (0..channels.min(3))
                .map(|channel| decoded[left + channel].abs_diff(decoded[right + channel]) as usize)
                .sum::<usize>();
            tested_boundaries += 1;
            high_contrast_boundaries += usize::from(delta >= channels.min(3) * 192);
        }
    }
    bundle.findings.push(finding_for_text(
        EvidenceClass::Derived,
        EvidenceConfidence::Exact,
        "PNG_HORIZONTAL_BOUNDARY_RESIDUAL_MEASUREMENT",
        EvidenceLocator::default(),
        &format!("highContrast={high_contrast_boundaries};tested={tested_boundaries}"),
        "The measurement counts strong adjacent-pixel boundaries after applying the file's declared PNG decoding transforms. It is an edge inventory, not content recovery.",
        options,
    )?);
    Ok(())
}

fn audit_jpeg(bytes: &[u8], options: ForensicAuditOptions) -> Result<EvidenceBundle, AppError> {
    let mut bundle = EvidenceBundle {
        coverage_complete: true,
        ..EvidenceBundle::default()
    };
    let mut offset = 2_usize;
    let mut segments = 0_usize;
    let mut saw_end = false;
    while offset < bytes.len() {
        while offset < bytes.len() && bytes[offset] != 0xff {
            offset += 1;
        }
        while offset < bytes.len() && bytes[offset] == 0xff {
            offset += 1;
        }
        if offset >= bytes.len() {
            break;
        }
        let marker = bytes[offset];
        offset += 1;
        if marker == 0xd9 {
            saw_end = true;
            break;
        }
        if marker == 0xda {
            if let Some(relative) = bytes[offset..]
                .windows(2)
                .position(|window| window == [0xff, 0xd9])
            {
                offset += relative + 2;
                saw_end = true;
            }
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        segments += 1;
        if segments > MAXIMUM_CHUNKS_OR_SEGMENTS {
            return Err(AppError::DataParsing(
                "JPEG_SEGMENT_LIMIT_EXCEEDED".to_string(),
            ));
        }
        if offset + 2 > bytes.len() {
            break;
        }
        let length = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        if length < 2
            || offset
                .checked_add(length)
                .is_none_or(|end| end > bytes.len())
        {
            bundle.coverage_complete = false;
            bundle.warnings.push("JPEG_TRUNCATED_SEGMENT".to_string());
            break;
        }
        let data_start = offset + 2;
        let data_end = offset + length;
        let data = &bytes[data_start..data_end];
        let kind = match marker {
            0xfe => Some("JPEG_COMMENT"),
            0xe1 => Some("JPEG_APP1_METADATA"),
            0xe2 => Some("JPEG_APP2_PROFILE_OR_PREVIEW"),
            0xed => Some("JPEG_APP13_METADATA"),
            _ => None,
        };
        if let Some(kind) = kind {
            let preview = (marker == 0xfe).then(|| String::from_utf8_lossy(data).into_owned());
            bundle.findings.push(finding_for_bytes(
                EvidenceClass::Preserved,
                EvidenceConfidence::Exact,
                kind,
                EvidenceLocator { byte_offset: Some((data_start - 4) as u64), ..EvidenceLocator::default() },
                data,
                preview.as_deref(),
                "The JPEG marker segment is preserved in the source bytes; no claim is made about whether its payload is current or authoritative.",
                options,
            )?);
        }
        offset = data_end;
    }
    if !saw_end {
        bundle.coverage_complete = false;
        bundle.warnings.push("JPEG_EOI_NOT_FOUND".to_string());
    } else if offset < bytes.len() {
        bundle.findings.push(finding_for_bytes(
            EvidenceClass::Preserved,
            EvidenceConfidence::Exact,
            "JPEG_TRAILING_BYTES",
            EvidenceLocator {
                byte_offset: Some(offset as u64),
                ..EvidenceLocator::default()
            },
            &bytes[offset..],
            None,
            "Bytes remain after the JPEG end-of-image marker.",
            options,
        )?);
    }
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn reports_png_text_and_trailing_bytes() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("header");
            writer
                .write_image_data(&[0, 0, 0, 0, 255, 255, 255, 255])
                .expect("pixels");
        }
        let iend_offset = bytes.len() - 12;
        let mut metadata = Vec::new();
        push_png_chunk(&mut metadata, b"tEXt", b"Author\0Example");
        bytes.splice(iend_offset..iend_offset, metadata);
        bytes.extend_from_slice(b"tail");
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("sample.png");
        fs::write(&path, bytes).expect("write");
        let result = audit_raster_file(&path, ForensicAuditOptions::default()).expect("audit");
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "PNG_METADATA_tEXt"));
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "PNG_TRAILING_BYTES"));
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.kind == "PNG_ALPHA_CHANNEL_STATISTICS"));
        assert!(result
            .findings
            .iter()
            .any(|finding| { finding.kind == "PNG_HORIZONTAL_BOUNDARY_RESIDUAL_MEASUREMENT" }));
    }

    fn push_png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        output.extend_from_slice(&(data.len() as u32).to_be_bytes());
        output.extend_from_slice(kind);
        output.extend_from_slice(data);
        let mut crc = 0xffff_ffff_u32;
        for byte in kind.iter().chain(data) {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320_u32 & (0_u32.wrapping_sub(crc & 1)));
            }
        }
        output.extend_from_slice(&(!crc).to_be_bytes());
    }
}
