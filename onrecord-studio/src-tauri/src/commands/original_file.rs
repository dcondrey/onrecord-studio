//! Reading a document's original file into the page view.
//!
//! The caller names a document, never a path: the path is the one the repository recorded, and it
//! is gated to the managed evidence store like every other evidence-reading command. What the
//! viewer is shown is the file as it is now, with its SHA-256 beside the digest recorded at
//! ingestion, so a file that no longer matches is reported rather than shown as the original.

use crate::commands::investigation::AppInvestigationPaths;
use crate::commands::validation::{managed_evidence_file, portal_key as validate_portal_key};
use crate::error::AppError;
use crate::integrity::sha256_hex;
use crate::storage::sqlite::DuckDbRepository;
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use tauri::{ipc::Response, State};
use tokio::sync::Semaphore;

/// Largest original file the page view will load. The whole file is held in memory while it is
/// hashed and sent, so the bound is on memory, not on what a PDF may be.
pub const MAXIMUM_ORIGINAL_BYTES: u64 = 256 * 1024 * 1024;

/// One load at a time: each holds up to `MAXIMUM_ORIGINAL_BYTES`.
static ORIGINAL_READS: Semaphore = Semaphore::const_new(1);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OriginalFileHeader {
    pub byte_length: u64,
    /// SHA-256 of the bytes that precede the header, computed here.
    pub sha256: String,
    /// The digest recorded at ingestion. `None`: none was recorded, which is not a match.
    pub recorded_sha256: Option<String>,
    /// `None` when there is no recorded digest to compare to.
    pub matches_recorded: Option<bool>,
}

#[derive(Debug)]
pub struct OriginalFile {
    pub bytes: Vec<u8>,
    pub header: OriginalFileHeader,
}

fn read_bounded(path: &Path, maximum_bytes: u64) -> Result<Vec<u8>, AppError> {
    let file = std::fs::File::open(path)
        .map_err(|error| AppError::Storage(format!("Failed to open the original file: {error}")))?;
    let mut bytes = Vec::new();
    // The size was checked before opening; a file that grew since is still refused.
    file.take(maximum_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| AppError::Storage(format!("Failed to read the original file: {error}")))?;
    if bytes.len() as u64 > maximum_bytes {
        return Err(AppError::PortalSecurity(format!(
            "The original file is larger than the {maximum_bytes}-byte limit for the page view"
        )));
    }
    Ok(bytes)
}

pub async fn load_original(
    stored_path: PathBuf,
    document_root: &Path,
    maximum_bytes: u64,
    recorded_sha256: Option<String>,
) -> Result<OriginalFile, AppError> {
    let path =
        managed_evidence_file(stored_path, document_root, maximum_bytes, "The page view").await?;
    let _permit = ORIGINAL_READS
        .acquire()
        .await
        .map_err(|_| AppError::Internal("The original file reader is unavailable".to_string()))?;
    let bytes = crate::blocking::spawn("Original file read", move || {
        read_bounded(&path, maximum_bytes)
    })
    .await??;
    let sha256 = sha256_hex(&bytes);
    let recorded_sha256 = recorded_sha256
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let matches_recorded = recorded_sha256.as_ref().map(|recorded| *recorded == sha256);
    Ok(OriginalFile {
        header: OriginalFileHeader {
            byte_length: bytes.len() as u64,
            sha256,
            recorded_sha256,
            matches_recorded,
        },
        bytes,
    })
}

/// A public document's stored file, with the digest recorded for it at ingestion.
pub async fn load_document_original(
    repository: &DuckDbRepository,
    document_root: &Path,
    document_id: u64,
    portal_key: &str,
) -> Result<OriginalFile, AppError> {
    let stored_path = repository
        .resolve_public_document_path(document_id, portal_key)
        .await?;
    let portal = portal_key.to_string();
    let recorded = repository
        .query_connection("Original file recorded digest", move |conn| {
            conn.query_row(
                "SELECT raw_file_sha256 FROM documents WHERE id = ?1 AND portal_subdomain = ?2",
                duckdb::params![document_id, portal],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(|error| {
                AppError::Storage(format!("Failed to read the recorded file digest: {error}"))
            })
        })
        .await?;
    load_original(stored_path, document_root, MAXIMUM_ORIGINAL_BYTES, recorded).await
}

/// The file's bytes followed by a JSON `OriginalFileHeader` and its byte length as a big-endian
/// `u32`. The header trails so the bytes are never copied to make room in front of them.
pub fn frame(original: OriginalFile) -> Result<Vec<u8>, AppError> {
    let header = serde_json::to_vec(&original.header).map_err(|error| {
        AppError::Internal(format!("Failed to encode the file header: {error}"))
    })?;
    let length = u32::try_from(header.len())
        .map_err(|_| AppError::Internal("The file header is too large".to_string()))?;
    let mut framed = original.bytes;
    framed.extend_from_slice(&header);
    framed.extend_from_slice(&length.to_be_bytes());
    Ok(framed)
}

#[tauri::command]
pub async fn read_document_original(
    document_id: u64,
    portal_key: String,
    repository: State<'_, DuckDbRepository>,
    paths: State<'_, AppInvestigationPaths>,
) -> Result<Response, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    let original =
        load_document_original(&repository, &paths.document_root, document_id, portal_key).await?;
    Ok(Response::new(frame(original)?))
}

/// The bytes one source check received and kept, read through the same store gate and digest
/// comparison as the original, against the digest that check observed.
#[tauri::command]
pub async fn read_source_check_copy(
    document_id: u64,
    portal_key: String,
    check_id: u64,
    repository: State<'_, DuckDbRepository>,
    paths: State<'_, AppInvestigationPaths>,
) -> Result<Response, AppError> {
    let portal_key = validate_portal_key(&portal_key)?;
    let (stored_path, observed_sha256) = repository
        .source_check_copy(portal_key, document_id, check_id)
        .await?;
    let copy = load_original(
        stored_path,
        &paths.document_root,
        MAXIMUM_ORIGINAL_BYTES,
        Some(observed_sha256),
    )
    .await?;
    Ok(Response::new(frame(copy)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_a_file_inside_the_store_within_the_limit_is_read_and_its_digest_is_checked() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let inside = root.path().join("a.pdf");
        std::fs::write(&inside, b"%PDF-1.7 body").unwrap();
        let digest = sha256_hex(b"%PDF-1.7 body");

        let read = load_original(inside.clone(), root.path(), 64, Some(digest.to_uppercase()))
            .await
            .unwrap();
        assert_eq!(read.bytes, b"%PDF-1.7 body");
        assert_eq!(read.header.sha256, digest);
        assert_eq!(read.header.matches_recorded, Some(true));

        let changed = load_original(inside.clone(), root.path(), 64, Some("0".repeat(64)))
            .await
            .unwrap();
        assert_eq!(changed.header.matches_recorded, Some(false));
        assert_eq!(changed.header.sha256, digest);

        let unrecorded = load_original(inside.clone(), root.path(), 64, None)
            .await
            .unwrap();
        assert_eq!(unrecorded.header.matches_recorded, None);

        let size = std::fs::metadata(&inside).unwrap().len();
        assert!(load_original(inside.clone(), root.path(), size, None)
            .await
            .is_ok());
        let over = load_original(inside.clone(), root.path(), size - 1, None)
            .await
            .unwrap_err();
        assert!(over.to_string().contains(&(size - 1).to_string()));

        let escaped = outside.path().join("b.pdf");
        std::fs::write(&escaped, b"%PDF").unwrap();
        assert!(load_original(escaped, root.path(), 64, None).await.is_err());
    }

    #[test]
    fn a_file_that_grew_after_the_size_check_is_still_refused() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("g.pdf");
        std::fs::write(&path, vec![0_u8; 11]).unwrap();
        assert!(read_bounded(&path, 10).is_err());
        assert_eq!(read_bounded(&path, 11).unwrap().len(), 11);
    }

    #[test]
    fn the_frame_ends_with_the_header_and_its_length() {
        let framed = frame(OriginalFile {
            bytes: b"abc".to_vec(),
            header: OriginalFileHeader {
                byte_length: 3,
                sha256: "x".into(),
                recorded_sha256: None,
                matches_recorded: None,
            },
        })
        .unwrap();
        let length = u32::from_be_bytes(framed[framed.len() - 4..].try_into().unwrap()) as usize;
        let header = &framed[framed.len() - 4 - length..framed.len() - 4];
        assert_eq!(&framed[..3], b"abc");
        assert!(std::str::from_utf8(header)
            .unwrap()
            .contains("\"byteLength\":3"));
    }
}
