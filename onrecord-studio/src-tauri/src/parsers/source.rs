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
    #[cfg(windows)]
    let inspected_identity = windows_identity::of_path(path, format)?;
    let expected_size = usize::try_from(inspected.len()).map_err(|_| {
        AppError::DataParsing(format!(
            "{format}_SIZE_UNSUPPORTED: '{}' cannot be represented on this platform",
            path.display()
        ))
    })?;
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
    if !opened.is_file() || !same_file_snapshot(&inspected, &opened) {
        return Err(changed_while_reading(path, format));
    }
    #[cfg(windows)]
    if windows_identity::of_file(&file, path, format)? != inspected_identity {
        return Err(changed_while_reading(path, format));
    }

    let mut bytes = Vec::with_capacity(expected_size);
    (&file)
        .take(maximum_bytes + 1)
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
    let after_read = file.metadata().map_err(|error| {
        AppError::Storage(format!(
            "{format}_INSPECTION_FAILED: failed to re-inspect opened source '{}': {error}",
            path.display()
        ))
    })?;
    if !same_file_snapshot(&opened, &after_read) {
        return Err(changed_while_reading(path, format));
    }
    Ok(bytes)
}

/// Opens `path` for a read that may outlast this call, refusing what [`inspect_regular_file`]
/// refuses. Pass the returned metadata to [`ensure_unchanged`] once the read is done.
pub(crate) fn open_regular_file(
    path: &Path,
    format: &'static str,
    maximum_bytes: u64,
) -> Result<(File, Metadata), AppError> {
    let inspected = inspect_regular_file(path, format, maximum_bytes)?;
    #[cfg(windows)]
    let inspected_identity = windows_identity::of_path(path, format)?;
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
    if !opened.is_file() || !same_file_snapshot(&inspected, &opened) {
        return Err(changed_while_reading(path, format));
    }
    #[cfg(windows)]
    if windows_identity::of_file(&file, path, format)? != inspected_identity {
        return Err(changed_while_reading(path, format));
    }
    Ok((file, opened))
}

pub(crate) fn ensure_unchanged(
    file: &File,
    opened: &Metadata,
    path: &Path,
    format: &'static str,
) -> Result<(), AppError> {
    let now = file.metadata().map_err(|error| {
        AppError::Storage(format!(
            "{format}_INSPECTION_FAILED: failed to re-inspect opened source '{}': {error}",
            path.display()
        ))
    })?;
    if same_file_snapshot(opened, &now) {
        Ok(())
    } else {
        Err(changed_while_reading(path, format))
    }
}

#[cfg(unix)]
fn same_file_snapshot(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}

/// Without the file's identity, which stable `std` does not expose on Windows: there
/// [`windows_identity`] checks that the opened handle is the file that was inspected.
#[cfg(not(unix))]
fn same_file_snapshot(left: &Metadata, right: &Metadata) -> bool {
    left.len() == right.len()
        && left.modified().ok() == right.modified().ok()
        && left.created().ok() == right.created().ok()
}

/// Volume serial and file index, the Windows counterpart of the `dev`/`ino` pair above.
#[cfg(windows)]
mod windows_identity {
    use std::fs::{File, OpenOptions};
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    use same_file::Handle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };

    use crate::error::AppError;

    /// The file `path` names itself, not a link target: opened without following reparse points
    /// and without read access, so it neither follows nor blocks anything.
    pub(super) fn of_path(path: &Path, format: &'static str) -> Result<Handle, AppError> {
        OpenOptions::new()
            .access_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .and_then(Handle::from_file)
            .map_err(|error| failed(path, format, error))
    }

    pub(super) fn of_file(
        file: &File,
        path: &Path,
        format: &'static str,
    ) -> Result<Handle, AppError> {
        file.try_clone()
            .and_then(Handle::from_file)
            .map_err(|error| failed(path, format, error))
    }

    fn failed(path: &Path, format: &'static str, error: std::io::Error) -> AppError {
        AppError::Storage(format!(
            "{format}_INSPECTION_FAILED: failed to identify '{}': {error}",
            path.display()
        ))
    }
}

fn changed_while_reading(path: &Path, format: &'static str) -> AppError {
    AppError::DataParsing(format!(
        "{format}_SOURCE_CHANGED: '{}' changed while its snapshot was being read",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
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

    #[test]
    fn stable_identity_distinguishes_same_size_files() {
        let mut first = NamedTempFile::new().unwrap();
        let mut second = NamedTempFile::new().unwrap();
        first.write_all(b"same").unwrap();
        second.write_all(b"size").unwrap();
        let first_metadata = fs::metadata(first.path()).unwrap();
        let second_metadata = fs::metadata(second.path()).unwrap();
        // On Windows the snapshot carries no identity (two files created in one clock tick match);
        // the identity check made beside it is what tells them apart there.
        #[cfg(unix)]
        assert!(!super::same_file_snapshot(
            &first_metadata,
            &second_metadata
        ));
        #[cfg(windows)]
        {
            let _ = (first_metadata, second_metadata);
            assert!(
                super::windows_identity::of_path(first.path(), "TEST").unwrap()
                    != super::windows_identity::of_path(second.path(), "TEST").unwrap()
            );
        }
    }
}
