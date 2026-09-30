//! The profile file: one small JSON document in the app data directory, written atomically.
//!
//! This is deliberately not in DuckDB. The profile is a per-device preference that must be
//! deletable as a single file, and it must survive a corpus or core database rebuild.

use crate::atomic_write::sync_directory;
use crate::error::AppError;
use crate::types::profile::{ProfileDeletionOutcome, UserProfile, PROFILE_SCHEMA_VERSION};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const PROFILE_FILE_NAME: &str = "profile.json";
const PROFILE_LABEL: &str = "the local profile";
/// The largest document the limits in `types::profile` can produce is well under this; a bigger
/// file was not written by this app and is refused before it is parsed.
pub const MAXIMUM_PROFILE_FILE_BYTES: u64 = 256 * 1024;

pub struct ProfileStore {
    path: PathBuf,
    // Serializes read-modify-write so two commands cannot each read the old document and the
    // second write silently discard the first.
    gate: Mutex<()>,
}

impl ProfileStore {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join(PROFILE_FILE_NAME),
            gate: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `Ok(None)` is the unset state. A file that exists but cannot be read is an error, never
    /// silently "unset": showing an unreadable profile as absent would then overwrite it on save.
    pub fn load(&self) -> Result<Option<UserProfile>, AppError> {
        let _guard = self.lock()?;
        read_profile(&self.path)
    }

    /// Reads the current document (or `None`), lets `change` produce the next one, and writes it
    /// atomically. An error from `change` writes nothing.
    pub fn update<F>(&self, change: F) -> Result<UserProfile, AppError>
    where
        F: FnOnce(Option<UserProfile>) -> Result<UserProfile, AppError>,
    {
        let _guard = self.lock()?;
        let current = read_profile(&self.path)?;
        let next = change(current)?;
        let bytes = serde_json::to_vec_pretty(&next)
            .map_err(|error| AppError::Storage(format!("Failed to encode the profile: {error}")))?;
        write_atomic(&self.path, &bytes)?;
        Ok(next)
    }

    /// Removes the file. Also removes an unreadable one: deleting is how a user recovers from it.
    pub fn delete(&self) -> Result<ProfileDeletionOutcome, AppError> {
        let _guard = self.lock()?;
        match fs::remove_file(&self.path) {
            Ok(()) => {
                sync_directory(&self.path);
                Ok(ProfileDeletionOutcome::Removed)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(ProfileDeletionOutcome::NothingToRemove)
            }
            Err(error) => Err(AppError::Storage(format!(
                "Failed to delete the local profile: {error}"
            ))),
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, AppError> {
        self.gate
            .lock()
            .map_err(|_| AppError::Internal("The profile store lock was poisoned".to_string()))
    }
}

fn read_profile(path: &Path) -> Result<Option<UserProfile>, AppError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AppError::Storage(format!(
                "Failed to open the local profile: {error}"
            )))
        }
    };
    let mut bytes = Vec::new();
    file.take(MAXIMUM_PROFILE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::Storage(format!("Failed to read the local profile: {error}")))?;
    if bytes.len() as u64 > MAXIMUM_PROFILE_FILE_BYTES {
        return Err(AppError::Storage(format!(
            "The local profile file is larger than {MAXIMUM_PROFILE_FILE_BYTES} bytes and was not \
             read. Delete the local profile to start again."
        )));
    }
    let profile: UserProfile = serde_json::from_slice(&bytes).map_err(|error| {
        AppError::Storage(format!(
            "The local profile file is not valid ({error}). Delete the local profile to start again."
        ))
    })?;
    if profile.schema_version != PROFILE_SCHEMA_VERSION {
        return Err(AppError::Storage(format!(
            "The local profile has schema version {}, this build reads {PROFILE_SCHEMA_VERSION}. \
             It was not modified.",
            profile.schema_version
        )));
    }
    Ok(Some(profile))
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    crate::atomic_write::write_atomic(path, PROFILE_LABEL, bytes)
}

/// Test seam: lets a test fail a write partway.
#[cfg(test)]
pub(crate) fn write_atomic_with<F>(path: &Path, write: F) -> Result<(), AppError>
where
    F: FnOnce(&mut File) -> io::Result<()>,
{
    crate::atomic_write::write_atomic_with(path, PROFILE_LABEL, write)
}
