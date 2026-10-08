//! Crash-safe replacement of one small file, shared by the JSON stores in the app data directory.

use crate::error::AppError;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

/// Reads a whole file of at most `maximum_bytes`. A missing file is `Ok(None)`; a larger one is an
/// error of kind `FileTooLarge`. The error stays an `io::Error` so each store words its own
/// remediation.
pub(crate) fn read_bounded(path: &Path, maximum_bytes: u64) -> io::Result<Option<Vec<u8>>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(maximum_bytes.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum_bytes {
        return Err(io::Error::from(io::ErrorKind::FileTooLarge));
    }
    Ok(Some(bytes))
}

pub(crate) fn write_atomic(path: &Path, label: &str, bytes: &[u8]) -> Result<(), AppError> {
    write_atomic_with(path, label, |file| file.write_all(bytes))
}

/// Writes through a temp file in the same directory, fsyncs it, then renames over the target.
/// `write` is a seam so a test can fail mid-write; the target is untouched on any error.
/// `label` names the document in error text ("the local profile").
pub(crate) fn write_atomic_with<F>(path: &Path, label: &str, write: F) -> Result<(), AppError>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    let directory = path
        .parent()
        .ok_or_else(|| AppError::Storage(format!("The path of {label} has no directory")))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Storage(format!("The path of {label} has no file name")))?
        .to_string_lossy();
    fs::create_dir_all(directory).map_err(|error| {
        AppError::Storage(format!(
            "Failed to create the directory for {label}: {error}"
        ))
    })?;
    let temp = directory.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let outcome = (|| -> io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&temp)?;
        write(&mut file)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    match outcome {
        Ok(()) => {
            sync_directory(path);
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(AppError::Storage(format!(
                "Failed to write {label}: {error}"
            )))
        }
    }
}

/// Best effort: a rename that is not yet durable is still atomic, so a failure here is not an
/// error the user can act on.
pub(crate) fn sync_directory(path: &Path) {
    #[cfg(unix)]
    if let Some(directory) = path.parent() {
        if let Ok(directory) = File::open(directory) {
            let _ = directory.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_write_leaves_the_previous_file_intact_and_no_temp_file_behind() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.json");
        write_atomic(&path, "the document", b"previous").unwrap();

        let failure = write_atomic_with(&path, "the document", |file| {
            file.write_all(b"half of the new")?;
            Err(io::Error::other("disk full"))
        });
        assert!(failure.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"previous");
        let names: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from("document.json")]);
    }

    #[test]
    fn read_bounded_admits_exactly_the_maximum_and_refuses_one_byte_more() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.json");
        assert_eq!(read_bounded(&path, 4).unwrap(), None);

        fs::write(&path, b"1234").unwrap();
        assert_eq!(
            read_bounded(&path, 4).unwrap().as_deref(),
            Some(&b"1234"[..])
        );

        fs::write(&path, b"12345").unwrap();
        let error = read_bounded(&path, 4).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
    }
}
