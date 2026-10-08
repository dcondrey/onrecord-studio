//! Downloaded payload processing, extraction guidance, and indexing of extracted documents.

use super::*;
use super::{
    crawl::record_frontier_exclusion,
    extraction::{extract_document, extraction_route_label, DocumentExtractionInput},
    objects::{
        content_address, parser_memory_permits, release_object_link, sanitize_file_name,
        sha256_file, stream_download, validate_cached_payload, validate_internal_output_directory,
    },
    structured::expand_structured_document_links,
    worker::send_progress,
};

pub(super) async fn process_binary_payload(
    context: &WorkerContext,
    request_id: u64,
    mut document: DocumentMetadata,
    output_directory: PathBuf,
    depth: PayloadDepth,
    container_root_directory: Option<PathBuf>,
) -> Result<(), AppError> {
    validate_internal_output_directory(&context.download_root, &output_directory)?;
    if let Some(root) = container_root_directory.as_deref() {
        validate_internal_output_directory(&context.download_root, root)?;
    }
    let file_name = sanitize_file_name(document.file_name.as_deref().unwrap_or(""), document.id);
    send_progress(
        context,
        IngestionProgress::DocumentStarted {
            portal_key: context.portal.portal_key.clone(),
            document_id: document.id,
            file_name,
        },
    )
    .await;
    let (local_path, byte_count, raw_file_sha256, acquisition) =
        if let Some(local_path) = &document.local_path {
            let (path, count) = validate_cached_payload(
                &context.download_root,
                Path::new(local_path),
                document.file_size,
            )
            .await?;
            let hash = sha256_file(&path).await?;
            // An object's file name is the digest it was committed under. Re-hashing a damaged or
            // edited object and recording the new digest would make the corruption look like the
            // evidence, so a mismatch stops here.
            if let Some(committed) = content_address(&context.download_root, &path) {
                if !constant_time_equal(committed.as_bytes(), hash.as_bytes()) {
                    return Err(AppError::Storage(format!(
                        "Stored evidence '{}' no longer matches the SHA-256 it was committed under",
                        path.display()
                    )));
                }
            }
            (path, count, hash, None)
        } else {
            context
                .repository
                .check_retained_file_budget(&context.download_root)
                .await?;
            context.limiter.acquire(&context.portal.portal_key).await?;
            let download_url = document
                .download_url
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    AppError::DataParsing(format!("Document {} has no download URL", document.id))
                })?;
            let downloaded = stream_download(
                &context.client,
                &context.portal,
                download_url,
                &context.download_root,
                document.id,
            )
            .await?;
            (
                downloaded.path,
                downloaded.byte_count,
                downloaded.sha256,
                Some(downloaded.acquisition),
            )
        };
    let entropy_path = local_path.clone();
    let entropy_file_name = document.file_name.clone();
    let entropy = spawn_blocking("Entropy assessment", move || {
        assess_file_entropy(&entropy_path, entropy_file_name.as_deref())
    })
    .await??;
    document.local_path = Some(local_path.to_string_lossy().into_owned());
    document.file_size = Some(byte_count);
    let format_path = local_path.clone();
    let declared_file_name = document.file_name.clone();
    let content_type = document.content_type.clone();
    let format = spawn_blocking("Format detector", move || {
        detect_document_format(
            &format_path,
            declared_file_name.as_deref(),
            content_type.as_deref(),
        )
    })
    .await??;
    let capability = parser_capability(format);
    if byte_count > capability.maximum_source_bytes {
        return Err(AppError::DataParsing(format!(
            "PARSER_SOURCE_SIZE_EXCEEDED: {format:?} source contains {byte_count} bytes; capability limit is {}",
            capability.maximum_source_bytes
        )));
    }
    let memory_permits = parser_memory_permits(byte_count, capability.working_set_multiplier);
    let _memory_permit = context
        .parser_memory
        .clone()
        .acquire_many_owned(memory_permits)
        .await
        .map_err(|_| AppError::Internal("Parser memory governor is unavailable".to_string()))?;
    context
        .repository
        .insert_or_update_document(&context.portal.portal_key, request_id, &document)
        .await?;
    release_object_link(&local_path).await;
    // Recorded once the row exists and before extraction rewrites its digest, so the event
    // captures the digest this fetch replaced.
    if let Some(acquisition) = &acquisition {
        context
            .repository
            .record_document_acquisition(
                &context.portal.portal_key,
                document.id,
                Some(&context.sync_run_id),
                &raw_file_sha256,
                byte_count,
                acquisition,
            )
            .await?;
    }
    expand_structured_document_links(context, request_id, &document, &local_path, depth).await?;
    let extracted_document = extract_document(
        context,
        DocumentExtractionInput {
            request_id,
            document: &document,
            local_path: &local_path,
            format,
            depth,
            raw_file_sha256: &raw_file_sha256,
            container_root_directory: container_root_directory.as_deref(),
        },
    )
    .await?;
    let download_finalization = DocumentDownloadFinalization {
        document_id: document.id,
        portal_subdomain: &context.portal.portal_key,
        raw_file_size: byte_count,
        raw_file_sha256: &raw_file_sha256,
        local_path: &local_path,
        shannon_entropy: Some(entropy.shannon_entropy),
        entropy_sample_bytes: entropy.sampled_bytes,
        suspicious_data_density: entropy.suspicious_data_density,
    };
    // NextRequest's API supplies the document's own `created_at`. Every other source kind in this
    // queue currently fills `DocumentMetadata::created_at` with `Utc::now()` when it discovers or
    // materializes a payload (including local files, HTML pages, and ArcGIS pages). Treating those
    // ingestion timestamps as document dates would make a bounded historical range refuse nearly
    // every non-NextRequest document. Container members inherit their parent's value, so this gate
    // remains correct for nested documents too.
    let document_date =
        document_date_for_guidance(context.portal.source_kind, document.created_at.as_str());
    if matches!(
        &extracted_document,
        ExtractedDocument::Expanded { .. } | ExtractedDocument::Deferred { .. }
    ) {
        context
            .repository
            .mark_document_downloaded(download_finalization)
            .await?;
    }
    match extracted_document {
        ExtractedDocument::Searchable {
            text,
            was_ocr_processed,
        } => {
            let Some(character_count) = finalize_and_index_extracted_document(
                context,
                request_id,
                download_finalization,
                text,
                was_ocr_processed,
                document_date,
            )
            .await?
            else {
                return Ok(());
            };
            send_progress(
                context,
                IngestionProgress::DocumentExtracted {
                    portal_key: context.portal.portal_key.clone(),
                    document_id: document.id,
                    character_count,
                    was_ocr_processed,
                },
            )
            .await;
        }
        ExtractedDocument::Expanded {
            format,
            entry_count,
        } => {
            let format_label = format!("{format:?}");
            context
                .repository
                .mark_document_extraction_state(
                    document.id,
                    &context.portal.portal_key,
                    "EXPANDED",
                    Some(&format_label),
                    None,
                )
                .await?;
            send_progress(
                context,
                IngestionProgress::DocumentExpanded {
                    portal_key: context.portal.portal_key.clone(),
                    document_id: document.id,
                    format: format_label,
                    entry_count,
                },
            )
            .await;
        }
        ExtractedDocument::Deferred { format, route } => {
            let format_label = format!("{format:?}");
            let reason = extraction_route_label(route);
            context
                .repository
                .mark_document_extraction_state(
                    document.id,
                    &context.portal.portal_key,
                    "DEFERRED",
                    Some(&format_label),
                    Some(&reason),
                )
                .await?;
            send_progress(
                context,
                IngestionProgress::DocumentDeferred {
                    portal_key: context.portal.portal_key.clone(),
                    document_id: document.id,
                    format: format_label,
                    reason,
                },
            )
            .await;
        }
        ExtractedDocument::Salvaged { report, format } => {
            let decoder_summary = report.decoder_summary();
            let provenance_marker = report.provenance_marker.to_string();
            let source_sha256 = report.source_sha256.clone();
            let recovered_run_count = report.runs.len();
            let primary_error = report.primary_error.clone();
            if finalize_and_index_extracted_document(
                context,
                request_id,
                download_finalization,
                report.text,
                false,
                document_date,
            )
            .await?
            .is_none()
            {
                return Ok(());
            }
            let format_label = format!("{format:?}");
            context
                .repository
                .mark_document_extraction_state(
                    document.id,
                    &context.portal.portal_key,
                    "SALVAGED",
                    Some(&format_label),
                    None,
                )
                .await?;
            send_progress(
                context,
                IngestionProgress::DocumentSalvaged {
                    portal_key: context.portal.portal_key.clone(),
                    document_id: document.id,
                    format: format_label,
                    provenance_marker,
                    source_sha256,
                    recovered_run_count,
                    decoder_summary,
                    primary_error,
                },
            )
            .await;
        }
    }
    send_progress(
        context,
        IngestionProgress::DocumentCompleted {
            portal_key: context.portal.portal_key.clone(),
            document_id: document.id,
            bytes_saved: byte_count,
        },
    )
    .await;
    context
        .statistics
        .documents_processed
        .fetch_add(1, Ordering::AcqRel);
    if !document_is_html(&document) {
        context
            .statistics
            .data_documents_processed
            .fetch_add(1, Ordering::AcqRel);
    }
    Ok(())
}

/// Whether a downloaded document declares one of the given kinds, by its media type (compared
/// case-insensitively, without parameters) or by its file extension.
pub(super) fn document_declares_type(
    document: &DocumentMetadata,
    media_type_matches: impl Fn(&str) -> bool,
    extensions: &[&str],
) -> bool {
    document.content_type.as_deref().is_some_and(|value| {
        media_type_matches(
            &value
                .split(';')
                .next()
                .unwrap_or(value)
                .trim()
                .to_ascii_lowercase(),
        )
    }) || document.file_name.as_deref().is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        extensions.iter().any(|extension| name.ends_with(extension))
    })
}

pub(super) fn document_is_html(document: &DocumentMetadata) -> bool {
    document_declares_type(
        document,
        |media_type| matches!(media_type, "text/html" | "application/xhtml+xml"),
        &[".html", ".htm", ".xhtml"],
    )
}

async fn index_extracted_document(
    context: &WorkerContext,
    request_id: u64,
    document_id: u64,
    raw_file_sha256: &str,
    extracted_text_sha256: &str,
    text: String,
) -> Result<(), AppError> {
    context
        .corpus_index
        .index_document(CorpusDocumentInput {
            portal_key: context.portal.portal_key.clone(),
            request_id,
            document_id,
            raw_file_sha256: raw_file_sha256.to_string(),
            extracted_text_sha256: extracted_text_sha256.to_string(),
            text,
        })
        .await
}

/// Content keyword/regex, organization, and document-date guidance predicates, evaluated once
/// extracted text (and, via `agency_label_for_request`, the owning agency) are actually available
/// -- unlike the pre-download predicates in `guidance_allows_url`. `agency_label` is `None` both
/// when the source has no organization guidance (never fetched) and when the request has no
/// agency on file; either way the organization predicate contributes nothing. `document_created_at`
/// must be `None` unless the caller can attest it is a date the document itself actually carries
/// -- see the call site's comment on `SourceKind::NextRequest` for why most ingestion paths stamp
/// this field with the ingestion instant instead, which would make the date predicate exclude
/// nearly everything on any bounded range.
pub(super) fn evaluate_extracted_document_guidance(
    guidance: &SourceGuidance,
    compiled: &CompiledGuidancePatterns,
    text: &str,
    agency_label: Option<&str>,
    document_created_at: Option<&str>,
) -> Result<(), FrontierExclusion> {
    if guidance
        .exclude_content_keywords
        .iter()
        .any(|keyword| text.contains(keyword.as_str()))
    {
        return Err(FrontierExclusion::ContentKeyword);
    }
    if !guidance.include_content_keywords.is_empty()
        && !guidance
            .include_content_keywords
            .iter()
            .any(|keyword| text.contains(keyword.as_str()))
    {
        return Err(FrontierExclusion::ContentKeyword);
    }
    if compiled
        .exclude_content_patterns
        .iter()
        .any(|pattern| pattern.is_match(text))
    {
        return Err(FrontierExclusion::ContentPattern);
    }
    if !compiled.include_content_patterns.is_empty()
        && !compiled
            .include_content_patterns
            .iter()
            .any(|pattern| pattern.is_match(text))
    {
        return Err(FrontierExclusion::ContentPattern);
    }
    if let Some(agency) = agency_label {
        let agency_lower = agency.to_ascii_lowercase();
        if guidance
            .exclude_organizations
            .iter()
            .any(|name| agency_lower.contains(&name.to_ascii_lowercase()))
        {
            return Err(FrontierExclusion::Organization);
        }
        if !guidance.include_organizations.is_empty()
            && !guidance
                .include_organizations
                .iter()
                .any(|name| agency_lower.contains(&name.to_ascii_lowercase()))
        {
            return Err(FrontierExclusion::Organization);
        }
    }
    if guidance.document_date_from.is_some() || guidance.document_date_to.is_some() {
        // The first 10 bytes of an ISO-8601 timestamp are its calendar date; a value too short to
        // hold one carries no date to compare, so the predicate contributes nothing rather than
        // excluding on an absence.
        if let Some(date) = document_created_at.and_then(|value| value.get(0..10)) {
            let before_start = guidance
                .document_date_from
                .as_deref()
                .is_some_and(|from| date < from);
            let after_end = guidance
                .document_date_to
                .as_deref()
                .is_some_and(|to| date > to);
            if before_start || after_end {
                return Err(FrontierExclusion::DocumentDate);
            }
        }
    }
    Ok(())
}

pub(super) fn document_date_for_guidance(
    source_kind: SourceKind,
    created_at: &str,
) -> Option<&str> {
    if source_kind != SourceKind::NextRequest {
        return None;
    }
    let date = created_at.get(0..10)?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Some(date)
}

async fn finalize_and_index_extracted_document(
    context: &WorkerContext,
    request_id: u64,
    download: DocumentDownloadFinalization<'_>,
    text: String,
    was_ocr_processed: bool,
    document_created_at: Option<&str>,
) -> Result<Option<usize>, AppError> {
    let guidance = &context.crawl.guidance;
    let agency_label =
        if guidance.include_organizations.is_empty() && guidance.exclude_organizations.is_empty() {
            None
        } else {
            context
                .repository
                .agency_label_for_request(&context.portal.portal_key, request_id)
                .await?
        };
    if let Err(exclusion) = evaluate_extracted_document_guidance(
        guidance,
        &context.crawl.compiled_guidance,
        &text,
        agency_label.as_deref(),
        document_created_at,
    ) {
        // A resumed sync re-extracts every document regardless of prior completion, so this
        // document may already carry a `documents_fts` row from an earlier pass. Guidance edited
        // between syncs must not retroactively purge evidence a prior sync already finalized --
        // `delete_unextracted_document` enforces that itself and reports whether it actually
        // removed anything. When it didn't, the document stays and falls through to the normal
        // finalize path below rather than being counted as a refusal that did not happen.
        let discarded = context
            .repository
            .delete_unextracted_document(download.portal_subdomain, download.document_id)
            .await?;
        if discarded {
            record_frontier_exclusion(context, &exclusion).await;
            return Ok(None);
        }
    }
    let extracted_text_sha256 = sha256_hex(text.as_bytes());
    let character_count = text.chars().count();
    context
        .repository
        .finalize_document_extraction(DocumentExtractionFinalization {
            document_id: download.document_id,
            portal_subdomain: download.portal_subdomain,
            extracted_text: &text,
            extracted_text_sha256: &extracted_text_sha256,
            raw_file_sha256: download.raw_file_sha256,
            was_ocr_processed,
            raw_file_size: download.raw_file_size,
            local_path: download.local_path,
            shannon_entropy: download.shannon_entropy,
            entropy_sample_bytes: download.entropy_sample_bytes,
            suspicious_data_density: download.suspicious_data_density,
        })
        .await?;
    context
        .repository
        .capture_monitoring_document(download.portal_subdomain, download.document_id)
        .await?;
    index_extracted_document(
        context,
        request_id,
        download.document_id,
        download.raw_file_sha256,
        &extracted_text_sha256,
        text,
    )
    .await?;
    Ok(Some(character_count))
}
