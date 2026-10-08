//! Link discovery inside structured documents and the ArcGIS, Tableau, and catalog companion URLs.

use super::*;
use super::{
    crawl::{crawl_host_in_scope, mark_crawl_url, record_frontier_exclusion},
    frontier::enqueue_tasks_resilient,
    guidance::{guidance_allows_url, may_expand_structured_links},
    intake::has_likely_document_extension,
    resources::{prepare_crawl_url, prepare_discovered_download},
};

#[derive(Clone)]
pub(super) struct StructuredLinkCandidate {
    pub(super) url: String,
    pub(super) label: String,
    pub(super) media_type: Option<String>,
    pub(super) download_candidate: bool,
}

pub(super) struct StructuredDocumentDiscovery {
    pub(super) candidates: Vec<StructuredLinkCandidate>,
    pub(super) next_page: Option<String>,
    pub(super) companion_urls: Vec<String>,
    pub(super) layer_urls: Vec<String>,
}

pub(super) async fn expand_structured_document_links(
    context: &WorkerContext,
    request_id: u64,
    document: &DocumentMetadata,
    local_path: &Path,
    depth: PayloadDepth,
) -> Result<(), AppError> {
    if !document_is_json(document) {
        return Ok(());
    }
    // Discovered downloads never pass through `reserve_crawl_page`, so before this the chain
    // JSON -> download -> JSON honored no depth ceiling at all. It was still finite, bounded by
    // `maximum_resources` and `maximum_pages`, but nothing capped the chain itself. The cap is
    // its own axis so that a catalog deep in a portal keeps yielding documents.
    if !may_expand_structured_links(depth.structured) {
        return Ok(());
    }
    let metadata = tokio::fs::metadata(local_path).await.map_err(|error| {
        AppError::Storage(format!(
            "Failed to inspect structured source '{}': {error}",
            local_path.display()
        ))
    })?;
    if metadata.len() > MAXIMUM_STRUCTURED_DISCOVERY_BYTES {
        return Ok(());
    }
    let bytes = tokio::fs::read(local_path).await.map_err(|error| {
        AppError::Storage(format!(
            "Failed to read structured source '{}': {error}",
            local_path.display()
        ))
    })?;
    let source_url = document.download_url.clone();
    // Parsing a 64 MB JSON value and recursively walking it are CPU/allocation work. Tauri IPC
    // shares this Tokio runtime, so doing either directly in this future can stall unrelated UI
    // commands even though the file read itself is asynchronous.
    let Some(discovery) = spawn_blocking("Structured document discovery", move || {
        Ok(discover_structured_document_links(
            &bytes,
            source_url.as_deref(),
        ))
    })
    .await??
    else {
        return Ok(());
    };
    let mut tasks = Vec::with_capacity(
        discovery.candidates.len()
            + discovery.companion_urls.len()
            + discovery.layer_urls.len()
            + usize::from(discovery.next_page.is_some()),
    );
    // Paging one search result set revisits a single resource, so it holds its own depth and is
    // bounded by `maximum_pages` and `visited_urls` instead.
    if let Some(next_page) = discovery.next_page {
        if let Some(task) =
            prepare_crawl_url(context, next_page, depth.crawl, depth.structured).await
        {
            tasks.push(task);
        }
    }
    for companion_url in discovery.companion_urls {
        if let Some(task) = prepare_crawl_url(
            context,
            companion_url,
            depth.crawl.saturating_add(1),
            depth.structured.saturating_add(1),
        )
        .await
        {
            tasks.push(task);
        }
    }
    for layer_url in discovery.layer_urls {
        let identity = format!("arcgis-layer|{layer_url}");
        if mark_crawl_url(context, &identity).await {
            tasks.push(IngestionTask::CrawlArcGisLayer {
                subdomain: context.portal.portal_key.clone(),
                layer_url,
                offset: 0,
                crawl_depth: depth.crawl.saturating_add(1),
                structured_depth: depth.structured.saturating_add(1),
            });
        }
    }
    let discovered_from = document
        .download_url
        .clone()
        .unwrap_or_else(|| format!("local document {}", document.id));
    for candidate in discovery.candidates {
        if let Err(exclusion) = guidance_allows_url(
            &context.crawl.guidance,
            &context.crawl.compiled_guidance,
            &candidate.url,
            None,
        ) {
            record_frontier_exclusion(context, &exclusion).await;
            continue;
        }
        if candidate.download_candidate {
            let inspection = PublicResourceInspection {
                final_url: candidate.url,
                content_type: candidate.media_type,
                title: candidate.label,
                kind: PublicResourceKind::Download,
                links: Vec::new(),
                body: None,
                embedded_connectors: Vec::new(),
                content_length: None,
                acquisition: None,
            };
            if let Some(task) = prepare_discovered_download(
                context,
                &inspection,
                Some(request_id),
                &discovered_from,
                PayloadDepth::discovered(
                    depth.crawl.saturating_add(1),
                    depth.structured.saturating_add(1),
                ),
            )
            .await?
            {
                tasks.push(task);
            }
        } else {
            // A JSON payload is content the remote site chose, exactly like an HTML page, so a
            // crawl node it names is held to the host rule `admit_crawl_link` applies to HTML
            // links. Without this, `{"type":"dataset","url":"https://anywhere/"}` in any JSON
            // the crawl reads became a `CrawlPublicResource` on that host, and the page fetched
            // there was recorded as a document of this source.
            if !crawl_host_in_scope(
                &context.portal.canonical_url,
                &context.crawl.guidance,
                &context.crawl.policy,
                &candidate.url,
            ) {
                record_frontier_exclusion(context, &FrontierExclusion::OffScopeHost).await;
                continue;
            }
            if let Some(task) = prepare_crawl_url(
                context,
                candidate.url,
                depth.crawl.saturating_add(1),
                depth.structured.saturating_add(1),
            )
            .await
            {
                tasks.push(task);
            }
        }
    }
    enqueue_tasks_resilient(context, tasks).await
}

fn document_is_json(document: &DocumentMetadata) -> bool {
    document.content_type.as_deref().is_some_and(|value| {
        let value = value.split(';').next().unwrap_or(value).trim();
        value == "application/json" || value == "application/ld+json" || value.ends_with("+json")
    }) || document.file_name.as_deref().is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        [".json", ".jsonld"]
            .iter()
            .any(|extension| name.ends_with(extension))
    })
}

/// Parses and walks one structured payload without retaining the potentially large JSON tree in
/// the async worker. The caller runs this function on the blocking executor and receives only the
/// bounded link/layer summaries needed for asynchronous policy checks and task scheduling.
pub(super) fn discover_structured_document_links(
    bytes: &[u8],
    source_url: Option<&str>,
) -> Option<StructuredDocumentDiscovery> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes).ok()?;
    let base_url = source_url.and_then(|value| reqwest::Url::parse(value).ok());
    let mut candidates = HashMap::new();
    collect_structured_link_candidates(&value, base_url.as_ref(), 0, None, &mut candidates);
    let mut candidates = candidates.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.url.cmp(&right.url));
    Some(StructuredDocumentDiscovery {
        candidates,
        next_page: arcgis_search_next_page(source_url, &value),
        companion_urls: arcgis_structured_item_companion_urls(&value),
        layer_urls: arcgis_layers_from_metadata(source_url, &value),
    })
}

pub(super) fn collect_structured_link_candidates(
    value: &serde_json::Value,
    base_url: Option<&reqwest::Url>,
    depth: usize,
    inherited_label: Option<&str>,
    candidates: &mut HashMap<String, StructuredLinkCandidate>,
) {
    if depth > 64 || candidates.len() >= MAXIMUM_STRUCTURED_DISCOVERY_LINKS {
        return;
    }
    match value {
        serde_json::Value::Object(object) => {
            let local_label = ["title", "name", "identifier"]
                .into_iter()
                .find_map(|key| object.get(key).and_then(serde_json::Value::as_str))
                .or(inherited_label)
                .unwrap_or("Structured data resource");
            let declared_format = object
                .get("format")
                .and_then(serde_json::Value::as_str)
                .and_then(crate::ingestion::client::catalog_format_media_type);
            let media_type = ["mediaType", "contentType", "mimeType", "mimetype"]
                .into_iter()
                .find_map(|key| object.get(key).and_then(serde_json::Value::as_str))
                .or(declared_format);
            let object_is_data_descriptor = object
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| {
                    let value = value.to_ascii_lowercase();
                    value.contains("service")
                        || value.contains("dataset")
                        || value.contains("csv")
                        || value.contains("geojson")
                });
            for (key, nested) in object {
                if let Some(raw_url) = nested.as_str() {
                    let normalized_key = key
                        .chars()
                        .filter(|character| character.is_ascii_alphanumeric())
                        .flat_map(char::to_lowercase)
                        .collect::<String>();
                    let declared_download = matches!(
                        normalized_key.as_str(),
                        "downloadurl" | "downloaduri" | "contenturl" | "fileurl" | "describedby"
                    ) || (normalized_key == "url"
                        && declared_format.is_some());
                    let declared_access = matches!(
                        normalized_key.as_str(),
                        "accessurl" | "endpointurl" | "apiurl" | "serviceurl"
                    );
                    let generic_link = matches!(normalized_key.as_str(), "url" | "href");
                    if declared_download || declared_access || generic_link {
                        if let Some(url) = resolve_structured_url(base_url, raw_url) {
                            let extension_download = has_likely_document_extension(url.path());
                            if declared_download
                                || declared_access
                                || extension_download
                                || (generic_link && object_is_data_descriptor)
                            {
                                let canonical = url.to_string();
                                let download_candidate = declared_download || extension_download;
                                candidates
                                    .entry(canonical.clone())
                                    .and_modify(|candidate| {
                                        candidate.download_candidate |= download_candidate;
                                        if candidate.media_type.is_none() {
                                            candidate.media_type = media_type.map(str::to_string);
                                        }
                                    })
                                    .or_insert_with(|| StructuredLinkCandidate {
                                        url: canonical,
                                        label: local_label.chars().take(256).collect(),
                                        media_type: media_type.map(str::to_string),
                                        download_candidate,
                                    });
                            }
                        }
                    }
                }
                collect_structured_link_candidates(
                    nested,
                    base_url,
                    depth.saturating_add(1),
                    Some(local_label),
                    candidates,
                );
                if candidates.len() >= MAXIMUM_STRUCTURED_DISCOVERY_LINKS {
                    return;
                }
            }
        }
        serde_json::Value::Array(values) => {
            for nested in values {
                collect_structured_link_candidates(
                    nested,
                    base_url,
                    depth.saturating_add(1),
                    inherited_label,
                    candidates,
                );
                if candidates.len() >= MAXIMUM_STRUCTURED_DISCOVERY_LINKS {
                    return;
                }
            }
        }
        _ => {}
    }
}

fn resolve_structured_url(base_url: Option<&reqwest::Url>, value: &str) -> Option<reqwest::Url> {
    if value.is_empty() || value.len() > 4_096 {
        return None;
    }
    let mut url = reqwest::Url::parse(value)
        .ok()
        .or_else(|| base_url?.join(value).ok())?;
    url.set_fragment(None);
    (url.scheme() == "https").then_some(url)
}

pub(super) fn arcgis_item_companion_urls(value: &str) -> Vec<String> {
    let Ok(url) = reqwest::Url::parse(value) else {
        return Vec::new();
    };
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let has_arcgis_signature = host.contains("arcgis.com")
        || host.contains("arcgisonline.com")
        || url.path().to_ascii_lowercase().contains("/arcgis/")
        || url
            .query_pairs()
            .any(|(key, _)| key.eq_ignore_ascii_case("webmap"));
    if !has_arcgis_signature {
        return Vec::new();
    }
    let item_id = url
        .query_pairs()
        .find(|(key, value)| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "webmap" | "id" | "itemid"
            ) && is_arcgis_item_id(value)
        })
        .map(|(_, value)| value.into_owned())
        .or_else(|| {
            url.path_segments()?
                .find(|segment| is_arcgis_item_id(segment))
                .map(str::to_string)
        });
    let Some(item_id) = item_id else {
        return Vec::new();
    };
    let root = format!("https://www.arcgis.com/sharing/rest/content/items/{item_id}");
    vec![format!("{root}?f=json"), format!("{root}/data?f=json")]
}

pub(crate) fn arcgis_hub_companion_urls(page_url: &str, body: &[u8]) -> Vec<String> {
    let is_hub = reqwest::Url::parse(page_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| {
            host == "hub.arcgis.com"
                || host.ends_with(".hub.arcgis.com")
                || host.ends_with(".opendata.arcgis.com")
        });
    if !is_hub {
        return Vec::new();
    }
    let decoded = crate::ingestion::client::percent_decode_bytes(body);
    let mut urls = BTreeSet::new();
    let decoded = String::from_utf8_lossy(&decoded);
    let identifiers =
        regex::Regex::new(r#"(?i)"(contentGroupId|siteId|itemId|webmap)"\s*:\s*"([a-f0-9]{32})""#)
            .expect("static ArcGIS property pattern");
    for capture in identifiers.captures_iter(&decoded).take(128) {
        let item_id = &capture[2];
        if capture[1].eq_ignore_ascii_case("contentGroupId") {
            let mut search = reqwest::Url::parse("https://www.arcgis.com/sharing/rest/search")
                .expect("static ArcGIS search URL is valid");
            search.query_pairs_mut().extend_pairs([
                ("f", "json"),
                ("q", &format!("group:{item_id}")),
                ("num", "100"),
                ("start", "1"),
            ]);
            urls.insert(search.to_string());
        } else {
            let root = format!("https://www.arcgis.com/sharing/rest/content/items/{item_id}");
            urls.insert(format!("{root}?f=json"));
            urls.insert(format!("{root}/data?f=json"));
        }
    }
    urls.into_iter().collect()
}

pub(super) fn arcgis_structured_item_companion_urls(value: &serde_json::Value) -> Vec<String> {
    fn visit(value: &serde_json::Value, depth: usize, urls: &mut BTreeSet<String>) {
        if depth > 64 || urls.len() >= MAXIMUM_STRUCTURED_DISCOVERY_LINKS {
            return;
        }
        match value {
            serde_json::Value::Object(object) => {
                if object
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
                {
                    if let Some(item_id) = object
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .filter(|value| is_arcgis_item_id(value))
                    {
                        let root =
                            format!("https://www.arcgis.com/sharing/rest/content/items/{item_id}");
                        urls.insert(format!("{root}?f=json"));
                        urls.insert(format!("{root}/data?f=json"));
                    }
                }
                for nested in object.values() {
                    visit(nested, depth.saturating_add(1), urls);
                }
            }
            serde_json::Value::Array(values) => {
                for nested in values {
                    visit(nested, depth.saturating_add(1), urls);
                }
            }
            _ => {}
        }
    }

    let mut urls = BTreeSet::new();
    visit(value, 0, &mut urls);
    urls.into_iter().collect()
}

pub(super) fn arcgis_search_next_page(
    source_url: Option<&str>,
    value: &serde_json::Value,
) -> Option<String> {
    let next_start = value.get("nextStart")?.as_u64()?;
    if next_start == 0 {
        return None;
    }
    let mut url = reqwest::Url::parse(source_url?).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    if !(host == "arcgis.com" || host.ends_with(".arcgis.com"))
        || !url.path().ends_with("/sharing/rest/search")
    {
        return None;
    }
    let retained = url
        .query_pairs()
        .filter(|(key, _)| !key.eq_ignore_ascii_case("start"))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    url.set_query(None);
    url.query_pairs_mut()
        .extend_pairs(retained)
        .append_pair("start", &next_start.to_string());
    Some(url.to_string())
}

pub(super) fn tableau_export_companion_urls(value: &str) -> Vec<String> {
    let Ok(url) = reqwest::Url::parse(value) else {
        return Vec::new();
    };
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if !host.contains("tableau.com") {
        return Vec::new();
    }
    let segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();
    let view_pair = segments
        .iter()
        .position(|segment| *segment == "viz")
        .and_then(|index| Some((*segments.get(index + 1)?, *segments.get(index + 2)?)))
        .or_else(|| {
            segments
                .iter()
                .position(|segment| *segment == "views")
                .and_then(|index| Some((*segments.get(index + 1)?, *segments.get(index + 2)?)))
        });
    let Some((workbook, raw_sheet)) = view_pair else {
        return Vec::new();
    };
    if workbook.is_empty() || raw_sheet.is_empty() {
        return Vec::new();
    }
    let sheet = raw_sheet
        .trim_end_matches(".csv")
        .trim_end_matches(".pdf")
        .trim_end_matches(".png");
    let origin = format!("{}://{}", url.scheme(), host);
    let base = format!("{origin}/views/{workbook}/{sheet}");
    vec![
        format!("{base}.csv?:showVizHome=no"),
        format!("{base}.pdf?:showVizHome=no"),
    ]
}

fn is_arcgis_item_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn arcgis_layer_url(value: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(value).ok()?;
    let segments = url.path_segments()?.collect::<Vec<_>>();
    let server_index = segments
        .iter()
        .position(|segment| matches!(*segment, "FeatureServer" | "MapServer"))?;
    let layer_id = segments.get(server_index.saturating_add(1))?;
    if segments
        .iter()
        .skip(server_index + 2)
        .any(|part| !part.is_empty())
        || url
            .query_pairs()
            .any(|(key, value)| key != "f" || !matches!(value.as_ref(), "json" | "pjson"))
    {
        // A supplied /query is a selected export, possibly filtered. Do not turn it into 1=1.
        return None;
    }
    if layer_id.is_empty() || !layer_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let path = format!("/{}", segments[..=server_index + 1].join("/"));
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

pub(super) fn arcgis_metadata_url(value: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(value).ok()?;
    let segments = url.path_segments()?.collect::<Vec<_>>();
    let server_index = segments
        .iter()
        .position(|segment| matches!(*segment, "FeatureServer" | "MapServer"))?;
    if segments
        .get(server_index.saturating_add(1))
        .is_some_and(|segment| !segment.is_empty())
    {
        return None;
    }
    let path = format!("/{}", segments[..=server_index].join("/"));
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut().append_pair("f", "json");
    Some(url.to_string())
}

pub(super) fn arcgis_layers_from_metadata(
    source_url: Option<&str>,
    value: &serde_json::Value,
) -> Vec<String> {
    let Some(source_url) = source_url else {
        return Vec::new();
    };
    let Ok(mut base) = reqwest::Url::parse(source_url) else {
        return Vec::new();
    };
    base.set_query(None);
    base.set_fragment(None);
    let path = base.path().trim_end_matches('/');
    if !path.ends_with("/FeatureServer") && !path.ends_with("/MapServer") {
        return Vec::new();
    }
    ["layers", "tables"]
        .into_iter()
        .filter_map(|key| value.get(key).and_then(serde_json::Value::as_array))
        .flatten()
        .filter_map(|layer| layer.get("id").and_then(serde_json::Value::as_u64))
        .take(10_000)
        .map(|layer_id| {
            let mut layer_url = base.clone();
            layer_url.set_path(&format!("{path}/{layer_id}"));
            layer_url.to_string()
        })
        .collect()
}
