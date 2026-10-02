//! Crawl admission: path identity, host scope, frontier exclusions, and page and resource budgets.

use super::*;
use super::{
    frontier::enqueue_tasks_resilient,
    guidance::{
        guidance_allows_url, guidance_explicitly_allows_host, is_supported_external_data_platform,
        source_host_matches, validate_page_parameters,
    },
    worker::send_progress,
};

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
    let response = context
        .client
        .fetch_requests(&context.portal, page, per_page)
        .await?;
    let record_count = response.requests.len();
    let processed_requests = u64::try_from(record_count).map_err(|error| {
        AppError::Internal(format!("Request page count cannot be represented: {error}"))
    })?;
    let mut discovered_tasks = Vec::new();
    for mut record in response.requests {
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
            });
        }
    }
    enqueue_tasks_resilient(context, discovered_tasks).await?;
    Ok(())
}
