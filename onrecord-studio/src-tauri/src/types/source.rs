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

// FEATURE(source-repair): Persist successful selector/field mappings as reusable connector
// hypotheses, then let the user approve them for other sources with the same publisher pattern.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceGuidance {
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
