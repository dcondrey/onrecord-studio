//! Controlled, bounded measurement of uniform block grids in PNG rasters.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use crate::parsers::source::inspect_regular_file;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

const MAXIMUM_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAXIMUM_PIXELS: u64 = 100_000_000;
const MAXIMUM_DECODED_BYTES: usize = 512 * 1024 * 1024;
const BLOCK_SIZES: &[usize] = &[2, 4, 8, 16, 32];

pub fn analyze_pixelation<P: AsRef<Path>>(
    path: P,
    options: ForensicAuditOptions,
) -> Result<EvidenceBundle, AppError> {
    // FEATURE(raster-audit): Measure block size, grid origin uncertainty, color quantization,
    // resampling, and renderer fingerprints before any candidate simulation is considered.
    // TODO(raster-audit): Add a sandboxed renderer fixture using investigator-supplied plaintext;
    // never run an open-ended DFS against real-world personal data in the desktop process.
    // NOTE(raster-audit): A successful fixture match demonstrates a vulnerable transformation,
    // not that arbitrary pixelated text is recoverable.
    let path = path.as_ref();
    inspect_regular_file(path, "PIXELATION", MAXIMUM_IMAGE_BYTES)?;
    let file = File::open(path)
        .map_err(|error| AppError::Storage(format!("PIXELATION_OPEN_FAILED: {error}")))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = match decoder.read_info() {
        Ok(reader) => reader,
        Err(error) => {
            return Ok(EvidenceBundle {
                coverage_complete: false,
                warnings: vec![format!("PIXELATION_PNG_REQUIRED:{error}")],
                ..EvidenceBundle::default()
            });
        }
    };
    let header = reader.info();
    let pixel_count = u64::from(header.width)
        .checked_mul(u64::from(header.height))
        .ok_or_else(|| AppError::DataParsing("PIXELATION_DIMENSION_OVERFLOW".to_string()))?;
    if pixel_count > MAXIMUM_PIXELS {
        return Err(AppError::DataParsing(format!(
            "PIXELATION_PIXEL_LIMIT_EXCEEDED: image contains more than {MAXIMUM_PIXELS} pixels"
        )));
    }
    let buffer_size = reader
        .output_buffer_size()
        .ok_or_else(|| AppError::DataParsing("PIXELATION_OUTPUT_SIZE_UNKNOWN".to_string()))?;
    if buffer_size > MAXIMUM_DECODED_BYTES {
        return Err(AppError::DataParsing(format!(
            "PIXELATION_DECODED_SIZE_EXCEEDED: decoded image exceeds {MAXIMUM_DECODED_BYTES} bytes"
        )));
    }
    let mut decoded = vec![0_u8; buffer_size];
    let output = reader
        .next_frame(&mut decoded)
        .map_err(|error| AppError::DataParsing(format!("PIXELATION_DECODE_FAILED: {error}")))?;
    decoded.truncate(output.buffer_size());
    let channels = match output.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => {
            return Err(AppError::DataParsing(
                "PIXELATION_INDEXED_OUTPUT_UNEXPECTED: palette expansion did not occur".to_string(),
            ));
        }
    };
    let width = output.width as usize;
    let height = output.height as usize;
    let row_bytes = output.line_size;
    let mut findings = Vec::new();
    for &block_size in BLOCK_SIZES {
        let columns = width / block_size;
        let rows = height / block_size;
        let total_blocks = columns.saturating_mul(rows);
        if total_blocks == 0 {
            continue;
        }
        let mut uniform_blocks = 0_usize;
        for block_y in 0..rows {
            for block_x in 0..columns {
                if block_is_uniform(
                    &decoded,
                    row_bytes,
                    channels,
                    block_x * block_size,
                    block_y * block_size,
                    block_size,
                ) {
                    uniform_blocks += 1;
                }
            }
        }
        let basis_points = (uniform_blocks as u128 * 10_000 / total_blocks as u128) as u16;
        findings.push(finding_for_text(
            EvidenceClass::Derived,
            EvidenceConfidence::Exact,
            "RASTER_UNIFORM_BLOCK_GRID_MEASUREMENT",
            EvidenceLocator::default(),
            &format!("blockSize={block_size};uniformBlocks={uniform_blocks};totalBlocks={total_blocks};coverageBasisPoints={basis_points}"),
            "This deterministic measurement counts completely uniform aligned blocks. High coverage can result from ordinary flat artwork and does not prove pixelation, redaction, or recoverable text.",
            options,
        )?);
    }
    Ok(EvidenceBundle {
        findings,
        warnings: Vec::new(),
        coverage_complete: true,
    })
}

fn block_is_uniform(
    bytes: &[u8],
    row_bytes: usize,
    channels: usize,
    x: usize,
    y: usize,
    size: usize,
) -> bool {
    let first = y * row_bytes + x * channels;
    let reference = &bytes[first..first + channels];
    for row in y..y + size {
        for column in x..x + size {
            let start = row * row_bytes + column * channels;
            if &bytes[start..start + channels] != reference {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn measures_uniform_png_blocks_without_inferring_hidden_text() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("uniform.png");
        let file = File::create(&path).expect("create");
        let mut encoder = png::Encoder::new(file, 8, 8);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("header");
        writer
            .write_image_data(&vec![42; 8 * 8 * 3])
            .expect("pixels");
        drop(writer);

        let result = analyze_pixelation(&path, ForensicAuditOptions::default()).expect("audit");
        assert!(result.coverage_complete);
        assert!(result.findings.iter().any(|finding| finding
            .masked_preview
            .as_deref()
            .is_some_and(|preview| preview.contains("coverageBasisPoints=10000"))));
        assert!(result
            .findings
            .iter()
            .all(|finding| finding.explanation.contains("does not prove")));
    }
}
