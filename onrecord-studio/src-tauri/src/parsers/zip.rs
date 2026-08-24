//! Archive decompression pipeline processing nested attachment packages.

use std::fs;
use std::path::Path;
use crate::error::AppError;

/// Recursively unzips document containers into target tracking space paths.
pub fn extract_zip_container<P: AsRef<Path>>(archive_path: P, destination: P) -> Result<(), AppError> {
    let file = fs::File::open(archive_path)
        .map_err(|e| AppError::Internal(format!("Failed to access file stream: {}", e)))?;
        
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| AppError::DataParsing(format!("Corrupt compressed stream layer: {}", e)))?;

    zip.extract(destination)
        .map_err(|e| AppError::DataParsing(format!("Failed to unpack internal nodes: {}", e)))
}
