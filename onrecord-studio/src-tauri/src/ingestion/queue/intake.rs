//! Source intake: public-source discovery, local files, pasted content, and their synthetic identities.

use super::*;
use super::{
    guidance::{validate_crawl_policy, validate_source_guidance},
    objects::{local_import_limit, sanitize_file_name, store_inline_payload},
};

impl QueueManager {
    pub async fn discover_and_start_public_source(
        &self,
        source_url: &str,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        rate_limit_policy: RateLimitPolicy,
    ) -> Result<SourceIngestionStart, AppError> {
        self.discover_and_start_public_source_with_policy(
            source_url,
            crawl_policy,
            guidance,
            rate_limit_policy,
            PoolSeedPolicy::OperatorRequested,
        )
        .await
    }

    /// Automatic discovery admits only a new resolved source. Aliases must not
    /// reactivate a paused/removed source or replace an owner's saved recipe.
    pub async fn discover_and_start_public_source_automatically(
        &self,
        source_url: &str,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        rate_limit_policy: RateLimitPolicy,
    ) -> Result<SourceIngestionStart, AppError> {
        self.discover_and_start_public_source_with_policy(
            source_url,
            crawl_policy,
            guidance,
            rate_limit_policy,
            PoolSeedPolicy::AutomaticDiscovery {
                submitted_url: source_url.to_string(),
            },
        )
        .await
    }

    async fn discover_and_start_public_source_with_policy(
        &self,
        source_url: &str,
        crawl_policy: CrawlPolicy,
        guidance: SourceGuidance,
        rate_limit_policy: RateLimitPolicy,
        seed_policy: PoolSeedPolicy,
    ) -> Result<SourceIngestionStart, AppError> {
        validate_crawl_policy(&crawl_policy)?;
        validate_source_guidance(&guidance)?;
        // Platform detection runs ahead of the crawler, not as a fallback from it. A catalog
        // portal's index API names every asset it holds in a few paged requests; walking the HTML
        // that renders that index costs a request per page, per facet, and per navigation link,
        // which is what earned this client its 403s. Operator guidance overrides detection,
        // because a repair recipe is a statement about this source that a probe cannot outrank.
        let operator_supplied_recipe = !guidance.additional_seed_urls.is_empty()
            || guidance.link_selector.is_some()
            || guidance.download_selector.is_some();
        let catalog = if operator_supplied_recipe {
            None
        } else {
            self.client.detect_data_catalog(source_url).await
        };
        let (mut source, inspection) = self.client.discover_public_source(source_url).await?;
        if catalog.is_none()
            && matches!(inspection.kind, PublicResourceKind::HtmlPage)
            && inspection.links.is_empty()
            && inspection.embedded_connectors.is_empty()
            && !operator_supplied_recipe
        {
            return Err(AppError::PortalDiscovery(
                "The page was reachable but exposed no static data links. Open guided repair to select or paste the underlying dataset, download, folder, or dashboard URL."
                    .to_string(),
            ));
        }
        let initial_tasks = match catalog {
            Some(endpoint) => {
                source.source_kind = SourceKind::OpenDataCatalog;
                vec![IngestionTask::FetchDataCatalogPage {
                    subdomain: source.portal_key.clone(),
                    endpoint,
                    cursor: None,
                }]
            }
            None => {
                let mut seeds = vec![inspection.final_url.clone()];
                seeds.extend(guidance.additional_seed_urls.iter().cloned());
                seeds.sort();
                seeds.dedup();
                seeds
                    .into_iter()
                    .map(|url| IngestionTask::CrawlPublicResource {
                        subdomain: source.portal_key.clone(),
                        url,
                        depth: 0,
                    })
                    .collect()
            }
        };
        let detected_kind = source.source_kind;
        if seed_policy == PoolSeedPolicy::OperatorRequested
            && (guidance != SourceGuidance::default() || crawl_policy != CrawlPolicy::default())
        {
            self.repository
                .insert_or_update_portal_registration(&source)
                .await?;
            self.repository
                .save_source_connector_recipe(&source, source_url, &crawl_policy, &guidance)
                .await?;
        }
        let sync_run_id = self
            .start_task_pool(
                source.clone(),
                rate_limit_policy,
                initial_tasks,
                crawl_policy,
                guidance,
                seed_policy,
            )
            .await?;
        Ok(SourceIngestionStart {
            source,
            sync_run_id,
            detected_kind,
            used_saved_guidance: false,
        })
    }

    pub async fn preview_public_source(
        &self,
        source_url: &str,
        guidance: Option<SourceGuidance>,
    ) -> Result<crate::types::SourceDiscoveryPreview, AppError> {
        let guidance = guidance.unwrap_or_default();
        validate_source_guidance(&guidance)?;
        let selector_preview_applied =
            guidance.link_selector.is_some() || guidance.download_selector.is_some();
        if let Ok(source) = self.client.discover_portal(source_url).await {
            return Ok(crate::types::SourceDiscoveryPreview {
                detected_kind: SourceKind::NextRequest,
                source,
                confidence_basis_points: 10_000,
                candidate_links: Vec::new(),
                warnings: Vec::new(),
                repair_available: false,
                selector_preview_applied: false,
            });
        }
        let (source, default_inspection) = self.client.discover_public_source(source_url).await?;
        let inspection = if selector_preview_applied {
            self.client
                .inspect_public_resource_with_selectors(
                    source_url,
                    guidance.link_selector.as_deref(),
                    guidance.download_selector.as_deref(),
                )
                .await?
        } else {
            default_inspection
        };
        let has_embedded_connector = !inspection.embedded_connectors.is_empty();
        let candidate_links = inspection
            .links
            .into_iter()
            .take(500)
            .map(|link| crate::types::DiscoveredSourceLink {
                url: link.url,
                label: link.label,
                download_candidate: link.download_candidate,
                follow_candidate: link.follow_candidate && !link.navigation_chrome,
            })
            .collect::<Vec<_>>();
        let mut warnings = Vec::new();
        if matches!(inspection.kind, PublicResourceKind::HtmlPage)
            && candidate_links.is_empty()
            && !has_embedded_connector
        {
            warnings.push(
                "No static data links were visible; this source may need a dashboard, catalog, or custom-selector recipe."
                    .to_string(),
            );
        }
        let confidence_basis_points = if matches!(inspection.kind, PublicResourceKind::Download) {
            10_000
        } else if has_embedded_connector {
            9_500
        } else if candidate_links.iter().any(|link| link.download_candidate) {
            9_000
        } else if !candidate_links.is_empty() {
            7_500
        } else {
            4_000
        };
        Ok(crate::types::SourceDiscoveryPreview {
            detected_kind: source.source_kind,
            source,
            confidence_basis_points,
            candidate_links,
            repair_available: !warnings.is_empty(),
            selector_preview_applied,
            warnings,
        })
    }

    pub async fn start_local_files(
        &self,
        paths: Vec<String>,
    ) -> Result<SourceIngestionStart, AppError> {
        crate::licensing::recheck()?;
        if paths.is_empty() || paths.len() > MAXIMUM_LOCAL_IMPORT_FILES {
            return Err(AppError::DataParsing(format!(
                "Select between 1 and {MAXIMUM_LOCAL_IMPORT_FILES} files per import"
            )));
        }
        let mut canonical_paths = Vec::with_capacity(paths.len());
        for path in paths {
            if path.trim().is_empty() || path.len() > 32_768 {
                return Err(AppError::DataParsing(
                    "Selected file paths must not be empty".to_string(),
                ));
            }
            let path = tokio::fs::canonicalize(&path).await.map_err(|error| {
                AppError::Storage(format!("Failed to resolve selected file '{path}': {error}"))
            })?;
            let metadata = tokio::fs::symlink_metadata(&path).await.map_err(|error| {
                AppError::Storage(format!("Failed to inspect selected file: {error}"))
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(AppError::DataParsing(format!(
                    "Selected path '{}' is not a regular file",
                    path.display()
                )));
            }
            let maximum_bytes = local_import_limit(&path).await;
            if metadata.len() > maximum_bytes {
                return Err(AppError::DataParsing(format!(
                    "Selected file '{}' exceeds the {maximum_bytes}-byte import limit",
                    path.display()
                )));
            }
            canonical_paths.push(path);
        }
        canonical_paths.sort();
        canonical_paths.dedup();
        let identity = canonical_paths
            .iter()
            .map(|path| path.to_string_lossy())
            .collect::<Vec<_>>()
            .join("\n");
        let source = local_source_registration(
            SourceKind::LocalFile,
            if canonical_paths.len() == 1 {
                canonical_paths[0]
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Local file".to_string())
            } else {
                format!("{} selected files", canonical_paths.len())
            },
            &identity,
        );
        self.repository
            .insert_or_update_portal_registration(&source)
            .await?;
        let request = synthetic_source_request(&source, "User-selected local evidence files");
        self.repository
            .insert_or_update_request(&source.portal_key, &request)
            .await?;
        let output_directory = self
            .download_root
            .join(format!("{}_documents", source.portal_key));
        let initial_tasks = canonical_paths
            .into_iter()
            .map(|path| {
                let path_string = path.to_string_lossy().into_owned();
                let document_id = stable_source_identifier(&format!("local|{path_string}"));
                let now = Utc::now().to_rfc3339();
                IngestionTask::ImportLocalPayload {
                    subdomain: source.portal_key.clone(),
                    request_id: request.id,
                    source_path: path_string,
                    output_directory: output_directory.to_string_lossy().into_owned(),
                    document: Box::new(DocumentMetadata {
                        id: document_id,
                        file_name: path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned()),
                        file_size: None,
                        description: Some("User-selected local evidence file".to_string()),
                        content_type: None,
                        download_url: None,
                        created_at: now.clone(),
                        updated_at: now,
                        visibility: "public".to_string(),
                        local_path: None,
                        extracted_text_sha256: None,
                        was_ocr_processed: false,
                    }),
                }
            })
            .collect();
        let sync_run_id = self
            .start_task_pool(
                source.clone(),
                RateLimitPolicy::default(),
                initial_tasks,
                CrawlPolicy::default(),
                SourceGuidance::default(),
                PoolSeedPolicy::OperatorRequested,
            )
            .await?;
        Ok(SourceIngestionStart {
            source,
            sync_run_id,
            detected_kind: SourceKind::LocalFile,
            used_saved_guidance: false,
        })
    }

    pub async fn start_pasted_content(
        &self,
        input: PastedContentInput,
    ) -> Result<SourceIngestionStart, AppError> {
        crate::licensing::recheck()?;
        let title = input.title.trim();
        if title.is_empty() || title.len() > 256 {
            return Err(AppError::DataParsing(
                "Pasted content title must contain between 1 and 256 characters".to_string(),
            ));
        }
        if input.content.trim().is_empty() || input.content.len() > MAXIMUM_PASTED_CONTENT_BYTES {
            return Err(AppError::DataParsing(format!(
                "Pasted content must contain between 1 and {MAXIMUM_PASTED_CONTENT_BYTES} UTF-8 bytes"
            )));
        }
        let content_sha256 = sha256_hex(input.content.as_bytes());
        let source = local_source_registration(
            SourceKind::PastedContent,
            title.to_string(),
            &content_sha256,
        );
        self.repository
            .insert_or_update_portal_registration(&source)
            .await?;
        let request = synthetic_source_request(&source, "User-pasted content");
        self.repository
            .insert_or_update_request(&source.portal_key, &request)
            .await?;
        let extension = media_type_extension(input.declared_media_type.as_deref(), &input.content);
        let document_id = stable_source_identifier(&format!("pasted|{content_sha256}"));
        let managed_path =
            store_inline_payload(&self.download_root, document_id, input.content.as_bytes())
                .await?;
        let now = Utc::now().to_rfc3339();
        let document = DocumentMetadata {
            id: document_id,
            file_name: Some(format!(
                "{}.{}",
                sanitize_file_name(title, document_id),
                extension
            )),
            file_size: Some(input.content.len() as u64),
            description: Some("User-pasted content".to_string()),
            content_type: input.declared_media_type,
            download_url: None,
            created_at: now.clone(),
            updated_at: now,
            visibility: "public".to_string(),
            local_path: Some(managed_path.to_string_lossy().into_owned()),
            extracted_text_sha256: None,
            was_ocr_processed: false,
        };
        let output_directory = self
            .download_root
            .join(format!("{}_documents", source.portal_key));
        let sync_run_id = self
            .start_task_pool(
                source.clone(),
                RateLimitPolicy::default(),
                vec![IngestionTask::DownloadBinaryPayload {
                    subdomain: source.portal_key.clone(),
                    request_id: request.id,
                    document: Box::new(document),
                    output_directory: output_directory.to_string_lossy().into_owned(),
                    container_depth: 0,
                    container_root_directory: None,
                    crawl_depth: 0,
                    structured_depth: 0,
                }],
                CrawlPolicy::default(),
                SourceGuidance::default(),
                PoolSeedPolicy::OperatorRequested,
            )
            .await?;
        Ok(SourceIngestionStart {
            source,
            sync_run_id,
            detected_kind: SourceKind::PastedContent,
            used_saved_guidance: false,
        })
    }
}

pub(super) fn stable_source_identifier(value: &str) -> u64 {
    let digest = Sha256::digest(value.as_bytes());
    let prefix = u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 prefix is 8 bytes"));
    SYNTHETIC_DOCUMENT_NAMESPACE | (prefix & SYNTHETIC_SOURCE_IDENTIFIER_MASK)
}

pub(super) fn source_file_name(value: &str, document_id: u64) -> String {
    let candidate = reqwest::Url::parse(value)
        .ok()
        .and_then(|url| {
            url.path_segments()
                .and_then(|mut segments| segments.next_back())
                .map(str::to_string)
        })
        .unwrap_or_default();
    sanitize_file_name(&candidate, document_id)
}

pub(super) fn discovered_file_name(
    inspection: &PublicResourceInspection,
    document_id: u64,
) -> String {
    let from_url = source_file_name(&inspection.final_url, document_id);
    let title = inspection.title.trim();
    if title.is_empty() || has_likely_document_extension(&from_url) {
        from_url
    } else {
        title.chars().take(512).collect()
    }
}

pub(super) fn has_likely_document_extension(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        ".pdf", ".csv", ".tsv", ".json", ".jsonl", ".geojson", ".xml", ".xlsx", ".xls", ".ods",
        ".docx", ".doc", ".odt", ".pptx", ".ppt", ".rtf", ".txt", ".md", ".zip", ".tar", ".gz",
        ".db", ".sqlite", ".msg", ".eml", ".png", ".jpg", ".jpeg", ".tif", ".tiff", ".webp",
    ]
    .iter()
    .any(|extension| value.ends_with(extension))
}

fn local_source_registration(
    kind: SourceKind,
    title: String,
    identity: &str,
) -> PortalRegistration {
    let digest = sha256_hex(identity.as_bytes());
    let source_key = format!("source-{}", &digest[..24]);
    let source_uri = format!("onrecord://local/{source_key}");
    PortalRegistration {
        portal_key: source_key,
        display_name: title,
        canonical_url: source_uri.clone(),
        origin_url: "onrecord://local/".to_string(),
        api_layout: PortalApiLayout {
            requests_endpoint: source_uri,
            requests_pointer: "/resources".to_string(),
            metadata_pointer: None,
            page_parameter: "import_page".to_string(),
            page_size_parameter: "import_limit".to_string(),
        },
        verified_at: Utc::now().to_rfc3339(),
        source_kind: kind,
    }
}

fn synthetic_source_request(source: &PortalRegistration, description: &str) -> PublicRequest {
    let now = Utc::now().to_rfc3339();
    PublicRequest {
        id: stable_source_identifier(&format!("request|{}", source.portal_key)),
        rpid: Some(format!(
            "source-{}",
            &source.portal_key[source.portal_key.len().saturating_sub(12)..]
        )),
        title: Some(source.display_name.clone()),
        description: Some(description.to_string()),
        status: "imported".to_string(),
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
    }
}

fn media_type_extension(media_type: Option<&str>, content: &str) -> &'static str {
    match media_type.unwrap_or_default().to_ascii_lowercase().as_str() {
        "application/json" | "application/geo+json" => "json",
        "application/x-ndjson" => "ndjson",
        "text/csv" => "csv",
        "text/tab-separated-values" => "tsv",
        "text/html" | "application/xhtml+xml" => "html",
        "application/xml" | "text/xml" => "xml",
        "text/markdown" => "md",
        _ if serde_json::from_str::<serde_json::Value>(content).is_ok() => "json",
        _ => "txt",
    }
}
