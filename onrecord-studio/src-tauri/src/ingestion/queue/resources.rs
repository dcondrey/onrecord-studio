//! Public resources, data catalogs, ArcGIS layers, and the downloads and crawl requests they discover.

use super::*;
use super::{
    crawl::{
        admit_crawl_link, mark_crawl_url, record_frontier_exclusion, reserve_crawl_page,
        reserve_paginated_page, reserve_resource, standard_document_task,
    },
    frontier::{enqueue_task_resilient, enqueue_tasks_resilient},
    guidance::{describe_discovery_origin, download_host_allowed, guidance_allows_url},
    intake::{discovered_file_name, source_file_name, stable_source_identifier},
    objects::{release_object_link, store_inline_payload},
    structured::{
        arcgis_item_companion_urls, arcgis_layer_url, arcgis_metadata_url,
        tableau_export_companion_urls,
    },
};

pub(super) async fn process_public_resource(
    context: &WorkerContext,
    url: String,
    depth: usize,
) -> Result<(), AppError> {
    if !reserve_crawl_page(context, depth).await? {
        return Ok(());
    }
    context
        .repository
        .check_retained_file_budget(&context.download_root)
        .await?;
    let inspection = context
        .client
        .inspect_public_resource_with_selectors(
            &url,
            context.crawl.guidance.link_selector.as_deref(),
            context.crawl.guidance.download_selector.as_deref(),
        )
        .await?;
    match inspection.kind {
        PublicResourceKind::Download => {
            enqueue_discovered_download(
                context,
                &inspection,
                None,
                &url,
                PayloadDepth::discovered(depth, 0),
            )
            .await?;
        }
        PublicResourceKind::HtmlPage => {
            // A disappearance lead is created only after a successfully retrieved listing.
            // Failed requests never imply that a publisher removed a record.
            if let Some(body) = inspection.body.as_deref() {
                context
                    .repository
                    .record_publication_listing(
                        inspection.final_url.clone(),
                        sha256_hex(body),
                        crate::ingestion::client::publication_listing_links(
                            &inspection.final_url,
                            body,
                        )?,
                    )
                    .await?;
            }
            let request_id = ensure_crawl_request(context, &inspection).await?;
            let mut discovered_tasks = Vec::new();
            for companion_url in arcgis_item_companion_urls(&inspection.final_url) {
                if let Some(task) =
                    prepare_crawl_url(context, companion_url, depth.saturating_add(1), 0).await
                {
                    discovered_tasks.push(task);
                }
            }
            for companion_url in tableau_export_companion_urls(&inspection.final_url) {
                if let Some(task) =
                    prepare_crawl_url(context, companion_url, depth.saturating_add(1), 0).await
                {
                    discovered_tasks.push(task);
                }
            }
            if let Some(body) = inspection.body.as_deref() {
                for companion_url in arcgis_hub_companion_urls(&inspection.final_url, body) {
                    if let Some(task) =
                        prepare_crawl_url(context, companion_url, depth.saturating_add(1), 0).await
                    {
                        discovered_tasks.push(task);
                    }
                }
                if let Some(task) = prepare_html_page_document(
                    context,
                    &inspection,
                    request_id,
                    body,
                    PayloadDepth::discovered(depth, 0),
                )
                .await?
                {
                    discovered_tasks.push(task);
                }
            }
            for connector in inspection.embedded_connectors.iter().cloned() {
                match connector {
                    PublicEmbeddedConnector::WordPressGoogleDrive(connector) => {
                        if let Some(task) = prepare_wordpress_google_drive(
                            context,
                            connector,
                            depth.saturating_add(1),
                        )
                        .await
                        {
                            discovered_tasks.push(task);
                        }
                    }
                }
            }
            for link in inspection.links {
                if let Err(exclusion) = guidance_allows_url(
                    &context.crawl.guidance,
                    &context.crawl.compiled_guidance,
                    &link.url,
                    None,
                ) {
                    record_frontier_exclusion(context, &exclusion).await;
                    continue;
                }
                for companion_url in arcgis_item_companion_urls(&link.url) {
                    if let Some(task) =
                        prepare_crawl_url(context, companion_url, depth.saturating_add(1), 0).await
                    {
                        discovered_tasks.push(task);
                    }
                }
                for companion_url in tableau_export_companion_urls(&link.url) {
                    if let Some(task) =
                        prepare_crawl_url(context, companion_url, depth.saturating_add(1), 0).await
                    {
                        discovered_tasks.push(task);
                    }
                }
                if link.download_candidate {
                    if let Some(task) = prepare_discovered_download(
                        context,
                        &PublicResourceInspection {
                            final_url: link.url,
                            content_type: None,
                            title: link.label,
                            kind: PublicResourceKind::Download,
                            links: Vec::new(),
                            body: None,
                            embedded_connectors: Vec::new(),
                            content_length: None,
                            acquisition: None,
                        },
                        Some(request_id),
                        &inspection.final_url,
                        PayloadDepth::discovered(depth.saturating_add(1), 0),
                    )
                    .await?
                    {
                        discovered_tasks.push(task);
                    }
                } else if link.follow_candidate {
                    match admit_crawl_link(context, &link, depth).await {
                        Ok(()) => {
                            if let Some(task) = prepare_crawl_url(
                                context,
                                link.url.clone(),
                                depth.saturating_add(1),
                                0,
                            )
                            .await
                            {
                                discovered_tasks.push(task);
                            }
                        }
                        Err(exclusion) => record_frontier_exclusion(context, &exclusion).await,
                    }
                }
            }
            enqueue_tasks_resilient(context, discovered_tasks).await?;
        }
    }
    context
        .statistics
        .pages_processed
        .fetch_add(1, Ordering::AcqRel);
    Ok(())
}

pub(super) async fn process_wordpress_google_drive(
    context: &WorkerContext,
    connector: WordPressGoogleDriveConnector,
    depth: usize,
) -> Result<(), AppError> {
    if !reserve_crawl_page(context, depth).await? {
        return Ok(());
    }
    let items = context
        .client
        .list_wordpress_google_drive_folder(&connector)
        .await?;
    let request_id = stable_source_identifier(&format!(
        "google-drive-folder|{}|{}",
        connector.endpoint, connector.folder_id
    ));
    persist_crawl_request(
        context,
        request_id,
        format!("drive-{request_id}"),
        connector.folder_name.clone(),
        format!(
            "Public Google Drive folder discovered through {}",
            context.portal.canonical_url
        ),
    )
    .await?;

    let folder_origin = format!(
        "{} Google Drive folder {}",
        connector.endpoint, connector.folder_id
    );
    let mut discovered_tasks = Vec::with_capacity(items.len());
    for item in items {
        if item.is_folder {
            if depth < context.crawl.policy.maximum_depth {
                let mut child = connector.clone();
                child.folder_id = item.id;
                child.account_id = item.account_id;
                child.folder_name = item.name;
                if let Some(task) =
                    prepare_wordpress_google_drive(context, child, depth.saturating_add(1)).await
                {
                    discovered_tasks.push(task);
                }
            }
            continue;
        }
        let download_url = ApiClient::wordpress_google_drive_download_url(&connector, &item)?;
        if let Some(task) = prepare_discovered_download(
            context,
            &PublicResourceInspection {
                final_url: download_url,
                content_type: Some(item.media_type),
                title: item.name,
                kind: PublicResourceKind::Download,
                links: Vec::new(),
                body: None,
                embedded_connectors: Vec::new(),
                content_length: None,
                acquisition: None,
            },
            Some(request_id),
            &folder_origin,
            PayloadDepth::discovered(depth.saturating_add(1), 0),
        )
        .await?
        {
            discovered_tasks.push(task);
        }
    }
    enqueue_tasks_resilient(context, discovered_tasks).await?;
    context
        .statistics
        .pages_processed
        .fetch_add(1, Ordering::AcqRel);
    Ok(())
}

/// Reads one page of a platform catalog index and schedules the files it names.
///
/// This is the path that replaces crawling on a detected catalog. It never follows links: an index
/// page names assets, and every asset it names is either scheduled for download or counted as one
/// the index could not resolve to a file. Nothing in between is possible, which is what makes the
/// resulting corpus accountable to the catalog's own asset count.
pub(super) async fn process_data_catalog_page(
    context: &WorkerContext,
    endpoint: DataCatalogEndpoint,
    cursor: Option<String>,
) -> Result<(), AppError> {
    if !reserve_paginated_page(context).await? {
        return Ok(());
    }
    let page = context
        .client
        .fetch_data_catalog_page(&endpoint, cursor.as_deref())
        .await?;
    let request_id = stable_source_identifier(&format!("catalog|{}", endpoint.api_url));
    persist_crawl_request(
        context,
        request_id,
        format!("catalog-{request_id}"),
        format!(
            "{} catalog at {}",
            endpoint.platform.as_str(),
            endpoint.domain
        ),
        match page.asset_count {
            Some(count) => format!(
                "{} index reporting {count} assets",
                endpoint.platform.as_str()
            ),
            None => format!(
                "{} index reporting no asset total",
                endpoint.platform.as_str()
            ),
        },
    )
    .await?;
    let mut tasks = Vec::new();
    for download in &page.downloads {
        if let Err(exclusion) = guidance_allows_url(
            &context.crawl.guidance,
            &context.crawl.compiled_guidance,
            &download.url,
            download.byte_count,
        ) {
            record_frontier_exclusion(context, &exclusion).await;
            continue;
        }
        let inspection = PublicResourceInspection {
            final_url: download.url.clone(),
            content_type: download.media_type.clone(),
            title: download.label.clone(),
            kind: PublicResourceKind::Download,
            links: Vec::new(),
            body: None,
            embedded_connectors: Vec::new(),
            content_length: download.byte_count,
            acquisition: None,
        };
        if let Some(task) = prepare_discovered_download(
            context,
            &inspection,
            Some(request_id),
            &endpoint.api_url,
            // A catalog asset is one structured hop from the index and occupies no crawl depth,
            // because paging an index is not walking a site graph.
            PayloadDepth::discovered(0, 1),
        )
        .await?
        {
            tasks.push(task);
        }
    }
    // An asset the index could not resolve to a file is a hole in coverage, so it is counted under
    // the reason it could not be resolved rather than dropped.
    for (reason, count) in &page.assets_without_downloads {
        *context
            .crawl
            .frontier_exclusions
            .lock()
            .await
            .entry(reason.clone())
            .or_insert(0) += *count;
    }
    if page.foreign_domain_assets > 0 {
        *context
            .crawl
            .frontier_exclusions
            .lock()
            .await
            .entry("catalog-asset-outside-portal-domain".to_string())
            .or_insert(0) += page.foreign_domain_assets;
    }
    if let Some(next_cursor) = page.next_cursor {
        tasks.push(IngestionTask::FetchDataCatalogPage {
            subdomain: context.portal.portal_key.clone(),
            endpoint: endpoint.clone(),
            cursor: Some(next_cursor),
        });
    }
    enqueue_tasks_resilient(context, tasks).await?;
    let _ = context
        .progress_sender
        .send(IngestionProgress::PageCompleted {
            portal_key: context.portal.portal_key.clone(),
            page: context.crawl.pages_started.load(Ordering::Acquire),
            records_found: page.downloads.len(),
        })
        .await;
    context
        .statistics
        .pages_processed
        .fetch_add(1, Ordering::AcqRel);
    Ok(())
}

pub(super) async fn process_arcgis_layer(
    context: &WorkerContext,
    layer_url: String,
    offset: usize,
    crawl_depth: usize,
    structured_depth: usize,
) -> Result<(), AppError> {
    if !reserve_paginated_page(context).await? {
        return Ok(());
    }
    let page = context
        .client
        .fetch_arcgis_feature_page(&layer_url, offset, ARCGIS_FEATURE_PAGE_SIZE)
        .await?;
    if page.feature_count == 0 {
        if page.exceeded_transfer_limit {
            return Err(AppError::DataParsing(
                "ArcGIS reported more features but returned an empty page; coverage is incomplete."
                    .into(),
            ));
        }
        context
            .statistics
            .pages_processed
            .fetch_add(1, Ordering::AcqRel);
        return Ok(());
    }
    let page_hash = sha256_hex(&page.bytes);
    let page_identity = format!("{layer_url}|{page_hash}");
    if !context
        .crawl
        .arcgis_page_hashes
        .lock()
        .await
        .insert(page_identity)
    {
        return Err(AppError::DataParsing(
            "ArcGIS repeated a feature page without advancing; coverage is incomplete.".into(),
        ));
    }
    if !reserve_resource(context, &page.request_url).await? {
        return Ok(());
    }
    let request_id = stable_source_identifier(&format!("arcgis-layer|{layer_url}"));
    persist_crawl_request(
        context,
        request_id,
        format!("arcgis-{request_id}"),
        format!("ArcGIS layer {layer_url}"),
        "Paginated ArcGIS feature layer export".to_string(),
    )
    .await?;
    let document_id = stable_source_identifier(&format!("arcgis-page|{layer_url}|{offset}"));
    context
        .repository
        .check_retained_file_budget(&context.download_root)
        .await?;
    let managed_path =
        store_inline_payload(&context.download_root, document_id, &page.bytes).await?;
    let now = Utc::now().to_rfc3339();
    let document = DocumentMetadata {
        id: document_id,
        file_name: Some(format!("arcgis-{request_id}-{offset}.geojson")),
        file_size: Some(page.bytes.len() as u64),
        description: Some(format!(
            "ArcGIS features {} through {} from {layer_url}",
            offset,
            offset.saturating_add(page.feature_count)
        )),
        content_type: Some("application/geo+json".to_string()),
        download_url: Some(page.request_url),
        created_at: now.clone(),
        updated_at: now,
        visibility: "public".to_string(),
        local_path: Some(managed_path.to_string_lossy().into_owned()),
        extracted_text_sha256: None,
        was_ocr_processed: false,
    };
    record_inline_acquisition(
        context,
        request_id,
        &document,
        &page.bytes,
        &page.acquisition,
    )
    .await?;
    let mut tasks = vec![standard_document_task(
        context,
        request_id,
        document,
        PayloadDepth::discovered(crawl_depth, structured_depth),
    )];
    if page.exceeded_transfer_limit || page.feature_count == ARCGIS_FEATURE_PAGE_SIZE {
        let next_offset = offset.checked_add(page.feature_count).ok_or_else(|| {
            AppError::PortalCapacity("ArcGIS feature offset overflowed".to_string())
        })?;
        if next_offset <= 100_000_000 {
            tasks.push(IngestionTask::CrawlArcGisLayer {
                subdomain: context.portal.portal_key.clone(),
                layer_url,
                offset: next_offset,
                crawl_depth,
                structured_depth,
            });
        }
    }
    enqueue_tasks_resilient(context, tasks).await?;
    context
        .statistics
        .pages_processed
        .fetch_add(1, Ordering::AcqRel);
    Ok(())
}

async fn prepare_wordpress_google_drive(
    context: &WorkerContext,
    connector: WordPressGoogleDriveConnector,
    depth: usize,
) -> Option<IngestionTask> {
    let identity = format!(
        "wordpress-google-drive|{}|{}",
        connector.endpoint, connector.folder_id
    );
    if !mark_crawl_url(context, &identity).await {
        return None;
    }
    Some(IngestionTask::CrawlWordPressGoogleDrive {
        subdomain: context.portal.portal_key.clone(),
        connector,
        depth,
    })
}

async fn prepare_html_page_document(
    context: &WorkerContext,
    inspection: &PublicResourceInspection,
    request_id: u64,
    body: &[u8],
    depth: PayloadDepth,
) -> Result<Option<IngestionTask>, AppError> {
    if !reserve_resource(context, &inspection.final_url).await? {
        return Ok(None);
    }
    let document_id = stable_source_identifier(&format!("page|{}", inspection.final_url));
    let managed_path = store_inline_payload(&context.download_root, document_id, body).await?;
    let mut file_name = source_file_name(&inspection.final_url, document_id);
    let lower_file_name = file_name.to_ascii_lowercase();
    if ![".html", ".htm", ".xhtml"]
        .iter()
        .any(|extension| lower_file_name.ends_with(extension))
    {
        file_name.push_str(".html");
    }
    let now = Utc::now().to_rfc3339();
    let document = DocumentMetadata {
        id: document_id,
        file_name: Some(file_name),
        file_size: Some(body.len() as u64),
        description: Some(format!("Crawled HTML page: {}", inspection.final_url)),
        // The server's declared type, or none. The `.html` file name already routes extraction;
        // asserting `text/html` for a body the server never described would be a claim the
        // record does not support, and the stored type gates PDF-only detectors downstream.
        content_type: inspection.content_type.clone(),
        download_url: Some(inspection.final_url.clone()),
        created_at: now.clone(),
        updated_at: now,
        visibility: "public".to_string(),
        local_path: Some(managed_path.to_string_lossy().into_owned()),
        extracted_text_sha256: None,
        was_ocr_processed: false,
    };
    if let Some(acquisition) = &inspection.acquisition {
        record_inline_acquisition(context, request_id, &document, body, acquisition).await?;
    }
    Ok(Some(standard_document_task(
        context, request_id, document, depth,
    )))
}

/// A page fetched and stored inline is already an acquisition before its document task runs, and
/// that task reads the stored bytes as cached. The row is written first so the event always has a
/// document to belong to; the task's own write of the same metadata is an idempotent upsert.
async fn record_inline_acquisition(
    context: &WorkerContext,
    request_id: u64,
    document: &DocumentMetadata,
    bytes: &[u8],
    acquisition: &AcquisitionRecord,
) -> Result<(), AppError> {
    context
        .repository
        .insert_or_update_document(&context.portal.portal_key, request_id, document)
        .await?;
    if let Some(path) = &document.local_path {
        release_object_link(Path::new(path)).await;
    }
    context
        .repository
        .record_document_acquisition(
            &context.portal.portal_key,
            document.id,
            Some(&context.sync_run_id),
            &sha256_hex(bytes),
            bytes.len() as u64,
            acquisition,
        )
        .await
}

async fn ensure_crawl_request(
    context: &WorkerContext,
    inspection: &PublicResourceInspection,
) -> Result<u64, AppError> {
    let request_id = stable_source_identifier(&inspection.final_url);
    persist_crawl_request(
        context,
        request_id,
        format!("web-{request_id}"),
        inspection.title.clone(),
        format!("Crawled public source page: {}", inspection.final_url),
    )
    .await?;
    Ok(request_id)
}

async fn persist_crawl_request(
    context: &WorkerContext,
    request_id: u64,
    rpid: String,
    title: String,
    description: String,
) -> Result<(), AppError> {
    let now = Utc::now().to_rfc3339();
    let mut request = PublicRequest {
        id: request_id,
        rpid: Some(rpid),
        title: Some(title),
        description: Some(description),
        status: "published".to_string(),
        current_status_id: None,
        created_at: now.clone(),
        updated_at: now,
        closed_at: None,
        due_date: None,
        determination_at: None,
        determination_source: None,
        neighborhood_block: None,
        statutory_extension: None,
        documents: Vec::new(),
        agency: None,
        category: None,
        timeline_anomalies: Vec::new(),
    };
    request.compute_timeline_anomalies()?;
    context
        .repository
        .insert_or_update_request(&context.portal.portal_key, &request)
        .await?;
    context
        .statistics
        .requests_processed
        .fetch_add(1, Ordering::AcqRel);
    Ok(())
}

async fn enqueue_discovered_download(
    context: &WorkerContext,
    inspection: &PublicResourceInspection,
    parent_request_id: Option<u64>,
    discovered_from: &str,
    depth: PayloadDepth,
) -> Result<(), AppError> {
    let Some(task) = prepare_discovered_download(
        context,
        inspection,
        parent_request_id,
        discovered_from,
        depth,
    )
    .await?
    else {
        return Ok(());
    };
    enqueue_task_resilient(context, task).await
}

pub(super) async fn prepare_discovered_download(
    context: &WorkerContext,
    inspection: &PublicResourceInspection,
    parent_request_id: Option<u64>,
    discovered_from: &str,
    depth: PayloadDepth,
) -> Result<Option<IngestionTask>, AppError> {
    // Before `reserve_resource`, so a refused host does not spend a resource slot, and reported
    // rather than dropped: a crawl that silently declines documents claims coverage it does not
    // have. This is the only place a remote URL becomes a fetch task -- the other
    // `DownloadBinaryPayload` births carry bytes already on disk (`local_path`) or come from the
    // source's own metadata API.
    if !download_host_allowed(
        &context.crawl.guidance,
        &context.portal.canonical_url,
        &inspection.final_url,
    ) {
        record_frontier_exclusion(context, &FrontierExclusion::OffSourceHostDownload).await;
        return Ok(None);
    }
    if !reserve_resource(context, &inspection.final_url).await? {
        return Ok(None);
    }
    let request_id = match parent_request_id {
        Some(request_id) => request_id,
        None => ensure_crawl_request(context, inspection).await?,
    };
    let document_id = stable_source_identifier(&format!("document|{}", inspection.final_url));
    let now = Utc::now().to_rfc3339();
    let document = DocumentMetadata {
        id: document_id,
        file_name: Some(discovered_file_name(inspection, document_id)),
        file_size: None,
        description: Some(describe_discovery_origin(
            discovered_from,
            &inspection.final_url,
        )),
        content_type: inspection.content_type.clone(),
        download_url: Some(inspection.final_url.clone()),
        created_at: now.clone(),
        updated_at: now,
        visibility: "public".to_string(),
        local_path: None,
        extracted_text_sha256: None,
        was_ocr_processed: false,
    };
    Ok(Some(standard_document_task(
        context, request_id, document, depth,
    )))
}

pub(super) async fn prepare_crawl_url(
    context: &WorkerContext,
    url: String,
    depth: usize,
    structured_depth: usize,
) -> Option<IngestionTask> {
    // CKAN and Socrata are detected before this path and use their native paginated catalog task;
    // ArcGIS uses the layer task below. Other public catalogs remain bounded generic discovery,
    // while credentialed SharePoint/HUD tenant systems are manual-import-only by policy.
    if let Some(layer_url) = arcgis_layer_url(&url) {
        let identity = format!("arcgis-layer|{layer_url}");
        if !mark_crawl_url(context, &identity).await {
            return None;
        }
        return Some(IngestionTask::CrawlArcGisLayer {
            subdomain: context.portal.portal_key.clone(),
            layer_url,
            offset: 0,
            crawl_depth: depth,
            structured_depth,
        });
    }
    let url = arcgis_metadata_url(&url).unwrap_or(url);
    if !mark_crawl_url(context, &url).await {
        return None;
    }
    Some(IngestionTask::CrawlPublicResource {
        subdomain: context.portal.portal_key.clone(),
        url,
        depth,
    })
}
