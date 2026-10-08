//! Serverless collaboration: which signing keys the user has verified with their owners, and a
//! plain folder as the transport for signed case packets.
//!
//! A packet signature proves origin and integrity of the file, never who holds the key. The
//! trust list records only that the user confirmed a key out of band; it changes no verification
//! result. The folder is whatever the user points at (a synced or network folder, a USB stick):
//! nothing here talks to a network.

use crate::atomic_write::write_atomic;
use crate::error::AppError;
use crate::integrity::is_sha256_hex;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const CONTRIBUTOR_FILE_NAME: &str = "contributors.json";
pub const MAXIMUM_TRUSTED_CONTRIBUTORS: usize = 500;
pub const MAXIMUM_LABEL_CHARACTERS: usize = 120;
pub const MAXIMUM_FOLDER_ENTRIES: usize = 500;
/// Most packet files read from a folder before listing stops: a resource guard, reported as truncation.
pub const MAXIMUM_FOLDER_SCAN: usize = 50_000;
/// Matches the transport limit enforced when a packet is previewed or imported.
pub const MAXIMUM_PACKET_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAXIMUM_FILE_NAME_CHARACTERS: usize = 120;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::DataParsing(format!("COLLABORATION_INVALID: {}", message.into()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrustedContributor {
    pub key_id: String,
    pub label: String,
    pub trusted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ContributorDocument {
    contributors: Vec<TrustedContributor>,
}

/// A signer key id is `sha256(public key)`: exactly 64 lowercase hexadecimal characters.
pub fn validate_key_id(key_id: &str) -> Result<String, AppError> {
    let key_id = key_id.trim().to_ascii_lowercase();
    if !is_sha256_hex(&key_id) {
        return Err(invalid("a key id is 64 hexadecimal characters"));
    }
    Ok(key_id)
}

pub struct ContributorStore {
    path: PathBuf,
    gate: Mutex<()>,
}

impl ContributorStore {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join(CONTRIBUTOR_FILE_NAME),
            gate: Mutex::new(()),
        }
    }

    pub fn list(&self) -> Result<Vec<TrustedContributor>, AppError> {
        let _guard = self.lock()?;
        Ok(self.read()?.contributors)
    }

    /// Re-trusting a key replaces its label; the list never holds one key twice.
    pub fn trust(&self, key_id: &str, label: &str) -> Result<TrustedContributor, AppError> {
        let key_id = validate_key_id(key_id)?;
        let label = label.trim();
        if label.is_empty() {
            return Err(invalid("say who this key belongs to"));
        }
        if label.chars().any(char::is_control) || label.chars().count() > MAXIMUM_LABEL_CHARACTERS {
            return Err(invalid(format!(
                "the label must be one line of at most {MAXIMUM_LABEL_CHARACTERS} characters"
            )));
        }
        let _guard = self.lock()?;
        let mut document = self.read()?;
        document
            .contributors
            .retain(|contributor| contributor.key_id != key_id);
        if document.contributors.len() >= MAXIMUM_TRUSTED_CONTRIBUTORS {
            return Err(invalid(format!(
                "at most {MAXIMUM_TRUSTED_CONTRIBUTORS} contributors are kept; remove one first"
            )));
        }
        let entry = TrustedContributor {
            key_id,
            label: label.to_string(),
            trusted_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        };
        document.contributors.push(entry.clone());
        self.write(&document)?;
        Ok(entry)
    }

    pub fn untrust(&self, key_id: &str) -> Result<(), AppError> {
        let key_id = validate_key_id(key_id)?;
        let _guard = self.lock()?;
        let mut document = self.read()?;
        let before = document.contributors.len();
        document
            .contributors
            .retain(|contributor| contributor.key_id != key_id);
        if document.contributors.len() == before {
            return Err(invalid("that key is not on the list"));
        }
        self.write(&document)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, AppError> {
        self.gate
            .lock()
            .map_err(|_| AppError::Internal("The contributor list lock was poisoned".to_string()))
    }

    /// A missing file is an empty list. An unreadable one is an error, never "empty": a trust
    /// list that silently reads as empty would drop every verified contributor on the next save.
    fn read(&self) -> Result<ContributorDocument, AppError> {
        match fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                AppError::Storage(format!(
                    "The contributor list is not valid ({error}). It was not modified."
                ))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
            Err(error) => Err(AppError::Storage(format!(
                "Failed to read the contributor list: {error}"
            ))),
        }
    }

    fn write(&self, document: &ContributorDocument) -> Result<(), AppError> {
        let bytes = serde_json::to_vec(document).map_err(|error| {
            AppError::Storage(format!("Failed to encode the contributor list: {error}"))
        })?;
        write_atomic(&self.path, "the contributor list", &bytes)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolderPacket {
    pub file_name: String,
    pub bytes: u64,
    pub modified_at: Option<String>,
    /// Larger than the transport limit: listed so it is not silently missing, but not readable.
    pub oversized: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolderListing {
    pub packets: Vec<SharedFolderPacket>,
    /// More packets exist than are listed. The listed ones are the newest.
    pub truncated: bool,
}

/// One plain file name inside the shared folder: no separators, no traversal, a `.json` suffix.
pub fn validate_packet_file_name(name: &str) -> Result<(), AppError> {
    let allowed = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-');
    if name.is_empty()
        || name.len() > MAXIMUM_FILE_NAME_CHARACTERS
        || name.starts_with('.')
        || !name.ends_with(".json")
        || !name.bytes().all(allowed)
    {
        return Err(invalid(
            "a packet file name uses letters, digits, '.', '_' or '-' and ends in .json",
        ));
    }
    Ok(())
}

fn checked_folder(folder: &str) -> Result<PathBuf, AppError> {
    let path = PathBuf::from(folder);
    if !path.is_absolute() {
        return Err(invalid("choose the folder with the folder picker"));
    }
    let metadata = fs::metadata(&path)
        .map_err(|error| invalid(format!("the shared folder cannot be read: {error}")))?;
    if !metadata.is_dir() {
        return Err(invalid("the shared location is not a folder"));
    }
    Ok(path)
}

/// Regular `.json` files only. A symlink is skipped, not followed: the folder is shared with
/// other people, and a link must not be able to point a read at some other file on this computer.
pub fn list_packets(folder: &str) -> Result<SharedFolderListing, AppError> {
    let folder = checked_folder(folder)?;
    let entries = fs::read_dir(&folder)
        .map_err(|error| invalid(format!("the shared folder cannot be listed: {error}")))?;
    let mut packets = Vec::new();
    let mut scan_cut = false;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if validate_packet_file_name(&name).is_err() {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !metadata.file_type().is_file() {
            continue;
        }
        packets.push(SharedFolderPacket {
            file_name: name,
            bytes: metadata.len(),
            modified_at: metadata
                .modified()
                .ok()
                .map(|time| DateTime::<Utc>::from(time).to_rfc3339_opts(SecondsFormat::Secs, true)),
            oversized: metadata.len() > MAXIMUM_PACKET_FILE_BYTES,
        });
        if packets.len() >= MAXIMUM_FOLDER_SCAN {
            scan_cut = true;
            break;
        }
    }
    // Newest first, then cut: truncating in directory order would drop whichever files the
    // filesystem happens to list last, which can be the newest ones.
    packets.sort_by(|a, b| {
        b.modified_at
            .cmp(&a.modified_at)
            .then(a.file_name.cmp(&b.file_name))
    });
    let truncated = scan_cut || packets.len() > MAXIMUM_FOLDER_ENTRIES;
    packets.truncate(MAXIMUM_FOLDER_ENTRIES);
    Ok(SharedFolderListing { packets, truncated })
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file(_left: &fs::Metadata, _right: &fs::Metadata) -> bool {
    true
}

pub fn read_packet(folder: &str, file_name: &str) -> Result<String, AppError> {
    validate_packet_file_name(file_name)?;
    let path = checked_folder(folder)?.join(file_name);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| invalid(format!("that packet cannot be read: {error}")))?;
    if !metadata.file_type().is_file() {
        return Err(invalid("that entry is not a regular file"));
    }
    let file = File::open(&path)
        .map_err(|error| invalid(format!("that packet cannot be read: {error}")))?;
    // The folder is writable by others: refuse a file that was swapped for a link between the
    // check above and this open.
    let opened = file
        .metadata()
        .map_err(|error| invalid(format!("that packet cannot be read: {error}")))?;
    if !opened.is_file() || !same_file(&metadata, &opened) {
        return Err(invalid("that entry changed while it was being read"));
    }
    let mut text = String::new();
    file.take(MAXIMUM_PACKET_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|error| invalid(format!("that packet cannot be read: {error}")))?;
    if text.len() as u64 > MAXIMUM_PACKET_FILE_BYTES {
        return Err(invalid(format!(
            "that packet is larger than the {MAXIMUM_PACKET_FILE_BYTES}-byte transport limit"
        )));
    }
    Ok(text)
}

/// Writes a new file and refuses to replace one: a colleague's packet in the same folder must
/// never be overwritten by a name collision.
pub fn publish_packet(folder: &str, file_name: &str, payload: &str) -> Result<(), AppError> {
    validate_packet_file_name(file_name)?;
    if payload.len() as u64 > MAXIMUM_PACKET_FILE_BYTES {
        return Err(invalid(format!(
            "the packet is larger than the {MAXIMUM_PACKET_FILE_BYTES}-byte transport limit"
        )));
    }
    let path = checked_folder(folder)?.join(file_name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                invalid("a packet with that name is already in the folder")
            } else {
                invalid(format!("the packet could not be written there: {error}"))
            }
        })?;
    file.write_all(payload.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| invalid(format!("the packet could not be written there: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";

    #[test]
    fn a_key_is_trusted_once_and_a_bad_key_or_label_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let store = ContributorStore::new(directory.path());
        assert!(store.trust("not-a-key", "Maria").is_err());
        assert!(store.trust(KEY, "   ").is_err());
        assert!(store
            .trust(KEY, &"x".repeat(MAXIMUM_LABEL_CHARACTERS + 1))
            .is_err());
        store.trust(&KEY.to_uppercase(), "Maria").unwrap();
        store.trust(KEY, "Maria R.").unwrap();
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].label, "Maria R.");
        store.untrust(KEY).unwrap();
        assert!(store.untrust(KEY).is_err());
        std::fs::write(directory.path().join(CONTRIBUTOR_FILE_NAME), b"{ no").unwrap();
        assert!(store.list().is_err());
        assert!(store.trust(KEY, "Maria").is_err());
    }

    #[test]
    fn the_folder_lists_only_regular_json_files_and_refuses_unsafe_names() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().to_str().unwrap();
        std::fs::write(directory.path().join("case-1.json"), "{}").unwrap();
        std::fs::write(directory.path().join("notes.txt"), "x").unwrap();
        std::fs::create_dir(directory.path().join("sub.json")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", directory.path().join("link.json")).unwrap();
        let listed = list_packets(folder).unwrap().packets;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].file_name, "case-1.json");
        for bad in [
            "../x.json",
            "a/b.json",
            ".hidden.json",
            "x.txt",
            "",
            "sp ace.json",
        ] {
            assert!(read_packet(folder, bad).is_err(), "{bad}");
            assert!(publish_packet(folder, bad, "{}").is_err(), "{bad}");
        }
        #[cfg(unix)]
        assert!(read_packet(folder, "link.json").is_err());
    }

    #[test]
    fn a_crowded_folder_keeps_the_newest_packets_and_says_it_cut_the_rest() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().to_str().unwrap();
        for index in 0..MAXIMUM_FOLDER_ENTRIES + 3 {
            let path = directory.path().join(format!("p{index:04}.json"));
            std::fs::write(&path, "{}").unwrap();
            let when = std::time::UNIX_EPOCH
                + std::time::Duration::from_secs(1_700_000_000 + index as u64);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        let listing = list_packets(folder).unwrap();
        assert!(listing.truncated);
        assert_eq!(listing.packets.len(), MAXIMUM_FOLDER_ENTRIES);
        let newest = format!("p{:04}.json", MAXIMUM_FOLDER_ENTRIES + 2);
        assert_eq!(listing.packets[0].file_name, newest);
        assert!(
            !list_packets(tempfile::tempdir().unwrap().path().to_str().unwrap())
                .unwrap()
                .truncated
        );
    }

    #[test]
    fn publishing_never_overwrites_and_reading_enforces_the_size_limit() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().to_str().unwrap();
        publish_packet(folder, "a.json", "{\"v\":1}").unwrap();
        assert!(publish_packet(folder, "a.json", "{\"v\":2}").is_err());
        assert_eq!(read_packet(folder, "a.json").unwrap(), "{\"v\":1}");
        std::fs::write(
            directory.path().join("big.json"),
            vec![b'a'; MAXIMUM_PACKET_FILE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(read_packet(folder, "big.json").is_err());
        assert!(list_packets(folder)
            .unwrap()
            .packets
            .iter()
            .any(|p| p.file_name == "big.json" && p.oversized));
        assert!(list_packets("relative/path").is_err());
    }
}
