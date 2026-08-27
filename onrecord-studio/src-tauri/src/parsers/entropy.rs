use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::AppError;

const SAMPLE_WINDOW_BYTES: u64 = 64 * 1024;
const MAXIMUM_SAMPLE_BYTES: u64 = SAMPLE_WINDOW_BYTES * 3;
const MINIMUM_CLASSIFICATION_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct EntropyAssessment {
    pub shannon_entropy: f64,
    pub sampled_bytes: u64,
    pub suspicious_data_density: bool,
}

pub fn calculate_shannon_entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }

    let mut frequencies = [0_u64; 256];
    for &byte in data {
        frequencies[usize::from(byte)] += 1;
    }

    let total = data.len() as f64;
    frequencies
        .iter()
        .filter(|&&count| count != 0)
        .fold(0.0, |entropy, &count| {
            let probability = count as f64 / total;
            entropy - probability * probability.log2()
        })
}

pub fn assess_file_entropy(
    path: &Path,
    declared_file_name: Option<&str>,
) -> Result<EntropyAssessment, AppError> {
    let mut file = File::open(path).map_err(|error| {
        AppError::DataParsing(format!(
            "Failed to open '{}' for entropy assessment: {error}",
            path.display()
        ))
    })?;
    let metadata = file.metadata().map_err(|error| {
        AppError::DataParsing(format!(
            "Failed to inspect '{}' for entropy assessment: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(AppError::DataParsing(format!(
            "Entropy assessment requires a regular file: '{}'",
            path.display()
        )));
    }

    let file_size = metadata.len();
    let sample = read_deterministic_sample(&mut file, file_size, path)?;
    let shannon_entropy = calculate_shannon_entropy(&sample);
    let claimed_file_name = declared_file_name
        .filter(|value| !value.trim().is_empty())
        .or_else(|| path.file_name().and_then(|value| value.to_str()));
    let suspicious_data_density = sample.len() >= MINIMUM_CLASSIFICATION_BYTES
        && anomaly_threshold(claimed_file_name)
            .is_some_and(|threshold| shannon_entropy > threshold);

    Ok(EntropyAssessment {
        shannon_entropy,
        sampled_bytes: sample.len() as u64,
        suspicious_data_density,
    })
}

fn read_deterministic_sample(
    file: &mut File,
    file_size: u64,
    path: &Path,
) -> Result<Vec<u8>, AppError> {
    let capacity = usize::try_from(file_size.min(MAXIMUM_SAMPLE_BYTES)).map_err(|error| {
        AppError::DataParsing(format!(
            "Entropy sample size for '{}' cannot be represented: {error}",
            path.display()
        ))
    })?;
    let mut sample = Vec::with_capacity(capacity);
    if file_size <= MAXIMUM_SAMPLE_BYTES {
        sample.resize(capacity, 0);
        file.read_exact(&mut sample).map_err(|error| {
            AppError::DataParsing(format!(
                "Failed to read entropy sample from '{}': {error}",
                path.display()
            ))
        })?;
        return Ok(sample);
    }

    let middle = (file_size - SAMPLE_WINDOW_BYTES) / 2;
    for offset in [0, middle, file_size - SAMPLE_WINDOW_BYTES] {
        file.seek(SeekFrom::Start(offset)).map_err(|error| {
            AppError::DataParsing(format!(
                "Failed to seek entropy sample in '{}': {error}",
                path.display()
            ))
        })?;
        let start = sample.len();
        sample.resize(start + SAMPLE_WINDOW_BYTES as usize, 0);
        file.read_exact(&mut sample[start..]).map_err(|error| {
            AppError::DataParsing(format!(
                "Failed to read entropy sample from '{}': {error}",
                path.display()
            ))
        })?;
    }
    Ok(sample)
}

fn anomaly_threshold(file_name: Option<&str>) -> Option<f64> {
    let extension = Path::new(file_name?)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    match extension.as_str() {
        "txt" | "log" | "csv" | "tsv" | "ini" | "conf" | "config" | "sql" | "inf" | "reg"
        | "sh" | "bat" | "cmd" | "ps1" | "patch" | "diff" | "bib" | "ris" | "enw" | "end"
        | "tex" | "prj" | "pbtxt" | "prototxt" => Some(6.25),
        "json" | "jsonl" | "ndjson" | "geojson" | "xml" | "yaml" | "yml" | "kml" | "html"
        | "htm" | "xhtml" | "css" | "sass" | "markdown" | "md" | "vcf" | "vcard" | "dxf"
        | "fods" | "sylk" | "slk" => Some(6.75),
        _ => None,
    }
}
