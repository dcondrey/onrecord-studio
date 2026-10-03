//! Crawl admission: path identity, host scope, frontier exclusions, and page and resource budgets.

use super::intake::stable_source_identifier;
use super::objects::store_inline_payload;
use super::*;
use super::{
    frontier::enqueue_tasks_resilient,
    guidance::{
        guidance_allows_url, guidance_explicitly_allows_host, is_supported_external_data_platform,
        source_host_matches, subject_searches, validate_page_parameters,
    },
    relevance::admitted,
    worker::send_progress,
};
use crate::ingestion::hosted_nextrequest;

/// Claims the site path a crawl node occupies, refusing a second URL that reaches the same path
/// with a different query string, except the known San Diego policy category/page keys.
///
/// The first URL to reach a path wins, which is normally the one linked from the seed. Later
/// query variants are the faceted-search shape: `/dataset/` and `/dataset/?tags=species` are the
/// same listing under a filter, and following every filter combination is what made the queue
/// unbounded on data.ca.gov.
///
/// Scoped to site crawling on purpose. Structured expansion is untouched, because a catalog API
/// legitimately addresses distinct records through the query string
/// (`/api/3/action/package_show?id=...`), and that chain is already bounded by
/// `MAXIMUM_STRUCTURED_CHAIN`.
pub(super) async fn claim_crawl_path(context: &WorkerContext, url: &str) -> bool {
    context
        .crawl
        .crawl_path_claims
        .lock()
        .await
        .insert(crawl_path_identity(url))
}

/// The site path a crawl node occupies: scheme, host, and path with the query dropped and a
/// trailing slash normalized away, so `/dataset`, `/dataset/`, and `/dataset/?tags=species` are
/// one identity.
pub(super) fn crawl_path_identity(url: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return url.to_string();
    };
    if let Some(identity) = crate::ingestion::client::san_diego_policy_listing_identity(&parsed) {
        return identity;
    }
    if let Some(identity) = crate::ingestion::regional::meeting_identity(&parsed) {
        return identity;
    }
    format!(
        "{}://{}{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or_default(),
        parsed.path().trim_end_matches('/')
    )
}

/// Announces, once, that the run has stopped contacting the host a failed task targeted.
pub(super) async fn announce_host_stop(context: &WorkerContext, target_id: &str) {
    let Ok(url) = reqwest::Url::parse(target_id) else {
        return;
    };
    let governor = context.client.host_governor();
    let Some(reason) = governor.stop_reason(&url) else {
        return;
    };
    let host = crate::ingestion::host_policy::HostGovernor::host_key(&url);
    if !context
        .crawl
        .announced_host_stops
        .lock()
        .await
        .insert(host.clone())
    {
        return;
    }
    let requests_spent = governor
        .snapshots()
        .into_iter()
        .find(|snapshot| snapshot.host == host)
        .map_or(0, |snapshot| snapshot.requests_spent);
    let _ = context
        .progress_sender
        .send(IngestionProgress::HostStopped {
            portal_key: context.portal.portal_key.clone(),
            host,
            requests_spent,
            reason: reason.describe(),
        })
        .await;
}

pub(super) async fn record_frontier_exclusion(
    context: &WorkerContext,
    exclusion: &FrontierExclusion,
) {
    *context
        .crawl
        .frontier_exclusions
        .lock()
        .await
        .entry(exclusion.rule().to_string())
        .or_insert(0) += 1;
}

/// Decides whether a link discovered on an HTML page becomes a crawl node, naming the rule that
/// refused it when it does not.
pub(super) async fn admit_crawl_link(
    context: &WorkerContext,
    link: &PublicResourceLink,
    depth: usize,
) -> Result<(), FrontierExclusion> {
    let url = link.url.as_str();
    guidance_allows_url(
        &context.crawl.guidance,
        &context.crawl.compiled_guidance,
        url,
        None,
    )?;
    if link.navigation_chrome {
        return Err(FrontierExclusion::NavigationChrome);
    }
    if depth >= context.crawl.policy.maximum_depth {
        return Err(FrontierExclusion::DepthCeiling);
    }
    if let Ok(parsed) = reqwest::Url::parse(url) {
        let governor = context.client.host_governor();
        // The cached rules if this host's robots.txt has already been read; `None` while it is
        // still being fetched, in which case the same check runs again at dispatch. Reading them
        // here is what keeps a robots exclusion counted under its own rule instead of surfacing
        // as a dispatch failure.
        let cell = governor.robots_cell(&parsed);
        let rules = cell.get().cloned();
        if let Err(refusal) = governor.admits(&parsed, rules.as_deref()) {
            return Err(match refusal {
                RequestRefusal::RobotsExcluded { rule, .. } => {
                    FrontierExclusion::RobotsExcluded { rule }
                }
                refusal => FrontierExclusion::HostStopped {
                    detail: refusal.describe(),
                },
            });
        }
    }
    if !crawl_host_in_scope(
        &context.portal.canonical_url,
        &context.crawl.guidance,
        &context.crawl.policy,
        url,
    ) {
        return Err(FrontierExclusion::OffScopeHost);
    }
    if !claim_crawl_path(context, url).await {
        return Err(FrontierExclusion::QueryVariant {
            path: crawl_path_identity(url),
        });
    }
    Ok(())
}

/// The host-scope rule every crawl node must satisfy, whether the link was found in HTML or in a
/// structured payload: the source's own host, a host the operator named in `allowed_hostnames`,
/// or a supported external data platform when the policy opts in. Downloads are governed
/// separately by `download_host_allowed`, because real portals serve documents from storage hosts
/// this rule does not enumerate.
pub(super) fn crawl_host_in_scope(
    canonical_url: &str,
    guidance: &SourceGuidance,
    policy: &CrawlPolicy,
    url: &str,
) -> bool {
    source_host_matches(canonical_url, url)
        || guidance_explicitly_allows_host(guidance, url)
        || (policy.follow_supported_external_platforms && is_supported_external_data_platform(url))
}

pub(super) async fn mark_crawl_url(context: &WorkerContext, url: &str) -> bool {
    let mut visited = context.crawl.visited_urls.lock().await;
    insert_bounded_identity(&mut visited, url, context.crawl.policy.maximum_pages)
}

async fn mark_resource_url(context: &WorkerContext, url: &str) -> bool {
    let mut resources = context.crawl.resource_urls.lock().await;
    insert_bounded_identity(&mut resources, url, context.crawl.policy.maximum_resources)
}

pub(super) fn insert_bounded_identity(
    identities: &mut HashSet<String>,
    value: &str,
    limit: usize,
) -> bool {
    if identities.contains(value) || identities.len() >= limit {
        return false;
    }
    identities.insert(value.to_string())
}

pub(super) async fn reserve_crawl_page(
    context: &WorkerContext,
    depth: usize,
) -> Result<bool, AppError> {
    if depth > context.crawl.policy.maximum_depth {
        return Ok(false);
    }
    reserve_paginated_page(context).await
}

/// The page budget and rate limit without the depth check, for walking one resource's pages.
/// Paginating a single ArcGIS feature layer is not descending the site, so it must not spend
/// `maximum_depth`; `maximum_pages`, the per-layer page-hash dedup and the 100M offset ceiling
/// bound it instead.
pub(super) async fn reserve_paginated_page(context: &WorkerContext) -> Result<bool, AppError> {
    let Some(page_number) = context
        .repository
        .reserve_ingestion_crawl_page(
            &context.sync_run_id,
            &context.portal.portal_key,
            context.crawl.policy.maximum_pages,
        )
        .await?
    else {
        return Ok(false);
    };
    let page_number = usize::try_from(page_number).map_err(|_| {
        AppError::PortalCapacity("Crawl page counter exceeds platform capacity".to_string())
    })?;
    context
        .crawl
        .pages_started
        .fetch_max(page_number, Ordering::AcqRel);
    context.limiter.acquire(&context.portal.portal_key).await?;
    Ok(true)
}

pub(super) async fn reserve_resource(context: &WorkerContext, url: &str) -> Result<bool, AppError> {
    if !mark_resource_url(context, url).await {
        return Ok(false);
    }
    let Some(resources_started) = context
        .repository
        .reserve_ingestion_resource(
            &context.sync_run_id,
            &context.portal.portal_key,
            context.crawl.policy.maximum_resources,
        )
        .await?
    else {
        return Ok(false);
    };
    let resources_started = usize::try_from(resources_started).map_err(|_| {
        AppError::PortalCapacity("Resource counter exceeds platform capacity".to_string())
    })?;
    context
        .crawl
        .resources_started
        .fetch_max(resources_started, Ordering::AcqRel);
    Ok(true)
}

pub(super) fn standard_document_task(
    context: &WorkerContext,
    request_id: u64,
    document: DocumentMetadata,
    depth: PayloadDepth,
) -> IngestionTask {
    let output_directory = context
        .download_root
        .join(format!("{}_documents", context.portal.portal_key));
    IngestionTask::DownloadBinaryPayload {
        subdomain: context.portal.portal_key.clone(),
        request_id,
        document: Box::new(document),
        output_directory: output_directory.to_string_lossy().into_owned(),
        container_depth: depth.container,
        container_root_directory: None,
        crawl_depth: depth.crawl,
        structured_depth: depth.structured,
    }
}

pub(super) async fn fetch_metadata_page(
    context: &WorkerContext,
    page: usize,
    per_page: usize,
    search: Option<String>,
) -> Result<(), AppError> {
    validate_page_parameters(page, per_page)?;
    context.limiter.acquire(&context.portal.portal_key).await?;
    send_progress(
        context,
        IngestionProgress::PageStarted {
            portal_key: context.portal.portal_key.clone(),
            page,
        },
    )
    .await;
    if context.portal.api_layout.page_parameter == hosted_nextrequest::PAGE_PARAMETER {
        return list_hosted_page(context, page, per_page, search).await;
    }
    let response = context
        .client
        .fetch_requests(&context.portal, page, per_page)
        .await?;
    let record_count = response.requests.len();
    // Any request API other than hosted NextRequest is filtered here on what each request says
    // it is about; hosted NextRequest is searched for the subject server-side.
    let on_subject = {
        let candidates: Vec<String> = response
            .requests
            .iter()
            .map(|record| {
                let description: String = record
                    .description
                    .as_deref()
                    .unwrap_or_default()
                    .chars()
                    .take(400)
                    .collect();
                format!(
                    "{} {description}",
                    record.title.as_deref().unwrap_or_default()
                )
            })
            .collect();
        admitted(context.crawl.subject.as_ref(), &candidates).await
    };
    let requests: Vec<_> = response
        .requests
        .into_iter()
        .zip(on_subject)
        .filter_map(|(record, on_subject)| on_subject.then_some(record))
        .collect();
    let processed_requests = u64::try_from(requests.len()).map_err(|error| {
        AppError::Internal(format!("Request page count cannot be represented: {error}"))
    })?;
    let mut discovered_tasks = Vec::new();
    for mut record in requests {
        record.compute_timeline_anomalies()?;
        context
            .repository
            .insert_or_update_request(&context.portal.portal_key, &record)
            .await?;
        for document in record.documents {
            discovered_tasks.push(standard_document_task(
                context,
                record.id,
                document,
                PayloadDepth::discovered(0, 0),
            ));
        }
    }
    send_progress(
        context,
        IngestionProgress::PageCompleted {
            portal_key: context.portal.portal_key.clone(),
            page,
            records_found: record_count,
        },
    )
    .await;
    context
        .statistics
        .pages_processed
        .fetch_add(1, Ordering::AcqRel);
    context
        .statistics
        .requests_processed
        .fetch_add(processed_requests, Ordering::AcqRel);
    if record_count > 0 {
        let next_page = response
            .meta
            .as_ref()
            .and_then(|meta| meta.next_page)
            .or_else(|| (record_count == per_page).then_some(page.saturating_add(1)));
        if let Some(next_page) =
            next_page.filter(|next| *next > page && *next <= MAXIMUM_METADATA_PAGE)
        {
            discovered_tasks.push(IngestionTask::FetchMetadataPage {
                subdomain: context.portal.portal_key.clone(),
                page: next_page,
                per_page,
                search: None,
            });
        }
    }
    enqueue_tasks_resilient(context, discovered_tasks).await?;
    Ok(())
}

/// A hosted NextRequest listing page, searched for the subject. Each listed request becomes its
/// own durable task, so a request's dates and documents are saved as they are read, and one that
/// fails is retried alone rather than with the hundred listed beside it.
async fn list_hosted_page(
    context: &WorkerContext,
    page: usize,
    per_page: usize,
    search: Option<String>,
) -> Result<(), AppError> {
    let searches = context
        .crawl
        .guidance
        .subject
        .as_deref()
        .map(subject_searches)
        .unwrap_or_default();
    // A subject naming several searches ("encampment, safe parking") runs each one; a request
    // two searches both find is queued once, because its task identifier is the request's.
    if search.is_none() && searches.len() > 1 {
        let tasks = searches
            .into_iter()
            .map(|search| IngestionTask::FetchMetadataPage {
                subdomain: context.portal.portal_key.clone(),
                page,
                per_page,
                search: Some(search),
            })
            .collect();
        return enqueue_tasks_resilient(context, tasks).await;
    }
    let search = search.or_else(|| searches.into_iter().next());
    let listed = context
        .client
        .list_hosted_nextrequest_page(&context.portal, page, search.as_deref())
        .await?;
    let records_found = listed.requests.len();
    let mut tasks: Vec<IngestionTask> = listed
        .requests
        .into_iter()
        .map(|request| IngestionTask::FetchHostedRequest {
            subdomain: context.portal.portal_key.clone(),
            request,
        })
        .collect();
    if let Some(next_page) = listed
        .meta
        .next_page
        .filter(|next| *next > page && *next <= MAXIMUM_METADATA_PAGE)
    {
        tasks.push(IngestionTask::FetchMetadataPage {
            subdomain: context.portal.portal_key.clone(),
            page: next_page,
            per_page,
            search: search.clone(),
        });
    }
    send_progress(
        context,
        IngestionProgress::PageCompleted {
            portal_key: context.portal.portal_key.clone(),
            page,
            records_found,
        },
    )
    .await;
    context
        .statistics
        .pages_processed
        .fetch_add(1, Ordering::AcqRel);
    enqueue_tasks_resilient(context, tasks).await
}

/// One hosted NextRequest request: its dates and public documents, stored, with each document
/// queued for download.
pub(super) async fn fetch_hosted_request(
    context: &WorkerContext,
    request: hosted_nextrequest::ListedRequest,
) -> Result<(), AppError> {
    context.limiter.acquire(&context.portal.portal_key).await?;
    let (mut record, closure_notice, releases) = context
        .client
        .fetch_hosted_request(&context.portal, request)
        .await?;
    record.compute_timeline_anomalies()?;
    context
        .repository
        .insert_or_update_request(&context.portal.portal_key, &record)
        .await?;
    context
        .repository
        .replace_release_events(&context.portal.portal_key, record.id, releases)
        .await?;
    context
        .statistics
        .requests_processed
        .fetch_add(1, Ordering::AcqRel);
    let request_id = record.id;
    let mut tasks: Vec<IngestionTask> = std::mem::take(&mut record.documents)
        .into_iter()
        .map(|document| {
            standard_document_task(
                context,
                request_id,
                document,
                PayloadDepth::discovered(0, 0),
            )
        })
        .collect();
    if let (Some(notice), Some(closed_at)) = (closure_notice, record.closed_at.as_deref()) {
        tasks.push(closure_notice_task(context, &record, closed_at, &notice).await?);
    }
    enqueue_tasks_resilient(context, tasks).await
}

/// The agency's closing response ("No responsive documents ...") is its answer to the request,
/// so it is kept as a document of the request, where denial and boilerplate detectors read it. It
/// is labelled as the portal's closure notice, not as a released record.
async fn closure_notice_task(
    context: &WorkerContext,
    record: &PublicRequest,
    closed_at: &str,
    notice: &str,
) -> Result<IngestionTask, AppError> {
    let rpid = record.rpid.as_deref().unwrap_or_default();
    let document_id = stable_source_identifier(&format!(
        "nextrequest-closure|{}|{rpid}",
        context.portal.portal_key
    ));
    let managed_path =
        store_inline_payload(&context.download_root, document_id, notice.as_bytes()).await?;
    let document = DocumentMetadata {
        id: document_id,
        file_name: Some(format!("closure-notice-{rpid}.txt")),
        file_size: Some(notice.len() as u64),
        description: Some(format!("NextRequest closure notice for request {rpid}")),
        content_type: None,
        download_url: None,
        created_at: closed_at.to_string(),
        updated_at: closed_at.to_string(),
        visibility: "public".to_string(),
        local_path: Some(managed_path.to_string_lossy().into_owned()),
        extracted_text_sha256: None,
        was_ocr_processed: false,
    };
    Ok(standard_document_task(
        context,
        record.id,
        document,
        PayloadDepth::discovered(0, 0),
    ))
}
