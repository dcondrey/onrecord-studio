use std::fs::{File, Metadata};
use std::io::Read;
use std::path::Path;

use crate::error::AppError;

pub(crate) fn inspect_regular_file(
    path: &Path,
    format: &'static str,
    maximum_bytes: u64,
) -> Result<Metadata, AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        AppError::Storage(format!(
            "{format}_INSPECTION_FAILED: failed to inspect '{}': {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::DataParsing(format!(
            "{format}_SOURCE_INVALID: '{}' must be a regular, non-symlink file",
            path.display()
        )));
    }
    if metadata.len() == 0 || metadata.len() > maximum_bytes {
        return Err(AppError::DataParsing(format!(
            "{format}_SIZE_INVALID: '{}' must contain between 1 and {maximum_bytes} bytes",
            path.display()
        )));
    }
    Ok(metadata)
}

pub(crate) fn read_regular_file(
    path: &Path,
    format: &'static str,
    maximum_bytes: u64,
) -> Result<Vec<u8>, AppError> {
    let inspected = inspect_regular_file(path, format, maximum_bytes)?;
    let expected_size = usize::try_from(inspected.len()).map_err(|_| {
        AppError::DataParsing(format!(
            "{format}_SIZE_UNSUPPORTED: '{}' cannot be represented on this platform",
            path.display()
        ))
    })?;
    // The caller has already confined the path to the managed evidence store. Re-stat the opened
    // descriptor and verify the exact byte count so external/untrusted inputs cannot substitute a
    // symlink or change size between inspection and the bounded snapshot read. A same-size rewrite
    // would require local write access to the managed evidence store, outside this trust boundary.
    let file = File::open(path).map_err(|error| {
        AppError::Storage(format!(
            "{format}_READ_FAILED: failed to open '{}': {error}",
            path.display()
        ))
    })?;
    let opened = file.metadata().map_err(|error| {
        AppError::Storage(format!(
            "{format}_INSPECTION_FAILED: failed to inspect opened source '{}': {error}",
            path.display()
        ))
    })?;
    if !opened.is_file() || opened.len() != inspected.len() {
        return Err(changed_while_reading(path, format));
    }

    let mut bytes = Vec::with_capacity(expected_size);
    file.take(maximum_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            AppError::Storage(format!(
                "{format}_READ_FAILED: failed to read '{}': {error}",
                path.display()
            ))
        })?;
    if bytes.len() != expected_size {
        return Err(changed_while_reading(path, format));
    }
    Ok(bytes)
}

fn changed_while_reading(path: &Path, format: &'static str) -> AppError {
    AppError::DataParsing(format!(
        "{format}_SOURCE_CHANGED: '{}' changed while its snapshot was being read",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::read_regular_file;

    #[test]
    fn reads_an_exact_bounded_snapshot() {
        let mut source = NamedTempFile::new().unwrap();
        source.write_all(b"forensic snapshot").unwrap();

        let bytes = read_regular_file(source.path(), "TEST", 64).unwrap();

        assert_eq!(bytes, b"forensic snapshot");
    }

    #[test]
    fn rejects_empty_and_oversized_sources() {
        let mut oversized = NamedTempFile::new().unwrap();
        oversized.write_all(b"12345").unwrap();
        let empty = NamedTempFile::new().unwrap();

        assert!(read_regular_file(empty.path(), "TEST", 5).is_err());
        assert!(read_regular_file(oversized.path(), "TEST", 4).is_err());
    }
}
