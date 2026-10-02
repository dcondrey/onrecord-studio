//! Bounded, portal-isolated ingestion scheduling and document extraction.

mod crawl;
mod extraction;
mod frontier;
mod guidance;
mod intake;
mod manager;
mod objects;
mod payload;
mod progress;
mod resources;
mod structured;
#[cfg(test)]
mod tests;
mod worker;

use crate::blocking::spawn as spawn_blocking;
use crate::error::AppError;
use crate::file_name::sanitized_component;
use crate::ingestion::backoff;
use crate::ingestion::client::ApiClient;
use crate::ingestion::client::{
    AcquisitionRecord, DataCatalogEndpoint, DownloadedPayload, PublicEmbeddedConnector,
    PublicResourceInspection, PublicResourceKind, PublicResourceLink,
    WordPressGoogleDriveConnector,
};
use crate::ingestion::host_policy::RequestRefusal;
use crate::ingestion::limiter::RateLimiter;
use crate::integrity::{constant_time_equal, digest_hex, sha256_file_async, sha256_hex};
use crate::parsers::{
    assess_file_entropy, detect_document_format, execute_local_ocr_scan, extract_archive_container,
    extract_duckdb_text, extract_html_text, extract_msg_layers, extract_office_package,
    extract_plain_text, extract_rtf_text, extract_salvage_report, extract_spreadsheet_layers,
    extract_sql_dump_text, extract_sqlite_text, extract_svg_text, extract_text_layers,
    extract_zip_container_with_manifest, parser_capability, DocumentFormat, ExtractionRoute,
    MsgExtractionContext,
};
use crate::storage::corpus::{CorpusDocumentInput, CorpusIndex};
use crate::storage::sqlite::{
    DocumentDownloadFinalization, DocumentExtractionFinalization, DuckDbRepository,
    PortalSyncRunFinalization,
};
use crate::types::{
    CrawlPolicy, DocumentMetadata, DocumentPurge, FrontierExclusionCount, MetadataPageWindow,
    PastedContentInput, PortalApiLayout, PortalRegistration, PortalRuntimeSnapshot,
    PortalSyncStart, PublicRequest, RateLimitPolicy, SourceGuidance, SourceIngestionStart,
    SourceKind, SourceLifecycleState, SourcePurge, SourcePurgePreview, SourceRemoval, StoppedHost,
};
use chrono::{NaiveDate, Utc};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, watch, Mutex, RwLock, Semaphore};
use uuid::Uuid;

use intake::stable_source_identifier;
pub(crate) use objects::remove_released_evidence_files;
pub use progress::{ingestion_telemetry_envelope, IngestionProgress, IngestionTelemetryEnvelope};
pub(crate) use structured::arcgis_hub_companion_urls;

// The concurrent-portal ceiling is a runtime setting (`QueueManager::max_active_portals`),
// not a hard limit: a start beyond it parks until a running pool's slot frees rather than
// being refused. This constant is only the setting's default for a fresh install.
const DEFAULT_MAXIMUM_ACTIVE_PORTALS: usize = 5;

const WORKERS_PER_PORTAL: usize = 2;

const GLOBAL_WORKER_LIMIT: usize = 10;

const MAXIMUM_CPU_PARSERS: usize = 6;

const MAXIMUM_CONCURRENT_OCR: usize = 2;

const PARSER_MEMORY_PERMIT_BYTES: u64 = 16 * 1024 * 1024;

const PARSER_MEMORY_PERMITS: u32 = 64;

const PORTAL_QUEUE_CAPACITY: usize = 2_048;

const MAXIMUM_METADATA_PAGE: usize = 50_000;

const MAXIMUM_CONTAINER_DEPTH: usize = 8;

// Structured (JSON) expansion chains on its own axis, exactly like container nesting: a JSON
// payload yields downloads, one of which is JSON again. Site-crawl depth must not pay for that,
// or a catalog found deep in a portal silently stops yielding documents.
const MAXIMUM_STRUCTURED_CHAIN: usize = 8;

const MAXIMUM_CONTAINER_ENTRIES: usize = 2_000;

const MAXIMUM_CONTAINER_TREE_ENTRIES: usize = MAXIMUM_CONTAINER_ENTRIES * MAXIMUM_CONTAINER_DEPTH;

const MAXIMUM_CONTAINER_TREE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

// Synthetic child IDs occupy a dedicated range that remains exactly representable by JavaScript.
// Keep this namespace disjoint from upstream IDs when introducing another derived document kind.
const SYNTHETIC_DOCUMENT_NAMESPACE: u64 = 1_u64 << 52;

const SYNTHETIC_SOURCE_IDENTIFIER_MASK: u64 = (1_u64 << 51) - 1;

const MAXIMUM_LOCAL_IMPORT_BYTES: u64 = 512 * 1024 * 1024;

const MAXIMUM_LOCAL_IMPORT_FILES: usize = 1_000;

const MAXIMUM_PASTED_CONTENT_BYTES: usize = 16 * 1024 * 1024;

// LIMIT(ingestion): Structured discovery is intentionally unavailable above this fixed per-file
// ceiling; raising it alone would permit downloads that parsers cannot safely hold.
const MAXIMUM_STRUCTURED_DISCOVERY_BYTES: u64 = 64 * 1024 * 1024;

const MAXIMUM_STRUCTURED_DISCOVERY_LINKS: usize = 100_000;

const ARCGIS_FEATURE_PAGE_SIZE: usize = 500;

const MAXIMUM_DURABLE_TASK_ATTEMPTS: u64 = 4;

// Scheduler admission counters, durable task attempts, and retry eligibility persist in DuckDB.
// Channel position is rebuilt from monotonically increasing frontier identifiers after restart.

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "taskKind", rename_all = "camelCase")]
pub enum IngestionTask {
    FetchMetadataPage {
        subdomain: String,
        page: usize,
        per_page: usize,
    },
    DownloadBinaryPayload {
        subdomain: String,
        request_id: u64,
        #[serde(with = "persisted_document")]
        document: Box<DocumentMetadata>,
        output_directory: String,
        container_depth: usize,
        /// Managed root of the recursive container tree. This is persisted so aggregate disk
        /// accounting remains effective when a durable child task resumes after restart.
        #[serde(default)]
        container_root_directory: Option<String>,
        /// Crawl-graph depth; see `PayloadDepth`. Defaulted because the durable frontier holds
        /// rows serialized before this field existed, and those resume at depth 0 as they did.
        #[serde(default)]
        crawl_depth: usize,
        /// Structured-chain depth; see `PayloadDepth`. Defaulted for the same reason.
        #[serde(default)]
        structured_depth: usize,
    },
    CrawlPublicResource {
        subdomain: String,
        url: String,
        depth: usize,
    },
    CrawlWordPressGoogleDrive {
        subdomain: String,
        connector: WordPressGoogleDriveConnector,
        depth: usize,
    },
    CrawlArcGisLayer {
        subdomain: String,
        layer_url: String,
        offset: usize,
        /// Crawl-graph depth; see `PayloadDepth`. Defaulted for the same reason.
        #[serde(default)]
        crawl_depth: usize,
        /// Structured-chain depth; see `PayloadDepth`. A layer reached from a JSON catalog is one
        /// chain hop, and its feature pages are `application/geo+json`, so they expand again.
        #[serde(default)]
        structured_depth: usize,
    },
    /// One page of a platform catalog index (CKAN, Socrata). Paging an index is not crawling: it
    /// visits one endpoint repeatedly with a cursor, so it holds no crawl depth.
    FetchDataCatalogPage {
        subdomain: String,
        endpoint: DataCatalogEndpoint,
        #[serde(default)]
        cursor: Option<String>,
    },
    ImportLocalPayload {
        subdomain: String,
        request_id: u64,
        #[serde(with = "persisted_document")]
        document: Box<DocumentMetadata>,
        source_path: String,
        output_directory: String,
    },
}

mod persisted_document {
    use serde::ser::SerializeStruct;
    use serde::Deserialize;

    use super::DocumentMetadata;

    #[derive(Deserialize)]
    struct StoredDocumentMetadata {
        id: u64,
        file_name: Option<String>,
        file_size: Option<u64>,
        description: Option<String>,
        content_type: Option<String>,
        download_url: Option<String>,
        created_at: String,
        updated_at: String,
        visibility: String,
        local_path: Option<String>,
        extracted_text_sha256: Option<String>,
        was_ocr_processed: bool,
    }

    pub(super) fn serialize<S>(
        document: &DocumentMetadata,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("DocumentMetadata", 12)?;
        state.serialize_field("id", &document.id)?;
        state.serialize_field("file_name", &document.file_name)?;
        state.serialize_field("file_size", &document.file_size)?;
        state.serialize_field("description", &document.description)?;
        state.serialize_field("content_type", &document.content_type)?;
        state.serialize_field("download_url", &document.download_url)?;
        state.serialize_field("created_at", &document.created_at)?;
        state.serialize_field("updated_at", &document.updated_at)?;
        state.serialize_field("visibility", &document.visibility)?;
        state.serialize_field("local_path", &document.local_path)?;
        state.serialize_field("extracted_text_sha256", &document.extracted_text_sha256)?;
        state.serialize_field("was_ocr_processed", &document.was_ocr_processed)?;
        state.end()
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Box<DocumentMetadata>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let document = StoredDocumentMetadata::deserialize(deserializer)?;
        Ok(Box::new(DocumentMetadata {
            id: document.id,
            file_name: document.file_name,
            file_size: document.file_size,
            description: document.description,
            content_type: document.content_type,
            download_url: document.download_url,
            created_at: document.created_at,
            updated_at: document.updated_at,
            visibility: document.visibility,
            local_path: document.local_path,
            extracted_text_sha256: document.extracted_text_sha256,
            was_ocr_processed: document.was_ocr_processed,
        }))
    }
}

/// Whether a pool start may re-seed a request index the previous run was part-way through.
///
/// `OperatorRequested` is the answer for every start an operator performed: they chose the page
/// window that this start seeds, and an inherited frontier does not outrank what they just asked
/// for. `ContinueInterruptedWalk` is the answer for a resume, which is a continuation of an
/// earlier start and not a new instruction: a source paused at page 900 already carries page 901
/// in its durable frontier, so seeding its configured start page there re-fetches every page in
/// between. Deduplication makes that correct and not free -- 900 requests against a portal that
/// rate-limits.
#[derive(Clone, PartialEq, Eq, Debug)]
enum PoolSeedPolicy {
    OperatorRequested,
    ContinueInterruptedWalk,
    AutomaticRefresh,
    AutomaticDiscovery { submitted_url: String },
}

struct ScheduledTask {
    task: IngestionTask,
    frontier_id: Option<i64>,
    frontier_attempt: u64,
}

impl IngestionTask {
    fn portal_key(&self) -> &str {
        match self {
            Self::FetchMetadataPage { subdomain, .. }
            | Self::DownloadBinaryPayload { subdomain, .. }
            | Self::CrawlPublicResource { subdomain, .. }
            | Self::CrawlWordPressGoogleDrive { subdomain, .. }
            | Self::CrawlArcGisLayer { subdomain, .. }
            | Self::FetchDataCatalogPage { subdomain, .. }
            | Self::ImportLocalPayload { subdomain, .. } => subdomain,
        }
    }

    fn target_id(&self) -> String {
        match self {
            Self::FetchMetadataPage {
                subdomain, page, ..
            } => format!("{subdomain}-page-{page}"),
            Self::DownloadBinaryPayload {
                subdomain,
                document,
                ..
            } => format!("{subdomain}-document-{}", document.id),
            Self::CrawlPublicResource { subdomain, url, .. } => {
                format!("{subdomain}-crawl-{}", stable_source_identifier(url))
            }
            Self::CrawlWordPressGoogleDrive {
                subdomain,
                connector,
                ..
            } => format!(
                "{subdomain}-google-drive-{}",
                stable_source_identifier(&format!(
                    "{}|{}",
                    connector.endpoint, connector.folder_id
                ))
            ),
            Self::CrawlArcGisLayer {
                subdomain,
                layer_url,
                offset,
                ..
            } => format!(
                "{subdomain}-arcgis-{}-{offset}",
                stable_source_identifier(layer_url)
            ),
            Self::FetchDataCatalogPage {
                subdomain,
                endpoint,
                cursor,
            } => format!(
                "{subdomain}-catalog-{}-{}",
                stable_source_identifier(&endpoint.api_url),
                cursor.as_deref().unwrap_or("start")
            ),
            Self::ImportLocalPayload {
                subdomain,
                source_path,
                ..
            } => format!(
                "{subdomain}-local-{}",
                stable_source_identifier(source_path)
            ),
        }
    }

    /// The prefix every `FetchMetadataPage` target identifier for `source_key` carries.
    ///
    /// Written here rather than at the call site that queries the frontier, because it is the
    /// same construction `target_id` performs and the two must not drift: a resume decides
    /// whether to re-seed its request index by asking whether a row with this prefix survived,
    /// and a prefix that no longer matched would silently re-seed every time.
    fn metadata_page_target_prefix(source_key: &str) -> String {
        format!("{source_key}-page-")
    }

    /// The remote URL this task will contact, when it contacts one. Used to attribute a host stop
    /// to the host that issued it; `target_id` is an internal identifier and never a URL.
    fn target_url(&self) -> Option<&str> {
        match self {
            Self::DownloadBinaryPayload { document, .. } => document.download_url.as_deref(),
            Self::CrawlPublicResource { url, .. } => Some(url),
            Self::CrawlWordPressGoogleDrive { connector, .. } => Some(&connector.endpoint),
            Self::CrawlArcGisLayer { layer_url, .. } => Some(layer_url),
            Self::FetchDataCatalogPage { endpoint, .. } => Some(&endpoint.api_url),
            Self::FetchMetadataPage { .. } | Self::ImportLocalPayload { .. } => None,
        }
    }
}

struct CrawlState {
    policy: CrawlPolicy,
    guidance: SourceGuidance,
    visited_urls: Mutex<HashSet<String>>,
    resource_urls: Mutex<HashSet<String>>,
    /// One site path, one crawl node. See `claim_crawl_path`.
    crawl_path_claims: Mutex<HashSet<String>>,
    /// Hosts whose stop has already been announced, so one stop produces one event.
    announced_host_stops: Mutex<HashSet<String>>,
    /// How many links each frontier rule refused, keyed by rule. A crawl that silently drops
    /// links reports a corpus it cannot account for, which is the same false claim of coverage
    /// this codebase forbids elsewhere; these counts are what make the drop legible.
    frontier_exclusions: Mutex<BTreeMap<String, u64>>,
    pages_started: AtomicUsize,
    resources_started: AtomicUsize,
    frontier_refill_lock: Mutex<()>,
    arcgis_page_hashes: Mutex<HashSet<String>>,
    /// Regex patterns from `guidance`, compiled once when the crawl starts. See
    /// `CompiledGuidancePatterns` for why a pattern that fails to compile is silently dropped
    /// here rather than aborting the crawl.
    compiled_guidance: CompiledGuidancePatterns,
}

/// Regex predicates from `SourceGuidance`, compiled once per crawl instead of once per
/// evaluation.
///
/// `validate_source_guidance` already refuses to save a pattern that fails `Regex::new`, so
/// reaching this code with an invalid one means the guidance was written some other way (an
/// older save format, a direct database edit). That must not panic the ingestion pipeline: an
/// invalid pattern is simply absent from the compiled list, which makes the predicate it belongs
/// to contribute nothing rather than crash or, worse, silently exclude everything (an `include`
/// list that compiled to nothing would otherwise admit no document).
struct CompiledGuidancePatterns {
    include_filename_patterns: Vec<Regex>,
    exclude_filename_patterns: Vec<Regex>,
    include_content_patterns: Vec<Regex>,
    exclude_content_patterns: Vec<Regex>,
}

impl CompiledGuidancePatterns {
    fn compile(guidance: &SourceGuidance) -> Self {
        let compile_all = |patterns: &[String]| -> Vec<Regex> {
            patterns
                .iter()
                .filter_map(|pattern| Regex::new(pattern).ok())
                .collect()
        };
        Self {
            include_filename_patterns: compile_all(&guidance.include_filename_patterns),
            exclude_filename_patterns: compile_all(&guidance.exclude_filename_patterns),
            include_content_patterns: compile_all(&guidance.include_content_patterns),
            exclude_content_patterns: compile_all(&guidance.exclude_content_patterns),
        }
    }
}

/// Why the frontier refused a discovered link.
///
/// Returned rather than folded into a bare `false` so the count that reaches the operator names
/// the rule that fired. Scoping rules added later extend this enum instead of replacing it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FrontierExclusion {
    /// The source's saved guidance excludes the URL by prefix or hostname.
    Guidance,
    /// The link differs from an already-scheduled crawl node only by its query string. A site
    /// crawl that follows these enumerates a faceted search, which is combinatorial and is what
    /// made the queue unbounded: `/dataset/?tags=species`, `?tags=wildlife`, `?tags=habitat` and
    /// so on are one page with a filter, not distinct records.
    QueryVariant { path: String },
    /// The host is off-scope for this source and no policy admits it.
    OffScopeHost,
    /// The payload is hosted off the source root and the source set
    /// `restrict_downloads_to_source_host`. Distinct from `OffScopeHost` because that rule governs
    /// which links are followed and this one governs only which payloads are fetched; a ledger that
    /// merged them could not say which policy dropped the document.
    OffSourceHostDownload,
    /// The crawl depth ceiling was reached.
    DepthCeiling,
    /// The link sits in the page's navigation chrome. See `is_navigation_chrome`.
    NavigationChrome,
    /// This run has stopped contacting the host. Refused before the task is scheduled so a
    /// stopped host does not accumulate tasks that each fail at dispatch.
    HostStopped { detail: String },
    /// The host's robots.txt excludes the path.
    RobotsExcluded { rule: String },
    /// The document's derived or API-reported filename fails a `SourceGuidance` filename glob or
    /// regex include/exclude rule. Evaluated before download.
    FilenamePattern,
    /// A known file size falls outside `minimum_file_size_bytes`/`maximum_file_size_bytes`.
    /// Evaluated before download, and only when a size is actually known pre-download.
    FileSize,
    /// The document's reported date falls outside `document_date_from`/`document_date_to`.
    /// Evaluated after extraction, and only when the source supplies a document-carried date.
    DocumentDate,
    /// The extracted text fails a content keyword include/exclude rule. Evaluated after
    /// extraction, once text is actually available.
    ContentKeyword,
    /// The extracted text fails a content regex include/exclude rule. Evaluated after extraction.
    ContentPattern,
    /// The owning request's agency fails an organization include/exclude rule. Evaluated after
    /// extraction, alongside the content rules above.
    Organization,
}

impl FrontierExclusion {
    fn rule(&self) -> &'static str {
        match self {
            Self::Guidance => "guidance",
            Self::QueryVariant { .. } => "query-variant",
            Self::OffScopeHost => "off-scope-host",
            Self::OffSourceHostDownload => "off-source-host-download",
            Self::DepthCeiling => "depth-ceiling",
            Self::NavigationChrome => "navigation-chrome",
            Self::HostStopped { .. } => "host-stopped",
            Self::RobotsExcluded { .. } => "robots-excluded",
            Self::FilenamePattern => "filename-pattern",
            Self::FileSize => "file-size",
            Self::DocumentDate => "document-date",
            Self::ContentKeyword => "content-keyword",
            Self::ContentPattern => "content-pattern",
            Self::Organization => "organization",
        }
    }
}

#[derive(Default)]
struct PortalStatistics {
    // Includes tasks persisted in the durable frontier, not only tasks currently in the channel.
    queued_tasks: AtomicUsize,
    active_tasks: AtomicUsize,
    live_workers: AtomicUsize,
    completed_tasks: AtomicU64,
    failed_tasks: AtomicU64,
    pages_processed: AtomicU64,
    requests_processed: AtomicU64,
    documents_processed: AtomicU64,
    data_documents_processed: AtomicU64,
    running: AtomicBool,
    stop_requested: AtomicBool,
    last_failure: StdMutex<Option<String>>,
}

struct PortalPool {
    portal: PortalRegistration,
    cancellation: watch::Sender<bool>,
    statistics: Arc<PortalStatistics>,
    sync_run_id: String,
    worker_handles: Vec<tokio::task::JoinHandle<()>>,
    crawl: Arc<CrawlState>,
}

/// Depth bookkeeping carried by a payload task.
///
/// Three independent axes, each with its own ceiling.
///
/// `container` counts archive nesting, bounded by `MAXIMUM_CONTAINER_DEPTH`.
///
/// `crawl` is the payload's depth in the site graph and has exactly the same meaning as
/// `IngestionTask::CrawlPublicResource::depth`: seeds are 0, anything derived from a node at
/// depth D is at D + 1, bounded by `CrawlPolicy::maximum_depth`. Paginating a single resource
/// (ArcGIS feature offsets, ArcGIS search pages) holds its depth, because it revisits one
/// resource rather than following a link.
///
/// `structured` counts JSON -> download -> JSON hops, bounded by `MAXIMUM_STRUCTURED_CHAIN`.
/// It is deliberately separate from `crawl`: a Socrata or ArcGIS catalog sitting eight pages
/// into a portal must still expand, and the chain it can build must still terminate. Laundering
/// a chain through an HTML page resets `structured` but spends `crawl`, since every HTML hop
/// goes through `reserve_crawl_page`, so the two axes bound the combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PayloadDepth {
    container: usize,
    crawl: usize,
    structured: usize,
}

impl PayloadDepth {
    /// A payload entering the pipeline from a crawl or listing rather than from inside another
    /// payload: no archive nesting, no structured chain behind it.
    fn discovered(crawl: usize, structured: usize) -> Self {
        Self {
            container: 0,
            crawl,
            structured,
        }
    }
}

#[derive(Clone)]
struct WorkerContext {
    portal: PortalRegistration,
    client: ApiClient,
    repository: DuckDbRepository,
    corpus_index: CorpusIndex,
    limiter: RateLimiter,
    dispatcher: mpsc::Sender<ScheduledTask>,
    progress_sender: mpsc::Sender<IngestionProgress>,
    global_semaphore: Arc<Semaphore>,
    parser_semaphore: Arc<Semaphore>,
    ocr_semaphore: Arc<Semaphore>,
    parser_memory: Arc<Semaphore>,
    download_root: PathBuf,
    statistics: Arc<PortalStatistics>,
    pools: Arc<RwLock<HashMap<String, PortalPool>>>,
    provisioning_lock: Arc<Mutex<()>>,
    // Bumped whenever this pool's slot frees, so a start parked on a full concurrency cap
    // wakes up and re-checks rather than staying refused forever. Read by `retire_portal_if_idle`
    // as well as the explicit stop/removal paths, which is why it lives on the context rather
    // than only on `QueueManager`: a worker retiring its own pool on natural completion has no
    // other handle back to the manager.
    slot_freed: watch::Sender<u64>,
    cancellation: watch::Sender<bool>,
    sync_run_id: String,
    crawl: Arc<CrawlState>,
}

pub struct QueueManager {
    client: ApiClient,
    repository: DuckDbRepository,
    corpus_index: CorpusIndex,
    limiter: RateLimiter,
    progress_sender: mpsc::Sender<IngestionProgress>,
    download_root: PathBuf,
    pools: Arc<RwLock<HashMap<String, PortalPool>>>,
    provisioning_lock: Arc<Mutex<()>>,
    // Runtime setting: how many portal task pools may run concurrently. Defaults to
    // `DEFAULT_MAXIMUM_ACTIVE_PORTALS` and is adjustable at runtime via
    // `max_active_portals`/`set_max_active_portals`. Not persisted across restarts; see the
    // doc comment on `set_max_active_portals` for why.
    max_active_portals: AtomicUsize,
    slot_freed: watch::Sender<u64>,
    global_semaphore: Arc<Semaphore>,
    parser_semaphore: Arc<Semaphore>,
    ocr_semaphore: Arc<Semaphore>,
    parser_memory: Arc<Semaphore>,
}

enum ExtractedDocument {
    Searchable {
        text: String,
        was_ocr_processed: bool,
    },
    Expanded {
        format: DocumentFormat,
        entry_count: usize,
    },
    Deferred {
        format: DocumentFormat,
        route: ExtractionRoute,
    },
    Salvaged {
        report: crate::parsers::SalvageReport,
        format: DocumentFormat,
    },
}
