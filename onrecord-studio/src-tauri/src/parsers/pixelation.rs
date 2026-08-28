//! Controlled, bounded measurement of uniform block grids in PNG rasters.

use crate::error::AppError;
use crate::parsers::forensic_evidence::{
    finding_for_text, EvidenceBundle, EvidenceClass, EvidenceConfidence, EvidenceLocator,
    ForensicAuditOptions,
};
use crate::parsers::source::inspect_regular_file;
use std::collections::HashSet;
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
    let mut unique_colors = HashSet::new();
    for row in 0..height {
        for column in 0..width {
            let start = row * row_bytes + column * channels;
            unique_colors.insert(decoded[start..start + channels].to_vec());
            if unique_colors.len() > 65_536 {
                break;
            }
        }
        if unique_colors.len() > 65_536 {
            break;
        }
    }
    let mut findings = Vec::new();
    let raster = RasterView {
        bytes: &decoded,
        row_bytes,
        channels,
        width,
        height,
    };
    for &block_size in BLOCK_SIZES {
        let mut best = (0_usize, 0_usize, 0_usize, 0_usize);
        for origin_y in [0, block_size / 2] {
            for origin_x in [0, block_size / 2] {
                let (uniform, total) =
                    measure_uniform_grid(&raster, block_size, (origin_x, origin_y));
                if total > 0
                    && (best.3 == 0
                        || uniform.saturating_mul(best.3) > best.2.saturating_mul(total))
                {
                    best = (origin_x, origin_y, uniform, total);
                }
            }
        }
        let (origin_x, origin_y, uniform_blocks, total_blocks) = best;
        if total_blocks == 0 {
            continue;
        }
        let basis_points = (uniform_blocks as u128 * 10_000 / total_blocks as u128) as u16;
        findings.push(finding_for_text(
            EvidenceClass::Derived,
            EvidenceConfidence::Exact,
            "RASTER_UNIFORM_BLOCK_GRID_MEASUREMENT",
            EvidenceLocator::default(),
            &format!("blockSize={block_size};bestOrigin={origin_x},{origin_y};uniformBlocks={uniform_blocks};totalBlocks={total_blocks};coverageBasisPoints={basis_points};uniqueColorsAtMost={}", unique_colors.len()),
            "This deterministic measurement compares two grid origins, counts uniform blocks, and reports color quantization. High coverage can result from ordinary flat artwork and does not prove pixelation, redaction, or recoverable text.",
            options,
        )?);
    }
    Ok(EvidenceBundle {
        findings,
        warnings: Vec::new(),
        coverage_complete: true,
    })
}

struct RasterView<'a> {
    bytes: &'a [u8],
    row_bytes: usize,
    channels: usize,
    width: usize,
    height: usize,
}

fn measure_uniform_grid(
    raster: &RasterView<'_>,
    size: usize,
    origin: (usize, usize),
) -> (usize, usize) {
    let (origin_x, origin_y) = origin;
    let columns = raster.width.saturating_sub(origin_x) / size;
    let rows = raster.height.saturating_sub(origin_y) / size;
    let mut uniform = 0;
    for block_y in 0..rows {
        for block_x in 0..columns {
            if block_is_uniform(
                raster.bytes,
                raster.row_bytes,
                raster.channels,
                origin_x + block_x * size,
                origin_y + block_y * size,
                size,
            ) {
                uniform += 1;
            }
        }
    }
    (uniform, columns.saturating_mul(rows))
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
        writer.write_image_data(&[42; 8 * 8 * 3]).expect("pixels");
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
