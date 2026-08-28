use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use bzip2::read::MultiBzDecoder;
use flate2::read::MultiGzDecoder;
use uuid::Uuid;
use xz2::read::XzDecoder;

use crate::error::AppError;
use crate::parsers::format::DocumentFormat;
use crate::parsers::staging::cleanup_failed_extraction;

const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ENTRY_COUNT: usize = 2_000;
const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TOTAL_UNCOMPRESSED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Slack for tar's own 512-byte headers and block padding, which the stream budget sees but the
/// per-entry accounting does not. At `MAX_ENTRY_COUNT` entries the real overhead is under 3 MB.
const TAR_METADATA_HEADROOM_BYTES: u64 = 8 * 1024 * 1024;

pub fn extract_archive_container<A: AsRef<Path>, D: AsRef<Path>>(
    archive_path: A,
    destination: D,
    format: DocumentFormat,
) -> Result<Vec<PathBuf>, AppError> {
    let archive_path = archive_path.as_ref();
    let destination = destination.as_ref();
    validate_source(archive_path, destination)?;
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        AppError::Internal(format!(
            "ARCHIVE_DESTINATION_CREATE_FAILED: failed to create '{}': {error}",
            parent.display()
        ))
    })?;
    let destination_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            AppError::DataParsing(format!(
                "ARCHIVE_DESTINATION_INVALID: '{}' has no valid final component",
                destination.display()
            ))
        })?;
    let staging = parent.join(format!(".{destination_name}.{}.extracting", Uuid::new_v4()));
    fs::create_dir(&staging).map_err(|error| {
        AppError::Internal(format!(
            "ARCHIVE_STAGING_CREATE_FAILED: failed to create '{}': {error}",
            staging.display()
        ))
    })?;

    let result = extract_to_staging(archive_path, &staging, format);
    let relative_files = match result {
        Ok(files) => files,
        Err(error) => {
            return Err(cleanup_failed_extraction(&staging, "ARCHIVE", error));
        }
    };
    if let Err(error) = fs::rename(&staging, destination) {
        let commit_error = AppError::Internal(format!(
            "ARCHIVE_COMMIT_FAILED: failed to commit '{}' as '{}': {error}",
            staging.display(),
            destination.display()
        ));
        return Err(cleanup_failed_extraction(&staging, "ARCHIVE", commit_error));
    }
    Ok(relative_files
        .into_iter()
        .map(|relative_path| destination.join(relative_path))
        .collect())
}

fn validate_source(archive_path: &Path, destination: &Path) -> Result<(), AppError> {
    let metadata = fs::metadata(archive_path).map_err(|error| {
        AppError::Internal(format!(
            "ARCHIVE_INSPECTION_FAILED: failed to inspect '{}': {error}",
            archive_path.display()
        ))
    })?;
    if !metadata.is_file() || metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(AppError::DataParsing(format!(
            "ARCHIVE_SOURCE_INVALID: '{}' must be a regular file no larger than {MAX_ARCHIVE_BYTES} bytes",
            archive_path.display()
        )));
    }
    if destination.as_os_str().is_empty() || destination.file_name().is_none() {
        return Err(AppError::DataParsing(
            "ARCHIVE_DESTINATION_INVALID: destination must identify a new directory".to_string(),
        ));
    }
    if destination.try_exists().map_err(|error| {
        AppError::Internal(format!(
            "ARCHIVE_DESTINATION_INSPECTION_FAILED: failed to inspect '{}': {error}",
            destination.display()
        ))
    })? {
        return Err(AppError::DataParsing(format!(
            "ARCHIVE_DESTINATION_EXISTS: '{}' already exists",
            destination.display()
        )));
    }
    Ok(())
}

fn extract_to_staging(
    archive_path: &Path,
    staging: &Path,
    format: DocumentFormat,
) -> Result<Vec<PathBuf>, AppError> {
    let file = File::open(archive_path).map_err(|error| {
        AppError::Internal(format!(
            "ARCHIVE_OPEN_FAILED: failed to open '{}': {error}",
            archive_path.display()
        ))
    })?;
    match format {
        DocumentFormat::Tar => extract_tar(budgeted(file), staging, archive_path),
        DocumentFormat::TarGzip => {
            extract_tar(budgeted(MultiGzDecoder::new(file)), staging, archive_path)
        }
        DocumentFormat::TarBzip2 => {
            extract_tar(budgeted(MultiBzDecoder::new(file)), staging, archive_path)
        }
        DocumentFormat::TarXz => extract_tar(
            budgeted(XzDecoder::new_multi_decoder(file)),
            staging,
            archive_path,
        ),
        DocumentFormat::TarZstd => {
            let decoder = zstd::stream::read::Decoder::new(file).map_err(|error| {
                AppError::DataParsing(format!(
                    "ARCHIVE_DECOMPRESSION_FAILED: failed to open '{}': {error}",
                    archive_path.display()
                ))
            })?;
            extract_tar(budgeted(decoder), staging, archive_path)
        }
        DocumentFormat::Gzip => {
            extract_single_stream(MultiGzDecoder::new(file), staging, archive_path, &[".gz"])
        }
        DocumentFormat::Bzip2 => {
            extract_single_stream(MultiBzDecoder::new(file), staging, archive_path, &[".bz2"])
        }
        DocumentFormat::Xz => extract_single_stream(
            XzDecoder::new_multi_decoder(file),
            staging,
            archive_path,
            &[".xz"],
        ),
        DocumentFormat::Zstd => {
            let decoder = zstd::stream::read::Decoder::new(file).map_err(|error| {
                AppError::DataParsing(format!(
                    "ARCHIVE_DECOMPRESSION_FAILED: failed to open '{}': {error}",
                    archive_path.display()
                ))
            })?;
            extract_single_stream(decoder, staging, archive_path, &[".zst", ".zstd"])
        }
        // FEATURE(archive): Add an explicit credential-vault workflow for encrypted containers
        // so password-protected evidence can be retried without placing secrets in task payloads.
        _ => Err(AppError::DataParsing(format!(
            "ARCHIVE_FORMAT_UNSUPPORTED: {:?} requires a specialist extraction tool",
            format
        ))),
    }
}

fn extract_tar<R: Read>(
    reader: R,
    staging: &Path,
    archive_path: &Path,
) -> Result<Vec<PathBuf>, AppError> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive.entries().map_err(|error| {
        AppError::DataParsing(format!(
            "TAR_ARCHIVE_INVALID: failed to read '{}': {error}",
            archive_path.display()
        ))
    })?;
    let mut files = Vec::new();
    let mut entry_count = 0_usize;
    let mut total_bytes = 0_u64;
    for entry in entries {
        entry_count = entry_count.checked_add(1).ok_or_else(|| {
            AppError::DataParsing("TAR_ENTRY_COUNT_OVERFLOW: entry count overflow".to_string())
        })?;
        if entry_count > MAX_ENTRY_COUNT {
            return Err(AppError::DataParsing(format!(
                "TAR_ENTRY_COUNT_EXCEEDED: archive exceeds {MAX_ENTRY_COUNT} entries"
            )));
        }
        let mut entry = entry.map_err(|error| {
            AppError::DataParsing(format!(
                "TAR_ENTRY_READ_FAILED: failed to read '{}': {error}",
                archive_path.display()
            ))
        })?;
        let relative_path = validated_relative_path(&entry.path().map_err(|error| {
            AppError::DataParsing(format!(
                "TAR_ENTRY_PATH_INVALID: failed to decode a path in '{}': {error}",
                archive_path.display()
            ))
        })?)?;
        let entry_type = entry.header().entry_type();
        let output_path = staging.join(&relative_path);
        // Account for the declared size before any branch that skips the entry. A directory entry
        // is normally zero-length, but tar advances past a non-zero one by reading and discarding
        // that many bytes from the decompressor -- so a single directory declaring 8 GiB burned
        // minutes of CPU expanding a 1 MB archive with no cap ever firing.
        validate_expanded_size(entry.size(), &mut total_bytes, &relative_path)?;
        if entry_type.is_dir() {
            fs::create_dir_all(&output_path).map_err(|error| {
                AppError::Internal(format!(
                    "TAR_DIRECTORY_CREATE_FAILED: failed to create '{}': {error}",
                    output_path.display()
                ))
            })?;
            continue;
        }
        if !entry_type.is_file() {
            return Err(AppError::DataParsing(format!(
                "TAR_ENTRY_TYPE_REJECTED: '{}' is not a regular file or directory",
                relative_path.display()
            )));
        }
        let entry_size = entry.size();
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                AppError::Internal(format!(
                    "TAR_DIRECTORY_CREATE_FAILED: failed to create '{}': {error}",
                    parent.display()
                ))
            })?;
        }
        let mut output = create_output(&output_path, "TAR_ENTRY_CREATE_FAILED")?;
        copy_exact_bounded(&mut entry, &mut output, entry_size, &relative_path)?;
        sync_output(&mut output, &output_path, "TAR_ENTRY_SYNC_FAILED")?;
        files.push(relative_path);
    }
    Ok(files)
}

fn extract_single_stream<R: Read>(
    mut reader: R,
    staging: &Path,
    archive_path: &Path,
    suffixes: &[&str],
) -> Result<Vec<PathBuf>, AppError> {
    let relative_path = decompressed_file_name(archive_path, suffixes);
    let output_path = staging.join(&relative_path);
    let mut output = create_output(&output_path, "ARCHIVE_OUTPUT_CREATE_FAILED")?;
    let copied =
        io::copy(&mut reader.by_ref().take(MAX_ENTRY_BYTES + 1), &mut output).map_err(|error| {
            AppError::DataParsing(format!(
                "ARCHIVE_DECOMPRESSION_FAILED: failed to expand '{}': {error}",
                archive_path.display()
            ))
        })?;
    if copied > MAX_ENTRY_BYTES {
        return Err(AppError::DataParsing(format!(
            "ARCHIVE_ENTRY_SIZE_EXCEEDED: '{}' expands beyond {MAX_ENTRY_BYTES} bytes",
            archive_path.display()
        )));
    }
    sync_output(&mut output, &output_path, "ARCHIVE_OUTPUT_SYNC_FAILED")?;
    Ok(vec![relative_path])
}

fn validated_relative_path(path: &Path) -> Result<PathBuf, AppError> {
    let mut relative = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => relative.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::DataParsing(format!(
                    "ARCHIVE_ENTRY_PATH_INVALID: '{}' escapes the extraction directory",
                    path.display()
                )));
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(AppError::DataParsing(
            "ARCHIVE_ENTRY_PATH_INVALID: archive entry path is empty".to_string(),
        ));
    }
    Ok(relative)
}

/// Caps the total number of bytes an archive reader may yield.
///
/// `extract_tar`'s own limits all live in its loop body, which is too late for two things the tar
/// crate does inside its iterator: it buffers GNU longname/longlink and PAX header members with an
/// unbounded `read_all`, and it advances past a skipped entry by reading and discarding that
/// entry's declared size. Both happen before any per-entry check can run, so the only place a
/// bound can be enforced is the byte stream itself.
struct BudgetedReader<R> {
    inner: R,
    remaining: u64,
}

impl<R: Read> Read for BudgetedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "ARCHIVE_TOTAL_SIZE_EXCEEDED: expanded data exceeds \
                     {MAX_TOTAL_UNCOMPRESSED_BYTES} bytes"
                ),
            ));
        }
        let ceiling = usize::try_from(self.remaining).unwrap_or(usize::MAX);
        let limit = buffer.len().min(ceiling);
        let read = self.inner.read(&mut buffer[..limit])?;
        self.remaining = self.remaining.saturating_sub(read as u64);
        Ok(read)
    }
}

/// Wraps a decompressed archive stream in the global expansion budget.
fn budgeted<R: Read>(inner: R) -> BudgetedReader<R> {
    // The budget counts raw stream bytes while `validate_expanded_size` counts entry payload bytes,
    // and a tar stream also carries 512-byte headers plus block padding. Without headroom this
    // reader would trip slightly before the per-entry accounting on an archive that is legitimately
    // at the limit, attributing a valid rejection to the wrong check.
    budgeted_with(
        inner,
        MAX_TOTAL_UNCOMPRESSED_BYTES.saturating_add(TAR_METADATA_HEADROOM_BYTES),
    )
}

/// `budgeted` with an explicit ceiling, so the bound can be exercised without moving gigabytes.
fn budgeted_with<R: Read>(inner: R, remaining: u64) -> BudgetedReader<R> {
    BudgetedReader { inner, remaining }
}

fn validate_expanded_size(
    entry_size: u64,
    total_bytes: &mut u64,
    relative_path: &Path,
) -> Result<(), AppError> {
    if entry_size > MAX_ENTRY_BYTES {
        return Err(AppError::DataParsing(format!(
            "ARCHIVE_ENTRY_SIZE_EXCEEDED: '{}' exceeds {MAX_ENTRY_BYTES} bytes",
            relative_path.display()
        )));
    }
    *total_bytes = total_bytes.checked_add(entry_size).ok_or_else(|| {
        AppError::DataParsing("ARCHIVE_TOTAL_SIZE_OVERFLOW: expanded size overflow".to_string())
    })?;
    if *total_bytes > MAX_TOTAL_UNCOMPRESSED_BYTES {
        return Err(AppError::DataParsing(format!(
            "ARCHIVE_TOTAL_SIZE_EXCEEDED: expanded data exceeds {MAX_TOTAL_UNCOMPRESSED_BYTES} bytes"
        )));
    }
    Ok(())
}

fn copy_exact_bounded<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    expected_size: u64,
    relative_path: &Path,
) -> Result<(), AppError> {
    let copied = io::copy(&mut reader.take(expected_size + 1), writer).map_err(|error| {
        AppError::DataParsing(format!(
            "ARCHIVE_ENTRY_EXTRACT_FAILED: failed to extract '{}': {error}",
            relative_path.display()
        ))
    })?;
    if copied != expected_size {
        return Err(AppError::DataParsing(format!(
            "ARCHIVE_ENTRY_SIZE_MISMATCH: '{}' declared {expected_size} bytes but produced {copied}",
            relative_path.display()
        )));
    }
    Ok(())
}

fn create_output(path: &Path, label: &str) -> Result<File, AppError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            AppError::Internal(format!(
                "{label}: failed to create '{}': {error}",
                path.display()
            ))
        })
}

fn sync_output(output: &mut File, path: &Path, label: &str) -> Result<(), AppError> {
    output.flush().map_err(|error| {
        AppError::Internal(format!(
            "{label}: failed to flush '{}': {error}",
            path.display()
        ))
    })?;
    output.sync_all().map_err(|error| {
        AppError::Internal(format!(
            "{label}: failed to sync '{}': {error}",
            path.display()
        ))
    })
}

fn decompressed_file_name(archive_path: &Path, suffixes: &[&str]) -> PathBuf {
    let name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("payload");
    let lower = name.to_ascii_lowercase();
    let stripped = suffixes
        .iter()
        .find_map(|suffix| {
            lower
                .strip_suffix(suffix)
                .map(|prefix| &name[..prefix.len()])
        })
        .filter(|value| !value.is_empty())
        .unwrap_or("payload");
    let sanitized = stripped
        .chars()
        .filter_map(|character| {
            if character.is_control() {
                None
            } else if matches!(character, '/' | '\\' | ':' | '\0') {
                Some('_')
            } else {
                Some(character)
            }
        })
        .take(180)
        .collect::<String>();
    let sanitized = sanitized.trim().trim_matches('.');
    PathBuf::from(if sanitized.is_empty() {
        "payload"
    } else {
        sanitized
    })
}

#[cfg(test)]
mod expansion_budget_tests {
    use super::*;

    /// The stream budget is the only bound that can apply to bytes the tar crate consumes inside
    /// its own iterator: GNU longname and PAX header members are buffered with an unbounded
    /// `read_all`, and a skipped entry is passed over by reading and discarding its declared size.
    /// Both happen before `extract_tar`'s loop body, where every other limit lives.
    #[test]
    fn the_stream_budget_stops_an_endless_reader() {
        let mut reader = budgeted_with(io::repeat(0), 4_096);
        let mut sink = Vec::new();
        let error = io::copy(&mut reader, &mut sink)
            .expect_err("an endless stream must be cut off at the budget");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(
            error.to_string().contains("ARCHIVE_TOTAL_SIZE_EXCEEDED"),
            "unexpected error: {error}"
        );
        assert_eq!(sink.len(), 4_096, "exactly the budget is yielded, no more");
    }

    /// Builds a tar holding a single directory entry that declares `declared_size` bytes.
    fn tar_with_oversized_directory(path: &Path, declared_size: u64) {
        let mut builder = tar::Builder::new(File::create(path).unwrap());
        let mut header = tar::Header::new_gnu();
        header.set_path("payload/").unwrap();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(declared_size);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append(&header, io::empty()).unwrap();
        builder.into_inner().unwrap();
    }

    /// A directory entry used to reach `continue` before any size accounting ran. Because tar
    /// advances past a skipped entry by reading and discarding its declared size, one directory
    /// claiming 8 GiB expanded a ~1 MB archive into gigabytes of decompression with no cap firing.
    #[test]
    fn a_directory_entry_cannot_declare_a_size_past_the_limit() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("bomb.tar");
        tar_with_oversized_directory(&archive, 8 * 1024 * 1024 * 1024);

        assert!(
            fs::metadata(&archive).unwrap().len() < 16_384,
            "the archive itself is tiny; only the declared size is large"
        );

        let staging = directory.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        let error = extract_tar(budgeted(File::open(&archive).unwrap()), &staging, &archive)
            .expect_err("an oversized directory entry must be rejected");
        assert!(
            error.to_string().contains("ARCHIVE_ENTRY_SIZE_EXCEEDED"),
            "unexpected error: {error}"
        );
    }

    /// A directory entry declaring nothing is ordinary and must still extract.
    #[test]
    fn an_ordinary_directory_entry_still_extracts() {
        let directory = tempfile::tempdir().unwrap();
        let archive = directory.path().join("plain.tar");
        tar_with_oversized_directory(&archive, 0);
        let staging = directory.path().join("staging");
        fs::create_dir_all(&staging).unwrap();

        let files = extract_tar(budgeted(File::open(&archive).unwrap()), &staging, &archive)
            .expect("a zero-size directory entry is normal");
        assert!(files.is_empty(), "a directory yields no files");
        assert!(staging.join("payload").is_dir(), "the directory is created");
    }
}
