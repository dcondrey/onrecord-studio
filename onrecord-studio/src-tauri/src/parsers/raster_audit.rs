//! Conservative, bounded raster metadata and embedded-preview inventory.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_bytes, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use crate::parsers::source::read_regular_file;
use std::path::Path;

const MAXIMUM_RASTER_BYTES: u64 = 256 * 1024 * 1024;
const MAXIMUM_CHUNKS_OR_SEGMENTS: usize = 100_000;

pub fn audit_raster_file<P: AsRef<Path>>(
    path: P,
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // FEATURE(raster-audit): Extract EXIF/IFD thumbnails, PNG text/XMP chunks, embedded previews,
    // alpha-channel statistics, and hashes as separate evidence artifacts.
    // TODO(raster-audit): Add derived gamma/curve previews and boundary-residual measurements.
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
        if matches!(kind, b"tEXt" | b"iTXt" | b"zTXt" | b"eXIf" | b"iCCP") {
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
    Ok(bundle)
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
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        push_png_chunk(&mut bytes, b"tEXt", b"Author\0Example");
        push_png_chunk(&mut bytes, b"IEND", b"");
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
    }

    fn push_png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        output.extend_from_slice(&(data.len() as u32).to_be_bytes());
        output.extend_from_slice(kind);
        output.extend_from_slice(data);
        output.extend_from_slice(&[0, 0, 0, 0]);
    }
}
