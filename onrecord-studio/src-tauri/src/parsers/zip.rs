//! Bounded, atomic ZIP extraction.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

use uuid::Uuid;

use crate::error::AppError;
use crate::parsers::staging::cleanup_failed_extraction;

const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENTRY_COUNT: usize = 2_000;
const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub fn extract_zip_container<A: AsRef<Path>, D: AsRef<Path>>(
    archive_path: A,
    destination: D,
) -> Result<(), AppError> {
    extract_zip_container_with_manifest(archive_path, destination).map(|_| ())
}

pub fn extract_zip_container_with_manifest<A: AsRef<Path>, D: AsRef<Path>>(
    archive_path: A,
    destination: D,
) -> Result<Vec<std::path::PathBuf>, AppError> {
    // Ordinary extraction is atomic and bounded. Compression side-channel inspection belongs in
    // compression_oracle.rs and analyzes a read-only snapshot without appending guesses.
    let archive_path = archive_path.as_ref();
    let destination = destination.as_ref();
    validate_archive(archive_path, destination)?;

    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        AppError::Internal(format!(
            "ZIP_DESTINATION_CREATE_FAILED: failed to create '{}': {error}",
            parent.display()
        ))
    })?;
    let destination_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "ZIP_DESTINATION_INVALID: '{}' has no valid final component",
                destination.display()
            ))
        })?;
    let staging = parent.join(format!(".{destination_name}.{}.extracting", Uuid::new_v4()));
    fs::create_dir(&staging).map_err(|error| {
        AppError::Internal(format!(
            "ZIP_STAGING_CREATE_FAILED: failed to create '{}': {error}",
            staging.display()
        ))
    })?;

    let relative_files = match extract_into(archive_path, &staging) {
        Ok(files) => files,
        Err(error) => {
            return Err(cleanup_failed_extraction(&staging, "ZIP", error));
        }
    };

    if let Err(error) = fs::rename(&staging, destination) {
        let commit_error = AppError::Internal(format!(
            "ZIP_COMMIT_FAILED: failed to commit '{}' as '{}': {error}",
            staging.display(),
            destination.display()
        ));
        return Err(cleanup_failed_extraction(&staging, "ZIP", commit_error));
    }

    Ok(relative_files
        .into_iter()
        .map(|relative_path| destination.join(relative_path))
        .collect())
}

fn validate_archive(archive_path: &Path, destination: &Path) -> Result<(), AppError> {
    let metadata = fs::metadata(archive_path).map_err(|error| {
        AppError::Internal(format!(
            "ZIP_ARCHIVE_INSPECTION_FAILED: failed to inspect '{}': {error}",
            archive_path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(AppError::DataParsing(format!(
            "ZIP_ARCHIVE_INVALID: '{}' is not a regular file",
            archive_path.display()
        )));
    }
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(AppError::DataParsing(format!(
            "ZIP_ARCHIVE_SIZE_EXCEEDED: '{}' exceeds {MAX_ARCHIVE_BYTES} bytes",
            archive_path.display()
        )));
    }
    if destination.as_os_str().is_empty() || destination.file_name().is_none() {
        return Err(AppError::DataParsing(
            "ZIP_DESTINATION_INVALID: destination must identify a new directory".to_string(),
        ));
    }
    if destination.try_exists().map_err(|error| {
        AppError::Internal(format!(
            "ZIP_DESTINATION_INSPECTION_FAILED: failed to inspect '{}': {error}",
            destination.display()
        ))
    })? {
        return Err(AppError::DataParsing(format!(
            "ZIP_DESTINATION_EXISTS: '{}' already exists",
            destination.display()
        )));
    }
    Ok(())
}

fn extract_into(archive_path: &Path, staging: &Path) -> Result<Vec<std::path::PathBuf>, AppError> {
    let file = File::open(archive_path).map_err(|error| {
        AppError::Internal(format!(
            "ZIP_ARCHIVE_OPEN_FAILED: failed to open '{}': {error}",
            archive_path.display()
        ))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| {
        AppError::DataParsing(format!(
            "ZIP_ARCHIVE_INVALID: failed to parse '{}': {error}",
            archive_path.display()
        ))
    })?;
    if archive.len() > MAX_ENTRY_COUNT {
        return Err(AppError::DataParsing(format!(
            "ZIP_ENTRY_COUNT_EXCEEDED: archive contains {} entries; limit is {MAX_ENTRY_COUNT}",
            archive.len()
        )));
    }

    let mut total_uncompressed = 0_u64;
    let mut files = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            if matches!(
                error,
                zip::result::ZipError::UnsupportedArchive(reason)
                    if reason == zip::result::ZipError::PASSWORD_REQUIRED
            ) {
                AppError::DataParsing(format!(
                    "ZIP_ENCRYPTED_ENTRY_REQUIRES_MANUAL_DECRYPTION: entry {index}; decrypt an investigator-controlled working copy and re-import it while retaining the encrypted original"
                ))
            } else {
                AppError::DataParsing(format!(
                    "ZIP_ENTRY_READ_FAILED: failed to open entry {index}: {error}"
                ))
            }
        })?;
        let relative_path = entry.enclosed_name().ok_or_else(|| {
            AppError::DataParsing(format!(
                "ZIP_ENTRY_PATH_INVALID: entry '{}' escapes the destination",
                entry.name()
            ))
        })?;
        validate_entry_type(&entry, &relative_path)?;

        let entry_size = entry.size();
        if entry_size > MAX_ENTRY_BYTES {
            return Err(AppError::DataParsing(format!(
                "ZIP_ENTRY_SIZE_EXCEEDED: '{}' exceeds {MAX_ENTRY_BYTES} bytes",
                relative_path.display()
            )));
        }
        total_uncompressed = total_uncompressed.checked_add(entry_size).ok_or_else(|| {
            AppError::DataParsing("ZIP_TOTAL_SIZE_OVERFLOW: expanded size overflow".to_string())
        })?;
        if total_uncompressed > MAX_TOTAL_UNCOMPRESSED_BYTES {
            return Err(AppError::DataParsing(format!(
                "ZIP_TOTAL_SIZE_EXCEEDED: expanded data exceeds {MAX_TOTAL_UNCOMPRESSED_BYTES} bytes"
            )));
        }

        let output_path = staging.join(&relative_path);
        if entry.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| {
                AppError::Internal(format!(
                    "ZIP_DIRECTORY_CREATE_FAILED: failed to create '{}': {error}",
                    output_path.display()
                ))
            })?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                AppError::Internal(format!(
                    "ZIP_DIRECTORY_CREATE_FAILED: failed to create '{}': {error}",
                    parent.display()
                ))
            })?;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)
            .map_err(|error| {
                AppError::Internal(format!(
                    "ZIP_ENTRY_CREATE_FAILED: failed to create '{}': {error}",
                    output_path.display()
                ))
            })?;
        let copied =
            io::copy(&mut entry.by_ref().take(entry_size + 1), &mut output).map_err(|error| {
                AppError::DataParsing(format!(
                    "ZIP_ENTRY_EXTRACT_FAILED: failed to extract '{}': {error}",
                    relative_path.display()
                ))
            })?;
        if copied != entry_size {
            return Err(AppError::DataParsing(format!(
                "ZIP_ENTRY_SIZE_MISMATCH: '{}' declared {entry_size} bytes but produced {copied}",
                relative_path.display()
            )));
        }
        output.flush().map_err(|error| {
            AppError::Internal(format!(
                "ZIP_ENTRY_FLUSH_FAILED: failed to flush '{}': {error}",
                output_path.display()
            ))
        })?;
        output.sync_all().map_err(|error| {
            AppError::Internal(format!(
                "ZIP_ENTRY_SYNC_FAILED: failed to sync '{}': {error}",
                output_path.display()
            ))
        })?;
        files.push(relative_path);
    }
    Ok(files)
}

fn validate_entry_type(
    entry: &zip::read::ZipFile<'_>,
    relative_path: &Path,
) -> Result<(), AppError> {
    if let Some(mode) = entry.unix_mode() {
        let file_type = mode & 0o170000;
        if file_type != 0 && file_type != 0o040000 && file_type != 0o100000 {
            return Err(AppError::DataParsing(format!(
                "ZIP_ENTRY_TYPE_REJECTED: '{}' is not a regular file or directory",
                relative_path.display()
            )));
        }
    }
    Ok(())
}
