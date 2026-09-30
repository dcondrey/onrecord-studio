use std::path::{Path, PathBuf};

use crate::error::AppError;

pub const MAXIMUM_PORTAL_KEY_BYTES: usize = 128;
pub const MAXIMUM_PATH_BYTES: usize = 32 * 1024;

pub fn required_text<'a>(
    value: &'a str,
    label: &str,
    maximum_bytes: usize,
) -> Result<&'a str, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::DataParsing(format!("{label} must not be empty")));
    }
    validate_text(value, label, maximum_bytes)?;
    Ok(value)
}

pub fn validate_text(value: &str, label: &str, maximum_bytes: usize) -> Result<(), AppError> {
    if value.len() > maximum_bytes {
        return Err(AppError::DataParsing(format!(
            "{label} exceeds the {maximum_bytes}-byte limit"
        )));
    }
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
    {
        return Err(AppError::DataParsing(format!(
            "{label} contains unsupported control characters"
        )));
    }
    Ok(())
}

pub fn optional_text(
    value: Option<String>,
    label: &str,
    maximum_bytes: usize,
) -> Result<Option<String>, AppError> {
    value
        .map(|value| {
            let value = value.trim();
            if value.is_empty() {
                return Ok(None);
            }
            validate_text(value, label, maximum_bytes)?;
            Ok(Some(value.to_string()))
        })
        .transpose()
        .map(Option::flatten)
}

pub fn portal_key(value: &str) -> Result<&str, AppError> {
    let value = required_text(value, "Portal key", MAXIMUM_PORTAL_KEY_BYTES)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(AppError::DataParsing(
            "Portal key may contain only ASCII letters, digits, hyphens, and underscores"
                .to_string(),
        ));
    }
    Ok(value)
}

/// Rejects a raw IPC-supplied filesystem path before it reaches disk I/O. The analytical import
/// and export commands accept a path the user chose through a native file dialog, which can point
/// anywhere on disk by design -- there is no managed directory to contain it in, unlike
/// `validate_report_path` in `commands/export.rs`, which targets a fixed reports directory. This
/// only closes the symlink indirection: a scripted or compromised IPC caller (not just the
/// intended dialog flow) could otherwise point either command at a symlink planted somewhere
/// reachable, redirecting an import read or an export overwrite to a file the user never chose.
pub fn validate_filesystem_target_path<'a>(
    value: &'a str,
    label: &str,
    maximum_bytes: usize,
) -> Result<&'a str, AppError> {
    let value = required_text(value, label, maximum_bytes)?;
    if let Ok(metadata) = std::fs::symlink_metadata(value) {
        if metadata.file_type().is_symlink() {
            return Err(AppError::PortalSecurity(format!(
                "{label} is a symbolic link, which this operation does not permit"
            )));
        }
    }
    Ok(value)
}

/// As [`validate_filesystem_target_path`], plus requires the path to already exist as a regular
/// file. Import has nothing to read if the path is missing, a directory, or a special file;
/// export may legitimately target a path that does not exist yet, so it stops at the symlink
/// check.
///
/// Reuses [`crate::parsers::source::inspect_regular_file`] rather than re-checking symlink and
/// regular-file status here: it is the same gate `import_analytical_csv` runs just before reading
/// the file, so this only moves that check earlier -- to the IPC boundary, before the CSV/XLSX
/// parsing machinery even starts -- instead of adding a second, independent implementation of it.
pub fn validate_import_source_path<'a>(
    value: &'a str,
    label: &str,
    maximum_bytes: usize,
) -> Result<&'a str, AppError> {
    let value = required_text(value, label, maximum_bytes)?;
    crate::parsers::source::inspect_regular_file(
        Path::new(value),
        "IMPORT",
        crate::storage::tabular::MAX_SOURCE_BYTES,
    )?;
    Ok(value)
}

pub fn optional_portal_key(value: Option<String>) -> Result<Option<String>, AppError> {
    value
        .map(|value| {
            let value = value.trim();
            if value.is_empty() {
                Ok(None)
            } else {
                portal_key(value).map(|value| Some(value.to_string()))
            }
        })
        .transpose()
        .map(Option::flatten)
}

/// Resolves a recorded evidence path for a read-only integrity check, reporting why a path is
/// unusable instead of failing the caller.
///
/// This is deliberately not [`managed_evidence_file`], which is built for parsers that materialize
/// a file and must refuse anything they cannot safely load. Three differences matter:
///
/// * **Rejection is data, not an error.** A provenance verification that cannot read its file
///   still has to be recorded; turning it into an error loses the entry from an append-only log
///   whose whole purpose is to show what was checked and when.
/// * **Empty files are accepted.** A zero-byte release is a real document -- a truncated download,
///   or an agency that released an empty file -- and its digest is still worth recording.
/// * **No size ceiling.** The digest streams through a fixed buffer, so memory is flat whatever
///   the file size. A cap here would bound time only, at the cost of refusing to verify exactly
///   the largest pieces of evidence.
///
/// The containment and symbolic-link checks are kept, because they are what stops a path in the
/// evidence store from reaching a file outside it. They are evaluated against the path as read;
/// nothing is held open afterwards, so a file replaced between this check and the hash is hashed
/// as it stands at hash time. That is acceptable for an integrity report that timestamps its own
/// result, and the alternative -- threading the document root into the repository -- puts
/// filesystem policy in the storage layer.
pub async fn readable_evidence_file(
    requested: &Path,
    document_root: &Path,
) -> Result<PathBuf, String> {
    let metadata = tokio::fs::symlink_metadata(requested)
        .await
        .map_err(|error| format!("The recorded evidence file is not accessible: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err(
            "The recorded evidence path is a symbolic link, which the managed evidence store does \
             not permit"
                .to_string(),
        );
    }
    let root = tokio::fs::canonicalize(document_root)
        .await
        .map_err(|error| format!("The managed evidence store is not accessible: {error}"))?;
    let canonical = tokio::fs::canonicalize(requested)
        .await
        .map_err(|error| format!("The recorded evidence file is not accessible: {error}"))?;
    if !canonical.starts_with(&root) {
        return Err(
            "The recorded evidence path resolves outside the managed evidence store".to_string(),
        );
    }
    if !tokio::fs::symlink_metadata(&canonical)
        .await
        .map_err(|error| format!("The recorded evidence file is not accessible: {error}"))?
        .is_file()
    {
        return Err("The recorded evidence path is not a regular file".to_string());
    }
    Ok(canonical)
}

pub async fn managed_evidence_file(
    requested: PathBuf,
    document_root: &Path,
    maximum_bytes: u64,
    operation: &str,
) -> Result<PathBuf, AppError> {
    let requested_metadata = tokio::fs::symlink_metadata(&requested)
        .await
        .map_err(|error| {
            AppError::Storage(format!("{operation} target is inaccessible: {error}"))
        })?;
    if requested_metadata.file_type().is_symlink() {
        return Err(AppError::PortalSecurity(format!(
            "{operation} does not accept symbolic-link targets"
        )));
    }

    let root = tokio::fs::canonicalize(document_root)
        .await
        .map_err(|error| AppError::Storage(format!("Document root is inaccessible: {error}")))?;
    let canonical = tokio::fs::canonicalize(&requested).await.map_err(|error| {
        AppError::Storage(format!("{operation} target is inaccessible: {error}"))
    })?;
    if !canonical.starts_with(&root) {
        return Err(AppError::PortalSecurity(format!(
            "{operation} is restricted to the managed evidence store"
        )));
    }

    let metadata = tokio::fs::symlink_metadata(&canonical)
        .await
        .map_err(|error| {
            AppError::Storage(format!("Failed to inspect {operation} target: {error}"))
        })?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > maximum_bytes
    {
        return Err(AppError::PortalSecurity(format!(
            "{operation} target must be a non-empty regular file no larger than {maximum_bytes} bytes"
        )));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::readable_evidence_file;

    /// The gate is what stops a path recorded in the evidence store from reaching a file outside
    /// it, and it must report each refusal rather than collapsing them into one message: an
    /// investigator needs to tell "the download never finished" from "something replaced this
    /// entry with a link".
    #[tokio::test]
    async fn the_evidence_gate_refuses_links_and_escapes_but_accepts_empty_files() {
        let root = tempfile::tempdir().expect("evidence root must be created");
        let outside = tempfile::tempdir().expect("outside directory must be created");

        let contained = root.path().join("release.pdf");
        std::fs::write(&contained, b"bytes").expect("contained file must be written");
        assert_eq!(
            readable_evidence_file(&contained, root.path()).await.ok(),
            Some(
                tokio::fs::canonicalize(&contained)
                    .await
                    .expect("canonicalization must succeed")
            )
        );

        // A zero-byte release is a real document and its digest is still worth recording, so the
        // provenance gate accepts what `managed_evidence_file` deliberately rejects.
        let empty = root.path().join("empty.pdf");
        std::fs::write(&empty, b"").expect("empty file must be written");
        assert!(readable_evidence_file(&empty, root.path()).await.is_ok());

        let escaping = outside.path().join("elsewhere.pdf");
        std::fs::write(&escaping, b"bytes").expect("outside file must be written");
        let escape_error = readable_evidence_file(&escaping, root.path())
            .await
            .expect_err("a path outside the store must be refused");
        assert!(
            escape_error.contains("outside the managed evidence store"),
            "unexpected reason: {escape_error}"
        );

        #[cfg(unix)]
        {
            let link = root.path().join("link.pdf");
            std::os::unix::fs::symlink(&escaping, &link).expect("symlink must be created");
            let link_error = readable_evidence_file(&link, root.path())
                .await
                .expect_err("a symbolic link must be refused");
            assert!(
                link_error.contains("symbolic link"),
                "unexpected reason: {link_error}"
            );
        }

        let absent_error = readable_evidence_file(&root.path().join("gone.pdf"), root.path())
            .await
            .expect_err("a missing file must be refused");
        assert!(
            absent_error.contains("not accessible"),
            "unexpected reason: {absent_error}"
        );

        let directory_error = readable_evidence_file(root.path(), root.path())
            .await
            .expect_err("a directory must be refused");
        assert!(
            directory_error.contains("not a regular file"),
            "unexpected reason: {directory_error}"
        );
    }
}
