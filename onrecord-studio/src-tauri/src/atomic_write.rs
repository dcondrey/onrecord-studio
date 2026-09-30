//! Crash-safe replacement of one small file, shared by the JSON stores in the app data directory.

use crate::error::AppError;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

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
