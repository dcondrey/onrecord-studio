use serde::{Deserialize, Serialize};

use super::{PortalRegistration, SourceKind};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CrawlPolicy {
    pub maximum_pages: usize,
    pub maximum_depth: usize,
    pub maximum_resources: usize,
    pub follow_supported_external_platforms: bool,
}

impl Default for CrawlPolicy {
    fn default() -> Self {
        Self {
            maximum_pages: 5_000,
            maximum_depth: 8,
            maximum_resources: 100_000,
            follow_supported_external_platforms: true,
        }
    }
}

/// Which slice of a portal's metadata API a synchronization starts from.
///
/// Only the NextRequest API path reads this; a crawled website or catalog has no page window.
/// `Default` is 1 and 50, the fixed values `start_data_source` used to pass, so an operator who
/// never opens the control keeps exactly today's behaviour. The bounds are
/// `validate_page_parameters` in src-tauri/src/ingestion/queue/guidance.rs, which stays the authority.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MetadataPageWindow {
    pub start_page: usize,
    pub page_size: usize,
}

impl Default for MetadataPageWindow {
    fn default() -> Self {
        Self {
            start_page: 1,
            page_size: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceGuidance {
    /// Apply this approved recipe to other source roots on the exact same normalized hostname.
    /// False by default so a mapping is never generalized without an operator's decision.
    #[serde(default)]
    pub reuse_for_publisher_pattern: bool,
    #[serde(default)]
    pub additional_seed_urls: Vec<String>,
    #[serde(default)]
    pub allowed_hostnames: Vec<String>,
    #[serde(default)]
    pub include_url_prefixes: Vec<String>,
    #[serde(default)]
    pub exclude_url_prefixes: Vec<String>,
    pub link_selector: Option<String>,
    pub download_selector: Option<String>,
    #[serde(default)]
    pub include_filename_globs: Vec<String>,
    #[serde(default)]
    pub exclude_filename_globs: Vec<String>,
    #[serde(default)]
    pub include_filename_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_filename_patterns: Vec<String>,
    #[serde(default)]
    pub include_content_keywords: Vec<String>,
    #[serde(default)]
    pub exclude_content_keywords: Vec<String>,
    #[serde(default)]
    pub include_content_patterns: Vec<String>,
    #[serde(default)]
    pub exclude_content_patterns: Vec<String>,
    #[serde(default)]
    pub include_organizations: Vec<String>,
    #[serde(default)]
    pub exclude_organizations: Vec<String>,
    #[serde(default)]
    pub minimum_file_size_bytes: Option<u64>,
    #[serde(default)]
    pub maximum_file_size_bytes: Option<u64>,
    #[serde(default)]
    pub document_date_from: Option<String>,
    #[serde(default)]
    pub document_date_to: Option<String>,
    /// Refuse a discovered download whose bytes live on a host other than the source root's,
    /// unless `allowed_hostnames` names that host. False is today's behaviour: a crawled page may
    /// steer a download anywhere, which is how government portals that host their PDFs on S3,
    /// Azure Blob, Granicus or CivicPlus are ingested at all. Setting it is an operator's decision
    /// to trade that reach for a hard scope boundary, and it never affects which links are
    /// followed -- only which payloads are fetched.
    #[serde(default)]
    pub restrict_downloads_to_source_host: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredSourceLink {
    pub url: String,
    pub label: String,
    pub download_candidate: bool,
    pub follow_candidate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceDiscoveryPreview {
    pub source: PortalRegistration,
    pub detected_kind: SourceKind,
    pub confidence_basis_points: u16,
    pub candidate_links: Vec<DiscoveredSourceLink>,
    pub warnings: Vec<String>,
    pub repair_available: bool,
    pub selector_preview_applied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceIngestionStart {
    pub source: PortalRegistration,
    pub sync_run_id: String,
    pub detected_kind: SourceKind,
    pub used_saved_guidance: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PastedContentInput {
    pub title: String,
    pub content: String,
    pub declared_media_type: Option<String>,
}
