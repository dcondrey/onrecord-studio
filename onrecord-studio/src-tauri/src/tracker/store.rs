//! The tracker file: one JSON document in the app data directory, written atomically.
//!
//! Not in DuckDB, for the same reasons as the profile: it is a per-device notebook that must be
//! deletable as one file and must survive a corpus or core database rebuild. Nothing that produces
//! findings reads it.

use super::validate_document;
use crate::atomic_write::{read_bounded, write_atomic};
use crate::error::AppError;
use crate::types::tracker::{TrackerDocument, TRACKER_SCHEMA_VERSION};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const TRACKER_FILE_NAME: &str = "tracker.json";
const TRACKER_LABEL: &str = "the request tracker";
/// Reads are bounded by this and `update` refuses any write past it, so the store cannot produce
/// a file it would then refuse to read. Realistic use is kilobytes; a tracker filled with
/// maximum-length evidence on every entry reaches it and is told to make room, never truncated.
pub const MAXIMUM_TRACKER_FILE_BYTES: u64 = 64 * 1024 * 1024;

pub struct TrackerStore {
    path: PathBuf,
    maximum_bytes: u64,
    // Serializes read-modify-write so two commands cannot each read the old document and the
    // second write silently discard the first.
    gate: Mutex<()>,
}

impl TrackerStore {
    pub fn new(directory: &Path) -> Self {
        Self::with_maximum_bytes(directory, MAXIMUM_TRACKER_FILE_BYTES)
    }

    pub(crate) fn with_maximum_bytes(directory: &Path, maximum_bytes: u64) -> Self {
        Self {
            path: directory.join(TRACKER_FILE_NAME),
            maximum_bytes,
            gate: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A missing file is an empty tracker. A file that exists but cannot be read is an error,
    /// never silently "empty": showing it as empty would overwrite it on the next save.
    pub fn load(&self) -> Result<TrackerDocument, AppError> {
        let _guard = self.lock()?;
        self.read()
    }

    /// Reads the document, lets `change` edit it and produce a result, then writes it atomically.
    /// An error from `change` writes nothing, and so does a document that would not fit the size
    /// limit: the store never writes a file it would refuse to read.
    pub fn update<T, F>(&self, change: F) -> Result<T, AppError>
    where
        F: FnOnce(&mut TrackerDocument) -> Result<T, AppError>,
    {
        let _guard = self.lock()?;
        let mut document = self.read()?;
        let result = change(&mut document)?;
        validate_document(&document)?;
        let bytes = serde_json::to_vec(&document).map_err(|error| {
            AppError::Storage(format!("Failed to encode {TRACKER_LABEL}: {error}"))
        })?;
        if bytes.len() as u64 > self.maximum_bytes {
            return Err(AppError::Storage(format!(
                "The tracker would exceed {} bytes; remove a request to make room. Nothing was saved.",
                self.maximum_bytes
            )));
        }
        write_atomic(&self.path, TRACKER_LABEL, &bytes)?;
        Ok(result)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, AppError> {
        self.gate
            .lock()
            .map_err(|_| AppError::Internal("The tracker store lock was poisoned".to_string()))
    }

    fn read(&self) -> Result<TrackerDocument, AppError> {
        let bytes = match read_bounded(&self.path, self.maximum_bytes) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return Ok(TrackerDocument::default()),
            Err(error) if error.kind() == io::ErrorKind::FileTooLarge => {
                return Err(AppError::Storage(format!(
                    "The tracker file is larger than {} bytes and was not read. It was not modified.",
                    self.maximum_bytes
                )))
            }
            Err(error) => {
                return Err(AppError::Storage(format!(
                    "Failed to read {TRACKER_LABEL}: {error}"
                )))
            }
        };
        let document: TrackerDocument = serde_json::from_slice(&bytes).map_err(|error| {
            AppError::Storage(format!(
                "The tracker file is not valid ({error}). It was not modified."
            ))
        })?;
        if document.schema_version != TRACKER_SCHEMA_VERSION {
            return Err(AppError::Storage(format!(
                "The tracker has schema version {}, this build reads {TRACKER_SCHEMA_VERSION}. \
                 It was not modified.",
                document.schema_version
            )));
        }
        validate_document(&document).map_err(|error| {
            AppError::Storage(format!(
                "The tracker file broke a limit ({error}). It was not modified."
            ))
        })?;
        Ok(document)
    }
}
