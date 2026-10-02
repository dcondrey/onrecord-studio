//! Format-routed document extraction and recursive container expansion within budget.

use super::*;
use super::{frontier::enqueue_tasks_resilient, guidance::next_msg_attachment_container_depth};
use crate::parsers::{extract_pst_container_with_manifest, inspect_pst_header};

#[derive(Clone, Copy)]
pub(super) struct DocumentExtractionInput<'a> {
    pub(super) request_id: u64,
    pub(super) document: &'a DocumentMetadata,
    pub(super) local_path: &'a Path,
    pub(super) format: DocumentFormat,
    pub(super) depth: PayloadDepth,
    pub(super) raw_file_sha256: &'a str,
    pub(super) container_root_directory: Option<&'a Path>,
}

pub(super) async fn extract_document(
    context: &WorkerContext,
    input: DocumentExtractionInput<'_>,
) -> Result<ExtractedDocument, AppError> {
    let DocumentExtractionInput {
        request_id,
        document,
        local_path,
        format,
        depth,
        raw_file_sha256: _,
        container_root_directory,
    } = input;
    let _parser_permit = context
        .parser_semaphore
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| AppError::Internal("Parser scheduler is unavailable".to_string()))?;
    let primary_result = async {
        match format {
        DocumentFormat::Pdf => {
            // Ordinary ingestion remains searchable-text-only. Route the
            // immutable residual-redaction audit through the dedicated forensic command so OCR
            // fallback cannot silently replace structural evidence.
            let path = local_path.to_path_buf();
            let extraction = spawn_blocking("PDF parser", move || extract_text_layers(&path))
                .await?;
            match extraction {
                Ok(text) => Ok(ExtractedDocument::Searchable {
                    text,
                    was_ocr_processed: false,
                }),
                Err(primary_error) => {
                    let _ocr_permit = context
                        .ocr_semaphore
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| {
                            AppError::Internal("OCR scheduler is unavailable".to_string())
                        })?;
                    let path = local_path.to_path_buf();
                    let ocr_result =
                        spawn_blocking("OCR", move || execute_local_ocr_scan(&path)).await?;
                    match ocr_result {
                        Ok(text) => Ok(ExtractedDocument::Searchable {
                            text,
                            was_ocr_processed: true,
                        }),
                        Err(ocr_error) => Err(AppError::DataParsing(format!(
                            "PDF_PRIMARY_AND_OCR_FAILED: primary parser: {primary_error}; OCR: {ocr_error}"
                        ))),
                    }
                }
            }
        }
        DocumentFormat::Spreadsheet => {
            let path = local_path.to_path_buf();
            let text = spawn_blocking("Spreadsheet parser", move || {
                extract_spreadsheet_layers(&path)
            })
            .await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::OutlookMessage => {
            let attachment_container_depth = next_msg_attachment_container_depth(depth.container);
            let attachment_cache = local_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(format!("{}_attachments", document.id));
            let attachment_root_directory = container_root_directory
                .map(Path::to_path_buf)
                .unwrap_or_else(|| attachment_cache.clone());
            let payload = extract_msg_layers(
                local_path,
                MsgExtractionContext {
                    source_document: document,
                    cache_directory: &attachment_cache,
                    collect_attachments: attachment_container_depth.is_some(),
                },
            )
            .await?;
            let attachment_tasks = if let Some(container_depth) = attachment_container_depth {
                if let Err(error) = validate_container_tree_budget(&attachment_root_directory) {
                    let _ = std::fs::remove_dir_all(&attachment_cache);
                    return Err(error);
                }
                let output_directory = attachment_cache.to_string_lossy().into_owned();
                let container_root_directory =
                    Some(attachment_root_directory.to_string_lossy().into_owned());
                payload
                    .attachment_documents
                    .into_iter()
                    .map(|attachment| IngestionTask::DownloadBinaryPayload {
                        subdomain: context.portal.portal_key.clone(),
                        request_id,
                        document: Box::new(attachment.metadata),
                        output_directory: output_directory.clone(),
                        container_depth,
                        container_root_directory: container_root_directory.clone(),
                        crawl_depth: depth.crawl,
                        structured_depth: depth.structured,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            enqueue_tasks_resilient(context, attachment_tasks).await?;
            // The body is still evidence, so this is not deferred like an archive at the same
            // depth; but the record must say the attachments exist and were not followed.
            let withheld_note = (payload.withheld_attachment_count > 0).then(|| {
                format!(
                    "{} attachment(s) not extracted: maximum nested-container depth reached.",
                    payload.withheld_attachment_count
                )
            });
            let text = [
                payload.metadata_summary,
                payload.body_text,
                withheld_note.unwrap_or_default(),
            ]
            .into_iter()
                .filter(|value| !value.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::PlainText => {
            let path = local_path.to_path_buf();
            let is_html = document
                .content_type
                .as_deref()
                .is_some_and(|value| {
                    matches!(
                        value.split(';').next().unwrap_or(value).trim(),
                        "text/html" | "application/xhtml+xml"
                    )
                })
                || document
                    .file_name
                    .as_deref()
                    .is_some_and(|name| {
                        let name = name.to_ascii_lowercase();
                        [".html", ".htm", ".xhtml"]
                            .iter()
                            .any(|extension| name.ends_with(extension))
                    });
            // The PlainText route carries three markup families. SVG needs its own collector:
            // `extract_plain_text` would index `<path d="M0 0 L400 200"/>` as if it were prose, and
            // `extract_html_text` drops every `svg` subtree by design.
            let is_svg = document
                .content_type
                .as_deref()
                .is_some_and(|value| {
                    value.split(';').next().unwrap_or(value).trim() == "image/svg+xml"
                })
                || document
                    .file_name
                    .as_deref()
                    .is_some_and(|name| name.to_ascii_lowercase().ends_with(".svg"));
            let text = spawn_blocking("Text parser", move || {
                if is_svg {
                    extract_svg_text(&path)
                } else if is_html {
                    extract_html_text(&path)
                } else {
                    extract_plain_text(&path)
                }
            })
            .await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::RichText => {
            let path = local_path.to_path_buf();
            let text = spawn_blocking("RTF parser", move || extract_rtf_text(&path)).await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::Image => {
            let _ocr_permit = context
                .ocr_semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| AppError::Internal("OCR scheduler is unavailable".to_string()))?;
            let path = local_path.to_path_buf();
            let text = spawn_blocking("OCR", move || execute_local_ocr_scan(&path)).await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: true,
            })
        }
        DocumentFormat::WordPackage
        | DocumentFormat::PresentationPackage
        | DocumentFormat::OpenDocumentText
        | DocumentFormat::OpenDocumentPresentation
        | DocumentFormat::Ebook => {
            // Search extraction stays separate from the attributed forensic command, which
            // retains part names, relationship IDs, comments, and revision markers.
            let path = local_path.to_path_buf();
            let text = spawn_blocking("Package parser", move || {
                extract_office_package(&path, format)
            })
            .await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::Sqlite => {
            let path = local_path.to_path_buf();
            let text =
                spawn_blocking("SQLite parser", move || extract_sqlite_text(&path)).await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::DuckDb => {
            let path = local_path.to_path_buf();
            let text =
                spawn_blocking("DuckDB parser", move || extract_duckdb_text(&path)).await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::SqlDump => {
            let path = local_path.to_path_buf();
            let text =
                spawn_blocking("SQL dump parser", move || extract_sql_dump_text(&path)).await??;
            Ok(ExtractedDocument::Searchable {
                text,
                was_ocr_processed: false,
            })
        }
        DocumentFormat::Zip | DocumentFormat::EmailArchive => {
            expand_container(context, input).await
        }
        DocumentFormat::OutlookStore => {
            let path = local_path.to_path_buf();
            match spawn_blocking("PST header", move || inspect_pst_header(&path)).await?? {
                Ok(_) => expand_container(context, input).await,
                Err(reason) => Ok(ExtractedDocument::Deferred {
                    format,
                    route: ExtractionRoute::ToolRequired(reason),
                }),
            }
        }
        DocumentFormat::Tar
        | DocumentFormat::TarGzip
        | DocumentFormat::Gzip
        | DocumentFormat::TarBzip2
        | DocumentFormat::Bzip2
        | DocumentFormat::TarXz
        | DocumentFormat::Xz
        | DocumentFormat::TarZstd
        | DocumentFormat::Zstd => {
            expand_container(context, input).await
        }
            _ => Ok(ExtractedDocument::Deferred {
                format,
                route: format.extraction_route(),
            }),
        }
    }
    .await;

    match primary_result {
        Err(error)
            if matches!(
                &error,
                AppError::DataParsing(_) | AppError::OcrRequired { .. }
            ) && salvage_allowed(format) =>
        {
            let path = local_path.to_path_buf();
            let primary_error = error.to_string();
            match spawn_blocking("Salvage parser", move || {
                extract_salvage_report(&path, primary_error)
            })
            .await
            {
                Ok(Ok(report)) => Ok(ExtractedDocument::Salvaged { report, format }),
                _ => Err(error),
            }
        }
        result => result,
    }
}

fn salvage_allowed(format: DocumentFormat) -> bool {
    matches!(
        format,
        DocumentFormat::Pdf
            | DocumentFormat::PlainText
            | DocumentFormat::RichText
            | DocumentFormat::Spreadsheet
            | DocumentFormat::OutlookMessage
            | DocumentFormat::Sqlite
            | DocumentFormat::DuckDb
            | DocumentFormat::SqlDump
    )
}

async fn expand_container(
    context: &WorkerContext,
    input: DocumentExtractionInput<'_>,
) -> Result<ExtractedDocument, AppError> {
    let DocumentExtractionInput {
        request_id,
        document: source_document,
        local_path,
        format,
        depth,
        raw_file_sha256,
        container_root_directory,
    } = input;
    if depth.container >= MAXIMUM_CONTAINER_DEPTH {
        return Ok(ExtractedDocument::Deferred {
            format,
            route: ExtractionRoute::ToolRequired("maximum nested-container depth reached"),
        });
    }
    let parent = local_path.parent().ok_or_else(|| {
        AppError::DataParsing(format!(
            "CONTAINER_PATH_INVALID: '{}' has no parent directory",
            local_path.display()
        ))
    })?;
    let hash_prefix = raw_file_sha256.get(..16).ok_or_else(|| {
        AppError::DataParsing(
            "CONTAINER_HASH_INVALID: raw file SHA-256 is not normalized".to_string(),
        )
    })?;
    let destination = parent.join(format!("{}_{}_contents", source_document.id, hash_prefix));
    let archive_path = local_path.to_path_buf();
    let extraction_destination = destination.clone();
    let extracted_files = spawn_blocking("Container parser", move || {
        if extraction_destination.try_exists().map_err(|error| {
            AppError::Storage(format!(
                "Failed to inspect cached container '{}': {error}",
                extraction_destination.display()
            ))
        })? {
            return existing_container_manifest(&extraction_destination);
        }
        match format {
            DocumentFormat::Zip | DocumentFormat::EmailArchive => {
                extract_zip_container_with_manifest(&archive_path, &extraction_destination)
            }
            DocumentFormat::OutlookStore => {
                extract_pst_container_with_manifest(&archive_path, &extraction_destination)
            }
            _ => extract_archive_container(&archive_path, &extraction_destination, format),
        }
    })
    .await??;

    let root_directory = container_root_directory
        .map(Path::to_path_buf)
        .unwrap_or_else(|| destination.clone());
    if let Err(error) = validate_container_tree_budget(&root_directory) {
        let _ = std::fs::remove_dir_all(&destination);
        return Err(error);
    }

    let mut identifiers = HashSet::with_capacity(extracted_files.len());
    let mut tasks = Vec::with_capacity(extracted_files.len());
    for extracted_path in extracted_files {
        let metadata = std::fs::symlink_metadata(&extracted_path).map_err(|error| {
            AppError::Internal(format!(
                "CONTAINER_ENTRY_INSPECTION_FAILED: failed to inspect '{}': {error}",
                extracted_path.display()
            ))
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(AppError::DataParsing(format!(
                "CONTAINER_ENTRY_INVALID: '{}' is not a regular file",
                extracted_path.display()
            )));
        }
        let relative_path = extracted_path.strip_prefix(&destination).map_err(|error| {
            AppError::Internal(format!(
                "CONTAINER_ENTRY_PATH_INVALID: '{}' is outside '{}': {error}",
                extracted_path.display(),
                destination.display()
            ))
        })?;
        let identifier = archive_entry_id(
            source_document.id,
            raw_file_sha256,
            depth.container,
            relative_path,
        );
        if !identifiers.insert(identifier) {
            let _ = std::fs::remove_dir_all(&destination);
            return Err(AppError::DataParsing(format!(
                "CONTAINER_IDENTIFIER_COLLISION: '{}' produced a duplicate document identifier",
                relative_path.display()
            )));
        }
        let file_name = extracted_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .ok_or_else(|| {
                AppError::DataParsing(format!(
                    "CONTAINER_ENTRY_NAME_INVALID: '{}' has no file name",
                    extracted_path.display()
                ))
            })?;
        let document = DocumentMetadata {
            id: identifier,
            file_name: Some(file_name),
            file_size: Some(metadata.len()),
            description: Some(format!(
                "Extracted from document {} at {}",
                source_document.id,
                relative_path.display()
            )),
            content_type: None,
            download_url: None,
            created_at: source_document.created_at.clone(),
            updated_at: source_document.updated_at.clone(),
            visibility: source_document.visibility.clone(),
            local_path: Some(extracted_path.to_string_lossy().into_owned()),
            extracted_text_sha256: None,
            was_ocr_processed: false,
        };
        tasks.push(IngestionTask::DownloadBinaryPayload {
            subdomain: context.portal.portal_key.clone(),
            request_id,
            document: Box::new(document),
            output_directory: destination.to_string_lossy().into_owned(),
            container_depth: depth.container + 1,
            container_root_directory: Some(root_directory.to_string_lossy().into_owned()),
            crawl_depth: depth.crawl,
            structured_depth: depth.structured,
        });
    }

    enqueue_tasks_resilient(context, tasks).await?;
    Ok(ExtractedDocument::Expanded {
        format,
        entry_count: identifiers.len(),
    })
}

pub(super) fn existing_container_manifest(destination: &Path) -> Result<Vec<PathBuf>, AppError> {
    let root_metadata = std::fs::symlink_metadata(destination).map_err(|error| {
        AppError::Storage(format!(
            "Failed to inspect cached container '{}': {error}",
            destination.display()
        ))
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(AppError::PortalSecurity(format!(
            "Cached container '{}' is not a managed directory",
            destination.display()
        )));
    }

    let mut directories = vec![destination.to_path_buf()];
    let mut files = Vec::new();
    let mut entry_count = 0_usize;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|error| {
            AppError::Storage(format!(
                "Failed to enumerate cached container '{}': {error}",
                directory.display()
            ))
        })? {
            let entry = entry.map_err(|error| {
                AppError::Storage(format!(
                    "Failed to enumerate cached container '{}': {error}",
                    directory.display()
                ))
            })?;
            entry_count = entry_count.checked_add(1).ok_or_else(|| {
                AppError::DataParsing("Cached container entry count overflowed".to_string())
            })?;
            if entry_count > MAXIMUM_CONTAINER_ENTRIES {
                return Err(AppError::DataParsing(format!(
                    "Cached container '{}' exceeds {MAXIMUM_CONTAINER_ENTRIES} entries",
                    destination.display()
                )));
            }
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                AppError::Storage(format!(
                    "Failed to inspect cached container entry '{}': {error}",
                    path.display()
                ))
            })?;
            if metadata.file_type().is_symlink() {
                return Err(AppError::PortalSecurity(format!(
                    "Cached container entry '{}' is a symbolic link",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                directories.push(path);
            } else if metadata.is_file() {
                files.push(path);
            } else {
                return Err(AppError::PortalSecurity(format!(
                    "Cached container entry '{}' is not a regular file or directory",
                    path.display()
                )));
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Measures the real managed extraction tree without following links. Unlike each format parser's
/// per-archive expansion ceiling, this bound covers every recursively extracted descendant and is
/// recomputed from disk, so a durable child task cannot reset it by resuming after process restart.
fn validate_container_tree_budget(root: &Path) -> Result<u64, AppError> {
    validate_container_tree_budget_with_limits(
        root,
        MAXIMUM_CONTAINER_TREE_ENTRIES,
        MAXIMUM_CONTAINER_TREE_BYTES,
    )
}

pub(super) fn validate_container_tree_budget_with_limits(
    root: &Path,
    maximum_entries: usize,
    maximum_bytes: u64,
) -> Result<u64, AppError> {
    let root_metadata = std::fs::symlink_metadata(root).map_err(|error| {
        AppError::Storage(format!(
            "CONTAINER_TREE_INSPECTION_FAILED: failed to inspect '{}': {error}",
            root.display()
        ))
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(AppError::PortalSecurity(format!(
            "CONTAINER_TREE_INVALID: '{}' is not a managed directory",
            root.display()
        )));
    }

    let mut directories = vec![root.to_path_buf()];
    let mut entry_count = 0_usize;
    let mut total_bytes = 0_u64;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|error| {
            AppError::Storage(format!(
                "CONTAINER_TREE_ENUMERATION_FAILED: failed to enumerate '{}': {error}",
                directory.display()
            ))
        })? {
            let entry = entry.map_err(|error| {
                AppError::Storage(format!(
                    "CONTAINER_TREE_ENUMERATION_FAILED: failed to enumerate '{}': {error}",
                    directory.display()
                ))
            })?;
            entry_count = entry_count.checked_add(1).ok_or_else(|| {
                AppError::DataParsing(
                    "CONTAINER_TREE_ENTRY_COUNT_OVERFLOW: entry count overflowed".to_string(),
                )
            })?;
            if entry_count > maximum_entries {
                return Err(AppError::DataParsing(format!(
                    "CONTAINER_TREE_ENTRY_COUNT_EXCEEDED: recursive extraction exceeds {maximum_entries} entries"
                )));
            }
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
                AppError::Storage(format!(
                    "CONTAINER_TREE_ENTRY_INSPECTION_FAILED: failed to inspect '{}': {error}",
                    path.display()
                ))
            })?;
            if metadata.file_type().is_symlink() {
                return Err(AppError::PortalSecurity(format!(
                    "CONTAINER_TREE_ENTRY_INVALID: '{}' is a symbolic link",
                    path.display()
                )));
            }
            if metadata.is_dir() {
                directories.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(AppError::PortalSecurity(format!(
                    "CONTAINER_TREE_ENTRY_INVALID: '{}' is not a regular file or directory",
                    path.display()
                )));
            }
            total_bytes = total_bytes.checked_add(metadata.len()).ok_or_else(|| {
                AppError::DataParsing(
                    "CONTAINER_TREE_SIZE_OVERFLOW: recursive extraction size overflowed"
                        .to_string(),
                )
            })?;
            if total_bytes > maximum_bytes {
                return Err(AppError::DataParsing(format!(
                    "CONTAINER_TREE_SIZE_EXCEEDED: recursive extraction exceeds {maximum_bytes} bytes"
                )));
            }
        }
    }
    Ok(total_bytes)
}

pub(super) fn archive_entry_id(
    source_document_id: u64,
    raw_file_sha256: &str,
    container_depth: usize,
    relative_path: &Path,
) -> u64 {
    let mut digest = Sha256::new();
    digest.update(b"onrecord-container-entry-v1");
    digest.update(source_document_id.to_le_bytes());
    digest.update((container_depth as u64).to_le_bytes());
    digest.update(raw_file_sha256.as_bytes());
    digest.update(relative_path.as_os_str().as_encoded_bytes());
    let hash = digest.finalize();
    let mut identifier_bytes = [0_u8; 8];
    identifier_bytes.copy_from_slice(&hash[..8]);
    let identifier = u64::from_le_bytes(identifier_bytes);
    SYNTHETIC_DOCUMENT_NAMESPACE | (identifier & (SYNTHETIC_DOCUMENT_NAMESPACE - 1))
}

pub(super) fn extraction_route_label(route: ExtractionRoute) -> String {
    match route {
        ExtractionRoute::Container(name) => format!("container extractor required: {name}"),
        ExtractionRoute::ToolRequired(name) => format!("specialist tool required: {name}"),
        ExtractionRoute::KeyRequired => "decryption key required".to_string(),
        ExtractionRoute::Unknown => "unrecognized document format".to_string(),
        ExtractionRoute::DuckDb | ExtractionRoute::SqlDump => {
            "trusted database extractor unexpectedly deferred".to_string()
        }
        _ => "native extractor unavailable".to_string(),
    }
}
