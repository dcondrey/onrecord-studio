//! The content-addressed evidence store: cached payloads, downloads, imports, and object links.

use super::*;

pub(super) fn validate_internal_output_directory(
    root: &Path,
    output: &Path,
) -> Result<(), AppError> {
    if output.as_os_str().is_empty()
        || output
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        || !output.starts_with(root)
    {
        return Err(AppError::PortalSecurity(format!(
            "Document output directory '{}' escapes the managed download root",
            output.display()
        )));
    }
    Ok(())
}

/// The digest a file in the content-addressed object store was committed under, if `path` is one.
pub(super) fn content_address<'a>(download_root: &Path, path: &'a Path) -> Option<&'a str> {
    let relative = path.strip_prefix(download_root.join("objects")).ok()?;
    let mut components = relative.components();
    let prefix = components.next()?.as_os_str().to_str()?;
    let digest = components.next()?.as_os_str().to_str()?;
    (components.next().is_none()
        && digest.len() == 64
        && digest
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        && digest.starts_with(prefix))
    .then_some(digest)
}

pub(super) async fn validate_cached_payload(
    managed_root: &Path,
    path: &Path,
    expected_size: Option<u64>,
) -> Result<(PathBuf, u64), AppError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        || !path.starts_with(managed_root)
    {
        return Err(AppError::PortalSecurity(format!(
            "Cached payload '{}' escapes the managed download root",
            path.display()
        )));
    }
    let metadata = tokio::fs::symlink_metadata(path).await.map_err(|error| {
        AppError::Storage(format!(
            "Failed to inspect cached payload '{}': {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::DataParsing(format!(
            "Cached payload '{}' is not a regular file",
            path.display()
        )));
    }
    if expected_size.is_some_and(|expected| metadata.len() != expected) {
        return Err(AppError::DataParsing(format!(
            "Cached payload '{}' size does not match its metadata",
            path.display()
        )));
    }
    Ok((path.to_path_buf(), metadata.len()))
}

#[cfg(test)]
pub(super) async fn persist_download(
    output_directory: &Path,
    document_id: u64,
    original_file_name: &str,
    bytes: &[u8],
) -> Result<PathBuf, AppError> {
    tokio::fs::create_dir_all(output_directory)
        .await
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to create download directory '{}': {error}",
                output_directory.display()
            ))
        })?;
    let file_name = sanitize_file_name(original_file_name, document_id);
    let final_path = output_directory.join(format!("{document_id}_{file_name}"));
    if tokio::fs::try_exists(&final_path)
        .await
        .map_err(|error| AppError::Storage(format!("Failed to inspect download: {error}")))?
    {
        let existing_hash = sha256_file(&final_path).await?;
        if constant_time_equal(existing_hash.as_bytes(), sha256_hex(bytes).as_bytes()) {
            return Ok(final_path);
        }
        return Err(AppError::Storage(format!(
            "Download collision at '{}': existing bytes differ",
            final_path.display()
        )));
    }
    let temporary_path = output_directory.join(format!(".{document_id}.{}.tmp", Uuid::new_v4()));
    let mut temporary_file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .await
        .map_err(|error| {
            AppError::Storage(format!("Failed to create temporary download: {error}"))
        })?;
    if let Err(error) = temporary_file.write_all(bytes).await {
        drop(temporary_file);
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(AppError::Storage(format!(
            "Failed to write temporary download: {error}"
        )));
    }
    if let Err(error) = temporary_file.sync_all().await {
        drop(temporary_file);
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(AppError::Storage(format!(
            "Failed to sync temporary download: {error}"
        )));
    }
    drop(temporary_file);
    match tokio::fs::hard_link(&temporary_path, &final_path).await {
        Ok(()) => {
            tokio::fs::remove_file(&temporary_path)
                .await
                .map_err(|error| {
                    AppError::Storage(format!("Failed to remove temporary link: {error}"))
                })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            let existing_hash = sha256_file(&final_path).await?;
            if !constant_time_equal(existing_hash.as_bytes(), sha256_hex(bytes).as_bytes()) {
                return Err(AppError::Storage(format!(
                    "Concurrent download collision at '{}'",
                    final_path.display()
                )));
            }
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(&temporary_path).await;
            return Err(AppError::Storage(format!(
                "Failed to commit download: {error}"
            )));
        }
    }
    Ok(final_path)
}

pub(super) async fn stream_download(
    client: &ApiClient,
    portal: &PortalRegistration,
    download_url: &str,
    managed_root: &Path,
    document_id: u64,
) -> Result<DownloadedPayload, AppError> {
    let staging_directory = managed_root.join(".staging");
    tokio::fs::create_dir_all(&staging_directory)
        .await
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to create download staging directory '{}': {error}",
                staging_directory.display()
            ))
        })?;
    let temporary_path =
        staging_directory.join(format!(".{document_id}.{}.partial", Uuid::new_v4()));
    let downloaded = client
        .download_document_to_path(portal, download_url, &temporary_path)
        .await?;
    if downloaded.byte_count == 0 {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return Err(AppError::DataParsing(format!(
            "Document {document_id} download was empty"
        )));
    }
    let path = commit_staged_object(managed_root, &temporary_path, &downloaded.sha256).await?;
    Ok(DownloadedPayload { path, ..downloaded })
}

/// The import ceiling for one selected file: the PST cap when it starts with the Outlook store
/// signature, the generic limit otherwise. The signature is re-checked by format detection on the
/// managed copy, and a non-PST over the generic limit is refused there by its parser capability.
pub(super) async fn local_import_limit(path: &Path) -> u64 {
    let mut signature = [0_u8; 4];
    let is_outlook_store = match tokio::fs::File::open(path).await {
        Ok(mut file) => file.read_exact(&mut signature).await.is_ok() && &signature == b"!BDN",
        Err(_) => false,
    };
    if is_outlook_store {
        MAXIMUM_PST_LOCAL_IMPORT_BYTES
    } else {
        MAXIMUM_LOCAL_IMPORT_BYTES
    }
}

pub(super) async fn import_local_payload(
    download_root: &Path,
    source_path: &Path,
    document_id: u64,
) -> Result<(PathBuf, u64), AppError> {
    let metadata = tokio::fs::symlink_metadata(source_path)
        .await
        .map_err(|error| {
            AppError::Storage(format!(
                "Failed to inspect selected file '{}': {error}",
                source_path.display()
            ))
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::DataParsing(format!(
            "Selected path '{}' is not a regular file",
            source_path.display()
        )));
    }
    let maximum_bytes = local_import_limit(source_path).await;
    if metadata.len() > maximum_bytes {
        return Err(AppError::DataParsing(format!(
            "Selected file '{}' exceeds the {maximum_bytes}-byte limit",
            source_path.display()
        )));
    }
    let staging_directory = download_root.join(".staging");
    tokio::fs::create_dir_all(&staging_directory)
        .await
        .map_err(|error| {
            AppError::Storage(format!("Failed to create import staging area: {error}"))
        })?;
    let staging_path = staging_directory.join(format!("{document_id}.{}.local", Uuid::new_v4()));
    let mut source = tokio::fs::File::open(source_path).await.map_err(|error| {
        AppError::Storage(format!(
            "Failed to open selected file '{}': {error}",
            source_path.display()
        ))
    })?;
    let mut destination = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging_path)
        .await
        .map_err(|error| AppError::Storage(format!("Failed to create staged import: {error}")))?;
    let mut digest = Sha256::new();
    let mut byte_count = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer).await.map_err(|error| {
            AppError::Storage(format!("Failed while reading selected file: {error}"))
        })?;
        if count == 0 {
            break;
        }
        byte_count = byte_count.checked_add(count as u64).ok_or_else(|| {
            AppError::DataParsing("Selected file byte count overflowed".to_string())
        })?;
        if byte_count > maximum_bytes {
            drop(destination);
            let _ = tokio::fs::remove_file(&staging_path).await;
            return Err(AppError::DataParsing(format!(
                "Selected file exceeded the {maximum_bytes}-byte limit while reading"
            )));
        }
        destination
            .write_all(&buffer[..count])
            .await
            .map_err(|error| {
                AppError::Storage(format!("Failed while staging selected file: {error}"))
            })?;
        digest.update(&buffer[..count]);
    }
    destination.sync_all().await.map_err(|error| {
        AppError::Storage(format!("Failed to sync staged selected file: {error}"))
    })?;
    drop(destination);
    let sha256 = digest_hex(digest.finalize());
    let final_path = commit_staged_object(download_root, &staging_path, &sha256).await?;
    Ok((final_path, byte_count))
}

pub(super) async fn store_inline_payload(
    download_root: &Path,
    document_id: u64,
    bytes: &[u8],
) -> Result<PathBuf, AppError> {
    let staging_directory = download_root.join(".staging");
    tokio::fs::create_dir_all(&staging_directory)
        .await
        .map_err(|error| {
            AppError::Storage(format!("Failed to create paste staging area: {error}"))
        })?;
    let staging_path = staging_directory.join(format!("{document_id}.{}.paste", Uuid::new_v4()));
    let mut destination = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging_path)
        .await
        .map_err(|error| AppError::Storage(format!("Failed to create staged paste: {error}")))?;
    destination.write_all(bytes).await.map_err(|error| {
        AppError::Storage(format!("Failed to write staged pasted content: {error}"))
    })?;
    destination.sync_all().await.map_err(|error| {
        AppError::Storage(format!("Failed to sync staged pasted content: {error}"))
    })?;
    drop(destination);
    commit_staged_object(download_root, &staging_path, &sha256_hex(bytes)).await
}

/// Objects an ingestion task linked, or found already present, and may not yet reference from a
/// committed row. Purge unlinks only under this lock and skips these, so it can never remove a file
/// between a concurrent sync's link and that sync's row; the grace outlasts any task.
static RECENT_OBJECT_LINKS: std::sync::LazyLock<Mutex<HashMap<PathBuf, std::time::Instant>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

const RECENT_OBJECT_LINK_GRACE: Duration = Duration::from_secs(6 * 60 * 60);

/// Unlinks evidence files a committed purge released, except any a running sync has linked and
/// not yet referenced. Returns the paths it left in place, so a caller never reports them deleted.
pub(crate) async fn remove_released_evidence_files(paths: &[String]) -> Vec<String> {
    let mut recent = RECENT_OBJECT_LINKS.lock().await;
    recent.retain(|_, linked_at| linked_at.elapsed() < RECENT_OBJECT_LINK_GRACE);
    let mut kept = Vec::new();
    for path in paths {
        if recent.contains_key(Path::new(path)) {
            kept.push(path.clone());
            continue;
        }
        let _ = tokio::fs::remove_file(path).await;
    }
    kept
}

/// A committed row now references `path`, so the purge-time reference check covers it.
pub(super) async fn release_object_link(path: &Path) {
    RECENT_OBJECT_LINKS.lock().await.remove(path);
}

pub(super) async fn commit_staged_object(
    download_root: &Path,
    staging_path: &Path,
    sha256: &str,
) -> Result<PathBuf, AppError> {
    let prefix = sha256.get(..2).ok_or_else(|| {
        AppError::Internal("Content-addressed digest was not normalized".to_string())
    })?;
    let object_directory = download_root.join("objects").join(prefix);
    tokio::fs::create_dir_all(&object_directory)
        .await
        .map_err(|error| AppError::Storage(format!("Failed to create object store: {error}")))?;
    let final_path = object_directory.join(sha256);
    let linked = {
        let mut recent = RECENT_OBJECT_LINKS.lock().await;
        let linked = tokio::fs::hard_link(staging_path, &final_path).await;
        if linked.as_ref().map_or_else(
            |error| error.kind() == std::io::ErrorKind::AlreadyExists,
            |()| true,
        ) {
            recent.insert(final_path.clone(), std::time::Instant::now());
        }
        linked
    };
    match linked {
        Ok(()) => {
            // The object's bytes were synced while staged; its new directory entry is not durable
            // until the directory is, and a committed row may point at it after power loss.
            let linked = final_path.clone();
            let _ =
                tokio::task::spawn_blocking(move || crate::atomic_write::sync_directory(&linked))
                    .await;
            tokio::fs::remove_file(staging_path)
                .await
                .map_err(|error| {
                    AppError::Storage(format!("Failed to clear staged input: {error}"))
                })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing_hash = sha256_file(&final_path).await?;
            let _ = tokio::fs::remove_file(staging_path).await;
            if !constant_time_equal(existing_hash.as_bytes(), sha256.as_bytes()) {
                return Err(AppError::Storage(format!(
                    "Content-addressed collision at '{}'",
                    final_path.display()
                )));
            }
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(staging_path).await;
            return Err(AppError::Storage(format!(
                "Failed to commit staged input: {error}"
            )));
        }
    }
    Ok(final_path)
}

pub(super) fn sanitize_file_name(file_name: &str, document_id: u64) -> String {
    let sanitized = sanitized_component(file_name, 180);
    if sanitized.is_empty() {
        format!("document_{document_id}.bin")
    } else {
        sanitized
    }
}

pub(super) async fn sha256_file(path: &Path) -> Result<String, AppError> {
    sha256_file_async(path)
        .await
        .map_err(|error| AppError::Storage(format!("Failed to hash '{}': {error}", path.display())))
}

pub(super) fn parser_memory_permits(source_bytes: u64, working_set_multiplier: u32) -> u32 {
    let estimated_bytes = source_bytes
        .saturating_mul(u64::from(working_set_multiplier.max(1)))
        .max(PARSER_MEMORY_PERMIT_BYTES);
    estimated_bytes
        .saturating_add(PARSER_MEMORY_PERMIT_BYTES - 1)
        .checked_div(PARSER_MEMORY_PERMIT_BYTES)
        .unwrap_or(u64::from(PARSER_MEMORY_PERMITS))
        .clamp(1, u64::from(PARSER_MEMORY_PERMITS)) as u32
}
